import AuthenticationServices
import XCTest
@testable import Whimbrel

/// The limit, swap, waitlist, fresh-start and delete logic in `SyncState`, against fakes:
/// no network, no Keychain, no OAuth.
@MainActor
final class SyncAccountActionsTests: XCTestCase {
    /// Every side effect the flows make, in order.
    final class Recorder { var events: [String] = [] }

    final class FakeWorker: SyncWorker {
        let recorder: Recorder
        var authResults: [GoogleAuthResult] = []
        var resetResult: Result<GoogleAuthResult, Error> = .success(.approved(tursoToken: "t2", tursoDatabaseUrl: "libsql://new", wrappedGroupKey: nil))
        var devices: [SyncDeviceRecord] = []
        var sentWants: [String]?
        var swappedOut: String?
        var listError: Error?
        var deleteError: Error?
        /// While set, auth (and list) calls wait here until the test resumes them.
        var holdAuth = false
        var heldAuth: [CheckedContinuation<GoogleAuthResult, Never>] = []
        var holdList = false
        var heldList: [CheckedContinuation<Void, Never>] = []
        init(_ recorder: Recorder) { self.recorder = recorder }

        func authenticateGoogle(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
            recorder.events.append("auth")
            if holdAuth { return await withCheckedContinuation { heldAuth.append($0) } }
            return authResults.isEmpty ? .pending : authResults.removeFirst()
        }
        func listDevices(idToken: String, devicePublicKey: String) async throws -> [SyncDeviceRecord] {
            recorder.events.append("listDevices")
            if holdList { await withCheckedContinuation { heldList.append($0) } }
            if let listError { throw listError }
            return devices
        }
        func approveDevice(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) async throws {}
        func revoke(idToken: String, devicePublicKey: String, targetDeviceId: String) async throws {}
        func setDeviceName(idToken: String, devicePublicKey: String, name: String) async throws {}
        func swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) async throws {
            recorder.events.append("swap")
            swappedOut = replaceDeviceId
        }
        func joinWaitlist(idToken: String, devicePublicKey: String, wants: [String]) async throws {
            recorder.events.append("waitlist")
            sentWants = wants
        }
        func resetAccount(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
            recorder.events.append("reset")
            return try resetResult.get()
        }
        var onDelete: () -> Void = {}
        var holdDelete = false
        var heldDelete: [CheckedContinuation<Void, Never>] = []
        func deleteAccount(idToken: String, devicePublicKey: String) async throws {
            recorder.events.append("delete")
            onDelete()
            if holdDelete { await withCheckedContinuation { heldDelete.append($0) } }
            if let deleteError { throw deleteError }
        }
    }

    final class FakeSignIn: SyncSignIn {
        let recorder: Recorder
        init(_ recorder: Recorder) { self.recorder = recorder }
        /// The stored sign-in is dead until the next interactive sign-in.
        var revoked = false
        func authorize(presentationContextProvider: ASWebAuthenticationPresentationContextProviding) async throws {
            recorder.events.append("authorize")
            revoked = false
        }
        func validIdToken() async throws -> String {
            if revoked { throw GoogleOAuthError.refreshTokenRevoked }
            return "id-token"
        }
        func deleteStoredTokens() { recorder.events.append("deleteTokens") }
    }

    final class FakeKeys: SyncKeyStore {
        let recorder: Recorder
        var deviceSecretKey: String?
        var groupKey: String?
        init(_ recorder: Recorder) {
            self.recorder = recorder
            deviceSecretKey = CoreMemoryStore.generateSyncDeviceKeypair()?.secretKeyBase64
        }
        func loadDeviceSecretKey() throws -> String? { deviceSecretKey }
        func saveDeviceSecretKey(_ base64: String) throws { deviceSecretKey = base64 }
        func loadGroupKey() throws -> String? { groupKey }
        func saveGroupKey(_ base64: String) throws {
            recorder.events.append("saveGroupKey")
            groupKey = base64
        }
        func deleteGroupKey() throws {
            recorder.events.append("deleteGroupKey")
            groupKey = nil
        }
    }

    private let holder = LimitHolder(id: "old-mac", name: "Studio Mac", platform: "mac")
    private var recorder: Recorder!
    private var worker: FakeWorker!
    private var keys: FakeKeys!
    private var signIn: FakeSignIn!
    private var defaults: UserDefaults!
    private var state: SyncState!
    private var cursorResetSucceeds = true

    override func setUp() async throws {
        recorder = Recorder()
        worker = FakeWorker(recorder)
        keys = FakeKeys(recorder)
        signIn = FakeSignIn(recorder)
        defaults = UserDefaults(suiteName: "SyncAccountActionsTests-\(UUID().uuidString)")
        state = SyncState(signIn: signIn, worker: worker, keys: keys, defaults: defaults)
        cursorResetSucceeds = true
        wire(state)
    }

    private func wire(_ state: SyncState) {
        state.resetPushCursor = { [weak self] in
            self?.recorder.events.append("resetPushCursor")
            return self?.cursorResetSucceeds ?? false
        }
    }

    /// Waits (up to `seconds`) for `condition`, letting other tasks run.
    private func waitUntil(_ seconds: Double = 6, _ condition: () -> Bool) async {
        let deadline = Date().addingTimeInterval(seconds)
        while !condition(), Date() < deadline { try? await Task.sleep(nanoseconds: 20_000_000) }
    }

    override func tearDown() async throws {
        state.stopWaitingPoll()
    }

    private var ownPublicKey: String { state.currentDevicePublicKeyBase64 ?? "" }

    private func atTheLimit() async {
        worker.authResults = [.deviceLimit(kind: "computer", holders: [holder])]
        await state.enableSync(presentationContextProvider: WindowPresentationContextProvider())
    }

    func test_device_limit_offers_swap_and_waitlist() async {
        await atTheLimit()
        XCTAssertEqual(state.status, .deviceLimit(kind: "computer", holders: [holder]))
        XCTAssertEqual(SyncState.limitMessage(kind: "computer", holders: [holder]),
                       "Whimbrel's free sync includes one computer and one phone. Studio Mac is your synced computer.")
        XCTAssertEqual(SyncState.waitlistOptions.map(\.label),
                       ["More devices", "Routines that run while my computer is closed", "Notifications on my phone", "Backups"])
        XCTAssertEqual(SyncState.waitlistOptions.map(\.want), ["more_devices", "routines_while_closed", "phone_notifications", "backups"])
    }

    func test_swap_moves_to_pending() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        XCTAssertEqual(worker.swappedOut, "old-mac")
        XCTAssertEqual(state.status, .waitingForApproval)
        XCTAssertEqual(state.swapHolderName, "Studio Mac")
        XCTAssertEqual(SyncState.swapWaitingMessage(holderName: "Studio Mac"), "Approve this computer on Studio Mac or on your phone.")
        XCTAssertTrue(SyncState.freshStartWarning(holderName: "Studio Mac").contains("anything that was only on Studio Mac or only on the server will not come back"))
        // Survives a relaunch: the waiting screen still knows this is a swap.
        XCTAssertEqual(SyncState(signIn: signIn, worker: worker, keys: keys, defaults: defaults).swapHolderName, "Studio Mac")
    }

    func test_start_fresh_resets_key_and_cursor_in_order() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        keys.groupKey = "stale-key"
        worker.devices = [SyncDeviceRecord(id: "me", accountId: "a", publicKey: ownPublicKey, name: nil, platform: "mac", status: "approved", enrolledAtUnixMs: 1, revokedAtUnixMs: nil)]
        recorder.events = []

        await state.startFresh()

        XCTAssertEqual(recorder.events, ["reset", "deleteGroupKey", "resetPushCursor", "listDevices", "saveGroupKey"])
        XCTAssertEqual(state.status, .ready(tursoToken: "t2", tursoDatabaseUrl: "libsql://new"))
        XCTAssertNotEqual(keys.groupKey, "stale-key")
        XCTAssertNil(state.swapHolderName)
        XCTAssertNil(state.actionError)
    }

    func test_start_fresh_recovers_through_auth_after_an_ambiguous_failure() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        worker.resetResult = .failure(SyncAPIError.httpError(status: 502, body: ""))
        worker.authResults = [.approved(tursoToken: "t2", tursoDatabaseUrl: "libsql://new", wrappedGroupKey: nil)]
        recorder.events = []

        await state.startFresh()

        XCTAssertEqual(recorder.events, ["reset", "auth", "deleteGroupKey", "resetPushCursor", "listDevices", "saveGroupKey"])
        XCTAssertEqual(state.status, .ready(tursoToken: "t2", tursoDatabaseUrl: "libsql://new"))
    }

    func test_start_fresh_never_makes_a_key_while_another_device_is_approved() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        worker.resetResult = .failure(SyncAPIError.networkError(URLError(.timedOut)))
        worker.authResults = [.approved(tursoToken: "t", tursoDatabaseUrl: "libsql://old", wrappedGroupKey: nil)]
        worker.devices = [SyncDeviceRecord(id: "phone", accountId: "a", publicKey: "other", name: "iPhone", platform: "ios", status: "approved", enrolledAtUnixMs: 1, revokedAtUnixMs: nil)]

        await state.startFresh()

        XCTAssertFalse(recorder.events.contains("saveGroupKey"))
        XCTAssertEqual(state.status, .waitingForGroupKey)
    }

    func test_start_fresh_refused_names_the_active_device() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        worker.resetResult = .failure(SyncAPIError.httpError(status: 403, body: #"{"error":"other_device_active","devices":[{"id":"old-mac","name":"Studio Mac","platform":"mac"}]}"#))
        recorder.events = []

        await state.startFresh()

        XCTAssertEqual(recorder.events, ["reset"], "a refused reset touches nothing local")
        XCTAssertEqual(state.actionError, "Approve this computer on Studio Mac; it was active in the last week.")
        XCTAssertEqual(state.status, .waitingForApproval)
    }

    func test_delete_clears_credentials() async {
        worker.authResults = [.approved(tursoToken: "t", tursoDatabaseUrl: "libsql://x", wrappedGroupKey: nil)]
        await state.enableSync(presentationContextProvider: WindowPresentationContextProvider())
        guard case .ready = state.status else { return XCTFail("expected ready, got \(state.status)") }
        recorder.events = []

        await state.deleteSyncAccount()

        XCTAssertEqual(recorder.events, ["delete", "deleteGroupKey", "deleteTokens", "resetPushCursor"])
        XCTAssertNil(keys.groupKey)
        XCTAssertNotNil(keys.deviceSecretKey, "the device key is this Mac's identity for its own rows; it stays")
        XCTAssertEqual(state.status, .notSignedIn)
        XCTAssertTrue(state.devices.isEmpty)
    }

    func test_waitlist_sends_only_checked_wants() async {
        await atTheLimit()
        let joined = await state.joinWaitlist(wants: ["backups", "phone_notifications", "not_a_want"])
        XCTAssertTrue(joined)
        XCTAssertEqual(worker.sentWants, ["phone_notifications", "backups"])
        let none = await state.joinWaitlist(wants: [])
        XCTAssertFalse(none)
        XCTAssertEqual(recorder.events.filter { $0 == "waitlist" }.count, 1, "nothing checked sends nothing")
    }

    func test_poll_result_during_fresh_start_never_generates_a_key() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        // The poll's auth call is in flight when the fresh start begins.
        worker.holdAuth = true
        await waitUntil { !self.worker.heldAuth.isEmpty }
        XCTAssertFalse(worker.heldAuth.isEmpty, "the poll should have called auth")
        worker.holdList = true
        let fresh = Task { await self.state.startFresh() }
        await waitUntil { !self.worker.heldList.isEmpty }
        // The stale poll result arrives while the fresh start is on the first-device path.
        worker.heldAuth.forEach { $0.resume(returning: .approved(tursoToken: "t2", tursoDatabaseUrl: "libsql://new", wrappedGroupKey: nil)) }
        worker.heldAuth = []
        worker.holdAuth = false
        try? await Task.sleep(nanoseconds: 200_000_000)
        worker.holdList = false
        worker.heldList.forEach { $0.resume() }
        worker.heldList = []
        await fresh.value
        try? await Task.sleep(nanoseconds: 200_000_000)

        XCTAssertEqual(recorder.events.filter { $0 == "saveGroupKey" }.count, 1, "exactly one new group key")
        XCTAssertEqual(state.status, .ready(tursoToken: "t2", tursoDatabaseUrl: "libsql://new"))
    }

    func test_poll_result_while_a_fresh_start_is_flagged_does_nothing_even_uncancelled() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        worker.holdAuth = true
        await waitUntil { !self.worker.heldAuth.isEmpty }
        // The poll task is NOT cancelled; only the in-flight flag is set.
        state.freshStartInFlight = true
        recorder.events = []
        worker.heldAuth.forEach { $0.resume(returning: .approved(tursoToken: "t", tursoDatabaseUrl: "libsql://x", wrappedGroupKey: nil)) }
        worker.heldAuth = []
        worker.holdAuth = false
        try? await Task.sleep(nanoseconds: 300_000_000)
        XCTAssertEqual(recorder.events, [], "no handle: no device list, no key")
        XCTAssertEqual(state.status, .waitingForApproval)
        XCTAssertFalse(state.isWaitingPollRunning, "the stopped poll frees its slot")
        state.freshStartInFlight = false
    }

    func test_delayed_reset_approval_still_cleans_key_and_cursor() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        // The reset times out and auth still says pending: the Worker is still working on it.
        worker.resetResult = .failure(SyncAPIError.networkError(URLError(.timedOut)))
        worker.authResults = [.pending]
        await state.startFresh()
        XCTAssertEqual(state.status, .waitingForApproval)
        state.stopWaitingPoll()
        keys.groupKey = "stale-key"

        // Later, after a relaunch, auth says approved with no key.
        let relaunched = SyncState(signIn: signIn, worker: worker, keys: keys, defaults: defaults)
        wire(relaunched)
        worker.authResults = [.approved(tursoToken: "t2", tursoDatabaseUrl: "libsql://new", wrappedGroupKey: nil)]
        recorder.events = []
        await relaunched.silentResumeIfPossible()

        XCTAssertEqual(recorder.events, ["auth", "deleteGroupKey", "resetPushCursor", "listDevices", "saveGroupKey"])
        XCTAssertEqual(relaunched.status, .ready(tursoToken: "t2", tursoDatabaseUrl: "libsql://new"))
        // Done once: a routine re-auth later touches nothing.
        worker.authResults = [.approved(tursoToken: "t3", tursoDatabaseUrl: "libsql://new", wrappedGroupKey: nil)]
        recorder.events = []
        await relaunched.silentResumeIfPossible()
        XCTAssertEqual(recorder.events, ["auth"])
        relaunched.stopWaitingPoll()
    }

    func test_fresh_start_fails_when_the_cursor_reset_fails() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        cursorResetSucceeds = false
        await state.startFresh()
        XCTAssertEqual(state.status, .waitingForApproval, "never ready with the old cursor")
        XCTAssertNotNil(state.actionError)
        XCTAssertFalse(recorder.events.contains("saveGroupKey"))
    }

    func test_lost_delete_reply_never_recreates_the_account() async {
        worker.authResults = [.approved(tursoToken: "t", tursoDatabaseUrl: "libsql://x", wrappedGroupKey: nil)]
        await state.enableSync(presentationContextProvider: WindowPresentationContextProvider())
        worker.deleteError = SyncAPIError.networkError(URLError(.timedOut))
        worker.listError = SyncAPIError.networkError(URLError(.timedOut))

        await state.deleteSyncAccount()
        guard case .error = state.status else { return XCTFail("expected an error status, got \(state.status)") }

        // Relaunch: resume must probe, never call auth.
        let relaunched = SyncState(signIn: signIn, worker: worker, keys: keys, defaults: defaults)
        wire(relaunched)
        recorder.events = []
        await relaunched.silentResumeIfPossible()
        XCTAssertFalse(recorder.events.contains("auth"))
        await relaunched.enableSync(presentationContextProvider: WindowPresentationContextProvider())
        XCTAssertFalse(recorder.events.contains("auth"))

        // The probe finally hears the account is gone: the usual cleanup.
        worker.listError = SyncAPIError.httpError(status: 403, body: #"{"error":"device not approved"}"#)
        await relaunched.silentResumeIfPossible()
        XCTAssertFalse(recorder.events.contains("auth"))
        XCTAssertEqual(relaunched.status, .notSignedIn)
        XCTAssertNil(keys.groupKey)
        XCTAssertTrue(recorder.events.contains("deleteTokens"))

        // Settled: a later sign-in may call auth again.
        recorder.events = []
        await relaunched.enableSync(presentationContextProvider: WindowPresentationContextProvider())
        XCTAssertTrue(recorder.events.contains("auth"))
        relaunched.stopWaitingPoll()
    }

    func test_holder_fallback_reads_mid_sentence() {
        let unnamed = LimitHolder(id: "x", name: nil, platform: "mac")
        XCTAssertEqual(SyncState.limitMessage(kind: "computer", holders: [unnamed]),
                       "Whimbrel's free sync includes one computer and one phone. Your other computer is your synced computer.")
        XCTAssertTrue(SyncState.freshStartWarning(holderName: SyncState.holderDisplayName(unnamed, kind: "computer")).contains("only on your other computer or"))
    }

    func test_fresh_start_ending_waiting_for_group_key_keeps_polling() async {
        await atTheLimit()
        await state.swap(replacing: holder)
        worker.devices = [SyncDeviceRecord(id: "phone", accountId: "a", publicKey: "other", name: "iPhone", platform: "ios", status: "approved", enrolledAtUnixMs: 1, revokedAtUnixMs: nil)]
        await state.startFresh()
        XCTAssertEqual(state.status, .waitingForGroupKey)
        XCTAssertTrue(state.isWaitingPollRunning)
    }

    func test_pending_delete_with_revoked_sign_in_recovers_via_interactive_sign_in() async {
        worker.authResults = [.approved(tursoToken: "t", tursoDatabaseUrl: "libsql://x", wrappedGroupKey: nil)]
        await state.enableSync(presentationContextProvider: WindowPresentationContextProvider())
        worker.deleteError = SyncAPIError.networkError(URLError(.timedOut))
        worker.listError = SyncAPIError.networkError(URLError(.timedOut))
        await state.deleteSyncAccount()

        // The stored Google sign-in dies: resume can't probe, and asks for a sign-in.
        signIn.revoked = true
        let relaunched = SyncState(signIn: signIn, worker: worker, keys: keys, defaults: defaults)
        wire(relaunched)
        recorder.events = []
        await relaunched.silentResumeIfPossible()
        XCTAssertEqual(relaunched.status, .notSignedIn)
        XCTAssertFalse(recorder.events.contains("auth"))

        // Enable Sync signs in interactively, then probes with the fresh token: gone.
        worker.listError = SyncAPIError.httpError(status: 403, body: #"{"error":"device not approved"}"#)
        await relaunched.enableSync(presentationContextProvider: WindowPresentationContextProvider())
        XCTAssertEqual(Array(recorder.events.prefix(2)), ["authorize", "listDevices"], "sign in, then probe, before any auth")
        XCTAssertTrue(recorder.events.contains("auth"), "settled: enrolment goes on")
        XCTAssertFalse(recorder.events.contains("deleteTokens"), "the sign-in just made is kept")
        XCTAssertNil(keys.groupKey, "the old key went in the cleanup")
        XCTAssertFalse(defaults.bool(forKey: "com.mpointer.kestrel.syncDeletePending"))
        relaunched.stopWaitingPoll()
    }

    func test_pending_delete_without_a_keypair_settles_as_gone() async {
        defaults.set(true, forKey: "com.mpointer.kestrel.syncDeletePending")
        keys.groupKey = "old-key"
        keys.deviceSecretKey = nil
        let relaunched = SyncState(signIn: signIn, worker: worker, keys: keys, defaults: defaults)
        wire(relaunched)
        await relaunched.silentResumeIfPossible()
        XCTAssertNil(keys.groupKey)
        XCTAssertFalse(defaults.bool(forKey: "com.mpointer.kestrel.syncDeletePending"))
        XCTAssertFalse(recorder.events.contains("auth"))
        XCTAssertEqual(relaunched.status, .notSignedIn, "a new enrolment starts from Enable Sync")
    }

    func test_delete_pending_is_persisted_before_the_call_and_cleared_on_a_definite_answer() async {
        worker.authResults = [.approved(tursoToken: "t", tursoDatabaseUrl: "libsql://x", wrappedGroupKey: nil)]
        await state.enableSync(presentationContextProvider: WindowPresentationContextProvider())
        var flaggedDuringCall = false
        worker.onDelete = { [defaults] in flaggedDuringCall = defaults?.bool(forKey: "com.mpointer.kestrel.syncDeletePending") ?? false }
        worker.deleteError = SyncAPIError.httpError(status: 503, body: #"{"error":"account_deleting"}"#)
        await state.deleteSyncAccount()
        XCTAssertTrue(flaggedDuringCall)
        XCTAssertFalse(defaults.bool(forKey: "com.mpointer.kestrel.syncDeletePending"), "a definite refusal clears it")
        XCTAssertNotNil(state.actionError)
    }

    func test_only_the_fallback_holder_name_is_capitalised() {
        XCTAssertEqual(SyncState.holderAtSentenceStart("iMac"), "iMac")
        XCTAssertEqual(SyncState.holderAtSentenceStart("your other computer"), "Your other computer")
    }

    func test_renew_during_delete_cannot_clear_the_pending_flag() async {
        worker.authResults = [.approved(tursoToken: "t", tursoDatabaseUrl: "libsql://x", wrappedGroupKey: nil)]
        await state.enableSync(presentationContextProvider: WindowPresentationContextProvider())
        worker.holdDelete = true
        let deleting = Task { await self.state.deleteSyncAccount() }
        await waitUntil { !self.worker.heldDelete.isEmpty }
        recorder.events = []
        // A failed sync round renews mid-delete; the account still exists right now.
        await state.renewIfDue(afterFailure: true)
        await state.silentResumeIfPossible()
        XCTAssertEqual(recorder.events, [], "no probe, no auth while the delete is in flight")

        // The delete's reply is lost and the probe can't tell.
        worker.deleteError = SyncAPIError.networkError(URLError(.timedOut))
        worker.listError = SyncAPIError.networkError(URLError(.timedOut))
        worker.holdDelete = false
        worker.heldDelete.forEach { $0.resume() }
        worker.heldDelete = []
        await deleting.value
        XCTAssertTrue(defaults.bool(forKey: "com.mpointer.kestrel.syncDeletePending"))
    }

    func test_pending_delete_probe_refused_by_email_keeps_the_flag() async {
        defaults.set(true, forKey: "com.mpointer.kestrel.syncDeletePending")
        keys.groupKey = "key"
        worker.listError = SyncAPIError.httpError(status: 403, body: #"{"error":"email not allowed"}"#)
        let relaunched = SyncState(signIn: signIn, worker: worker, keys: keys, defaults: defaults)
        wire(relaunched)
        await relaunched.silentResumeIfPossible()
        XCTAssertEqual(relaunched.status, .error("This Google account isn't allowed to sign in to Whimbrel."))
        XCTAssertTrue(defaults.bool(forKey: "com.mpointer.kestrel.syncDeletePending"))
        XCTAssertEqual(keys.groupKey, "key", "not treated as gone")
        XCTAssertFalse(recorder.events.contains("auth"))
    }
}
