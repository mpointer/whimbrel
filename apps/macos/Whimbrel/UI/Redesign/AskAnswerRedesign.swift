import AppKit
import Combine
import SwiftUI
import WhimbrelUI

// Ask on Today (docs/ui-redesign/screens/conversations.md parts 1 and 2). A question asked from the
// Ask bar on Today answers right under the bar, with its sources. Asking again replaces the answer; nothing is
// saved unless the person continues it as a conversation.

/// What Today's answer area holds once an answer is in. Pure: the view and `TodayAnswerModel` decide from it.
struct TodayAnswerState: Equatable {
    /// The finished answer on screen; nil while one streams, after it failed, and once cleared.
    var exchange: AskExchange?
    /// Continue was pressed for this answer: never a second conversation from it.
    var continued = false

    /// A new answer takes the old one's place (Today never grows a chat).
    mutating func replace(with exchange: AskExchange) {
        self.exchange = exchange
        continued = false
    }

    mutating func clear() {
        exchange = nil
        continued = false
    }

    /// Continue is offered only for a finished answer, and only once.
    func canContinue(phase: AskRunner.Phase) -> Bool {
        guard exchange != nil, !continued, case .done = phase else { return false }
        return true
    }
}

/// Storing an answer as a conversation. A new exchange saves only pointers to the sources it cited (`n`, kind and id,
/// `whimbrel_conversations_append_exchange_sourced`), never their words or labels: the page reads the cards from the
/// live rows each time it opens. The Library's Ask (`TimelineView.persistExchange`) still saves its own format.
enum AskConversationStore {
    /// The citations in the Library's `[SavedCitation]` format: what an exchange saved before numbered sources holds.
    static func citationsJson(_ items: [ContextDisplayItem]) -> String {
        (try? JSONEncoder().encode(items.map(\.saved))).flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
    }

    /// One display item per citation, in the answer's `[n]` order, read from the store. An item that is gone (or a
    /// kind with no read by id) keeps its place as a saved citation, so the numbers still match the answer.
    /// Call on `CoreMemoryStore.accessQueue`.
    nonisolated static func items(_ refs: [CoreContextItemRef], store: CoreMemoryStore) -> [ContextDisplayItem] {
        var runs: [CoreRoutineRunWithName]?
        return refs.map { ref in
            switch ref.kind {
            case .capture: if let item = store.getCaptureItem(id: ref.id) { return .capture(item) }
            case .transcript: if let segment = store.getTranscriptSegment(id: ref.id) { return .transcript(segment) }
            case .episode: if let episode = store.getEpisode(id: ref.id) { return .episode(episode) }
            case .thread: if let thread = store.getThread(id: ref.id) { return .thread(thread) }
            case .routineRun:
                if runs == nil { runs = store.recentRoutineRuns(limit: 200) ?? [] }
                if let run = runs?.first(where: { $0.id == ref.id }) { return .routineRun(run) }
            case .snapshot, .mcpItem: break
            }
            let label: String
            switch ref.kind {
            case .routineRun: label = String(localized: "A routine result")
            case .snapshot: label = String(localized: "A meeting slide")
            case .mcpItem: label = String(localized: "An added item")
            default: label = ""
            }
            return .saved(SavedCitation(item: ref, label: label, text: "", open: nil))
        }
    }

    /// The cited sources as a sourced exchange saves them: for each citation, the number its card shows (the answer's
    /// own `[n]`, `AskSourceCardModel.markerNumbers`), its kind and its id. Nothing else about a source. Pure.
    static func citedSources(_ exchange: AskExchange) -> [CoreCitedSource] {
        // A sourced answer (Find) has core's own pointers, numbers and kinds (a meeting among them).
        if let sourced = exchange.sourced { return sourced.citedSources }
        let numbers = AskSourceCardModel.markerNumbers(answer: exchange.answer, citationCount: exchange.citations.count)
        return zip(numbers, exchange.citations).map { CoreCitedSource(n: $0, kind: $1.kind.rawValue, id: $1.id) }
    }

    /// Stores `exchange` in conversation `id` with pointers to its sources only, the provider that answered, whether it
    /// was made on this computer (`answered_locally`) and the left-out count. The excluded-client line is core's own
    /// `local_only` marker, set from the answer's text. Call on `CoreMemoryStore.accessQueue`.
    nonisolated static func append(_ exchange: AskExchange, to id: Int64, store: CoreMemoryStore) -> Bool {
        store.appendConversationExchangeSourced(
            conversationId: id, query: exchange.query, answer: exchange.answer, citedSources: citedSources(exchange),
            model: exchange.model, provider: exchange.provider, answeredLocally: exchange.madeOnThisComputer, leftOut: exchange.leftOut)
    }

    /// Makes a conversation whose first exchange is `exchange` exactly as shown (answer, sources, model): no model
    /// call. Sets its scope; one asked about a topic is in that topic from the start (not as the user's choice, so
    /// they can still change it); otherwise it joins the topic its sources point at. Nil when the row could not be
    /// made; a row whose exchange could not be stored is removed again. Call on `CoreMemoryStore.accessQueue`.
    nonisolated static func continueAsConversation(
        _ exchange: AskExchange, scope: CoreConversationScope, store: CoreMemoryStore, findFilter: CoreSavedFindFilter? = nil
    ) -> Int64? {
        // Find's conversation is made with its kept Dates and Apps filters in the same write.
        guard let id = store.createConversation(firstQuery: exchange.query, findFilter: findFilter) else { return nil }
        guard append(exchange, to: id, store: store) else {
            _ = store.deleteConversation(id: id)
            return nil
        }
        _ = store.setConversationScope(id, scope: scope)
        // Spec part 4: a topic page's conversation is attached to its topic from the start. Any link stops the
        // automatic one below from moving it.
        if case .topic(let topicId) = scope { _ = store.setConversationTopic(id, topicId: topicId, byUser: false) }
        // Spec part 5: after each stored exchange, join the topic most of its sources belong to.
        _ = store.autoTopic(conversationId: id)
        return id
    }

    /// Stores `exchange` as a new conversation's first exchange (`continueAsConversation`) off the main thread and
    /// starts its title job, then calls `done` on the main thread with its id (nil when nothing was stored). Today's
    /// Continue, a meeting answer's Continue, a new conversation's first answer and Find's first answer all store it
    /// this way, with pointers to its sources only.
    /// `requireLocal`: it was asked held to this computer (or made here, Find's rule), so its title stays local or is skipped
    /// (`ConversationTitleJob.plan`); an exchange kept local for an excluded client (`local_only`) does the same.
    @MainActor
    static func store(
        _ exchange: AskExchange, scope: CoreConversationScope, appState: AppState, requireLocal: Bool = false,
        findFilter: CoreSavedFindFilter? = nil, done: @escaping @MainActor (Int64?) -> Void
    ) {
        guard let store = appState.coreStore else { return done(nil) }
        // The title job's inputs, read here on the main thread: the background lane and the route.
        let lane = appState.backgroundModelLane
        // An open conversation page shows the generated title once it lands.
        let titleUpdated = appState.conversationTitleUpdated
        let ollamaModel = appState.routing.defaultSelection().ollamaModel
        let keepLocal = requireLocal || exchange.answeredLocally
        let titlePlan = ConversationTitleJob.plan(keepLocal: keepLocal, appState: appState)
        // Core's check (the question, answer, scope and cited rows) can only turn "local" ON, and it reads the store, so
        // it runs on the access queue. Its plan is read here, where the plan's inputs live.
        let localPlan = keepLocal ? titlePlan : ConversationTitleJob.plan(keepLocal: true, appState: appState)
        let settings = appState.routing
        CoreMemoryStore.accessQueue.async {
            let id = continueAsConversation(exchange, scope: scope, store: store, findFilter: findFilter)
            let touchesExcluded = !keepLocal && store.exchangeTouchesExcludedClient(
                scope: scope, query: exchange.query, answer: exchange.answer, citedSources: citedSources(exchange))
            let plan = touchesExcluded ? localPlan : titlePlan
            if let id, let lane {
                ConversationTitleJob.schedule(
                    plan: plan, routing: settings, lane: lane, conversationId: id, query: exchange.query, answer: exchange.answer,
                    ollamaModel: ollamaModel) { titleUpdated.send(id) }
            }
            DispatchQueue.main.async { done(id) }
        }
    }
}

/// Today's answer: one question at a time, held by `AppState` for the app's life. Main thread only.
@MainActor
final class TodayAnswerModel: ObservableObject {
    let runner: AskRunner
    /// The question on screen ("You asked"); nil when the area is empty.
    @Published private(set) var question: String?
    @Published private(set) var state = TodayAnswerState()
    /// The answer's sources, one per citation in `[n]` order, once read from the store.
    @Published private(set) var sources: [ContextDisplayItem] = []
    /// Why the question failed, shown with Try again.
    @Published private(set) var failure: String?

    private unowned let appState: AppState
    /// Asked while an earlier question was still stopping: asked as soon as the runner is free.
    private var queued: String?
    /// Changes with every ask and clear, so late work for an answer no longer on screen is dropped.
    private var serial = 0
    private var subscriptions: [AnyCancellable] = []

    init(appState: AppState) {
        self.appState = appState
        runner = AskRunner(appState: appState, scope: .everything)
        // `$phase` publishes on willSet, so the new phase is the argument.
        runner.$phase.dropFirst().sink { [weak self] in self?.handle($0) }.store(in: &subscriptions)
    }

    var canContinue: Bool { state.canContinue(phase: runner.phase) }

    /// Asks `query`, replacing whatever the area shows. A question still streaming is stopped first.
    func ask(_ query: String) {
        let text = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        serial += 1
        question = text
        state.clear()
        sources = []
        failure = nil
        if runner.isBusy {
            queued = text
            runner.cancel()
        } else {
            queued = nil
            runner.ask(text, history: [])
        }
    }

    func retry() { if let question { ask(question) } }

    /// Empties the area; a question still streaming is stopped.
    func clear() {
        serial += 1
        queued = nil
        runner.cancel()
        question = nil
        state.clear()
        sources = []
        failure = nil
    }

    /// Spec part 2: the shown question and answer become a conversation's first exchange (no second model call),
    /// the conversation opens and the area clears. The button is off from the first press, so one row at most.
    func continueAsConversation() {
        guard canContinue, let exchange = state.exchange else { return }
        state.continued = true
        let mark = serial
        AskConversationStore.store(exchange, scope: .everything, appState: appState) { [weak self] id in
            guard let self, self.serial == mark else { return }
            guard let id else {
                // Nothing was stored: Continue can be pressed again.
                self.state.continued = false
                return
            }
            self.clear()
            self.appState.openConversation(id: id)
        }
    }

    private func handle(_ phase: AskRunner.Phase) {
        guard queued == nil else {
            // The stopped question ended: ask the one that replaced it, once the runner has settled.
            if phase != .streaming { DispatchQueue.main.async { [weak self] in self?.askQueued() } }
            return
        }
        switch phase {
        case .done(let exchange):
            guard question != nil else { return }
            state.replace(with: exchange)
            resolveSources(exchange)
        case .failed(let message):
            if question != nil { failure = message }
        case .nothingInScope:
            // Not reached with everything in scope; shown as a failure rather than an empty answer.
            if question != nil { failure = CoreLLMError.providerFailure.userFacingMessage }
        case .idle, .streaming:
            break
        }
    }

    private func askQueued() {
        guard let text = queued, !runner.isBusy else { return }
        queued = nil
        runner.ask(text, history: [])
    }

    private func resolveSources(_ exchange: AskExchange) {
        guard !exchange.citations.isEmpty, let store = appState.coreStore else { return }
        let mark = serial
        CoreMemoryStore.accessQueue.async { [weak self] in
            let items = AskConversationStore.items(exchange.citations, store: store)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.serial == mark else { return }
                self.sources = items
            }
        }
    }
}

/// One source under Today's answer, as a `WSourceCard` (card size) draws it. Main thread only (the skin's icons).
struct AskSourceCardModel {
    /// The `[n]` each citation answers to. Core lists citations in the order their markers first appear, each `[n]`
    /// naming the n-th context item, and drops markers past the end of the context (core `extract_citations`). So the
    /// distinct markers, in first-seen order without the out-of-range ones (always the largest), line up with the
    /// citations. Falls back to 1, 2, 3… when the text does not account for every citation.
    static func markerNumbers(answer: String, citationCount: Int) -> [Int] {
        var seen: [Int] = []
        for part in answer.split(separator: "[", omittingEmptySubsequences: false).dropFirst() {
            let digits = part.prefix { $0.isASCII && $0.isNumber }
            guard !digits.isEmpty, part.dropFirst(digits.count).first == "]", let n = Int(digits), n >= 1, !seen.contains(n) else { continue }
            seen.append(n)
        }
        guard seen.count >= citationCount else { return Array(1..<(citationCount + 1)) }
        let kept = Set(seen.sorted().prefix(citationCount))
        return seen.filter { kept.contains($0) }
    }

    let appGlyph: String
    let appName: String?
    let kind: WSourceKind
    let title: String
    let time: String
    let quote: String

    /// The card's kind for a citation's kind: a meeting moment, an email, a routine's result, or something seen on screen.
    static func sourceKind(_ kind: CoreContextItemRef.Kind) -> WSourceKind {
        switch kind {
        case .transcript: return .transcriptMoment
        case .thread: return .email
        case .routineRun: return .routineRun
        case .capture, .episode, .snapshot, .mcpItem: return .seenOnScreen
        }
    }

    /// Whether the source has somewhere to open: not a slide or a deleted item kept only as a saved citation.
    static func canOpen(_ item: ContextDisplayItem) -> Bool {
        if case .saved(let citation) = item { return citation.open != nil }
        return true
    }

    @MainActor
    init(_ item: ContextDisplayItem) {
        func when(_ ms: Int64) -> String {
            Date(timeIntervalSince1970: Double(ms) / 1000).formatted(.dateTime.month(.abbreviated).day().hour().minute())
        }
        appGlyph = item.systemImage
        quote = item.displayText
        kind = Self.sourceKind(item.ref.kind)
        switch item {
        case .transcript(let segment):
            let seconds = segment.startCs / 100
            appName = nil
            title = segment.meetingTitle
            time = String(format: "%d:%02d", seconds / 60, seconds % 60)
        case .snapshot(let hit):
            appName = nil
            title = hit.meetingTitle
            time = ""
        case .capture(let capture):
            appName = HomeRedesignAdapter.appName(capture.appBundleId)
            title = item.chipText
            time = when(capture.tsUnixMs)
        case .episode(let episode):
            appName = HomeRedesignAdapter.appName(episode.appBundleId)
            title = item.chipText
            time = when(episode.startTsUnixMs)
        case .thread(let thread):
            // The kind already says Email: name who wrote last.
            appName = thread.lastMessageFrom
            title = item.chipText
            time = when(thread.lastActivityTsUnixMs)
        case .routineRun(let run):
            appName = run.routineName
            title = item.chipText
            time = when(run.ranAtUnixMs)
        case .saved(let citation):
            appName = nil
            title = citation.label
            time = ""
        }
    }
}

// MARK: Shared exchange parts (Today's answer and the conversation page)

/// "You asked" and the question, above an answer on Today and on a conversation page.
struct AskQuestionLabel: View {
    @Environment(\.wTheme) private var theme
    let question: String

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(2)) {
            Text(String(localized: "You asked", comment: "Label above the question asked from the Ask bar on Today."))
                .wFont(.caption).foregroundStyle(theme.textSecondary)
            Text(question).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).textSelection(.enabled)
        }
    }
}

/// An answer still streaming: the text so far, then the quiet bar.
struct AskStreamingParts: View {
    let partial: String

    var body: some View {
        if !partial.isEmpty { AskAnswerText(markdown: partial) }
        WProgressLine()
    }
}

/// A finished answer in the redesigned Ask parts: the answer, its sources as numbered `WSourceCard`s (the answer's
/// own `[n]`, lowest first; Open only where there is somewhere to open), the model that answered in small text when
/// `showsModel`, and the answered-locally line. Lay it out inside a vertical stack.
struct AskAnswerParts: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openWindow) private var openWindow
    @Environment(\.wTheme) private var theme
    let exchange: AskExchange
    let sources: [ContextDisplayItem]
    /// A reopened exchange's saved `[n]`, one per source; nil numbers the sources from the answer's markers.
    var numbers: [Int]? = nil
    /// Cited meetings (no context item), each under its own number; only with `numbers`.
    var meetings: [ConversationMeetingSource] = []
    var showsModel = false

    /// One card: a context item, or a cited meeting.
    private enum Shown { case item(ContextDisplayItem), meeting(ConversationMeetingSource) }

    var body: some View {
        AskAnswerText(markdown: exchange.answer, allowLinks: AskMarkdown.linksAllowed(exchange))
        if !sources.isEmpty || !meetings.isEmpty {
            // Numbered as the answer's markers (or as saved), lowest first.
            let numbers = numbers.flatMap { $0.count == sources.count ? $0 : nil }
                ?? AskSourceCardModel.markerNumbers(answer: exchange.answer, citationCount: sources.count)
            let numbered = (zip(numbers, sources).map { ($0, Shown.item($1)) } + meetings.map { ($0.n, Shown.meeting($0)) })
                .sorted { $0.0 < $1.0 }
            VStack(alignment: .leading, spacing: theme.space(8)) {
                ForEach(numbered, id: \.0) { number, shown in
                    switch shown {
                    case .item(let item): card(item, number: number)
                    case .meeting(let meeting): meetingCard(meeting)
                    }
                }
            }
        }
        if showsModel, let model = exchange.model?.trimmingCharacters(in: .whitespacesAndNewlines), !model.isEmpty {
            Text(verbatim: model).wFont(.caption).foregroundStyle(theme.textSecondary)
        }
        if exchange.answeredLocally {
            Text(AskLocalRouteCopy.answered).wFont(.secondary).foregroundStyle(theme.textSecondary)
        }
    }

    private func card(_ item: ContextDisplayItem, number: Int) -> some View {
        let shown = AskSourceCardModel(item)
        return WSourceCard(
            number: number, size: .card, appGlyph: shown.appGlyph, appName: shown.appName, kind: shown.kind,
            title: shown.title, time: shown.time, quote: shown.quote, quoteLineLimit: 3,
            onSelect: {}, onOpen: AskSourceCardModel.canOpen(item) ? { open(item) } : nil)
    }

    private func meetingCard(_ meeting: ConversationMeetingSource) -> some View {
        let time = meeting.tsUnixMs.map {
            Date(timeIntervalSince1970: Double($0) / 1000).formatted(.dateTime.month(.abbreviated).day().hour().minute())
        } ?? ""
        return WSourceCard(
            number: meeting.n, size: .card, appGlyph: "person.2", appName: nil, kind: .meeting,
            title: meeting.title, time: time, quote: "", quoteLineLimit: 3,
            onSelect: {}, onOpen: meeting.openable ? { openWindow(id: "meeting-detail", value: meeting.meetingId) } : nil)
    }

    /// Opens a source as the Library's citations do; a transcript moment opens its meeting at that moment.
    private func open(_ item: ContextDisplayItem) {
        switch item {
        case .transcript(let segment):
            appState.openMeeting(id: segment.meetingId, at: segment.startCs)
        case .routineRun:
            appState.selectedTab = .automations
        default:
            if let id = item.detailWindowId, let value = item.detailWindowValue { openWindow(id: id, value: value) }
        }
    }
}

/// The area under the Ask bar on Today: "You asked", the answer as it streams, its numbered sources, the
/// answered-locally line, then Continue as a conversation, Copy and Clear. Draws nothing while empty.
struct TodayAnswerArea: View {
    @ObservedObject var model: TodayAnswerModel
    /// Observed directly: the model does not republish the runner's streaming text.
    @ObservedObject var runner: AskRunner
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var contentHeight: CGFloat = 0

    init(model: TodayAnswerModel) {
        self.model = model
        runner = model.runner
    }

    var body: some View {
        if let question = model.question {
            // Its own scroll, capped, so a long answer never pushes Today's line off the window.
            ScrollView {
                column(question)
                    .background(GeometryReader { geo in Color.clear.preference(key: AnswerHeightKey.self, value: geo.size.height) })
            }
            .frame(height: min(max(contentHeight, 1), 420 * k))
            .onPreferenceChange(AnswerHeightKey.self) { contentHeight = $0 }
            .overlay(alignment: .bottom) { Divider() }
        }
    }

    private func column(_ question: String) -> some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            AskQuestionLabel(question: question)
            if let failure = model.failure {
                WErrorState(title: failure, help: nil, onRetry: model.retry)
            } else if let exchange = model.state.exchange {
                AskAnswerParts(exchange: exchange, sources: model.sources)
                actions(exchange)
            } else {
                AskStreamingParts(partial: runner.partial)
            }
        }
        .frame(maxWidth: 760 * k, alignment: .leading)
        .padding(.horizontal, theme.space(28))
        .padding(.vertical, theme.space(12))
        .frame(maxWidth: .infinity, alignment: .center)
    }

    private func actions(_ exchange: AskExchange) -> some View {
        HStack(spacing: theme.space(8)) {
            WButton(String(localized: "Continue as a conversation", comment: "Button under the answer on Today: keeps the question and answer as a conversation and opens it."),
                    glyph: "bubble.left.and.bubble.right", kind: .primary) { model.continueAsConversation() }
                .disabled(!model.canContinue)
            WButton(String(localized: "Copy"), glyph: "doc.on.doc", kind: .quiet) {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(exchange.answer, forType: .string)
            }
            WButton(String(localized: "Clear"), glyph: "xmark", kind: .quiet) { model.clear() }
            Spacer(minLength: 0)
        }
    }
}

private struct AnswerHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// The Conversations section: a new conversation's page, else the chosen conversation's page, else the list. A page
/// has a way back to the list above it.
struct ConversationsHostView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.wTheme) private var theme

    var body: some View {
        Group {
            if let draft = appState.conversationDraft {
                page { ConversationRedesignView(draft: draft).id(draft.id) }
            } else if let id = appState.selectedConversationId {
                // A fresh page per conversation, so switching stops the old one's question.
                page { ConversationRedesignView(conversationId: id).id(id) }
            } else {
                ConversationsListRedesign()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func page<Page: View>(@ViewBuilder _ content: () -> Page) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            WButton(String(localized: "Conversations"), glyph: "chevron.left", kind: .quiet) { appState.showConversationList() }
                .padding(.horizontal, theme.space(16))
                .padding(.top, theme.space(8))
            content()
        }
    }
}
