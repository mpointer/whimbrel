import AppKit
import Combine
import Foundation
import OSLog

/// Periodically pushes/pulls content (capture items, meetings, transcript segments)
/// to/from the account's Turso data-plane database — the content-sync counterpart to
/// `SettingsSyncCoordinator` (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md).
/// Unlike settings sync, there's no delta to apply on the Swift side:
/// `CoreMemoryStore.syncContent` applies pulled rows directly to the local store
/// inside the Rust call.
@MainActor
final class ContentSyncCoordinator: ObservableObject {
    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "content-sync")

    @Published private(set) var lastSyncError: String?
    @Published private(set) var lastSyncedAt: Date?
    /// The outbox after the last round, for "changes waiting to send" in Settings (#590).
    @Published private(set) var outboxBacklog: CoreOutboxBacklog?

    private let coreStore: CoreMemoryStore?
    private let syncState: SyncState
    private let deviceId: String

    private var periodicTask: Task<Void, Never>?
    /// Runs after each successful round, once pulled rows are applied (live question alerts
    /// from other devices are notified here).
    var afterPull: (() async -> Void)?
    private var statusSubscription: AnyCancellable?

    /// Rounds that have failed in a row. A failed round holds `CoreMemoryStore.accessQueue`
    /// for up to 30 s, so retrying a broken sync every minute froze every other read.
    @Published private(set) var consecutiveFailures = 0
    /// Set while a round runs, so a stuck round reads as stale.
    private(set) var roundStartedAt: Date?
    private var moreToPull = false
    private static let lastSuccessKey = "com.mpointer.kestrel.contentSyncLastSuccess"
    /// The last good round, surviving a relaunch. Starts at the first try, so a device that
    /// has never synced still gets warned.
    private(set) var lastSuccess: Date? = UserDefaults.standard.object(forKey: lastSuccessKey) as? Date

    private static let lastRoundFinishedKey = "com.mpointer.kestrel.contentSyncLastRoundFinished"
    /// The last round that ended, good or bad, surviving a relaunch. With `lastSuccess` it tells
    /// a failing sync from one that isn't running at all (#590).
    private(set) var lastRoundFinishedAt: Date? = UserDefaults.standard.object(forKey: lastRoundFinishedKey) as? Date
    /// When the loop last started and when the Mac last woke: a fresh start gets time to finish its first round.
    private var loopStartedAt: Date?
    /// Published so the sidebar line re-reads at wake (its grace period starts then).
    @Published private(set) var lastWakeAt: Date?
    /// The round now running, so a replaced loop's `defer` can't clear a newer round's start.
    private var roundToken: UUID?
    private var stuckLoggedFor: Date?
    private var lastResumeAttemptAt: Date?
    private var healthTask: Task<Void, Never>?
    private var wakeSubscription: AnyCancellable?
    private var backlogRefreshInFlight = false

    private static let everSyncedKey = "com.mpointer.kestrel.contentSyncEverSynced"
    /// Whether a round has ever succeeded here. Not `lastSuccess`, which the first attempt also sets.
    private(set) var hasEverSynced = UserDefaults.standard.bool(forKey: everSyncedKey)

    /// The sync history goes when sync is turned off, signed out or wiped, so a later sign-in
    /// starts clean instead of reading as four days stale.
    func forgetSyncHistory() {
        lastSuccess = nil
        lastRoundFinishedAt = nil
        hasEverSynced = false
        lastWakeAt = nil
        loopStartedAt = nil
        outboxBacklog = nil
        for key in [Self.lastSuccessKey, Self.lastRoundFinishedKey, Self.everSyncedKey] {
            UserDefaults.standard.removeObject(forKey: key)
        }
        objectWillChange.send()
    }

    /// Whether sync will try again by itself: running, or stopped with a device key to resume from.
    var retryPossible: Bool {
        let status = syncState.status
        if case .ready = status { return true }
        return status.isResumable && syncState.currentDevicePublicKeyBase64 != nil
    }

    /// The Status page's line for a stopped sync, or nil to keep the usual text. Says "keeps
    /// trying" only when a retry can happen: sync worked here before and the device key is there.
    func stoppedLine() -> String? {
        guard case let .error(message) = syncState.status, hasEverSynced else { return nil }
        if retryPossible { return String(localized: "Sync stopped. Whimbrel keeps trying.") }
        return message
    }

    func isStale(now: Date = Date()) -> Bool {
        guard syncState.status.canBeStale else { return false }
        let lastActivityAt = [lastRoundFinishedAt, loopStartedAt, lastWakeAt].compactMap { $0 }.max()
        return SyncTiming.isStale(now: now, lastSuccess: lastSuccess, lastRoundFinishedAt: lastRoundFinishedAt, lastActivityAt: lastActivityAt, roundStartedAt: roundStartedAt)
    }

    private func recordSuccess(_ date: Date) {
        lastSuccess = date
        UserDefaults.standard.set(date, forKey: Self.lastSuccessKey)
    }

    private func recordRoundFinished(_ date: Date) {
        lastRoundFinishedAt = date
        UserDefaults.standard.set(date, forKey: Self.lastRoundFinishedKey)
    }

    /// A round that ended without reaching the core still counts as a failed round: backs off,
    /// shows in the log, and moves `lastRoundFinishedAt` so the watchdog sees the loop is alive.
    private func failRound(reason: String, message: String) {
        consecutiveFailures += 1
        lastSyncError = message
        log.error("syncNow: round not run (\(self.consecutiveFailures, privacy: .public) in a row): \(reason, privacy: .public)")
        recordRoundFinished(Date())
    }

    init(coreStore: CoreMemoryStore?, syncState: SyncState, deviceId: String) {
        self.coreStore = coreStore
        self.syncState = syncState
        self.deviceId = deviceId
    }

    /// Starts (or stops) the periodic timer automatically as `syncState` enters/leaves
    /// `.ready`. Call once, e.g. from `AppState.init()`.
    func observeSyncState() {
        statusSubscription = syncState.$status
            .sink { [weak self] status in
                guard let self else { return }
                // The sidebar line depends on the status as well as on this object.
                self.objectWillChange.send()
                if case .ready = status {
                    self.startPeriodicSync()
                } else {
                    self.stopPeriodicSync()
                }
            }
        startHealthChecks()
    }

    /// Starts the loop unless one is running. Use `restartPeriodicSync` to replace a stuck one.
    func startPeriodicSync() {
        guard periodicTask == nil else { return }
        loopStartedAt = Date()
        periodicTask = Task { [weak self] in
            // The primary-device gate needs the approved list. Without it, every Mac runs routines.
            await self?.syncState.refreshDevices()
            while !Task.isCancelled {
                await self?.syncNow()
                let failures = self?.consecutiveFailures ?? 0
                let more = self?.moreToPull ?? false
                try? await Task.sleep(nanoseconds: SyncTiming.nextDelaySeconds(consecutiveFailures: failures, moreToPull: more) * 1_000_000_000)
            }
        }
    }

    func stopPeriodicSync() {
        periodicTask?.cancel()
        periodicTask = nil
    }

    /// Cancels the loop and starts a new one, for a loop that is gone or asleep with no round
    /// running. It does not touch a round in flight (see `checkHealth`). A cancelled loop starts
    /// no further round (`syncNow` checks for cancellation after its waits).
    func restartPeriodicSync() {
        stopPeriodicSync()
        startPeriodicSync()
    }

    // MARK: - Watchdog and recovery (#590)

    /// A 5-minute timer and the wake notification. Both run whatever the sync status is, because
    /// the loop stops when it leaves `.ready` and nothing else would notice.
    private func startHealthChecks() {
        guard healthTask == nil else { return }
        healthTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: SyncTiming.watchdogIntervalSeconds * 1_000_000_000)
                guard !Task.isCancelled, let self else { return }
                self.refreshBacklog()
                await self.checkHealth(reason: "timer")
            }
        }
        wakeSubscription = NSWorkspace.shared.notificationCenter
            .publisher(for: NSWorkspace.didWakeNotification)
            .sink { [weak self] _ in
                Task { @MainActor in await self?.handleWake() }
            }
    }

    private func handleWake() async {
        lastWakeAt = Date()
        // The network is often not up yet in the first seconds after a wake.
        try? await Task.sleep(nanoseconds: 10 * 1_000_000_000)
        refreshBacklog()
        await checkHealth(reason: "wake")
    }

    /// Decides with `SyncTiming.watchdogAction`, then acts. Logged at `.public` so a stall is
    /// readable in a sysdiagnose.
    func checkHealth(reason: String) async {
        let now = Date()
        let status = syncState.status
        let isReady: Bool
        if case .ready = status { isReady = true } else { isReady = false }
        // The Keychain is read only when there is something to resume.
        let hasDeviceKey = status.isResumable ? syncState.currentDevicePublicKeyBase64 != nil : false
        let action = SyncTiming.watchdogAction(
            statusIsReady: isReady, statusIsResumable: status.isResumable, hasDeviceKey: hasDeviceKey,
            interactiveFlowActive: syncState.interactiveFlowActive,
            consecutiveFailures: consecutiveFailures, now: now, lastRoundFinishedAt: lastRoundFinishedAt, loopStartedAt: loopStartedAt,
            lastWakeAt: lastWakeAt, roundStartedAt: roundStartedAt
        )
        switch action {
        case .healthy:
            if status.isResumable, !hasDeviceKey {
                log.log("watchdog (\(reason, privacy: .public)): sync is stopped (\(String(describing: status), privacy: .public)) and this Mac has no device key, so it can't resume on its own")
            }
        case .restartRound:
            let last = lastRoundFinishedAt?.formatted(.iso8601) ?? "never"
            log.fault("watchdog (\(reason, privacy: .public)): status is ready but no sync round has finished since \(last, privacy: .public); loop \(self.periodicTask == nil ? "was not running" : "is stuck", privacy: .public), restarting it")
            restartPeriodicSync()
        case .roundStuck:
            // Once per stuck round. The round waits on the store queue, which can't be cancelled.
            guard stuckLoggedFor != roundStartedAt else { return }
            stuckLoggedFor = roundStartedAt
            let since = roundStartedAt?.formatted(.iso8601) ?? "unknown"
            log.fault("watchdog (\(reason, privacy: .public)): a sync round has been running since \(since, privacy: .public); leaving it, a second round would only queue behind it")
        case .resumeStatus:
            guard SyncTiming.resumeAllowed(now: now, lastAttemptAt: lastResumeAttemptAt) else { return }
            lastResumeAttemptAt = now
            log.notice("recovery (\(reason, privacy: .public)): sync is stopped (\(String(describing: status), privacy: .public)); signing in again")
            await syncState.silentResumeIfPossible()
            if case .ready = syncState.status {
                log.notice("recovery (\(reason, privacy: .public)): sync is ready again")
            } else {
                log.notice("recovery (\(reason, privacy: .public)): still stopped (\(String(describing: self.syncState.status), privacy: .public)); next try in 30 minutes")
            }
        }
    }

    /// Re-reads the outbox count without waiting on the store queue: a busy or stuck queue
    /// delays the answer, never this timer.
    func refreshBacklog() {
        guard let coreStore, !backlogRefreshInFlight, syncState.status.canBeStale else { return }
        backlogRefreshInFlight = true
        CoreMemoryStore.accessQueue.async { [weak self] in
            let backlog = coreStore.outboxBacklog()
            Task { @MainActor in
                guard let self else { return }
                self.backlogRefreshInFlight = false
                if let backlog, backlog != self.outboxBacklog { self.outboxBacklog = backlog }
            }
        }
    }

    /// One push/pull round trip. Safe to call even when not enrolled — just returns
    /// early. Errors are recorded in `lastSyncError`, never thrown. Every exit path
    /// records SOMETHING in `lastSyncError`/`lastSyncedAt` — a prior version of this
    /// method's very first guard returned silently on a non-`.ready` status, which
    /// left the UI showing literally nothing (neither an error nor a sync time) if
    /// that guard ever failed on the first-ever call, indistinguishable from "never
    /// ran at all."
    func syncNow() async {
        log.log("syncNow: starting (status=\(String(describing: self.syncState.status), privacy: .public))")
        await syncState.renewIfDue()
        // A replaced loop wakes up here after its wait; the new loop owns the next round.
        guard !Task.isCancelled else { return }
        guard case let .ready(tursoToken, tursoDatabaseUrl) = syncState.status else {
            failRound(reason: "sync status is not ready", message: String(localized: "Not enrolled in sync yet."))
            return
        }
        let groupKeyBase64: String
        do {
            guard let loaded = try SyncKeychain.loadGroupKey() else {
                failRound(reason: "no group key in the Keychain", message: String(localized: "No group key available yet."))
                return
            }
            groupKeyBase64 = loaded
        } catch {
            failRound(reason: "reading the group key from the Keychain failed: \(String(describing: error))", message: String(localized: "No group key available yet."))
            return
        }
        guard let coreStore else {
            failRound(reason: "the local store is unavailable", message: String(localized: "Local store is unavailable."))
            return
        }

        let deviceId = self.deviceId
        if lastSuccess == nil { recordSuccess(Date()) }
        let token = UUID()
        roundToken = token
        roundStartedAt = Date()
        defer {
            if roundToken == token {
                roundToken = nil
                roundStartedAt = nil
            }
        }
        // `CoreMemoryStore.accessQueue` serializes every background call against the
        // shared store handle (per its own doc comment, CoreBridge.swift:317-324) — a
        // bare `Task.detached` here would NOT provide that, since two concurrent
        // detached closures could race against the same handle. `withCheckedContinuation`
        // bridges this async function to that serial DispatchQueue.
        let (success, backlog) = await withCheckedContinuation { (continuation: CheckedContinuation<(Bool, CoreOutboxBacklog?), Never>) in
            CoreMemoryStore.accessQueue.async {
                let result = coreStore.syncContent(
                    tursoURL: tursoDatabaseUrl,
                    tursoToken: tursoToken,
                    groupKeyBase64: groupKeyBase64,
                    deviceId: deviceId
                )
                MeetingLifecycle.deleteFilesOfMeetingsForgottenElsewhere(CoreMemoryStore.takeForgottenMeetingFiles())
                continuation.resume(returning: (result, coreStore.outboxBacklog()))
            }
        }

        outboxBacklog = backlog
        log.log("syncNow: whimbrel_sync_content returned success=\(success, privacy: .public), outbox pending=\(backlog?.pending ?? -1, privacy: .public)")
        moreToPull = success && CoreMemoryStore.syncContentHasMoreToPull
        if success {
            consecutiveFailures = 0
            lastSyncError = nil
            lastSyncedAt = Date()
            await SkinSync.round(store: coreStore, skins: .shared, skinStore: .shared, defaults: .standard)
            await afterPull?()
            // One date for both, so a good round never reads as a failed one (`isStale`).
            let finishedAt = Date()
            recordSuccess(finishedAt)
            recordRoundFinished(finishedAt)
            if !hasEverSynced {
                hasEverSynced = true
                UserDefaults.standard.set(true, forKey: Self.everSyncedKey)
            }
        } else {
            consecutiveFailures += 1
            let cause = CoreMemoryStore.lastSyncContentError() ?? "unknown"
            log.error("syncNow: failed (\(self.consecutiveFailures, privacy: .public) in a row): \(cause, privacy: .public)")
            lastSyncError = String(localized: "Content sync failed. Will retry automatically.")
            await syncState.renewIfDue(afterFailure: true)
            recordRoundFinished(Date())
        }
    }
}

/// Skins Task 16: the Mac sends and receives user skins, in the order Windows' `skin_sync.rs` uses.
/// `core/skins::sync` does the work; this decides when. Every call that reads or writes the skins
/// folder for sync runs on `CoreMemoryStore.accessQueue` (the store handle) with `UserSkinStore.writing`
/// held, so a local save can't land half-way through. The main thread only takes `writing` for its own
/// file writes and never waits on `accessQueue` while holding it, so the two can't deadlock.
enum SkinSync {
    /// A skin moved to `.deleted/` is kept this long (an edit that lost to a deletion can be found), then purged.
    static let deletedKept: TimeInterval = 30 * 24 * 60 * 60
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "skins")

    /// At launch: purges old deletions and sends each change made here (`UserSkinStore.changed`) from
    /// then on. A nil store (no database) sends nothing.
    static func connect(store: CoreMemoryStore?, skins: UserSkinStore) {
        DispatchQueue.global(qos: .utility).async {
            let purged = skins.purgeDeleted(olderThan: deletedKept)
            if purged > 0 { log.notice("dropped \(purged, privacy: .public) skins deleted over 30 days ago") }
        }
        guard let store else { return }
        // Queued, not waited on: the store may be busy with a sync round. ponytail: a reconcile already
        // queued ahead of this send can write a peer's version first; the save it replaced stays in Undo,
        // as on Windows.
        skins.changed = { [weak skins] id in
            CoreMemoryStore.accessQueue.async {
                guard let skins else { return }
                UserSkinStore.writing.lock()
                defer { UserSkinStore.writing.unlock() }
                if !store.skinSyncEnqueue(skinsRoot: skins.rootURL, skinId: id) {
                    log.error("could not queue skin \(id, privacy: .public) for sync")
                }
            }
        }
    }

    /// After a good content round, on `accessQueue`: the one-time image downscale, then the skins that
    /// arrived, then (on the first round here, or when a peer has just started syncing skins) every skin
    /// again. Returns whether the folder changed.
    static func afterRound(store: CoreMemoryStore, skins: UserSkinStore, defaults: UserDefaults) -> Bool {
        UserSkinStore.writing.lock()
        defer { UserSkinStore.writing.unlock() }
        let migrated = SkinImageMigration.runIfNeeded(skins: skins, defaults: defaults)
        guard let report = store.skinSyncReconcile(skinsRoot: skins.rootURL) else {
            log.error("could not write the skins from other devices")
            return migrated
        }
        if let error = report.error {
            log.error("could not write the skins from other devices: \(error, privacy: .public)")
        }
        if !report.changed.isEmpty || !report.deleted.isEmpty {
            log.notice("skins from other devices: \(report.changed.count, privacy: .public) changed, \(report.deleted.count, privacy: .public) deleted")
        }
        if report.pushAll, !store.skinSyncEnqueueAll(skinsRoot: skins.rootURL) {
            log.error("could not queue the skins for sync")
        }
        return migrated || !report.changed.isEmpty || !report.deleted.isEmpty
    }

    /// `afterRound`, then a reload when the folder changed: it picks up new and changed skins, recomputes
    /// scrims, and falls back to Whimbrel when the selected skin was deleted (`SkinStore.reloadUserSkins`).
    @MainActor static func round(store: CoreMemoryStore?, skins: UserSkinStore, skinStore: SkinStore, defaults: UserDefaults) async {
        guard let store else { return }
        let changed = await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            CoreMemoryStore.accessQueue.async {
                continuation.resume(returning: afterRound(store: store, skins: skins, defaults: defaults))
            }
        }
        if changed { skinStore.reloadUserSkins() }
    }
}
