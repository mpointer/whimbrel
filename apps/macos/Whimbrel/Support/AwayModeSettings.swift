import Foundation

/// Away Mode: Whimbrel joins Zoom meetings on its own. Local to this Mac. While on, the Mac is kept awake.
@MainActor
final class AwayModeSettings: ObservableObject {
    private static let enabledKey = "com.mpointer.kestrel.awayModeEnabled"
    private let defaults: UserDefaults
    private let keepAwake: KeepAwakeControlling

    @Published var enabled: Bool {
        didSet {
            defaults.set(enabled, forKey: Self.enabledKey)
            if enabled { keepAwake.begin() } else { keepAwake.end() }
        }
    }

    /// `available` false (joining isn't built yet): stays off and clears an earlier choice, so
    /// the Mac isn't kept awake for a feature that can't run.
    init(defaults: UserDefaults = .standard, keepAwake: KeepAwakeControlling, available: Bool = true) {
        self.defaults = defaults
        self.keepAwake = keepAwake
        if !available { defaults.removeObject(forKey: Self.enabledKey) }
        enabled = available && defaults.bool(forKey: Self.enabledKey)
        if enabled { keepAwake.begin() }
    }
}
