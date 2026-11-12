import AppKit
import XCTest
@testable import Whimbrel

/// Fake reader: each `read` either returns its scripted result at once, or (when `hold` is
/// set) suspends until the test releases it, to model a slow or hung target app.
@MainActor
private final class FakeReader {
    var results: [WindowReadResult] = []
    var hold = false
    private(set) var calls: [HotkeyPanelModel.Frontmost] = []
    private var held: [CheckedContinuation<WindowReadResult, Never>] = []

    func read(_ frontmost: HotkeyPanelModel.Frontmost) async -> WindowReadResult {
        calls.append(frontmost)
        let result = results.isEmpty ? .unreadable(appName: frontmost.name) : results.removeFirst()
        guard hold else { return result }
        return await withCheckedContinuation { held.append($0) }
    }

    /// Resumes the oldest held read with `result`.
    func release(_ result: WindowReadResult) {
        guard !held.isEmpty else { return XCTFail("no held read") }
        held.removeFirst().resume(returning: result)
    }
}

@MainActor
private final class FakeBridge: WindowAskBridge {
    var prepareResult: Result<WindowAskPrepared, CoreLLMError>? = .success(
        WindowAskPrepared(prepareId: "p1", prompt: "PROMPT", truncated: false, destination: .local))
    var streamOutcome: WindowAskStreamOutcome = .completed(text: "The answer.", usage: .init())
    var streamDeltas: [String] = []
    private(set) var prepareCalls: [(window: WindowContext, query: String)] = []
    private(set) var streamCalls: [(prompt: String, model: String, route: AskStreamRoute)] = []
    private(set) var finalized: [String] = []
    private(set) var discarded: [String] = []
    private(set) var cancelCalls = 0

    /// The destination core reports. Nil: what core would say for the routing passed in.
    var preparedDestination: WindowAskDestination?

    func prepare(_ window: WindowContext, query: String, routing: CoreRoutingConfig) async -> Result<WindowAskPrepared, CoreLLMError>? {
        prepareCalls.append((window, query))
        guard case .success(let p)? = prepareResult else { return prepareResult }
        let destination = preparedDestination ?? HotkeyPanelModel.destination(routing)
        return .success(WindowAskPrepared(prepareId: p.prepareId, prompt: p.prompt, truncated: p.truncated, destination: destination))
    }

    func stream(prompt: String, model: String, route: AskStreamRoute,
                onText: @escaping @MainActor (String) -> Void) async -> WindowAskStreamOutcome {
        streamCalls.append((prompt, model, route))
        for delta in streamDeltas { onText(delta) }
        return streamOutcome
    }

    func cancelStream() { cancelCalls += 1 }

    func finalize(prepareId: String, text: String, model: String, usage: InferenceEvent.TokenUsage) async {
        finalized.append(prepareId)
    }

    func discard(prepareId: String) { discarded.append(prepareId) }
}

@MainActor
final class HotkeyPanelModelTests: XCTestCase {
    private let mail: HotkeyPanelModel.Frontmost = ("com.apple.mail", "Mail", 42)
    private var reader: FakeReader!
    private var bridge: FakeBridge!
    private var routing: WindowAskRouting!
    private var pasteboard: NSPasteboard!
    private var beforeAskCalls = 0

    override func setUp() async throws {
        reader = FakeReader()
        bridge = FakeBridge()
        routing = Self.routing(provider: "ollama")
        pasteboard = NSPasteboard(name: NSPasteboard.Name("HotkeyPanelModelTests-\(UUID().uuidString)"))
    }

    override func tearDown() async throws {
        pasteboard.releaseGlobally()
    }

    private static func routing(provider: String, sensitive: Bool = false, localModelAvailable: Bool = true,
                                stream: AskStreamRoute? = AskStreamRoute(provider: .ollama, apiKey: nil, baseURLOverride: nil)) -> WindowAskRouting {
        WindowAskRouting(
            config: CoreRoutingConfig(
                anthropicKey: nil, openaiKey: nil, googleKey: nil, typesafeKey: nil,
                userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: sensitive,
                preferredProvider: provider, preferredModel: "m1",
                customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: ""),
            stream: stream,
            localModelAvailable: localModelAvailable)
    }

    private func makeModel(timeout: Duration = .seconds(3)) -> HotkeyPanelModel {
        let reader = reader!
        return HotkeyPanelModel(
            read: { await reader.read($0) },
            bridge: bridge,
            routing: { [unowned self] in self.routing },
            beforeAsk: { [unowned self] _ in self.beforeAskCalls += 1 },
            ownBundleId: "com.mpointer.whimbrel",
            readTimeout: timeout,
            pasteboard: pasteboard)
    }

    private func selection(_ text: String = "Ship on Friday.") -> WindowReadResult {
        .read(WindowContext(appName: "Mail", title: "Re: Q3 rollout", text: text, isSelection: true, source: .selection))
    }

    private func readyLine(_ model: HotkeyPanelModel, file: StaticString = #filePath, line: UInt = #line) -> HotkeyPanelModel.ContextLine? {
        guard case .ready(let context) = model.state else {
            XCTFail("expected .ready, got \(model.state)", file: file, line: line)
            return nil
        }
        return context
    }

    // MARK: - Context and destination lines

    func testSelectedTextReadGivesTheExactContextLine() async {
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        let line = readyLine(model)
        XCTAssertEqual(line?.reading, "Reading: Mail, 'Re: Q3 rollout' (selected text)")
        XCTAssertEqual(line?.truncated, false)
        XCTAssertEqual(reader.calls.first?.pid, 42)
    }

    func testVisibleTextReadSaysVisibleText() async {
        reader.results = [.read(WindowContext(appName: "Mail", title: "Inbox", text: "Lots of mail", isSelection: false))]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(readyLine(model)?.reading, "Reading: Mail, 'Inbox' (visible text)")
    }

    func testLocalDestinationLine() async {
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(readyLine(model)?.destination, "This will be answered on this computer")
    }

    func testLocalOnlyDestinationLineEvenWithAHostedPreference() async {
        routing = Self.routing(provider: "anthropic", sensitive: true)
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(readyLine(model)?.destination, "This will be answered on this computer")
    }

    func testHostedDestinationLineNamesTheProvider() async {
        let expected = [
            "anthropic": "The text above will be sent to Anthropic",
            "openai": "The text above will be sent to OpenAI",
            "google": "The text above will be sent to Google",
            "openrouter": "The text above will be sent to OpenRouter",
            "whimbrel": "The text above will be sent to Anthropic, through Whimbrel's server",
        ]
        for (provider, text) in expected {
            routing = Self.routing(provider: provider)
            reader.results = [selection()]
            let model = makeModel()
            await model.open(frontmost: mail)
            XCTAssertEqual(readyLine(model)?.destination, text, provider)
        }
    }

    func testLongTextIsMarkedFirstPartOnly() async {
        reader.results = [selection(String(repeating: "a", count: 6_001))]
        let model = makeModel()
        await model.open(frontmost: mail)
        let line = readyLine(model)
        XCTAssertEqual(line?.truncated, true)
        XCTAssertEqual(line?.reading, "Reading: Mail, 'Re: Q3 rollout' (selected text) — first part only")
    }

    func testTextAtTheCapIsNotTruncated() async {
        reader.results = [selection(String(repeating: "a", count: 6_000))]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(readyLine(model)?.truncated, false)
    }

    func testOpeningSendsNothing() async {
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertTrue(bridge.prepareCalls.isEmpty)
        XCTAssertTrue(bridge.streamCalls.isEmpty)
        XCTAssertEqual(beforeAskCalls, 0, "no credential refresh (a network call) before Return")
        await model.ask("q")
        XCTAssertEqual(beforeAskCalls, 1)
    }

    func testADestinationChangeWhileOpenShowsTheNewLineAndSendsNothing() async {
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(readyLine(model)?.destination, "This will be answered on this computer")
        routing = Self.routing(provider: "openai")
        await model.ask("q")
        XCTAssertEqual(readyLine(model)?.destination, "The text above will be sent to OpenAI")
        XCTAssertTrue(bridge.prepareCalls.isEmpty)
        await model.ask("q")
        XCTAssertEqual(bridge.prepareCalls.count, 1)
    }

    func testAnUnknownProviderIsNamedNeverLocal() async {
        routing = Self.routing(provider: "mistral")
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(readyLine(model)?.destination, "The text above will be sent to mistral")
        XCTAssertEqual(HotkeyPanelModel.destination(Self.routing(provider: "local").config), .local)
    }

    func testCoreDisagreeingWithTheShownLineDiscardsAndWaitsForReturn() async {
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(readyLine(model)?.destination, "This will be answered on this computer")
        bridge.preparedDestination = .hosted(provider: "anthropic")
        await model.ask("q")
        XCTAssertEqual(readyLine(model)?.destination, "The text above will be sent to Anthropic")
        XCTAssertEqual(model.context?.destination, "The text above will be sent to Anthropic")
        XCTAssertEqual(bridge.discarded, ["p1"])
        XCTAssertTrue(bridge.streamCalls.isEmpty, "nothing sent")
        XCTAssertTrue(bridge.finalized.isEmpty)
        // The stream is built from the settings route (local here), so while core keeps
        // naming a different destination, even a confirmed Return sends nothing.
        await model.ask("q")
        XCTAssertEqual(bridge.prepareCalls.count, 2)
        XCTAssertTrue(bridge.streamCalls.isEmpty, "never streams to a route core did not name")
        XCTAssertEqual(bridge.discarded, ["p1", "p1"])
        if case .failed = model.state {} else { XCTFail("expected a failure, got \(model.state)") }
    }

    func testCoreAgreeingInDifferentWordsStillAsks() async {
        // Local-only and local render the same line, so the ask goes ahead.
        routing = Self.routing(provider: "ollama", sensitive: true)
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        bridge.preparedDestination = .local
        await model.ask("q")
        XCTAssertEqual(bridge.streamCalls.count, 1)
        XCTAssertTrue(bridge.discarded.isEmpty)
    }

    // MARK: - Blocked states

    func testExcludedAppIsBlockedAndAskSendsNothing() async {
        reader.results = [.excluded(appName: "Mail")]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(model.state, .blocked(.excluded("Mail")))
        XCTAssertEqual(HotkeyPanelModel.Reason.excluded("Mail").message, "This app is excluded from capture.")
        await model.ask("what is this?")
        XCTAssertEqual(model.state, .blocked(.excluded("Mail")))
        XCTAssertTrue(bridge.prepareCalls.isEmpty)
        XCTAssertTrue(bridge.streamCalls.isEmpty)
    }

    func testWhimbrelItselfIsExcludedWithoutReading() async {
        let model = makeModel()
        await model.open(frontmost: ("com.mpointer.whimbrel", "Whimbrel", 7))
        XCTAssertEqual(model.state, .blocked(.excluded("Whimbrel")))
        XCTAssertTrue(reader.calls.isEmpty)
    }

    func testUnreadableAndNotPermittedMessages() async {
        reader.results = [.unreadable(appName: "Mail")]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(model.state, .blocked(.unreadable("Mail")))
        XCTAssertEqual(HotkeyPanelModel.Reason.unreadable("Mail").message, "I couldn't read this window.")

        reader.results = [.notPermitted]
        await model.open(frontmost: mail)
        XCTAssertEqual(model.state, .blocked(.notPermitted))
        XCTAssertEqual(HotkeyPanelModel.Reason.notPermitted.message,
                       "Whimbrel needs Accessibility permission to read this window.")
    }

    func testLocalOnlyWithNoLocalModelIsBlockedBeforeAsking() async {
        routing = Self.routing(provider: "ollama", sensitive: true, localModelAvailable: false)
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        XCTAssertEqual(model.state, .blocked(.localModelMissing))
        XCTAssertEqual(HotkeyPanelModel.Reason.localModelMissing.message, "No local model is installed.")
        await model.ask("q")
        XCTAssertTrue(bridge.prepareCalls.isEmpty)
    }

    func testLocalModelMissingFromCoreIsBlocked() async {
        bridge.prepareResult = .failure(.localModelMissing)
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        await model.ask("q")
        XCTAssertEqual(model.state, .blocked(.localModelMissing))
        XCTAssertTrue(bridge.streamCalls.isEmpty)
    }

    // MARK: - Asking

    func testAskPreparesStreamsFinalizesAndShowsTheAnswer() async {
        bridge.streamDeltas = ["The ", "The answer"]
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        await model.ask("  what ships?  ")
        XCTAssertEqual(model.state, .answered("The answer."))
        XCTAssertEqual(bridge.prepareCalls.first?.query, "what ships?")
        XCTAssertEqual(bridge.prepareCalls.first?.window.text, "Ship on Friday.")
        XCTAssertEqual(bridge.streamCalls.first?.prompt, "PROMPT")
        XCTAssertEqual(bridge.streamCalls.first?.model, "m1")
        XCTAssertEqual(bridge.finalized, ["p1"])
        XCTAssertTrue(bridge.discarded.isEmpty)
        // The context line stays visible with the answer.
        XCTAssertEqual(model.context?.reading, "Reading: Mail, 'Re: Q3 rollout' (selected text)")
    }

    func testBlankQuestionSendsNothing() async {
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        await model.ask("   ")
        XCTAssertTrue(bridge.prepareCalls.isEmpty)
        XCTAssertNotNil(readyLine(model))
    }

    func testStreamFailureDiscardsThePreparedAsk() async {
        bridge.streamOutcome = .failed("Couldn't reach the AI provider. Try again.")
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        await model.ask("q")
        XCTAssertEqual(model.state, .failed("Couldn't reach the AI provider. Try again."))
        XCTAssertEqual(bridge.discarded, ["p1"])
        XCTAssertTrue(bridge.finalized.isEmpty)
    }

    func testNoStreamRouteFailsWithoutSendingAndDiscards() async {
        routing = Self.routing(provider: "anthropic", stream: nil)
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        await model.ask("q")
        guard case .failed = model.state else { return XCTFail("expected .failed, got \(model.state)") }
        XCTAssertTrue(bridge.streamCalls.isEmpty)
        XCTAssertEqual(bridge.discarded, ["p1"])
    }

    func testPrepareFailureShowsItsMessage() async {
        bridge.prepareResult = .failure(.providerFailureBecause(.noProviderConfigured))
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        await model.ask("q")
        XCTAssertEqual(model.state, .failed(CoreLLMError.providerFailureBecause(.noProviderConfigured).userFacingMessage))
    }

    func testCopyAnswerPutsItOnThePasteboard() async {
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        await model.ask("q")
        model.copyAnswer()
        XCTAssertEqual(pasteboard.string(forType: .string), "The answer.")
    }

    func testCloseDiscardsTheAnswerAndAnUnfinalizedAsk() async {
        reader.results = [selection()]
        let model = makeModel()
        await model.open(frontmost: mail)
        await model.ask("q")
        model.close()
        XCTAssertEqual(model.state, .reading)
        XCTAssertNil(model.context)
        XCTAssertEqual(model.partialAnswer, "")
    }

    // MARK: - Read deadline and stale results

    func testAHungReadTimesOutAsUnreadable() async {
        reader.hold = true
        let model = makeModel(timeout: .milliseconds(50))
        let start = ContinuousClock.now
        await model.open(frontmost: mail)
        XCTAssertLessThan(ContinuousClock.now - start, .seconds(2))
        XCTAssertEqual(model.state, .blocked(.unreadable("Mail")))
        // The read finally returns: the timed-out result must not replace the state.
        reader.release(selection())
        await Task.yield()
        await Task.yield()
        XCTAssertEqual(model.state, .blocked(.unreadable("Mail")))
    }

    func testALateResultFromAnEarlierOpenNeverOverwritesTheCurrentOne() async {
        reader.hold = true
        let model = makeModel()
        let first = Task { await model.open(frontmost: self.mail) }
        while reader.calls.isEmpty { await Task.yield() }
        XCTAssertEqual(model.state, .reading)

        // Second open (the user pressed the hotkey again in another app); its read is quick.
        reader.hold = false
        reader.results = [.read(WindowContext(appName: "Notes", title: "Plan", text: "notes", isSelection: false))]
        await model.open(frontmost: ("com.apple.Notes", "Notes", 9))
        XCTAssertEqual(readyLine(model)?.reading, "Reading: Notes, 'Plan' (visible text)")

        // Now the first, slow read finishes.
        reader.release(.excluded(appName: "Mail"))
        await first.value
        XCTAssertEqual(readyLine(model)?.reading, "Reading: Notes, 'Plan' (visible text)")
    }

    func testAPrepareThatReturnsAfterCloseIsDiscarded() async {
        // Close lands while prepare is in flight: model a prepare whose result arrives late.
        let gate = AsyncGate()
        let lateBridge = GatedBridge(gate: gate)
        let gated = HotkeyPanelModel(
            read: { [reader] in await reader!.read($0) }, bridge: lateBridge,
            routing: { [unowned self] in self.routing }, ownBundleId: "x", pasteboard: pasteboard)
        reader.results = [selection()]
        await gated.open(frontmost: mail)
        let asking = Task { await gated.ask("q") }
        while !lateBridge.prepareStarted { await Task.yield() }
        gated.close()
        gate.open()
        await asking.value
        XCTAssertEqual(lateBridge.discarded, ["late"])
        XCTAssertTrue(lateBridge.streamed == false)
        XCTAssertEqual(gated.state, .reading)
    }

    func testClosingDuringTheStreamCancelsAndDiscardsAndNeverFinalizes() async {
        let gate = AsyncGate()
        let lateBridge = GatedBridge(gate: gate)
        lateBridge.holdStream = true
        let gated = HotkeyPanelModel(
            read: { [reader] in await reader!.read($0) }, bridge: lateBridge,
            routing: { [unowned self] in self.routing }, ownBundleId: "x", pasteboard: pasteboard)
        reader.results = [selection()]
        await gated.open(frontmost: mail)
        let asking = Task { await gated.ask("q") }
        while !lateBridge.streamed { await Task.yield() }
        XCTAssertEqual(gated.state, .answering)
        gated.close()
        XCTAssertEqual(lateBridge.cancelled, 1)
        XCTAssertEqual(lateBridge.discarded, ["late"])
        gate.open()
        await asking.value
        XCTAssertTrue(lateBridge.finalized.isEmpty)
        XCTAssertEqual(gated.state, .reading)
    }
}

@MainActor
private final class AsyncGate {
    private var waiters: [CheckedContinuation<Void, Never>] = []
    private var isOpen = false
    func wait() async {
        if isOpen { return }
        await withCheckedContinuation { waiters.append($0) }
    }
    func open() {
        isOpen = true
        waiters.forEach { $0.resume() }
        waiters = []
    }
}

@MainActor
private final class GatedBridge: WindowAskBridge {
    let gate: AsyncGate
    /// When true the gate holds the stream instead of the prepare.
    var holdStream = false
    private(set) var prepareStarted = false
    private(set) var streamed = false
    private(set) var cancelled = 0
    private(set) var finalized: [String] = []
    private(set) var discarded: [String] = []
    init(gate: AsyncGate) { self.gate = gate }

    func prepare(_ window: WindowContext, query: String, routing: CoreRoutingConfig) async -> Result<WindowAskPrepared, CoreLLMError>? {
        prepareStarted = true
        if !holdStream { await gate.wait() }
        return .success(WindowAskPrepared(prepareId: "late", prompt: "P", truncated: false, destination: .local))
    }
    func stream(prompt: String, model: String, route: AskStreamRoute,
                onText: @escaping @MainActor (String) -> Void) async -> WindowAskStreamOutcome {
        streamed = true
        if holdStream { await gate.wait() }
        return .completed(text: "x", usage: .init())
    }
    func cancelStream() { cancelled += 1 }
    func finalize(prepareId: String, text: String, model: String, usage: InferenceEvent.TokenUsage) async { finalized.append(prepareId) }
    func discard(prepareId: String) { discarded.append(prepareId) }
}

@MainActor
final class FrontmostAppTrackerTests: XCTestCase {
    private let mail: HotkeyPanelModel.Frontmost = ("com.apple.mail", "Mail", 42)
    private let own: HotkeyPanelModel.Frontmost = ("com.mpointer.kestrel", "Whimbrel", 7)

    func testMenuThatActivatedWhimbrelGivesThePreviousApp() {
        let tracker = FrontmostAppTracker()
        let now = Date()
        tracker.record(mail, at: now.addingTimeInterval(-60))
        tracker.record(own, at: now.addingTimeInterval(-0.2))
        XCTAssertEqual(tracker.appBeforeMenu(openedAt: now, ownBundleId: own.bundleId)?.bundleId, "com.apple.mail")
    }

    func testWhimbrelAlreadyInFrontIsReportedAsItself() {
        let tracker = FrontmostAppTracker()
        let now = Date()
        tracker.record(mail, at: now.addingTimeInterval(-120))
        tracker.record(own, at: now.addingTimeInterval(-30))
        XCTAssertEqual(tracker.appBeforeMenu(openedAt: now, ownBundleId: own.bundleId)?.bundleId, own.bundleId)
    }

    func testMenuThatDidNotActivateWhimbrelGivesTheFrontmostApp() {
        let tracker = FrontmostAppTracker()
        let now = Date()
        tracker.record(own, at: now.addingTimeInterval(-120))
        tracker.record(mail, at: now.addingTimeInterval(-30))
        XCTAssertEqual(tracker.appBeforeMenu(openedAt: now, ownBundleId: own.bundleId)?.bundleId, "com.apple.mail")
    }
}

@MainActor
final class HotkeyPanelControllerTests: XCTestCase {
    func testPanelOpensKeyableFloatingAndCloseHidesAndResets() async {
        let model = HotkeyPanelModel(
            read: { _ in .unreadable(appName: "Mail") }, bridge: FakeBridge(),
            routing: {
                WindowAskRouting(config: CoreRoutingConfig(
                    anthropicKey: nil, openaiKey: nil, googleKey: nil, typesafeKey: nil,
                    userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: false,
                    preferredProvider: "ollama", preferredModel: "m",
                    customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: ""),
                    stream: nil, localModelAvailable: true)
            }, ownBundleId: "x")
        let controller = HotkeyPanelController(model: model)
        controller.toggle(frontmost: ("com.apple.mail", "Mail", 42))
        XCTAssertTrue(controller.isVisible)
        let panel = try? XCTUnwrap(controller.panelForTesting)
        XCTAssertEqual(panel?.canBecomeKey, true)
        XCTAssertEqual(panel?.level, .floating)
        XCTAssertEqual(panel?.collectionBehavior.contains(.fullScreenAuxiliary), true)
        while model.state == .reading { await Task.yield() }
        XCTAssertEqual(model.state, .blocked(.unreadable("Mail")))
        controller.toggle(frontmost: nil)  // the hotkey again
        XCTAssertFalse(controller.isVisible)
        XCTAssertEqual(model.state, .reading)
    }
}
