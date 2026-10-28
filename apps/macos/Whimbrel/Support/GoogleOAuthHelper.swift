import AuthenticationServices
import Foundation
import Security

/// Runs Google's OAuth 2.0 Authorization Code flow with PKCE via a loopback-listener
/// redirect (RFC 8252). PKCE, state, the loopback listener, the code exchange, refresh
/// and the scope/JWT helpers live in core/api::oauth (M3.0 Task 10); this class keeps
/// what is platform-bound: the `ASWebAuthenticationSession` browser hop and Keychain
/// storage of the returned tokens. Scope-parameterized so the same helper serves the
/// connectors and the sync feature's own Google Sign-In.
final class GoogleOAuthHelper: NSObject {
    /// Scoped to the running bundle so distinct app builds signed under the same Team ID
    /// (debug/QA/release) never share Keychain items via a fixed service string.
    static let keychainService = (Bundle.main.bundleIdentifier ?? "com.mpointer.kestrel") + ".google-oauth"
    private let clientId: String
    private let clientSecret: String
    private let scope: String
    private let keychainAccount: String

    /// `clientSecret` is required at the token-exchange/refresh steps even for a
    /// "Desktop app" OAuth client (confirmed live: Google's token endpoint rejects
    /// PKCE-only requests with `client_secret is missing`). Not a high-security value
    /// (it ships inside the app), but never hardcoded: supplied by the caller.
    init(clientId: String, clientSecret: String, scope: String, keychainAccount: String) {
        self.clientId = clientId
        self.clientSecret = clientSecret
        self.scope = scope
        self.keychainAccount = keychainAccount
    }

    private var config: CoreOAuthConfig {
        CoreOAuthConfig(provider: "google", clientId: clientId, clientSecret: clientSecret, scope: scope)
    }

    /// Runs the full loopback OAuth flow: the core binds the listener and builds the
    /// consent URL, `ASWebAuthenticationSession` shows it, the core catches the redirect
    /// and exchanges the code, and the tokens land in the Keychain.
    @MainActor
    func authorize(presentationContextProvider: ASWebAuthenticationPresentationContextProviding) async throws {
        guard let session = CoreMemoryStore.oauthBegin(config: config) else {
            throw GoogleOAuthError.loopbackListenerFailed
        }
        let cancellationBox = CancellationBox()
        switch await OAuthBrowserFlow.run(session: session, presentationContextProvider: presentationContextProvider, cancellationBox: cancellationBox) {
        case .success(let tokens):
            try storeTokensDirectly(tokens)
        case .failure(let error):
            if cancellationBox.wasCancelled {
                throw GoogleOAuthError.signInCancelled
            }
            throw GoogleOAuthError(coreError: error)
        }
    }

    /// Returns the stored access token, refreshing it first if it has expired.
    func validAccessToken() async throws -> String {
        try await validTokens().accessToken
    }

    /// Like `validAccessToken()`, but additionally verifies that this helper's own
    /// configured `scope` was actually GRANTED, throwing `.insufficientScope` if not.
    /// A refresh token minted under an older, narrower consent still refreshes into a
    /// valid access token; Google only rejects the write later, so the grant is checked
    /// up front to keep a drafted action retryable instead of destroyed. A token stored
    /// before the granted scope was recorded reads as `nil` (unknown): this forces one
    /// silent refresh, whose response carries the authoritative grant.
    func validAccessToken(requiringGrantedScope required: String) async throws -> String {
        var tokens = try await validTokens()
        if tokens.grantedScope == nil {
            tokens = try await refreshAndStore(tokens)
        }
        guard Self.grantedScope(tokens.grantedScope, satisfies: required) else {
            throw GoogleOAuthError.insufficientScope(required: required, granted: tokens.grantedScope ?? "")
        }
        return tokens.accessToken
    }

    /// Whether a granted scope string covers everything `required` asks for
    /// (core/api::oauth::granted_scope_satisfies: containment, honouring Google's
    /// scope hierarchy; `nil` never satisfies anything).
    static func grantedScope(_ granted: String?, satisfies required: String) -> Bool {
        CoreMemoryStore.oauthScopeSatisfies(granted: granted, required: required)
    }

    /// The `email` claim of an unverified ID token (read for account identity only).
    static func decodeJwtEmailClaim(_ jwt: String) -> String? {
        CoreMemoryStore.oauthJwtClaims(jwt)?["email"] as? String
    }

    /// Deletes this instance's own Keychain item. A missing item is not an error.
    func deleteStoredTokens() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.keychainService,
            kSecAttrAccount as String: keychainAccount,
        ]
        KeychainAccess.delete(query as CFDictionary)
    }

    /// Reads this instance's stored ID token directly, without validity/expiry checks;
    /// used only by the connector states' one-time migration to recover an old
    /// single-account connection's email.
    func storedIdTokenForMigration() throws -> String? {
        try readStoredTokens()?.idToken
    }

    /// Copies this instance's stored token blob verbatim to a new Keychain account.
    /// Returns whether the copy succeeded; the caller deletes the source afterward.
    func copyStoredTokens(toKeychainAccount newAccount: String) -> Bool {
        guard let stored = try? readStoredTokens() else { return false }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.keychainService,
            kSecAttrAccount as String: newAccount,
        ]
        guard let data = try? JSONEncoder().encode(stored) else { return false }
        return KeychainAccess.upsert(query, data: data) == errSecSuccess
    }

    /// Returns the stored Google ID token (what the sync Worker's `POST /v1/auth/google`
    /// wants), refreshing it first if it has expired.
    func validIdToken() async throws -> String {
        let tokens = try await validTokens()
        guard let idToken = tokens.idToken else {
            throw GoogleOAuthError.noIdTokenReturned
        }
        return idToken
    }

    private func validTokens() async throws -> OAuthStoredTokens {
        guard let stored = try readStoredTokens() else {
            throw GoogleOAuthError.notAuthorized
        }
        if stored.isFresh(nowUnixMs: Int64(Date().timeIntervalSince1970 * 1000)) {
            return stored
        }
        return try await refreshAndStore(stored)
    }

    /// The refresh grant runs in the core off the main actor; the merged blob (refresh
    /// token, ID token and scope kept unless the response replaced them) is persisted.
    private func refreshAndStore(_ stored: OAuthStoredTokens) async throws -> OAuthStoredTokens {
        let config = self.config
        let result = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.oauthRefresh(config: config, stored: stored, nowUnixMs: Int64(Date().timeIntervalSince1970 * 1000))
        }.value
        switch result {
        case .success(let refreshed):
            try storeTokensDirectly(refreshed)
            return refreshed
        case .failure(let error):
            throw GoogleOAuthError(coreError: error)
        }
    }

    private func storeTokensDirectly(_ stored: OAuthStoredTokens) throws {
        let data = try JSONEncoder().encode(stored)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.keychainService,
            kSecAttrAccount as String: keychainAccount,
        ]
        let status = KeychainAccess.upsert(query, data: data)
        guard status == errSecSuccess else {
            throw GoogleOAuthError.keychainWriteFailed(status)
        }
    }

    private func readStoredTokens() throws -> OAuthStoredTokens? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.keychainService,
            kSecAttrAccount as String: keychainAccount,
            kSecReturnData as String: true,
        ]
        var result: AnyObject?
        let status = KeychainAccess.copyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess, let data = result as? Data else {
            throw GoogleOAuthError.keychainReadFailed(status)
        }
        return try JSONDecoder().decode(OAuthStoredTokens.self, from: data)
    }
}

enum GoogleOAuthError: Error {
    case notAuthorized
    case tokenExchangeFailed
    case tokenRefreshFailed
    /// Google rejected the stored refresh token (revoked or expired); only reconnecting fixes it.
    case refreshTokenRevoked
    case noRefreshTokenReturned
    case noIdTokenReturned
    case keychainWriteFailed(OSStatus)
    case keychainReadFailed(OSStatus)
    case loopbackListenerFailed
    case randomGenerationFailed(OSStatus)
    case stateMismatch
    case timedOut
    case authorizationDenied(String)
    case signInCancelled
    /// The stored grant is valid but narrower than what this operation needs. Recoverable
    /// only by reconnecting, never by retrying with the same stored token.
    case insufficientScope(required: String, granted: String)
}

extension GoogleOAuthError {
    /// Maps the shared loopback mechanism's generic failure into this provider's own
    /// error case set 1:1, so every existing exhaustive switch over `GoogleOAuthError`
    /// (e.g. SyncState's user-facing message mapping) needs no changes.
    init(loopbackError: OAuthLoopbackError) {
        switch loopbackError {
        case .loopbackListenerFailed: self = .loopbackListenerFailed
        case .randomGenerationFailed(let status): self = .randomGenerationFailed(status)
        case .stateMismatch: self = .stateMismatch
        case .timedOut: self = .timedOut
        case .authorizationDenied(let reason): self = .authorizationDenied(reason)
        }
    }

    init(coreError: CoreOAuthError) {
        switch coreError {
        case .listenerFailed, .unknownSession, .unknownProvider, .cancelled: self = .loopbackListenerFailed
        case .stateMismatch: self = .stateMismatch
        case .timedOut: self = .timedOut
        case .authorizationDenied(let reason): self = .authorizationDenied(reason)
        case .tokenExchangeFailed: self = .tokenExchangeFailed
        case .tokenRefreshFailed: self = .tokenRefreshFailed
        case .refreshTokenRevoked: self = .refreshTokenRevoked
        case .noRefreshTokenReturned: self = .noRefreshTokenReturned
        }
    }
}
