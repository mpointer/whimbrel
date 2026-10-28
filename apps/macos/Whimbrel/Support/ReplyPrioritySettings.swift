import Foundation

/// Reply Priorities settings (docs/superpowers/specs/2026-10-05-reply-priorities-design.md).
/// Local to this Mac: the toggle lives in UserDefaults under a key that is NOT in the
/// settings-sync field list, and the TypeSafe key stays in the Keychain, so neither syncs.
final class ReplyPrioritySettings: ObservableObject {
    private static let enabledKey = "com.mpointer.kestrel.replyPriorityEnabled"

    private let defaults: UserDefaults
    private let loadKey: () -> String?
    private let deleteKey: () -> Void

    @Published var enabled: Bool {
        didSet { defaults.set(enabled, forKey: Self.enabledKey) }
    }
    @Published private(set) var hasKey: Bool
    /// Set when TypeSafe answered 401/403 to the last batch; cleared by the next batch that succeeds.
    @Published private(set) var keyRejected = false

    init(
        defaults: UserDefaults = .standard,
        keyExists: () -> Bool = { KeychainService.exists(for: .typesafe) },
        loadKey: @escaping () -> String? = { KeychainService.load(for: .typesafe) },
        deleteKey: @escaping () -> Void = { KeychainService.delete(for: .typesafe) }
    ) {
        self.defaults = defaults
        self.loadKey = loadKey
        self.deleteKey = deleteKey
        enabled = defaults.bool(forKey: Self.enabledKey)
        hasKey = keyExists()
    }

    @discardableResult
    func saveKey(_ key: String) -> Bool {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, KeychainService.save(trimmed, for: .typesafe) else { return false }
        hasKey = true
        keyRejected = false
        return true
    }

    func clearKey() {
        deleteKey()
        enabled = false
        hasKey = false
        keyRejected = false
    }

    func markKeyRejected(_ rejected: Bool) {
        if keyRejected != rejected { keyRejected = rejected }
    }

    /// The key to score with, or nil when the feature is off, has no key, or Local-only
    /// mode ("hosted providers are never called") is on.
    func activeKey(localOnly: Bool) -> String? {
        guard enabled, hasKey, !localOnly else { return nil }
        return loadKey()
    }
}
