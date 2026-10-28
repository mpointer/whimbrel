import AuthenticationServices
import Foundation
import OSLog

enum SyncStatus: Equatable {
    case notSignedIn
    case signingIn
    case waitingForApproval
    /// The account is approved but no group key is held yet — either the
    /// 5-minute handoff TTL expired before this device polled again, or
    /// (defensively) the device that approved us couldn't be identified
    /// among the account's current devices. The user needs to have
    /// another already-approved device re-approve this one.
    case waitingForGroupKey
    case ready(tursoToken: String, tursoDatabaseUrl: String)
    case error(String)
}

extension SyncStatus: CustomStringConvertible {
    /// Without this, `\(status)`/`String(describing: status)` (e.g. in a log line) dumps
    /// `.ready`'s raw associated values — a live Turso bearer token and the account's
    /// database URL — in cleartext into the system log. Every case name is safe to log;
    /// only `.ready`'s payload needs redacting.
    var description: String {
        switch self {
        case .notSignedIn: return "notSignedIn"
        case .signingIn: return "signingIn"
        case .waitingForApproval: return "waitingForApproval"
        case .waitingForGroupKey: return "waitingForGroupKey"
        case .ready: return "ready"
        case .error(let message): return "error(\(message))"
        }
    }
}

/// Orchestrates device enrollment/pairing end to end: the device keypair
/// and group key (Keychain, via `SyncKeychain`), Google Sign-In (via a
/// `GoogleOAuthHelper` scoped to `openid email` — a different scope and
/// Keychain account than the already-shipped Calendar connector's own
/// instance), and the sync auth Worker's three routes (via `SyncClient`).
/// Held on `AppState` as a `let`, matching `routing`/`retention`'s own
/// pattern.
@MainActor
final class SyncState: ObservableObject {
    @Published private(set) var status: SyncStatus = .notSignedIn
    @Published private(set) var devices: [SyncDeviceRecord] = []
    /// Error from a Devices-panel action (`approve`) that failed, kept
    /// separate from `status` so a transient failure re-approving/
    /// re-sending a key to one peer doesn't blow away an otherwise-healthy
    /// `.ready` state and its device list.
    @Published private(set) var actionError: String?
    /// This device's own locally-editable name draft, persisted so it
    /// survives relaunch — separate from whatever's actually saved
    /// server-side (`SyncDeviceRecord.name`, read via `devices`), since
    /// this is what the Settings text field is bound to before "Save" is
    /// pressed. Loaded from UserDefaults on first access, matching this
    /// app's existing simple-local-setting patterns.
    @Published var deviceName: String = UserDefaults.standard.string(forKey: "com.mpointer.kestrel.deviceName") ?? "" {
        didSet {
            UserDefaults.standard.set(deviceName, forKey: "com.mpointer.kestrel.deviceName")
        }
    }

    private let oauthHelper: GoogleOAuthHelper?
    private let syncClient: SyncClient?
    private let configError: String?
    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "sync-state")

    private var waitingPollTask: Task<Void, Never>?
    private var devicesPollTask: Task<Void, Never>?
    /// Wall-clock times (not uptime) so a Mac that slept through the Turso token's
    /// expiry still sees it as due on the first tick after waking.
    private var lastAuthAt: Date?
    private var lastRenewAttemptAt: Date?


    /// How often the waiting screen re-polls `POST /v1/auth/google` while
    /// this device's own status is still `pending`, absent any failures.
    static let waitingPollIntervalSeconds: UInt64 = 3
    /// Ceiling for the waiting poll's backoff (see `startWaitingPollIfNeeded`)
    /// after repeated transient failures — caps it around 120 requests/hour
    /// per pending device instead of ~1,200 at the un-backed-off interval.
    static let waitingPollMaxIntervalSeconds: UInt64 = 30
    /// How often the Devices panel re-polls `POST /v1/devices` while open.
    static let devicesPollIntervalSeconds: UInt64 = 5

    init() {
        do {
            let clientId = try SyncConfig.googleOAuthClientId()
            let clientSecret = try SyncConfig.googleOAuthClientSecret()
            let baseURL = try SyncConfig.workerBaseURL()
            oauthHelper = GoogleOAuthHelper(clientId: clientId, clientSecret: clientSecret, scope: "openid email", keychainAccount: "google-sync")
            syncClient = SyncClient(baseURL: baseURL)
            configError = nil
        } catch {
            // Missing Secrets.xcconfig (e.g. a fresh checkout that hasn't
            // copied Secrets.xcconfig.example yet) is expected and should
            // degrade gracefully — sync is simply unavailable, not a crash.
            oauthHelper = nil
            syncClient = nil
            configError = "Sync isn't configured in this build. See apps/macos/README.md to set up Secrets.xcconfig."
        }
    }

    /// The "Enable Sync" action: ensures a device keypair exists, runs the
    /// interactive Google Sign-In flow, and calls `POST /v1/auth/google`.
    /// Also the re-entry point for an already-`waitingForApproval`/
    /// `waitingForGroupKey` device to check again immediately rather than
    /// wait for the next poll tick.
    func enableSync(presentationContextProvider: ASWebAuthenticationPresentationContextProviding) async {
        guard let oauthHelper, let syncClient else {
            setStatus(.error(configError ?? "Sync is not configured"))
            return
        }
        setStatus(.signingIn)
        do {
            let devicePublicKeyBase64 = try ensureDeviceKeypair()
            try await oauthHelper.authorize(presentationContextProvider: presentationContextProvider)
            let idToken = try await oauthHelper.validIdToken()
            let result = try await syncClient.authenticateGoogle(idToken: idToken, devicePublicKey: devicePublicKeyBase64)
            try await handle(result: result, devicePublicKeyBase64: devicePublicKeyBase64)
        } catch {
            // A deliberate cancel isn't a failure worth alarming the user
            // over — just drop back to the pre-sign-in state so "Enable
            // Sync" is immediately available again, rather than showing a
            // red error for closing a window on purpose.
            if case GoogleOAuthError.signInCancelled = error {
                setStatus(.notSignedIn)
            } else {
                setStatus(.error(userMessage(for: error)))
            }
        }
    }

    /// Ensures this device has a persisted keypair, generating one on first call if
    /// needed, and returns its public key — usable as a stable per-device identity for
    /// local row provenance (`meeting.device_id`, `capture_item.device_id`) even before
    /// the user ever enables sync. Purely local (Keychain + crypto), no network. Returns
    /// nil only if Keychain access itself fails (e.g. denied), which should be treated
    /// as "no stable identity available yet" by callers, not a crash.
    func ensureLocalDeviceIdentity() -> String? {
        try? ensureDeviceKeypair()
    }

    /// Attempts a fully silent re-check on app launch (and on each token renewal, see
    /// `renewIfDue`), if a device keypair
    /// already exists (meaning this device signed in before) — no
    /// interactive consent screen, since `GoogleOAuthHelper.validIdToken()`
    /// refreshes silently via the stored refresh token.
    func silentResumeIfPossible() async {
        guard let oauthHelper, let syncClient else {
            log.log("silentResumeIfPossible: no oauthHelper/syncClient (config error?)")
            return
        }
        let devicePublicKeyBase64: String
        do {
            guard let loaded = try loadExistingDevicePublicKey() else {
                log.log("silentResumeIfPossible: no existing device keypair in Keychain — nothing to resume")
                return
            }
            devicePublicKeyBase64 = loaded
        } catch {
            log.log("silentResumeIfPossible: reading device keypair from Keychain threw \(String(describing: error), privacy: .public)")
            return
        }
        do {
            let idToken = try await oauthHelper.validIdToken()
            let result = try await syncClient.authenticateGoogle(idToken: idToken, devicePublicKey: devicePublicKeyBase64)
            // `handle` starts the waiting poll itself for the pending states.
            try await handle(result: result, devicePublicKeyBase64: devicePublicKeyBase64)
            log.log("silentResumeIfPossible: succeeded, status=\(String(describing: self.status), privacy: .public)")
        } catch {
            // Silent resume failing (e.g. GoogleOAuthError.notAuthorized —
            // never actually signed in, only a stray keypair on disk) just
            // means the user sees the normal "not signed in" state; not a
            // user-facing error.
            log.log("silentResumeIfPossible: failed with \(String(describing: error), privacy: .public)")
            // A renewal that the Worker refuses outright (device revoked, email no
            // longer allowed) leaves the old token to expire; say so instead of
            // showing "ready" while every sync round fails.
            if case .ready = status, isPermanentFailure(error) {
                setStatus(.error(userMessage(for: error)))
            }
        }
    }

    /// Renews the Turso token before it expires. Content sync calls this before each
    /// round, and with `afterFailure` after a failed one: the core reports only
    /// success or failure, so a failed round is the only hint the token was refused
    /// early (a revoke rotates every device's token).
    func renewIfDue(afterFailure: Bool = false) async {
        guard case .ready = status else { return }
        let now = Date()
        guard SyncTiming.renewalDue(now: now, lastAuthAt: lastAuthAt, lastAttemptAt: lastRenewAttemptAt, afterFailure: afterFailure) else { return }
        lastRenewAttemptAt = now
        log.log("renewIfDue: renewing the sync token (afterFailure=\(afterFailure, privacy: .public))")
        await silentResumeIfPossible()
    }

    private func handle(result: GoogleAuthResult, devicePublicKeyBase64: String) async throws {
        switch result {
        case .pending:
            setStatus(.waitingForApproval)
            startWaitingPollIfNeeded(devicePublicKeyBase64: devicePublicKeyBase64)
        case let .approved(tursoToken, tursoDatabaseUrl, wrappedGroupKey):
            try await resolveGroupKey(wrappedGroupKey: wrappedGroupKey, devicePublicKeyBase64: devicePublicKeyBase64)
            if try SyncKeychain.loadGroupKey() != nil {
                setStatus(.ready(tursoToken: tursoToken, tursoDatabaseUrl: tursoDatabaseUrl))
                // Also starts the retry floor, so a sync round failing right after
                // sign-in doesn't immediately re-authenticate.
                lastAuthAt = Date()
                lastRenewAttemptAt = lastAuthAt
                // Only cancel the waiting poll now that we're fully done —
                // nothing below this point awaits, so cancelling here (even
                // when this call is itself running inside that very poll
                // task) can't cause a spurious cancellation error the way
                // cancelling BEFORE the resolveGroupKey/listDevices await
                // above did.
                waitingPollTask?.cancel()
                waitingPollTask = nil
            } else {
                setStatus(.waitingForGroupKey)
                startWaitingPollIfNeeded(devicePublicKeyBase64: devicePublicKeyBase64)
            }
        }
    }

    /// Resolves this device's group key from an `authenticateGoogle`
    /// result.
    /// - `wrappedGroupKey` present: unwrap and persist.
    /// - `wrappedGroupKey` nil, we already hold a local group key: no-op,
    ///   routine re-auth.
    /// - `wrappedGroupKey` nil, no local key: ambiguous between "we are
    ///   the account's first device" (safe to generate our own key) and
    ///   "we are an Nth device whose handoff TTL expired before we could
    ///   consume it" (must NOT generate a divergent key). Resolved by
    ///   checking whether any OTHER already-`approved` device exists on
    ///   the account: if yes, the TTL expired — leave no key stored so
    ///   the caller ends up in `.waitingForGroupKey`; if no other
    ///   approved device exists, we truly are first, safe to generate.
    private func resolveGroupKey(wrappedGroupKey: String?, devicePublicKeyBase64: String) async throws {
        let localKeyHeld = try SyncKeychain.loadGroupKey() != nil
        // The decision table lives in core/api::sync_ops::enrollment (M3.0 Task 11);
        // the "is there another approved device" question is only asked when it
        // matters, as before.
        switch CoreMemoryStore.syncGroupKeyDecision(wrappedKeyPresent: wrappedGroupKey != nil, localKeyHeld: localKeyHeld, anotherApprovedDeviceExists: false) {
        case .unwrap:
            try await unwrapAndPersist(wrappedGroupKey: wrappedGroupKey ?? "", devicePublicKeyBase64: devicePublicKeyBase64)
        case .keep, .waitForHandoff:
            return
        case .generate:
            guard let syncClient, let oauthHelper else { return }
            let idToken = try await oauthHelper.validIdToken()
            let otherDevices = try await syncClient.listDevices(idToken: idToken, devicePublicKey: devicePublicKeyBase64)
            let anotherApprovedDeviceExists = otherDevices.contains { $0.publicKey != devicePublicKeyBase64 && $0.status == "approved" }
            guard CoreMemoryStore.syncGroupKeyDecision(wrappedKeyPresent: false, localKeyHeld: false, anotherApprovedDeviceExists: anotherApprovedDeviceExists) == .generate else {
                return
            }
            guard let groupKey = CoreMemoryStore.generateSyncGroupKey() else {
                throw SyncStateError.cryptoFailed
            }
            try SyncKeychain.saveGroupKey(groupKey)
        }
    }

    private func unwrapAndPersist(wrappedGroupKey: String, devicePublicKeyBase64: String) async throws {
        guard let syncClient, let oauthHelper else { return }
        let deviceSecretKeyBase64 = try loadOrThrowDeviceSecretKey()
        let idToken = try await oauthHelper.validIdToken()
        let candidates = try await syncClient.listDevices(idToken: idToken, devicePublicKey: devicePublicKeyBase64)
        for candidate in candidates where candidate.publicKey != devicePublicKeyBase64 && candidate.status == "approved" {
            if let groupKey = CoreMemoryStore.unwrapSyncGroupKey(
                recipientSecretKeyBase64: deviceSecretKeyBase64,
                senderPublicKeyBase64: candidate.publicKey,
                wrappedGroupKeyBase64: wrappedGroupKey
            ) {
                try SyncKeychain.saveGroupKey(groupKey)
                return
            }
        }
        // None of the current approved devices' public keys successfully
        // unwrapped it — leave no group key stored; the caller sees
        // `.waitingForGroupKey` and the polling loop will retry.
    }

    /// The Devices panel's "Approve" action: wraps this device's group
    /// key for `device`'s public key and pushes it via
    /// `POST /v1/devices/:id/approve`.
    func approve(_ device: SyncDeviceRecord) async {
        guard let oauthHelper, let syncClient else { return }
        actionError = nil
        do {
            let devicePublicKeyBase64 = try loadOrThrowDevicePublicKey()
            let deviceSecretKeyBase64 = try loadOrThrowDeviceSecretKey()
            let groupKeyBase64 = try loadOrThrowGroupKey()
            guard let wrapped = CoreMemoryStore.wrapSyncGroupKey(
                senderSecretKeyBase64: deviceSecretKeyBase64,
                recipientPublicKeyBase64: device.publicKey,
                groupKeyBase64: groupKeyBase64
            ) else {
                actionError = "Failed to wrap the group key for the new device."
                return
            }
            let idToken = try await oauthHelper.validIdToken()
            try await syncClient.approveDevice(idToken: idToken, devicePublicKey: devicePublicKeyBase64, targetDeviceId: device.id, wrappedGroupKey: wrapped)
            await refreshDevices()
        } catch {
            // Deliberately not `status = .error(...)` here — a failure
            // approving/re-sending a key to one peer shouldn't blow away
            // this device's own working `.ready` state and device list.
            actionError = userMessage(for: error)
        }
    }

    /// The Devices panel's "Revoke" action: removes `device`'s access via
    /// `POST /v1/devices/:id/revoke`. Works on this device's own row too
    /// (self-revocation) — no special-casing, matching the design's
    /// explicit decision.
    func revoke(_ device: SyncDeviceRecord) async {
        guard let oauthHelper, let syncClient else { return }
        actionError = nil
        do {
            let devicePublicKeyBase64 = try loadOrThrowDevicePublicKey()
            let idToken = try await oauthHelper.validIdToken()
            try await syncClient.revoke(idToken: idToken, devicePublicKey: devicePublicKeyBase64, targetDeviceId: device.id)
            await refreshDevices()
        } catch {
            // Deliberately not `status = .error(...)` — matches `approve(_:)`'s
            // own reasoning: a failure revoking one peer shouldn't blow away
            // this device's own working `.ready` state and device list.
            actionError = userMessage(for: error)
        }
    }

    /// The Devices panel's "Save" action for this device's own name.
    /// Pushes `deviceName`'s current value via `POST /v1/devices/name`,
    /// then refreshes `devices` so the Devices panel (and, indirectly,
    /// Timeline's device-attribution labels) pick up the change without
    /// waiting for the next 5-second poll tick.
    func saveDeviceName() async {
        guard let oauthHelper, let syncClient else { return }
        let trimmed = deviceName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        actionError = nil
        do {
            let devicePublicKeyBase64 = try loadOrThrowDevicePublicKey()
            let idToken = try await oauthHelper.validIdToken()
            try await syncClient.setDeviceName(idToken: idToken, devicePublicKey: devicePublicKeyBase64, name: trimmed)
            await refreshDevices()
        } catch {
            actionError = userMessage(for: error)
        }
    }

    func startDevicesPolling() {
        stopDevicesPolling()
        devicesPollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshDevices()
                try? await Task.sleep(nanoseconds: Self.devicesPollIntervalSeconds * 1_000_000_000)
            }
        }
    }

    func stopDevicesPolling() {
        devicesPollTask?.cancel()
        devicesPollTask = nil
    }

    private func refreshDevices() async {
        guard let oauthHelper, let syncClient else { return }
        do {
            let devicePublicKeyBase64 = try loadOrThrowDevicePublicKey()
            let idToken = try await oauthHelper.validIdToken()
            let fetched = try await syncClient.listDevices(idToken: idToken, devicePublicKey: devicePublicKeyBase64)
            setDevices(fetched)
        } catch {
            // A transient poll failure shouldn't blow away the last-known
            // device list or surface as a hard error — just skip this tick.
        }
    }

    /// Polls `POST /v1/auth/google` while this device is `waitingForApproval`/
    /// `waitingForGroupKey`. Backs off (doubling, capped at
    /// `waitingPollMaxIntervalSeconds`) after each transient failure and
    /// resets to the baseline interval after any successful tick, so a
    /// prolonged outage doesn't hammer the Worker at the base cadence
    /// indefinitely.
    private func startWaitingPollIfNeeded(devicePublicKeyBase64: String) {
        guard waitingPollTask == nil else { return }
        waitingPollTask = Task { [weak self] in
            guard let self else { return }
            var intervalSeconds = Self.waitingPollIntervalSeconds
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: intervalSeconds * 1_000_000_000)
                guard !Task.isCancelled, let oauthHelper = self.oauthHelper, let syncClient = self.syncClient else { return }
                do {
                    let idToken = try await oauthHelper.validIdToken()
                    let result = try await syncClient.authenticateGoogle(idToken: idToken, devicePublicKey: devicePublicKeyBase64)
                    try await self.handle(result: result, devicePublicKeyBase64: devicePublicKeyBase64)
                    if case .ready = self.status {
                        return
                    }
                    intervalSeconds = Self.waitingPollIntervalSeconds
                } catch {
                    if self.isPermanentFailure(error) {
                        self.setStatus(.error(self.userMessage(for: error)))
                        self.waitingPollTask = nil
                        return
                    }
                    // transient failure (network blip, 502, etc.) — keep
                    // polling, back off instead of hammering, don't surface
                    // an error on every missed tick
                    intervalSeconds = min(intervalSeconds * 2, Self.waitingPollMaxIntervalSeconds)
                }
            }
        }
    }

    /// `@Published` fires `objectWillChange` on every assignment regardless
    /// of equality, and `AppState` merges `syncState.objectWillChange` into
    /// its own app-wide publisher — so an unguarded `status =`/`devices =`
    /// on every poll tick (every 3-5s while a device is pending, or the
    /// Devices panel is open) would re-render the whole app that often,
    /// Settings open or not. Guarding on equality here keeps polling from
    /// being observable at all when nothing actually changed.
    private func setStatus(_ newValue: SyncStatus) {
        guard status != newValue else { return }
        status = newValue
    }

    private func setDevices(_ newValue: [SyncDeviceRecord]) {
        guard devices != newValue else { return }
        devices = newValue
    }

    /// 401/403 are permanent for this device/account (bad token, email
    /// not allowed, device revoked) — retrying won't help, so polling
    /// should stop rather than hammering the server forever.
    private func isPermanentFailure(_ error: Error) -> Bool {
        guard let apiError = error as? SyncAPIError else { return false }
        return CoreMemoryStore.syncWorkerErrorIsPermanent(apiError)
    }

    private func userMessage(for error: Error) -> String {
        // Worker failures are worded by core/api::sync_ops::enrollment so every shell
        // says the same thing; the OAuth/Keychain cases below describe Swift types.
        if let apiError = error as? SyncAPIError, let message = CoreMemoryStore.syncWorkerErrorMessage(apiError) {
            return message
        }
        switch error {
        case GoogleOAuthError.notAuthorized, GoogleOAuthError.refreshTokenRevoked:
            return "You're not signed in. Click Enable Sync to sign in."
        case GoogleOAuthError.signInCancelled:
            return "Sign-in was cancelled."
        case GoogleOAuthError.timedOut:
            return "Sign-in timed out. Try again."
        case let GoogleOAuthError.authorizationDenied(reason):
            return "Google sign-in was denied (\(reason))."
        case GoogleOAuthError.tokenExchangeFailed, GoogleOAuthError.tokenRefreshFailed:
            return "Couldn't complete Google sign-in. Try again."
        case GoogleOAuthError.noRefreshTokenReturned, GoogleOAuthError.noIdTokenReturned:
            return "Google didn't return the credentials sync needs. Try signing in again."
        case GoogleOAuthError.keychainReadFailed, GoogleOAuthError.keychainWriteFailed:
            return "Couldn't access the Keychain. Check Keychain access for Whimbrel and try again."
        case GoogleOAuthError.loopbackListenerFailed:
            return "Couldn't start the local sign-in listener. Try again."
        case GoogleOAuthError.randomGenerationFailed:
            return "A system security check failed. Try again."
        case GoogleOAuthError.stateMismatch:
            return "Sign-in response didn't match this request. Try again."
        case SyncStateError.cryptoFailed:
            return "A local encryption operation failed. Try again."
        case SyncStateError.noDeviceKeypair, SyncStateError.noGroupKey:
            return "This device isn't fully set up for sync yet. Try Enable Sync again."
        case SyncKeychainError.readFailed, SyncKeychainError.writeFailed:
            return "Couldn't access the Keychain. Check Keychain access for Whimbrel and try again."
        case is URLError:
            return "Couldn't reach the sync server. Check your connection and try again."
        default:
            return "Something went wrong. Try again."
        }
    }

    private func ensureDeviceKeypair() throws -> String {
        if let existing = try loadExistingDevicePublicKey() {
            return existing
        }
        guard let generated = CoreMemoryStore.generateSyncDeviceKeypair() else {
            throw SyncStateError.cryptoFailed
        }
        try SyncKeychain.saveDeviceSecretKey(generated.secretKeyBase64)
        return generated.publicKeyBase64
    }

    private func loadExistingDevicePublicKey() throws -> String? {
        guard let secretKeyBase64 = try SyncKeychain.loadDeviceSecretKey() else {
            return nil
        }
        return CoreMemoryStore.syncDevicePublicKey(secretKeyBase64: secretKeyBase64)
    }

    private func loadOrThrowDevicePublicKey() throws -> String {
        guard let publicKey = try loadExistingDevicePublicKey() else {
            throw SyncStateError.noDeviceKeypair
        }
        return publicKey
    }

    private func loadOrThrowDeviceSecretKey() throws -> String {
        guard let secretKey = try SyncKeychain.loadDeviceSecretKey() else {
            throw SyncStateError.noDeviceKeypair
        }
        return secretKey
    }

    private func loadOrThrowGroupKey() throws -> String {
        guard let groupKey = try SyncKeychain.loadGroupKey() else {
            throw SyncStateError.noGroupKey
        }
        return groupKey
    }

    /// The current device's own public key, exposed for UI use (e.g.
    /// excluding this device's own row from device-management actions).
    /// `nil` if no device keypair exists yet.
    var currentDevicePublicKeyBase64: String? {
        try? loadExistingDevicePublicKey()
    }
}

enum SyncStateError: Error {
    case cryptoFailed
    case noDeviceKeypair
    case noGroupKey
}
