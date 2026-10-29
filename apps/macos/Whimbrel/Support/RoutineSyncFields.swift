import Foundation

/// Syncs on/off and schedule time for the routines that can fail over (Daily Brief and Reply
/// Priorities) through settings sync, so a device standing in for the primary uses the same values
/// the primary does (docs/superpowers/specs/2026-10-12-routines-safety-net-design.md). Write-capable
/// routines never fail over, so their settings stay per-device.
enum RoutineSyncFields {
    /// `modelPref` joined on 2026-10-29: a device standing in for the primary should run the
    /// routine on the same model (`default`, `cheapest`, or `<provider>/<model>`).
    enum Field: String { case enabled, schedule, modelPref = "model_pref" }

    enum Change: Equatable {
        case enabled(id: Int64, Bool)
        case schedule(id: Int64, String)
        case modelPref(id: Int64, String)
    }

    static let templates = ["daily_brief", "reply_priorities"]
    private static let prefix = "routine."

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

    private static func isHhmm(_ s: String) -> Bool {
        let p = s.split(separator: ":", omittingEmptySubsequences: false)
        guard p.count == 2, p[0].count == 2, p[1].count == 2, let h = Int(p[0]), let m = Int(p[1]) else { return false }
        return (0...23).contains(h) && (0...59).contains(m)
    }
}
