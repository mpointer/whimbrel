import Foundation

/// "Alert me when someone asks me something" and "Also email me". Both on by default; local to
/// this Mac (not in the settings-sync field list), like `LiveTranscriptSettings`.
/// See docs/superpowers/specs/2026-11-02-live-question-alerts-design.md.
final class LiveQuestionSettings: ObservableObject {
    private static let enabledKey = "com.mpointer.kestrel.liveQuestionEnabled"
    private static let emailEnabledKey = "com.mpointer.kestrel.liveQuestionEmailEnabled"
    private let defaults: UserDefaults

    @Published var enabled: Bool {
        didSet { defaults.set(enabled, forKey: Self.enabledKey) }
    }
    @Published var emailEnabled: Bool {
        didSet { defaults.set(emailEnabled, forKey: Self.emailEnabledKey) }
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        enabled = defaults.object(forKey: Self.enabledKey) == nil ? true : defaults.bool(forKey: Self.enabledKey)
        emailEnabled = defaults.object(forKey: Self.emailEnabledKey) == nil ? true : defaults.bool(forKey: Self.emailEnabledKey)
    }
}
