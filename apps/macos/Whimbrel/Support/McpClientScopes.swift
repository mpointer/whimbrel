import Foundation

/// The permissions an MCP client token can hold, and the rules for choosing them. Core
/// checks each tool against these names (`core/mcp-server`). `google_data` only widens the
/// others: it lets the tools return Gmail and Google Calendar items and the titles, people
/// and summaries of Google Calendar meetings, so on its own it opens nothing.
enum McpClientScopes {
    static let googleData = "google_data"

    /// Every permission, in the order the form lists them.
    static let all = ["memory", "meetings", "transcripts", googleData]

    /// What a person reads for a scope id; an id this build doesn't know shows as itself.
    static func label(_ scope: String) -> String {
        switch scope {
        case googleData: return String(localized: "Gmail and Google Calendar")
        case "memory", "meetings", "transcripts": return scope.capitalized
        default: return scope
        }
    }

    /// The comma-separated names shown on a client's row.
    static func line(_ scopes: [String]) -> String {
        scopes.map(label).joined(separator: ", ")
    }

    /// A token needs a name and at least one thing to read; `google_data` alone reads nothing.
    static func canCreate(name: String, scopes: Set<String>) -> Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && scopes.contains { $0 != googleData && all.contains($0) }
    }

    /// The chosen scopes in form order, each once, without any id core doesn't know.
    static func ordered(_ scopes: Set<String>) -> [String] {
        all.filter { scopes.contains($0) }
    }
}
