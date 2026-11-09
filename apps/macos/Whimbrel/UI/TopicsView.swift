import SwiftUI

/// The Topics tab: named topics grouped by kind on the left, the chosen topic's page on the
/// right. Reads run on `CoreMemoryStore.accessQueue`; reloads on appear, when the selection
/// changes and every 30 s while visible.
struct TopicsView: View {
    @EnvironmentObject var appState: AppState
    @State private var topics: [TopicListEntry] = []
    @State private var page: TopicPage?
    @State private var hasLoaded = false
    @State private var errorText: String?

    private static let groups: [(kind: String, title: String)] = [
        ("customer", String(localized: "Customer")), ("product", String(localized: "Product")), ("project", String(localized: "Project")),
        ("subject", String(localized: "Subject")), ("other", String(localized: "Other")),
    ]

    var body: some View {
        HStack(spacing: 0) {
            list.frame(width: 240)
            Divider().overlay(Color.whimbrelBorder)
            Group {
                if let page {
                    // A new identity per topic resets the page's expanded sections and drafts.
                    TopicPageView(page: page, errorText: errorText, run: run)
                        .id(page.topic.topicId)
                } else {
                    Text(hasLoaded && topics.isEmpty
                         ? String(localized: "No topics yet. Name the customers, products and projects you care about in Settings > Topics.")
                         : hasLoaded ? String(localized: "Choose a topic.") : "")
                        .font(.whimbrelBody(.callout))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                        .padding(Spacing.xl)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(Color.whimbrelBackgroundPage)
        .task(id: appState.selectedTopicId) {
            while !Task.isCancelled {
                reload()
                try? await Task.sleep(for: .seconds(30))
            }
        }
    }

    private var list: some View {
        List(selection: $appState.selectedTopicId) {
            ForEach(Self.groups, id: \.kind) { group in
                let entries = topics.filter { kindGroup($0.kind) == group.kind }
                if !entries.isEmpty {
                    Section(group.title) {
                        ForEach(entries) { entry in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(entry.topic.name)
                                    .font(.whimbrelBody(.body))
                                    .foregroundStyle(Color.whimbrelTextPrimary)
                                    .lineLimit(1)
                                if let hint = hint(entry) {
                                    Text(hint)
                                        .font(.whimbrelBody(.caption))
                                        .foregroundStyle(Color.whimbrelTextSecondary)
                                }
                            }
                            .tag(entry.topic.topicId as Int64?)
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
    }

    private func kindGroup(_ kind: String?) -> String {
        guard let kind, Self.groups.contains(where: { $0.kind == kind }) else { return "other" }
        return kind
    }

    /// "call Tue" when the next meeting is within 7 days, "2 steps" for open steps Mike owes.
    private func hint(_ entry: TopicListEntry) -> String? {
        var parts: [String] = []
        if let ms = entry.nextMeetingUnixMs {
            let start = TopicDates.date(ms: ms)
            if start.timeIntervalSinceNow < 7 * 86_400 {
                parts.append(String(localized: "call \(start.formatted(.dateTime.weekday(.abbreviated)))"))
            }
        }
        if entry.openMine > 0 { parts.append(String(localized: "\(entry.openMine) steps")) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// Reads the list and the selected (else first) topic's page off the main thread.
    private func reload() {
        guard let store = appState.coreStore else { return }
        let wanted = appState.selectedTopicId
        CoreMemoryStore.accessQueue.async {
            let list = store.topicPageList()
            let id = wanted ?? list?.first?.topic.topicId
            let p = id.flatMap { store.topicPage(topicId: $0) }
            DispatchQueue.main.async {
                if let list {
                    topics = list
                    // A merged or deleted topic leaves the list: drop it so the first one shows.
                    if let wanted, !list.contains(where: { $0.topic.topicId == wanted }) {
                        appState.selectedTopicId = nil
                        return
                    }
                }
                hasLoaded = true
                // A reply for a topic no longer selected is dropped; the next reload covers it.
                guard appState.selectedTopicId == wanted else { return }
                page = p
                if wanted == nil, let id { appState.selectedTopicId = id }
            }
        }
    }

    /// Runs a store write off the main thread, then reloads; shows `failure` if it returns false.
    private func run(_ work: @escaping (CoreMemoryStore) -> Bool, failure: LocalizedStringResource) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let ok = work(store)
            DispatchQueue.main.async {
                errorText = ok ? nil : String(localized: failure)
                reload()
            }
        }
    }
}

/// Date formats for the topic page. Core sends due dates as local "YYYY-MM-DD".
enum TopicDates {
    static let iso: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd"
        return f
    }()
    static func date(ms: Int64) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }

    /// "Fri, Nov 14" (en-US), in the user's locale.
    static func day(_ d: Date) -> String {
        d.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day())
    }

    /// "Nov 3" (en-US), in the user's locale.
    static func monthDay(_ d: Date) -> String {
        d.formatted(.dateTime.month(.abbreviated).day())
    }

    /// The day for a "YYYY-MM-DD" string (the string itself if it does not parse).
    static func dayText(_ iso: String) -> String {
        Self.iso.date(from: iso).map(day) ?? iso
    }

    /// "due Fri Nov 14", or "overdue" once the date has passed.
    static func dueText(_ iso: String) -> String {
        isOverdue(iso) ? String(localized: "overdue") : String(localized: "due \(dayText(iso))")
    }

    static func isOverdue(_ iso: String) -> Bool { iso < Self.iso.string(from: Date()) }

    /// "Tue, Nov 18, 10:00" (en-US, 24-hour); the time follows the Mac's 12/24-hour setting.
    static func dayTime(ms: Int64) -> String {
        let d = date(ms: ms)
        return "\(day(d)), \(d.formatted(.dateTime.hour().minute()))"
    }
}

private enum StepSheet: Identifiable {
    case note(StepView), edit(StepView)
    var id: String {
        switch self {
        case .note(let s): "note-\(s.key)"
        case .edit(let s): "edit-\(s.key)"
        }
    }
}

private struct TopicPageView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openWindow) private var openWindow
    let page: TopicPage
    let errorText: String?
    let run: (@escaping (CoreMemoryStore) -> Bool, LocalizedStringResource) -> Void

    @AppStorage("com.mpointer.kestrel.topicSummaryCollapsed") private var summaryCollapsed = false
    @State private var roleDraft = ""
    @FocusState private var roleFocused: Bool
    @State private var sheet: StepSheet?
    @State private var showDone = false
    @State private var showOlder = false
    @State private var showAllRecent = false

    /// Recent items shown before "More" (core sends up to 20).
    private static let recentFirst = 10

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.lg) {
                Text(page.topic.name).font(.whimbrelPageTitle).foregroundStyle(Color.whimbrelTextPrimary)
                if let errorText {
                    Text(errorText).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
                }
                if let line = summaryLine {
                    DisclosureGroup(isExpanded: Binding(get: { !summaryCollapsed }, set: { summaryCollapsed = !$0 })) {
                        Text(line)
                            .font(.whimbrelBody(.callout))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)
                    } label: {
                        Text("Summary").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    }
                }
                section("Your role") { roleField }
                if let next = page.nextMeeting {
                    section("Next meeting") { nextMeeting(next) }
                }
                section("You owe") { steps(page.youOwe, empty: "Nothing you owe right now.") }
                section("Owed to you") { steps(page.owedToYou, empty: "Nothing owed to you right now.") }
                if !page.done.isEmpty {
                    DisclosureGroup(isExpanded: $showDone) { steps(page.done, empty: nil) } label: { heading("Done") }
                }
                if !page.older.isEmpty {
                    DisclosureGroup(isExpanded: $showOlder) { steps(page.older, empty: nil) } label: { heading("Older") }
                }
                section("Recent") { recent }
            }
            .frame(maxWidth: 720, alignment: .leading)
            .padding(Spacing.xl)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .onAppear { roleDraft = page.role.text }
        .onChange(of: page.role.text) { _, text in if !roleFocused { roleDraft = text } }
        .sheet(item: $sheet) { which in
            switch which {
            case .note(let step):
                StepNoteSheet(step: step) { text in act(step, .note(text)) }
            case .edit(let step):
                StepEditSheet(step: step) { text, due in act(step, .edit(text: text, dueDate: due)) }
            }
        }
    }

    // MARK: Summary

    /// "SOW due Fri Nov 14 · next call Tue Nov 18, 10:00", from stored facts only.
    private var summaryLine: String? {
        var parts: [String] = []
        if let lead = page.lead {
            if lead.kind == "date", let label = lead.label, let date = lead.date {
                parts.append("\(label) \(TopicDates.dayText(date))")
            } else if lead.kind == "step", let text = lead.text {
                parts.append(lead.dueDate.map { "\(text) \(TopicDates.dueText($0))" } ?? text)
            }
        }
        if let next = page.nextMeeting {
            parts.append(String(localized: "next call \(TopicDates.dayTime(ms: next.startUnixMs))"))
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    // MARK: Role

    private var roleField: some View {
        TextField("Your role on this topic", text: $roleDraft)
            .textFieldStyle(.roundedBorder)
            .focused($roleFocused)
            .frame(maxWidth: 360)
            .onSubmit {
                let key = page.topic.key
                let text = roleDraft.trimmingCharacters(in: .whitespacesAndNewlines)
                run({ $0.topicRoleSet(topicKey: key, text: text) }, "Could not save your role.")
            }
            .help(page.role.edited ? Text("Your wording. Return saves; empty it to let Whimbrel suggest one.") : Text("Whimbrel's guess. Edit it and press Return."))
    }

    // MARK: Next meeting

    private func nextMeeting(_ next: TopicNextMeeting) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: "\(TopicDates.dayTime(ms: next.startUnixMs))  \(next.title)")
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextPrimary)
            let orgs = attendeeLine(next)
            if !orgs.isEmpty {
                Text(orgs).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    /// "3 maximus.com · 1 you": other attendees by email domain, then Mike (matched by core on
    /// his exact addresses).
    private func attendeeLine(_ next: TopicNextMeeting) -> String {
        var parts = next.attendeesByOrg.map { "\($0.count) \($0.domain)" }
        if next.me > 0 { parts.append(String(localized: "\(next.me) you")) }
        return parts.joined(separator: " · ")
    }

    // MARK: Steps

    @ViewBuilder
    private func steps(_ list: [StepView], empty: LocalizedStringKey?) -> some View {
        if list.isEmpty {
            if let empty {
                Text(empty).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        } else {
            VStack(alignment: .leading, spacing: Spacing.md) {
                ForEach(list) { stepRow($0) }
            }
        }
    }

    private func stepRow(_ step: StepView) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                if step.confirmed {
                    Image(systemName: "checkmark.seal").foregroundStyle(Color.whimbrelSuccess).help("Confirmed")
                }
                Text(step.text)
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .strikethrough(step.status == "done")
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: Spacing.sm) {
                sourceLink(kind: step.sourceKind, title: step.sourceTitle, open: step.open)
                if step.fromScreen { Text("from screen") }
                Text("asked \(TopicDates.monthDay(TopicDates.date(ms: step.askedAtUnixMs)))")
                if let due = step.dueDate {
                    let text = TopicDates.dueText(due)
                    Text(text).foregroundStyle(TopicDates.isOverdue(due) ? Color.whimbrelError : Color.whimbrelTextSecondary)
                }
            }
            .font(.whimbrelBody(.caption))
            .foregroundStyle(Color.whimbrelTextSecondary)
            if let note = step.note, !note.isEmpty {
                Text("Note: \(note)").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            if let closedBy = step.closedBy, step.status == "done" {
                Text("Closed by \(closedBy)").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            HStack(spacing: Spacing.sm) {
                if step.status == "done" {
                    Button("Reopen") { act(step, .reopen) }
                } else {
                    Button("Check off") { act(step, .done) }
                    Button("Note") { sheet = .note(step) }
                    Menu("Dismiss") {
                        Button("Not mine") { act(step, .dismiss(reason: "not_mine")) }
                        Button("Not a task") { act(step, .dismiss(reason: "not_task")) }
                    }
                    .menuStyle(.borderlessButton)
                    .fixedSize()
                    if !step.confirmed { Button("Confirm") { act(step, .confirm) } }
                    Button("Edit") { sheet = .edit(step) }
                }
            }
            .buttonStyle(.whimbrelQuiet)
            .font(.whimbrelBody(.caption))
        }
    }

    private func act(_ step: StepView, _ action: TopicStepAction) {
        let key = step.key
        run({ $0.topicStepAction(key: key, action: action) }, "Could not update that step.")
    }

    // MARK: Recent

    @ViewBuilder
    private var recent: some View {
        if page.recent.isEmpty {
            Text("Nothing yet.").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            VStack(alignment: .leading, spacing: Spacing.sm) {
                ForEach(showAllRecent ? page.recent : Array(page.recent.prefix(Self.recentFirst))) { m in
                    HStack(spacing: Spacing.sm) {
                        Text(TopicDates.monthDay(TopicDates.date(ms: m.tsUnixMs)))
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                            .frame(width: 52, alignment: .leading)
                        sourceLink(kind: m.kind, title: m.title, open: m.open)
                            .font(.whimbrelBody(.callout))
                    }
                }
                if !showAllRecent, page.recent.count > Self.recentFirst {
                    Button("More") { showAllRecent = true }.buttonStyle(.whimbrelQuiet)
                } else if page.recentTotal > Int64(page.recent.count) {
                    Text("\(page.recentTotal - Int64(page.recent.count)) older items not shown")
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
        }
    }

    // MARK: Pieces

    /// The source's icon and title; a link when it can still be opened on this Mac.
    @ViewBuilder
    private func sourceLink(kind: String, title: String, open: TopicOpenTarget?) -> some View {
        let label = Label(title.isEmpty ? kind.capitalized : title, systemImage: Self.icon(kind)).lineLimit(1)
        if let open {
            Button { openWindow(id: open.window, value: open.id) } label: { label.foregroundStyle(Color.whimbrelLink) }
                .buttonStyle(.plain)
        } else {
            label
        }
    }

    private static func icon(_ kind: String) -> String {
        switch kind {
        case "email": "envelope"
        case "meeting", "snapshot": "person.2"
        case "calendar": "calendar"
        case "episode": "display"
        default: "doc"
        }
    }

    private func section<Content: View>(_ title: LocalizedStringKey, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            heading(title)
            content()
        }
    }

    private func heading(_ title: LocalizedStringKey) -> some View {
        Text(title).font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
    }
}

private struct StepNoteSheet: View {
    let step: StepView
    let onSave: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text: String

    init(step: StepView, onSave: @escaping (String) -> Void) {
        self.step = step
        self.onSave = onSave
        _text = State(initialValue: step.note ?? "")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text("Note").font(.whimbrelDisplay())
            Text(step.text).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            TextField("e.g. sent v2 Monday", text: $text, axis: .vertical)
                .textFieldStyle(.roundedBorder)
                .lineLimit(2...6)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.buttonStyle(.whimbrelSecondary)
                Button("Save") {
                    onSave(text.trimmingCharacters(in: .whitespacesAndNewlines))
                    dismiss()
                }
                .buttonStyle(.whimbrelPrimary)
            }
        }
        .padding(Spacing.lg)
        .frame(width: 420)
    }
}

private struct StepEditSheet: View {
    let step: StepView
    let onSave: (String, String?) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text: String
    @State private var hasDue: Bool
    @State private var due: Date

    init(step: StepView, onSave: @escaping (String, String?) -> Void) {
        self.step = step
        self.onSave = onSave
        _text = State(initialValue: step.text)
        let parsed = step.dueDate.flatMap { TopicDates.iso.date(from: $0) }
        _hasDue = State(initialValue: parsed != nil)
        _due = State(initialValue: parsed ?? Date())
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text("Edit step").font(.whimbrelDisplay())
            TextField("Step", text: $text, axis: .vertical)
                .textFieldStyle(.roundedBorder)
                .lineLimit(1...4)
            Toggle("Due date", isOn: $hasDue)
            if hasDue {
                DatePicker("Due", selection: $due, displayedComponents: .date).labelsHidden()
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.buttonStyle(.whimbrelSecondary)
                Button("Save") {
                    onSave(text.trimmingCharacters(in: .whitespacesAndNewlines), hasDue ? TopicDates.iso.string(from: due) : nil)
                    dismiss()
                }
                .buttonStyle(.whimbrelPrimary)
                .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(Spacing.lg)
        .frame(width: 420)
    }
}
