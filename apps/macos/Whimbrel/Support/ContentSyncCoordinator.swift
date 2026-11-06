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

    func isStale(now: Date = Date()) -> Bool {
        SyncTiming.isStale(now: now, lastSuccess: lastSuccess, consecutiveFailures: consecutiveFailures, roundStartedAt: roundStartedAt)
    }

    private func recordSuccess(_ date: Date) {
        lastSuccess = date
        UserDefaults.standard.set(date, forKey: Self.lastSuccessKey)
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
                if case .ready = status {
                    self.startPeriodicSync()
                } else {
                    self.stopPeriodicSync()
                }
            }
    }

    func startPeriodicSync() {
        guard periodicTask == nil else { return }
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
        guard case let .ready(tursoToken, tursoDatabaseUrl) = syncState.status else {
            lastSyncError = String(localized: "Not enrolled in sync yet.")
            return
        }
        guard let groupKeyBase64 = try? SyncKeychain.loadGroupKey() else {
            lastSyncError = String(localized: "No group key available yet.")
            return
        }
        guard let coreStore else {
            lastSyncError = String(localized: "Local store is unavailable.")
            return
        }

        let deviceId = self.deviceId
        if lastSuccess == nil { recordSuccess(Date()) }
        roundStartedAt = Date()
        defer { roundStartedAt = nil }
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
            recordSuccess(Date())
            await SkinSync.round(store: coreStore, skins: .shared, skinStore: .shared, defaults: .standard)
            await afterPull?()
        } else {
            consecutiveFailures += 1
            let cause = CoreMemoryStore.lastSyncContentError() ?? "unknown"
            log.error("syncNow: failed (\(self.consecutiveFailures, privacy: .public) in a row): \(cause, privacy: .public)")
            lastSyncError = String(localized: "Content sync failed. Will retry automatically.")
            await syncState.renewIfDue(afterFailure: true)
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
