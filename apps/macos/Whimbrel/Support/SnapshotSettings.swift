import Foundation

/// "Meeting snapshots": slide images saved as PNGs in a user-visible folder. On by default;
/// local to this Mac (keys are not in the settings-sync field list).
@MainActor
final class SnapshotSettings: ObservableObject {
    private static let enabledKey = "com.mpointer.kestrel.meetingSnapshots.enabled"
    private static let rootKey = "com.mpointer.kestrel.meetingSnapshots.root"
    private let defaults: UserDefaults

    @Published var enabled: Bool {
        didSet { defaults.set(enabled, forKey: Self.enabledKey) }
    }

    /// nil means the default `~/Pictures/Whimbrel Snapshots`.
    @Published var customRootPath: String? {
        didSet { defaults.set(customRootPath, forKey: Self.rootKey) }
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        enabled = defaults.object(forKey: Self.enabledKey) == nil ? true : defaults.bool(forKey: Self.enabledKey)
        customRootPath = defaults.string(forKey: Self.rootKey)
    }

    var rootURL: URL {
        if let path = customRootPath, !path.isEmpty { return URL(fileURLWithPath: path, isDirectory: true) }
        return FileManager.default.urls(for: .picturesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Whimbrel Snapshots", isDirectory: true)
    }
}
