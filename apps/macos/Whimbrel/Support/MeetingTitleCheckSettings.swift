import Foundation

/// Opt-in calendar title check (core `api::title_check`): after a calendar meeting is
/// summarized, TypeSafe judges whether the summary matches the event's title. Local to
/// this Mac: the toggle lives in UserDefaults under a key that is NOT in the settings-sync
/// field list. The key is the one Reply Priorities already stores in the Keychain.
final class MeetingTitleCheckSettings: ObservableObject {
    private static let enabledKey = "com.mpointer.kestrel.meetingTitleCheckEnabled"

    private let defaults: UserDefaults
    private let keyExists: () -> Bool
    private let loadKey: () -> String?

    @Published var enabled: Bool {
        didSet { defaults.set(enabled, forKey: Self.enabledKey) }
    }

    init(
        defaults: UserDefaults = .standard,
        keyExists: @escaping () -> Bool = { KeychainService.exists(for: .typesafe) },
        loadKey: @escaping () -> String? = { KeychainService.load(for: .typesafe) }
    ) {
        self.defaults = defaults
        self.keyExists = keyExists
        self.loadKey = loadKey
        enabled = defaults.bool(forKey: Self.enabledKey)
    }

    /// The key to send with a summarize call, or nil when the check is off, has no key,
    /// or Local-only mode ("hosted providers are never called") is on.
    func activeKey(localOnly: Bool) -> String? {
        guard enabled, keyExists(), !localOnly else { return nil }
        return loadKey()
    }
}
