import XCTest
@testable import Whimbrel

/// Smoke tests for `MicrosoftOAuthHelper.decodeAccountIdentifier` over core/api::oauth's
/// JWT claims; the full case set is `core/api/src/oauth.rs`'s tests.
final class MicrosoftOAuthHelperTests: XCTestCase {
    private func makeJwt(payloadJSON: String) -> String {
        func base64url(_ string: String) -> String {
            Data(string.utf8).base64EncodedString()
                .replacingOccurrences(of: "+", with: "-")
                .replacingOccurrences(of: "/", with: "_")
                .replacingOccurrences(of: "=", with: "")
        }
        return "\(base64url("{\"alg\":\"RS256\"}")).\(base64url(payloadJSON)).signature"
    }

    func testDecodeAccountIdentifierPrefersEmailThenPreferredUsername() {
        XCTAssertEqual(MicrosoftOAuthHelper.decodeAccountIdentifier(makeJwt(payloadJSON: "{\"email\":\"alice@outlook.com\",\"preferred_username\":\"alice@contoso.com\"}")), "alice@outlook.com")
        XCTAssertEqual(MicrosoftOAuthHelper.decodeAccountIdentifier(makeJwt(payloadJSON: "{\"preferred_username\":\"alice@contoso.com\",\"sub\":\"123\"}")), "alice@contoso.com")
        XCTAssertNil(MicrosoftOAuthHelper.decodeAccountIdentifier("not-a-real-jwt"))
    }

    func testMicrosoftBeginAdvertisesLocalhostRedirect() {
        guard let session = CoreMemoryStore.oauthBegin(config: CoreOAuthConfig(provider: "microsoft", clientId: "cid", clientSecret: nil, scope: "openid email")) else {
            return XCTFail("begin returned nil")
        }
        XCTAssertTrue(session.redirectURI.hasPrefix("http://localhost:"))
        CoreMemoryStore.oauthCancel(sessionId: session.sessionId)
        _ = CoreMemoryStore.oauthWait(sessionId: session.sessionId, timeoutMs: 1_000, nowUnixMs: 0)
    }
}
