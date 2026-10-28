import Foundation
import Security

enum SyncKeychainError: Error {
    case writeFailed(OSStatus)
    case readFailed(OSStatus)
}

/// Keychain storage for this device's sync keypair secret and the
/// account's group key — both base64-encoded 32-byte values from
/// `core/sync` via `CoreBridge.swift`'s `whimbrel_sync_*` wrappers.
/// A small, focused helper mirroring `GoogleOAuthHelper`'s own private
/// Keychain pattern (delete-then-add), rather than extending
/// `KeychainService`'s `Provider` enum, which is scoped to LLM provider
/// API keys and not meant to grow arbitrary new secret categories.
enum SyncKeychain {
    private static let service = (Bundle.main.bundleIdentifier ?? "com.mpointer.kestrel") + ".sync"
    private static let deviceSecretKeyAccount = "device-secret-key"
    private static let groupKeyAccount = "group-key"

    static func saveDeviceSecretKey(_ base64: String) throws {
        try save(base64, account: deviceSecretKeyAccount)
    }

    static func loadDeviceSecretKey() throws -> String? {
        try load(account: deviceSecretKeyAccount)
    }

    static func saveGroupKey(_ base64: String) throws {
        try save(base64, account: groupKeyAccount)
    }

    static func loadGroupKey() throws -> String? {
        try load(account: groupKeyAccount)
    }

    static func deleteDeviceSecretKey() throws {
        try delete(account: deviceSecretKeyAccount)
    }

    static func deleteGroupKey() throws {
        try delete(account: groupKeyAccount)
    }

    private static func delete(account: String) throws {
        let status = KeychainAccess.delete([
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ] as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw SyncKeychainError.writeFailed(status)
        }
    }

    private static func save(_ value: String, account: String) throws {
        let data = Data(value.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        var addAttributes: [String: Any] = [:]
        #if os(iOS)
        // The phone syncs in the background, after the first unlock, and these keys must
        // never leave the device (no iCloud Keychain, no backup restore elsewhere).
        addAttributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        #endif
        let status = KeychainAccess.upsert(query, data: data, addAttributes: addAttributes)
        guard status == errSecSuccess else {
            throw SyncKeychainError.writeFailed(status)
        }
    }

    private static func load(account: String) throws -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
        ]
        var result: AnyObject?
        let status = KeychainAccess.copyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess, let data = result as? Data else {
            throw SyncKeychainError.readFailed(status)
        }
        return String(data: data, encoding: .utf8)
    }
}
