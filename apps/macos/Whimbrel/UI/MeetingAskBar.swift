import AppKit
import Combine
import SwiftUI

/// One question and its answer in the per-visit Ask panel.
struct MeetingAskExchange: Identifiable {
    let id = UUID()
    let question: String
    /// What the model was asked, when it isn't `question` (a preset shows its label).
    var prompt: String?
    var answer: String = ""
    var citedSegmentIds: [Int64] = []
    var narrowed = false
    var errorText: String?
    /// The error's fix is in Settings → AI Providers (a spent monthly budget).
    var opensAIProvidersSettings = false
    /// Output received before a provider failure. Shown above the error, never part of history.
    var partial: String = ""
}

/// A meeting Ask's exchanges and its in-flight run. An object, not `MeetingAskBar` state, so the
/// owner decides how long it lives: the live meeting's (`LiveTranscriptionService.askSession`)
/// outlives `ThemedRoot` rebuilding the window on a skin switch, so a streaming answer keeps
/// streaming into the rebuilt bar. Phase handling is here, not in the view, so a run that ends
/// while no bar is on screen still finalizes. Main thread only.
@MainActor
final class MeetingAskSession: ObservableObject {
    let streaming = StreamingAskViewModel()
    @Published var exchanges: [MeetingAskExchange] = []
    @Published var pending: (prepareId: String, index: Int, model: String, provider: InferenceStreamClient.Provider)?
    @Published var isPreparing = false
    /// The Whimbrel AI token is being refreshed before a send; busy, so a second send is ignored.
    @Published var isRefreshingCredentials = false
    /// Identifies the in-flight prepare. Cleared by Stop or teardown, so a late prepare
    /// completion discards its result instead of streaming the transcript.
    private var prepareToken: UUID?
    private var store: CoreMemoryStore?
    /// The provider the current answer streams from; a Whimbrel AI 402 shows the credit message.
    private var streamProvider: InferenceStreamClient.Provider = .ollama
    private var subscriptions: [AnyCancellable] = []

    var isBusy: Bool { isPreparing || isRefreshingCredentials || pending != nil }

    init() {
        // Republish the stream's text so the bar redraws, and handle each phase as it is set.
        streaming.objectWillChange.sink { [weak self] in self?.objectWillChange.send() }.store(in: &subscriptions)
        streaming.$phase.dropFirst().sink { [weak self] in self?.handle($0) }.store(in: &subscriptions)
    }

    private func history() -> [CoreChatTurn] {
        exchanges.flatMap { e -> [CoreChatTurn] in
            guard e.errorText == nil, !e.answer.isEmpty else { return [] }
            return [CoreChatTurn(role: .user, text: e.prompt ?? e.question), CoreChatTurn(role: .assistant, text: e.answer)]
        }
    }

    /// Removes a failed exchange and returns its question to ask again.
    func takeForRetry(_ index: Int) -> String? {
        guard !isBusy, exchanges.indices.contains(index) else { return nil }
        return exchanges.remove(at: index).question
    }

    /// `route` nil means no provider can stream; the exchange shows `noRouteMessage`.
    /// `fallback` answers through the governed chain when the streamed provider's budget is spent.
    func submit(
        _ text: String, prompt: String? = nil, meetingId: Int64, store: CoreMemoryStore, routing: CoreRoutingConfig, model: String,
        route: (provider: InferenceStreamClient.Provider, apiKey: String?, baseURLOverride: String?)?, noRouteMessage: String,
        fallback: (slowStore: CoreMemoryStore, ollamaModel: String)? = nil
    ) {
        guard !isBusy else { return }
        self.store = store
        let past = history()
        exchanges.append(MeetingAskExchange(question: text, prompt: prompt))
        let index = exchanges.count - 1
        guard let route else {
            exchanges[index].errorText = noRouteMessage
            return
        }
        isPreparing = true
        let token = UUID()
        prepareToken = token
        CoreMemoryStore.accessQueue.async {
            let prepared = store.prepareMeetingAsk(meetingId: meetingId, query: prompt ?? text, history: past, routing: routing)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.prepareToken == token else {
                    // Stopped (or torn down) while preparing: never stream, release the prepare.
                    if case .some(.success(let data)) = prepared { Self.discard(data.prepareId, store: store) }
                    return
                }
                self.isPreparing = false
                self.prepareToken = nil
                switch prepared {
                case .none:
                    self.exchanges[index].errorText = String(localized: "Couldn't prepare the question. Try again.")
                case .some(.failure(let error)):
                    if let fallback, error.fallsBackToGovernedAsk(alreadyFellBack: false) {
                        self.answerGoverned(index: index, meetingId: meetingId, query: prompt ?? text, history: past, routing: routing, fallback: fallback)
                    } else {
                        self.show(error, at: index)
                    }
                case .some(.success(let data)):
                    self.exchanges[index].narrowed = data.narrowed
                    self.pending = (data.prepareId, index, model, route.provider)
                    self.streamProvider = route.provider
                    self.streaming.start(prompt: data.prompt, model: model, provider: route.provider, apiKey: route.apiKey, baseURLOverride: route.baseURLOverride)
                }
            }
        }
    }

    private func show(_ error: CoreLLMError, at index: Int) {
        exchanges[index].errorText = error.userFacingMessage
        exchanges[index].opensAIProvidersSettings = error.opensAIProvidersSettings
    }

    /// The streamed provider's budget is spent: answer once through the governed chain, which
    /// skips it. Busy as while preparing, so Stop and teardown make a late answer inert.
    private func answerGoverned(
        index: Int, meetingId: Int64, query: String, history: [CoreChatTurn], routing: CoreRoutingConfig,
        fallback: (slowStore: CoreMemoryStore, ollamaModel: String)
    ) {
        isPreparing = true
        let token = UUID()
        prepareToken = token
        CoreMemoryStore.slowLaneQueue.async {
            let answered = fallback.slowStore.answerMeetingQuestion(
                meetingId: meetingId, query: query, model: fallback.ollamaModel, history: history, routing: routing)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.prepareToken == token, self.exchanges.indices.contains(index) else { return }
                self.isPreparing = false
                self.prepareToken = nil
                switch answered {
                case .none:
                    self.exchanges[index].errorText = String(localized: "Couldn't prepare the question. Try again.")
                case .some(.failure(let error)):
                    self.show(error, at: index)
                case .some(.success(let answer)):
                    self.exchanges[index].narrowed = answer.narrowed
                    self.exchanges[index].answer = answer.text
                    self.exchanges[index].citedSegmentIds = TranscriptCitations.segmentIds(from: answer.citations)
                }
            }
        }
    }

    nonisolated private static func discard(_ prepareId: String, store: CoreMemoryStore?) {
        CoreMemoryStore.accessQueue.async { store?.discardAsk(prepareId: prepareId) }
    }

    /// Stop. Returns the question to put back in the field when nothing was streaming yet.
    func cancel() -> String? {
        if isPreparing, prepareToken != nil {
            // Nothing is streaming yet: invalidate the prepare and put the question back.
            prepareToken = nil
            isPreparing = false
            if let last = exchanges.last, last.errorText == nil {
                exchanges.removeLast()
                return last.question
            }
        } else {
            streaming.cancel()
        }
        return nil
    }

    /// The owner is done (page closed, live meeting ended): stop the stream, release the
    /// prepare, make a late prepare completion inert, and forget the exchanges.
    func teardown() {
        prepareToken = nil
        isPreparing = false
        streaming.cancel()
        if let pending {
            Self.discard(pending.prepareId, store: store)
            self.pending = nil
        }
        exchanges = []
    }

    /// Called as `phase` is set (willSet), so the new phase is the argument, not `streaming.phase`.
    func handle(_ phase: StreamingAskViewModel.Phase) {
        guard let pending else { return }
        let store = store
        switch phase {
        case .completed:
            let text = streaming.streamedText
            exchanges[pending.index].answer = text
            let usage = streaming.usage
            let index = pending.index
            // Logged under the model that answered; core notes a substitution for Settings.
            // Hosted providers only: the local helper's stream names "local", not its model.
            let hosted = HostedModelDefaults.hostedProviders.contains(pending.provider.rawValue)
            let answered = hosted ? (streaming.answeredModel ?? pending.model) : pending.model
            self.pending = nil
            CoreMemoryStore.accessQueue.async {
                if hosted {
                    store?.noteModelAnswer(provider: pending.provider.rawValue, requested: pending.model, answered: answered)
                }
                let refs = store?.finalizeAsk(
                    prepareId: pending.prepareId, finalText: text, model: answered,
                    inputTokens: Int64(usage.inputTokens ?? -1), outputTokens: Int64(usage.outputTokens ?? -1)) ?? []
                DispatchQueue.main.async { [weak self] in
                    guard let self, self.exchanges.indices.contains(index) else { return }
                    self.exchanges[index].citedSegmentIds = TranscriptCitations.segmentIds(from: refs)
                }
            }
        case .cancelled:
            Self.discard(pending.prepareId, store: store)
            exchanges.remove(at: pending.index)
            self.pending = nil
        case .failed(let message):
            Self.discard(pending.prepareId, store: store)
            exchanges[pending.index].partial = streaming.streamedText
            if streamProvider == .whimbrel, message.hasPrefix("HTTP 402") {
                // A 402 from Whimbrel's server: the Whimbrel AI credit is used up.
                show(.providerFailureBecause(.creditExhausted), at: pending.index)
            } else {
                exchanges[pending.index].errorText = String(localized: "Couldn't reach the AI provider. Try again.")
            }
            self.pending = nil
        default:
            break
        }
    }
}

/// The Ask bar pinned to the bottom of a meeting page. Answers come from this meeting's
/// transcript only (`whimbrel_llm_prepare_meeting_ask`); each `[n]` becomes a chip that
/// calls `onCite` with the transcript segment id. Not saved: `session` lives as long as its
/// owner (the page visit, or the live meeting), which tears it down.
struct MeetingAskBar: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject var session: MeetingAskSession
    let meeting: CoreMeeting
    let segmentCount: Int
    /// True during a live meeting: answers come from the transcript so far, and say so.
    var liveContext: Bool = false
    let onCite: (Int64) -> Void

    @State private var question = ""

    private var disabledReason: String? {
        MeetingAskAvailability.disabledReason(
            status: meeting.status, isProcessing: appState.processingMeetingId == meeting.id, segmentCount: segmentCount)
    }

    private var isBusy: Bool { session.isBusy }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            if !session.exchanges.isEmpty { answerPanel }
            if disabledReason == nil {
                HStack(spacing: Spacing.xs) {
                    ForEach(AskPreset.allCases) { preset in
                        Button(preset.label) { submit(preset.label) }
                            .buttonStyle(.whimbrelSecondary).controlSize(.small).disabled(isBusy)
                    }
                }
            }
            HStack(spacing: Spacing.sm) {
                TextField(disabledReason ?? String(localized: "Ask about this meeting"), text: $question)
                    .textFieldStyle(.plain)
                    .disabled(disabledReason != nil || isBusy)
                    .onSubmit(send)
                    .accessibilityIdentifier("meeting-ask-field")
                if isBusy {
                    Button { if let q = session.cancel() { question = q } } label: { Image(systemName: "stop.circle") }.buttonStyle(.plain).help("Stop")
                } else {
                    Button(action: send) { Image(skinRole: .ask) }
                        .buttonStyle(.plain)
                        .foregroundStyle(Color.whimbrelAccentPrimary)
                        .disabled(disabledReason != nil || question.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .padding(Spacing.md)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
            .overlay(RoundedRectangle(cornerRadius: Radius.card).stroke(Color.whimbrelBorder))
        }
        .padding(Spacing.md)
        .frame(maxWidth: 760)
    }

    private var answerPanel: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.md) {
                if liveContext {
                    Text("Based on the transcript so far")
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
                ForEach(Array(session.exchanges.enumerated()), id: \.element.id) { index, exchange in
                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        Text(exchange.question).font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                        if let error = exchange.errorText {
                            if !exchange.partial.isEmpty { AskAnswerText(markdown: exchange.partial) }
                            Text(error).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelError)
                            HStack(spacing: Spacing.xs) {
                                Button("Retry") { retry(index) }
                                    .buttonStyle(.whimbrelSecondary).controlSize(.small).disabled(isBusy)
                                if exchange.opensAIProvidersSettings { OpenAIProvidersSettingsButton() }
                            }
                        } else {
                            AskAnswerText(markdown: session.pending?.index == index ? session.streaming.streamedText : exchange.answer)
                            if session.pending?.index != index, !exchange.answer.isEmpty {
                                HStack(spacing: Spacing.xs) {
                                    Button("Copy") { Self.copy(exchange.answer) }
                                        .help("Copy the answer as Markdown")
                                    Button("Copy as plain text") { Self.copy(AskMarkdown.plainText(exchange.answer)) }
                                }
                                .buttonStyle(.whimbrelSecondary).controlSize(.small)
                            }
                        }
                        if exchange.narrowed {
                            Text("Answered from the most relevant parts of a long transcript.")
                                .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                        }
                        if !exchange.citedSegmentIds.isEmpty {
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: Spacing.xs) {
                                    ForEach(Array(exchange.citedSegmentIds.enumerated()), id: \.offset) { n, id in
                                        Button { onCite(id) } label: { Text("Transcript \(n + 1)") }
                                            .buttonStyle(.whimbrelSecondary).controlSize(.small)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        .frame(maxHeight: 280)
        .padding(Spacing.md)
        .background(Color.whimbrelBackgroundCard)
        .clipShape(RoundedRectangle(cornerRadius: Radius.card))
        .overlay(RoundedRectangle(cornerRadius: Radius.card).stroke(Color.whimbrelBorder))
    }

    private static func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    private func send() {
        let text = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !isBusy, disabledReason == nil else { return }
        question = ""
        submit(text)
    }

    private func retry(_ index: Int) {
        guard disabledReason == nil, let text = session.takeForRetry(index) else { return }
        submit(text)
    }

    private func submit(_ text: String) {
        // Whimbrel AI needs a live sync ID token: refresh it first when 5 minutes or less are
        // left. The session is busy meanwhile, so a second send is ignored.
        let syncState = appState.syncState
        if appState.routing.preferredProvider == .whimbrel, syncState.whimbrelCredentialsNeedRefresh {
            guard !session.isBusy else { return }
            session.isRefreshingCredentials = true
            Task { @MainActor in
                await syncState.refreshWhimbrelCredentials()
                session.isRefreshingCredentials = false
                submitNow(text)
            }
            return
        }
        submitNow(text)
    }

    private func submitNow(_ text: String) {
        guard let store = appState.coreStore else { return }
        let defaults = appState.routing.defaultSelection()
        let model = defaults.provider == .ollama ? defaults.ollamaModel : defaults.hostedModel
        let routing = appState.routing.routingConfig(
            preferredProvider: defaults.provider, preferredModel: model,
            customInstructions: appState.promptLibrary.instructions[.ask] ?? "")
        // Local-only mode never streams to a hosted provider; a hosted route with no key
        // cannot stream either, so it reports an error rather than silently going local.
        let route: (provider: InferenceStreamClient.Provider, apiKey: String?, baseURLOverride: String?)? = {
            let hosted = { (p: InferenceStreamClient.Provider, k: KeychainService.Provider) -> (InferenceStreamClient.Provider, String?, String?)? in
                guard !appState.routing.localOnlyMode, let key = KeychainService.load(for: k), !key.isEmpty else { return nil }
                return (p, key, nil)
            }
            switch defaults.provider {
            case .ollama: return (LocalModel.replacesOllama(appState.routing) ? .local : .ollama, nil, nil)
            // The ID token rides `apiKey` and the device key `baseURLOverride`.
            case .whimbrel:
                guard !appState.routing.localOnlyMode, let credentials = appState.syncState.whimbrelCredentials else { return nil }
                return (.whimbrel, credentials.idToken, credentials.deviceKey)
            case .google: return hosted(.google, .google)
            case .anthropic: return hosted(.anthropic, .anthropic)
            case .openai: return hosted(.openai, .openai)
            case .openrouter: return model.isEmpty ? nil : hosted(.openrouter, .openrouter)
            }
        }()
        let noRouteMessage = appState.routing.localOnlyMode
            ? String(localized: "Local-only mode is on, so Ask can't use a hosted model. Choose a local model in Settings.")
            : defaults.provider == .whimbrel
            ? String(localized: "Sign in to sync to use Whimbrel AI")
            : String(localized: "No API key is set for the selected provider. Choose a local model or add a key in Settings.")
        let preset = AskPreset.matching(text)
        session.submit(preset?.label ?? text, prompt: preset?.prompt, meetingId: meeting.id, store: store, routing: routing, model: model, route: route, noRouteMessage: noRouteMessage,
            fallback: appState.slowStore.map { ($0, defaults.ollamaModel) })
    }
}
