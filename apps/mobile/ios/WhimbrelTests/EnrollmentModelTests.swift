import Security
import XCTest
@testable import Whimbrel

/// Scripted stand-in for Google sign-in and the sync Worker. Crypto (core FFI) and the
/// Keychain (in-memory under XCTest, see KeychainAccess) are real.
final class FakeBackend: EnrollmentBackend {
    var signInError: Error?
    /// Answers to successive `authenticate` calls; each gets the caller's public key.
    var authReplies: [(String) throws -> GoogleAuthResult] = []
    var devices: [SyncDeviceRecord] = []
    private(set) var signInCalls = 0
    private(set) var authCalls = 0

    func signIn() async throws {
        signInCalls += 1
        if let signInError { throw signInError }
    }

    /// Thrown by `idToken()` while set (e.g. `GoogleSignInError.notSignedIn`).
    var idTokenError: Error?

    /// Like URLSession: `idToken()` and `revokeDevice` throw when called from a cancelled task.
    var throwsWhenCancelled = false

    func idToken() async throws -> String {
        if throwsWhenCancelled, Task.isCancelled { throw CancellationError() }
        if let idTokenError { throw idTokenError }
        return "id-token"
    }

    /// This `authenticate` call (1-based) blocks until its task is cancelled and then still
    /// answers, like the real non-cancellable network call.
    var blockingAuthCall: Int?
    /// If set, the blocking call throws this (once cancelled) instead of answering.
    var blockingAuthError: Error?

    func authenticate(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
        authCalls += 1
        if authCalls == blockingAuthCall {
            while !Task.isCancelled { try? await Task.sleep(nanoseconds: 5_000_000) }
            if let blockingAuthError { throw blockingAuthError }
            return .pending
        }
        guard !authReplies.isEmpty else { return .pending }
        return try authReplies.removeFirst()(devicePublicKey)
    }

    private(set) var listCalls = 0
    func listDevices(idToken: String, devicePublicKey: String) async throws -> [SyncDeviceRecord] { listCalls += 1; return devices }

    /// While set, `revokeDevice` throws it (offline).
    var revokeError: Error?
    private(set) var signOutCalls = 0
    private(set) var deviceNames: [String] = []
    func setDeviceName(idToken: String, devicePublicKey: String, name: String) async throws { deviceNames.append(name) }
    func signOut() { signOutCalls += 1 }
    private(set) var approvals: [(targetDeviceId: String, wrappedGroupKey: String)] = []
    func approve(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) async throws {
        approvals.append((targetDeviceId, wrappedGroupKey))
    }
    private(set) var swapTargets: [String] = []
    func swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) async throws { swapTargets.append(replaceDeviceId) }
    private(set) var waitlistWants: [[String]] = []
    func joinWaitlist(idToken: String, devicePublicKey: String, wants: [String]) async throws { waitlistWants.append(wants) }
    private(set) var revokedTargets: [String] = []
    func revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) async throws {
        if throwsWhenCancelled, Task.isCancelled { throw CancellationError() }
        if let revokeError { throw revokeError }
        revokedTargets.append(targetDeviceId)
    }
}

/// Records poll sleeps and stops the loop (by throwing, like a cancelled Task.sleep)
/// after `limit` of them.
final class FakeSleeper {
    var intervals: [UInt64] = []
    var limit = 10
    func sleep(_ seconds: UInt64) async throws {
        intervals.append(seconds)
        if intervals.count > limit { throw CancellationError() }
    }
}

private func device(_ publicKey: String, status: String = "approved") -> SyncDeviceRecord {
    SyncDeviceRecord(id: "mac", accountId: "acct", publicKey: publicKey, name: "Mac", platform: "macos", status: status, enrolledAtUnixMs: 0, revokedAtUnixMs: nil)
}

@MainActor
final class EnrollmentModelTests: XCTestCase {
    var backend: FakeBackend!
    var sleeper: FakeSleeper!

    override func setUp() async throws {
        backend = FakeBackend()
        sleeper = FakeSleeper()
        // The in-memory Keychain is process-wide; start each test with no keys.
        let service = (Bundle.main.bundleIdentifier ?? "com.mpointer.kestrel") + ".sync"
        for account in ["device-secret-key", "group-key"] {
            KeychainAccess.delete([kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: account] as CFDictionary)
        }
    }

    private func makeModel() -> EnrollmentModel {
        let sleeper = sleeper!
        return EnrollmentModel(backend: backend, sleep: { try await sleeper.sleep($0) })
    }

    private func storedPublicKey() throws -> String {
        let secret = try XCTUnwrap(try SyncKeychain.loadDeviceSecretKey())
        return try XCTUnwrap(CoreMemoryStore.syncDevicePublicKey(secretKeyBase64: secret))
    }

    func testPollIntervalDoublesOnFailureCapsAndResetsOnSuccess() {
        var interval = EnrollmentModel.basePollSeconds
        var seen: [UInt64] = []
        for _ in 0..<5 {
            interval = EnrollmentModel.nextPollInterval(current: interval, succeeded: false)
            seen.append(interval)
        }
        XCTAssertEqual(seen, [6, 12, 24, 30, 30])
        XCTAssertEqual(EnrollmentModel.nextPollInterval(current: 30, succeeded: true), 3)
    }

    func testClientIdValidation() {
        XCTAssertFalse(EnrollmentConfig.isUsableClientId(""))
        XCTAssertFalse(EnrollmentConfig.isUsableClientId("REPLACE_ME"))
        XCTAssertFalse(EnrollmentConfig.isUsableClientId("your-client-id"))
        XCTAssertTrue(EnrollmentConfig.isUsableClientId("123-abc.apps.googleusercontent.com"))
        XCTAssertEqual(EnrollmentConfig.redirectScheme(clientId: "123-abc.apps.googleusercontent.com"), "com.googleusercontent.apps.123-abc")
    }

    func testNotConfiguredNeverTouchesTheBackend() async {
        let model = EnrollmentModel(backend: nil, configError: "Google sign-in is not configured")
        XCTAssertEqual(model.status, .notConfigured("Google sign-in is not configured"))
        await model.signIn()
        XCTAssertEqual(model.status, .notConfigured("Google sign-in is not configured"))
    }

    func testSignInPendingShowsTheComparisonCodeOfTheStoredKey() async throws {
        sleeper.limit = 0
        let model = makeModel()
        await model.signIn()
        await model.pollTask?.value
        let code = try XCTUnwrap(CoreMemoryStore.deviceComparisonCode(publicKeyBase64: try storedPublicKey()))
        XCTAssertEqual(model.status, .pending(code: code))
        XCTAssertEqual(backend.signInCalls, 1)
    }

    func testCancelledSignInReturnsToSignedOut() async {
        backend.signInError = GoogleSignInError.cancelled
        let model = makeModel()
        await model.signIn()
        XCTAssertEqual(model.status, .signedOut)
    }

    func testApprovalUnwrapsAndStoresTheGroupKey() async throws {
        let mac = try XCTUnwrap(CoreMemoryStore.generateSyncDeviceKeypair())
        let groupKey = try XCTUnwrap(CoreMemoryStore.generateSyncGroupKey())
        backend.devices = [device(mac.publicKeyBase64)]
        backend.authReplies = [
            { _ in .pending },
            { phoneKey in
                let wrapped = CoreMemoryStore.wrapSyncGroupKey(senderSecretKeyBase64: mac.secretKeyBase64, recipientPublicKeyBase64: phoneKey, groupKeyBase64: groupKey)
                return .approved(tursoToken: "t", tursoDatabaseUrl: "u", wrappedGroupKey: wrapped)
            },
        ]
        let model = makeModel()
        await model.signIn()
        await model.pollTask?.value
        XCTAssertEqual(model.status, .approved)
        XCTAssertEqual(try SyncKeychain.loadGroupKey(), groupKey)
        XCTAssertEqual(sleeper.intervals, [3])
        // Named while pending (for the Mac's approve prompt) and again once approved.
        XCTAssertEqual(backend.deviceNames.count, 2)
        XCTAssertFalse(backend.deviceNames[0].isEmpty)
    }

    func testApprovedWithoutHandoffWaitsForGroupKeyAndNeverGeneratesOne() async throws {
        sleeper.limit = 0
        let mac = try XCTUnwrap(CoreMemoryStore.generateSyncDeviceKeypair())
        backend.devices = [device(mac.publicKeyBase64)]
        backend.authReplies = [{ _ in .approved(tursoToken: "t", tursoDatabaseUrl: "u", wrappedGroupKey: nil) }]
        let model = makeModel()
        await model.signIn()
        await model.pollTask?.value
        let code = try XCTUnwrap(CoreMemoryStore.deviceComparisonCode(publicKeyBase64: try storedPublicKey()))
        XCTAssertEqual(model.status, .waitingForGroupKey(code: code))
        XCTAssertNil(try SyncKeychain.loadGroupKey())
    }

    /// The phone never creates the account's group key: with no approved desktop on the
    /// account (core says "generate"), it asks for the Mac to be set up first.
    func testNoApprovedDesktopNeedsDesktopFirstAndNeverGeneratesAKey() async throws {
        backend.devices = []
        backend.authReplies = [{ _ in .approved(tursoToken: "t", tursoDatabaseUrl: "u", wrappedGroupKey: nil) }]
        let model = makeModel()
        await model.signIn()
        await model.pollTask?.value
        XCTAssertEqual(model.status, .needsDesktopFirst)
        XCTAssertNil(try SyncKeychain.loadGroupKey())
        XCTAssertNil(model.pollTask)
    }

    // MARK: M4.0 hardening: the phone cannot strand the account

    /// "Approved, no key, no other desktop": the server lists this phone as approved.
    private func approvedNoKeyReply() -> (String) throws -> GoogleAuthResult {
        { [unowned self] phoneKey in
            backend.devices = [SyncDeviceRecord(id: "phone-id", accountId: "acct", publicKey: phoneKey, name: "iPhone", platform: "ios", status: "approved", enrolledAtUnixMs: 0, revokedAtUnixMs: nil)]
            return .approved(tursoToken: "t", tursoDatabaseUrl: "u", wrappedGroupKey: nil)
        }
    }

    /// The Worker refuses an iOS first device (409 desktop_required): nothing was enrolled,
    /// so there is nothing to revoke and the device key stays.
    func testDesktopRequiredAnswerNeedsDesktopFirstWithoutRevoking() async throws {
        backend.authReplies = [{ _ in throw SyncAPIError.httpError(status: 409, body: "{\"error\":\"desktop_required\"}") }]
        let model = makeModel()
        await model.signIn()
        XCTAssertEqual(model.status, .needsDesktopFirst)
        XCTAssertNotNil(try SyncKeychain.loadDeviceSecretKey())
        XCTAssertTrue(backend.revokedTargets.isEmpty)
    }

    func testOtherConflictsAreNotDesktopRequired() async {
        backend.authReplies = [{ _ in throw SyncAPIError.httpError(status: 409, body: "{\"error\":\"other\"}") }]
        let model = makeModel()
        await model.signIn()
        guard case .error = model.status else { return XCTFail("expected error, got \(model.status)") }
    }

    /// Approved with no key and no other approved desktop: revoke this phone's own row, then
    /// discard its device key, so the phone never sits approved-but-keyless.
    func testApprovedWithNoDesktopRevokesItselfAndDiscardsTheDeviceKey() async throws {
        backend.authReplies = [approvedNoKeyReply()]
        let model = makeModel()
        await model.signIn()
        XCTAssertEqual(model.status, .needsDesktopFirst)
        XCTAssertEqual(backend.revokedTargets, ["phone-id"])
        XCTAssertNil(try SyncKeychain.loadDeviceSecretKey())
    }

    /// Keyless approval discovered by the waiting poll: the revoke must not run in the
    /// poll's (cancelled) task.
    func testSelfRevokeFromThePollPathRunsUncancelled() async throws {
        backend.throwsWhenCancelled = true
        backend.authReplies = [{ _ in .pending }, approvedNoKeyReply()]
        let model = makeModel()
        await model.signIn()
        await model.pollTask?.value
        XCTAssertEqual(model.status, .needsDesktopFirst)
        XCTAssertEqual(backend.revokedTargets, ["phone-id"])
        XCTAssertNil(try SyncKeychain.loadDeviceSecretKey())
    }

    /// Own row not found in the device list: nothing was revoked, so the key must stay.
    func testSelfRevokeWithoutAnOwnDeviceRowKeepsTheKey() async throws {
        backend.devices = []
        backend.authReplies = [{ _ in .approved(tursoToken: "t", tursoDatabaseUrl: "u", wrappedGroupKey: nil) }]
        let model = makeModel()
        await model.signIn()
        XCTAssertEqual(model.status, .needsDesktopFirst)
        XCTAssertNotNil(try SyncKeychain.loadDeviceSecretKey())
        XCTAssertTrue(backend.revokedTargets.isEmpty)
    }

    func testFailedSelfRevokeKeepsTheKeyAndTheNextSignInRetriesBeforeEnrolling() async throws {
        backend.authReplies = [approvedNoKeyReply()]
        backend.revokeError = URLError(.notConnectedToInternet)
        let model = makeModel()
        await model.signIn()
        XCTAssertEqual(model.status, .needsDesktopFirst)
        let keptKey = try storedPublicKey()
        XCTAssertTrue(backend.revokedTargets.isEmpty)

        backend.revokeError = nil
        backend.authReplies = [{ _ in .pending }]
        sleeper.limit = 0
        await model.signIn()
        await model.pollTask?.value
        XCTAssertEqual(backend.revokedTargets, ["phone-id"], "retried before enrolling")
        XCTAssertNotEqual(try storedPublicKey(), keptKey, "enrolled with a fresh keypair")
    }

    func testSignInAfterSelfRevokeGeneratesAFreshKeypair() async throws {
        backend.authReplies = [approvedNoKeyReply()]
        let model = makeModel()
        await model.signIn()
        let oldKey = try XCTUnwrap(backend.devices.first?.publicKey)
        sleeper.limit = 0
        backend.authReplies = [{ _ in .pending }]
        await model.signIn()
        await model.pollTask?.value
        XCTAssertNotEqual(try storedPublicKey(), oldKey)
    }

    // MARK: M4.0 hardening: a revoked phone wipes itself

    private static let revoked = SyncAPIError.httpError(status: 403, body: "{\"error\":\"device revoked\"}")
    private var wipes = 0

    private func makeWipingModel() -> EnrollmentModel {
        let model = makeModel()
        model.onWipe = { [unowned self] in wipes += 1 }
        return model
    }

    private func assertWiped(_ model: EnrollmentModel, file: StaticString = #filePath, line: UInt = #line) throws {
        XCTAssertEqual(model.status, .removed, file: file, line: line)
        XCTAssertNil(try SyncKeychain.loadGroupKey(), file: file, line: line)
        XCTAssertNil(try SyncKeychain.loadDeviceSecretKey(), file: file, line: line)
        XCTAssertEqual(backend.signOutCalls, 1, "Google tokens deleted", file: file, line: line)
        XCTAssertEqual(wipes, 1, "local store wiped once", file: file, line: line)
    }

    func testRevokedOnResumeFromApprovedWipes() async throws {
        try seedApprovedPhone()
        backend.authReplies = [{ _ in throw Self.revoked }]
        let model = makeWipingModel()
        await model.resume()
        try assertWiped(model)
        XCTAssertNil(model.syncCredentials())
    }

    func testRevokedWhilePendingPollWipes() async throws {
        backend.authReplies = [{ _ in .pending }, { _ in throw Self.revoked }]
        let model = makeWipingModel()
        await model.signIn()
        await model.pollTask?.value
        try assertWiped(model)
        XCTAssertNil(model.pollTask)
    }

    func testRevokedWhileWaitingForGroupKeyWipes() async throws {
        let mac = try XCTUnwrap(CoreMemoryStore.generateSyncDeviceKeypair())
        backend.devices = [device(mac.publicKeyBase64)]
        backend.authReplies = [{ _ in .approved(tursoToken: "t", tursoDatabaseUrl: "u", wrappedGroupKey: nil) }, { _ in throw Self.revoked }]
        let model = makeWipingModel()
        await model.signIn()
        guard case .waitingForGroupKey = model.status else { return XCTFail("got \(model.status)") }
        await model.pollTask?.value
        try assertWiped(model)
    }

    func testOtherPermanentErrorsDoNotWipe() async throws {
        for failure: Error in [GoogleSignInError.notSignedIn, SyncAPIError.httpError(status: 401, body: ""), SyncAPIError.httpError(status: 403, body: "email not allowed")] {
            wipes = 0
            try seedApprovedPhone()
            backend.authReplies = [{ _ in throw failure }]
            let model = makeWipingModel()
            await model.resume()
            guard case .error = model.status else { return XCTFail("got \(model.status)") }
            XCTAssertNotNil(try SyncKeychain.loadGroupKey())
            XCTAssertNotNil(try SyncKeychain.loadDeviceSecretKey())
            XCTAssertEqual(wipes, 0)
            XCTAssertEqual(backend.signOutCalls, 0)
        }
    }

    func testASecondRevokedAnswerDoesNotWipeAgain() async throws {
        let key = try seedApprovedPhone()
        backend.authReplies = [{ _ in throw Self.revoked }]
        let model = makeWipingModel()
        await model.resume()
        model.wipeRevoked(for: key)
        try assertWiped(model)
    }

    /// A revoked answer for an older keypair (a request in flight across the wipe and a new
    /// Sign in) must not wipe the new one.
    /// Fails toward wiping: with no readable device key there is nothing to be stale against.
    func testRevokedAnswerWithNoReadableDeviceKeyStillWipes() async throws {
        try seedApprovedPhone()
        let model = makeWipingModel()
        try SyncKeychain.deleteDeviceSecretKey()
        model.wipeRevoked(for: "any-key")
        XCTAssertEqual(model.status, .removed)
        XCTAssertEqual(wipes, 1)
    }

    func testAStaleRevokedAnswerDoesNotWipeANewKeypair() async throws {
        try seedApprovedPhone()
        let model = makeWipingModel()
        model.wipeRevoked(for: "an-older-public-key")
        XCTAssertEqual(model.status, .approved)
        XCTAssertNotNil(try SyncKeychain.loadDeviceSecretKey())
        XCTAssertEqual(wipes, 0)
    }

    func testSignInAfterTheWipeMakesANewKeypairAndComesUpPending() async throws {
        sleeper.limit = 1
        backend.authReplies = [{ _ in .pending }, { _ in throw Self.revoked }]
        let model = makeWipingModel()
        await model.signIn()
        let oldKey = try storedPublicKey()
        await model.pollTask?.value
        XCTAssertEqual(model.status, .removed)
        backend.authReplies = [{ _ in .pending }]
        await model.signIn()
        await model.pollTask?.value
        guard case let .pending(code) = model.status else { return XCTFail("got \(model.status)") }
        XCTAssertNotEqual(try storedPublicKey(), oldKey)
        XCTAssertNotEqual(code, CoreMemoryStore.deviceComparisonCode(publicKeyBase64: oldKey))
    }

    func testFirstLaunchCleanupRunsOnceAndSetsTheMarker() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "whimbrel.test.\(UUID().uuidString)"))
        try seedApprovedPhone()
        var signOuts = 0
        EnrollmentConfig.clearStaleSecretsOnFirstLaunch(defaults: defaults) { signOuts += 1 }
        XCTAssertNil(try SyncKeychain.loadGroupKey())
        XCTAssertNil(try SyncKeychain.loadDeviceSecretKey())
        XCTAssertEqual(signOuts, 1)
        try seedApprovedPhone()
        EnrollmentConfig.clearStaleSecretsOnFirstLaunch(defaults: defaults) { signOuts += 1 }
        XCTAssertNotNil(try SyncKeychain.loadGroupKey(), "marker present: nothing is touched")
        XCTAssertEqual(signOuts, 1)
    }

    func testGoogleSignOutDeletesItsKeychainTokens() {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                    kSecAttrService as String: (Bundle.main.bundleIdentifier ?? "com.mpointer.whimbrel.mobile") + ".google-oauth",
                                    kSecAttrAccount as String: "google-sync"]
        var add = query
        add[kSecValueData as String] = Data("x".utf8)
        XCTAssertEqual(KeychainAccess.add(add as CFDictionary, nil), errSecSuccess)
        GoogleSignIn(clientId: "x.apps.googleusercontent.com").signOut()
        var result: AnyObject?
        XCTAssertEqual(KeychainAccess.copyMatching(query as CFDictionary, &result), errSecItemNotFound)
    }

    func testWrappedKeyFromANonApprovedSenderStoresNothing() async throws {
        sleeper.limit = 0
        let stranger = try XCTUnwrap(CoreMemoryStore.generateSyncDeviceKeypair())
        let groupKey = try XCTUnwrap(CoreMemoryStore.generateSyncGroupKey())
        backend.devices = [device(stranger.publicKeyBase64, status: "pending")]
        backend.authReplies = [{ phoneKey in
            let wrapped = CoreMemoryStore.wrapSyncGroupKey(senderSecretKeyBase64: stranger.secretKeyBase64, recipientPublicKeyBase64: phoneKey, groupKeyBase64: groupKey)
            return .approved(tursoToken: "t", tursoDatabaseUrl: "u", wrappedGroupKey: wrapped)
        }]
        let model = makeModel()
        await model.signIn()
        await model.pollTask?.value
        let code = try XCTUnwrap(CoreMemoryStore.deviceComparisonCode(publicKeyBase64: try storedPublicKey()))
        XCTAssertEqual(model.status, .waitingForGroupKey(code: code))
        XCTAssertNil(try SyncKeychain.loadGroupKey())
    }

    func testPermanentFailureWhilePollingStopsWithAnError() async {
        backend.authReplies = [
            { _ in .pending },
            { _ in throw SyncAPIError.httpError(status: 403, body: "") },
        ]
        let model = makeModel()
        await model.signIn()
        await model.pollTask?.value
        guard case .error = model.status else { return XCTFail("expected error, got \(model.status)") }
        XCTAssertEqual(sleeper.intervals, [3])
    }

    func testTransientFailureBacksOffThenResets() async {
        sleeper.limit = 3
        backend.authReplies = [
            { _ in .pending },
            { _ in throw SyncAPIError.httpError(status: 502, body: "") },
            { _ in .pending },
        ]
        let model = makeModel()
        await model.signIn()
        await model.pollTask?.value
        XCTAssertEqual(sleeper.intervals, [3, 6, 3, 3])
        guard case .pending = model.status else { return XCTFail("expected pending, got \(model.status)") }
    }

    func testResumeWithoutAKeypairStaysSignedOut() async {
        let model = makeModel()
        await model.resume()
        XCTAssertEqual(model.status, .signedOut)
        XCTAssertEqual(backend.authCalls, 0)
    }

    func testResumeWithAKeypairPicksUpPending() async throws {
        sleeper.limit = 0
        let pair = try XCTUnwrap(CoreMemoryStore.generateSyncDeviceKeypair())
        try SyncKeychain.saveDeviceSecretKey(pair.secretKeyBase64)
        let model = makeModel()
        await model.resume()
        await model.pollTask?.value
        XCTAssertEqual(model.status, .pending(code: try XCTUnwrap(CoreMemoryStore.deviceComparisonCode(publicKeyBase64: pair.publicKeyBase64))))
        XCTAssertEqual(backend.signInCalls, 0)
    }

    // MARK: 6c: offline launch, credentials, renewal, background

    /// Saves a device keypair and a group key, as after an earlier approval.
    @discardableResult
    private func seedApprovedPhone() throws -> String {
        let pair = try XCTUnwrap(CoreMemoryStore.generateSyncDeviceKeypair())
        try SyncKeychain.saveDeviceSecretKey(pair.secretKeyBase64)
        try SyncKeychain.saveGroupKey(try XCTUnwrap(CoreMemoryStore.generateSyncGroupKey()))
        return pair.publicKeyBase64
    }

    func testLaunchWithAGroupKeyStartsApprovedEvenOffline() async throws {
        try seedApprovedPhone()
        backend.authReplies = [{ _ in throw URLError(.notConnectedToInternet) }]
        let model = makeModel()
        XCTAssertEqual(model.status, .approved)
        await model.resume()
        XCTAssertEqual(model.status, .approved)
        XCTAssertNil(model.syncCredentials(), "no Turso token until the Worker answers")
    }

    func testApprovedResumeKeepsTheTursoTokenInMemoryForSync() async throws {
        let publicKey = try seedApprovedPhone()
        backend.authReplies = [{ _ in .approved(tursoToken: "turso-token", tursoDatabaseUrl: "libsql://db", wrappedGroupKey: nil) }]
        let model = makeModel()
        await model.resume()
        let credentials = try XCTUnwrap(model.syncCredentials())
        XCTAssertEqual(credentials.tursoToken, "turso-token")
        XCTAssertEqual(credentials.tursoDatabaseUrl, "libsql://db")
        XCTAssertEqual(credentials.groupKeyBase64, try SyncKeychain.loadGroupKey())
        XCTAssertEqual(credentials.deviceId, publicKey, "the device id is the device public key, as on the Mac")
        XCTAssertFalse(String(describing: credentials).contains("turso-token"), "never printable")
        XCTAssertFalse(String(describing: credentials).contains("libsql://db"))
    }

    func testRevokedOnResumeWhileApprovedIsAnError() async throws {
        try seedApprovedPhone()
        backend.authReplies = [{ _ in throw SyncAPIError.httpError(status: 403, body: "") }]
        let model = makeModel()
        await model.resume()
        guard case .error = model.status else { return XCTFail("expected error, got \(model.status)") }
        XCTAssertNil(model.syncCredentials())
    }

    func testNotSignedInWhilePollingIsPermanent() async {
        let backend = backend!
        backend.authReplies = [{ _ in
            backend.idTokenError = GoogleSignInError.notSignedIn
            return .pending
        }]
        let model = makeModel()
        await model.signIn()
        await model.pollTask?.value
        XCTAssertEqual(model.status, .error("You're not signed in. Sign in with Google again."))
        XCTAssertEqual(sleeper.intervals, [3], "stops instead of backing off forever")
        XCTAssertNil(model.pollTask)
    }

    func testBackgroundStopsPollingAndForegroundResumesIt() async throws {
        // A poll sleep that only ends when cancelled.
        let model = EnrollmentModel(backend: backend, sleep: { _ in try await Task.sleep(nanoseconds: 3_600 * 1_000_000_000) })
        await model.signIn()
        guard case .pending = model.status else { return XCTFail("expected pending, got \(model.status)") }
        let poll = try XCTUnwrap(model.pollTask)
        model.enterBackground()
        XCTAssertNil(model.pollTask)
        guard poll.isCancelled else { return XCTFail("the poll must be cancelled") }  // else the await below hangs
        await poll.value
        XCTAssertEqual(backend.authCalls, 1, "no poll while in the background")

        await model.enterForeground()
        XCTAssertEqual(backend.authCalls, 2, "re-checks at once on foreground")
        XCTAssertNotNil(model.pollTask, "and polls again")
        model.enterBackground()
    }

    func testForegroundWhenApprovedDoesNotPoll() async throws {
        try seedApprovedPhone()
        let model = makeModel()
        await model.enterForeground()
        XCTAssertNil(model.pollTask)
        XCTAssertEqual(backend.authCalls, 0)
    }

    /// SyncTiming.renewalDue (shared with the Mac): renew at 12 h, or after a failed round, at
    /// most once per 5 minutes; with no token yet, as soon as the floor allows.
    func testRenewalDue() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        XCTAssertTrue(SyncTiming.renewalDue(now: now, lastAuthAt: nil, lastAttemptAt: nil, afterFailure: false))
        XCTAssertFalse(SyncTiming.renewalDue(now: now, lastAuthAt: nil, lastAttemptAt: now.addingTimeInterval(-60), afterFailure: false))
        XCTAssertTrue(SyncTiming.renewalDue(now: now, lastAuthAt: nil, lastAttemptAt: now.addingTimeInterval(-301), afterFailure: false))
        XCTAssertFalse(SyncTiming.renewalDue(now: now, lastAuthAt: now.addingTimeInterval(-3_600), lastAttemptAt: now.addingTimeInterval(-3_600), afterFailure: false))
        XCTAssertTrue(SyncTiming.renewalDue(now: now, lastAuthAt: now.addingTimeInterval(-3_600), lastAttemptAt: now.addingTimeInterval(-3_600), afterFailure: true))
        XCTAssertTrue(SyncTiming.renewalDue(now: now, lastAuthAt: now.addingTimeInterval(-12 * 3_600), lastAttemptAt: now.addingTimeInterval(-12 * 3_600), afterFailure: false))
    }

    func testRenewIfDueReauthenticatesOnceWithinTheFloor() async throws {
        try seedApprovedPhone()
        let reply: (String) throws -> GoogleAuthResult = { _ in .approved(tursoToken: "t", tursoDatabaseUrl: "u", wrappedGroupKey: nil) }
        backend.authReplies = [reply, reply, reply]
        let model = makeModel()
        await model.renewIfDue(afterFailure: false, force: false)
        XCTAssertEqual(backend.authCalls, 1, "no token yet: renew")
        await model.renewIfDue(afterFailure: true, force: false)
        XCTAssertEqual(backend.authCalls, 1, "inside the 5-minute floor")
        await model.renewIfDue(afterFailure: true, force: true)
        XCTAssertEqual(backend.authCalls, 2, "Sync now forces it")
    }

    /// A poll request in flight when the app goes to the background must not start a new
    /// poll when it returns (review fix: background leak).
    func testBackgroundDuringAnInFlightPollRequestStartsNoNewPoll() async throws {
        backend.blockingAuthCall = 2
        let model = makeModel()
        await model.signIn()
        let poll = try XCTUnwrap(model.pollTask)
        while backend.authCalls < 2 { await Task.yield() }
        model.enterBackground()
        await poll.value
        XCTAssertNil(model.pollTask, "no poll restarted by the late answer")
        XCTAssertEqual(backend.authCalls, 2)
        XCTAssertEqual(sleeper.intervals, [3], "no further poll ticks")
    }

    /// A permanent error thrown by a request that was cancelled meanwhile (app backgrounded)
    /// must not change state.
    func testPermanentErrorAfterCancellationDoesNotChangeStatus() async throws {
        backend.blockingAuthCall = 2
        backend.blockingAuthError = SyncAPIError.httpError(status: 403, body: "")
        let model = makeModel()
        await model.signIn()
        let before = model.status
        let poll = try XCTUnwrap(model.pollTask)
        while backend.authCalls < 2 { await Task.yield() }
        model.enterBackground()
        await poll.value
        XCTAssertEqual(model.status, before)
    }

    // MARK: Device limit, swap, waitlist

    func test_device_limit_for_phone_offers_swap_and_waitlist() async throws {
        sleeper.limit = 0
        let holder = LimitHolder(id: "old", name: "Mike's iPhone 14", platform: "ios")
        backend.authReplies = [{ _ in .deviceLimit(kind: "phone", holders: [holder]) }, { _ in .pending }]
        let model = makeModel()
        await model.signIn()
        XCTAssertEqual(model.status, .deviceLimit(kind: "phone", holders: [holder]))
        XCTAssertNil(model.pollTask, "nothing to poll for at the limit")
        XCTAssertEqual(EnrollmentModel.limitMessage(holders: [holder]),
                       "Whimbrel's free sync includes one computer and one phone. Mike's iPhone 14 is your synced phone.")
        XCTAssertEqual(EnrollmentModel.limitMessage(holders: []),
                       "Whimbrel's free sync includes one computer and one phone. Your other phone is your synced phone.")
        XCTAssertEqual(EnrollmentModel.waitlistOptions.map(\.want), ["more_devices", "routines_while_closed", "phone_notifications", "backups"])
        XCTAssertEqual(EnrollmentModel.waitlistOptions.map(\.label),
                       ["More devices", "Routines that run while my computer is closed", "Notifications on my phone", "Backups"])

        // Waitlist: sends the chosen wants, never swaps.
        let joined = await model.joinWaitlist(wants: ["backups", "more_devices"])
        XCTAssertTrue(joined)
        XCTAssertEqual(backend.waitlistWants, [["backups", "more_devices"]])

        // Swap: this phone becomes pending and waits for a computer (or the old phone).
        await model.swap(replacing: holder)
        XCTAssertEqual(backend.swapTargets, ["old"])
        guard case .pending = model.status else { return XCTFail("got \(model.status)") }
        XCTAssertTrue(model.swapRequested)
        model.enterBackground()
    }

    func test_two_syncs_within_5_minutes_list_devices_once() async throws {
        let pair = CoreMemoryStore.generateSyncDeviceKeypair()!
        try SyncKeychain.saveDeviceSecretKey(pair.secretKeyBase64)
        try SyncKeychain.saveGroupKey(CoreMemoryStore.generateSyncGroupKey()!)
        backend.devices = [device(pair.publicKeyBase64)]
        let model = makeModel()
        XCTAssertEqual(model.status, .approved)
        await model.refreshDeviceLabels()
        await model.refreshDeviceLabels()
        XCTAssertEqual(backend.listCalls, 1)
        await model.enterForeground() // becoming active refreshes at the next sync
        await model.refreshDeviceLabels()
        XCTAssertEqual(backend.listCalls, 2)
    }

    func test_revoked_phone_reenrolls_with_new_key() async throws {
        sleeper.limit = 0
        backend.authReplies = [{ _ in .pending }]
        let model = makeWipingModel()
        await model.signIn()
        let oldKey = try storedPublicKey()
        model.enterBackground()
        // A computer's fresh start (or a revoke) answers "device revoked" at the next resume.
        backend.authReplies = [{ _ in throw Self.revoked }]
        await model.resume()
        XCTAssertEqual(model.status, .removed)
        XCTAssertNil(try SyncKeychain.loadDeviceSecretKey(), "the old device keypair is deleted")
        backend.authReplies = [{ _ in .pending }]
        await model.signIn()
        XCTAssertNotEqual(try storedPublicKey(), oldKey)
        model.enterBackground()
    }
}
