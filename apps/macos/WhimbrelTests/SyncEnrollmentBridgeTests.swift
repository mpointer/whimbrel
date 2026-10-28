import XCTest
@testable import Whimbrel

/// Smoke tests for the CoreBridge wrappers over core/api::sync_ops::enrollment; the
/// Worker client and decision tables are Rust tests against a fake Worker.
final class SyncEnrollmentBridgeTests: XCTestCase {
    func testUnreachableWorkerSurfacesANetworkErrorWithTheSharedMessage() {
        switch CoreMemoryStore.syncWorkerAuthenticateGoogle(baseURL: "http://127.0.0.1:1", idToken: "id", devicePublicKey: "pk") {
        case .success: XCTFail("nothing listens on port 1")
        case .failure(let error):
            guard case .networkError = error else { return XCTFail("expected networkError, got \(error)") }
            XCTAssertFalse(CoreMemoryStore.syncWorkerErrorIsPermanent(error))
            XCTAssertEqual(CoreMemoryStore.syncWorkerErrorMessage(error), "Couldn't reach the sync server. Check your connection and try again.")
        }
    }

    func testPermanentFailuresAndMessagesThroughTheBridge() {
        let revoked = SyncAPIError.httpError(status: 403, body: "device revoked")
        XCTAssertTrue(CoreMemoryStore.syncWorkerErrorIsPermanent(revoked))
        XCTAssertEqual(CoreMemoryStore.syncWorkerErrorMessage(revoked), "This device's access was revoked. Sign in again from Settings to request access.")
        XCTAssertEqual(CoreMemoryStore.syncWorkerErrorMessage(.httpError(status: 502, body: "")), "Sync is temporarily unavailable (server error 502).")
        XCTAssertFalse(CoreMemoryStore.syncWorkerErrorIsPermanent(.malformedResponse))
    }

    func testGroupKeyDecisionTableThroughTheBridge() {
        XCTAssertEqual(CoreMemoryStore.syncGroupKeyDecision(wrappedKeyPresent: true, localKeyHeld: false, anotherApprovedDeviceExists: true), .unwrap)
        XCTAssertEqual(CoreMemoryStore.syncGroupKeyDecision(wrappedKeyPresent: false, localKeyHeld: true, anotherApprovedDeviceExists: true), .keep)
        XCTAssertEqual(CoreMemoryStore.syncGroupKeyDecision(wrappedKeyPresent: false, localKeyHeld: false, anotherApprovedDeviceExists: true), .waitForHandoff)
        XCTAssertEqual(CoreMemoryStore.syncGroupKeyDecision(wrappedKeyPresent: false, localKeyHeld: false, anotherApprovedDeviceExists: false), .generate)
    }

    @MainActor
    func testTokenRenewalIsDueAtHalfItsLifeOrAfterAFailureButNeverMoreThanEveryFiveMinutes() {
        let now = Date()
        let hours = { (h: Double) in now.addingTimeInterval(-h * 3600) }
        XCTAssertTrue(SyncTiming.renewalDue(now: now, lastAuthAt: nil, lastAttemptAt: nil, afterFailure: false))
        XCTAssertFalse(SyncTiming.renewalDue(now: now, lastAuthAt: hours(11), lastAttemptAt: nil, afterFailure: false))
        XCTAssertTrue(SyncTiming.renewalDue(now: now, lastAuthAt: hours(12), lastAttemptAt: nil, afterFailure: false))
        // A Mac asleep past the token's expiry renews on the first tick after waking.
        XCTAssertTrue(SyncTiming.renewalDue(now: now, lastAuthAt: hours(30), lastAttemptAt: hours(30), afterFailure: false))
        XCTAssertTrue(SyncTiming.renewalDue(now: now, lastAuthAt: hours(1), lastAttemptAt: nil, afterFailure: true))
        XCTAssertFalse(SyncTiming.renewalDue(now: now, lastAuthAt: hours(30), lastAttemptAt: hours(0.05), afterFailure: true))
        XCTAssertTrue(SyncTiming.renewalDue(now: now, lastAuthAt: hours(30), lastAttemptAt: hours(0.1), afterFailure: false))
    }
}
