import Foundation

struct ProfileFact: Codable, Identifiable, Equatable {
    var id: String
    var text: String
    var sourceMeetingId: Int64?
}

enum ProfileComposer {
    static let maxChars = 2000
    static let maxFactChars = 200
    static let maxFacts = 30

    /// The block core receives as `RoutingConfig.profile`: labelled lines, blank ones skipped, capped.
    static func compose(name: String, role: String, organization: String, about: String, facts: [ProfileFact]) -> String {
        var lines: [String] = []
        for (label, value) in [("Name", name), ("Role", role), ("Organization", organization), ("About", about)] {
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty { lines.append("\(label): \(trimmed)") }
        }
        let texts = facts.map { $0.text.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        if !texts.isEmpty { lines.append("Facts:\n" + texts.map { "- \($0)" }.joined(separator: "\n")) }
        return String(lines.joined(separator: "\n").prefix(maxChars))
    }

    /// Lowercase, collapse whitespace, drop trailing punctuation: the key for "same fact" (mirrors core).
    static func normalize(_ text: String) -> String {
        let collapsed = text.split(whereSeparator: \.isWhitespace).joined(separator: " ").lowercased()
        return String(collapsed.reversed().drop(while: { $0.isPunctuation }).reversed())
    }
}

/// The user's Profile: four fields and a list of facts, per person, so synced. Reaches core as
/// `RoutingConfig.profile` (empty when the switch is off). See
/// docs/superpowers/specs/2026-10-06-profile-design.md.
@MainActor
final class ProfileSettings: ObservableObject {
    private static let prefix = "com.mpointer.kestrel.profile."
    private let defaults: UserDefaults

    nonisolated static func fieldKey(_ name: String) -> String { "profile.\(name)" }

    @Published var name: String { didSet { store("name", name) } }
    @Published var role: String { didSet { store("role", role) } }
    @Published var organization: String { didSet { store("organization", organization) } }
    @Published var about: String { didSet { store("about", about) } }
    @Published var facts: [ProfileFact] { didSet { storeFacts() } }
    @Published var enabled: Bool { didSet { store("enabled", enabled ? "true" : "false") } }
    @Published var suggestEnabled: Bool { didSet { store("suggest_enabled", suggestEnabled ? "true" : "false") } }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        func string(_ key: String) -> String { defaults.string(forKey: Self.prefix + key) ?? "" }
        func flag(_ key: String) -> Bool { defaults.object(forKey: Self.prefix + key) == nil ? true : defaults.string(forKey: Self.prefix + key) == "true" }
        name = string("name")
        role = string("role")
        organization = string("organization")
        about = string("about")
        enabled = flag("enabled")
        suggestEnabled = flag("suggest_enabled")
        facts = (try? JSONDecoder().decode([ProfileFact].self, from: Data(string("facts").utf8))) ?? []
    }

    private func store(_ key: String, _ value: String) {
        defaults.set(value, forKey: Self.prefix + key)
        SettingsFieldTimestamps.touch(Self.fieldKey(key))
    }

    private func storeFacts() {
        store("facts", Self.encode(facts))
    }

    private static func encode(_ facts: [ProfileFact]) -> String {
        (try? JSONEncoder().encode(facts)).flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
    }

    /// What core receives: empty when the switch is off or there is nothing written.
    var routingProfile: String {
        guard enabled else { return "" }
        return ProfileComposer.compose(name: name, role: role, organization: organization, about: about, facts: facts)
    }

    var factTexts: [String] { facts.map(\.text) }

    /// True once the user has written anything (a field or a fact).
    var hasContent: Bool {
        [name, role, organization, about].contains { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } || !facts.isEmpty
    }

    /// Whether the daily suggestion job may call a model: the switch is on AND the user has
    /// engaged with the Profile. A profile nobody has written to never causes a send.
    var suggestionsActive: Bool { suggestEnabled && hasContent }

    /// A fact is one line: whitespace runs (including newlines) collapse to single spaces, so a fact
    /// cannot open a new section of a prompt.
    private static func oneLine(_ text: String) -> String {
        text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    private func isAcceptable(_ text: String, excluding id: String? = nil) -> Bool {
        let t = Self.oneLine(text)
        guard !t.isEmpty, t.count <= ProfileComposer.maxFactChars else { return false }
        let key = ProfileComposer.normalize(t)
        return !facts.contains { $0.id != id && ProfileComposer.normalize($0.text) == key }
    }

    @discardableResult
    func addFact(_ text: String, sourceMeetingId: Int64?) -> Bool {
        guard facts.count < ProfileComposer.maxFacts, isAcceptable(text) else { return false }
        facts.append(ProfileFact(id: UUID().uuidString, text: Self.oneLine(text), sourceMeetingId: sourceMeetingId))
        return true
    }

    @discardableResult
    func updateFact(id: String, text: String) -> Bool {
        guard let index = facts.firstIndex(where: { $0.id == id }), isAcceptable(text, excluding: id) else { return false }
        facts[index].text = Self.oneLine(text)
        return true
    }

    func removeFact(id: String) { facts.removeAll { $0.id == id } }

    // MARK: settings sync

    /// Current values keyed for `SettingsSyncCoordinator.gatherLocalSettings`.
    func syncValues() -> [(key: String, value: String)] {
        [
            (Self.fieldKey("name"), name), (Self.fieldKey("role"), role),
            (Self.fieldKey("organization"), organization), (Self.fieldKey("about"), about),
            (Self.fieldKey("facts"), Self.encode(facts)),
            (Self.fieldKey("enabled"), enabled ? "true" : "false"),
            (Self.fieldKey("suggest_enabled"), suggestEnabled ? "true" : "false"),
        ]
    }

    /// Applies one pulled value; false (not applied) on an unknown key or a value that does not parse.
    func apply(fieldKey: String, value: String) -> Bool {
        switch fieldKey {
        case Self.fieldKey("name"): name = value
        case Self.fieldKey("role"): role = value
        case Self.fieldKey("organization"): organization = value
        case Self.fieldKey("about"): about = value
        case Self.fieldKey("facts"):
            guard let decoded = try? JSONDecoder().decode([ProfileFact].self, from: Data(value.utf8)) else { return false }
            facts = decoded
        case Self.fieldKey("enabled"), Self.fieldKey("suggest_enabled"):
            guard value == "true" || value == "false" else { return false }
            if fieldKey == Self.fieldKey("enabled") { enabled = value == "true" } else { suggestEnabled = value == "true" }
        default: return false
        }
        return true
    }
}
