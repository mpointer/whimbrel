import XCTest
@testable import Whimbrel

/// Scripted backend: records every call, replays the events it is given.
final class FakeAskBackend: AskBackend, @unchecked Sendable {
    var prepareResult: PrepareResult = .ok(prepareId: "p1", prompt: "PROMPT")
    /// When set, successive `prepare` calls hand out these ids in order (then `prepareResult`).
    var prepareIds: [String] = []
    var events: [InferenceEvent] = []
    /// When true the stream stays open until it is cancelled (a run the user stops).
    var holdOpen = false
    /// Nanoseconds `prepare` takes, so a test can stop mid-prepare.
    var prepareDelay: UInt64 = 0
    var finalizeResult: [CitationCard]? = []
    private let lock = NSLock()
    private var _discards: [String] = []
    private var _finalizes: [String] = []
    private var _streams: [(model: String, provider: AskProvider, key: String)] = []
    private var _routing: [String] = []
    private var _history: [String] = []
    var discards: [String] { lock.lock(); defer { lock.unlock() }; return _discards }
    var finalizes: [String] { lock.lock(); defer { lock.unlock() }; return _finalizes }
    var streams: [(model: String, provider: AskProvider, key: String)] { lock.lock(); defer { lock.unlock() }; return _streams }
    var routingJSONs: [String] { lock.lock(); defer { lock.unlock() }; return _routing }
    var historyJSONs: [String] { lock.lock(); defer { lock.unlock() }; return _history }

    func prepare(query: String, historyJSON: String, routingJSON: String) async -> PrepareResult {
        lock.lock(); _routing.append(routingJSON); _history.append(historyJSON); lock.unlock()
        if prepareDelay > 0 { try? await Task.sleep(nanoseconds: prepareDelay) }
        lock.lock(); defer { lock.unlock() }
        if !prepareIds.isEmpty { return .ok(prepareId: prepareIds.removeFirst(), prompt: "PROMPT") }
        return prepareResult
    }

    func finalize(prepareId: String, text: String, model: String, inputTokens: Int64, outputTokens: Int64) async -> [CitationCard]? {
        lock.lock(); _finalizes.append(prepareId); lock.unlock()
        return finalizeResult
    }

    func discard(prepareId: String) {
        lock.lock(); _discards.append(prepareId); lock.unlock()
    }

    func resolve(refs: [CitationRef]) async -> [CitationCard] { [] }

    func stream(prompt: String, model: String, provider: AskProvider, key: String) -> AsyncStream<InferenceEvent> {
        lock.lock(); _streams.append((model, provider, key)); lock.unlock()
        let events = events, hold = holdOpen
        return AsyncStream { c in
            for e in events { c.yield(e) }
            if !hold { c.finish() }
        }
    }
}

@MainActor
final class AskModelTests: XCTestCase {
    private var suite = ""
    private var defaults: UserDefaults!
    private let p = PhoneRoutingPrefs.prefix

    override func setUp() {
        suite = "AskModelTests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
    }

    override func tearDown() { defaults.removePersistentDomain(forName: suite) }

    private func prefs() -> PhoneRoutingPrefs { PhoneRoutingPrefs(defaults: defaults) }
    private func set(_ field: String, _ value: String) { defaults.set(value, forKey: p + field) }

    private func terminal(_ outcome: InferenceEvent.TerminalOutcome = .completed, text: String) -> InferenceEvent {
        .terminal(runId: "r", outcome: outcome, atUnixMs: 0, finalText: text, usage: .init(inputTokens: 10, outputTokens: 5))
    }

    private func model(_ backend: FakeAskBackend, keys: [AskProvider: String] = [.anthropic: "sk-a"]) -> AskModel {
        AskModel(backend: backend, prefs: prefs(), keys: { keys[$0] })
    }

    /// Lets the model's task run to its end (or to where it waits on the held-open stream).
    private func settle(_ m: AskModel, until done: () -> Bool) async {
        for _ in 0..<200 where !done() { try? await Task.sleep(nanoseconds: 5_000_000) }
    }

    // MARK: route

    func test_route_prefers_synced_provider_with_key() {
        set("routing.preferredProvider", "openai")
        let r = AskModel.route(prefs: prefs(), keys: { [.anthropic: "a", .openai: "o"][$0] })
        XCTAssertEqual(r, .ready(provider: .openai, model: "gpt-4o-mini", key: "o"))
    }

    func test_route_falls_back_in_order_when_preferred_has_no_key() {
        set("routing.preferredProvider", "openai")
        XCTAssertEqual(AskModel.route(prefs: prefs(), keys: { [.google: "g", .openrouter: "r"][$0] }),
                       .ready(provider: .google, model: "gemini-2.5-flash", key: "g"))
        // "ollama" is not a hosted provider: same fallback order.
        set("routing.preferredProvider", "ollama")
        XCTAssertEqual(AskModel.route(prefs: prefs(), keys: { [.anthropic: "a", .google: "g"][$0] }),
                       .ready(provider: .anthropic, model: "claude-haiku-4-5", key: "a"))
    }

    func test_route_local_only_wins() {
        set("routing.localOnlyMode", "true")
        XCTAssertEqual(AskModel.route(prefs: prefs(), keys: { _ in "k" }), .localOnly)
    }

    func test_route_no_key_is_noHostedKey() {
        XCTAssertEqual(AskModel.route(prefs: prefs(), keys: { _ in nil }), .noHostedKey)
        // OpenRouter has no Mac default model: a key without a synced model is no route.
        XCTAssertEqual(AskModel.route(prefs: prefs(), keys: { [.openrouter: "r"][$0] }), .noHostedKey)
    }

    func test_model_falls_back_to_mac_defaults() {
        let keys: [AskProvider: String] = [.anthropic: "a", .openai: "o", .google: "g"]
        for (preferred, want) in [("anthropic", "claude-haiku-4-5"), ("openai", "gpt-4o-mini"), ("google", "gemini-2.5-flash")] {
            set("routing.preferredProvider", preferred)
            guard case .ready(_, let m, _) = AskModel.route(prefs: prefs(), keys: { keys[$0] }) else { return XCTFail(preferred) }
            XCTAssertEqual(m, want)
        }
        set("routing.anthropicDefaultModel", "claude-opus-x")
        set("routing.preferredProvider", "anthropic")
        guard case .ready(_, let m, _) = AskModel.route(prefs: prefs(), keys: { keys[$0] }) else { return XCTFail() }
        XCTAssertEqual(m, "claude-opus-x")
    }

    // MARK: engine

    func test_stream_then_finalize_attaches_citations() async {
        let backend = FakeAskBackend()
        backend.events = [.textDelta(runId: "r", sequence: 1, text: "Hel"), terminal(text: "Hello [1]")]
        let card = CitationCard(kind: "capture", id: 7, title: "t", subtitle: "s", tsUnixMs: nil, meetingId: nil, deviceId: nil, found: true)
        backend.finalizeResult = [card]
        let m = model(backend)
        m.send("what?")
        XCTAssertEqual(m.turns.count, 1)
        XCTAssertEqual(m.turns[0].state, .streaming)
        await settle(m) { m.turns.first?.state != .streaming }
        XCTAssertEqual(m.turns[0].state, .done)
        XCTAssertEqual(m.turns[0].answer, "Hello [1]")
        XCTAssertEqual(m.turns[0].citations, [card])
        XCTAssertEqual(backend.finalizes, ["p1"])
        XCTAssertEqual(backend.discards, [])
        XCTAssertEqual(backend.streams.first?.model, "claude-haiku-4-5")
        XCTAssertEqual(backend.streams.first?.key, "sk-a")
    }

    func test_routing_json_and_history_shape() async throws {
        set("routing.userDailyCapCents", "50")
        set("summary.customPromptInstructions", "Be brief")
        let backend = FakeAskBackend()
        backend.events = [terminal(text: "A1")]
        let m = model(backend)
        m.send("Q1")
        await settle(m) { m.turns.first?.state == .done }
        m.send("Q2")
        await settle(m) { m.turns.count == 2 && m.turns[1].state == .done }

        let routing = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(backend.routingJSONs[0].utf8)) as? [String: Any])
        XCTAssertEqual(routing["anthropic_key"] as? String, "sk-a")
        XCTAssertEqual(routing["preferred_provider"] as? String, "anthropic")
        XCTAssertEqual(routing["preferred_model"] as? String, "claude-haiku-4-5")
        XCTAssertEqual(routing["sensitive"] as? Bool, false)
        XCTAssertEqual(routing["user_daily_cap_cents"] as? Int, 50)
        XCTAssertNil(routing["global_daily_cap_cents"])
        XCTAssertEqual(routing["custom_instructions"] as? String, "Be brief")
        XCTAssertEqual(routing["use_local_model"] as? Bool, false)

        XCTAssertEqual(backend.historyJSONs[0], "[]")
        let history = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(backend.historyJSONs[1].utf8)) as? [[String: String]])
        XCTAssertEqual(history, [["role": "User", "text": "Q1"], ["role": "Assistant", "text": "A1"]])
    }

    func test_stop_discards_exactly_once_and_marks_stopped() async {
        let backend = FakeAskBackend()
        backend.events = [.textDelta(runId: "r", sequence: 1, text: "par")]
        backend.holdOpen = true
        let m = model(backend)
        m.send("q")
        await settle(m) { m.turns.first?.answer == "par" }
        m.stop()
        m.stop()
        await settle(m) { !backend.discards.isEmpty }
        XCTAssertEqual(m.turns[0].state, .stopped)
        XCTAssertEqual(backend.discards, ["p1"])
        XCTAssertEqual(backend.finalizes, [])
    }

    func test_drop_before_any_text_is_failed() async {
        let backend = FakeAskBackend()
        backend.events = [terminal(.failed(classification: "x", message: "boom"), text: "")]
        let m = model(backend)
        m.send("q")
        await settle(m) { m.turns.first?.state != .streaming }
        XCTAssertEqual(m.turns[0].state, .failed)
        XCTAssertEqual(backend.discards, ["p1"])
        XCTAssertEqual(backend.finalizes, [])
    }

    func test_drop_after_partial_answer_is_stopped() async {
        for outcome in [InferenceEvent.TerminalOutcome.failed(classification: "net", message: "lost"), .cancelled] {
            let backend = FakeAskBackend()
            backend.events = [.textDelta(runId: "r", sequence: 1, text: "par"), terminal(outcome, text: "")]
            let m = model(backend)
            m.send("q")
            await settle(m) { m.turns.first?.state != .streaming }
            XCTAssertEqual(m.turns[0].state, .stopped)
            XCTAssertEqual(m.turns[0].answer, "par")
            XCTAssertEqual(backend.discards, ["p1"])
            XCTAssertEqual(backend.finalizes, [])
        }
    }

    func test_stopped_run_cleanup_never_discards_next_turns_prepare() async {
        let backend = FakeAskBackend()
        backend.prepareIds = ["p1", "p2"]
        backend.events = [.textDelta(runId: "r", sequence: 1, text: "a")]
        backend.holdOpen = true
        let m = model(backend)
        m.send("q1")
        await settle(m) { m.turns.first?.answer == "a" }
        m.stop()
        m.send("q2")
        await settle(m) { backend.streams.count == 2 }
        try? await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertEqual(backend.discards, ["p1"])
        m.stop()
        await settle(m) { backend.discards.count == 2 }
        XCTAssertEqual(backend.discards, ["p1", "p2"])
    }

    func test_spend_cap_shows_state_and_never_streams() async {
        let backend = FakeAskBackend()
        backend.prepareResult = .spendCap
        let m = model(backend)
        m.send("q")
        await settle(m) { m.turns.first?.state != .streaming }
        XCTAssertEqual(m.turns[0].state, .spendCap)
        XCTAssertTrue(backend.streams.isEmpty)
        XCTAssertTrue(backend.discards.isEmpty)
    }

    func test_model_line_reads_model_dot_your_key() {
        let backend = FakeAskBackend()
        backend.holdOpen = true
        let m = model(backend)
        m.send("q")
        XCTAssertEqual(m.turns[0].modelLine, "claude-haiku-4-5 · your key")
    }

    func test_send_without_a_route_adds_no_turn() {
        let m = model(FakeAskBackend(), keys: [:])
        m.send("q")
        XCTAssertTrue(m.turns.isEmpty)
    }

    func test_stop_during_prepare_discards_once_and_stays_stopped() async {
        let backend = FakeAskBackend()
        backend.prepareDelay = 50_000_000
        let m = model(backend)
        m.send("q")
        await settle(m) { backend.routingJSONs.count == 1 }
        m.stop()
        await settle(m) { !backend.discards.isEmpty }
        try? await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertEqual(m.turns[0].state, .stopped)
        XCTAssertEqual(backend.discards, ["p1"])
        XCTAssertEqual(backend.finalizes, [])
        XCTAssertTrue(backend.streams.isEmpty)
    }

    func test_stop_during_prepare_then_spend_cap_stays_stopped() async {
        let backend = FakeAskBackend()
        backend.prepareDelay = 50_000_000
        backend.prepareResult = .spendCap
        let m = model(backend)
        m.send("q")
        await settle(m) { backend.routingJSONs.count == 1 }
        m.stop()
        try? await Task.sleep(nanoseconds: 150_000_000)
        XCTAssertEqual(m.turns[0].state, .stopped)
        XCTAssertTrue(backend.discards.isEmpty)
    }

    func test_stream_ending_without_terminal_event_discards_and_fails() async {
        let backend = FakeAskBackend()
        backend.events = [.textDelta(runId: "r", sequence: 1, text: "par")]
        let m = model(backend)
        m.send("q")
        await settle(m) { m.turns.first?.state != .streaming }
        XCTAssertEqual(m.turns[0].state, .failed)
        XCTAssertEqual(backend.discards, ["p1"])
        XCTAssertEqual(backend.finalizes, [])
    }

    func test_stop_with_nothing_streaming_is_safe() {
        let m = model(FakeAskBackend())
        m.stop()
        XCTAssertTrue(m.turns.isEmpty)
    }
}
