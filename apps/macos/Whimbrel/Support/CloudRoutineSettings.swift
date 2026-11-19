import Foundation

/// The cloud routine runner's settings on this Mac (docs/superpowers/specs/2026-11-15-cloud-routine-runner-design.md):
/// which routines run in the cloud and their ready-by times (synced as `routine.<template>.cloud|cloudGrant|readyBy`,
/// see `RoutineSyncFields`), the runner's last run (`cloud.runner.lastRun`, written only by the runner and never
/// pushed from here), and the account's time zone (`profile.timeZone`, pushed from here when this Mac's changes).
@MainActor
final class CloudRoutineSettings: ObservableObject {
    nonisolated static let heartbeatKey = "cloud.runner.lastRun"
    nonisolated static let timeZoneKey = "profile.timeZone"
    private static let cloudDefaultsKey = "com.mpointer.whimbrel.cloudRoutines"
    private static let readyByDefaultsKey = "com.mpointer.whimbrel.cloudReadyBy"
    private static let lastRunDefaultsKey = "com.mpointer.whimbrel.cloudRunnerLastRun"
    private static let pushedZoneDefaultsKey = "com.mpointer.whimbrel.cloudPushedTimeZone"
    private let defaults: UserDefaults

    /// Template name to "run in the cloud"; only the cloud templates are ever set.
    @Published private(set) var cloud: [String: Bool]
    /// Template name to its ready-by "HH:MM", once chosen.
    @Published private(set) var readyBy: [String: String]
    /// The runner's last round (unix ms), as its heartbeat said; nil until one is pulled.
    @Published private(set) var runnerLastRunMs: Int64?

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        cloud = defaults.dictionary(forKey: Self.cloudDefaultsKey) as? [String: Bool] ?? [:]
        readyBy = defaults.dictionary(forKey: Self.readyByDefaultsKey) as? [String: String] ?? [:]
        runnerLastRunMs = (defaults.object(forKey: Self.lastRunDefaultsKey) as? NSNumber)?.int64Value
    }

    func isCloud(_ template: String) -> Bool { cloud[template] ?? false }

    /// The templates flagged for the cloud, for the routine tick.
    var cloudTemplates: [String] { RoutineSyncFields.cloudTemplates.filter(isCloud) }

    /// A person's choice in Settings. The grant is re-stamped with it, so the two always travel together.
    /// Turning it on is refused for a routine that is off (`routineEnabled`); turning it off is always allowed.
    func setCloud(_ on: Bool, for template: String, routineEnabled: Bool) {
        guard RoutineSyncFields.cloudTemplates.contains(template) else { return }
        // Never "on" for a routine that is off: the stamping below would push enabled=false over another device's on.
        if on && !routineEnabled { return }
        storeCloud(on, for: template)
        SettingsFieldTimestamps.touch(RoutineSyncFields.cloudKey(template: template, field: .cloud))
        SettingsFieldTimestamps.touch(RoutineSyncFields.cloudKey(template: template, field: .cloudGrant))
        // The runner reads this routine's on/off, time and model from sync. Fields never edited here go out at
        // timestamp 0 and lose to any other device's value, so the Mac that hands the routine to the cloud stamps
        // its own values now. Only for a routine that is on, so "off" is never stamped.
        if on && routineEnabled {
            for field in [RoutineSyncFields.Field.enabled, .schedule, .modelPref] {
                RoutineSyncFields.noteLocalChange(template: template, field: field)
            }
        }
    }

    func setReadyBy(_ hhmm: String, for template: String) {
        guard RoutineSyncFields.cloudTemplates.contains(template), RoutineSyncFields.isHhmm(hhmm) else { return }
        storeReadyBy(hhmm, for: template)
        SettingsFieldTimestamps.touch(RoutineSyncFields.cloudKey(template: template, field: .readyBy))
    }

    /// The fields to push: the cloud fields (with core's grant lists) and this Mac's time zone. Never the heartbeat.
    func syncValues(timeZone: TimeZone = .current) -> [String: String] {
        var grants: [String: [String]] = [:]
        for template in RoutineSyncFields.cloudTemplates {
            grants[template] = CloudRoutineInfo.load(template)?.grant ?? []
        }
        var out = RoutineSyncFields.cloudValues(cloud: cloud, readyBy: readyBy, grants: grants)
        out[Self.timeZoneKey] = timeZoneForPush(timeZone)
        return out
    }

    /// This Mac's zone. Its timestamp moves only when the zone differs from the one last pushed from here, so two
    /// Macs in different zones do not take turns overwriting each other: the one whose zone changed last wins.
    private func timeZoneForPush(_ zone: TimeZone) -> String {
        let id = zone.identifier
        if defaults.string(forKey: Self.pushedZoneDefaultsKey) != id {
            defaults.set(id, forKey: Self.pushedZoneDefaultsKey)
            SettingsFieldTimestamps.touch(Self.timeZoneKey)
        }
        return id
    }

    /// Applies one pulled field this type owns. False (not applied, so it is pulled again or ignored) for a deleted
    /// field, an unknown `cloud.*` key or a value that does not parse.
    func apply(fieldKey: String, value: String, isDeleted: Bool) -> Bool {
        guard !isDeleted else { return false }
        if fieldKey == Self.heartbeatKey {
            guard let ms = Int64(value.trimmingCharacters(in: .whitespaces)), ms > 0 else { return false }
            runnerLastRunMs = ms
            defaults.set(NSNumber(value: ms), forKey: Self.lastRunDefaultsKey)
            return true
        }
        // Another device's zone: accepted (the latest change wins); this Mac pushes its own only when it changes.
        if fieldKey == Self.timeZoneKey { return !value.isEmpty }
        switch RoutineSyncFields.cloudChange(key: fieldKey, value: value) {
        case let .cloud(template, on)?: storeCloud(on, for: template)
        case let .readyBy(template, at)?: storeReadyBy(at, for: template)
        // The grant is core's list for the template: nothing to store, and the pulled one needs no retry.
        case .cloudGrant?: break
        case nil: return false
        }
        return true
    }

    /// Whether this type owns a pulled key (the coordinator routes it here).
    nonisolated static func owns(_ fieldKey: String) -> Bool {
        fieldKey.hasPrefix("cloud.") || fieldKey == timeZoneKey || RoutineSyncFields.isCloudKey(fieldKey)
    }

    private func storeCloud(_ on: Bool, for template: String) {
        cloud[template] = on
        defaults.set(cloud, forKey: Self.cloudDefaultsKey)
    }

    private func storeReadyBy(_ hhmm: String, for template: String) {
        readyBy[template] = hhmm
        defaults.set(readyBy, forKey: Self.readyByDefaultsKey)
    }
}
