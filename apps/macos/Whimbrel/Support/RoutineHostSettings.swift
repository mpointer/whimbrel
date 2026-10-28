import Foundation

/// What this device may run (mirrors core's `RunGate`; the raw value is the FFI byte).
enum RoutineGate: UInt8 {
    case off = 0, full = 1, failover = 2
}

enum RoutineHost {
    /// `.full` unless a DIFFERENT approved device was chosen; then `.failover`: this device runs
    /// the read-only routines only when they are 3 hours overdue. An unset, unknown or revoked
    /// choice behaves as "every device runs", so a stale setting can never silently stop routines.
    static func gate(primaryId: String, ownId: String, approvedDeviceIds: Set<String>) -> RoutineGate {
        let primary = primaryId.trimmingCharacters(in: .whitespacesAndNewlines)
        if primary.isEmpty || primary == ownId || ownId == "spike-device" { return .full }
        return approvedDeviceIds.contains(primary) ? .failover : .full
    }
}

/// Which device runs routines, shared across the user's devices through settings sync
/// (docs/superpowers/specs/2026-10-08-primary-device-routines-design.md). Empty = unset.
@MainActor
final class RoutineHostSettings: ObservableObject {
    nonisolated static let fieldKey = "routines.primaryDeviceId"
    private static let defaultsKey = "com.mpointer.kestrel.routinePrimaryDeviceId"
    private let defaults: UserDefaults

    @Published var primaryDeviceId: String {
        didSet {
            defaults.set(primaryDeviceId, forKey: Self.defaultsKey)
            SettingsFieldTimestamps.touch(Self.fieldKey)
        }
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        primaryDeviceId = defaults.string(forKey: Self.defaultsKey) ?? ""
    }

    func apply(fieldKey: String, value: String) -> Bool {
        guard fieldKey == Self.fieldKey else { return false }
        primaryDeviceId = value
        return true
    }
}
