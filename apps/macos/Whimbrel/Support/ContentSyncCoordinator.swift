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

    private let coreStore: CoreMemoryStore?
    private let syncState: SyncState
    private let deviceId: String

    private var periodicTask: Task<Void, Never>?
    private var statusSubscription: AnyCancellable?

    /// Rounds that have failed in a row. A failed round holds `CoreMemoryStore.accessQueue`
    /// for up to 30 s, so retrying a broken sync every minute froze every other read.
    private var consecutiveFailures = 0

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
            while !Task.isCancelled {
                await self?.syncNow()
                let failures = self?.consecutiveFailures ?? 0
                try? await Task.sleep(nanoseconds: SyncTiming.nextDelaySeconds(consecutiveFailures: failures) * 1_000_000_000)
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
            lastSyncError = "Not enrolled in sync yet."
            return
        }
        guard let groupKeyBase64 = try? SyncKeychain.loadGroupKey() else {
            lastSyncError = "No group key available yet."
            return
        }
        guard let coreStore else {
            lastSyncError = "Local store is unavailable."
            return
        }

        let deviceId = self.deviceId
        // `CoreMemoryStore.accessQueue` serializes every background call against the
        // shared store handle (per its own doc comment, CoreBridge.swift:317-324) — a
        // bare `Task.detached` here would NOT provide that, since two concurrent
        // detached closures could race against the same handle. `withCheckedContinuation`
        // bridges this async function to that serial DispatchQueue.
        let success = await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            CoreMemoryStore.accessQueue.async {
                let result = coreStore.syncContent(
                    tursoURL: tursoDatabaseUrl,
                    tursoToken: tursoToken,
                    groupKeyBase64: groupKeyBase64,
                    deviceId: deviceId
                )
                continuation.resume(returning: result)
            }
        }

        log.log("syncNow: whimbrel_sync_content returned success=\(success, privacy: .public)")
        if success {
            consecutiveFailures = 0
            lastSyncError = nil
            lastSyncedAt = Date()
        } else {
            consecutiveFailures += 1
            let cause = CoreMemoryStore.lastSyncContentError() ?? "unknown"
            log.error("syncNow: failed (\(self.consecutiveFailures, privacy: .public) in a row): \(cause, privacy: .public)")
            lastSyncError = "Content sync failed. Will retry automatically."
            await syncState.renewIfDue(afterFailure: true)
        }
    }
}
