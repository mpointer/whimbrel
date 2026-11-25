import XCTest
@testable import Whimbrel

/// Smoke tests for the CoreBridge wrappers over core/api::prompts and
/// core/api::hosted_models; the cases themselves are Rust tests.
final class AskHelpersBridgeTests: XCTestCase {
    func testStarterPromptsDecodeThroughTheBridge() {
        let prompts = CoreMemoryStore.starterPrompts()
        XCTAssertEqual(prompts.count, 6)
        XCTAssertEqual(prompts.first?.id, "quietly-slipping")
        XCTAssertEqual(prompts.first?.title, "What's quietly slipping?")
        XCTAssertTrue(prompts.first?.query.hasPrefix("What's quietly slipping? Look across") ?? false)
    }

    func testUrgencyThroughTheBridge() {
        XCTAssertFalse(CoreMemoryStore.isTimeCritical("Can you summarize my meetings this week?"))
        XCTAssertTrue(CoreMemoryStore.isTimeCritical("what time is my next meeting?"))
        XCTAssertTrue(CoreMemoryStore.isTimeCritical(""))
    }

    func testHostedModelsFetchSurfacesADisplayReadyErrorForAnUnreachableServer() {
        switch CoreMemoryStore.fetchHostedModels(provider: "ollama", apiKey: nil, baseURLOverride: "http://127.0.0.1:1") {
        case .success: XCTFail("nothing listens on port 1")
        case .failure(let error): XCTAssertEqual(error.message, "Ollama model list request failed (status -1).")
        }
    }

    /// An answer citing an item a local MCP client added decodes (one unknown kind would fail the whole answer).
    func testAnAnswerCitingAnAddedItemDecodes() throws {
        let json = #"{"text":"Ana sent it [1].","citations":[{"kind":"mcp_item","id":7},{"kind":"capture","id":2}]}"#
        let answer = try JSONDecoder().decode(CoreChatAnswer.self, from: Data(json.utf8))
        XCTAssertEqual(answer.citations, [CoreContextItemRef(kind: .mcpItem, id: 7), CoreContextItemRef(kind: .capture, id: 2)])
    }

    func testDecodesSnapshotCitation() throws {
        let ref = try JSONDecoder().decode(CoreContextItemRef.self, from: Data(#"{"kind":"snapshot","id":7}"#.utf8))
        XCTAssertEqual(ref, CoreContextItemRef(kind: .snapshot, id: 7))
    }

    /// A slide's source chip shows its image; other sources keep their icon.
    @MainActor
    func testOnlySnapshotChipsCarryAThumbnail() {
        let hit = CoreMeetingSnapshotHit(id: 7, meetingId: 1, meetingTitle: "Standup", text: "Roadmap", filePath: "/tmp/09.00.00.png")
        XCTAssertEqual(ContextDisplayItem.snapshot(hit).thumbnailPath, "/tmp/09.00.00.png")
        let saved = SavedCitation(item: CoreContextItemRef(kind: .snapshot, id: 7), label: "slide", text: "Roadmap", open: nil)
        XCTAssertNil(ContextDisplayItem.saved(saved).thumbnailPath)
    }

    // MARK: Ask label for a question rerouted for an excluded client

    func testAnsweredLabelIsTheModelCoreSaysAnswered() {
        let local = CoreAnsweredBy(provider: "ollama", model: "gemma4:12b", local: true)
        XCTAssertEqual(answeredModelLabel(requested: "gemini-flash-lite-latest", answeredBy: local), "gemma4:12b")
        XCTAssertEqual(answeredModelLabel(requested: "gemini-flash-lite-latest", answeredBy: nil), "gemini-flash-lite-latest")
        let blank = CoreAnsweredBy(provider: "ollama", model: "  ", local: true)
        XCTAssertEqual(answeredModelLabel(requested: "picked", answeredBy: blank), "picked")
    }

    func testDetailedAnswerDecodesTheAnsweringModelAndExclusion() throws {
        let json = #"{"text":"Hi","citations":[],"token_usage":{},"exclusion":{"kind":"local_only"},"answered_by":{"provider":"ollama","model":"gemma4:12b","local":true}}"#
        let d = try JSONDecoder().decode(CoreDetailedAnswer.self, from: Data(json.utf8))
        XCTAssertEqual(d.chatAnswer.text, "Hi")
        XCTAssertEqual(d.exclusion, .localOnly)
        XCTAssertEqual(d.answeredBy, CoreAnsweredBy(provider: "ollama", model: "gemma4:12b", local: true))
        // A core that does not report them still decodes.
        let bare = try JSONDecoder().decode(CoreDetailedAnswer.self, from: Data(#"{"text":"Hi","citations":[]}"#.utf8))
        XCTAssertNil(bare.answeredBy)
        XCTAssertNil(bare.exclusion)
    }

    func testAnsweredLocallyNoteNeedsANamedClientAndALocalAnswer() {
        XCTAssertTrue(answeredLocallyForExclusion(namesExcludedClient: true, outcome: .localOnly))
        XCTAssertFalse(answeredLocallyForExclusion(namesExcludedClient: false, outcome: .localOnly))
        XCTAssertFalse(answeredLocallyForExclusion(namesExcludedClient: true, outcome: .allowed))
        XCTAssertFalse(answeredLocallyForExclusion(namesExcludedClient: true, outcome: nil))
    }

    func testSpinnerTitleSaysLocalWhenRoutedLocallyAndNeverNamesAClient() {
        XCTAssertEqual(AskLocalRouteCopy.spinnerTitle(routedLocally: false, modelConfirmed: true, modelLabel: "gemini-flash-lite-latest"), "Asking gemini-flash-lite-latest…")
        let local = AskLocalRouteCopy.spinnerTitle(routedLocally: true, modelConfirmed: false, modelLabel: "gemini-flash-lite-latest")
        XCTAssertTrue(local.hasPrefix("Answering on this Mac"))
        XCTAssertFalse(local.contains("gemini"))
        XCTAssertTrue(AskLocalRouteCopy.answered.hasPrefix("Answered locally"))
    }

    func testSpinnerNamesNoModelUntilTheQuestionIsKnownNotToBeRoutedLocally() {
        let before = AskLocalRouteCopy.spinnerTitle(routedLocally: false, modelConfirmed: false, modelLabel: "gemini-flash-lite-latest")
        XCTAssertEqual(before, "Asking…")
        XCTAssertFalse(before.contains("gemini"))
        let waiting = AskLocalRouteCopy.spinnerTitle(routedLocally: false, waitingForLane: true, modelConfirmed: false, modelLabel: "gemini-flash-lite-latest")
        XCTAssertEqual(waiting, "Waiting for background work…")
        XCTAssertFalse(waiting.contains("gemini"))
        // The local note wins: it says where the answer is made.
        XCTAssertTrue(AskLocalRouteCopy.spinnerTitle(routedLocally: true, waitingForLane: true, modelConfirmed: false, modelLabel: "m").hasPrefix("Answering on this Mac"))
    }

    func testUserModelGateCountsQueuedAndRunningRequests() {
        let gate = UserModelGate()
        let now = Date()
        XCTAssertFalse(gate.isBusy(now: now))
        let a = gate.enter(now: now)
        let b = gate.enter(now: now)
        XCTAssertTrue(gate.isBusy(now: now))
        gate.leave(a)
        XCTAssertTrue(gate.isBusy(now: now), "one request is still waiting")
        gate.leave(a)
        gate.leave(nil)
        XCTAssertTrue(gate.isBusy(now: now), "a repeated leave frees nothing else")
        gate.leave(b)
        XCTAssertFalse(gate.isBusy(now: now))
    }

    func testUserModelGateIgnoresAStaleEntry() {
        let gate = UserModelGate()
        let start = Date()
        _ = gate.enter(now: start)
        XCTAssertTrue(gate.isBusy(now: start.addingTimeInterval(UserModelGate.maxHold - 1)))
        XCTAssertFalse(gate.isBusy(now: start.addingTimeInterval(UserModelGate.maxHold + 1)), "a view that vanished must not stall background work for good")
    }

    func testBackgroundModelJobWaitsWhileAUserRequestIsBusy() {
        XCTAssertEqual(BackgroundModelLane.start(userBusy: false), .run)
        XCTAssertEqual(BackgroundModelLane.start(userBusy: true), .retryLater(seconds: UserModelGate.retryDelay))
    }

    func testBackgroundModelLaneFallsBackToTheSlowLaneOnlyWithoutItsOwnConnection() {
        XCTAssertTrue(BackgroundModelLane.choose(dedicatedOpened: true).queue === CoreMemoryStore.backgroundModelQueue)
        XCTAssertTrue(BackgroundModelLane.choose(dedicatedOpened: false).queue === CoreMemoryStore.slowLaneQueue)
        XCTAssertFalse(CoreMemoryStore.backgroundModelQueue === CoreMemoryStore.slowLaneQueue)
    }

    func testLaneStartFlagIsSetOnce() {
        let flag = LaneStartFlag()
        XCTAssertFalse(flag.isSet)
        flag.set()
        XCTAssertTrue(flag.isSet)
    }
}
