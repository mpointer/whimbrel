import XCTest
@testable import Whimbrel

/// Smoke tests for the scope and JWT wrappers over core/api::oauth; the full case set
/// (hierarchy, whitespace, multi-scope) is `core/api/src/oauth.rs`'s tests.
final class GoogleOAuthScopeTests: XCTestCase {
    private let calendarEvents = "https://www.googleapis.com/auth/calendar.events"
    private let calendarReadonly = "https://www.googleapis.com/auth/calendar.readonly"
    private let calendarFull = "https://www.googleapis.com/auth/calendar"

    func testScopeCheckThroughTheBridge() {
        XCTAssertTrue(GoogleOAuthHelper.grantedScope(calendarEvents, satisfies: calendarEvents))
        XCTAssertTrue(GoogleOAuthHelper.grantedScope(calendarFull, satisfies: calendarEvents), "broader parent scope")
        XCTAssertFalse(GoogleOAuthHelper.grantedScope(calendarReadonly, satisfies: calendarEvents), "stale readonly-only grant")
        XCTAssertFalse(GoogleOAuthHelper.grantedScope(nil, satisfies: calendarEvents), "unknown grant is never sufficient")
    }

    private func makeJwt(payloadJSON: String) -> String {
        func base64url(_ string: String) -> String {
            Data(string.utf8).base64EncodedString()
                .replacingOccurrences(of: "+", with: "-")
                .replacingOccurrences(of: "/", with: "_")
                .replacingOccurrences(of: "=", with: "")
        }
        return "\(base64url("{\"alg\":\"RS256\"}")).\(base64url(payloadJSON)).signature"
    }

    func testDecodeJwtEmailClaimThroughTheBridge() {
        XCTAssertEqual(GoogleOAuthHelper.decodeJwtEmailClaim(makeJwt(payloadJSON: "{\"email\":\"alice@genesys.com\",\"sub\":\"123\"}")), "alice@genesys.com")
        XCTAssertNil(GoogleOAuthHelper.decodeJwtEmailClaim(makeJwt(payloadJSON: "{\"sub\":\"123\"}")))
        XCTAssertNil(GoogleOAuthHelper.decodeJwtEmailClaim("not-a-real-jwt"))
    }

    func testBeginBindsLoopbackAndCancelEndsTheWait() {
        guard let session = CoreMemoryStore.oauthBegin(config: CoreOAuthConfig(provider: "google", clientId: "cid", clientSecret: "sec", scope: "openid email")) else {
            return XCTFail("begin returned nil")
        }
        XCTAssertTrue(session.redirectURI.hasPrefix("http://127.0.0.1:"))
        XCTAssertEqual(session.authURL.host, "accounts.google.com")
        XCTAssertTrue(CoreMemoryStore.oauthCancel(sessionId: session.sessionId))
        XCTAssertEqual(CoreMemoryStore.oauthWait(sessionId: session.sessionId, timeoutMs: 5_000, nowUnixMs: 0), .failure(.cancelled))
    }
}
