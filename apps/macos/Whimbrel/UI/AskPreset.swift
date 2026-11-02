import Foundation

/// A prepared Ask, offered as a chip above the meeting Ask field. The exchange shows the
/// chip's label; the model gets `prompt`. Typing the label runs it too.
enum AskPreset: String, CaseIterable, Identifiable {
    case makeMeSoundSmart

    var id: String { rawValue }

    var label: String {
        switch self {
        case .makeMeSoundSmart: return String(localized: "Make me sound smart")
        }
    }

    /// The label in English, so typing it works in any app language.
    private var englishLabel: String {
        switch self {
        case .makeMeSoundSmart: return "Make me sound smart"
        }
    }

    /// English on purpose: the answer's language follows Settings through the routing config,
    /// and the profile reaches the model through the Ask instructions.
    var prompt: String {
        switch self {
        case .makeMeSoundSmart:
            return """
            Make me sound smart in this meeting. From the transcript so far, answer in Markdown with these sections:
            ## Where we are
            Two sentences on what's being discussed right now.
            ## Key points
            The main points and any decisions so far, as short bullets.
            ## Where I come in
            Points that connect to my profile (my role, projects and expertise), and why they matter to me. Leave this section out if nothing connects.
            ## Say this next
            Two or three smart questions or contributions I could make right now, worded the way I'd say them.
            Keep it short enough to read in under a minute.
            """
        }
    }

    /// The preset whose label `text` is, ignoring case, surrounding space and end punctuation.
    static func matching(_ text: String) -> AskPreset? {
        let key = { (s: String) in s.trimmingCharacters(in: .whitespacesAndNewlines.union(.punctuationCharacters)).lowercased() }
        let typed = key(text)
        return allCases.first { key($0.label) == typed || key($0.englishLabel) == typed }
    }
}
