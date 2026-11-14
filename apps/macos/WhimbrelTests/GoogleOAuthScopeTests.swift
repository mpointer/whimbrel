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

    // MARK: Google verification scopes (2026-11-12)

    private let gmailReadonly = "https://www.googleapis.com/auth/gmail.readonly"
    private let gmailSend = "https://www.googleapis.com/auth/gmail.send"
    private let gmailCompose = "https://www.googleapis.com/auth/gmail.compose"

    func testScopeStringsPerMode() {
        XCTAssertEqual(GoogleConnectorScopes.calendar(customClient: false), "openid email \(calendarEvents)")
        XCTAssertEqual(GoogleConnectorScopes.calendar(customClient: true), "openid email \(calendarEvents)")
        XCTAssertEqual(GoogleConnectorScopes.gmail(customClient: false), "openid email \(gmailSend)", "the public client never asks to read")
        XCTAssertEqual(GoogleConnectorScopes.gmail(customClient: true), "openid email \(gmailReadonly) \(gmailSend)")
        for scope in [GoogleConnectorScopes.calendar(customClient: false), GoogleConnectorScopes.gmail(customClient: false)] {
            XCTAssertFalse(scope.contains("readonly"), scope)
            XCTAssertFalse(scope.contains("compose"), scope)
        }
    }

    func testReadCapabilityComesFromTheGrantedScopes() {
        XCTAssertTrue(GmailReadAccess.canRead(grantedScope: "openid email \(gmailReadonly) \(gmailCompose)"), "old full grant reads")
        XCTAssertTrue(GmailReadAccess.canRead(grantedScope: nil), "a legacy token with no recorded grant reads until a refresh records it")
        XCTAssertFalse(GmailReadAccess.canRead(grantedScope: "openid email \(gmailSend)"), "send-only grant")
        XCTAssertTrue(GmailReadAccess.canRead(grantedScope: "openid email \(gmailReadonly) \(gmailSend)"), "own-project grant")
        XCTAssertEqual(GoogleStoredGrant.unrecorded.grantedScope, nil)
        XCTAssertEqual(GoogleStoredGrant.granted("x").grantedScope, "x")
    }

    func testPollingSkipsSendOnlyAccounts() {
        let grants = ["reader@x.com": "openid email \(gmailReadonly) \(gmailSend)", "sender@x.com": "openid email \(gmailSend)"]
        let toSync = GmailPollingService.accountsToSync(["reader@x.com", "sender@x.com", "legacy@x.com"]) {
            GmailReadAccess.canRead(grantedScope: grants[$0])
        }
        XCTAssertEqual(toSync, ["reader@x.com", "legacy@x.com"])
    }

    func testReadToolsAreHiddenForSendOnlyAccounts() {
        let all = ["send_email", "search_threads", "get_thread", "create_draft", "sync_messages"]
        XCTAssertEqual(GmailReadAccess.offeredTools(all, grantedScope: "openid email \(gmailSend)"), ["send_email"])
        XCTAssertEqual(
            GmailReadAccess.offeredTools(all, grantedScope: "openid email \(gmailReadonly) \(gmailSend)"),
            ["send_email", "search_threads", "get_thread", "sync_messages"], "own project: reads, but makes no drafts"
        )
        XCTAssertEqual(GmailReadAccess.offeredTools(all, grantedScope: "openid email \(gmailReadonly) \(gmailCompose)"), all, "old full grant")
        XCTAssertEqual(GmailReadAccess.offeredTools(all, grantedScope: nil), all, "legacy unrecorded grant")
    }

    func testSettingOrClearingTheCustomClientFlagsAccountsForReconnect() {
        // No issuer record: the built-in client issued the tokens.
        XCTAssertTrue(GoogleOAuthHelper.issuer(recorded: nil, legacy: "builtin", matches: "builtin"))
        XCTAssertFalse(GoogleOAuthHelper.issuer(recorded: nil, legacy: "builtin", matches: "mine"), "setting a custom client")
        XCTAssertFalse(GoogleOAuthHelper.issuer(recorded: "mine", legacy: "builtin", matches: "other"), "changing it")
        XCTAssertFalse(GoogleOAuthHelper.issuer(recorded: "mine", legacy: "builtin", matches: "builtin"), "clearing it")
        XCTAssertTrue(GoogleOAuthHelper.issuer(recorded: "mine", legacy: "builtin", matches: "mine"))

        let account = "test-connector:\(UUID().uuidString)"
        let builtIn = GoogleOAuthHelper(clientId: "builtin", clientSecret: "s", scope: "openid", keychainAccount: account, legacyIssuerClientId: "builtin")
        let custom = GoogleOAuthHelper(clientId: "mine", clientSecret: "s", scope: "openid", keychainAccount: account, legacyIssuerClientId: "builtin")
        XCTAssertTrue(builtIn.tokensIssuedByThisClient)
        XCTAssertFalse(custom.tokensIssuedByThisClient, "a legacy account must sign in again under the custom client")
        XCTAssertEqual(custom.storedGrant(), .noTokens)
    }

    func testCustomClientStoreRoundTripsInTheKeychain() {
        let store = CustomGoogleClientStore(service: "whimbrel-google-oauth-test-\(UUID().uuidString)")
        XCTAssertNil(store.load())
        XCTAssertFalse(store.save(clientId: "id.apps.googleusercontent.com", clientSecret: "  "), "both fields are required")
        XCTAssertTrue(store.save(clientId: " id.apps.googleusercontent.com\n", clientSecret: "shh"))
        XCTAssertEqual(store.load(), GoogleOAuthClient(clientId: "id.apps.googleusercontent.com", clientSecret: "shh", isCustom: true))
        XCTAssertEqual(GoogleOAuthClient.active(store: store)?.isCustom, true)
        store.clear()
        XCTAssertNil(store.load())
        XCTAssertNotEqual(GoogleOAuthClient.active(store: store)?.isCustom, true)
    }

    func testSavingTheSameCustomClientIsANoOp() {
        let saved = GoogleOAuthClient(clientId: "id", clientSecret: "shh", isCustom: true)
        XCTAssertFalse(ConnectorsSettingsPane.canSaveCustomClient(id: "id", secret: "shh", saved: saved))
        XCTAssertFalse(ConnectorsSettingsPane.canSaveCustomClient(id: "id", secret: "", saved: nil))
        XCTAssertTrue(ConnectorsSettingsPane.canSaveCustomClient(id: "id", secret: "new", saved: saved))
        XCTAssertTrue(ConnectorsSettingsPane.canSaveCustomClient(id: "id", secret: "shh", saved: nil))
    }

    func testCustomClientSecretNeverAppearsInLogs() throws {
        let secret = "GOCSPX-never-log-this"
        let clientId = "123-never-log.apps.googleusercontent.com"
        let client = GoogleOAuthClient(clientId: clientId, clientSecret: secret, isCustom: true)
        var dumped = ""
        dump(client, to: &dumped)
        for rendered in [String(describing: client), String(reflecting: client), "\(client)", dumped, String(describing: Optional(client))] {
            XCTAssertFalse(rendered.contains(secret), rendered)
            XCTAssertFalse(rendered.contains(clientId), rendered)
        }

        // No log call anywhere in the app interpolates a client ID or secret.
        let sources = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel")
        guard let files = FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil) else {
            return XCTFail("cannot enumerate \(sources.path)")
        }
        let logCalls = ["log.", "logger.", "Logger(", "print(", "NSLog(", "os_log("]
        let values = ["clientSecret", "clientId", "ClientSecret", "ClientId", "customClient"]
        var offenders: [String] = []
        for case let url as URL in files where url.pathExtension == "swift" {
            for line in try String(contentsOf: url, encoding: .utf8).split(separator: "\n")
            where logCalls.contains(where: { line.contains($0) }) && values.contains(where: { line.contains($0) }) {
                offenders.append("\(url.lastPathComponent): \(line.trimmingCharacters(in: .whitespaces))")
            }
        }
        XCTAssertEqual(offenders, [], "never log an OAuth client ID or secret")
    }
}
