import Foundation
import OSLog

/// What one pull round needs. Lives only in memory; `description` hides the secrets so a
/// stray `\(credentials)` can never log them.
struct SyncCredentials: CustomStringConvertible {
    let tursoToken: String
    let tursoDatabaseUrl: String
    let groupKeyBase64: String
    /// This device's public key, the Mac's device-id convention.
    let deviceId: String

    var description: String { "SyncCredentials(redacted)" }
}

/// Where the Turso token comes from: `EnrollmentModel` (faked in tests).
@MainActor
protocol SyncCredentialSource: AnyObject {
    func syncCredentials() -> SyncCredentials?
    func renewIfDue(afterFailure: Bool, force: Bool) async
}

/// The local store seam (faked in tests). Calls block; `SyncModel` runs them off main.
protocol MeetingSyncStore: AnyObject {
    func pullOnly(_ credentials: SyncCredentials, captureWindowDays: Int32) -> Bool
    func groupedMeetingCount() -> Int?
    func lastSyncError() -> String?
}

extension CoreMemoryStore: MeetingSyncStore {
    func pullOnly(_ credentials: SyncCredentials, captureWindowDays: Int32) -> Bool {
        setDeviceId(credentials.deviceId)
        return syncContentPullOnly(tursoURL: credentials.tursoDatabaseUrl, tursoToken: credentials.tursoToken,
                                   groupKeyBase64: credentials.groupKeyBase64, deviceId: credentials.deviceId,
                                   captureWindowDays: captureWindowDays)
    }

    func groupedMeetingCount() -> Int? { groupedMeetingCount(limit: 200) }
    func lastSyncError() -> String? { Self.lastSyncContentError() }
}

/// The phone's pull-only content sync (docs/15-mobile-app-design.md §5.1): once approved
/// and in the foreground, pulls now and then every minute, backing off on failures like
/// the Mac's `ContentSyncCoordinator`, and shows the synced meeting count. No background
/// work in M4.0: the shell calls `stop()` when the app leaves the foreground.
@MainActor
final class SyncModel: ObservableObject {
    @Published private(set) var meetingCount: Int?
    @Published private(set) var isSyncing = false
    @Published private(set) var lastSyncedAt: Date?
    @Published private(set) var lastError: String?
    /// Bumped after every pull that succeeded and when the store is reset, so screens know to
    /// reload what they read from the replica.
    @Published private(set) var readRevision = 0

    /// Screen captures older than this aren't pulled (the user's M4.0 decision).
    static let captureWindowDays: Int32 = 30

    /// Set once opened (nil too if opening failed); `openStore` runs once, on the store queue.
    private var store: MeetingSyncStore?
    private var openStore: (() -> MeetingSyncStore?)?
    /// Kept so `reset()` can arm a lazy re-open for the next enrollment.
    private var reopenStore: (() -> MeetingSyncStore?)?
    /// A reset asked for while a round was in flight: runs when that round ends.
    private var pendingReset: (() -> Void)?
    private weak var credentials: SyncCredentialSource?
    private let sleep: (UInt64) async throws -> Void
    private let queue = DispatchQueue(label: "com.mpointer.kestrel.mobile.store", qos: .userInitiated)
    private let log = Logger(subsystem: "com.mpointer.kestrel.mobile", category: "content-sync")
    private var consecutiveFailures = 0
    /// The running cadence loop, if any. Exposed so tests can await it.
    private(set) var loopTask: Task<Void, Never>?

    init(
        store: MeetingSyncStore?,
        credentials: SyncCredentialSource,
        sleep: @escaping (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0 * 1_000_000_000) }
    ) {
        self.store = store
        self.credentials = credentials
        self.sleep = sleep
    }

    /// Opens the store (runs migrations) on the store queue at the first sync instead of
    /// on the main thread at launch; the UI shows "Syncing…" meanwhile.
    init(
        openStore: @escaping () -> MeetingSyncStore?,
        credentials: SyncCredentialSource,
        sleep: @escaping (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0 * 1_000_000_000) }
    ) {
        self.openStore = openStore
        self.reopenStore = openStore
        self.credentials = credentials
        self.sleep = sleep
    }

    func start() {
        guard loopTask == nil else { return }
        loopTask = Task { [weak self] in
            while let self, !Task.isCancelled {
                await self.syncNow()
                do { try await self.sleep(SyncTiming.nextDelaySeconds(consecutiveFailures: self.consecutiveFailures)) } catch { break }
            }
            if !Task.isCancelled { self?.loopTask = nil }
        }
    }

    func stop() {
        loopTask?.cancel()
        loopTask = nil
    }

    /// The phone was removed from the account: stop, wait out a round in flight, release the
    /// store (its deinit closes the handle, so the caller can delete the directory) and forget
    /// what was shown. The next sync re-opens a fresh store.
    /// `cleanup` runs once the store is released (delete the directory there). Never waits:
    /// the revoke is usually found by a round's own `renewIfDue`, so waiting would deadlock;
    /// during a round the reset is deferred to that round's end.
    func reset(then cleanup: @escaping () -> Void) {
        stop()
        if isSyncing {
            pendingReset = cleanup
            return
        }
        store = nil
        openStore = reopenStore
        meetingCount = nil
        lastSyncedAt = nil
        lastError = nil
        consecutiveFailures = 0
        readRevision += 1
        cleanup()
    }

    /// One pull round. `manual` (the Sync now button) may re-authenticate inside the
    /// automatic retry floor when there's no token yet.
    func syncNow(manual: Bool = false) async {
        guard !isSyncing else { return }
        isSyncing = true
        defer {
            isSyncing = false
            if let cleanup = pendingReset {
                pendingReset = nil
                reset(then: cleanup)
            }
        }
        if let open = openStore {
            openStore = nil
            store = await withCheckedContinuation { continuation in
                queue.async { continuation.resume(returning: open()) }
            }
            // Screens that asked before the store existed got nothing: have them read again.
            readRevision += 1
        }
        guard let store else {
            lastError = "This iPhone's local storage is unavailable."
            return
        }
        if meetingCount == nil { meetingCount = await run(store) { $0.groupedMeetingCount() } }

        await credentials?.renewIfDue(afterFailure: false, force: manual)
        guard let current = credentials?.syncCredentials() else {
            consecutiveFailures += 1
            lastError = "Couldn't reach the server. Will retry automatically."
            return
        }
        let ok = await run(store) { $0.pullOnly(current, captureWindowDays: Self.captureWindowDays) }
        if ok {
            consecutiveFailures = 0
            lastError = nil
            lastSyncedAt = Date()
            readRevision += 1
            meetingCount = await run(store) { $0.groupedMeetingCount() } ?? meetingCount
        } else {
            consecutiveFailures += 1
            // The core's cause can name the database host: private in the log, never shown.
            let cause = await run(store) { $0.lastSyncError() } ?? "unknown"
            log.error("pull failed (\(self.consecutiveFailures, privacy: .public) in a row): \(cause, privacy: .private)")
            lastError = "Couldn't sync. Will retry automatically."
            await credentials?.renewIfDue(afterFailure: true, force: false)
        }
    }

    /// Reads the replica for a screen: runs `body` on the store queue, off the main thread.
    /// Nil when the store is not open (not yet synced, or wiped) or cannot be read.
    func read<T>(_ body: @escaping (PhoneReadStore) -> T) async -> T? {
        guard let store = store as? PhoneReadStore else { return nil }
        return await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: body(store)) }
        }
    }

    /// Runs a blocking store call on the serial store queue.
    private func run<T>(_ store: MeetingSyncStore, _ body: @escaping (MeetingSyncStore) -> T) async -> T {
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: body(store)) }
        }
    }
}
