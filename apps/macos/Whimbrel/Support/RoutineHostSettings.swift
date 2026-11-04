import Foundation

/// What this device may run (mirrors core's `RunGate`; the raw value is the FFI byte).
enum RoutineGate: UInt8 {
    case off = 0, full = 1, failover = 2
}

enum RoutineHost {
    /// `.full` unless a DIFFERENT approved device was chosen; then `.failover`: this device runs
    /// the read-only routines only when they are 3 hours overdue. An unset choice, or one the
    /// approved list doesn't name, behaves as "every device runs", so a stale setting can never
    /// silently stop routines. An empty list hasn't loaded yet, so it says nothing about the
    /// choice: this device waits as a failover rather than doubling up on the primary.
    static func gate(primaryId: String, ownId: String, approvedDeviceIds: Set<String>) -> RoutineGate {
        let primary = primaryId.trimmingCharacters(in: .whitespacesAndNewlines)
        if primary.isEmpty || primary == ownId || ownId == "spike-device" { return .full }
        return approvedDeviceIds.isEmpty || approvedDeviceIds.contains(primary) ? .failover : .full
    }
}

/// Which device runs routines, shared across the user's devices through settings sync
/// (docs/superpowers/specs/2026-10-08-primary-device-routines-design.md). Empty = unset.
@MainActor
final class RoutineHostSettings: ObservableObject {
    nonisolated static let fieldKey = "routines.primaryDeviceId"
    /// The device that made the current choice, synced beside it; its timestamp says when.
    nonisolated static let setByFieldKey = "routines.primaryDeviceId.setBy"
    private static let defaultsKey = "com.mpointer.kestrel.routinePrimaryDeviceId"
    private static let setByDefaultsKey = "com.mpointer.kestrel.routinePrimaryDeviceSetBy"
    private let defaults: UserDefaults

    @Published var primaryDeviceId: String {
        didSet {
            defaults.set(primaryDeviceId, forKey: Self.defaultsKey)
            SettingsFieldTimestamps.touch(Self.fieldKey)
        }
    }

    @Published private(set) var setBy: String {
        didSet {
            defaults.set(setBy, forKey: Self.setByDefaultsKey)
            SettingsFieldTimestamps.touch(Self.setByFieldKey)
        }
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        primaryDeviceId = defaults.string(forKey: Self.defaultsKey) ?? ""
        setBy = defaults.string(forKey: Self.setByDefaultsKey) ?? ""
    }

    /// A person's choice in Settings: the only path that should change the primary on this
    /// device. Records this device as the one that chose, so every device can show who did.
    func choose(_ deviceId: String, by ownDeviceId: String) {
        primaryDeviceId = deviceId
        setBy = ownDeviceId
    }

    func apply(fieldKey: String, value: String) -> Bool {
        switch fieldKey {
        case Self.fieldKey: primaryDeviceId = value
        case Self.setByFieldKey: setBy = value
        default: return false
        }
        return true
    }
}
