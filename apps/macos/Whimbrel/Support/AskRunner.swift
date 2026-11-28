import Combine
import Foundation
import WhimbrelUI

/// One finished question and answer on a redesigned Ask surface (Today's answer, a conversation page).
/// `citations` are the refs core extracted, in the answer's `[n]` order. `model` is the model that
/// answered; `answeredLocally` is true when a client excluded from hosted models kept it on this Mac.
/// `provider` is the provider id that answered when known (never a client's name), `leftOut` how many
/// retrieved items client exclusion left out of a hosted prompt, and `madeOnThisComputer` whether the answer was
/// made on this computer for any reason (saved as `answered_locally`; not the excluded-client line, which is
/// `answeredLocally`).
struct AskExchange: Equatable {
    let query: String
    var answer: String
    var citations: [CoreContextItemRef]
    var model: String?
    var answeredLocally: Bool
    var provider: String? = nil
    var leftOut: Int = 0
    var madeOnThisComputer: Bool = false
    /// What the sourced core calls said (Find's Ask): the numbered list the model saw, the numbers cited, the
    /// pointers saved with the answer and the range of days it drew on. Nil for the other calls.
    var sourced: AskSourced? = nil
}

/// A sourced answer's numbered sources, as core returned them (`AskRunner.Options.sourced`).
struct AskSourced: Equatable {
    var sources: [CoreNumberedSource]
    var cited: [Int]
    /// The cited sources' pointers (`n`, kind and id), as a conversation saves them. A meeting is among the kinds.
    var citedSources: [CoreCitedSource]
    var dateRange: CoreDateRange?
}

/// Asks one question at a time for the Ask screens. No picker: the account's routing
/// (`RoutingSettings.defaultSelection()`) and the prompt library's Ask instructions decide.
///
/// The scope decides what is asked: `.everything` prepares with `prepareAsk` (the Library's pipeline,
/// including its blocking fallback), `.topic` with `prepareAskInTopic` (nothing found in the topic is
/// `.nothingInScope`), `.meeting` with `prepareMeetingAsk` (the meeting Ask's pipeline). The answer streams
/// through `StreamingAskViewModel`; `finalizeAsk` returns its citations. A question that carries a client
/// excluded from hosted models is answered again on this Mac, as each existing surface does, and the
/// exchange says so (`answeredLocally`).
///
/// Persists nothing: the caller decides whether an exchange becomes part of a conversation. There is no
/// teardown: an owner that goes away mid-question calls `cancel()` first.
/// Main thread only.
@MainActor
final class AskRunner: ObservableObject {
    enum Phase: Equatable {
        case idle
        case streaming
        case done(AskExchange)
        case failed(String)
        /// A topic-scoped question found nothing in the topic; nothing was stashed or sent.
        case nothingInScope
    }

    /// How one question is asked. The defaults are how Today, the conversation page and a meeting ask.
    struct Options {
        /// With `sourced` (Find, whose button promised "on this Mac"): never reach a hosted model. The sourced
        /// calls take it, the stream runs only on this computer, and no hosted sign-in is refreshed. Ignored
        /// without `sourced`: no other caller asks held to this computer.
        var requireLocal = false
        /// Find: ask through the sourced core calls (`prepareAskSourced`, `askQuestionSourced`,
        /// `finalizeAskSourced`) bounded by `filter`, so the exchange carries core's numbered sources
        /// (`AskExchange.sourced`). The stream's end follows `WAskStreamEnd` (no answer again on this Mac; an
        /// empty failure goes to the blocking call, still under `requireLocal`). Everything scope only.
        var sourced = false
        var filter: CoreFindFilter?
        /// The route the caller worked out (Find's picker and its route preview); nil asks on the account's.
        /// The caller has refreshed a hosted sign-in for it already.
        var route: Route?
    }

    /// A route decided by the caller.
    struct Route {
        let config: CoreRoutingConfig
        let stream: AskStreamRoute?
        /// The model the answer is asked of (and named by).
        let model: String
        let ollamaModel: String
    }

    /// Where a question in flight is, for the waiting lines (sourced asks only fill in more than `streaming`).
    struct Progress: Equatable {
        /// The answer is streaming (the prompt is prepared and sent).
        var streaming = false
        /// Stop can end it now: the prompt is still being prepared. False on the blocking call, which cannot stop.
        var canStop = false
        /// The blocking call waits for the slow lane.
        var waitingForLane = false
        /// The blocking call found a client excluded from hosted models: it is answered here.
        var routedLocally = false
        /// The call reached its model.
        var modelConfirmed = false
    }

    @Published private(set) var phase: Phase = .idle
    /// The answer as it streams; empty until the first text arrives and after the run ends.
    @Published private(set) var partial: String = ""
    @Published private(set) var progress = Progress()
    /// Why the last question failed, when a model call said (set before `phase` turns `.failed`): a sourced ask's
    /// caller offers the AI settings for it.
    private(set) var lastError: CoreLLMError?

    let scope: CoreConversationScope

    private let appState: AppState
    private let streaming = StreamingAskViewModel()
    /// Identifies the question in flight. Cleared by `cancel` and by every ending, so a late prepare or
    /// blocking answer for an abandoned question is dropped (and its prepare released).
    private var token: UUID?
    /// The run streaming now, if any.
    private var run: Run?
    /// Held from the moment a question is in flight until it ends, so background model work waits.
    private var gateToken: UUID?
    private var subscriptions: [AnyCancellable] = []
    /// The current question's earlier exchanges: an answer kept on this Mac there marks this one too.
    private var askedHistory: [AskExchange] = []
    /// Where a sourced (Find) question is, for whether Stop may end it (`WAskStop`). The other callers' Stop is as before.
    private var stage: WAskStage = .preparing
    private var sourcedRun = false

    /// One question's settings, captured when it is asked.
    private struct Request {
        let query: String
        let history: [CoreChatTurn]
        let routing: CoreRoutingConfig
        /// The model asked for (Ollama's name when the account routes to Ollama).
        let model: String
        let ollamaModel: String
        let route: AskStreamRoute?
        var options = Options()
    }

    /// A streaming run: what the phase handler needs once it ends.
    private struct Run {
        let token: UUID
        let request: Request
        let prepareId: String
        /// The model the answer streams from (the local model on a local retry).
        let model: String
        let provider: InferenceStreamClient.Provider
        /// This run is the answer-again-on-this-Mac retry of an excluded-client refusal.
        let localRetry: Bool
        /// What client exclusion did to this run's prompt (nil when the prepare does not say).
        let exclusion: CoreExclusionOutcome?
        /// A sourced prepare's numbered sources and notes; nil for the other prepares.
        var sourced: SourcedRun?
    }

    /// What a sourced prepare said, kept until the stream ends.
    private struct SourcedRun {
        let sources: [CoreNumberedSource]
        let provider: String?
        let answeredLocally: Bool
        let dateRange: CoreDateRange?
        let leftOut: Int
    }

    init(appState: AppState, scope: CoreConversationScope) {
        self.appState = appState
        self.scope = scope
        streaming.$streamedText.sink { [weak self] text in
            guard let self, self.run != nil else { return }
            self.partial = text
        }.store(in: &subscriptions)
        // `$phase` publishes on willSet, so the new phase is the argument.
        streaming.$phase.dropFirst().sink { [weak self] in self?.handle($0) }.store(in: &subscriptions)
    }

    var isBusy: Bool { phase == .streaming }

    // MARK: Pure helpers

    /// The earlier exchanges as model history, oldest first. An exchange with no answer (it failed)
    /// adds nothing: a user turn with no assistant turn is a malformed transcript.
    nonisolated static func history(_ exchanges: [AskExchange]) -> [CoreChatTurn] {
        exchanges.flatMap { exchange -> [CoreChatTurn] in
            guard !exchange.answer.isEmpty else { return [] }
            return [CoreChatTurn(role: .user, text: exchange.query), CoreChatTurn(role: .assistant, text: exchange.answer)]
        }
    }

    /// The one line under a conversation's title that says what it asks about. Nil for a topic or meeting
    /// with no name to show (empty after trimming): the caller hides the line.
    nonisolated static func scopeLine(_ scope: CoreConversationScope, topicName: String?, meetingLabel: String?) -> String? {
        let name: String?
        switch scope {
        case .everything: return String(localized: "Asking about everything", comment: "Under a conversation's title: its questions are asked of all of memory.")
        case .topic: name = topicName
        case .meeting: name = meetingLabel
        }
        let shown = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !shown.isEmpty else { return nil }
        return String(localized: "Asking about \(shown)", comment: "Under a conversation's title: the topic or meeting its questions are asked of.")
    }

    /// The scope line of an everything conversation kept with Find filters: "Asking about Apps: Slack, Dates: Nov 9 to Nov 15", the
    /// filters in the chips' own words (`FindAdapter.filterSummary`). The same string as a topic's or meeting's line.
    nonisolated static func scopeLine(filters summary: String) -> String {
        String(localized: "Asking about \(summary)", comment: "Under a conversation's title: the topic or meeting its questions are asked of.")
    }

    /// Spec call 4: once a conversation holds an answer kept on this Mac, every follow-up says so too
    /// (its earlier exchanges count as part of the request), whatever route answered it.
    nonisolated static func answeredLocally(fresh: Bool, history: [AskExchange]) -> Bool {
        fresh || history.contains { $0.answeredLocally }
    }

    /// Whether a streamed answer was kept on this Mac: the excluded-client retry, or a prepare whose exclusion
    /// outcome says the prompt holds an excluded client's material and stays local (a default route that is
    /// already local).
    nonisolated static func answeredOnThisMac(localRetry: Bool, exclusion: CoreExclusionOutcome?) -> Bool {
        localRetry || exclusion == .localOnly
    }

    /// Whether an answer was made on this computer, as core rules it for a saved answer's `answered_locally`: kept
    /// local by client exclusion, or answered by a provider that runs here (the bundled model, or an Ollama model that
    /// is not a cloud model). An unknown provider counts as not made here.
    nonisolated static func madeOnThisComputer(provider: String?, model: String?, keptLocal: Bool) -> Bool {
        if keptLocal { return true }
        switch provider {
        case "local": return true
        case "ollama": return !isOllamaCloudModel(model ?? "")
        default: return false
        }
    }

    /// How many retrieved items client exclusion left out of a hosted prompt (0 when it says nothing).
    nonisolated static func leftOut(_ exclusion: CoreExclusionOutcome?) -> Int {
        if case .filteredHosted(let leftOut) = exclusion { return max(leftOut, 0) }
        return 0
    }

    /// A stream route that runs on this computer: the bundled model, or an Ollama model that is not a cloud one.
    nonisolated static func streamIsOnDevice(_ route: AskStreamRoute?, model: String) -> Bool {
        switch route?.provider {
        case .local: return true
        case .ollama: return !isOllamaCloudModel(model)
        default: return false
        }
    }

    /// The sourced pointers as context refs, for the callers that read `citations`. A meeting has no ref and is left
    /// out here; `AskSourced.citedSources` keeps it.
    nonisolated static func refs(_ cited: [CoreCitedSource]) -> [CoreContextItemRef] {
        cited.compactMap { c in CoreContextItemRef.Kind(rawValue: c.kind).map { CoreContextItemRef(kind: $0, id: c.id) } }
    }

    /// What an ask held to this computer says when no local model is set up to answer it (`WAskLocalFailure`). Not core's
    /// "Couldn't reach the AI provider", which would be false and invite a retry.
    nonisolated static var localOnlyFailure: String {
        String(localized: "Not sent: this question was to be answered on this Mac, and no model on this Mac could answer it. Set one up in Settings → AI Providers.",
               comment: "Shown when a question that was to stay on this Mac (the Ask button said so, or the caller requires it) could not be answered by any model on this Mac. Nothing went to a hosted model.")
    }

    /// A sourced ask's failure words: Find's (`FindAdapter.failureText`), except that a provider failure of an ask
    /// held to this computer with no local model set up says no model here could answer.
    nonisolated static func sourcedFailureText(_ error: CoreLLMError, requireLocal: Bool, localModelAvailable: Bool) -> String {
        WAskLocalFailure.saysNoLocalModel(requireLocal: requireLocal, isProviderFailure: error == .providerFailure, localModelAvailable: localModelAvailable)
            ? localOnlyFailure : FindAdapter.failureText(error)
    }

    /// A sourced call returned nothing at all (a read of the store failed part way): Find's words for it.
    static var databaseDidNotAnswer: String {
        String(localized: "Whimbrel's local database didn't answer. Try again.", comment: "Help line under the Library search failure title.")
    }

    // MARK: Asking

    /// Asks `query` with `history` (the conversation's earlier exchanges) as context. Ignored while a
    /// question is in flight or when `query` is blank.
    /// Returns whether the question started; false (nothing changed) for blank words or while one is in flight.
    @discardableResult
    func ask(_ query: String, history: [AskExchange], options: Options = Options()) -> Bool {
        guard let text = WAskStart.question(query, busy: isBusy) else { return false }
        let token = UUID()
        stage = .preparing
        sourcedRun = options.sourced
        self.token = token
        partial = ""
        progress = Progress()
        lastError = nil
        phase = .streaming
        if gateToken == nil { gateToken = UserModelGate.shared.enter() }
        let turns = Self.history(history)
        askedHistory = history
        // Whimbrel AI needs a live sync ID token: refresh it first when 5 minutes or less are left. Not for a call held
        // to this computer (it sends nothing there), nor for a caller's own route (the caller refreshed it).
        let syncState = appState.syncState
        if !options.requireLocal, options.route == nil,
           appState.routing.defaultSelection().provider == .whimbrel, syncState.whimbrelCredentialsNeedRefresh {
            Task { @MainActor [weak self] in
                await syncState.refreshWhimbrelCredentials()
                guard let self, self.token == token else { return }
                self.start(text, history: turns, token: token, options: options)
            }
            return true
        }
        start(text, history: turns, token: token, options: options)
        return true
    }

    /// Stop. A streaming answer is cancelled (its prepare released); a question still being prepared or
    /// answered by the blocking path is dropped when it returns. Back to `.idle`.
    func cancel() {
        guard isBusy else { return }
        // A finished Find answer being finalized, or its blocking call, is never dropped by a late Stop.
        if sourcedRun, !WAskStop.mayStop(stage) { return }
        if run != nil, streaming.isCancellable {
            streaming.cancel()
            // While the local model warms up there is no run to cancel yet, so the phase stays put.
            if streaming.phase == .cancelling { return }
        }
        if let run { discard(run.prepareId) }
        end(.idle)
    }

    private func start(_ query: String, history: [CoreChatTurn], token: UUID, options: Options) {
        // The hotkey panel's route (the default provider; the local model in local-only mode), so the
        // privacy-relevant routing lives in one place. A caller's own route (Find's picker) replaces it, and the
        // account's is then not worked out at all (that reads the Keychain on this thread).
        let window = options.route == nil ? appState.windowAskRouting() : nil
        let defaults = appState.routing.defaultSelection()
        let routing = options.route?.config ?? window?.config ?? appState.routing.routingConfig(
            preferredProvider: defaults.provider, preferredModel: defaults.provider == .ollama ? defaults.ollamaModel : defaults.hostedModel)
        let route = options.route.map(\.stream) ?? window?.stream
        let model = options.route?.model ?? routing.preferredModel
        let ollamaModel = options.route?.ollamaModel ?? defaults.ollamaModel
        let request = Request(query: query, history: history, routing: routing, model: model,
                              ollamaModel: ollamaModel, route: route, options: options)
        if options.sourced { return startSourced(request, token: token) }
        guard let store = appState.coreStore else {
            return end(.failed(CoreLLMError.providerFailure.userFacingMessage))
        }
        guard let route else {
            switch scope {
            // The Library's rule: no stream route answers through the governed chain.
            case .everything: return answerBlocking(request, token: token)
            case .topic, .meeting: return end(.failed(noRouteMessage(provider: defaults.provider)))
            }
        }
        switch scope {
        case .everything:
            CoreMemoryStore.accessQueue.async { [weak self] in
                let prepared = store.prepareAsk(query, contextLimit: 5, history: history, routing: routing)
                DispatchQueue.main.async { [weak self] in
                    let result = prepared.map { $0.map(Prepared.init) }
                    guard let self else { return Self.release(result, store: store) }
                    self.prepared(result, request: request, route: route, token: token, localRetry: false)
                }
            }
        case .topic(let topicId):
            CoreMemoryStore.accessQueue.async { [weak self] in
                let prepared = store.prepareAskInTopic(topicId: topicId, query: query, contextLimit: 5, history: history, routing: routing)
                DispatchQueue.main.async { [weak self] in
                    let result = prepared.map { $0.map(Prepared.init) }
                    guard let self else { return Self.release(result, store: store) }
                    self.prepared(result, request: request, route: route, token: token, localRetry: false)
                }
            }
        case .meeting(let meetingId):
            CoreMemoryStore.accessQueue.async { [weak self] in
                let prepared = store.prepareMeetingAsk(meetingId: meetingId, query: query, history: history, routing: routing)
                DispatchQueue.main.async { [weak self] in
                    let result = prepared.map { $0.map(Prepared.init) }
                    guard let self else { return Self.release(result, store: store) }
                    self.prepared(result, request: request, route: route, token: token, localRetry: false)
                }
            }
        }
    }

    /// What every scope's prepare gives back, reduced to what the stream needs.
    private struct Prepared {
        let prepareId: String
        let prompt: String
        var scopeEmpty = false
        /// What client exclusion did (nil for a meeting prepare, which does not say).
        var exclusion: CoreExclusionOutcome?

        init(_ data: CorePrepareAskData) {
            prepareId = data.prepareId
            prompt = data.prompt
            scopeEmpty = data.scopeEmpty
            exclusion = data.exclusion
        }

        init(_ data: CorePrepareMeetingAskData) {
            prepareId = data.prepareId
            prompt = data.prompt
        }
    }

    private func prepared(
        _ result: Result<Prepared, CoreLLMError>?, request: Request, route: AskStreamRoute, token: UUID, localRetry: Bool
    ) {
        guard self.token == token else {
            // Stopped while preparing: never stream, release the prepare.
            if case .some(.success(let data)) = result, !data.prepareId.isEmpty { discard(data.prepareId) }
            return
        }
        switch result {
        case .none:
            end(.failed(CoreLLMError.providerFailure.userFacingMessage))
        case .some(.failure(let error)):
            if error == .providerFailureBecause(.excludedClient), !localRetry {
                answerLocally(request, token: token)
            } else if error.fallsBackToGovernedAsk(alreadyFellBack: false) {
                // The streamed provider's budget is spent: the governed chain skips it.
                answerGoverned(request, token: token, failure: error)
            } else {
                end(.failed(error.userFacingMessage))
            }
        case .some(.success(let data)):
            if data.scopeEmpty {
                // Nothing in the topic matched: nothing was stashed or sent.
                return end(.nothingInScope)
            }
            run = Run(token: token, request: request, prepareId: data.prepareId, model: request.model,
                      provider: route.provider, localRetry: localRetry, exclusion: data.exclusion)
            streaming.start(prompt: data.prompt, model: request.model, provider: route.provider,
                            apiKey: route.apiKey, baseURLOverride: route.baseURLOverride)
        }
    }

    // MARK: The stream's end

    /// Called as `streaming.phase` is set (willSet), so the new phase is the argument.
    private func handle(_ phase: StreamingAskViewModel.Phase) {
        guard let run, run.token == token else { return }
        if run.sourced != nil { return handleSourced(phase, run: run) }
        switch phase {
        case .completed:
            self.run = nil
            finalize(run, text: streaming.streamedText)
        case .cancelled:
            self.run = nil
            discard(run.prepareId)
            end(.idle)
        case .failed(let message):
            self.run = nil
            discard(run.prepareId)
            let noOutput = streaming.streamedText.isEmpty
            if run.provider == .whimbrel, message.hasPrefix("HTTP 402") {
                // The Whimbrel AI credit is used up: the chain would only meet the same 402.
                end(.failed(CoreLLMError.providerFailureBecause(.creditExhausted).userFacingMessage))
            } else if streaming.exclusionRefusal == .unreadable {
                // Nothing was sent, and nothing hosted goes until the setting reads.
                end(.failed(CoreLLMError.providerFailureBecause(.exclusionUnreadable).userFacingMessage))
            } else if streaming.exclusionRefusal == .excludedClient, noOutput, !run.localRetry {
                // Held back by the exclusion gate, nothing sent: answer it on this Mac instead.
                partial = ""
                answerLocally(run.request, token: run.token)
            } else if streaming.exclusionRefusal == .excludedClient {
                end(.failed(CoreLLMError.providerFailureBecause(.excludedClient).userFacingMessage))
            } else if noOutput, scope == .everything, !run.localRetry {
                // Never emitted anything (e.g. Ollama not running): the Library retries through the
                // blocking path, whose chain falls back.
                answerBlocking(run.request, token: run.token)
            } else {
                // Output was already shown: never fall back silently once that is true.
                end(.failed(CoreLLMError.providerFailure.userFacingMessage))
            }
        default:
            break
        }
    }

    /// Citations for a completed stream, logged under the model that answered.
    private func finalize(_ run: Run, text: String) {
        guard let store = appState.coreStore else {
            return end(.done(AskExchange(query: run.request.query, answer: text, citations: [], model: run.model,
                                         answeredLocally: Self.answeredOnThisMac(localRetry: run.localRetry, exclusion: run.exclusion),
                                         provider: run.provider.rawValue, leftOut: Self.leftOut(run.exclusion),
                                         madeOnThisComputer: Self.madeOnThisComputer(
                                            provider: run.provider.rawValue, model: run.model,
                                            keptLocal: Self.answeredOnThisMac(localRetry: run.localRetry, exclusion: run.exclusion)))))
        }
        let usage = streaming.usage
        // Hosted providers only: the local helper's stream names "local", not its model.
        let hosted = HostedModelDefaults.hostedProviders.contains(run.provider.rawValue)
        let answered = hosted ? (streaming.answeredModel ?? run.model) : run.model
        CoreMemoryStore.accessQueue.async { [weak self] in
            if hosted {
                store.noteModelAnswer(provider: run.provider.rawValue, requested: run.model, answered: answered)
            }
            let citations = store.finalizeAsk(
                prepareId: run.prepareId, finalText: text, model: answered,
                inputTokens: usage.inputTokens ?? -1, outputTokens: usage.outputTokens ?? -1) ?? []
            DispatchQueue.main.async { [weak self] in
                guard let self, self.token == run.token else { return }
                self.end(.done(AskExchange(query: run.request.query, answer: text, citations: citations, model: answered,
                                           answeredLocally: Self.answeredOnThisMac(localRetry: run.localRetry, exclusion: run.exclusion),
                                           provider: run.provider.rawValue, leftOut: Self.leftOut(run.exclusion),
                                           madeOnThisComputer: Self.madeOnThisComputer(
                                              provider: run.provider.rawValue, model: answered,
                                              keptLocal: Self.answeredOnThisMac(localRetry: run.localRetry, exclusion: run.exclusion)))))
            }
        }
    }

    // MARK: Answering again

    /// A question carrying a client excluded from hosted models, refused for the hosted stream. Each scope
    /// answers it on this Mac the way its existing surface does: everything through the Library's blocking
    /// path (core keeps it local), a meeting through the meeting Ask's blocking answer, and a topic (which has
    /// no blocking answer) by preparing again for a model on this Mac and streaming there.
    private func answerLocally(_ request: Request, token: UUID) {
        switch scope {
        case .everything:
            answerBlocking(request, token: token)
        case .meeting:
            answerGoverned(request, token: token, failure: .providerFailureBecause(.excludedClient), excludedRetry: true)
        case .topic(let topicId):
            guard let local = appState.windowAskRouting().local, let store = appState.coreStore else {
                return end(.failed(CoreLLMError.providerFailureBecause(.excludedClient).userFacingMessage))
            }
            let localRequest = Request(query: request.query, history: request.history, routing: local.config,
                                       model: local.config.preferredModel, ollamaModel: request.ollamaModel, route: local.stream,
                                       options: request.options)
            CoreMemoryStore.accessQueue.async { [weak self] in
                let prepared = store.prepareAskInTopic(
                    topicId: topicId, query: request.query, contextLimit: 5, history: request.history, routing: local.config)
                DispatchQueue.main.async { [weak self] in
                    let result = prepared.map { $0.map(Prepared.init) }
                    guard let self else { return Self.release(result, store: store) }
                    self.prepared(result, request: localRequest, route: local.stream,
                                   token: token, localRetry: true)
                }
            }
        }
    }

    /// The streamed provider can't answer (budget spent, or an excluded client): one answer through the
    /// governed chain. Everything uses the Library's blocking path; a meeting uses the meeting Ask's; a topic
    /// has no governed answer, so it shows `failure`.
    private func answerGoverned(_ request: Request, token: UUID, failure: CoreLLMError, excludedRetry: Bool = false) {
        switch scope {
        case .everything:
            answerBlocking(request, token: token)
        case .topic:
            end(.failed(failure.userFacingMessage))
        case .meeting(let meetingId):
            guard let slowStore = appState.slowStore else { return end(.failed(failure.userFacingMessage)) }
            UserModelGate.runOnSlowLane { [weak self] in
                let answered = slowStore.answerMeetingQuestion(
                    meetingId: meetingId, query: request.query, model: request.ollamaModel,
                    history: request.history, routing: request.routing)
                DispatchQueue.main.async { [weak self] in
                    guard let self, self.token == token else { return }
                    switch answered {
                    case .none:
                        self.end(.failed(CoreLLMError.providerFailure.userFacingMessage))
                    case .some(.failure(let error)):
                        self.end(.failed(error.userFacingMessage))
                    case .some(.success(let answer)):
                        self.end(.done(AskExchange(query: request.query, answer: answer.text, citations: answer.citations,
                                                   model: nil, answeredLocally: excludedRetry, madeOnThisComputer: excludedRetry)))
                    }
                }
            }
        }
    }

    /// The Library's blocking ask (`TimelineView.askBlocking`): on the slow lane, the excluded-client check
    /// first, then `askQuestionDetailed`, which keeps an excluded client's question on this Mac.
    private func answerBlocking(_ request: Request, token: UUID) {
        guard let store = appState.slowStore else {
            return end(.failed(CoreLLMError.providerFailure.userFacingMessage))
        }
        CoreMemoryStore.slowLaneQueue.async { [weak self] in
            let namesExcluded = store.askNamesExcludedClient(request.query, history: request.history)
            let detailed = store.askQuestionDetailed(
                request.query, model: request.ollamaModel, contextLimit: 5, history: request.history, routing: request.routing)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.token == token else { return }
                switch detailed {
                case .none:
                    self.end(.failed(CoreLLMError.providerFailure.userFacingMessage))
                case .some(.failure(let error)):
                    self.end(.failed(error.userFacingMessage))
                case .some(.success(let answer)):
                    self.end(.done(AskExchange(
                        query: request.query, answer: answer.text, citations: answer.citations,
                        model: answeredModelLabel(requested: request.model, answeredBy: answer.answeredBy),
                        answeredLocally: answeredLocallyForExclusion(namesExcludedClient: namesExcluded, outcome: answer.exclusion),
                        provider: answer.answeredBy?.provider, leftOut: Self.leftOut(answer.exclusion),
                        madeOnThisComputer: Self.madeOnThisComputer(
                            provider: answer.answeredBy?.provider, model: answer.answeredBy?.model,
                            keptLocal: answer.exclusion == .localOnly || answer.answeredBy?.local == true))))
                }
            }
        }
    }

    // MARK: Sourced (Find)

    /// Find's Ask: `prepareAskSourced` and a stream when `WAskStreamEnd.mayStream` allows it, else the blocking
    /// `askQuestionSourced`. An excluded client's question, and a spent streamed budget, go to the blocking call,
    /// which keeps an excluded client's question on this Mac or refuses it.
    private func startSourced(_ request: Request, token: UUID) {
        guard let store = appState.coreStore else { return end(.failed(AskShared.storeUnavailableMessage)) }
        let requireLocal = request.options.requireLocal
        let streams = WAskStreamEnd.mayStream(
            promisesLocal: requireLocal, streamIsOnDevice: Self.streamIsOnDevice(request.route, model: request.model),
            hasStreamRoute: request.route != nil)
        guard streams, let route = request.route else { return answerSourcedBlocking(request, token: token) }
        progress.canStop = true
        let filter = request.options.filter
        CoreMemoryStore.accessQueue.async { [weak self] in
            let prepared = store.prepareAskSourced(
                request.query, contextLimit: 5, history: request.history, routing: request.routing, filter: filter, requireLocal: requireLocal)
            DispatchQueue.main.async { [weak self] in
                // Whatever happened meanwhile, a prepared ask is finalized or discarded: never left behind.
                guard let self, self.token == token else {
                    if case .success(let data)? = prepared { CoreMemoryStore.accessQueue.async { store.discardAsk(prepareId: data.prepareId) } }
                    return
                }
                guard let prepared else { return self.end(.failed(Self.databaseDidNotAnswer)) }
                switch prepared {
                case .failure(.providerFailureBecause(.excludedClient)):
                    self.answerSourcedBlocking(request, token: token)
                case .failure(let error) where error.fallsBackToGovernedAsk(alreadyFellBack: false):
                    self.answerSourcedBlocking(request, token: token)
                case .failure(let error):
                    self.failSourced(error, requireLocal: requireLocal)
                case .success(let data):
                    self.progress = Progress(streaming: true, canStop: false, modelConfirmed: true)
                    self.stage = .streaming
                    self.run = Run(
                        token: token, request: request, prepareId: data.prepareId, model: request.model, provider: route.provider,
                        localRetry: false, exclusion: data.exclusion,
                        sourced: SourcedRun(sources: data.sources, provider: data.provider, answeredLocally: data.answeredLocally,
                                            dateRange: data.dateRange, leftOut: data.exclusion?.leftOut ?? 0))
                    self.streaming.start(prompt: data.prompt, model: request.model, provider: route.provider,
                                         apiKey: route.apiKey, baseURLOverride: route.baseURLOverride)
                }
            }
        }
    }

    /// A sourced stream's end, by `WAskStreamEnd`'s table.
    private func handleSourced(_ phase: StreamingAskViewModel.Phase, run: Run) {
        let kind: WAskStreamEnd.Phase
        var is402 = false
        switch phase {
        case .completed: kind = .completed
        case .cancelled: kind = .cancelled
        case .failed(let message):
            kind = .failed
            is402 = run.provider == .whimbrel && message.hasPrefix("HTTP 402")
        default: return
        }
        self.run = nil
        let decision = WAskStreamEnd.decide(
            phase: kind, hadOutput: !streaming.streamedText.isEmpty, isWhimbrelAI402: is402,
            exclusionUnreadable: streaming.exclusionRefusal == .unreadable, requireLocal: run.request.options.requireLocal)
        switch decision {
        case .finalize:
            // Answered: from here Stop is off and the answer is finished and kept.
            stage = .finalizing
            progress.streaming = false
            progress.canStop = false
            finalizeSourced(run, text: streaming.streamedText)
        case .discardAndRestoreQuestion:
            discard(run.prepareId)
            end(.idle)
        case .failWith(let failure):
            discard(run.prepareId)
            switch failure {
            case .creditExhausted: failSourced(.providerFailureBecause(.creditExhausted), requireLocal: run.request.options.requireLocal)
            case .exclusionUnreadable: failSourced(.providerFailureBecause(.exclusionUnreadable), requireLocal: run.request.options.requireLocal)
            case .providerFailure: failSourced(.providerFailure, requireLocal: run.request.options.requireLocal)
            }
        case .fallBackToBlocking:
            // Nothing was emitted: the blocking call, under the same `requireLocal`.
            discard(run.prepareId)
            partial = ""
            answerSourcedBlocking(run.request, token: run.token)
        }
    }

    private func finalizeSourced(_ run: Run, text: String) {
        guard let store = appState.coreStore, let sourced = run.sourced else {
            return end(.failed(AskShared.storeUnavailableMessage))
        }
        let usage = streaming.usage
        let hosted = HostedModelDefaults.hostedProviders.contains(run.provider.rawValue)
        let answered = hosted ? (streaming.answeredModel ?? run.model) : run.model
        CoreMemoryStore.accessQueue.async { [weak self] in
            if hosted { store.noteModelAnswer(provider: run.provider.rawValue, requested: run.model, answered: answered) }
            let done = store.finalizeAskSourced(
                prepareId: run.prepareId, finalText: text, model: answered,
                inputTokens: usage.inputTokens ?? -1, outputTokens: usage.outputTokens ?? -1)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.token == run.token else { return }
                let cited = done?.citedSources ?? []
                self.end(.done(AskExchange(
                    query: run.request.query, answer: text, citations: Self.refs(cited), model: answered,
                    answeredLocally: Self.answeredOnThisMac(localRetry: false, exclusion: run.exclusion),
                    provider: sourced.provider, leftOut: done?.leftOut ?? sourced.leftOut,
                    madeOnThisComputer: sourced.answeredLocally,
                    sourced: AskSourced(sources: sourced.sources, cited: done?.cited ?? [], citedSources: cited, dateRange: sourced.dateRange))))
            }
        }
    }

    /// The blocking sourced call on the slow lane, with the waiting lines: the excluded-client check first, then
    /// `askQuestionSourced`. It cannot be stopped.
    private func answerSourcedBlocking(_ request: Request, token: UUID) {
        guard let store = appState.slowStore else { return end(.failed(AskShared.storeUnavailableMessage)) }
        stage = .blocking
        progress.streaming = false
        progress.canStop = false
        let laneStart = LaneStartFlag()
        DispatchQueue.main.asyncAfter(deadline: .now() + AskLocalRouteCopy.waitingShownAfter) { [weak self] in
            guard let self, self.token == token, !laneStart.isSet else { return }
            self.progress.waitingForLane = true
        }
        let filter = request.options.filter
        let requireLocal = request.options.requireLocal
        CoreMemoryStore.slowLaneQueue.async { [weak self] in
            laneStart.set()
            DispatchQueue.main.async { [weak self] in
                guard let self, self.token == token else { return }
                self.progress.waitingForLane = false
            }
            let namesExcluded = store.askNamesExcludedClient(request.query, history: request.history)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.token == token else { return }
                if namesExcluded { self.progress.routedLocally = true } else { self.progress.modelConfirmed = true }
            }
            let outcome = store.askQuestionSourced(
                request.query, model: request.ollamaModel, contextLimit: 5, history: request.history, routing: request.routing,
                filter: filter, requireLocal: requireLocal)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.token == token else { return }
                // A bare NULL from the core: the call could not be made, or a read of the store failed part way.
                guard let outcome else { return self.end(.failed(Self.databaseDidNotAnswer)) }
                switch outcome {
                case .failure(let error):
                    self.failSourced(error, requireLocal: requireLocal)
                case .success(let answer):
                    self.end(.done(AskExchange(
                        query: request.query, answer: answer.text, citations: Self.refs(answer.citedSources),
                        model: answeredModelLabel(requested: request.model, answeredBy: answer.answeredBy),
                        answeredLocally: answeredLocallyForExclusion(namesExcludedClient: namesExcluded, outcome: answer.exclusion),
                        provider: answer.answeredBy?.provider, leftOut: answer.exclusion?.leftOut ?? 0,
                        madeOnThisComputer: answer.answeredLocally,
                        sourced: AskSourced(sources: answer.sources, cited: answer.cited, citedSources: answer.citedSources,
                                            dateRange: answer.dateRange))))
                }
            }
        }
    }

    private func failSourced(_ error: CoreLLMError, requireLocal: Bool) {
        lastError = error
        // A model here that failed keeps the failure's words; only none set up says no model here could answer.
        let localModelAvailable = !appState.routing.ollamaModels.filter { !isOllamaCloudModel($0) }.isEmpty || LocalModel.isInstalled
        end(.failed(Self.sourcedFailureText(error, requireLocal: requireLocal, localModelAvailable: localModelAvailable)))
    }

    // MARK: Endings

    private func end(_ phase: Phase) {
        var phase = phase
        if case .done(var exchange) = phase {
            exchange.answeredLocally = Self.answeredLocally(fresh: exchange.answeredLocally, history: askedHistory)
            phase = .done(exchange)
        }
        askedHistory = []
        token = nil
        run = nil
        partial = ""
        progress = Progress()
        UserModelGate.shared.leave(gateToken)
        gateToken = nil
        self.phase = phase
    }

    /// The runner went away while preparing: release a stashed prepare nobody will finish.
    nonisolated private static func release(_ result: Result<Prepared, CoreLLMError>?, store: CoreMemoryStore) {
        guard case .some(.success(let data)) = result, !data.prepareId.isEmpty else { return }
        CoreMemoryStore.accessQueue.async { store.discardAsk(prepareId: data.prepareId) }
    }

    private func discard(_ prepareId: String) {
        let store = appState.coreStore
        CoreMemoryStore.accessQueue.async { store?.discardAsk(prepareId: prepareId) }
    }

    /// Why a topic or meeting question can't stream (the meeting Ask's words).
    private func noRouteMessage(provider: RoutingSettings.Provider) -> String {
        appState.routing.localOnlyMode
            ? String(localized: "Local-only mode is on, so Ask can't use a hosted model. Choose a local model in Settings.")
            : provider == .whimbrel
            ? String(localized: "Sign in to sync to use Whimbrel AI")
            : String(localized: "No API key is set for the selected provider. Choose a local model or add a key in Settings.")
    }
}

/// A conversation's placeholder title replaced by a generated one, in the background. One job for every Ask surface
/// (the Library's Ask and Today's Continue as a conversation).
enum ConversationTitleJob {
    /// The route a title is asked on when no picker was touched: the non-time-critical route (the sentinel query), so
    /// with a hosted default the title goes to the local model and never spends the person's hosted budget.
    static func defaultRoute(
        _ defaults: (provider: RoutingSettings.Provider, ollamaModel: String, hostedModel: String)
    ) -> (provider: RoutingSettings.Provider, modelLabel: String) {
        effectiveAskRoutingProvider(
            query: AskShared.nonTimeCriticalSentinelQuery, pickerProvider: defaults.provider,
            pickerModelLabel: defaults.provider == .ollama ? defaults.ollamaModel : defaults.hostedModel,
            pickerTouched: false, defaults: defaults)
    }

    /// How a new conversation's title is routed, decided before any credential is read.
    enum Plan {
        /// No title call: the placeholder stays.
        case skip
        /// Kept on this computer: built with no hosted key and no Whimbrel sign-in.
        case ready(CoreRoutingConfig)
        /// The account's own title route: its keys are read off the main thread (`schedule(plan:)`).
        case hosted(provider: RoutingSettings.Provider, model: String)
    }

    /// The title plan. `keepLocal` (the conversation was asked with `requireLocal`, made on this computer, or kept
    /// local for an excluded client) keeps it to a model on this computer and skips it when there is none
    /// (`WAskTitleRoute`); neither reads a credential. Otherwise the default route, as before. Reads no Keychain.
    @MainActor
    static func plan(keepLocal: Bool, appState: AppState) -> Plan {
        let route = defaultRoute(appState.routing.defaultSelection())
        // An Ollama cloud model runs on Ollama's servers: never a model on this computer.
        let localModelAvailable = keepLocal
            && (!appState.routing.ollamaModels.filter { !isOllamaCloudModel($0) }.isEmpty || LocalModel.isInstalled)
        let decided = WAskTitleRoute.decide(keepLocal: keepLocal, localModelAvailable: localModelAvailable)
        guard decided.readsCredentials else {
            guard decided == .localOnly else { return .skip }
            return .ready(appState.routing.routingConfig(
                preferredProvider: route.provider, preferredModel: route.modelLabel,
                keys: RoutingSettings.ProviderKeys(), whimbrelCredentials: false).localOnly())
        }
        return .hosted(provider: route.provider, model: route.modelLabel)
    }

    /// Starts the title job for `plan`. Call off the main thread: a hosted plan's keys are read here, its config is
    /// built on the main thread, then the job runs on the lane.
    nonisolated static func schedule(
        plan: Plan, routing settings: RoutingSettings, lane: (store: CoreMemoryStore, queue: DispatchQueue), conversationId: Int64,
        query: String, answer: String, ollamaModel: String, completion: @escaping () -> Void = {}
    ) {
        switch plan {
        case .skip:
            return
        case .ready(let routing):
            schedule(lane: lane, conversationId: conversationId, query: query, answer: answer, ollamaModel: ollamaModel,
                     routing: routing, completion: completion)
        case .hosted(let provider, let model):
            let keys = RoutingSettings.ProviderKeys.load()
            DispatchQueue.main.async {
                let routing = settings.routingConfig(preferredProvider: provider, preferredModel: model, keys: keys)
                schedule(lane: lane, conversationId: conversationId, query: query, answer: answer, ollamaModel: ollamaModel,
                         routing: routing, completion: completion)
            }
        }
    }

    /// Runs the title call on the background model lane, but only while no question of the person's is queued:
    /// otherwise tries again later (`attemptsLeft` more times; the placeholder stays if it never runs). On any failure
    /// the placeholder title stays, with no retry and nothing shown. `completion` runs on the main thread once a
    /// generated title was offered to the row (it replaces only a placeholder).
    nonisolated static func schedule(
        lane: (store: CoreMemoryStore, queue: DispatchQueue), conversationId: Int64, query: String, answer: String,
        ollamaModel: String, routing: CoreRoutingConfig, attemptsLeft: Int = 8, completion: @escaping () -> Void = {}
    ) {
        lane.queue.async {
            if case .retryLater(let seconds) = BackgroundModelLane.start(userBusy: UserModelGate.shared.isBusy()) {
                guard attemptsLeft > 0 else { return }
                DispatchQueue.main.asyncAfter(deadline: .now() + seconds) {
                    schedule(lane: lane, conversationId: conversationId, query: query, answer: answer, ollamaModel: ollamaModel,
                             routing: routing, attemptsLeft: attemptsLeft - 1, completion: completion)
                }
                return
            }
            guard let title = lane.store.generateConversationTitle(model: ollamaModel, query: query, answer: answer, routing: routing)
            else { return }
            _ = lane.store.setConversationTitleIfPlaceholder(id: conversationId, generatedTitle: title)
            DispatchQueue.main.async { completion() }
        }
    }
}
