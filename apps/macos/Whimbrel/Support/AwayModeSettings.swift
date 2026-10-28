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

    init(defaults: UserDefaults = .standard, keepAwake: KeepAwakeControlling) {
        self.defaults = defaults
        self.keepAwake = keepAwake
        enabled = defaults.bool(forKey: Self.enabledKey)
        if enabled { keepAwake.begin() }
    }
}
