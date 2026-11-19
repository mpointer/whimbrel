import AppKit
import Combine
import Foundation

/// The route the hotkey panel uses for one ask. Read when the panel opens (for the
/// destination line) and again on Return; the ask goes ahead only if both agree. Not
/// stored between the two, since it carries API keys.
struct WindowAskRouting {
    let config: CoreRoutingConfig
    /// Nil when the chosen provider cannot stream (a hosted provider with no key, or a
    /// hosted provider in local-only mode): the panel then refuses rather than letting a
    /// fallback chain answer through a provider the destination line did not name.
    let stream: AskStreamRoute?
    /// False in local-only mode when neither Ollama models nor the bundled model exist.
    let localModelAvailable: Bool
    /// The route for a request core says must stay on this Mac (it carries a client excluded from hosted
    /// models) while the settings route is hosted: a model on this Mac (never an Ollama cloud model). Nil when
    /// there is none; such a request is then refused before Return.
    var local: Local? = nil

    struct Local {
        let config: CoreRoutingConfig
        let stream: AskStreamRoute
    }
}

/// Core's rule (`llm::is_ollama_cloud_model`): an Ollama model with a "cloud" word in its tag runs on Ollama's
/// servers, so it leaves this computer like any hosted provider.
func isOllamaCloudModel(_ model: String) -> Bool {
    model.lowercased().split(whereSeparator: { !($0.isASCII && ($0.isLetter || $0.isNumber)) }).contains("cloud")
}

enum WindowAskStreamOutcome {
    case completed(text: String, usage: InferenceEvent.TokenUsage)
    case failed(String)
}

/// The core and stream calls an "Ask about this window" makes. A seam so the model is
/// testable without a store or a provider.
@MainActor
protocol WindowAskBridge: AnyObject {
    func prepare(_ window: WindowContext, query: String, routing: CoreRoutingConfig) async -> Result<WindowAskPrepared, CoreLLMError>?
    /// Streams `prompt`; `onText` gets the text so far. Returns once the run ends.
    func stream(prompt: String, model: String, route: AskStreamRoute,
                onText: @escaping @MainActor (String) -> Void) async -> WindowAskStreamOutcome
    func cancelStream()
    func finalize(prepareId: String, text: String, model: String, usage: InferenceEvent.TokenUsage) async
    func discard(prepareId: String)
    /// Core's destination line for this window and question (`window_ask::destination_for`); nil when it could
    /// not be read, and the panel falls back to its mirror of the settings.
    func destination(_ window: WindowContext, query: String, routing: CoreRoutingConfig) async -> WindowAskDestination?
}

extension WindowAskBridge {
    func destination(_ window: WindowContext, query: String, routing: CoreRoutingConfig) async -> WindowAskDestination? { nil }
}

/// The hotkey panel: reads the window that was frontmost before the panel opened, shows
/// what will be read and where it will go, and asks only when the user presses Return.
/// Keeps no history. Window text, questions and answers are never logged.
@MainActor
final class HotkeyPanelModel: ObservableObject {
    typealias Frontmost = (bundleId: String, name: String, pid: pid_t)

    enum State: Equatable {
        case reading, ready(ContextLine), blocked(Reason), answering, answered(String), failed(String)
    }

    struct ContextLine: Equatable {
        let reading: String
        let destination: String
        let truncated: Bool
    }

    enum Reason: Equatable {
        case excluded(String), unreadable(String), notPermitted, localModelMissing
        /// The window or question carries a client excluded from hosted models, and no model on this Mac can answer.
        case clientNeedsLocalModel

        var message: String {
            switch self {
            case .excluded: return String(localized: "This app is excluded from capture.")
            case .unreadable: return String(localized: "I couldn't read this window.")
            case .notPermitted: return String(localized: "Whimbrel needs Accessibility permission to read this window.")
            case .localModelMissing: return String(localized: "No local model is installed.")
            case .clientNeedsLocalModel:
                return String(localized: "This window or question involves a client you excluded from hosted models. Only a local model on this Mac can answer it, and none is set up. Nothing was sent.")
            }
        }
    }

    /// Core's window-text cap (`window_ask::WINDOW_TEXT_MAX_CHARS`), in Unicode scalars as
    /// Rust counts `char`s.
    static let windowTextMaxChars = 6_000

    @Published private(set) var state: State = .reading
    /// The context and destination lines, kept while answering and once answered.
    @Published private(set) var context: ContextLine?
    /// The streamed answer so far, while `.answering`.
    @Published private(set) var partialAnswer = ""

    private let read: @MainActor (Frontmost) async -> WindowReadResult
    private let bridge: WindowAskBridge
    private let routing: @MainActor () -> WindowAskRouting
    private let beforeAsk: @MainActor (WindowAskRouting) async -> Void
    private let ownBundleId: String?
    private let readTimeout: Duration
    private let pasteboard: NSPasteboard

    /// Bumped by every open and close: a result for an older request is dropped.
    private var requestId = 0
    private var window: WindowContext?
    /// A prepared ask not yet finalized or discarded.
    private var pendingPrepareId: String?
    /// The destination line the settings gave when the line was last worked out from them.
    /// Core's line may since have replaced the shown one; a settings change is judged
    /// against this, so core's line is not swapped back and forth with the settings' line.
    private var settingsDestination: String?
    /// The question as typed, for core's destination line before Return.
    private var typedQuery = ""
    private var destinationTask: Task<Void, Never>?

    init(read: @escaping @MainActor (Frontmost) async -> WindowReadResult,
         bridge: WindowAskBridge,
         routing: @escaping @MainActor () -> WindowAskRouting,
         beforeAsk: @escaping @MainActor (WindowAskRouting) async -> Void = { _ in },
         ownBundleId: String? = Bundle.main.bundleIdentifier,
         readTimeout: Duration = .seconds(3),
         pasteboard: NSPasteboard = .general) {
        self.read = read
        self.bridge = bridge
        self.routing = routing
        self.beforeAsk = beforeAsk
        self.ownBundleId = ownBundleId
        self.readTimeout = readTimeout
        self.pasteboard = pasteboard
    }

    convenience init(reader: FocusedWindowReader, store: CoreMemoryStore, routing: @escaping @MainActor () -> WindowAskRouting) {
        self.init(reader: reader, store: { store }, routing: routing)
    }

    /// `store` is read at ask time: `AppState.coreStore` is nil until the store opens.
    /// `beforeAsk` runs on Return only (never on open), e.g. to refresh Whimbrel AI's token.
    convenience init(reader: FocusedWindowReader, store: @escaping @MainActor () -> CoreMemoryStore?,
                     routing: @escaping @MainActor () -> WindowAskRouting,
                     beforeAsk: @escaping @MainActor (WindowAskRouting) async -> Void = { _ in }) {
        self.init(read: { await reader.read(frontmost: $0) }, bridge: CoreWindowAskBridge(store: store),
                  routing: routing, beforeAsk: beforeAsk)
    }

    // MARK: - Open

    /// `frontmost` is the app that was frontmost before the panel took focus. Reading is
    /// local; nothing is sent until `ask`.
    func open(frontmost: Frontmost) async {
        reset()
        let id = requestId
        if let ownBundleId, frontmost.bundleId == ownBundleId {
            state = .blocked(.excluded(frontmost.name))
            return
        }
        guard frontmost.pid > 0 else {
            state = .blocked(.unreadable(frontmost.name))
            return
        }
        let result = await readWithDeadline(frontmost)
        guard id == requestId else { return }
        switch result {
        case nil:
            state = .blocked(.unreadable(frontmost.name))
        case .excluded(let app)?:
            state = .blocked(.excluded(app))
        case .unreadable(let app)?:
            state = .blocked(.unreadable(app))
        case .notPermitted?:
            state = .blocked(.notPermitted)
        case .read(let window)?:
            let route = routing()
            let core = await destinationWithDeadline(window, route: route)
            guard id == requestId else { return }
            self.window = window
            show(Self.plan(route, core: core), window: window, route: route)
        }
    }

    /// Core's line, or nil (the settings mirror) when the store queue is busy past the read timeout: the
    /// prepare checks the destination again before anything is sent.
    private func destinationWithDeadline(_ window: WindowContext, route: WindowAskRouting) async -> WindowAskDestination? {
        let bridge = self.bridge
        let timeout = readTimeout
        return await withCheckedContinuation { (continuation: CheckedContinuation<WindowAskDestination?, Never>) in
            var resumed = false
            var timer: Task<Void, Never>?
            let finish: @MainActor (WindowAskDestination?) -> Void = { value in
                guard !resumed else { return }
                resumed = true
                timer?.cancel()
                continuation.resume(returning: value)
            }
            timer = Task { @MainActor in
                do { try await Task.sleep(for: timeout) } catch { return }
                finish(nil)
            }
            Task { @MainActor in finish(await bridge.destination(window, query: "", routing: route.config)) }
        }
    }

    /// The question changed: core's line is read again (debounced), so "kept on this Mac" or "not sent" shows
    /// before Return. Sends nothing.
    func questionChanged(_ question: String) {
        typedQuery = question.trimmingCharacters(in: .whitespacesAndNewlines)
        destinationTask?.cancel()
        guard let window else { return }
        switch state {
        case .ready, .blocked(.clientNeedsLocalModel), .answered, .failed: break
        default: return
        }
        let id = requestId
        let query = typedQuery
        destinationTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard let self, !Task.isCancelled, id == self.requestId else { return }
            let route = self.routing()
            let core = await self.bridge.destination(window, query: query, routing: route.config)
            guard !Task.isCancelled, id == self.requestId, query == self.typedQuery else { return }
            switch self.state {
            case .ready, .blocked(.clientNeedsLocalModel):
                self.show(Self.plan(route, core: core), window: window, route: route)
            case .answered, .failed:
                // Keep the answer (and Copy) on screen: only the line changes.
                let plan = Self.plan(route, core: core)
                let base = Self.contextLine(window, config: route.config)
                self.context = ContextLine(reading: base.reading, destination: plan.line, truncated: base.truncated)
                self.settingsDestination = plan.line
            default:
                break
            }
        }
    }

    /// Where one request goes: the line to show and the route that sends it, or a refusal.
    enum Plan: Equatable {
        /// `viaLocal`: core kept it on this Mac though the settings route is hosted.
        case send(line: String, viaLocal: Bool)
        case refuse(line: String)

        var line: String {
            switch self {
            case .send(let line, _), .refuse(let line): return line
            }
        }
    }

    /// Core's destination when it gave one, else the mirror of the settings. A request core keeps on this Mac
    /// goes to the local route, or is refused when there is none.
    static func plan(_ route: WindowAskRouting, core: WindowAskDestination?) -> Plan {
        if core == .localOnly && !route.config.sensitive {
            return route.local == nil ? .refuse(line: keptLocalRefusedText) : .send(line: keptLocalText, viaLocal: true)
        }
        return .send(line: destinationText(core ?? destination(route.config)), viaLocal: false)
    }

    static let keptLocalText = String(localized: "This will be answered on this computer only: it involves a client excluded from hosted models")
    static let keptLocalRefusedText = String(localized: "Not sent: it involves a client excluded from hosted models, and no local model is set up")

    private func show(_ plan: Plan, window: WindowContext, route: WindowAskRouting) {
        let base = Self.contextLine(window, config: route.config)
        let line = ContextLine(reading: base.reading, destination: plan.line, truncated: base.truncated)
        context = line
        settingsDestination = line.destination
        if case .refuse = plan {
            state = .blocked(.clientNeedsLocalModel)
        } else if route.config.sensitive && !route.localModelAvailable {
            state = .blocked(.localModelMissing)
        } else {
            state = .ready(line)
        }
    }

    /// The read, or nil if it has not finished within `readTimeout`. The reader cannot be
    /// cancelled (its AX work runs detached), so this races it rather than awaiting it; a
    /// task group would wait for the hung read before returning.
    private func readWithDeadline(_ frontmost: Frontmost) async -> WindowReadResult? {
        let read = self.read
        let timeout = readTimeout
        return await withCheckedContinuation { (continuation: CheckedContinuation<WindowReadResult?, Never>) in
            var resumed = false
            var timer: Task<Void, Never>?
            let finish: @MainActor (WindowReadResult?) -> Void = { value in
                guard !resumed else { return }
                resumed = true
                timer?.cancel()
                continuation.resume(returning: value)
            }
            timer = Task { @MainActor in
                do { try await Task.sleep(for: timeout) } catch { return }
                finish(nil)
            }
            Task { @MainActor in finish(await read(frontmost)) }
        }
    }

    // MARK: - Ask

    func ask(_ question: String) async {
        let query = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty, let window, let shown = context else { return }
        switch state {
        case .ready, .answered, .failed: break
        case .reading, .blocked, .answering: return
        }
        // The panel answers one question at a time; a new question replaces the last answer.
        discardPending()
        requestId += 1
        let id = requestId
        state = .answering
        partialAnswer = ""

        await beforeAsk(routing())
        guard id == requestId else { return }
        // Read fresh (keys and tokens are not kept), but never send anywhere other than
        // the destination line the user saw: if the setting changed while the panel was
        // open, show the new line and wait for Return again.
        let route = routing()
        let core = await bridge.destination(window, query: query, routing: route.config)
        guard id == requestId else { return }
        let plan = Self.plan(route, core: core)
        let base = Self.contextLine(window, config: route.config)
        let line = ContextLine(reading: base.reading, destination: plan.line, truncated: base.truncated)
        guard line.destination == settingsDestination else {
            show(plan, window: window, route: route)
            return
        }
        if case .refuse = plan {
            state = .blocked(.clientNeedsLocalModel)
            return
        }
        // Kept on this Mac: prepared and streamed through the local route, never the hosted one.
        let viaLocal = plan == .send(line: line.destination, viaLocal: true)
        let local = viaLocal ? route.local : nil
        let config = local?.config ?? route.config

        guard let prepared = await bridge.prepare(window, query: query, routing: config) else {
            if id == requestId { state = .failed(CoreLLMError.providerFailure.userFacingMessage) }
            return
        }
        let data: WindowAskPrepared
        switch prepared {
        case .failure(.localModelMissing):
            if id == requestId { state = .blocked(.localModelMissing) }
            return
        case .failure(let error):
            if id == requestId { state = .failed(error.userFacingMessage) }
            return
        case .success(let value):
            data = value
        }
        guard id == requestId else {
            // Closed (or reopened) while preparing: nothing will finalize this one.
            bridge.discard(prepareId: data.prepareId)
            return
        }
        if let local {
            // Core must agree it stays on this computer; anything else sends nothing.
            guard data.destination == .local || data.destination == .localOnly else {
                bridge.discard(prepareId: data.prepareId)
                state = .failed(CoreLLMError.excludedClientMessage)
                return
            }
            await streamAndFinish(prepareId: data.prepareId, prompt: data.prompt, model: local.config.preferredModel, stream: local.stream, id: id)
            return
        }
        // Core's own word on where this goes. If it isn't the line the user saw, send
        // nothing: show core's line and wait for Return again.
        let coreLine = Self.destinationText(data.destination)
        guard coreLine == shown.destination else {
            bridge.discard(prepareId: data.prepareId)
            let corrected = ContextLine(reading: line.reading, destination: coreLine, truncated: line.truncated)
            context = corrected
            state = .ready(corrected)
            return
        }
        // The stream below is built from the settings route, so core's line must also match
        // the settings line. If the two keep disagreeing, even a confirmed line cannot send.
        guard coreLine == line.destination else {
            bridge.discard(prepareId: data.prepareId)
            state = .failed(CoreLLMError.providerFailure.userFacingMessage)
            return
        }
        guard let stream = route.stream else {
            bridge.discard(prepareId: data.prepareId)
            state = .failed(CoreLLMError.providerFailureBecause(.noProviderConfigured).userFacingMessage)
            return
        }
        await streamAndFinish(prepareId: data.prepareId, prompt: data.prompt, model: route.config.preferredModel, stream: stream, id: id)
    }

    private func streamAndFinish(prepareId: String, prompt: String, model: String, stream: AskStreamRoute, id: Int) async {
        let data = (prepareId: prepareId, prompt: prompt)
        pendingPrepareId = data.prepareId
        let outcome = await bridge.stream(prompt: data.prompt, model: model, route: stream) { [weak self] text in
            guard let self, id == self.requestId else { return }
            self.partialAnswer = text
        }
        // A close or reopen during the stream already discarded the prepared ask.
        guard id == requestId else { return }
        pendingPrepareId = nil
        switch outcome {
        case .completed(let text, let usage):
            await bridge.finalize(prepareId: data.prepareId, text: text, model: model, usage: usage)
            if id == requestId { state = .answered(text) }
        case .failed(let message):
            bridge.discard(prepareId: data.prepareId)
            state = .failed(message)
        }
    }

    func copyAnswer() {
        guard case .answered(let text) = state else { return }
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
    }

    /// Discards everything: the answer, the read text, any unfinished ask.
    func close() {
        reset()
    }

    private func reset() {
        requestId += 1
        destinationTask?.cancel()
        typedQuery = ""
        discardPending()
        window = nil
        context = nil
        settingsDestination = nil
        partialAnswer = ""
        state = .reading
    }

    private func discardPending() {
        guard let pendingPrepareId else { return }
        self.pendingPrepareId = nil
        bridge.cancelStream()
        bridge.discard(prepareId: pendingPrepareId)
    }

    // MARK: - Lines (exact strings; Task 9 reuses them on Windows)

    static func contextLine(_ window: WindowContext, config: CoreRoutingConfig) -> ContextLine {
        let kind = window.isSelection ? String(localized: "selected text") : String(localized: "visible text")
        let title = window.title.trimmingCharacters(in: .whitespacesAndNewlines)
        var reading = title.isEmpty
            ? String(localized: "Reading: \(window.appName) (\(kind))")
            : String(localized: "Reading: \(window.appName), '\(title)' (\(kind))")
        let truncated = window.text.unicodeScalars.count > windowTextMaxChars
        if truncated { reading += String(localized: " — first part only") }
        return ContextLine(reading: reading, destination: destinationText(config), truncated: truncated)
    }

    /// Mirrors core's `window_ask::destination`: local only for Ollama and the bundled
    /// model; any other provider, even one this build doesn't know, is named as leaving.
    static func destination(_ config: CoreRoutingConfig) -> WindowAskDestination {
        if config.sensitive { return .localOnly }
        switch config.preferredProvider {
        case "ollama" where isOllamaCloudModel(config.ollamaModel) || isOllamaCloudModel(config.preferredModel):
            return .hosted(provider: "ollama")
        case "ollama", "local": return .local
        case let provider: return .hosted(provider: provider)
        }
    }

    static func destinationText(_ config: CoreRoutingConfig) -> String {
        destinationText(destination(config))
    }

    static func destinationText(_ destination: WindowAskDestination) -> String {
        switch destination {
        case .localOnly, .local:
            return String(localized: "This will be answered on this computer")
        case .hosted(let provider):
            switch provider {
            case "anthropic": return String(localized: "The text above will be sent to Anthropic")
            case "openai": return String(localized: "The text above will be sent to OpenAI")
            case "google": return String(localized: "The text above will be sent to Google")
            case "openrouter": return String(localized: "The text above will be sent to OpenRouter")
            case "whimbrel": return String(localized: "The text above will be sent to Anthropic, through Whimbrel's server")
            case "ollama": return String(localized: "The text above will be sent to Ollama's cloud")
            default: return String(localized: "The text above will be sent to \(provider)")
            }
        }
    }
}

/// Real bridge: core calls on `CoreMemoryStore.accessQueue`, streaming through the same
/// `StreamingAskViewModel` the Ask tab uses.
@MainActor
final class CoreWindowAskBridge: WindowAskBridge {
    private let store: @MainActor () -> CoreMemoryStore?
    private let streaming = StreamingAskViewModel()
    private var observation: AnyCancellable?
    private var finishPending: (@MainActor (WindowAskStreamOutcome) -> Void)?

    init(store: @escaping @MainActor () -> CoreMemoryStore?) { self.store = store }

    func prepare(_ window: WindowContext, query: String, routing: CoreRoutingConfig) async -> Result<WindowAskPrepared, CoreLLMError>? {
        guard let store = store() else { return nil }
        return await withCheckedContinuation { done in
            CoreMemoryStore.accessQueue.async {
                done.resume(returning: store.prepareWindowAsk(window, query: query, routing: routing))
            }
        }
    }

    func stream(prompt: String, model: String, route: AskStreamRoute,
                onText: @escaping @MainActor (String) -> Void) async -> WindowAskStreamOutcome {
        // A run still waiting (one whose cancel could not land) ends now, so its caller,
        // which has already moved on, is not left suspended.
        finishPending?(.failed(String(localized: "Stopped.")))
        let streaming = self.streaming
        return await withCheckedContinuation { done in
            var finished = false
            let finish: @MainActor (WindowAskStreamOutcome) -> Void = { [weak self] outcome in
                guard !finished else { return }
                finished = true
                self?.observation = nil
                self?.finishPending = nil
                done.resume(returning: outcome)
            }
            finishPending = finish
            // Start first: `start` sets `.connecting` synchronously, so the subscription's
            // first value is this run's, never the previous run's terminal phase.
            streaming.start(prompt: prompt, model: model, provider: route.provider,
                            apiKey: route.apiKey, baseURLOverride: route.baseURLOverride)
            observation = streaming.$streamedText.combineLatest(streaming.$phase)
                .sink { text, phase in
                    MainActor.assumeIsolated {
                        switch phase {
                        case .idle, .connecting, .streaming:
                            onText(text)
                        case .cancelling:
                            break
                        case .completed:
                            finish(.completed(text: streaming.streamedText, usage: streaming.usage))
                        case .cancelled:
                            finish(.failed(String(localized: "Stopped.")))
                        case .failed(let message) where route.provider == .whimbrel && message.hasPrefix("HTTP 402"):
                            finish(.failed(CoreLLMError.providerFailureBecause(.creditExhausted).userFacingMessage))
                        case .failed where streaming.exclusionRefusal == .excludedClient:
                            // The gate held it back; nothing was sent. No local model was tried here, so the
                            // words do not say one could not answer.
                            finish(.failed(ClientCopy.hotkeyGateRefused))
                        case .failed(let message) where streaming.exclusionRefusal != nil:
                            finish(.failed(message))
                        case .failed:
                            finish(.failed(CoreLLMError.providerFailure.userFacingMessage))
                        }
                    }
                }
        }
    }

    func cancelStream() { streaming.cancel() }

    func destination(_ window: WindowContext, query: String, routing: CoreRoutingConfig) async -> WindowAskDestination? {
        guard let store = store() else { return nil }
        return await withCheckedContinuation { done in
            CoreMemoryStore.accessQueue.async { done.resume(returning: store.windowAskDestination(window, query: query, routing: routing)) }
        }
    }

    func finalize(prepareId: String, text: String, model: String, usage: InferenceEvent.TokenUsage) async {
        guard let store = store() else { return }
        await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
            CoreMemoryStore.accessQueue.async {
                _ = store.finalizeAsk(prepareId: prepareId, finalText: text, model: model,
                                      inputTokens: usage.inputTokens ?? -1, outputTokens: usage.outputTokens ?? -1)
                done.resume()
            }
        }
    }

    func discard(prepareId: String) { store()?.discardAsk(prepareId: prepareId) }
}
