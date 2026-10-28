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
}
