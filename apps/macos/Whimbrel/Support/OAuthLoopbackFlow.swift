import AuthenticationServices
import Foundation

/// Generic errors from the loopback-redirect PKCE OAuth mechanics (now core/api::oauth,
/// M3.0 Task 10), shared by every OAuth provider helper (GoogleOAuthHelper,
/// MicrosoftOAuthHelper). Each provider's own error enum maps these 1:1 into its own
/// case set via an `init(loopbackError:)` initializer, so existing exhaustive `switch`es
/// over that provider's error type (e.g. SyncState's Google-error-to-user-message
/// mapping) see no change.
enum OAuthLoopbackError: Error {
    case loopbackListenerFailed
    case randomGenerationFailed(OSStatus)
    case stateMismatch
    case timedOut
    case authorizationDenied(String)
}

/// The token blob every provider helper keeps in the Keychain (one item per account).
/// Field names match core/api::oauth's `StoredTokens`, which is what
/// `whimbrel_oauth_wait`/`whimbrel_oauth_refresh` produce, and the entries the app
/// wrote before this type existed: `grantedScope` is optional so older Google items
/// (and every Microsoft item) decode cleanly.
struct OAuthStoredTokens: Codable, Equatable {
    let accessToken: String
    let refreshToken: String
    let idToken: String?
    let expiresAtUnixMs: Int64
    /// The granted (not requested) scope, persisted so a write action can check it
    /// before use. `nil` means "unknown": `validAccessToken(requiringGrantedScope:)`
    /// resolves it by forcing one silent refresh rather than guessing.
    let grantedScope: String?

    /// Treat a token as due for refresh once it's within a minute of actual expiry.
    func isFresh(nowUnixMs: Int64) -> Bool {
        nowUnixMs < expiresAtUnixMs - 60_000
    }
}

/// Records whether the user cancelled the `ASWebAuthenticationSession` window, set from
/// that session's completion handler (main thread, but not statically `@MainActor`-
/// isolated to the compiler) and read from a provider helper's `authorize()` catch
/// block. `@unchecked Sendable` because access is serialized by its own lock.
final class CancellationBox: @unchecked Sendable {
    private let lock = NSLock()
    private var _wasCancelled = false

    var wasCancelled: Bool {
        lock.lock()
        defer { lock.unlock() }
        return _wasCancelled
    }

    func markCancelled() {
        lock.lock()
        defer { lock.unlock() }
        _wasCancelled = true
    }
}

/// The browser hop the core cannot do: runs `whimbrel_oauth_wait` off the main actor
/// while `ASWebAuthenticationSession` shows the consent page, and maps a closed auth
/// window to the core's cancel. Shared by both provider helpers.
enum OAuthBrowserFlow {
    /// If the browser/auth window is closed without completing or explicitly
    /// cancelling, don't hang (and keep the port bound) forever.
    static let authorizationTimeoutMs: UInt64 = 5 * 60 * 1000

    @MainActor
    static func run(
        session: CoreOAuthSession,
        presentationContextProvider: ASWebAuthenticationPresentationContextProviding,
        cancellationBox: CancellationBox
    ) async -> Result<OAuthStoredTokens, CoreOAuthError> {
        let sessionId = session.sessionId
        async let outcome = Task.detached(priority: .userInitiated) {
            CoreMemoryStore.oauthWait(sessionId: sessionId, timeoutMs: authorizationTimeoutMs, nowUnixMs: Int64(Date().timeIntervalSince1970 * 1000))
        }.value

        let authSession = ASWebAuthenticationSession(url: session.authURL, callbackURLScheme: nil) { _, error in
            guard let error else {
                // No callback URL for this loopback flow's own completion: the
                // redirect is caught by the core's listener instead.
                return
            }
            if case ASWebAuthenticationSessionError.canceledLogin = error {
                cancellationBox.markCancelled()
            }
            // Force the loopback wait to resolve now instead of idling for up to
            // the timeout.
            CoreMemoryStore.oauthCancel(sessionId: sessionId)
        }
        authSession.presentationContextProvider = presentationContextProvider
        authSession.prefersEphemeralWebBrowserSession = false
        // Always tear down the auth window on the way out.
        defer { authSession.cancel() }
        authSession.start()
        return await outcome
    }
}

