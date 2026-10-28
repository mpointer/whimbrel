import Foundation

/// "Live transcript during meetings". On by default; local to this Mac (the key is not in
/// the settings-sync field list).
final class LiveTranscriptSettings: ObservableObject {
    private static let enabledKey = "com.mpointer.kestrel.liveTranscriptEnabled"
    private let defaults: UserDefaults

    @Published var enabled: Bool {
        didSet { defaults.set(enabled, forKey: Self.enabledKey) }
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        enabled = defaults.object(forKey: Self.enabledKey) == nil ? true : defaults.bool(forKey: Self.enabledKey)
    }
}
