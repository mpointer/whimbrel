import AuthenticationServices
import CryptoKit
import Foundation
import Security
import UIKit

enum GoogleSignInError: Error {
    case cancelled
    case notSignedIn
    case failed(String)
}

/// The tokens blob kept in the Keychain; same shape (and JSON keys) as the Mac's
/// `OAuthStoredTokens` and core/api::oauth::StoredTokens, so the core can refresh it.
struct OAuthStoredTokens: Codable {
    let accessToken: String
    let refreshToken: String
    let idToken: String?
    let expiresAtUnixMs: Int64
    let grantedScope: String?
}

/// Google sign-in for an iOS OAuth client: Authorization Code + PKCE through
/// ASWebAuthenticationSession, redirecting to the reversed-client-id custom scheme.
/// The Mac's loopback flow (core `whimbrel_oauth_begin`) can't be used: an iOS client only
/// accepts its custom-scheme redirect. The code exchange is here (iOS clients have no
/// secret); refresh goes through the core's `whimbrel_oauth_refresh`, as on the Mac.
/// Tokens are never logged.
final class GoogleSignIn: NSObject, ASWebAuthenticationPresentationContextProviding {
    private static let keychainService = (Bundle.main.bundleIdentifier ?? "com.mpointer.whimbrel.mobile") + ".google-oauth"
    private static let keychainAccount = "google-sync"
    private static let scope = "openid email"

    private let clientId: String
    /// Held for the life of the browser hop; ASWebAuthenticationSession isn't retained otherwise.
    private var session: ASWebAuthenticationSession?
    private var redirectScheme: String { EnrollmentConfig.redirectScheme(clientId: clientId) }
    private var redirectURI: String { redirectScheme + ":/oauthredirect" }

    init(clientId: String) {
        self.clientId = clientId
    }

    @MainActor
    func authorize() async throws {
        let verifier = try Self.randomURLSafe(bytes: 32)
        let state = try Self.randomURLSafe(bytes: 16)
        let challenge = Self.base64URL(Data(SHA256.hash(data: Data(verifier.utf8))))
        var components = URLComponents(string: "https://accounts.google.com/o/oauth2/v2/auth")!
        components.queryItems = [
            URLQueryItem(name: "client_id", value: clientId),
            URLQueryItem(name: "redirect_uri", value: redirectURI),
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "scope", value: Self.scope),
            URLQueryItem(name: "code_challenge", value: challenge),
            URLQueryItem(name: "code_challenge_method", value: "S256"),
            URLQueryItem(name: "state", value: state),
            // Without these Google returns no refresh token (needed for silent resume).
            URLQueryItem(name: "access_type", value: "offline"),
            URLQueryItem(name: "prompt", value: "consent"),
        ]
        let callback: URL = try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(url: components.url!, callbackURLScheme: redirectScheme) { url, error in
                if let url {
                    continuation.resume(returning: url)
                } else if let error = error as? ASWebAuthenticationSessionError, error.code == .canceledLogin {
                    continuation.resume(throwing: GoogleSignInError.cancelled)
                } else {
                    continuation.resume(throwing: GoogleSignInError.failed("browser session failed"))
                }
            }
            session.presentationContextProvider = self
            self.session = session
            if !session.start() {
                continuation.resume(throwing: GoogleSignInError.failed("browser session did not start"))
            }
        }
        session = nil
        let items = URLComponents(url: callback, resolvingAgainstBaseURL: false)?.queryItems ?? []
        func item(_ name: String) -> String? { items.first { $0.name == name }?.value }
        guard item("state") == state else { throw GoogleSignInError.failed("state mismatch") }
        if let denied = item("error") { throw GoogleSignInError.failed("denied: \(denied)") }
        guard let code = item("code") else { throw GoogleSignInError.failed("no code") }
        try store(try await exchange(code: code, verifier: verifier))
    }

    /// A valid ID token, refreshed through the core when stale.
    func validIdToken() async throws -> String {
        guard let stored = try readStored() else { throw GoogleSignInError.notSignedIn }
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        if now < stored.expiresAtUnixMs - 60_000, let idToken = stored.idToken { return idToken }
        let refreshed = try await Task.detached(priority: .userInitiated) { [clientId] in
            try Self.coreRefresh(clientId: clientId, stored: stored, nowUnixMs: now)
        }.value
        try store(refreshed)
        guard let idToken = refreshed.idToken else { throw GoogleSignInError.failed("no id token") }
        return idToken
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        return scenes.flatMap(\.windows).first { $0.isKeyWindow } ?? ASPresentationAnchor()
    }

    // MARK: - Token endpoint

    private struct TokenResponse: Decodable {
        let access_token: String
        let refresh_token: String?
        let id_token: String?
        let expires_in: Int64
        let scope: String?
    }

    private func exchange(code: String, verifier: String) async throws -> OAuthStoredTokens {
        var request = URLRequest(url: URL(string: "https://oauth2.googleapis.com/token")!)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        var form = URLComponents()
        form.queryItems = [
            URLQueryItem(name: "code", value: code),
            URLQueryItem(name: "client_id", value: clientId),
            URLQueryItem(name: "redirect_uri", value: redirectURI),
            URLQueryItem(name: "grant_type", value: "authorization_code"),
            URLQueryItem(name: "code_verifier", value: verifier),
        ]
        // URLComponents leaves "+" unescaped, which a form body reads as a space.
        request.httpBody = Data((form.percentEncodedQuery ?? "").replacingOccurrences(of: "+", with: "%2B").utf8)
        let (data, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200,
              let reply = try? JSONDecoder().decode(TokenResponse.self, from: data) else {
            throw GoogleSignInError.failed("token exchange failed")
        }
        guard let refreshToken = reply.refresh_token, reply.id_token != nil else {
            throw GoogleSignInError.failed("missing refresh or id token")
        }
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        return OAuthStoredTokens(accessToken: reply.access_token, refreshToken: refreshToken, idToken: reply.id_token,
                                 expiresAtUnixMs: now + reply.expires_in * 1000, grantedScope: reply.scope)
    }

    private struct CoreOAuthConfig: Encodable {
        let provider = "google"
        let client_id: String
        let scope: String
    }

    private struct CoreOAuthOutcome: Decodable {
        let tokens: OAuthStoredTokens?
        let error: String?
    }

    /// BLOCKING: `whimbrel_oauth_refresh` with no client secret (an iOS client has none).
    private static func coreRefresh(clientId: String, stored: OAuthStoredTokens, nowUnixMs: Int64) throws -> OAuthStoredTokens {
        let config = String(decoding: try JSONEncoder().encode(CoreOAuthConfig(client_id: clientId, scope: scope)), as: UTF8.self)
        let blob = String(decoding: try JSONEncoder().encode(stored), as: UTF8.self)
        guard let json = config.withCString({ c in blob.withCString { s in whimbrel_oauth_refresh(c, s, nowUnixMs) } }) else {
            throw GoogleSignInError.failed("refresh failed")
        }
        defer { whimbrel_core_free_string(json) }
        let outcome = try? JSONDecoder().decode(CoreOAuthOutcome.self, from: Data(String(cString: json).utf8))
        if let tokens = outcome?.tokens { return tokens }
        if outcome?.error == "refresh_token_revoked" { throw GoogleSignInError.notSignedIn }
        throw GoogleSignInError.failed("refresh failed")
    }

    // MARK: - Keychain (KeychainAccess, so tests use the in-memory store)

    private var baseQuery: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: Self.keychainService,
         kSecAttrAccount as String: Self.keychainAccount]
    }

    private func store(_ tokens: OAuthStoredTokens) throws {
        KeychainAccess.delete(baseQuery as CFDictionary)
        var add = baseQuery
        add[kSecValueData as String] = try JSONEncoder().encode(tokens)
        // Readable after the first unlock so a background sync can refresh; never synced off-device.
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = KeychainAccess.add(add as CFDictionary, nil)
        guard status == errSecSuccess else { throw SyncKeychainError.writeFailed(status) }
    }

    /// Forgets the sign-in: deletes the stored tokens.
    func signOut() {
        KeychainAccess.delete(baseQuery as CFDictionary)
    }

    private func readStored() throws -> OAuthStoredTokens? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        var result: AnyObject?
        let status = KeychainAccess.copyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw SyncKeychainError.readFailed(status) }
        return try? JSONDecoder().decode(OAuthStoredTokens.self, from: data)
    }

    // MARK: - PKCE helpers

    private static func randomURLSafe(bytes count: Int) throws -> String {
        var bytes = [UInt8](repeating: 0, count: count)
        let status = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        guard status == errSecSuccess else { throw GoogleSignInError.failed("random generation failed") }
        return base64URL(Data(bytes))
    }

    private static func base64URL(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }
}
