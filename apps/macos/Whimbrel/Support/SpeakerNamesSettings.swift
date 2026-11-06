import Foundation

/// "Remember voices": keep each meeting's speaker voices so a known voice is named next time.
/// On by default; local to this Mac, like `LiveQuestionSettings`.
/// See docs/superpowers/specs/2026-11-05-speaker-names-design.md.
final class SpeakerNamesSettings: ObservableObject {
    private static let rememberVoicesKey = "com.mpointer.kestrel.rememberVoices"
    private let defaults: UserDefaults

    @Published var rememberVoices: Bool {
        didSet { defaults.set(rememberVoices, forKey: Self.rememberVoicesKey) }
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        rememberVoices = defaults.object(forKey: Self.rememberVoicesKey) == nil ? true : defaults.bool(forKey: Self.rememberVoicesKey)
    }
}
