import Foundation
import Security

/// The OAuth client the Google Calendar and Gmail connectors sign in with: the app's own
/// built-in client (Secrets.xcconfig), or the user's own Desktop client from Settings →
/// Connectors → "Use your own Google Cloud project". Sync sign-in never uses this: it keeps
/// the built-in client through `SyncConfig`.
///
/// The secret must never reach a log, so `description`, `debugDescription` and the mirror
/// all redact both values.
struct GoogleOAuthClient: Equatable, CustomStringConvertible, CustomDebugStringConvertible, CustomReflectable {
    let clientId: String
    let clientSecret: String
    /// True for the user's own client (Gmail reading is requested only then).
    let isCustom: Bool

    var description: String { isCustom ? "GoogleOAuthClient(custom, <redacted>)" : "GoogleOAuthClient(built-in, <redacted>)" }
    var debugDescription: String { description }
    var customMirror: Mirror { Mirror(self, children: ["isCustom": isCustom], displayStyle: .struct) }

    /// The custom client when one is saved, else the built-in one; nil when the build has
    /// no built-in client and the user set none.
    static func active(store: CustomGoogleClientStore = .shared) -> GoogleOAuthClient? {
        if let custom = store.load() { return custom }
        return builtIn()
    }

    static func builtIn() -> GoogleOAuthClient? {
        guard let id = try? SyncConfig.googleOAuthClientId(), let secret = try? SyncConfig.googleOAuthClientSecret() else {
            return nil
        }
        return GoogleOAuthClient(clientId: id, clientSecret: secret, isCustom: false)
    }
}

/// The scopes the Google connectors request. The public app asks only for scopes Google
/// can verify without a CASA assessment: Gmail reading (`gmail.readonly`, restricted) is
/// requested only with the user's own client.
enum GoogleConnectorScopes {
    static let gmailReadonly = "https://www.googleapis.com/auth/gmail.readonly"
    static let gmailSend = "https://www.googleapis.com/auth/gmail.send"
    static let gmailCompose = "https://www.googleapis.com/auth/gmail.compose"
    static let calendarEvents = "https://www.googleapis.com/auth/calendar.events"

    /// `calendar.events` covers reading and writing events, so `calendar.readonly` is not asked for.
    static func calendar(customClient: Bool) -> String {
        "openid email \(calendarEvents)"
    }

    static func gmail(customClient: Bool) -> String {
        customClient ? "openid email \(gmailReadonly) \(gmailSend)" : "openid email \(gmailSend)"
    }
}

/// What a Gmail account's token may do, from the scopes Google actually GRANTED (never the
/// requested ones). A token with no recorded grant predates recording it and was minted
/// under the old `gmail.readonly gmail.compose` request, so it reads until a refresh
/// records the real grant.
enum GmailReadAccess {
    /// The tools that read mail: hidden from a send-only account, and never called for it.
    static let readTools: Set<String> = ["search_threads", "get_thread", "sync_messages"]

    static func canRead(grantedScope: String?) -> Bool {
        guard let grantedScope else { return true }
        return GoogleOAuthHelper.grantedScope(grantedScope, satisfies: GoogleConnectorScopes.gmailReadonly)
    }

    /// The connector tools that may be offered for an account with this grant. `create_draft`
    /// needs `gmail.compose`, which no current request asks for, so only an old grant has it.
    static func offeredTools(_ all: [String], grantedScope: String?) -> [String] {
        all.filter { tool in
            if readTools.contains(tool) { return canRead(grantedScope: grantedScope) }
            if tool == "create_draft", let grantedScope {
                return GoogleOAuthHelper.grantedScope(grantedScope, satisfies: GoogleConnectorScopes.gmailCompose)
            }
            return true
        }
    }
}

/// The user's own Google OAuth client, in the Keychain only (never UserDefaults or settings
/// sync). The names match the Windows credential store's.
final class CustomGoogleClientStore {
    static let shared = CustomGoogleClientStore()
    static let service = "whimbrel-google-oauth"
    static let idAccount = "custom-client-id"
    static let secretAccount = "custom-client-secret"

    private let service: String

    init(service: String = CustomGoogleClientStore.service) {
        self.service = service
    }

    func load() -> GoogleOAuthClient? {
        guard let id = read(Self.idAccount), !id.isEmpty, let secret = read(Self.secretAccount), !secret.isEmpty else {
            return nil
        }
        return GoogleOAuthClient(clientId: id, clientSecret: secret, isCustom: true)
    }

    /// Saves both values, trimmed. Returns false when either is empty or a write failed.
    @discardableResult
    func save(clientId: String, clientSecret: String) -> Bool {
        let id = clientId.trimmingCharacters(in: .whitespacesAndNewlines)
        let secret = clientSecret.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty, !secret.isEmpty else { return false }
        return write(Self.idAccount, id) && write(Self.secretAccount, secret)
    }

    func clear() {
        for account in [Self.idAccount, Self.secretAccount] {
            KeychainAccess.delete(query(account) as CFDictionary)
        }
    }

    private func query(_ account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    private func read(_ account: String) -> String? {
        var q = query(account)
        q[kSecReturnData as String] = true
        var result: AnyObject?
        guard KeychainAccess.copyMatching(q as CFDictionary, &result) == errSecSuccess, let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    private func write(_ account: String, _ value: String) -> Bool {
        KeychainAccess.upsert(query(account), data: Data(value.utf8)) == errSecSuccess
    }
}
