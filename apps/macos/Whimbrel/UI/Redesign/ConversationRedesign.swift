import Combine
import SwiftUI
import WhimbrelUI
import os

// The conversation page (docs/ui-redesign/screens/conversations.md parts 3, 4 and 5). A header with the
// title (click to rename), the star, the topic chip and a menu with Delete; the scope line; the exchanges oldest
// first in the same parts as Today's answer; a composer at the bottom. Provider and model are never picked here.

/// The topic chip in a conversation's header. Pure: the page decides from it.
enum TopicChipModel: Equatable {
    /// The conversation belongs to this topic: the chip names it.
    case topic(TopicRef)
    /// No topic (mixed sources, never joined, or the user removed it): an "Add to topic" button instead.
    case add

    static func make(_ t: CoreConversationTopic) -> TopicChipModel {
        if let topic = t.topic { return .topic(topic) }
        return .add
    }

    /// The topics the chip's menu offers: the ones the conversation's sources belong to, in the order they were
    /// cited, each once, without the one it is in now.
    static func menuTopics(cited: [TopicRef], current: TopicRef?) -> [TopicRef] {
        var seen = Set<Int64>()
        if let current { seen.insert(current.topicId) }
        return cited.filter { seen.insert($0.topicId).inserted }
    }
}

/// Reading a stored conversation back for the page. Pure.
enum ConversationPageLoad {
    /// The sources of a stored exchange: each citation read again from the store (`live`, in `[n]` order), except
    /// that one whose item is gone keeps the card it was saved with (`stored`), not a blank one.
    static func sources(live: [ContextDisplayItem], stored: [SavedCitation]) -> [ContextDisplayItem] {
        live.map { item in
            guard case .saved = item, let kept = stored.first(where: { $0.item == item.ref }) else { return item }
            return .saved(kept)
        }
    }

    /// Whether each stored exchange (by id, oldest first) shows the answered-locally line: one stored `local_only`
    /// does, and so does every later one (spec call 4, as `AskRunner.answeredLocally` rules for a new answer).
    static func answeredLocally(exchangeIds: [Int64], localOnly: Set<Int64>) -> [Bool] {
        answeredLocally(fresh: exchangeIds.map { localOnly.contains($0) })
    }

    /// Whether each stored exchange shows the answered-locally line, from each one's own flag (oldest first): a later
    /// one shows it too once an earlier one did (spec call 4).
    static func answeredLocally(fresh: [Bool]) -> [Bool] {
        var earlier: [AskExchange] = []
        return fresh.map { own in
            let shown = AskRunner.answeredLocally(fresh: own, history: earlier)
            earlier.append(AskExchange(query: "", answer: "", citations: [], model: nil, answeredLocally: shown))
            return shown
        }
    }

    /// The citation a saved source points at; nil for a kind this page has no card for.
    nonisolated static func ref(_ card: CoreSourceCard) -> CoreContextItemRef? {
        CoreContextItemRef.Kind(rawValue: card.kind).map { CoreContextItemRef(kind: $0, id: card.id) }
    }

    /// A sourced exchange's cards, each numbered with its saved `n` (lowest first), read from the live rows by `read`
    /// (`AskConversationStore.items`; called once, for the rows that are there). A source whose row is gone says
    /// `gone` and shows no text; one that could not be read just now says `unreadable` (never `gone`). Nothing about
    /// a source is ever kept but its number, kind and id. Pure apart from `read`.
    nonisolated static func sourcedSources(
        _ sources: [CoreNumberedSource], gone: String, unreadable: String,
        read: ([CoreContextItemRef]) -> [ContextDisplayItem]
    ) -> (numbers: [Int], items: [ContextDisplayItem], meetings: [ConversationMeetingSource]) {
        // A cited meeting has no context ref: it keeps a card of its own under its number.
        let meetings = sources.filter { $0.card.kind == "meeting" }.sorted { $0.n < $1.n }.map { source in
            let shown = source.card.found && !source.unreadable
            return ConversationMeetingSource(
                n: source.n, meetingId: source.card.id, title: shown ? source.card.title : (source.unreadable ? unreadable : gone),
                tsUnixMs: shown ? source.card.tsUnixMs : nil, openable: shown)
        }
        let kept = sources.compactMap { source in ref(source.card).map { (source, $0) } }.sorted { $0.0.n < $1.0.n }
        let there = kept.filter { $0.0.card.found && !$0.0.unreadable }.map(\.1)
        var live = read(there).makeIterator()
        let items = kept.map { source, ref -> ContextDisplayItem in
            guard source.card.found, !source.unreadable else {
                return .saved(SavedCitation(item: ref, label: source.unreadable ? unreadable : gone, text: "", open: nil))
            }
            let item = live.next() ?? .saved(SavedCitation(item: ref, label: "", text: "", open: nil))
            // A kind with no read by id: the live card's own title and words.
            if case .saved(let placeholder) = item, placeholder.text.isEmpty {
                let title = source.card.title.isEmpty ? placeholder.label : source.card.title
                return .saved(SavedCitation(item: ref, label: title, text: source.card.text ?? "", open: nil))
            }
            return item
        }
        return (kept.map(\.0.n), items, meetings)
    }

    /// A sourced answer's cards straight from core's numbered list: the cited numbers only, read as `sourcedSources`
    /// reads a saved exchange, so a follow-up shows the same cards, meetings included, before any reload.
    nonisolated static func citedSources(
        _ sourced: AskSourced, gone: String, unreadable: String, read: ([CoreContextItemRef]) -> [ContextDisplayItem]
    ) -> (numbers: [Int], items: [ContextDisplayItem], meetings: [ConversationMeetingSource]) {
        let cited = Set(sourced.cited)
        return sourcedSources(sourced.sources.filter { cited.contains($0.n) }, gone: gone, unreadable: unreadable, read: read)
    }

    /// What a source whose row is gone says, with no text (as the Find page says it).
    static var goneLabel: String {
        String(localized: "No longer on this device", comment: "Shown on a source or a result whose saved item is gone from this device. No text is shown with it.")
    }

    /// What a source that could not be read just now says. It is not deleted.
    static var unreadableLabel: String {
        String(localized: "Could not read this just now.", comment: "Shown on a source of an answer whose saved item could not be read just now. It is not deleted, only unreadable for the moment.")
    }

    /// A stored exchange's citations (`[SavedCitation]`; "[]" or unreadable JSON for exchanges saved before).
    static func savedCitations(_ json: String) -> [SavedCitation] {
        (try? JSONDecoder().decode([SavedCitation].self, from: Data(json.utf8))) ?? []
    }

    /// The scope a stored conversation's follow-ups are asked with: core's resolved scope (a merged topic's
    /// survivor; everything once its topic or meeting is gone), or the stored scope when that read failed, so a
    /// store error never widens what is asked.
    static func followUpScope(stored: CoreConversationScope, resolved: CoreConversationScope?) -> CoreConversationScope {
        resolved ?? stored
    }

    /// The scope line for `scope`, naming its topic (from `topics`, read when not given) or meeting from the
    /// store. Call on `CoreMemoryStore.accessQueue`.
    nonisolated static func scopeLine(_ scope: CoreConversationScope, store: CoreMemoryStore, topics: [TopicRef]? = nil) -> String? {
        var topicName: String?
        var meetingName: String?
        switch scope {
        case .everything: break
        case .topic(let topicId):
            topicName = (topics ?? (store.topicPageList() ?? []).map(\.topic)).first { $0.topicId == topicId }?.name
        case .meeting(let meetingId):
            meetingName = store.getMeeting(id: meetingId).map { meetingLabel(title: $0.title, startUnixMs: $0.startUnixMs) }
        }
        return AskRunner.scopeLine(scope, topicName: topicName, meetingLabel: meetingName)
    }

    /// The scope line of an everything conversation kept with Find filters, in the chips' words: "Asking about Apps: Slack, Dates: Nov 9
    /// to Nov 15". Nil when it kept no Dates or Apps (only a left-out person): the line is then "Asking about everything".
    @MainActor
    static func filteredScopeLine(_ kept: CoreSavedFindFilter) -> String? {
        let filter = FindAdapter.reopenedFilter(kept)
        let datesTitle: String? = { if case .exact(_, _, let title) = filter.dates { return title } else { return nil } }()
        guard let summary = FindAdapter.filterSummary(
            filter, appName: { HomeRedesignAdapter.appName($0) }, datesTitle: datesTitle, strings: WhimbrelUIStrings.make().home)
        else { return nil }
        return AskRunner.scopeLine(filters: summary)
    }

    /// The scope line's meeting name: "Pod 2 Team Call, Nov 19".
    static func meetingLabel(title: String, startUnixMs: Int64) -> String {
        let day = Date(timeIntervalSince1970: Double(startUnixMs) / 1000).formatted(.dateTime.month(.abbreviated).day())
        return "\(title), \(day)"
    }
}

/// A cited meeting on the conversation page (it has no context item): its number, the meeting, and the words its card shows.
struct ConversationMeetingSource: Equatable {
    let n: Int
    let meetingId: Int64
    /// The meeting's title, or the gone or unreadable label.
    let title: String
    let tsUnixMs: Int64?
    /// The meeting is there to open.
    let openable: Bool
}

/// One exchange on the page, with its sources read for display. `numbers` are the saved `[n]` of a reopened sourced
/// exchange's sources, in order; nil numbers them from the answer's markers.
struct ConversationEntry: Identifiable {
    let id: Int
    let exchange: AskExchange
    var sources: [ContextDisplayItem]
    var numbers: [Int]? = nil
    var meetings: [ConversationMeetingSource] = []
}

/// A conversation page's state and its questions. Made empty by the view, then `attach`ed once the app state is at
/// hand. Main thread only.
@MainActor
final class ConversationPageModel: ObservableObject {
    nonisolated private static let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "app", category: "conversation")

    @Published private(set) var loaded = false
    /// The conversation could not be read (deleted elsewhere, or a store error).
    @Published private(set) var missing = false
    @Published private(set) var title = ""
    @Published private(set) var starred = false
    @Published private(set) var scopeLine: String?
    /// A Find conversation asked under a People filter, which is never kept, says so under its scope line.
    @Published private(set) var filterNote: String?
    @Published private(set) var topicLink = CoreConversationTopic(topic: nil, setByUser: false)
    /// The live topics the conversation's sources belong to, in the order first cited.
    @Published private(set) var citedTopics: [TopicRef] = []
    /// Live topics for "Choose another topic…".
    @Published private(set) var allTopics: [TopicRef] = []
    @Published private(set) var entries: [ConversationEntry] = []
    /// The question in flight or just failed; nil when the page is settled.
    @Published private(set) var pendingQuestion: String?
    @Published private(set) var failure: String?
    /// The topic held nothing for the pending question: the page offers to ask about everything instead.
    @Published private(set) var nothingInScope = false
    /// The runner asking now (the conversation's scope, or everything for one re-asked question).
    @Published private(set) var active: AskRunner?
    /// A question is in flight (the composer waits).
    @Published private(set) var isBusy = false

    private(set) var conversationId: Int64 = 0
    /// Set for a conversation not stored yet (New conversation, a question asked on the Conversations tab or a
    /// topic page): its scope. Its first answer makes the row and opens the stored conversation in its place.
    private(set) var draftScope: CoreConversationScope?
    /// The draft this page shows (`AppState.conversationDraft`), and whether its first answer is being stored.
    private var draftId: UUID?
    private var storingDraft = false
    private weak var appState: AppState?
    private var scopedRunner: AskRunner?
    /// The Find filters an everything conversation kept (Dates and Apps): its follow-ups are asked under them.
    private var keptFilter: CoreFindFilter?
    private var everythingRunner: AskRunner?
    private var subscriptions: [AnyCancellable] = []
    /// Set by `stop`: late work for this page is dropped.
    private var stopped = false

    var isDraft: Bool { draftScope != nil }
    var chip: TopicChipModel { TopicChipModel.make(topicLink) }
    var menuTopics: [TopicRef] { TopicChipModel.menuTopics(cited: citedTopics, current: topicLink.topic) }

    /// Called each time the page appears: the first time loads it; back after `stop` it takes results again (and
    /// loads, if the first load was dropped).
    func attach(appState: AppState, conversationId: Int64) {
        stopped = false
        if self.appState == nil {
            self.appState = appState
            self.conversationId = conversationId
            // A generated title landing (Today's Continue, or this page's first exchange) replaces the placeholder.
            appState.conversationTitleUpdated
                .filter { $0 == conversationId }
                .sink { [weak self] _ in self?.reloadTitle() }
                .store(in: &subscriptions)
        }
        if !loaded { load() }
    }

    /// A new conversation's page: empty, or asking `draft.question` straight away. Called each time it appears.
    func attachDraft(appState: AppState, draft: AppState.ConversationDraft) {
        stopped = false
        guard self.appState == nil else { return }
        self.appState = appState
        draftScope = draft.scope
        draftId = draft.id
        title = String(localized: "New conversation", comment: "The Conversations list's button for an empty conversation about everything, and that page's title until its first answer is in.")
        makeRunner(scope: draft.scope)
        loaded = true
        if let question = draft.question { ask(question) }
        // The scope line names the topic or meeting: read it from the store.
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async { [weak self] in
            let line = ConversationPageLoad.scopeLine(draft.scope, store: store)
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.stopped else { return }
                self.scopeLine = line
            }
        }
    }

    /// Reads the conversation again after it could not be read.
    func reload() {
        guard missing else { return }
        missing = false
        load()
    }

    /// The page went away: stop any question and drop late results until it appears again. A new conversation's
    /// page that goes before its first answer is stored is dropped with it: coming back shows the list, and its
    /// question is never asked a second time unprompted.
    func stop() {
        stopped = true
        cancelQuestions()
        if let draftId, !storingDraft, let appState {
            // After this view update, not during it.
            DispatchQueue.main.async {
                if appState.conversationDraft?.id == draftId { appState.conversationDraft = nil }
            }
        }
    }

    private func cancelQuestions() {
        scopedRunner?.cancel()
        everythingRunner?.cancel()
    }

    // MARK: Loading

    private func load() {
        guard let store = appState?.coreStore else { missing = true; return }
        let id = conversationId
        CoreMemoryStore.accessQueue.async { [weak self] in
            guard let detail = store.getConversationSourced(id: id) else {
                DispatchQueue.main.async { [weak self] in self?.missing = true }
                return
            }
            // Follow-ups ask the topic a merged one became, or everything once the topic or meeting is gone; the
            // scope line names what is really asked.
            let scope = ConversationPageLoad.followUpScope(
                stored: store.conversationScope(id), resolved: store.conversationResolvedScope(id))
            let link = store.conversationTopic(id)
            let topics = (store.topicPageList() ?? []).map(\.topic)
            // The excluded-client line comes from core's `local_only` marker on every exchange, sourced or not; a sourced
            // exchange's `answered_locally` (made on this computer for any reason) is not that line.
            let local = ConversationPageLoad.answeredLocally(
                exchangeIds: detail.exchanges.map(\.id), localOnly: store.localOnlyExchangeIds(conversationId: id))
            let entries = detail.exchanges.enumerated().map { index, row -> ConversationEntry in
                guard !row.legacy else {
                    // Saved before numbered sources: its saved citations, drawn as before.
                    let saved = ConversationPageLoad.savedCitations(row.citationsJson ?? "[]")
                    let refs = saved.map(\.item)
                    let live = AskConversationStore.items(refs, store: store)
                    let exchange = AskExchange(query: row.query, answer: row.answer, citations: refs, model: row.model,
                                               answeredLocally: local[index])
                    return ConversationEntry(id: index, exchange: exchange, sources: ConversationPageLoad.sources(live: live, stored: saved))
                }
                // Saved as pointers: every card is read from the live row now.
                let shown = ConversationPageLoad.sourcedSources(
                    row.sources, gone: ConversationPageLoad.goneLabel, unreadable: ConversationPageLoad.unreadableLabel
                ) { AskConversationStore.items($0, store: store) }
                let exchange = AskExchange(query: row.query, answer: row.answer, citations: shown.items.map(\.ref), model: row.model,
                                           answeredLocally: local[index], provider: row.provider, leftOut: row.leftOut ?? 0,
                                           madeOnThisComputer: row.answeredLocally ?? false)
                return ConversationEntry(id: index, exchange: exchange, sources: shown.items, numbers: shown.numbers, meetings: shown.meetings)
            }
            let cited = store.conversationCitedTopics(id)
            let line = ConversationPageLoad.scopeLine(scope, store: store, topics: topics)
            // Core keeps Find filters for an everything conversation only.
            let kept = scope == .everything ? store.conversationFindFilter(id) : nil
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.stopped else { return }
                self.title = detail.title
                self.starred = detail.starred
                self.keptFilter = kept.flatMap { FindAdapter.coreFilter($0) }
                self.scopeLine = kept.flatMap { ConversationPageLoad.filteredScopeLine($0) } ?? line
                self.filterNote = kept?.peopleLeftOut == true ? FindAdapter.peopleFilterNotKept : nil
                self.topicLink = link
                self.allTopics = topics
                self.entries = entries
                self.citedTopics = cited
                self.makeRunner(scope: scope)
                self.loaded = true
            }
        }
    }

    private func makeRunner(scope: CoreConversationScope) {
        guard let appState, scopedRunner == nil else { return }
        let runner = AskRunner(appState: appState, scope: scope)
        observe(runner)
        scopedRunner = runner
        if scope == .everything { everythingRunner = runner }
    }

    private func observe(_ runner: AskRunner) {
        // `$phase` publishes on willSet, so the new phase is the argument.
        runner.$phase.dropFirst().sink { [weak self, weak runner] phase in
            guard let self, let runner, self.active === runner else { return }
            self.handle(phase)
        }.store(in: &subscriptions)
    }

    // MARK: Asking

    /// A follow-up, asked with the conversation's scope and its earlier exchanges as context.
    func ask(_ query: String) {
        let text = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, loaded, !isBusy, let runner = scopedRunner else { return }
        run(text, on: runner)
    }

    /// The topic held nothing: the same question once more about everything. The conversation's scope stays.
    func askEverythingInstead() {
        guard nothingInScope, let question = pendingQuestion, !isBusy, let appState else { return }
        let runner: AskRunner
        if let everythingRunner {
            runner = everythingRunner
        } else {
            runner = AskRunner(appState: appState, scope: .everything)
            observe(runner)
            everythingRunner = runner
        }
        run(question, on: runner)
    }

    /// Try again after a failure, on the runner that failed.
    func retry() {
        guard failure != nil, let question = pendingQuestion, let runner = active else { return }
        run(question, on: runner)
    }

    private func run(_ question: String, on runner: AskRunner) {
        pendingQuestion = question
        failure = nil
        nothingInScope = false
        active = runner
        // A kept Find conversation asks under its Dates and Apps filters, through the sourced calls that take them.
        if runner === scopedRunner, let keptFilter {
            runner.ask(question, history: entries.map(\.exchange), options: AskRunner.Options(sourced: true, filter: keptFilter))
        } else {
            runner.ask(question, history: entries.map(\.exchange))
        }
        isBusy = runner.isBusy
    }

    private func handle(_ phase: AskRunner.Phase) {
        isBusy = phase == .streaming
        switch phase {
        case .done(let exchange):
            pendingQuestion = nil
            append(exchange)
        case .failed(let message):
            failure = message
        case .nothingInScope:
            nothingInScope = true
        case .idle:
            // Stopped: nothing is kept.
            if failure == nil, !nothingInScope { pendingQuestion = nil }
        case .streaming:
            break
        }
    }

    /// Shows the exchange, then stores it (pointers to its sources only) and lets the conversation join the topic
    /// most of its sources belong to. The first exchange of an empty conversation also starts the title job.
    private func append(_ exchange: AskExchange) {
        let index = entries.count
        entries.append(ConversationEntry(id: index, exchange: exchange, sources: []))
        guard let appState, let store = appState.coreStore else { return }
        if let scope = draftScope { return storeFirst(exchange, scope: scope, appState: appState) }
        let id = conversationId
        let first = index == 0
        let lane = appState.backgroundModelLane
        let titleUpdated = appState.conversationTitleUpdated
        let defaults = appState.routing.defaultSelection()
        // An answer kept on this computer for an excluded client keeps its title here too, or skips it.
        let titlePlan = ConversationTitleJob.plan(keepLocal: exchange.answeredLocally, appState: appState)
        let settings = appState.routing
        CoreMemoryStore.accessQueue.async { [weak self] in
            // Shown from the live rows; stored as pointers only. A sourced answer (kept Find filters) shows core's cited
            // list, numbered as its marks, meetings included.
            let shown: (numbers: [Int]?, items: [ContextDisplayItem], meetings: [ConversationMeetingSource]) = exchange.sourced.map { sourced in
                let cards = ConversationPageLoad.citedSources(
                    sourced, gone: ConversationPageLoad.goneLabel, unreadable: ConversationPageLoad.unreadableLabel
                ) { AskConversationStore.items($0, store: store) }
                return (cards.numbers, cards.items, cards.meetings)
            } ?? (nil, AskConversationStore.items(exchange.citations, store: store), [])
            let stored = AskConversationStore.append(exchange, to: id, store: store)
            if !stored { Self.log.error("conversation: could not store a follow-up") }
            // Spec part 5: after each stored exchange, join the topic most of its sources belong to.
            _ = store.autoTopic(conversationId: id)
            let link = store.conversationTopic(id)
            let cited = store.conversationCitedTopics(id)
            let title = store.getConversation(id: id)?.title
            if stored, first, let lane {
                ConversationTitleJob.schedule(
                    plan: titlePlan, routing: settings, lane: lane, conversationId: id, query: exchange.query, answer: exchange.answer,
                    ollamaModel: defaults.ollamaModel
                ) { titleUpdated.send(id) }
            }
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.stopped else { return }
                if self.entries.indices.contains(index) {
                    self.entries[index].sources = shown.items
                    self.entries[index].numbers = shown.numbers
                    self.entries[index].meetings = shown.meetings
                }
                self.topicLink = link
                self.citedTopics = cited
                if let title { self.title = title }
            }
        }
    }

    /// A new conversation's first answer: the row is made now, the way Today's Continue makes it, and the stored
    /// conversation opens in this page's place. When it could not be stored the answer goes and the question waits
    /// with an error, so the next try makes the row with it.
    private func storeFirst(_ exchange: AskExchange, scope: CoreConversationScope, appState: AppState) {
        isBusy = true
        storingDraft = true
        AskConversationStore.store(exchange, scope: scope, appState: appState) { [weak self] id in
            guard let self else { return }
            self.isBusy = false
            self.storingDraft = false
            if let id {
                if self.stopped {
                    // Left while it was being stored: the stored conversation shows on coming back (unless another
                    // page took the section meanwhile).
                    if appState.conversationDraft?.id == self.draftId {
                        appState.conversationDraft = nil
                        appState.selectedConversationId = id
                    }
                } else {
                    appState.openConversation(id: id)
                }
                return
            }
            Self.log.error("conversation: could not store a new conversation")
            if self.stopped {
                // Left while it was being stored: nothing to come back to.
                if appState.conversationDraft?.id == self.draftId { appState.conversationDraft = nil }
                return
            }
            self.entries.removeAll()
            self.pendingQuestion = exchange.query
            self.failure = String(localized: "Couldn't save the conversation. Try again.",
                                  comment: "On a new conversation page: its first answer could not be saved.")
        }
    }

    private func reloadTitle() {
        guard let store = appState?.coreStore else { return }
        let id = conversationId
        CoreMemoryStore.accessQueue.async { [weak self] in
            let title = store.getConversation(id: id)?.title
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.stopped, let title else { return }
                self.title = title
            }
        }
    }

    // MARK: Header actions

    func rename(_ newTitle: String) {
        let trimmed = newTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != title, let store = appState?.coreStore else { return }
        let id = conversationId
        let before = title
        title = trimmed
        CoreMemoryStore.accessQueue.async { [weak self] in
            let ok = store.renameConversation(id: id, newTitle: trimmed)
            DispatchQueue.main.async { [weak self] in
                if !ok, let self, self.title == trimmed { self.title = before }
            }
        }
    }

    func toggleStar() {
        guard let store = appState?.coreStore else { return }
        let id = conversationId
        let wanted = !starred
        starred = wanted
        CoreMemoryStore.accessQueue.async { [weak self] in
            let ok = store.setConversationStarred(id: id, starred: wanted)
            DispatchQueue.main.async { [weak self] in
                if !ok, let self, self.starred == wanted { self.starred = !wanted }
            }
        }
    }

    /// The user's choice of topic, or nil to remove it. Final: Whimbrel never moves it again.
    func setTopic(_ topic: TopicRef?) {
        guard let store = appState?.coreStore else { return }
        let id = conversationId
        CoreMemoryStore.accessQueue.async { [weak self] in
            _ = store.setConversationTopic(id, topicId: topic?.topicId, byUser: true)
            let link = store.conversationTopic(id)
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.stopped else { return }
                self.topicLink = link
            }
        }
    }

    /// Deletes the conversation and leaves the page (the selection clears).
    func delete() {
        guard let appState, let store = appState.coreStore else { return }
        // Not `stop`: if the delete fails the page stays and keeps working.
        cancelQuestions()
        let id = conversationId
        CoreMemoryStore.accessQueue.async {
            let ok = store.deleteConversation(id: id)
            DispatchQueue.main.async {
                guard ok else { return Self.log.error("conversation: could not delete") }
                if appState.selectedConversationId == id { appState.selectedConversationId = nil }
            }
        }
    }
}

/// A conversation's page: a stored conversation, or a new one not stored yet.
struct ConversationRedesignView: View {
    private let conversationId: Int64
    private let draft: AppState.ConversationDraft?

    @EnvironmentObject var appState: AppState
    @StateObject private var page = ConversationPageModel()
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k

    init(conversationId: Int64) {
        self.conversationId = conversationId
        draft = nil
    }

    /// A new conversation: its row is made once its first answer is in.
    init(draft: AppState.ConversationDraft) {
        conversationId = 0
        self.draft = draft
    }

    var body: some View {
        Group {
            if page.missing {
                WErrorState(title: String(localized: "This item is no longer available."), onRetry: page.reload)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if page.loaded {
                VStack(spacing: 0) {
                    ConversationHeader(page: page)
                    Divider()
                    transcript
                    Divider()
                    ConversationComposer(page: page)
                }
            } else {
                WProgressLine().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onAppear {
            if let draft { page.attachDraft(appState: appState, draft: draft) }
            else { page.attach(appState: appState, conversationId: conversationId) }
        }
        .onDisappear { page.stop() }
    }

    private static let bottomId = "conversation-bottom"

    private var transcript: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: theme.space(24)) {
                    ForEach(page.entries) { entry in
                        VStack(alignment: .leading, spacing: theme.space(10)) {
                            AskQuestionLabel(question: entry.exchange.query)
                            AskAnswerParts(exchange: entry.exchange, sources: entry.sources, numbers: entry.numbers, meetings: entry.meetings, showsModel: true)
                        }
                    }
                    if let question = page.pendingQuestion {
                        VStack(alignment: .leading, spacing: theme.space(10)) {
                            AskQuestionLabel(question: question)
                            pending(proxy)
                        }
                    }
                    Color.clear.frame(height: 1).id(Self.bottomId)
                }
                .frame(maxWidth: 760 * k, alignment: .leading)
                .padding(.horizontal, theme.space(28))
                .padding(.vertical, theme.space(16))
                .frame(maxWidth: .infinity, alignment: .center)
            }
            .onChange(of: page.entries.count) { _, _ in withAnimation { proxy.scrollTo(Self.bottomId, anchor: .bottom) } }
            .onChange(of: page.pendingQuestion) { _, _ in withAnimation { proxy.scrollTo(Self.bottomId, anchor: .bottom) } }
            // The newest answer's sources are read after it lands: keep them in view.
            .onChange(of: page.entries.last?.sources.count) { _, _ in withAnimation { proxy.scrollTo(Self.bottomId, anchor: .bottom) } }
            .onAppear { proxy.scrollTo(Self.bottomId, anchor: .bottom) }
        }
    }

    @ViewBuilder
    private func pending(_ proxy: ScrollViewProxy) -> some View {
        if let failure = page.failure {
            WErrorState(title: failure, help: nil, onRetry: page.retry)
        } else if page.nothingInScope {
            Text(String(localized: "Nothing in this topic answers that question.",
                        comment: "On a conversation asked about one topic: nothing in the topic matched the question."))
                .wFont(.body).foregroundStyle(theme.textSecondary)
            WButton(String(localized: "Ask about everything instead",
                           comment: "Button on a conversation asked about one topic: asks the same question once more about all of memory."),
                    glyph: "magnifyingglass", kind: .secondary) { page.askEverythingInstead() }
        } else if let runner = page.active {
            ConversationStreaming(runner: runner) { proxy.scrollTo(Self.bottomId, anchor: .bottom) }
        }
    }
}

/// The answer streaming in, observed straight from its runner. `onGrow` runs as the text grows, so the page can
/// follow it.
private struct ConversationStreaming: View {
    @ObservedObject var runner: AskRunner
    let onGrow: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) { AskStreamingParts(partial: runner.partial) }
            .onChange(of: runner.partial) { _, _ in onGrow() }
    }
}

/// Title (click to rename), star, topic chip and the menu with Delete; the scope line under them.
private struct ConversationHeader: View {
    @ObservedObject var page: ConversationPageModel
    @Environment(\.wTheme) private var theme
    @State private var renaming = false
    @State private var draft = ""
    @State private var confirmingDelete = false
    @State private var showingTopics = false
    @FocusState private var titleFocused: Bool

    var body: some View {
        // The layout and the scope lines are WhimbrelUI's (WConversationHeader), so the gallery draws this header too.
        WConversationHeader(scopeLine: page.scopeLine, note: page.filterNote) {
            HStack(spacing: theme.space(10)) {
                if page.isDraft {
                    // Not stored yet: nothing to rename, star, file or delete.
                    Text(verbatim: page.title).wFont(.pageTitle).foregroundStyle(theme.textPrimary).lineLimit(1)
                    Spacer(minLength: 0)
                } else {
                    titleView
                    star
                    chip
                    Spacer(minLength: 0)
                    menu
                }
            }
        }
        .confirmationDialog(
            String(localized: "Delete this conversation?", comment: "Asked before a conversation is deleted from its page."),
            isPresented: $confirmingDelete, titleVisibility: .visible
        ) {
            Button(String(localized: "Delete"), role: .destructive) { page.delete() }
            Button(String(localized: "Cancel"), role: .cancel) {}
        } message: {
            Text(String(localized: "This can't be undone."))
        }
    }

    @ViewBuilder
    private var titleView: some View {
        if renaming {
            TextField(String(localized: "Title"), text: $draft)
                .textFieldStyle(.plain)
                .wFont(.pageTitle)
                .foregroundStyle(theme.textPrimary)
                .focused($titleFocused)
                .onSubmit(commitRename)
                .onExitCommand { renaming = false }
                .onChange(of: titleFocused) { _, focused in if !focused, renaming { commitRename() } }
                .onAppear { titleFocused = true }
        } else {
            Button {
                draft = page.title
                renaming = true
            } label: {
                Text(verbatim: page.title).wFont(.pageTitle).foregroundStyle(theme.textPrimary).lineLimit(1)
            }
            .buttonStyle(.plain)
            .help(String(localized: "Rename"))
            .accessibilityHint(String(localized: "Rename"))
        }
    }

    private func commitRename() {
        renaming = false
        page.rename(draft)
    }

    private var star: some View {
        let label = page.starred ? String(localized: "Unstar conversation") : String(localized: "Star conversation")
        return Button { page.toggleStar() } label: {
            Image(systemName: page.starred ? "star.fill" : "star")
                .foregroundStyle(page.starred ? theme.accentPrimary : theme.textSecondary)
        }
        .buttonStyle(.plain)
        .help(label)
        .accessibilityLabel(label)
    }

    @ViewBuilder
    private var chip: some View {
        Group {
            switch page.chip {
            case .topic(let topic):
                WFilterChip(topic.name, glyph: "number") { showingTopics = true }
            case .add:
                WButton(String(localized: "Add to topic", comment: "Button in a conversation's header when it belongs to no topic: picks one."),
                        glyph: "plus", kind: .quiet) { showingTopics = true }
            }
        }
        .popover(isPresented: $showingTopics, arrowEdge: .bottom) {
            TopicChipMenu(page: page) { showingTopics = false }
        }
    }

    private var menu: some View {
        Menu {
            Button(String(localized: "Delete"), role: .destructive) { confirmingDelete = true }
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel(String(localized: "More"))
    }
}

/// The chip's menu: the topics its sources belong to, "Choose another topic…" (every live topic), and "Remove from
/// topic" while it has one. Every choice is the user's: final.
private struct TopicChipMenu: View {
    @ObservedObject var page: ConversationPageModel
    let dismiss: () -> Void
    @Environment(\.wTheme) private var theme
    @State private var choosingAnother = false

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(2)) {
            if choosingAnother {
                if page.allTopics.isEmpty {
                    Text(String(localized: "No topics yet")).wFont(.secondary).foregroundStyle(theme.textSecondary).padding(theme.space(8))
                } else {
                    ScrollView {
                        VStack(alignment: .leading, spacing: theme.space(2)) {
                            ForEach(page.allTopics, id: \.topicId) { topic in row(topic, current: topic.topicId == page.topicLink.topic?.topicId) }
                        }
                    }
                    .frame(maxHeight: 320)
                }
            } else {
                ForEach(page.menuTopics, id: \.topicId) { topic in row(topic, current: false) }
                if !page.menuTopics.isEmpty { Divider() }
                item(String(localized: "Choose another topic…", comment: "In a conversation's topic menu: shows every topic to pick from.")) {
                    choosingAnother = true
                }
                if page.topicLink.topic != nil {
                    item(String(localized: "Remove from topic", comment: "In a conversation's topic menu: the conversation leaves its topic.")) {
                        page.setTopic(nil)
                        dismiss()
                    }
                }
            }
        }
        .padding(theme.space(6))
        .frame(minWidth: 220, alignment: .leading)
    }

    private func row(_ topic: TopicRef, current: Bool) -> some View {
        item(topic.name, checked: current) {
            if !current { page.setTopic(topic) }
            dismiss()
        }
    }

    private func item(_ title: String, checked: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: theme.space(6)) {
                Image(systemName: "checkmark").opacity(checked ? 1 : 0).accessibilityHidden(true)
                Text(verbatim: title).wFont(.body).foregroundStyle(theme.textPrimary).lineLimit(1)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, theme.space(8))
            .padding(.vertical, theme.space(5))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(checked ? .isSelected : [])
    }
}

/// The follow-up field at the foot of the page; the setup button when Ask can't run.
private struct ConversationComposer: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject var page: ConversationPageModel
    @Environment(\.openSettings) private var openSettings
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var text = ""

    var body: some View {
        Group {
            switch HomeRedesignAdapter.ask(appState) {
            case .available(let placeholder):
                HStack(spacing: theme.space(8)) {
                    TextField(placeholder, text: $text)
                        .textFieldStyle(.plain)
                        .wFont(.body)
                        .foregroundStyle(theme.textPrimary)
                        .accessibilityLabel(placeholder)
                        .onSubmit(send)
                    WButton(String(localized: "Send"), glyph: "arrow.up", kind: .primary, action: send)
                        .disabled(!canSend)
                }
                .padding(.horizontal, theme.space(12))
                .frame(minHeight: 40 * k)
                .background(theme.backgroundCard)
                .overlay(Rectangle().strokeBorder(theme.border, lineWidth: max(1, theme.borderWidth)))
            case .needsSetup(let title):
                WButton(title, glyph: "sparkles", kind: .secondary) { openSettings() }
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(maxWidth: 760 * k)
        .padding(.horizontal, theme.space(28))
        .padding(.vertical, theme.space(12))
        .frame(maxWidth: .infinity, alignment: .center)
    }

    private var canSend: Bool {
        !page.isBusy && !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func send() {
        guard canSend else { return }
        page.ask(text)
        text = ""
    }
}
