import Foundation

/// A citation the core returns from `whimbrel_llm_finalize_ask`, and what
/// `whimbrel_citations_resolve` takes back in.
struct CitationRef: Codable, Equatable {
    let kind: String
    let id: Int64
}

/// What the phone shows for one citation (`core/api/src/citations.rs`'s `CitationCard`).
struct CitationCard: Decodable, Equatable {
    let kind: String
    let id: Int64
    let title: String
    let subtitle: String
    let tsUnixMs: Int64?
    let meetingId: Int64?
    let deviceId: String?
    /// False when the row isn't in this replica.
    let found: Bool

    private enum CodingKeys: String, CodingKey {
        case kind, id, title, subtitle, found
        case tsUnixMs = "ts_unix_ms", meetingId = "meeting_id", deviceId = "device_id"
    }
}

enum PrepareResult: Equatable {
    case ok(prepareId: String, prompt: String)
    case spendCap
    case failed
}

/// The core calls an Ask needs. `prepare`/`finalize`/`resolve` run on the store queue (via
/// `SyncModel.read`); `stream` never does (faked in tests).
protocol AskBackend: Sendable {
    func prepare(query: String, historyJSON: String, routingJSON: String) async -> PrepareResult
    /// Extracts the answer's citations and logs usage; nil if the core call failed.
    func finalize(prepareId: String, text: String, model: String, inputTokens: Int64, outputTokens: Int64) async -> [CitationCard]?
    /// Fire and forget: nothing waits on a discard.
    func discard(prepareId: String)
    func resolve(refs: [CitationRef]) async -> [CitationCard]
    func stream(prompt: String, model: String, provider: AskProvider, key: String) -> AsyncStream<InferenceEvent>
}

/// The store calls behind `PhoneAskBackend`, on the store queue (`CoreMemoryStore` on the
/// phone). Separate from `PhoneReadStore` so the screens' fakes needn't know about Ask.
protocol PhoneAskStore: AnyObject {
    func prepareAsk(query: String, historyJSON: String, routingJSON: String) -> PrepareResult
    func finalizeAsk(prepareId: String, text: String, model: String, inputTokens: Int64, outputTokens: Int64) -> [CitationCard]?
    func discardAsk(prepareId: String)
    func resolveCitations(_ refs: [CitationRef]) -> [CitationCard]
}

enum AskRoute: Equatable {
    case ready(provider: AskProvider, model: String, key: String)
    case noHostedKey
    case localOnly
}

struct AskTurn: Identifiable, Equatable {
    enum State: Equatable { case streaming, done, stopped, spendCap, failed }
    let id = UUID()
    let question: String
    var answer = ""
    var state = State.streaming
    var citations: [CitationCard] = []
    var modelLine: String
}

/// Ask on the phone: picks a hosted provider from the synced settings, has the core assemble
/// the prompt from the replica, streams the answer with the synced key, then asks the core
/// for the citations. Mirrors the Mac's `askStreaming` (`TimelineView+Ask.swift`).
@MainActor
final class AskModel: ObservableObject {
    @Published private(set) var turns: [AskTurn] = []

    private let backend: AskBackend
    private let prefs: PhoneRoutingPrefs
    private let keys: (AskProvider) -> String?
    /// The running ask, if any. Exposed so tests can await it.
    private(set) var task: Task<Void, Never>?
    /// Each run's prepare awaiting a finalize or discard, by turn; whoever takes it clears it, so
    /// it is finalized or discarded exactly once, and a stopped run never touches the next turn's.
    private var pendingPrepareIds: [UUID: String] = [:]

    /// The Mac's default model per provider; OpenRouter has none (the synced pick or nothing).
    private static let macDefaults: [AskProvider: String] = [
        .anthropic: "claude-haiku-4-5", .openai: "gpt-4o-mini", .google: "gemini-2.5-flash",
    ]

    init(backend: AskBackend, prefs: PhoneRoutingPrefs = PhoneRoutingPrefs(), keys: @escaping (AskProvider) -> String? = { PhoneKeyStore().key(for: $0) }) {
        self.backend = backend
        self.prefs = prefs
        self.keys = keys
    }

    var route: AskRoute { Self.route(prefs: prefs, keys: keys) }
    var isBusy: Bool { turns.last?.state == .streaming }

    /// The synced preferred provider if it has a key here, else the first with one in
    /// anthropic, openai, google, openrouter order. Local-only mode has no route.
    static func route(prefs: PhoneRoutingPrefs, keys: (AskProvider) -> String?) -> AskRoute {
        if prefs.localOnlyMode { return .localOnly }
        let preferred = prefs.preferredProvider.flatMap(AskProvider.init(rawValue:))
        for provider in ([preferred].compactMap { $0 } + AskProvider.allCases) {
            guard let key = keys(provider), !key.isEmpty,
                  let model = prefs.defaultModel(for: provider) ?? macDefaults[provider] else { continue }
            return .ready(provider: provider, model: model, key: key)
        }
        return .noHostedKey
    }

    func send(_ text: String) {
        let question = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !question.isEmpty, !isBusy, case .ready(let provider, let model, let key) = route else { return }
        let history = historyJSON()
        turns.append(AskTurn(question: question, modelLine: "\(model) · your key"))
        let turnId = turns[turns.count - 1].id
        let routing = routingJSON(provider: provider, model: model, key: key)
        task = Task { [weak self] in
            await self?.run(turnId: turnId, question: question, history: history, routing: routing, provider: provider, model: model, key: key)
        }
    }

    /// Ends the running ask: the question stays, with whatever had streamed.
    func stop() {
        guard let i = turns.indices.last, turns[i].state == .streaming else { return }
        turns[i].state = .stopped
        task?.cancel()
        discardPending(turns[i].id)
    }

    private func run(turnId: UUID, question: String, history: String, routing: String, provider: AskProvider, model: String, key: String) async {
        let prepared = await backend.prepare(query: question, historyJSON: history, routingJSON: routing)
        switch prepared {
        case .spendCap: return update(turnId) { if $0.state == .streaming { $0.state = .spendCap } }
        case .failed: return update(turnId) { if $0.state == .streaming { $0.state = .failed } }
        case .ok(let prepareId, let prompt):
            // Stopped while preparing: nobody will stream or finalize this one.
            if Task.isCancelled { return backend.discard(prepareId: prepareId) }
            pendingPrepareIds[turnId] = prepareId
            for await event in backend.stream(prompt: prompt, model: model, provider: provider, key: key) {
                guard turn(turnId)?.state == .streaming else { break }
                switch event {
                case .textDelta(_, _, let text): update(turnId) { $0.answer += text }
                case .terminal(_, .completed, _, let finalText, let usage):
                    guard let id = takePending(turnId) else { return }
                    update(turnId) { $0.answer = finalText }
                    let citations = await backend.finalize(prepareId: id, text: finalText, model: model,
                                                           inputTokens: usage.inputTokens ?? -1, outputTokens: usage.outputTokens ?? -1)
                    update(turnId) {
                        guard $0.state == .streaming else { return }
                        $0.citations = citations ?? []
                        $0.state = .done
                    }
                    return
                case .terminal(_, .cancelled, _, _, _), .terminal(_, .failed, _, _, _):
                    discardPending(turnId)
                    // A drop after some text keeps it ("Stopped"); before any text it is a failure.
                    update(turnId) { $0.state = $0.answer.isEmpty ? .failed : .stopped }
                    return
                case .state, .progress: break
                }
            }
            // The stream ended without a terminal event: stopped, or the run went away.
            discardPending(turnId)
            update(turnId) { if $0.state == .streaming { $0.state = .failed } }
        }
    }

    private func turn(_ id: UUID) -> AskTurn? { turns.first { $0.id == id } }

    private func update(_ id: UUID, _ change: (inout AskTurn) -> Void) {
        if let i = turns.firstIndex(where: { $0.id == id }) { change(&turns[i]) }
    }

    private func takePending(_ turnId: UUID) -> String? { pendingPrepareIds.removeValue(forKey: turnId) }

    private func discardPending(_ turnId: UUID) {
        if let id = takePending(turnId) { backend.discard(prepareId: id) }
    }

    /// `whimbrel_llm_answer`'s `history_json`: finished turns, oldest first.
    private func historyJSON() -> String {
        let pairs = turns.filter { $0.state == .done }.flatMap { [["role": "User", "text": $0.question], ["role": "Assistant", "text": $0.answer]] }
        return Self.json(pairs) ?? "[]"
    }

    /// `routing_config_json`: the minimum the brief lists, plus `use_local_model: false`
    /// (main's routing adds Ollama to every chain unless told otherwise; the phone has none).
    /// Only the chosen provider's key is sent. Nil caps are left out (the core's default).
    private func routingJSON(provider: AskProvider, model: String, key: String) -> String {
        var routing: [String: Any] = [
            "sensitive": false,
            "preferred_provider": provider.rawValue,
            "preferred_model": model,
            "custom_instructions": prefs.customInstructions,
            "use_local_model": false,
        ]
        switch provider {
        case .anthropic: routing["anthropic_key"] = key
        case .openai: routing["openai_key"] = key
        case .google: routing["google_key"] = key
        case .openrouter:
            routing["openrouter_key"] = key
            routing["openrouter_model"] = model
        }
        if let cap = prefs.userDailyCapCents { routing["user_daily_cap_cents"] = cap }
        if let cap = prefs.globalDailyCapCents { routing["global_daily_cap_cents"] = cap }
        return Self.json(routing) ?? "{}"
    }

    private static func json(_ value: Any) -> String? {
        (try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])).flatMap { String(data: $0, encoding: .utf8) }
    }
}

/// The real backend: store calls through `SyncModel.read` (the store queue), streaming
/// through the shared `InferenceStreamClient` on its own task, off the store queue and main.
final class PhoneAskBackend: AskBackend, @unchecked Sendable {
    private let sync: SyncModel

    init(sync: SyncModel) { self.sync = sync }

    func prepare(query: String, historyJSON: String, routingJSON: String) async -> PrepareResult {
        await sync.read { ($0 as? PhoneAskStore)?.prepareAsk(query: query, historyJSON: historyJSON, routingJSON: routingJSON) ?? .failed } ?? .failed
    }

    func finalize(prepareId: String, text: String, model: String, inputTokens: Int64, outputTokens: Int64) async -> [CitationCard]? {
        let cards: [CitationCard]?? = await sync.read {
            ($0 as? PhoneAskStore)?.finalizeAsk(prepareId: prepareId, text: text, model: model, inputTokens: inputTokens, outputTokens: outputTokens)
        }
        return cards ?? nil
    }

    func discard(prepareId: String) {
        Task { await sync.read { ($0 as? PhoneAskStore)?.discardAsk(prepareId: prepareId) } }
    }

    func resolve(refs: [CitationRef]) async -> [CitationCard] {
        await sync.read { ($0 as? PhoneAskStore)?.resolveCitations(refs) ?? [] } ?? []
    }

    func stream(prompt: String, model: String, provider: AskProvider, key: String) -> AsyncStream<InferenceEvent> {
        AsyncStream { continuation in
            let work = Task.detached(priority: .userInitiated) {
                guard let p = InferenceStreamClient.Provider(rawValue: provider.rawValue),
                      let runId = InferenceStreamClient.start(prompt: prompt, model: model, provider: p, apiKey: key) else {
                    continuation.yield(.terminal(runId: "", outcome: .failed(classification: "start", message: "could not start"), atUnixMs: 0, finalText: "", usage: .init()))
                    continuation.finish()
                    return
                }
                while !Task.isCancelled {
                    for event in InferenceStreamClient.poll(runId: runId) where event.runId == runId {
                        continuation.yield(event)
                        if case .terminal = event {
                            InferenceStreamClient.release(runId: runId)
                            continuation.finish()
                            return
                        }
                    }
                    try? await Task.sleep(nanoseconds: 100_000_000)
                }
                // Cancelled by the consumer (Stop): same as the Mac's `releaseCurrentRun`.
                InferenceStreamClient.cancel(runId: runId)
                InferenceStreamClient.release(runId: runId)
            }
            continuation.onTermination = { _ in work.cancel() }
        }
    }
}
