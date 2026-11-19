import Foundation

/// Syncs on/off and schedule time for the routines that can fail over (Daily Brief and Reply
/// Priorities) through settings sync, so a device standing in for the primary uses the same values
/// the primary does (docs/superpowers/specs/2026-10-12-routines-safety-net-design.md), and for the
/// routines that may run in the cloud (EOD Recap, Weekly Digest, Meeting Prep too), whose runner reads
/// them (docs/superpowers/specs/2026-11-15-cloud-routine-runner-design.md, Amendment 3). Focus Time
/// writes to the calendar and stays per-device.
///
/// The cloud routines also sync `routine.<template>.cloud` ("true"/"false"), `.cloudGrant` (a JSON array
/// of core's `routines::grant_for` entries while in the cloud, `[]` otherwise; informational: the runner
/// enforces its own copy of the grant) and `.readyBy` ("HH:MM", only once chosen). These live in
/// `CloudRoutineSettings`, not in the routine rows.
enum RoutineSyncFields {
    /// `modelPref` joined on 2026-10-29: a device standing in for the primary should run the
    /// routine on the same model (`default`, `cheapest`, or `<provider>/<model>`).
    enum Field: String { case enabled, schedule, modelPref = "model_pref" }

    enum Change: Equatable {
        case enabled(id: Int64, Bool)
        case schedule(id: Int64, String)
        case modelPref(id: Int64, String)
    }

    /// The routine-row fields above sync for these.
    static let templates = ["daily_brief", "reply_priorities", "eod_recap", "weekly_digest", "meeting_prep"]
    /// The templates core lets run in the cloud (`routines::cloud_eligible`); Reply Priorities is not one.
    static let cloudTemplates = ["daily_brief", "eod_recap", "weekly_digest", "meeting_prep"]
    private static let prefix = "routine."

    enum CloudField: String { case cloud, cloudGrant, readyBy }

    enum CloudChange: Equatable {
        case cloud(template: String, Bool)
        case cloudGrant(template: String, [String])
        case readyBy(template: String, String)
    }

    static func cloudKey(template: String, field: CloudField) -> String { "\(prefix)\(template).\(field.rawValue)" }

    static func isCloudKey(_ key: String) -> Bool { parseCloud(key) != nil }

    /// The cloud fields to push. `grants`: core's grant list per template, pushed only while it is in the cloud.
    static func cloudValues(cloud: [String: Bool], readyBy: [String: String], grants: [String: [String]]) -> [String: String] {
        var out: [String: String] = [:]
        for template in cloudTemplates {
            let on = cloud[template] ?? false
            out[cloudKey(template: template, field: .cloud)] = on ? "true" : "false"
            let grant = on ? (grants[template] ?? []) : []
            let json = (try? JSONEncoder().encode(grant)).flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
            out[cloudKey(template: template, field: .cloudGrant)] = json
            if let at = readyBy[template], isHhmm(at) {
                out[cloudKey(template: template, field: .readyBy)] = at
            }
        }
        return out
    }

    /// The local edit a pulled cloud field asks for, or nil when it is unknown or malformed.
    static func cloudChange(key: String, value: String) -> CloudChange? {
        guard let (template, field) = parseCloud(key) else { return nil }
        switch field {
        case .cloud:
            guard value == "true" || value == "false" else { return nil }
            return .cloud(template: template, value == "true")
        case .cloudGrant:
            guard let list = try? JSONDecoder().decode([String].self, from: Data(value.utf8)) else { return nil }
            return .cloudGrant(template: template, list)
        case .readyBy:
            guard isHhmm(value) else { return nil }
            return .readyBy(template: template, value)
        }
    }

    private static func parseCloud(_ key: String) -> (String, CloudField)? {
        let parts = key.split(separator: ".", omittingEmptySubsequences: false).map(String.init)
        guard parts.count == 3, parts[0] == "routine", cloudTemplates.contains(parts[1]), let field = CloudField(rawValue: parts[2]) else { return nil }
        return (parts[1], field)
    }

    static func key(template: String, field: Field) -> String { "\(prefix)\(template).\(field.rawValue)" }

    static func isRoutineKey(_ key: String) -> Bool { parse(key) != nil }

    /// Call after a local edit so the next sync pushes it.
    static func noteLocalChange(template: String, field: Field) {
        guard templates.contains(template) else { return }
        SettingsFieldTimestamps.touch(key(template: template, field: field))
    }

    /// The fields to push, read from this device's routine rows.
    static func values(from routines: [CoreRoutine]) -> [String: String] {
        var out: [String: String] = [:]
        for r in routines where templates.contains(r.template) {
            out[key(template: r.template, field: .enabled)] = r.enabled ? "true" : "false"
            out[key(template: r.template, field: .schedule)] = r.scheduleHhmm
            out[key(template: r.template, field: .modelPref)] = r.modelPref
        }
        return out
    }

    /// The local edit a pulled field asks for, or nil when it is unknown, malformed, or this device has
    /// no routine of that template yet (the caller then leaves the timestamp alone, so it retries).
    static func change(key: String, value: String, routines: [CoreRoutine]) -> Change? {
        guard let (template, field) = parse(key), let routine = routines.first(where: { $0.template == template }) else { return nil }
        switch field {
        case .enabled:
            guard value == "true" || value == "false" else { return nil }
            return .enabled(id: routine.id, value == "true")
        case .schedule:
            guard isHhmm(value) else { return nil }
            return .schedule(id: routine.id, value)
        case .modelPref:
            guard isModelPref(value) else { return nil }
            return .modelPref(id: routine.id, value)
        }
    }

    /// The three shapes core accepts (`api::routing::RoutineModelPref`): `default`,
    /// `cheapest`, or `<provider>/<model>` with a provider core knows. OpenRouter model ids
    /// carry their own slash, so only the first one splits.
    static func isModelPref(_ value: String) -> Bool {
        if value == "default" || value == "cheapest" { return true }
        guard let slash = value.firstIndex(of: "/") else { return false }
        let provider = String(value[..<slash])
        let model = value[value.index(after: slash)...]
        return ["ollama", "anthropic", "openai", "google", "openrouter"].contains(provider) && !model.isEmpty
    }

    private static func parse(_ key: String) -> (String, Field)? {
        let parts = key.split(separator: ".", omittingEmptySubsequences: false).map(String.init)
        guard parts.count == 3, parts[0] == "routine", templates.contains(parts[1]), let field = Field(rawValue: parts[2]) else { return nil }
        return (parts[1], field)
    }

    static func isHhmm(_ s: String) -> Bool {
        let p = s.split(separator: ":", omittingEmptySubsequences: false)
        guard p.count == 2, p[0].count == 2, p[1].count == 2, let h = Int(p[0]), let m = Int(p[1]) else { return false }
        return (0...23).contains(h) && (0...59).contains(m)
    }
}
