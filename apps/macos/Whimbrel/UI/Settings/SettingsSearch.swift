import SwiftUI

/// One searchable setting. `id` is also the scroll anchor on the row (`.settingsAnchor(id)`).
/// Titles repeat the control's English label; keywords are the other words people reach for.
/// Setting values are never indexed. Search text is English only and is not localized.
struct SettingsSearchEntry: Equatable {
    let id: String
    let title: String
    let section: SettingsSection
    let keywords: [String]
    /// False for a card the classic panes do not have (Tell a friend): it has no `.settingsAnchor` call there. The redrawn
    /// section carries its id through `SettingsAnchorID`, as for every redrawn row.
    var inClassicPane = true
}

enum SettingsSearch {
    private static func e(_ id: String, _ title: String, _ section: SettingsSection, _ keywords: [String] = []) -> SettingsSearchEntry {
        SettingsSearchEntry(id: id, title: title, section: section, keywords: keywords)
    }

    static let entries: [SettingsSearchEntry] = [
        // General
        e("general.update", "Check for Updates", .general, ["update", "version", "upgrade", "restart", "new version"]),
        e("general.language", "Language", .general, ["locale", "translate", "translation", "espanol", "french", "german"]),
        e("general.writeIn", "Write notes and summaries in", .general, ["language", "output language", "summaries"]),
        e("general.appearance", "Appearance", .general, ["dark mode", "light mode", "theme", "system", "colors"]),
        e("general.permissions", "Permissions", .general, ["microphone", "accessibility", "screen recording", "permission", "notifications", "access"]),
        e("general.skin", "Skin", .general, ["theme", "gallery", "colors", "make a skin", "look"]),
        // Profile
        e("profile.use", "Use my profile in summaries and answers", .profile, ["about me", "personalize", "context"]),
        e("profile.suggest", "Suggest additions from my meetings", .profile, ["suggestions", "learned facts"]),
        e("profile.about", "About me and my work", .profile, ["name", "role", "organization", "bio"]),
        e("profile.facts", "Learned facts", .profile, ["memory", "remember", "facts"]),
        // Plan & billing
        e("plan.summary", "Plan & billing", .plan, ["plus", "cloud", "subscription", "refund", "invoice", "cancel", "upgrade", "price", "payment"]),
        e("plan.manage", "Manage subscription", .plan, ["cancel", "refund", "invoice", "payment method", "stripe", "billing portal"]),
        e("plan.provider", "Use Whimbrel AI as your provider", .plan, ["allowance", "credit", "plus ai", "hosted ai"]),
        // Capture & Privacy
        e("capture.yourData", "Your data", .capture, ["privacy", "what leaves this mac", "storage", "folder"]),
        e("capture.export", "Export everything", .capture, ["download", "backup", "markdown", "json", "take my data"]),
        e("capture.delete", "Delete everything on this Mac", .capture, ["erase", "wipe", "remove data"]),
        e("capture.obsidian", "Keep an Obsidian vault up to date", .capture, ["notes", "markdown", "vault"]),
        e("capture.encryption", "Database encryption", .capture, ["encrypt", "passphrase", "database", "protect", "password", "key"]),
        e("capture.screenCaptures", "Show screen captures in Library", .capture, ["recent", "screen text", "ocr"]),
        e("capture.excludeApps", "Exclude apps from screen-text capture", .capture, ["ignore", "block", "blocklist", "privacy", "password manager"]),
        e("capture.excludeSites", "Sites never captured", .capture, ["websites", "domains", "block", "private"]),
        e("capture.hotkey", "Hotkey panel", .capture, ["shortcut", "keyboard", "global", "ask"]),
        e("capture.retention", "Retention", .capture, ["delete old", "keep", "history", "days", "purge", "expire"]),
        e("capture.media", "Media", .capture, ["media folder", "audio", "recordings", "slides", "location", "disk"]),
        e("capture.storage", "Storage", .capture, ["free up space", "disk", "space", "compact", "clean up"]),
        // Meetings
        e("meetings.recordingNotice", "Remind me to tell people I'm recording", .meetings, ["consent", "notice", "disclosure", "legal"]),
        e("meetings.autoStart", "Auto-start recording for calendar meetings", .meetings, ["calendar", "prompt", "lead time", "automatic"]),
        e("meetings.liveTranscript", "Live transcript during meetings", .meetings, ["captions", "realtime", "real time"]),
        e("meetings.heuristic", "Detect meetings with no calendar entry", .meetings, ["zoom", "teams", "detection", "ad hoc"]),
        e("meetings.snapshots", "Save slides and shared screens", .meetings, ["snapshots", "presentation", "screenshots"]),
        e("meetings.voices", "Remember people's voices", .meetings, ["speaker", "names", "diarization", "voice"]),
        e("meetings.transcription", "Transcription", .meetings, ["whisper", "model", "quality", "speed", "accuracy", "diarization"]),
        e("meetings.multilingual", "I have meetings in other languages", .meetings, ["multilingual", "language", "translate"]),
        // AI
        e("ai.localOnly", "Local-only mode", .ai, ["offline", "private", "ollama", "no cloud", "privacy"]),
        e("ai.whimbrelAI", "Whimbrel AI", .ai, ["allowance", "credit", "balance", "plus", "prepaid", "top up"]),
        e("ai.anthropicKey", "Anthropic API Key", .ai, ["api key", "claude", "key"]),
        e("ai.openaiKey", "OpenAI API Key", .ai, ["api key", "gpt", "chatgpt", "key"]),
        e("ai.googleKey", "Google API Key", .ai, ["api key", "gemini", "key"]),
        e("ai.openrouterKey", "OpenRouter API Key", .ai, ["api key", "key"]),
        e("ai.ollama", "Ollama", .ai, ["local model", "offline", "llm", "gemma"]),
        e("ai.provider", "Default provider", .ai, ["preferred provider", "routing", "which ai"]),
        e("ai.model", "Default model", .ai, ["preferred model", "choose model", "llm"]),
        e("ai.dailyCap", "Daily cap per identity", .ai, ["cents", "limit", "spend", "budget", "cost"]),
        e("ai.spend", "Spend today", .ai, ["usage", "cost", "budget"]),
        e("ai.usage", "Model usage", .ai, ["cost", "tokens", "spending", "last 30 days"]),
        e("ai.typesafe", "Reply priorities", .ai, ["typesafe", "email priority", "rank", "reply"]),
        e("ai.prompts", "Prompts", .ai, ["instructions", "custom", "tune", "reset"]),
        // Topics
        e("topics.list", "Topics", .topics, ["customers", "projects", "products", "clients"]),
        e("topics.add", "Add topic", .topics, ["new topic", "aliases"]),
        e("topics.names", "Names I go by", .topics, ["my name", "nicknames", "steps"]),
        // Sync & Devices
        e("devices.overview", "Devices", .devices, ["sync", "macs", "sign in", "approve", "account"]),
        e("devices.syncNow", "Sync now", .devices, ["sync", "refresh", "upload"]),
        e("devices.name", "This device's name", .devices, ["rename", "device name"]),
        e("devices.phone", "Whimbrel on your phone", .devices, ["iphone", "android", "mobile", "phone", "beta", "qr"]),
        e("devices.deleteAccount", "Delete my sync account", .devices, ["remove account", "erase"]),
        SettingsSearchEntry(id: "devices.tellAFriend", title: "Tell a friend", section: .devices, keywords: ["share", "recommend", "invite", "link", "copy message", "friend"], inClassicPane: false),
        // Connectors
        e("connectors.calendar", "Google Calendar", .connectors, ["calendar", "accounts", "connect", "add account"]),
        e("connectors.gmail", "Gmail", .connectors, ["email", "mail", "google", "accounts"]),
        e("connectors.outlook", "Outlook 365", .connectors, ["microsoft", "email", "calendar", "office"]),
        e("connectors.ownClient", "Use your own Google Cloud project", .connectors, ["oauth", "client id", "client secret"]),
        e("connectors.mcp", "MCP Clients", .connectors, ["claude desktop", "token", "server", "assistant", "revoke"]),
    ]

    /// Entries matching `query` in the title, a keyword or the section's title: case- and
    /// diacritic-insensitive, never a regular expression. Title matches come first.
    static func results(for query: String) -> [SettingsSearchEntry] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return [] }
        var titleHits: [SettingsSearchEntry] = []
        var otherHits: [SettingsSearchEntry] = []
        for entry in entries {
            if entry.title.localizedStandardContains(q) {
                titleHits.append(entry)
            } else if entry.keywords.contains(where: { $0.localizedStandardContains(q) })
                        || entry.section.title.localizedStandardContains(q) {
                otherHits.append(entry)
            }
        }
        return titleHits + otherHits
    }
}

/// Which anchor was just chosen from search, so its row can tint for a moment.
@MainActor
final class SettingsSearchState: ObservableObject {
    @Published private(set) var highlighted: String?
    private var clearTask: Task<Void, Never>?

    func flash(_ id: String) {
        clearTask?.cancel()
        highlighted = id
        clearTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            guard !Task.isCancelled else { return }
            self?.highlighted = nil
        }
    }
}

private struct SettingsSearchStateKey: EnvironmentKey {
    static let defaultValue: SettingsSearchState? = nil
}

extension EnvironmentValues {
    var settingsSearchState: SettingsSearchState? {
        get { self[SettingsSearchStateKey.self] }
        set { self[SettingsSearchStateKey.self] = newValue }
    }
}

private struct SettingsAnchorTint: View {
    let id: String
    @Environment(\.settingsSearchState) private var state

    var body: some View {
        if let state { Observed(state: state, id: id) }
    }

    private struct Observed: View {
        @ObservedObject var state: SettingsSearchState
        let id: String

        var body: some View {
            RoundedRectangle(cornerRadius: Radius.control)
                .fill(Color.whimbrelSelection)
                .padding(-4)
                .opacity(state.highlighted == id ? 1 : 0)
                .animation(.easeInOut(duration: 0.3), value: state.highlighted)
                .allowsHitTesting(false)
        }
    }
}

extension View {
    /// Marks a settings row as a search target: the scroll anchor, plus a brief highlight when chosen from search.
    func settingsAnchor(_ id: String) -> some View {
        self.id(id).background { SettingsAnchorTint(id: id) }
    }
}
