import AppKit
import Combine
import SwiftUI
import WhimbrelUI

/// Library's Find page, behind the redesign flag: search from this device, Ask with numbered sources from the core, and the conversations.
/// The Ask button's promise (the picker, the route preview, the send decision, the sign-in refresh) is decided here; the question is then
/// asked by the shared `AskRunner` with its sourced options (the filter, `requireLocal`, this page's route). The old screen's own re-search
/// for citations is not used here.
///
/// Every finished answer is saved (`AskConversationStore`, pointers to its cited sources only): a first question makes the conversation,
/// a follow-up is added to it. The page shows no "Continue as a conversation": its answers are a saved conversation already.
///
/// The streamed text goes to `stream` and nowhere else, so a token redraws the answer card only: the runner is held here without being
/// observed by the screen, and its changes arrive by Combine.
@MainActor
final class FindController: ObservableObject {
    // MARK: Wiring

    private weak var appState: AppState?
    private weak var bridge: FindBridge?
    private var openWindow: (String, Int64) -> Void = { _, _ in }
    private var cancellables = Set<AnyCancellable>()
    private var attached = false
    let stream = WFindStream()
    /// Asks the questions (made on attach, with the app state).
    private var runner: AskRunner?
    /// Switches Library to its Find view. Set by the screen when it appears: a Command Return from Recent, Episodes or Digest asks, and the
    /// answer must be on screen when it lands.
    var showFind: () -> Void = {}

    // MARK: The picker (the old Ask mode's state)

    @Published var selectedProvider: RoutingSettings.Provider = .ollama
    @Published var ollamaModel = "gemma4:12b"
    @Published var selectedHostedModel = HostedModelDefaults.resolve(provider: "anthropic", stored: nil)
    /// Distinguishes "the picker still shows the account default" from "the person picked the default's value" (see the old screen).
    var providerPickerTouched = false

    // MARK: What the page shows

    @Published private(set) var stage: WFindStage = .empty
    @Published private(set) var results: [WFindCard] = []
    @Published private(set) var mentions: [WFindCard] = []
    @Published private(set) var page = CoreFindPage()
    @Published var order: WFindOrder = .byDay
    /// The words `order` was last set for: a new search with other words resets it from their shape.
    private var orderChosenFor: String?
    @Published private(set) var loadingMore = false
    @Published private(set) var held: [Held] = []
    @Published private(set) var focusedId: String?
    /// The core's preview of where an answer to what is typed would come from, and what it was made for: the Ask button is drawn from it
    /// only while it is still about the text, filters, history and picker on screen.
    @Published private(set) var route: CoreRoutePreview?
    private var routeFor: RouteKey?
    @Published private(set) var notice: String?
    @Published private(set) var conversations: [CoreMemoryStore.ConversationSummary] = []
    @Published private(set) var activeConversationId: Int64?
    @Published var showConversations = true
    @Published var renaming: CoreMemoryStore.ConversationSummary?
    @Published var renameText = ""
    @Published private(set) var spendTodayCents: Int64?
    @Published private(set) var namedTopics: [TopicRef] = []
    @Published private(set) var isAsking = false
    /// The answer of an ask that just ended is being saved. A follow-up waits for it: until the save lands, a first question's conversation
    /// has no id, and a second question sent now would create a second conversation.
    @Published private(set) var isSaving = false
    private var savesInFlight = 0
    /// A new value sends the follow-up field's text (Command Return while that field has the keyboard).
    @Published private(set) var followUpSubmitToken = 0
    private(set) var followUpFocused = false
    /// Something that can still be cancelled is in flight before the runner has the question: the sign-in refresh, the route check. Once the
    /// runner has it, its own `progress.canStop` says (the preparing of the prompt); the blocking model call cannot be cancelled, as today.
    @Published private(set) var canStopWaiting = false

    @Published private(set) var inFlightQuestion = ""
    /// Set when an ask finishes, never when a saved conversation is opened: the page says the answer aloud once per value.
    @Published private(set) var announceToken: String?
    @Published private(set) var restore: WFindRestore?
    @Published private(set) var focusToken = 0
    private(set) var inFlightModelLabel = ""
    private var inFlightField: WFindField = .header
    private var restoreCounter = 0
    /// Another screen of Library keeps the model busy (a digest): asking waits, as on the old page where both shared one busy flag.
    var isBusyElsewhere: () -> Bool = { false }

    /// One question and its answer on the page, with what the next question needs (the answer's text, for the history).
    struct Held: Identifiable {
        var view: WFindExchange
        var answer: String?
        var id: String { view.id }
    }

    /// What a preview was made for. A preview of other words, filters, history or picker choices says nothing about this ask.
    private struct RouteKey: Equatable {
        var query: String
        var filter: WHomeFilter
        var historyCount: Int
        var epoch: Int
        var picker: String
    }

    /// The picker as it was when an ask (or a preview) began. Everything that ask does, the preview and the plan included, reads this and not
    /// the live picker, so a change made while it is being prepared cannot make the two differ.
    struct PickerSnapshot: Equatable {
        var provider: RoutingSettings.Provider
        var ollamaModel: String
        var hostedModel: String
        var touched: Bool
        var modelLabel: String { provider == .ollama ? ollamaModel : hostedModel }
        var key: String { [provider.rawValue, ollamaModel, hostedModel, touched ? "1" : "0"].joined(separator: "|") }
    }

    private func pickerSnapshot() -> PickerSnapshot {
        PickerSnapshot(provider: selectedProvider, ollamaModel: ollamaModel, hostedModel: selectedHostedModel, touched: providerPickerTouched)
    }

    // MARK: Bookkeeping

    private lazy var starters: [WFindStarter] = CoreMemoryStore.starterPrompts().map { WFindStarter(title: $0.title, query: $0.query) }
    /// A generation for each kind of asynchronous work. A result that comes back after its generation moved on lands nowhere.
    private var searchSeq = 0
    private var routeSeq = 0
    private var loadSeq = 0
    private var askSeq = 0
    /// Which conversation the page shows. It moves when a conversation is started, opened or deleted, so an answer that finishes after
    /// that is saved to its own conversation and never drawn into another (a first ask has no conversation id yet, so ids cannot tell).
    private var epoch = 0
    /// The filters the open conversation's answers were made under; a change after an answer starts a new conversation.
    private var conversationFilter = WHomeFilter()
    private var gateToken: UUID?
    /// The ask the runner is answering, and whether it was sent held to this Mac.
    private var inFlight: (request: AskRequest, requireLocal: Bool)?
    /// The ask is before the runner has it: keys, route check, sign-in refresh. Stop can end it.
    private var preparing = false

    /// Everything an ask needs that is read once, when it starts, and sent as one value.
    private struct AskRequest {
        let seq: Int
        let question: String
        let field: WFindField
        /// What the Ask button said when it was pressed; nil when no button was on screen.
        let label: WFindAskButton?
        let filter: CoreFindFilter?
        let scope: String
        let history: [CoreChatTurn]
        /// The same earlier exchanges, as the runner takes them.
        let exchanges: [AskExchange]
        let epoch: Int
        let conversationId: Int64?
        let picker: PickerSnapshot
        var ollamaModel: String { picker.ollamaModel }
    }

    private struct Plan {
        var effective: (provider: RoutingSettings.Provider, modelLabel: String)
        var routing: CoreRoutingConfig
        var streamRoute: AskStreamRoute?
        var history: [CoreChatTurn]
    }

    // MARK: Strings and cards

    private var f: WFindStrings { WhimbrelUIStrings.make().find }

    private func env() -> FindCardEnv? {
        appState.map { FindCardEnv.live(appState: $0, strings: f) }
    }

    // MARK: Attach

    func attach(appState: AppState, bridge: FindBridge, openWindow: @escaping (String, Int64) -> Void) {
        self.appState = appState
        self.bridge = bridge
        self.openWindow = openWindow
        guard !attached else { return }
        attached = true
        let defaults = appState.routing.defaultSelection()
        selectedProvider = defaults.provider
        ollamaModel = defaults.ollamaModel
        if defaults.provider != .ollama { selectedHostedModel = defaults.hostedModel }
        providerPickerTouched = false
        conversationFilter = bridge.filter

        bridge.filterChanges.removeDuplicates().sink { [weak self] in self?.filterChanged($0) }.store(in: &cancellables)
        bridge.$searchRequest.dropFirst().sink { [weak self] _ in self?.search() }.store(in: &cancellables)
        // The request comes through the controller, not the screen: the screen may not be showing (Recent, Episodes, Digest) when it arrives.
        bridge.$askRequest.dropFirst().sink { [weak self] _ in self?.askFromHeader() }.store(in: &cancellables)
        // The route is looked at when there are words to look at, a pause after typing: not when Find opens.
        bridge.queryChanges.removeDuplicates().debounce(for: .milliseconds(350), scheduler: DispatchQueue.main)
            .sink { [weak self] _ in self?.refreshRoute() }.store(in: &cancellables)
        // A change of routing settings (a key saved, local-only mode, the default provider) makes any earlier preview stale.
        appState.routing.objectWillChange.debounce(for: .milliseconds(350), scheduler: DispatchQueue.main)
            .sink { [weak self] _ in self?.routingSettingsChanged() }.store(in: &cancellables)
        // The runner drives the answer card by Combine. `@Published` sends on will-set, so the new value is the argument.
        let runner = AskRunner(appState: appState, scope: .everything)
        self.runner = runner
        runner.$partial.sink { [weak self] text in self?.stream.text = text }.store(in: &cancellables)
        runner.$progress.dropFirst().removeDuplicates().sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &cancellables)
        runner.$phase.dropFirst().sink { [weak self] phase in self?.runnerPhaseChanged(phase) }.store(in: &cancellables)
        // A generated title lands after the save: the list shows it.
        appState.conversationTitleUpdated.sink { [weak self] _ in self?.refreshConversationList() }.store(in: &cancellables)

        refreshConversationList()
        refreshSpendToday()
        if !bridge.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !bridge.filter.isEmpty { search() }
    }

    /// The screen went away (another section was chosen). The controller lives on with the shell, so an answer being written keeps arriving,
    /// is saved, and is there on return; only what belongs to the screen's own controls is reset. The model gate is held for as long as the
    /// ask is in flight and released when it ends.
    func detach() {
        followUpFocused = false
    }

    func followUpFocusChanged(_ focused: Bool) { followUpFocused = focused }

    // MARK: The model

    /// What the Ask button said, as drawn now. Nothing is promised while the preview is about other words than the ones in the field.
    private func currentAskButton(canAsk: Bool, strings: WFindStrings) -> WFindAskButton {
        let live = (route != nil && routeFor == currentRouteKey()) ? route : nil
        return WFindAskButton.make(route: FindAdapter.route(live), canAsk: canAsk && !isBusyElsewhere() && !isSaving, strings: strings)
    }

    /// A provider is set up: asking can run at all (otherwise the page shows its set-up affordance in place of the Ask row).
    private var canAskNow: Bool { appState.map { HomeRedesignAdapter.ask($0).isAvailable } ?? false }

    func model() -> WFindModel {
        var m = WFindModel()
        guard let appState, let bridge else { return m }
        let all = WhimbrelUIStrings.make()
        let strings = all.find
        m.stage = stage
        m.query = bridge.query
        m.order = order
        m.capped = page.capped
        m.noMeaningSearch = !page.semanticAvailable && !bridge.query.isEmpty && stage == .results
        m.starters = starters
        m.canAsk = canAskNow
        m.askButton = currentAskButton(canAsk: m.canAsk, strings: strings)
        m.results = results
        m.mentions = mentions
        m.mentionsName = bridge.filter.person
        m.mentionsTotal = page.mentionsTotal
        m.more = page.more
        m.loadingMore = loadingMore
        m.exchanges = held.map(\.view)
        m.focusedExchangeId = focusedId
        m.announceToken = announceToken
        m.restore = restore
        m.focusToken = focusToken
        m.notice = notice
        m.hasFilters = !bridge.filter.isEmpty
        m.showsConversations = showConversations
        // An answer being written belongs to the open conversation: deleting it then would drop the answer, so Delete is off for it.
        let answerInFlight = isAsking || isSaving
        m.submitFollowUpToken = followUpSubmitToken
        m.conversations = sortedForSidebar(conversations).map {
            WFindConversation(id: $0.id, title: $0.title, starred: $0.starred, isActive: $0.id == activeConversationId, canDelete: !(answerInFlight && $0.id == activeConversationId))
        }
        let apps = appNames
        let range = FindAdapter.rangeText(bridge.filter)
        m.filterLines = FindAdapter.filterLines(bridge.filter, appName: apps, range: range)
        m.filterSummary = FindAdapter.filterSummary(bridge.filter, appName: apps, datesTitle: datesTitle(bridge.filter), strings: all.home)
        let progress = runner?.progress ?? AskRunner.Progress()
        if inFlight != nil, progress.streaming {
            m.asking = .streaming(question: inFlightQuestion, line: spinnerLine())
        } else if isAsking {
            m.asking = .waiting(line: spinnerLine(), canStop: inFlight != nil ? progress.canStop : canStopWaiting)
        }
        m.countText = countText(focused: m.focusedExchange, strings: strings)
        return m
    }

    /// "Results: 3", or the sources of the answer shown. Cheap: the shell asks for it without a whole model.
    func countText() -> String? {
        let focused = held.first { $0.id == focusedId }?.view ?? held.last?.view
        return countText(focused: focused, strings: f)
    }

    private func countText(focused: WFindExchange?, strings: WFindStrings) -> String? {
        FindAdapter.countText(
            total: page.total, mentions: page.mentionsTotal, answerSources: focused.flatMap { $0.sources.isEmpty ? nil : $0.sources.count }, strings: strings)
    }

    private var appNames: (String) -> String { { HomeRedesignAdapter.appName($0) } }

    private func datesTitle(_ filter: WHomeFilter) -> String? {
        guard let dates = filter.dates else { return nil }
        let h = WhimbrelUIStrings.make().home
        switch dates {
        case .today: return h.today
        case .yesterday: return h.yesterday
        case .lastSevenDays: return h.lastSevenDays
        case .exact(_, _, let title): return title
        case .custom(let from, let to):
            let style = Date.FormatStyle().month(.abbreviated).day()
            return "\(from.formatted(style)) - \(to.formatted(style))"
        }
    }

    private func spinnerLine() -> String {
        // Before the runner has the question nothing is known yet: the plain "Asking ..." line.
        let p = inFlight != nil ? (runner?.progress ?? AskRunner.Progress()) : AskRunner.Progress()
        return AskLocalRouteCopy.spinnerTitle(
            routedLocally: p.routedLocally, waitingForLane: p.waitingForLane, modelConfirmed: p.modelConfirmed, modelLabel: inFlightModelLabel)
    }

    var currentModelLabel: String { pickerSnapshot().modelLabel }

    // MARK: Search

    private func trimmedQuery() -> String { (bridge?.query ?? "").trimmingCharacters(in: .whitespacesAndNewlines) }

    func search() {
        guard let appState, let bridge else { return }
        let query = trimmedQuery()
        let filter = bridge.filter
        refreshNamedTopics(query)
        // New words open on the order their shape suggests (a question on best matches first); the same words again, or a
        // change of order from the picker, keep the order chosen.
        if query != orderChosenFor {
            orderChosenFor = query
            order = .default(for: query)
        }
        searchSeq += 1
        let seq = searchSeq
        guard !query.isEmpty || !filter.isEmpty else {
            results = []; mentions = []; page = CoreFindPage()
            stage = .empty
            bridge.countText = countText()
            return
        }
        guard let store = appState.coreStore else {
            stage = .failed(title: f.searchFailedTitle, help: TimelineView.storeUnavailableMessage)
            return
        }
        stage = .searching
        readFirstPage(store: store, query: query, filter: FindAdapter.coreFilter(filter), order: order.coreName, seq: seq, tries: 0, limit: Self.pageSize)
        refreshRoute()
    }

    /// How many times at most a search is read again while its meaning results are catching up: 5 quick reads, then one every 10 s
    /// for about ten minutes. The embedding sweep waits for mains power and an idle Mac, so it can be minutes away; a read is one search.
    nonisolated static let catchUpTries = 65

    /// When to read `page`'s search again after `tries` reads: while core says new rows still wait for their meaning, a bounded
    /// number of times.
    nonisolated static func catchUpDelay(_ page: CoreFindPage, tries: Int) -> TimeInterval? {
        guard page.meaningCatchingUp, tries < catchUpTries else { return nil }
        return tries < 5 ? 2 : 10
    }

    /// Results asked for at a time; Load more asks for the next page.
    nonisolated static let pageSize = 50
    /// The most rows one read returns (core's `FIND_MAX_LIMIT`).
    nonisolated static let findMaxLimit = 200

    /// How many rows a catch-up read asks for when `loaded` rows are shown: all of them, at least a page. Nil past what one read
    /// returns, and then the catch-up reads stop, so the rows Load more added stay as they are.
    nonisolated static func catchUpLimit(loaded: Int) -> Int? {
        let n = max(pageSize, loaded)
        return n > findMaxLimit ? nil : n
    }

    /// The rows to show after a catch-up read that asked for the first `asked` rows: its answer, then the rows loaded past those, each
    /// once.
    nonisolated static func mergeCatchUp(shown: [CoreSourceCard], answer: [CoreSourceCard], asked: Int) -> [CoreSourceCard] {
        var seen = Set(answer.map { "\($0.kind)-\($0.id)" })
        var out = answer
        for card in shown.dropFirst(asked) where seen.insert("\(card.kind)-\(card.id)").inserted {
            out.append(card)
        }
        return out
    }

    /// Reads a search's first `limit` rows and shows them. While its meaning results are still catching up (rows a local Claude added,
    /// synced rows, waiting for their embedding) the rows shown are read again and replace what is shown, so the same words never stay
    /// on an older, shorter answer; rows Load more added are read again with them, never dropped. A newer search (`searchSeq` moved)
    /// ends it.
    private func readFirstPage(store: CoreMemoryStore, query: String, filter core: CoreFindFilter, order orderName: String, seq: Int, tries: Int, limit: Int) {
        CoreMemoryStore.accessQueue.async {
            let page = store.memoryFind(query: query, filter: core.isEmpty ? nil : core, order: orderName, offset: 0, limit: limit)
            DispatchQueue.main.async { [weak self] in
                guard let self, seq == self.searchSeq else { return }
                guard let page, let env = self.env() else {
                    self.stage = .failed(title: self.f.searchFailedTitle, help: String(localized: "Whimbrel's local database didn't answer. Try again.", comment: "Help line under the Library search failure title."))
                    return
                }
                var shown = page
                if tries > 0 {
                    // Rows Load more added past what this read asked for stay, and so does its word on whether there is more.
                    let kept = self.page.results.count > limit || self.page.mentions.count > limit
                    shown.results = Self.mergeCatchUp(shown: self.page.results, answer: page.results, asked: limit)
                    shown.mentions = Self.mergeCatchUp(shown: self.page.mentions, answer: page.mentions, asked: limit)
                    if kept { shown.more = self.page.more }
                }
                self.page = shown
                self.results = shown.results.map { FindAdapter.card($0, env: env) }
                self.mentions = shown.mentions.map { FindAdapter.card($0, env: env) }
                self.stage = (page.results.isEmpty && page.mentions.isEmpty) ? .noResults : .results
                self.bridge?.countText = self.countText()
                if let delay = Self.catchUpDelay(page, tries: tries) {
                    DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                        guard let self, seq == self.searchSeq, !self.loadingMore,
                              let next = Self.catchUpLimit(loaded: max(self.page.results.count, self.page.mentions.count)) else { return }
                        self.readFirstPage(store: store, query: query, filter: core, order: orderName, seq: seq, tries: tries + 1, limit: next)
                    }
                }
            }
        }
    }

    func loadMore() {
        guard let appState, let store = appState.coreStore, page.more, !loadingMore, let bridge else { return }
        loadingMore = true
        let seq = searchSeq
        let query = trimmedQuery()
        let core = FindAdapter.coreFilter(bridge.filter)
        let offset = results.count
        let orderName = order.coreName
        CoreMemoryStore.accessQueue.async {
            let next = store.memoryFind(query: query, filter: core.isEmpty ? nil : core, order: orderName, offset: offset, limit: Self.pageSize)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                // A newer search owns the list now: this page of an older one is dropped, and the flag cleared only if it is still ours.
                guard seq == self.searchSeq else { self.loadingMore = false; return }
                self.loadingMore = false
                guard let next, let env = self.env() else { return }
                var merged = self.page
                merged.results += next.results
                merged.mentions += next.mentions
                merged.more = next.more
                self.page = merged
                self.results += next.results.map { FindAdapter.card($0, env: env) }
                self.mentions += next.mentions.map { FindAdapter.card($0, env: env) }
            }
        }
    }

    func setOrder(_ new: WFindOrder) {
        guard new != order else { return }
        order = new
        search()
    }

    /// A filter change after an answer starts a new conversation, as the spec says. While an ask is in flight it goes on under the filters it
    /// was sent with and its answer lands in the conversation it came from; the new conversation then starts with the next question
    /// (`ask` checks the same rule), so an in-flight answer is never dropped or filed under filters it was not made with.
    private func filterChanged(_ new: WHomeFilter) {
        if !isAsking, FindAdapter.startsNewConversation(old: conversationFilter, new: new, hasExchanges: !held.isEmpty) {
            startNewConversation()
            notice = f.newConversationNote
        }
        // While the first ask is in flight the conversation keeps the filters that ask was sent with.
        if held.isEmpty, !isAsking { conversationFilter = new }
        search()
    }

    func clearFilters() { bridge?.setFilter(WHomeFilter()) }

    private func refreshNamedTopics(_ text: String) {
        guard let store = appState?.coreStore, !text.isEmpty else { namedTopics = []; return }
        CoreMemoryStore.accessQueue.async {
            let found = store.topicsNamedIn(text: text) ?? []
            DispatchQueue.main.async { [weak self] in self?.namedTopics = found }
        }
    }

    func useStarter(_ starter: WFindStarter) {
        bridge?.setQuery(starter.query)
        search()
    }

    // MARK: Open

    func open(_ target: WFindOpen) {
        switch target {
        case .capture(let id): openWindow("capture-detail", id)
        case .meetingAt(let id, let cs): appState?.openMeeting(id: id, at: cs)
        case .meeting(let id): openWindow("meeting-detail", id)
        case .episode(let id): openWindow("episode-detail", id)
        case .thread(let id): openWindow("thread-detail", id)
        }
    }

    // MARK: The route (the Ask button's promise)

    private func currentRouteKey(picker: PickerSnapshot? = nil) -> RouteKey {
        RouteKey(
            query: trimmedQuery(), filter: bridge?.filter ?? WHomeFilter(), historyCount: held.filter { $0.answer != nil }.count, epoch: epoch,
            picker: (picker ?? pickerSnapshot()).key)
    }

    /// Works out how an answer would be made, as the call will make it: the same effective routing, and "stream" only when there is a stream
    /// route. The provider keys come in from the caller, read off the main thread.
    private func makePlan(query: String, history: [CoreChatTurn], keys: RoutingSettings.ProviderKeys, picker: PickerSnapshot) -> Plan? {
        guard let appState else { return nil }
        let defaults = appState.routing.defaultSelection()
        let effective = effectiveAskRoutingProvider(
            query: query, pickerProvider: picker.provider, pickerModelLabel: picker.modelLabel, pickerTouched: picker.touched, defaults: defaults)
        let routing = appState.routing.routingConfig(
            preferredProvider: effective.provider, preferredModel: effective.modelLabel, customInstructions: appState.promptLibrary.instructions[.ask] ?? "", keys: keys)
        let streamRoute = askStreamRoute(
            provider: effective.provider, modelLabel: effective.modelLabel, localOnlyMode: appState.routing.localOnlyMode,
            replacesOllama: LocalModel.replacesOllama(appState.routing), whimbrelCredentials: appState.syncState.whimbrelCredentials,
            loadKey: { keys.key(for: $0) })
        return Plan(effective: effective, routing: routing, streamRoute: streamRoute, history: history)
    }

    /// The page's earlier answers as the runner takes them, oldest first (a failed exchange has no answer to replay).
    private func answeredExchanges() -> [AskExchange] {
        held.compactMap { h in h.answer.map { AskExchange(query: h.view.question, answer: $0, citations: [], model: nil, answeredLocally: false) } }
    }

    /// The same as model history: what the runner sends, so the route preview is made for it.
    private func answeredHistory() -> [CoreChatTurn] { AskRunner.history(answeredExchanges()) }

    /// Runs `work` on a queue and comes back to the caller's actor with its result.
    private func offMain<T>(_ queue: DispatchQueue, _ work: @escaping () -> T) async -> T {
        await withCheckedContinuation { continuation in queue.async { continuation.resume(returning: work()) } }
    }

    private func previewOf(question: String, plan: Plan, filter: CoreFindFilter?, ollamaModel: String) async -> CoreRoutePreview? {
        guard let store = appState?.coreStore else { return nil }
        let mode = plan.streamRoute != nil ? "stream" : "blocking"
        let history = plan.history, routing = plan.routing
        return await offMain(CoreMemoryStore.accessQueue) {
            store.askRoutePreview(question, model: ollamaModel, history: history, routing: routing, filter: filter, requireLocal: false, mode: mode)
        }
    }

    /// Looks at where an answer to the words in the field would come from, for the Ask button. The Keychain is read off the main thread, only
    /// when there is something to ask, and the result is kept only if nothing it was made for has changed meanwhile.
    func refreshRoute() {
        routeSeq += 1
        let seq = routeSeq
        let query = trimmedQuery()
        guard let appState, appState.coreStore != nil, let bridge, HomeRedesignAdapter.ask(appState).isAvailable, !query.isEmpty else {
            route = nil
            routeFor = nil
            return
        }
        // The picker is read once, here: the key, the plan and the preview all describe the same choice, whatever the picker does meanwhile.
        let picker = pickerSnapshot()
        let key = currentRouteKey(picker: picker)
        let core = FindAdapter.coreFilter(bridge.filter)
        let history = answeredHistory()
        Task { @MainActor [weak self] in
            let keys = await self?.offMain(DispatchQueue.global(qos: .userInitiated)) { RoutingSettings.ProviderKeys.load() }
            guard let self, let keys, seq == self.routeSeq, let plan = self.makePlan(query: query, history: history, keys: keys, picker: picker) else { return }
            let preview = await self.previewOf(question: query, plan: plan, filter: core.isEmpty ? nil : core, ollamaModel: picker.ollamaModel)
            guard seq == self.routeSeq else { return }
            self.route = preview
            self.routeFor = key
        }
    }

    private func routingSettingsChanged() {
        route = nil
        routeFor = nil
        refreshRoute()
    }

    // MARK: Ask

    private var busy: Bool { isAsking || isSaving || isBusyElsewhere() }

    /// Tells the shell whether an answer is being written or saved, so the header's Ask button on other sections shows off.
    private func syncBusy() { bridge?.askBusy = isAsking || isSaving }

    /// The Ask button and Command Return on the page: asks with what the field holds, and keeps the promise the button made.
    func askFromHeader() {
        // Command Return can arrive while Recent, Episodes or Digest shows. Find comes first, so the answer, its Stop and its notes are on
        // screen when the ask begins and not somewhere the person is not looking.
        showFind()
        if followUpFocused {
            // The follow-up field has the keyboard: Command Return sends what is in it, as Return does, and not the header field's text.
            followUpSubmitToken += 1
            return
        }
        let query = trimmedQuery()
        guard !query.isEmpty else { return }
        // No provider is set up: the same set-up the Ask row's button offers, not a silent nothing.
        guard canAskNow else {
            openAISettings()
            return
        }
        ask(query, from: .header, label: currentAskButton(canAsk: true, strings: f))
    }

    /// The follow-up field. Returns whether the ask started: the field keeps its text when it did not.
    func askFollowUp(_ text: String) -> Bool { ask(text, from: .followUp, label: nil) }

    /// Sends `question` to a model. Nothing calls this but the Ask button, Command Return, the follow-up field and a hand-off from another
    /// screen: Enter only searches. It looks at where the answer would come from for this very question, history, filters and picker, and
    /// sends only if that keeps what the button said. Returns whether the ask started.
    ///
    /// From the moment it returns true until `setAsking(false)`, every way out ends in `fail` or in an answer: a stop, a failure, a missing
    /// store, a call that returns nothing. None leaves `isAsking` set, the model gate held, or the question gone from its field.
    @discardableResult
    func ask(_ words: String, from field: WFindField = .header, label: WFindAskButton? = nil) -> Bool {
        // Trimmed here, whatever the caller did: blank words never start an ask.
        guard appState != nil, let bridge, canAskNow, !busy, let question = WAskStart.question(words, busy: false) else { return false }
        notice = nil
        // An answer made under one set of filters is never fed into a question under another (also after a change made mid-answer).
        if FindAdapter.startsNewConversation(old: conversationFilter, new: bridge.filter, hasExchanges: !held.isEmpty) {
            startNewConversation()
            notice = f.newConversationNote
        }
        if held.isEmpty { conversationFilter = bridge.filter }
        askSeq += 1
        let filter = FindAdapter.coreFilter(bridge.filter)
        // The picker is read once, here, and travels with the request.
        let picker = pickerSnapshot()
        let request = AskRequest(
            seq: askSeq, question: question, field: field, label: label, filter: filter.isEmpty ? nil : filter,
            scope: FindAdapter.rangeText(bridge.filter).map { f.fromMemoryRange($0) } ?? f.fromMemory, history: answeredHistory(),
            exchanges: answeredExchanges(), epoch: epoch,
            conversationId: activeConversationId, picker: picker)
        inFlightQuestion = question
        inFlightField = field
        inFlightModelLabel = picker.modelLabel
        stream.text = ""
        preparing = true
        canStopWaiting = true
        setAsking(true)
        Task { @MainActor [weak self] in await self?.run(request) }
        return true
    }

    private func isLive(_ request: AskRequest) -> Bool { request.seq == askSeq }

    /// The steps before the model call, each checked against the ask's generation: a Stop in between ends it quietly (it has already put
    /// the question back and cleared the state).
    private func run(_ request: AskRequest) async {
        let keys = await offMain(DispatchQueue.global(qos: .userInitiated)) { RoutingSettings.ProviderKeys.load() }
        guard isLive(request) else { return }
        guard let appState, var plan = makePlan(query: request.question, history: request.history, keys: keys, picker: request.picker) else {
            failStoreUnavailable(request)
            return
        }
        var preview = await previewOf(question: request.question, plan: plan, filter: request.filter, ollamaModel: request.ollamaModel)
        guard isLive(request) else { return }
        var decision = WFindSendDecision.decide(label: request.label, fresh: FindAdapter.route(preview))
        // Whimbrel AI needs a live sign-in token, but a call that stays on this Mac sends nothing there: no refresh then.
        let syncState = appState.syncState
        let wantsWhimbrel = request.picker.provider == .whimbrel || appState.routing.preferredProvider == .whimbrel
        if case .send(let requireLocal) = decision, !requireLocal, wantsWhimbrel, syncState.whimbrelCredentialsNeedRefresh {
            await syncState.refreshWhimbrelCredentials()
            guard isLive(request) else { return }
            guard let again = makePlan(query: request.question, history: request.history, keys: keys, picker: request.picker) else {
                failStoreUnavailable(request)
                return
            }
            plan = again
            preview = await previewOf(question: request.question, plan: plan, filter: request.filter, ollamaModel: request.ollamaModel)
            guard isLive(request) else { return }
            decision = WFindSendDecision.decide(label: request.label, fresh: FindAdapter.route(preview))
        }
        switch decision {
        case .changed:
            // Nothing is sent. The label is made again from the page as it is now. The words are the neutral ones: the cause may be a change
            // of settings, a key, the picker or the question, and nothing here knows which.
            route = nil
            routeFor = nil
            refreshRoute()
            fail(request, message: f.routeChanged, button: nil)
        case .send(let requireLocal):
            send(request, plan: plan, requireLocal: requireLocal)
        }
    }

    /// Hands the question to the runner, with the route this page worked out and the promise the button made (`requireLocal`, kept by every
    /// call the runner makes, the blocking fallback included).
    private func send(_ request: AskRequest, plan: Plan, requireLocal: Bool) {
        guard isLive(request) else { return }
        guard let runner else {
            failStoreUnavailable(request)
            return
        }
        // A stopped stream still winding down (the local model warming up): not lost, the page says why.
        guard !runner.isBusy else {
            fail(request, message: f.askBusyReason, button: nil)
            return
        }
        inFlightModelLabel = plan.effective.modelLabel
        preparing = false
        canStopWaiting = false
        inFlight = (request, requireLocal)
        let started = runner.ask(request.question, history: request.exchanges, options: AskRunner.Options(
            requireLocal: requireLocal, sourced: true, filter: request.filter,
            route: AskRunner.Route(config: plan.routing, stream: plan.streamRoute, model: plan.effective.modelLabel, ollamaModel: request.ollamaModel)))
        // Nothing started (no words, or a question still in flight): the page is not left asking, and the question goes back.
        guard started else {
            inFlight = nil
            fail(request, message: f.askBusyReason, button: nil)
            return
        }
    }

    /// The runner's question ended (`@Published` will-set: the new phase is the argument).
    private func runnerPhaseChanged(_ phase: AskRunner.Phase) {
        guard let flight = inFlight else { return }
        let request = flight.request, requireLocal = flight.requireLocal
        switch phase {
        case .streaming:
            return
        case .done(let exchange):
            inFlight = nil
            setAsking(false)
            complete(request, exchange: exchange, requireLocal: requireLocal)
        case .failed(let message):
            inFlight = nil
            let error = runner?.lastError
            fail(request, message: message, button: error?.opensAIProvidersSettings == true ? String(localized: "Open AI Providers settings") : nil)
        case .idle:
            // Stopped: nothing is shown, and the question goes back to its field.
            inFlight = nil
            setAsking(false)
            restoreQuestion(request.question, field: request.field)
            notice = f.stoppedNote
        case .nothingInScope:
            // Not reached with everything in scope; shown as a failure rather than an empty answer.
            inFlight = nil
            fail(request, message: FindAdapter.failureText(.providerFailure), button: nil)
        }
    }

    /// Stop: while the answer streams, and while the ask is still getting ready (the sign-in refresh, the route check, the preparing of the
    /// prompt). The blocking model call has no Stop, as today. Either way the question goes back to the field it came from.
    func stop() {
        if inFlight != nil, let runner {
            if runner.progress.streaming || runner.progress.canStop { runner.cancel() }
        } else if isAsking, preparing {
            askSeq += 1
            preparing = false
            canStopWaiting = false
            setAsking(false)
            restoreQuestion(inFlightQuestion, field: inFlightField)
            notice = f.stoppedNote
        }
    }

    // MARK: Outcomes

    /// The notes under an answer. Saved with the answer: whether it was made on this Mac, and how many items were left out. The reason it was
    /// made here is not saved (it may be a client excluded from hosted models, or Local-only mode), and neither is which computer, so a
    /// reopened answer says only "by a local model" (conversations do not sync today, but the wording stays true if they ever do). A live
    /// one knows it happened here.
    private func notes(leftOut: Int, answeredLocally: Bool, reopened: Bool) -> [String] {
        var out: [String] = []
        if let line = ClientCopy.leftOut(leftOut) { out.append(line) }
        if answeredLocally { out.append(reopened ? f.answeredByLocalModel : f.answeredOnThisMac) }
        return out
    }

    private func complete(_ request: AskRequest, exchange: AskExchange, requireLocal: Bool) {
        if let env = env(), request.epoch == epoch {
            let sourced = exchange.sourced
            let cards = FindAdapter.sourceCards(sourced?.sources ?? [], cited: sourced?.cited ?? [], env: env)
            // The line names the range the answer actually drew on when the core says one (a "yesterday" question has no filter of its own).
            let scope = FindAdapter.rangeText(sourced?.dateRange).map { f.fromMemoryRange($0) } ?? request.scope
            let view = WFindExchange(
                id: UUID().uuidString, question: request.question, scope: scope, body: .answered(text: exchange.answer), modelLine: exchange.model,
                notes: notes(leftOut: exchange.leftOut, answeredLocally: exchange.madeOnThisComputer, reopened: false), sources: cards,
                cited: sourced?.cited ?? [])
            held.append(Held(view: view, answer: exchange.answer))
            focusedId = view.id
            announceToken = view.id
        }
        refreshSpendToday()
        persist(request, exchange: exchange, requireLocal: requireLocal)
        // The history changed, so the next question may be routed differently.
        refreshRoute()
    }

    /// Puts a question that did not go back in the field it was typed in. The header field is the shell's; the follow-up field takes a token.
    private func restoreQuestion(_ question: String, field: WFindField) {
        switch field {
        case .header: bridge?.setQuery(question)
        case .followUp:
            restoreCounter += 1
            restore = WFindRestore(token: restoreCounter, field: .followUp, text: question)
        }
    }

    /// The one way an ask ends without an answer: the asking state and the gate are cleared, the question goes back to the field it came
    /// from, and the page says why. Every early exit of an ask that has begun comes here.
    private func fail(_ request: AskRequest, message: String, button: String?) {
        preparing = false
        canStopWaiting = false
        setAsking(false)
        // A page that moved on (another conversation) is not touched: the question goes back only where it was asked.
        guard request.epoch == epoch else { return }
        restoreQuestion(request.question, field: request.field)
        let view = WFindExchange(id: UUID().uuidString, question: request.question, scope: request.scope, body: .failed(message: message, buttonTitle: button))
        held.append(Held(view: view, answer: nil))
        focusedId = view.id
        announceToken = view.id
    }

    /// The store, the plan or a core call was not there: the same failure, with the old page's own words for a database that did not open.
    private func failStoreUnavailable(_ request: AskRequest) {
        fail(request, message: TimelineView.storeUnavailableMessage, button: nil)
    }

    func openAISettings() {
        appState?.pendingSettingsSection = .ai
        appState?.openSettingsWindow?()
    }

    // MARK: Gate

    private func setAsking(_ asking: Bool) {
        isAsking = asking
        defer { syncBusy() }
        if asking {
            if gateToken == nil { gateToken = UserModelGate.shared.enter() }
        } else {
            releaseGate()
            preparing = false
            canStopWaiting = false
        }
    }

    private func releaseGate() {
        UserModelGate.shared.leave(gateToken)
        gateToken = nil
    }

    // MARK: Conversations

    func focus(_ id: String) { focusedId = id }

    func startNewConversation() {
        epoch += 1
        searchSeq += 1
        // A saved conversation still loading must not land on the blank page this makes.
        loadSeq += 1
        held = []
        focusedId = nil
        activeConversationId = nil
        notice = nil
        namedTopics = []
        conversationFilter = bridge?.filter ?? WHomeFilter()
        route = nil
        routeFor = nil
        refreshRoute()
    }

    /// Command N, or a header Ask from another tab: a new conversation, and a question to send when there is one. A question from another
    /// screen had no Ask button on screen, so it promises nothing; where the answer would come from is looked at for it before it is sent.
    func newConversation(asking prompt: String?) {
        guard !busy else {
            // Not dropped without a word: a question that came from another screen waits in the field, and the page says why.
            if let prompt, !prompt.isEmpty { bridge?.setQuery(prompt) }
            if isAsking || isSaving { notice = f.askBusyReason }
            return
        }
        startNewConversation()
        if let prompt, !prompt.isEmpty {
            bridge?.setQuery(prompt)
            ask(prompt, from: .header, label: nil)
        }
    }

    func refreshConversationList() {
        guard let store = appState?.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let list = store.listConversations() ?? []
            DispatchQueue.main.async { [weak self] in self?.conversations = list }
        }
    }

    func toggleStar(_ id: Int64) {
        guard let store = appState?.coreStore, let c = conversations.first(where: { $0.id == id }) else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.setConversationStarred(id: id, starred: !c.starred)
            DispatchQueue.main.async { [weak self] in self?.refreshConversationList() }
        }
    }

    func beginRename(_ id: Int64) {
        guard let c = conversations.first(where: { $0.id == id }) else { return }
        renameText = c.title
        renaming = c
    }

    func commitRename() {
        guard let c = renaming else { return }
        renaming = nil
        let title = renameText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let store = appState?.coreStore, !title.isEmpty else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.renameConversation(id: c.id, newTitle: title)
            DispatchQueue.main.async { [weak self] in self?.refreshConversationList() }
        }
    }

    func deleteConversation(_ id: Int64) {
        guard let store = appState?.coreStore else { return }
        // Not while an answer is being written in it: it would be dropped, and the page left spinning in a new empty conversation. The
        // control is off in the list (`WFindConversation.canDelete`); this is the same rule for any other way in.
        if id == activeConversationId, isAsking || isSaving { return }
        if id == activeConversationId { startNewConversation() }
        CoreMemoryStore.accessQueue.async {
            _ = store.deleteConversation(id: id)
            DispatchQueue.main.async { [weak self] in self?.refreshConversationList() }
        }
    }

    /// Opens a saved conversation. An everything conversation opens here under the Dates and Apps filters it kept, and its follow-ups are
    /// asked under them; when it was asked under a People filter, which is never kept, the page says so. A topic or meeting conversation
    /// opens on the Conversations page instead (`WFindConversationOpen`).
    /// The answer is never announced aloud (nothing was just written); the keyboard and the screen reader go to it instead.
    func openConversation(_ id: Int64) {
        guard let store = appState?.coreStore, !busy else { return }
        loadSeq += 1
        let seq = loadSeq
        CoreMemoryStore.accessQueue.async {
            let scope = store.conversationScope(id)
            // A topic or meeting conversation opens on the Conversations page, which keeps its scope (Find asks over everything).
            guard WFindConversationOpen.destination(FindAdapter.scopeKind(scope)) == .find else {
                DispatchQueue.main.async { [weak self] in
                    guard let self, seq == self.loadSeq else { return }
                    self.appState?.openConversation(id: id)
                }
                return
            }
            let detail = store.getConversationSourced(id: id)
            let kept = store.conversationFindFilter(id)
            DispatchQueue.main.async { [weak self] in
                // A newer open, a new conversation or an ask that began meanwhile owns the page: this load is dropped.
                guard let self, seq == self.loadSeq, !self.busy, let detail, let env = self.env() else { return }
                // Asked again under the Dates and Apps filters it kept; a People filter was never kept.
                let filter = kept.map { FindAdapter.reopenedFilter($0) } ?? WHomeFilter()
                let scopeText = FindAdapter.rangeText(filter).map { self.f.fromMemoryRange($0) } ?? self.f.fromMemory
                self.epoch += 1
                self.searchSeq += 1
                self.activeConversationId = detail.id
                self.held = detail.exchanges.map { row in
                    let body: WFindExchange.Body
                    var cards: [WFindCard] = []
                    var noteLines: [String] = []
                    if row.legacy {
                        body = .legacy(text: row.answer, chips: FindAdapter.legacyChips(row.citationsJson))
                    } else {
                        body = .answered(text: row.answer)
                        cards = row.sources.sorted { $0.n < $1.n }.map { FindAdapter.numberedCard($0, env: env) }
                        noteLines = self.notes(leftOut: row.leftOut ?? 0, answeredLocally: row.answeredLocally ?? false, reopened: true)
                    }
                    // The numbers drawn as marks are the core's saved `cited` (the saved sources are exactly those).
                    // Older payloads carry none, and then the saved sources' numbers stand in.
                    let view = WFindExchange(
                        id: String(row.id), question: row.query, scope: scopeText, body: body, modelLine: row.model, notes: noteLines, sources: cards,
                        cited: row.cited ?? cards.compactMap(\.number))
                    return Held(view: view, answer: row.answer)
                }
                self.focusedId = self.held.last?.id
                self.stage = .empty
                self.results = []; self.mentions = []; self.page = CoreFindPage()
                self.route = nil
                self.routeFor = nil
                self.bridge?.setQuery("")
                self.conversationFilter = filter
                if self.bridge?.filter != filter { self.bridge?.setFilter(filter) }
                self.notice = kept?.peopleLeftOut == true ? FindAdapter.peopleFilterNotKept : nil
                self.focusToken += 1
                self.bridge?.countText = self.countText()
            }
        }
    }

    /// Saves a finished answer. A first question makes its conversation (as Today's Continue makes one: scope everything, the automatic
    /// topic, the title job) and the page takes it as active once it lands; until then a follow-up waits (`isSaving`), so a second question
    /// can never start a conversation of its own. A page that moved on meanwhile keeps its own conversation; the answer is saved all the
    /// same. Asked held to this Mac, or made here, the title stays on this Mac or is skipped.
    private func persist(_ request: AskRequest, exchange: AskExchange, requireLocal: Bool) {
        if let conversationId = request.conversationId { return appendToSaved(exchange, conversationId: conversationId) }
        guard let appState else { return }
        let capturedEpoch = request.epoch
        beginSave()
        // The filters it was asked under, kept with it: Dates and Apps only, and whether a person was left out.
        // Written with the row itself, so the conversation never exists without them.
        let kept = FindAdapter.savedFindFilter(request.filter)
        AskConversationStore.store(
            exchange, scope: .everything, appState: appState, requireLocal: requireLocal || exchange.madeOnThisComputer, findFilter: kept
        ) { [weak self] id in
            guard let self else { return }
            if let id, self.epoch == capturedEpoch { self.activeConversationId = id }
            self.endSave()
            if id != nil { self.refreshConversationList() }
        }
    }

    /// Adds a follow-up to the saved conversation it was asked in (pointers to its sources only), then lets the conversation join the topic
    /// most of its sources belong to. Until it lands, Delete is off for it.
    private func appendToSaved(_ exchange: AskExchange, conversationId: Int64) {
        guard let store = appState?.coreStore else {
            notice = TimelineView.storeUnavailableMessage
            return
        }
        beginSave()
        CoreMemoryStore.accessQueue.async {
            _ = AskConversationStore.append(exchange, to: conversationId, store: store)
            _ = store.autoTopic(conversationId: conversationId)
            DispatchQueue.main.async { [weak self] in
                self?.endSave()
                self?.refreshConversationList()
            }
        }
    }

    private func beginSave() {
        savesInFlight += 1
        isSaving = true
        syncBusy()
    }

    private func endSave() {
        savesInFlight = max(0, savesInFlight - 1)
        isSaving = savesInFlight > 0
        syncBusy()
    }

    func refreshSpendToday() {
        guard let store = appState?.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let result = store.spendToday()
            DispatchQueue.main.async { [weak self] in self?.spendTodayCents = result?.userCents }
        }
    }
}
