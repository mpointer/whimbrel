import Foundation
import OSLog
import UIKit

/// Where the phone is in device enrollment (docs/15-mobile-app-design.md §5.1).
/// No case carries a token or key, so logging a status is always safe.
enum EnrollmentStatus: Equatable {
    /// The build has no usable iOS OAuth client id / Worker URL.
    case notConfigured(String)
    case signedOut
    case signingIn
    /// Signed in, waiting for an approved desktop to approve this phone. `code` is the
    /// comparison code of this device's public key, shown on the Mac's approve prompt too.
    case pending(code: String)
    /// Approved, but no group key arrived (the handoff expired before we polled).
    /// The user has to re-approve this phone from the Mac.
    case waitingForGroupKey(code: String)
    /// Signed in, but no approved desktop exists on the account. The phone never creates
    /// the account's group key (it's a peer approved by a desktop), so set up the Mac first.
    case needsDesktopFirst
    /// `409 device_limit`: the free plan's phone slot is taken. Nothing was enrolled.
    case deviceLimit(kind: String, holders: [LimitHolder])
    /// Approved and the group key is in the Keychain.
    case approved
    case error(String)
    /// The account revoked this phone: its keys, Google sign-in and local store are gone.
    case removed
}

/// The network/Google seam, faked in tests. `LiveEnrollmentBackend` is the real one.
protocol EnrollmentBackend: AnyObject {
    /// Interactive Google sign-in (consent screen).
    func signIn() async throws
    /// A currently valid Google ID token, refreshed silently; throws when not signed in.
    func idToken() async throws -> String
    func authenticate(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult
    func listDevices(idToken: String, devicePublicKey: String) async throws -> [SyncDeviceRecord]
    /// Revokes a device row; the phone revokes its own when it is approved but cannot get a key.
    func revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) async throws
    /// Names this phone's own device row (approved devices only), so the Mac lists it by name.
    func setDeviceName(idToken: String, devicePublicKey: String, name: String) async throws
    /// Hands `targetDeviceId` the group key, wrapped to its public key (`wrappedGroupKey`).
    func approve(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) async throws
    /// "Use this phone instead": this phone becomes a pending swap for `replaceDeviceId`.
    func swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) async throws
    func joinWaitlist(idToken: String, devicePublicKey: String, wants: [String]) async throws
    /// Forgets the Google sign-in (deletes its Keychain tokens).
    func signOut()
}

/// The phone's enrollment flow. Mirrors the Mac's `SyncState` (enableSync,
/// silentResumeIfPossible, handle(result:), resolveGroupKey, the waiting poll, renewIfDue)
/// minus the Devices-panel actions, and minus group-key generation: the phone only ever
/// receives the key from an approved desktop. The decisions (group-key table, permanent
/// errors, messages) come from the core, as there. Like the Mac, the Turso token lives
/// only in memory and is renewed by re-authenticating.
@MainActor
final class EnrollmentModel: ObservableObject, SyncCredentialSource {
    @Published private(set) var status: EnrollmentStatus
    /// Called once per wipe, after the Keychain is cleared: closes and deletes the local store
    /// (wired by the app to `SyncModel.reset`).
    var onWipe: (() -> Void)?

    /// Same cadence as the Mac's waiting poll (SyncState.waitingPoll*).
    static let basePollSeconds: UInt64 = 3
    static let maxPollSeconds: UInt64 = 30

    /// Names of the account's devices (public key to name) and this phone's own id, for
    /// labelling where a meeting was recorded. Best effort: empty until the first refresh.
    @Published private(set) var deviceLabels = DeviceLabels()
    /// Devices waiting for this phone's (or a computer's) approval: the Settings badge and the Today banner.
    @Published private(set) var pendingDeviceCount = 0
    /// This phone asked to replace the synced phone: the pending screen says who approves it.
    @Published private(set) var swapRequested = false
    /// A failed swap or waitlist request, for the limit screen.
    @Published private(set) var actionError: String?

    private let backend: EnrollmentBackend?
    private let sleep: (UInt64) async throws -> Void
    private let log = Logger(subsystem: "com.mpointer.kestrel.mobile", category: "enrollment")
    /// The running waiting poll, if any. Exposed so tests can await it.
    private(set) var pollTask: Task<Void, Never>?
    private var resumeTask: Task<Void, Never>?

    /// The Worker's Turso token for the account's data database: memory only, as on the Mac.
    private var turso: (token: String, databaseUrl: String)?
    /// Wall-clock times, as SyncState's: a phone asleep past the token's expiry renews at once.
    private var lastAuthAt: Date?
    private var lastRenewAttemptAt: Date?
    /// This phone's server device id when a self-revoke failed (offline): retried at the next sign-in.
    private var owedSelfRevoke: String?

    init(
        backend: EnrollmentBackend?,
        configError: String? = nil,
        sleep: @escaping (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0 * 1_000_000_000) }
    ) {
        self.backend = backend
        self.sleep = sleep
        if backend == nil {
            status = .notConfigured(configError ?? "Google sign-in is not configured in this build.")
        } else if (try? SyncKeychain.loadGroupKey()) != nil, (try? SyncKeychain.loadDeviceSecretKey()) != nil {
            // Approved before: stay approved offline; `resume()` fetches a fresh token.
            status = .approved
        } else {
            status = .signedOut
        }
    }

    /// Doubles after a failed poll, capped; back to the base after a good one.
    static func nextPollInterval(current: UInt64, succeeded: Bool) -> UInt64 {
        succeeded ? basePollSeconds : min(current * 2, maxPollSeconds)
    }

    /// The "Sign in with Google" button.
    func signIn() async {
        guard let backend else { return }
        status = .signingIn
        var requestKey: String?
        do {
            try await backend.signIn()
            let idToken = try await backend.idToken()
            if let owed = owedSelfRevoke, let oldKey = try loadDevicePublicKey() {
                await selfRevoke(deviceId: owed, publicKey: oldKey)
                if owedSelfRevoke != nil { return status = .needsDesktopFirst }
            }
            let publicKey = try ensureDeviceKeypair()
            requestKey = publicKey
            let result = try await backend.authenticate(idToken: idToken, devicePublicKey: publicKey)
            try await handle(result, publicKey: publicKey)
        } catch GoogleSignInError.cancelled {
            status = .signedOut
        } catch where Self.isRevoked(error) && requestKey != nil {
            wipeRevoked(for: requestKey ?? "")
        } catch SyncAPIError.httpError(status: 409, body: let body) where body.contains("desktop_required") {
            // The Worker refused to make a phone the account's first device: nothing was enrolled.
            status = .needsDesktopFirst
        } catch {
            status = .error(Self.userMessage(for: error))
        }
    }

    /// Revokes this phone's own row, then discards its device key (a fresh keypair at the next
    /// sign-in). If the revoke fails the key stays and `owedSelfRevoke` makes the next sign-in retry.
    private func selfRevoke(deviceId: String?, publicKey: String) async {
        // No id (own row not in the device list): nothing was revoked, so keep the key; the
        // next sign-in re-runs this path.
        guard let deviceId, let backend else { return }
        do {
            try await backend.revokeDevice(idToken: try await backend.idToken(), devicePublicKey: publicKey, targetDeviceId: deviceId)
        } catch {
            owedSelfRevoke = deviceId
            return
        }
        owedSelfRevoke = nil
        try? SyncKeychain.deleteDeviceSecretKey()
    }

    /// On launch (and for each token renewal): if this phone signed in before, re-check
    /// silently (no consent screen). Joins a re-check already in flight.
    func resume() async {
        if let resumeTask { return await resumeTask.value }
        let task = Task { await self.resumeOnce() }
        resumeTask = task
        await task.value
        resumeTask = nil
    }

    private func resumeOnce() async {
        guard let backend else { return }
        guard let publicKey = try? loadDevicePublicKey() else { return }
        lastRenewAttemptAt = Date()
        do {
            let idToken = try await backend.idToken()
            let result = try await backend.authenticate(idToken: idToken, devicePublicKey: publicKey)
            try await handle(result, publicKey: publicKey)
        } catch {
            // Offline or a passing failure: the user keeps what they see (approved stays
            // approved; otherwise the sign-in screen).
            log.log("resume failed: \(String(describing: type(of: error)), privacy: .public)")
            if Self.isRevoked(error) { return wipeRevoked(for: publicKey) }
            // Refused outright (revoked, signed out of Google): say so rather than show
            // "approved" while every pull fails. SyncState.silentResumeIfPossible does the same.
            if status == .approved, Self.isPermanent(error) {
                turso = nil
                status = .error(Self.userMessage(for: error))
            }
        }
    }

    // MARK: SyncCredentialSource

    func syncCredentials() -> SyncCredentials? {
        guard status == .approved, let turso,
              let groupKey = try? SyncKeychain.loadGroupKey(),
              let deviceId = try? loadDevicePublicKey() else { return nil }
        return SyncCredentials(tursoToken: turso.token, tursoDatabaseUrl: turso.databaseUrl, groupKeyBase64: groupKey, deviceId: deviceId)
    }

    /// SyncState.renewIfDue: re-authenticate before the Turso token expires, or after a
    /// failed pull (a revoke rotates every device's token). `force` (Sync now) skips the
    /// retry floor.
    func renewIfDue(afterFailure: Bool, force: Bool) async {
        guard status == .approved else { return }
        if let resumeTask { return await resumeTask.value }
        guard SyncTiming.renewalDue(now: Date(), lastAuthAt: lastAuthAt, lastAttemptAt: force ? nil : lastRenewAttemptAt, afterFailure: afterFailure) else { return }
        await resume()
    }

    // MARK: Foreground / background

    /// No polling in the background (M4.0 runs no background work).
    func enterBackground() {
        pollTask?.cancel()
        pollTask = nil
    }

    /// Back in the foreground while waiting for approval: re-check now, which restarts
    /// the poll.
    func enterForeground() async {
        lastDeviceRefreshAt = nil // the next sync refreshes the device list
        guard pollTask == nil else { return }
        switch status {
        case .pending, .waitingForGroupKey: await resume()
        default: return
        }
    }

    /// Names this phone's row (while pending, so the Mac's approve prompt shows it; again once
    /// approved, as a retry). Best effort: a failure leaves the Mac showing "Unnamed device".
    private func nameDevice(publicKey: String) async {
        guard let backend, let idToken = try? await backend.idToken() else { return }
        try? await backend.setDeviceName(idToken: idToken, devicePublicKey: publicKey, name: UIDevice.current.name)
    }

    private func handle(_ result: GoogleAuthResult, publicKey: String) async throws {
        let code = CoreMemoryStore.deviceComparisonCode(publicKeyBase64: publicKey) ?? "unavailable"
        switch result {
        case .pending:
            status = .pending(code: code)
            await nameDevice(publicKey: publicKey)
            startPolling(publicKey: publicKey)
        case let .deviceLimit(kind, holders):
            pollTask?.cancel()
            pollTask = nil
            swapRequested = false
            status = .deviceLimit(kind: kind, holders: holders)
        case let .approved(tursoToken, tursoDatabaseUrl, wrappedGroupKey):
            swapRequested = false
            if case let .needsDesktop(ownDeviceId) = try await resolveGroupKey(wrappedGroupKey: wrappedGroupKey, publicKey: publicKey) {
                // Approved but keyless, and no desktop to hand over a key: leaving would strand the
                // account (a later Mac setup would find an approved peer), so step out of it.
                // Revoke first: from the poll task, cancelling it would cancel the revoke request.
                await selfRevoke(deviceId: ownDeviceId, publicKey: publicKey)
                pollTask?.cancel()
                pollTask = nil
                status = .needsDesktopFirst
            } else if try SyncKeychain.loadGroupKey() != nil {
                status = .approved
                turso = (tursoToken, tursoDatabaseUrl)
                lastAuthAt = Date()
                lastRenewAttemptAt = lastAuthAt
                pollTask?.cancel()
                pollTask = nil
                await nameDevice(publicKey: publicKey)
            } else {
                status = .waitingForGroupKey(code: code)
                startPolling(publicKey: publicKey)
            }
        }
    }

    private enum GroupKeyOutcome { case resolvedOrWaiting, needsDesktop(ownDeviceId: String?) }

    /// SyncState.resolveGroupKey (core's decision table), except the phone never
    /// generates the account's group key: where the Mac would ("generate": no other
    /// approved device exists), the phone reports `.needsDesktop` instead.
    private func resolveGroupKey(wrappedGroupKey: String?, publicKey: String) async throws -> GroupKeyOutcome {
        guard let backend else { return .resolvedOrWaiting }
        let localKeyHeld = try SyncKeychain.loadGroupKey() != nil
        switch CoreMemoryStore.syncGroupKeyDecision(wrappedKeyPresent: wrappedGroupKey != nil, localKeyHeld: localKeyHeld, anotherApprovedDeviceExists: false) {
        case .keep, .waitForHandoff:
            return .resolvedOrWaiting
        case .unwrap:
            guard let secretKey = try SyncKeychain.loadDeviceSecretKey() else { throw SyncStateError.noDeviceKeypair }
            let devices = try await backend.listDevices(idToken: try await backend.idToken(), devicePublicKey: publicKey)
            for sender in devices where sender.publicKey != publicKey && sender.status == "approved" {
                if let groupKey = CoreMemoryStore.unwrapSyncGroupKey(recipientSecretKeyBase64: secretKey, senderPublicKeyBase64: sender.publicKey, wrappedGroupKeyBase64: wrappedGroupKey ?? "") {
                    try SyncKeychain.saveGroupKey(groupKey)
                    return .resolvedOrWaiting
                }
            }
            return .resolvedOrWaiting
        case .generate:
            let devices = try await backend.listDevices(idToken: try await backend.idToken(), devicePublicKey: publicKey)
            let anotherApproved = devices.contains { $0.publicKey != publicKey && $0.status == "approved" }
            let decision = CoreMemoryStore.syncGroupKeyDecision(wrappedKeyPresent: false, localKeyHeld: false, anotherApprovedDeviceExists: anotherApproved)
            return decision == .generate ? .needsDesktop(ownDeviceId: devices.first { $0.publicKey == publicKey }?.id) : .resolvedOrWaiting
        }
    }

    /// SyncState.startWaitingPollIfNeeded: re-ask `POST /v1/auth/google` until approved,
    /// backing off on transient failures, stopping on permanent ones (401/403).
    private func startPolling(publicKey: String) {
        // Not from a cancelled poll (e.g. `handle` after its own awaits, once backgrounded).
        guard pollTask == nil, !Task.isCancelled, let backend else { return }
        pollTask = Task { [weak self] in
            var interval = Self.basePollSeconds
            while let self, !Task.isCancelled {
                do { try await self.sleep(interval) } catch { break }
                do {
                    let result = try await backend.authenticate(idToken: try await backend.idToken(), devicePublicKey: publicKey)
                    // The request ignores cancellation: if the app went to the background
                    // meanwhile, drop the answer rather than act on it (it could start a poll).
                    guard !Task.isCancelled else { break }
                    try await self.handle(result, publicKey: publicKey)
                    if case .approved = self.status { break }
                    interval = Self.nextPollInterval(current: interval, succeeded: true)
                } catch {
                    // Cancelled meanwhile (backgrounded): the error is stale, don't act on it.
                    guard !Task.isCancelled else { break }
                    if Self.isRevoked(error) {
                        self.wipeRevoked(for: publicKey)
                        break
                    }
                    if Self.isPermanent(error) {
                        self.status = .error(Self.userMessage(for: error))
                        break
                    }
                    interval = Self.nextPollInterval(current: interval, succeeded: false)
                }
            }
            // A cancelled poll was already cleared by whoever cancelled it, and a new one
            // may have started since.
            if !Task.isCancelled { self?.pollTask = nil }
        }
    }

    /// The Worker's permanent "device revoked" answer (403), and only that one.
    static func isRevoked(_ error: Error) -> Bool {
        if case let SyncAPIError.httpError(status: 403, body: body) = error { return body.contains("device revoked") }
        return false
    }

    /// Best-effort cleanup of an honest device (the group key is not rotated on revoke):
    /// forget keys, Google tokens and the local store, and go back to signed-out. Status flips
    /// first, so a second revoked answer (or one racing the poll) is a no-op.
    /// `publicKey` is the keypair the revoked answer was for: an answer for an older keypair
    /// (a request in flight across a wipe and a new Sign in) must not wipe the new one.
    func wipeRevoked(for publicKey: String) {
        // Fail toward wiping: skip only when a key was read and it is a different one.
        if let current = try? loadDevicePublicKey(), current != publicKey { return }
        guard status != .removed else { return }
        status = .removed
        pollTask?.cancel()
        pollTask = nil
        turso = nil
        owedSelfRevoke = nil
        swapRequested = false
        try? SyncKeychain.deleteGroupKey()
        try? SyncKeychain.deleteDeviceSecretKey()
        PhoneSettingsSync.wipeLocal()
        backend?.signOut()
        onWipe?()
    }

    /// Retrying won't help: the Worker refused this device/account (401/403), or Google
    /// sign-in is gone (refresh token revoked, Keychain item missing).
    static func isPermanent(_ error: Error) -> Bool {
        if case GoogleSignInError.notSignedIn = error { return true }
        guard let apiError = error as? SyncAPIError else { return false }
        return CoreMemoryStore.syncWorkerErrorIsPermanent(apiError)
    }

    private func ensureDeviceKeypair() throws -> String {
        if let existing = try loadDevicePublicKey() { return existing }
        guard let generated = CoreMemoryStore.generateSyncDeviceKeypair() else { throw SyncStateError.cryptoFailed }
        try SyncKeychain.saveDeviceSecretKey(generated.secretKeyBase64)
        return generated.publicKeyBase64
    }

    // MARK: The device limit

    /// The waitlist's checkboxes, in order, with the Worker's exact `wants` strings (same as the Mac).
    static let waitlistOptions: [(label: String, want: String)] = [
        ("More devices", "more_devices"),
        ("Routines that run while my computer is closed", "routines_while_closed"),
        ("Notifications on my phone", "phone_notifications"),
        ("Backups", "backups"),
    ]

    static func limitMessage(holders: [LimitHolder]) -> String {
        "Whimbrel's free sync includes one computer and one phone. \(holders.first?.name ?? "Your other phone") is your synced phone."
    }

    /// "Use this phone instead": becomes a pending swap for `holder`, approved by a computer
    /// or by the old phone. Unlike the Mac there is no fresh start: a phone can't make the group key.
    func swap(replacing holder: LimitHolder) async {
        guard let backend, let publicKey = (try? loadDevicePublicKey()) ?? nil else { return }
        actionError = nil
        do {
            try await backend.swapDevice(idToken: try await backend.idToken(), devicePublicKey: publicKey, replaceDeviceId: holder.id)
            swapRequested = true
            status = .pending(code: CoreMemoryStore.deviceComparisonCode(publicKeyBase64: publicKey) ?? "unavailable")
            await nameDevice(publicKey: publicKey)
            startPolling(publicKey: publicKey)
        } catch {
            actionError = Self.userMessage(for: error)
        }
    }

    /// True when the Worker took the request.
    func joinWaitlist(wants: [String]) async -> Bool {
        guard let backend, let publicKey = (try? loadDevicePublicKey()) ?? nil else { return false }
        actionError = nil
        do {
            try await backend.joinWaitlist(idToken: try await backend.idToken(), devicePublicKey: publicKey, wants: wants)
            return true
        } catch {
            actionError = Self.userMessage(for: error)
            return false
        }
    }

    /// The Devices screen's model, or nil in a build with no Worker.
    func makeApprovalModel() -> DeviceApprovalModel? {
        backend.map { DeviceApprovalModel(backend: $0, onDevices: { [weak self] in self?.noteDevices($0) }) }
    }

    private func noteDevices(_ records: [SyncDeviceRecord]) {
        pendingDeviceCount = records.filter { $0.status == "pending" && $0.publicKey != deviceLabels.ownDeviceId }.count
    }

    /// Fetches the account's device list for `deviceLabels`. Silent on any failure (offline,
    /// token trouble): labels fall back to "Device xxxxxx".
    /// Throttled (each call is a registry write): runs when names are missing, after the app
    /// became active, or 5 minutes after the last one.
    static let deviceRefreshInterval: TimeInterval = 300
    private var lastDeviceRefreshAt: Date?

    func refreshDeviceLabels() async {
        if !deviceLabels.names.isEmpty, let last = lastDeviceRefreshAt, Date().timeIntervalSince(last) < Self.deviceRefreshInterval { return }
        guard let backend, status == .approved, let publicKey = (try? loadDevicePublicKey()) ?? nil,
              let idToken = try? await backend.idToken(),
              let records = try? await backend.listDevices(idToken: idToken, devicePublicKey: publicKey) else { return }
        lastDeviceRefreshAt = Date()
        deviceLabels = DeviceLabels(names: DeviceLabels.names(from: records), ownDeviceId: publicKey)
        noteDevices(records)
    }

    private func loadDevicePublicKey() throws -> String? {
        guard let secret = try SyncKeychain.loadDeviceSecretKey() else { return nil }
        return CoreMemoryStore.syncDevicePublicKey(secretKeyBase64: secret)
    }

    static func userMessage(for error: Error) -> String {
        if let apiError = error as? SyncAPIError, let message = CoreMemoryStore.syncWorkerErrorMessage(apiError) {
            return message
        }
        switch error {
        case GoogleSignInError.notSignedIn: return "You're not signed in. Sign in with Google again."
        case GoogleSignInError.failed: return "Couldn't complete Google sign-in. Try again."
        case SyncKeychainError.readFailed, SyncKeychainError.writeFailed: return "Couldn't access the Keychain. Try again."
        case SyncStateError.cryptoFailed: return "A local encryption operation failed. Try again."
        case is URLError: return "Couldn't reach the server. Check your connection and try again."
        default: return "Something went wrong. Try again."
        }
    }
}

/// SyncState.swift's error type (Mac-only file), same cases.
enum SyncStateError: Error {
    case cryptoFailed
    case noDeviceKeypair
    case noGroupKey
}

/// Reads and checks the build's sync config (Info.plist via the shared SyncConfig).
enum EnrollmentConfig {
    /// Rejects the empty value (no Secrets.xcconfig) and placeholders like REPLACE_ME.
    static func isUsableClientId(_ id: String) -> Bool {
        id.hasSuffix(".apps.googleusercontent.com") && id.count > ".apps.googleusercontent.com".count
    }

    /// Google's iOS-client redirect scheme: the client id reversed,
    /// "123-abc.apps.googleusercontent.com" -> "com.googleusercontent.apps.123-abc".
    static func redirectScheme(clientId: String) -> String {
        clientId.split(separator: ".").reversed().joined(separator: ".")
    }

    /// Keychain items survive app deletion, UserDefaults do not: with no marker this is a
    /// fresh install, so drop any sync keys / Google tokens a previous install left behind.
    static func clearStaleSecretsOnFirstLaunch(defaults: UserDefaults = .standard, signOutGoogle: () -> Void) {
        let marker = "kestrel.sync.keychainCleaned"
        guard !defaults.bool(forKey: marker) else { return }
        try? SyncKeychain.deleteGroupKey()
        try? SyncKeychain.deleteDeviceSecretKey()
        PhoneSettingsSync.wipeLocal(defaults: defaults)
        signOutGoogle()
        defaults.set(true, forKey: marker)
    }

    /// The live model, or a `.notConfigured` one with the reason.
    @MainActor
    static func makeModel() -> EnrollmentModel {
        let notConfigured = "Google sign-in is not configured in this build. Set GOOGLE_IOS_CLIENT_ID in apps/mobile/ios/Secrets.xcconfig."
        guard let clientId = try? SyncConfig.googleOAuthClientId(), isUsableClientId(clientId) else {
            return EnrollmentModel(backend: nil, configError: notConfigured)
        }
        guard let baseURL = try? SyncConfig.workerBaseURL() else {
            return EnrollmentModel(backend: nil, configError: "The sync server URL is not configured in this build.")
        }
        let google = GoogleSignIn(clientId: clientId)
        clearStaleSecretsOnFirstLaunch(signOutGoogle: google.signOut)
        return EnrollmentModel(backend: LiveEnrollmentBackend(googleSignIn: google, client: SyncClient(baseURL: baseURL)))
    }
}

final class LiveEnrollmentBackend: EnrollmentBackend {
    private let googleSignIn: GoogleSignIn
    private let client: SyncClient

    init(googleSignIn: GoogleSignIn, client: SyncClient) {
        self.googleSignIn = googleSignIn
        self.client = client
    }

    func signIn() async throws { try await googleSignIn.authorize() }
    func idToken() async throws -> String { try await googleSignIn.validIdToken() }
    func authenticate(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
        try await client.authenticateGoogle(idToken: idToken, devicePublicKey: devicePublicKey)
    }
    func listDevices(idToken: String, devicePublicKey: String) async throws -> [SyncDeviceRecord] {
        try await client.listDevices(idToken: idToken, devicePublicKey: devicePublicKey)
    }
    func revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) async throws {
        try await client.revoke(idToken: idToken, devicePublicKey: devicePublicKey, targetDeviceId: targetDeviceId)
    }
    func setDeviceName(idToken: String, devicePublicKey: String, name: String) async throws {
        try await client.setDeviceName(idToken: idToken, devicePublicKey: devicePublicKey, name: name)
    }
    func approve(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) async throws {
        try await client.approveDevice(idToken: idToken, devicePublicKey: devicePublicKey, targetDeviceId: targetDeviceId, wrappedGroupKey: wrappedGroupKey)
    }
    func swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) async throws {
        try await client.swapDevice(idToken: idToken, devicePublicKey: devicePublicKey, replaceDeviceId: replaceDeviceId)
    }
    func joinWaitlist(idToken: String, devicePublicKey: String, wants: [String]) async throws {
        try await client.joinWaitlist(idToken: idToken, devicePublicKey: devicePublicKey, wants: wants)
    }
    func signOut() { googleSignIn.signOut() }
}
