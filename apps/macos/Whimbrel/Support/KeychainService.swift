import Foundation
import Security

/// Wraps macOS Keychain generic-password items for hosted LLM API keys (Settings UI —
/// see docs/superpowers/specs/2026-07-27-settings-ui-routing-design.md) — never written
/// to a plaintext file or UserDefaults, matching Whimbrel's existing posture toward
/// captured screen text. `kSecAttrService` is the app's bundle id; `kSecAttrAccount`
/// distinguishes Anthropic's key from OpenAI's within that service. No Keychain access
/// group — single-app, not shared with an extension.
enum KeychainService {
    enum Provider: String {
        case anthropic = "anthropic-api-key"
        case openai = "openai-api-key"
        case google = "google-api-key"
        case typesafe = "typesafe-api-key"
        /// The web image search key (Skins Release 2), local to this Mac.
        case imageSearch = "image-search-api-key"
    }

    private static let service = Bundle.main.bundleIdentifier ?? "com.mpointer.kestrel"

    /// Saves `key`, replacing any existing value for `provider`. Returns whether the
    /// save succeeded.
    @discardableResult
    static func save(_ key: String, for provider: Provider) -> Bool {
        guard let data = key.data(using: .utf8) else { return false }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: provider.rawValue,
        ]
        let attributesToUpdate: [String: Any] = [kSecValueData as String: data]

        let updateStatus = KeychainAccess.update(query as CFDictionary, attributesToUpdate as CFDictionary)
        if updateStatus == errSecSuccess {
            return true
        }
        guard updateStatus == errSecItemNotFound else { return false }

        var addQuery = query
        addQuery[kSecValueData as String] = data
        return KeychainAccess.add(addQuery as CFDictionary, nil) == errSecSuccess
    }

    /// Returns the saved key for `provider`, or nil if none is saved (or the read
    /// failed).
    static func load(for provider: Provider) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: provider.rawValue,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: AnyObject?
        let status = KeychainAccess.copyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess, let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Like `load(for:)` but never materializes the secret value itself — uses
    /// `kSecReturnData: false` so a presence check doesn't pull the real key into
    /// memory just to discard it.
    static func exists(for provider: Provider) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: provider.rawValue,
            kSecReturnData as String: false,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        return KeychainAccess.copyMatching(query as CFDictionary, nil) == errSecSuccess
    }

    /// Removes the saved key for `provider`, if any. Safe to call when no key is saved.
    static func delete(for provider: Provider) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: provider.rawValue,
        ]
        KeychainAccess.delete(query as CFDictionary)
    }
}
