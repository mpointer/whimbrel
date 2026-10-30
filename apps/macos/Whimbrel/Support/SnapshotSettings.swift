import Foundation

/// "Meeting snapshots": slide images saved to the media folder's `Snapshots` (`MediaSettings`).
/// On by default; local to this Mac (keys are not in the settings-sync field list).
@MainActor
final class SnapshotSettings: ObservableObject {
    private static let enabledKey = "com.mpointer.kestrel.meetingSnapshots.enabled"
    private let defaults: UserDefaults

    @Published var enabled: Bool {
        didSet { defaults.set(enabled, forKey: Self.enabledKey) }
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        enabled = defaults.object(forKey: Self.enabledKey) == nil ? true : defaults.bool(forKey: Self.enabledKey)
    }
}
