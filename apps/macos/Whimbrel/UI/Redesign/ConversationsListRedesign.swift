import SwiftUI
import WhimbrelUI
import os

// The Conversations section's list (docs/ui-redesign/screens/conversations.md part 6). New conversation
// at the top, a search over titles, then every conversation, starred first and then most recently updated. Each row
// shows its title, its topic, when it was last updated and the first line of its latest answer; its menu stars,
// renames and deletes it.

typealias ConversationListRow = CoreConversationRow

/// The list's rows, read off the main thread. Main thread only.
@MainActor
final class ConversationsListModel: ObservableObject {
    nonisolated private static let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "app", category: "conversations")

    /// What the list shows once read: its rows, or that the read failed (an error with Try again, never the
    /// empty-state line, so a read error does not look like every conversation is gone).
    enum ListState: Equatable {
        case rows([ConversationListRow])
        case failed

        var rows: [ConversationListRow] {
            if case .rows(let rows) = self { return rows }
            return []
        }
    }

    /// The list's state from a read: nil is a failed read, not an empty list.
    nonisolated static func listState(_ read: [ConversationListRow]?) -> ListState {
        read.map(ListState.rows) ?? .failed
    }

    @Published private(set) var rows: [ConversationListRow] = []
    @Published private(set) var loaded = false
    /// The last read failed: the list shows an error with Try again.
    @Published private(set) var failed = false
    private weak var appState: AppState?
    /// Changes with every load, so an older read landing late is dropped.
    private var serial = 0

    func attach(_ appState: AppState) {
        self.appState = appState
        load()
    }

    /// Reads every row (with its topic and latest answer line) in one core call on the store's queue.
    func load() {
        guard let store = appState?.coreStore else { return }
        serial += 1
        let mark = serial
        CoreMemoryStore.accessQueue.async { [weak self] in
            let rows = store.conversationRows()
            DispatchQueue.main.async { [weak self] in
                guard let self, self.serial == mark else { return }
                if rows == nil { Self.log.error("conversations: could not read the list") }
                let state = Self.listState(rows)
                self.rows = state.rows
                self.failed = state == .failed
                self.loaded = true
            }
        }
    }

    func toggleStar(_ row: ConversationListRow) {
        let id = row.id, starred = !row.starred
        write { $0.setConversationStarred(id: id, starred: starred) }
    }

    func rename(_ row: ConversationListRow, to newTitle: String) {
        let title = newTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, title != row.title else { return }
        let id = row.id
        write { $0.renameConversation(id: id, newTitle: title) }
    }

    func delete(_ row: ConversationListRow) {
        let id = row.id
        write { $0.deleteConversation(id: id) }
    }

    /// Runs a store write on the store's queue, then reads the list again.
    private func write(_ work: @escaping (CoreMemoryStore) -> Bool) {
        guard let store = appState?.coreStore else { return }
        CoreMemoryStore.accessQueue.async { [weak self] in
            if !work(store) { Self.log.error("conversations: a change could not be saved") }
            DispatchQueue.main.async { [weak self] in self?.load() }
        }
    }
}

/// The Conversations section with no conversation open.
struct ConversationsListRedesign: View {
    /// Starred first (regardless of age), then most recently updated: the order core lists them in, and the
    /// Library's sidebar sorts them in.
    static func sorted(_ rows: [CoreMemoryStore.ConversationSummary]) -> [CoreMemoryStore.ConversationSummary] {
        sortedForSidebar(rows)
    }

    /// The name of a row's menu button (Star, Rename, Delete): it holds the conversation's title, so Tab and VoiceOver say which
    /// conversation the menu acts on, not a bare "More".
    static func menuLabel(_ row: ConversationListRow) -> String {
        String(localized: "Actions for \(row.title)",
               comment: "The name of a conversation row's menu button (Star, Rename, Delete), read by VoiceOver; the argument is the conversation's title.")
    }

    /// The rows whose title holds `query`, ignoring case; every row for a blank query.
    static func filter(_ rows: [CoreMemoryStore.ConversationSummary], query: String) -> [CoreMemoryStore.ConversationSummary] {
        let wanted = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !wanted.isEmpty else { return rows }
        return rows.filter { $0.title.range(of: wanted, options: [.caseInsensitive, .diacriticInsensitive]) != nil }
    }

    @EnvironmentObject var appState: AppState
    @StateObject private var model = ConversationsListModel()
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var query = ""
    @State private var renaming: ConversationListRow?
    @State private var renameDraft = ""
    @State private var deleting: ConversationListRow?

    private var shown: [ConversationListRow] {
        let byId = Dictionary(model.rows.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        return Self.filter(Self.sorted(model.rows.map(\.summary)), query: query).compactMap { byId[$0.id] }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: theme.space(16)) {
                HStack(spacing: theme.space(12)) {
                    WButton(String(localized: "New conversation", comment: "The Conversations list's button for an empty conversation about everything, and that page's title until its first answer is in."),
                            glyph: "plus", kind: .primary) { appState.startConversation(scope: .everything, question: nil) }
                    if !model.rows.isEmpty {
                        WSearchBar(text: $query, placeholder: String(localized: "Search"), shortcutHint: nil)
                    }
                }
                content
            }
            .frame(maxWidth: 760 * k, alignment: .leading)
            .padding(.horizontal, theme.space(28))
            .padding(.vertical, theme.space(16))
            .frame(maxWidth: .infinity, alignment: .center)
        }
        .onAppear { model.attach(appState) }
        // A generated title landing (a conversation just continued) shows in its row.
        .onReceive(appState.conversationTitleUpdated.receive(on: DispatchQueue.main)) { _ in model.load() }
        .alert(String(localized: "Rename"), isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField(String(localized: "Title"), text: $renameDraft)
            Button(String(localized: "Save")) {
                if let renaming { model.rename(renaming, to: renameDraft) }
                renaming = nil
            }
            Button(String(localized: "Cancel"), role: .cancel) { renaming = nil }
        }
        .confirmationDialog(
            String(localized: "Delete this conversation?", comment: "Asked before a conversation is deleted from its page."),
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), titleVisibility: .visible
        ) {
            Button(String(localized: "Delete"), role: .destructive) {
                if let deleting { model.delete(deleting) }
                deleting = nil
            }
            Button(String(localized: "Cancel"), role: .cancel) { deleting = nil }
        } message: {
            Text(String(localized: "This can't be undone."))
        }
    }

    @ViewBuilder
    private var content: some View {
        if model.loaded, model.failed {
            WErrorState(title: String(localized: "Couldn't load your conversations.",
                                      comment: "The Conversations list when reading it failed; a Try again button follows."),
                        help: nil, onRetry: model.load)
        } else if model.loaded, model.rows.isEmpty {
            Text(String(localized: "Questions you continue from Today show up here.",
                        comment: "The Conversations list when there are none yet."))
                .wFont(.body).foregroundStyle(theme.textSecondary)
        } else if model.loaded, shown.isEmpty {
            Text(String(localized: "No matches")).wFont(.body).foregroundStyle(theme.textSecondary)
        } else {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(shown) { row in
                    rowView(row)
                    Divider()
                }
            }
        }
    }

    private func rowView(_ row: ConversationListRow) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: theme.space(10)) {
            Button { appState.openConversation(id: row.id) } label: {
                VStack(alignment: .leading, spacing: theme.space(4)) {
                    HStack(spacing: theme.space(6)) {
                        if row.starred {
                            Image(systemName: "star.fill").foregroundStyle(theme.accentPrimary)
                                .accessibilityLabel(String(localized: "Star"))
                        }
                        Text(verbatim: row.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).lineLimit(1)
                    }
                    if let line = row.latestAnswerLine {
                        Text(verbatim: line).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1)
                    }
                    HStack(spacing: theme.space(8)) {
                        if let topic = row.topic { ConversationTopicTag(name: topic.name) }
                        Text(Date(timeIntervalSince1970: Double(row.updatedAtUnixMs) / 1000)
                            .formatted(.relative(presentation: .named)))
                            .wFont(.caption).foregroundStyle(theme.textSecondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, theme.space(10))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .contextMenu { menuItems(row) }
            Menu { menuItems(row) } label: { Image(systemName: "ellipsis.circle") }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .accessibilityLabel(Self.menuLabel(row))
        }
    }

    @ViewBuilder
    private func menuItems(_ row: ConversationListRow) -> some View {
        Button(row.starred ? String(localized: "Unstar") : String(localized: "Star")) { model.toggleStar(row) }
        Button(String(localized: "Rename")) {
            renameDraft = row.title
            renaming = row
        }
        Divider()
        Button(String(localized: "Delete"), role: .destructive) { deleting = row }
    }
}

/// A conversation's topic in a list row: shown, not chosen here (the conversation page's chip changes it).
private struct ConversationTopicTag: View {
    @Environment(\.wTheme) private var theme
    let name: String

    var body: some View {
        HStack(spacing: theme.space(4)) {
            Image(systemName: "number").accessibilityHidden(true)
            Text(verbatim: name).lineLimit(1)
        }
        .wFont(.caption)
        .foregroundStyle(theme.textSecondary)
        .padding(.horizontal, theme.space(6))
        .padding(.vertical, theme.space(2))
        .overlay(Capsule().strokeBorder(theme.border, lineWidth: max(1, theme.borderWidth)))
    }
}
