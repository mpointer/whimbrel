import Foundation
import OSLog
import Security

/// The providers Ask can route to. Raw values are the Mac's `Provider` raw values and the
/// `apiKey.<p>` field-key suffixes.
enum AskProvider: String, CaseIterable {
    case anthropic, openai, google, openrouter
}

/// One settings field as the core returns it (`pulled`) or plans it (`apply_plan`).
/// Pulled fields are keyed by the dictionary, so `field_key` is absent there.
struct PulledSettingsField: Codable, Equatable {
    var fieldKey: String
    let value: String
    let updatedAtUnixMs: Int64
    let isDeleted: Bool

    init(fieldKey: String, value: String, updatedAtUnixMs: Int64, isDeleted: Bool) {
        self.fieldKey = fieldKey
        self.value = value
        self.updatedAtUnixMs = updatedAtUnixMs
        self.isDeleted = isDeleted
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        fieldKey = try c.decodeIfPresent(String.self, forKey: .fieldKey) ?? ""
        value = try c.decode(String.self, forKey: .value)
        updatedAtUnixMs = try c.decode(Int64.self, forKey: .updatedAtUnixMs)
        isDeleted = try c.decode(Bool.self, forKey: .isDeleted)
    }

    enum CodingKeys: String, CodingKey {
        case fieldKey = "field_key"
        case value
        case updatedAtUnixMs = "updated_at_unix_ms"
        case isDeleted = "is_deleted"
    }
}

/// The core's settings sync, as the phone uses it (faked in tests). Calls block.
protocol SettingsPulling: Sendable {
    /// One round trip; `localJSON` is what to push (the phone passes nothing).
    func iterate(credentials: SyncCredentials, localJSON: String) -> [String: PulledSettingsField]?
    func applyPlan(pulled: [String: PulledSettingsField], localTimestamps: [String: Int64], apiKeySyncEnabled: [String: Bool]) -> [PulledSettingsField]?
}

/// API keys synced from the account, in the Keychain (in memory under XCTest).
struct PhoneKeyStore {
    static let service = "com.mpointer.kestrel.phone.apiKey"

    private func query(_ provider: AskProvider) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: Self.service, kSecAttrAccount as String: provider.rawValue]
    }

    func key(for provider: AskProvider) -> String? {
        var q = query(provider)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: AnyObject?
        guard KeychainAccess.copyMatching(q as CFDictionary, &result) == errSecSuccess, let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    @discardableResult
    func save(_ key: String, for provider: AskProvider) -> Bool {
        // Background pulls run after first unlock; the key must never leave this device.
        KeychainAccess.upsert(query(provider), data: Data(key.utf8),
                              addAttributes: [kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]) == errSecSuccess
    }

    func deleteAll() { AskProvider.allCases.forEach { delete(for: $0) } }

    func delete(for provider: AskProvider) {
        KeychainAccess.delete(query(provider) as CFDictionary)
    }
}

/// The synced routing settings the phone's Ask reads.
struct PhoneRoutingPrefs {
    static let prefix = "com.mpointer.kestrel.phone."
    let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) { self.defaults = defaults }

    private func string(_ field: String) -> String? {
        let s = defaults.string(forKey: Self.prefix + field)
        return s?.isEmpty == false ? s : nil
    }

    var preferredProvider: String? { string("routing.preferredProvider") }
    func defaultModel(for provider: AskProvider) -> String? { string("routing.\(provider.rawValue)DefaultModel") }
    var localOnlyMode: Bool { string("routing.localOnlyMode") == "true" }
    var userDailyCapCents: Int64? { string("routing.userDailyCapCents").flatMap { Int64($0) } }
    var globalDailyCapCents: Int64? { string("routing.globalDailyCapCents").flatMap { Int64($0) } }
    var customInstructions: String { defaults.string(forKey: Self.prefix + "summary.customPromptInstructions") ?? "" }
}

/// Pull-only settings sync: takes the account's API keys and routing fields. Never pushes.
/// A key is applied only while this phone's toggle for it is on, and the toggle arrives in
/// the same round as the key, so the key lands one round later (the core's `apply_plan`).
final class PhoneSettingsSync {
    private let puller: SettingsPulling
    private let defaults: UserDefaults
    private let keys: PhoneKeyStore
    private let log = Logger(subsystem: "com.mpointer.whimbrel.mobile", category: "settings-sync")

    private static let prefix = PhoneRoutingPrefs.prefix
    private static let emptyLocal = #"{"fields":{},"api_keys":[]}"#

    init(puller: SettingsPulling, defaults: UserDefaults, keys: PhoneKeyStore) {
        self.puller = puller
        self.defaults = defaults
        self.keys = keys
    }

    /// One round. A failure is logged and never thrown. Blocking: call off the main thread.
    /// `epoch` is `currentWipeEpoch` read when the credentials were captured, so a wipe between
    /// then and now also drops the writes; nil reads it now.
    func pullOnce(credentials: SyncCredentials, epoch: Int? = nil) {
        let epoch = epoch ?? Self.currentWipeEpoch
        guard let pulled = puller.iterate(credentials: credentials, localJSON: Self.emptyLocal) else {
            log.error("settings pull failed")
            return
        }
        let relevant = pulled.filter { Self.handles($0.key) }
        var timestamps: [String: Int64] = [:]
        for key in relevant.keys {
            if let ts = defaults.object(forKey: Self.prefix + "settingsTs." + key) as? Int64 { timestamps[key] = ts }
        }
        var toggles: [String: Bool] = [:]
        for p in AskProvider.allCases {
            toggles["apiKey.\(p.rawValue)"] = defaults.bool(forKey: Self.prefix + "apiKey.\(p.rawValue).syncEnabled")
        }
        guard let plan = puller.applyPlan(pulled: relevant, localTimestamps: timestamps, apiKeySyncEnabled: toggles) else {
            log.error("settings apply plan failed")
            return
        }
        for field in plan where Self.handles(field.fieldKey) {
            Self.wipeLock.lock()
            defer { Self.wipeLock.unlock() }
            guard Self.wipeEpoch == epoch else {
                log.info("wipe during settings pull: dropping the rest")
                return
            }
            apply(field)
        }
    }

    /// Revoke, wipe and fresh install: forget every synced key, setting and timestamp, so
    /// nothing outlives the account (a re-enrolled phone starts from the full delta).
    static func wipeLocal(defaults: UserDefaults = .standard, keys: PhoneKeyStore = PhoneKeyStore()) {
        // Under the same lock as every apply, so a pull in flight can't write after the clear.
        wipeLock.lock()
        defer { wipeLock.unlock() }
        wipeEpoch += 1
        keys.deleteAll()
        for k in defaults.dictionaryRepresentation().keys where k.hasPrefix(prefix) { defaults.removeObject(forKey: k) }
    }

    /// Bumped by every wipe; a pull that started before one drops its remaining writes.
    /// Both are guarded by `wipeLock`.
    static var currentWipeEpoch: Int {
        wipeLock.lock()
        defer { wipeLock.unlock() }
        return wipeEpoch
    }

    private nonisolated(unsafe) static var wipeEpoch = 0
    private static let wipeLock = NSLock()

    private static func handles(_ key: String) -> Bool {
        if key.hasPrefix("routing.") || key == "summary.customPromptInstructions" { return true }
        return AskProvider.allCases.contains { key == "apiKey.\($0.rawValue)" || key == "apiKey.\($0.rawValue).syncEnabled" }
    }

    private func apply(_ field: PulledSettingsField) {
        let key = field.fieldKey
        if let p = AskProvider.allCases.first(where: { key == "apiKey.\($0.rawValue)" }) {
            if field.isDeleted { keys.delete(for: p) } else if !keys.save(field.value, for: p) { return }
        } else if let p = AskProvider.allCases.first(where: { key == "apiKey.\($0.rawValue).syncEnabled" }) {
            let on = !field.isDeleted && field.value == "true"
            defaults.set(on, forKey: Self.prefix + "apiKey.\(p.rawValue).syncEnabled")
            if !on { keys.delete(for: p) }
        } else {
            defaults.set(field.isDeleted ? "" : field.value, forKey: Self.prefix + key)
        }
        defaults.set(field.updatedAtUnixMs, forKey: Self.prefix + "settingsTs." + key)
    }
}
