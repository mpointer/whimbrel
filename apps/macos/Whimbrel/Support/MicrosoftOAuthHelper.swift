import AuthenticationServices
import Foundation
import Security

/// Runs Microsoft's OAuth 2.0 Authorization Code flow with PKCE via a loopback-listener
/// redirect against Azure AD's `common` endpoint (work/school and personal accounts).
/// Microsoft's "Mobile and desktop applications" platform is a public client: no client
/// secret is sent, and the advertised redirect host is `localhost` (wildcard-port
/// matching). Both rules live in core/api::oauth's Microsoft provider (M3.0 Task 10);
/// this class keeps the `ASWebAuthenticationSession` browser hop and Keychain storage.
final class MicrosoftOAuthHelper: NSObject {
    /// Scoped to the running bundle so distinct app builds signed under the same Team ID
    /// never share Keychain items via a fixed service string.
    static let keychainService = (Bundle.main.bundleIdentifier ?? "com.mpointer.kestrel") + ".microsoft-oauth"
    private let clientId: String
    private let scope: String
    private let keychainAccount: String

    init(clientId: String, scope: String, keychainAccount: String) {
        self.clientId = clientId
        self.scope = scope
        self.keychainAccount = keychainAccount
    }

    private var config: CoreOAuthConfig {
        CoreOAuthConfig(provider: "microsoft", clientId: clientId, clientSecret: nil, scope: scope)
    }

    /// Runs the full loopback OAuth flow (see `GoogleOAuthHelper.authorize`).
    @MainActor
    func authorize(presentationContextProvider: ASWebAuthenticationPresentationContextProviding) async throws {
        guard let session = CoreMemoryStore.oauthBegin(config: config) else {
            throw MicrosoftOAuthError.loopbackListenerFailed
        }
        let cancellationBox = CancellationBox()
        switch await OAuthBrowserFlow.run(session: session, presentationContextProvider: presentationContextProvider, cancellationBox: cancellationBox) {
        case .success(let tokens):
            try storeTokensDirectly(tokens)
        case .failure(let error):
            if cancellationBox.wasCancelled {
                throw MicrosoftOAuthError.signInCancelled
            }
            throw MicrosoftOAuthError(coreError: error)
        }
    }

    /// Returns the stored access token, refreshing it first if it has expired.
    func validAccessToken() async throws -> String {
        try await validTokens().accessToken
    }

    /// Returns the stored Microsoft ID token, refreshing it first if it has expired.
    func validIdToken() async throws -> String {
        let tokens = try await validTokens()
        guard let idToken = tokens.idToken else {
            throw MicrosoftOAuthError.noIdTokenReturned
        }
        return idToken
    }

    /// The connected account's identifying claim from an unverified ID token: `email`
    /// first, falling back to `preferred_username` (work/school accounts frequently
    /// omit `email`).
    static func decodeAccountIdentifier(_ jwt: String) -> String? {
        guard let claims = CoreMemoryStore.oauthJwtClaims(jwt) else { return nil }
        if let email = claims["email"] as? String { return email }
        return claims["preferred_username"] as? String
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

    /// Copies this instance's stored token blob verbatim to a new Keychain account.
    func copyStoredTokens(toKeychainAccount newAccount: String) -> Bool {
        guard let stored = try? readStoredTokens() else { return false }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.keychainService,
            kSecAttrAccount as String: newAccount,
        ]
        guard let data = try? JSONEncoder().encode(stored) else { return false }
        KeychainAccess.delete(query as CFDictionary)
        var addQuery = query
        addQuery[kSecValueData as String] = data
        return KeychainAccess.add(addQuery as CFDictionary, nil) == errSecSuccess
    }

    private func validTokens() async throws -> OAuthStoredTokens {
        guard let stored = try readStoredTokens() else {
            throw MicrosoftOAuthError.notAuthorized
        }
        if stored.isFresh(nowUnixMs: Int64(Date().timeIntervalSince1970 * 1000)) {
            return stored
        }
        let config = self.config
        let result = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.oauthRefresh(config: config, stored: stored, nowUnixMs: Int64(Date().timeIntervalSince1970 * 1000))
        }.value
        switch result {
        case .success(let refreshed):
            try storeTokensDirectly(refreshed)
            return refreshed
        case .failure(let error):
            throw MicrosoftOAuthError(coreError: error)
        }
    }

    private func storeTokensDirectly(_ stored: OAuthStoredTokens) throws {
        let data = try JSONEncoder().encode(stored)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.keychainService,
            kSecAttrAccount as String: keychainAccount,
        ]
        KeychainAccess.delete(query as CFDictionary)
        var addQuery = query
        addQuery[kSecValueData as String] = data
        let status = KeychainAccess.add(addQuery as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw MicrosoftOAuthError.keychainWriteFailed(status)
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
            throw MicrosoftOAuthError.keychainReadFailed(status)
        }
        return try JSONDecoder().decode(OAuthStoredTokens.self, from: data)
    }
}

enum MicrosoftOAuthError: Error {
    case notAuthorized
    case tokenExchangeFailed
    case tokenRefreshFailed
    case noRefreshTokenReturned
    case noIdTokenReturned
    case keychainWriteFailed(OSStatus)
    case keychainReadFailed(OSStatus)
    case loopbackListenerFailed
    case randomGenerationFailed(OSStatus)
    case stateMismatch
    case timedOut
    case authorizationDenied(String)
    /// The user cancelled the `ASWebAuthenticationSession` window.
    case signInCancelled
}

extension MicrosoftOAuthError {
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
        case .tokenRefreshFailed, .refreshTokenRevoked: self = .tokenRefreshFailed
        case .noRefreshTokenReturned: self = .noRefreshTokenReturned
        }
    }
}
