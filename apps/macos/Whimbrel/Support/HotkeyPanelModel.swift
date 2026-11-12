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

        var message: String {
            switch self {
            case .excluded: return String(localized: "This app is excluded from capture.")
            case .unreadable: return String(localized: "I couldn't read this window.")
            case .notPermitted: return String(localized: "Whimbrel needs Accessibility permission to read this window.")
            case .localModelMissing: return String(localized: "No local model is installed.")
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
            let line = Self.contextLine(window, config: route.config)
            self.window = window
            context = line
            settingsDestination = line.destination
            if route.config.sensitive && !route.localModelAvailable {
                state = .blocked(.localModelMissing)
            } else {
                state = .ready(line)
            }
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
        let line = Self.contextLine(window, config: route.config)
        guard line.destination == settingsDestination else {
            settingsDestination = line.destination
            context = line
            state = route.config.sensitive && !route.localModelAvailable ? .blocked(.localModelMissing) : .ready(line)
            return
        }

        guard let prepared = await bridge.prepare(window, query: query, routing: route.config) else {
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
        pendingPrepareId = data.prepareId
        let model = route.config.preferredModel
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
                        case .failed:
                            finish(.failed(CoreLLMError.providerFailure.userFacingMessage))
                        }
                    }
                }
        }
    }

    func cancelStream() { streaming.cancel() }

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
