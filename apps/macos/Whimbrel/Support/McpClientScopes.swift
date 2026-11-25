import Foundation

/// The permissions an MCP client token can hold, and the rules for choosing them. Core
/// checks each tool against these names (`core/mcp-server`). `google_data` only widens the
/// others: it lets the tools return Gmail and Google Calendar items and the titles, people
/// and summaries of Google Calendar meetings, so on its own it opens nothing.
enum McpClientScopes {
    static let googleData = "google_data"
    /// Lets a client add to memory, and read nothing. The form does not offer it yet (bridge
    /// phase 3 adds the checkbox and its confirmation), but a client holding it is listed.
    static let write = "write"

    /// Every permission the form lists, in its order.
    static let all = ["memory", "meetings", "transcripts", googleData]

    /// Every permission core knows, in its order: the form's, then `write`.
    static let known = all + [write]

    /// What a person reads for a scope id; an id this build doesn't know shows as itself.
    static func label(_ scope: String) -> String {
        switch scope {
        case googleData: return String(localized: "Gmail and Google Calendar")
        case write: return String(localized: "Write")
        case "memory", "meetings", "transcripts": return scope.capitalized
        default: return scope
        }
    }

    /// The comma-separated names shown on a client's row.
    static func line(_ scopes: [String]) -> String {
        scopes.map(label).joined(separator: ", ")
    }

    /// A token needs a name and at least one thing to read or `write`; `google_data` alone
    /// opens nothing.
    static func canCreate(name: String, scopes: Set<String>) -> Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && scopes.contains { $0 != googleData && known.contains($0) }
    }

    /// The chosen scopes in core's order, each once, without any id core doesn't know.
    static func ordered(_ scopes: Set<String>) -> [String] {
        known.filter { scopes.contains($0) }
    }
}
