import XCTest
@testable import Whimbrel

final class ConversationStoreTests: XCTestCase {
    private func makeStore() -> CoreMemoryStore {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return CoreMemoryStore(path: dir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    func testCreateAppendListGetRoundTrip() {
        let store = makeStore()
        guard let conversationId = store.createConversation(firstQuery: "What's on my calendar this week?") else {
            return XCTFail("createConversation returned nil")
        }
        XCTAssertTrue(store.appendConversationExchange(
            conversationId: conversationId, query: "What's on my calendar this week?",
            answer: "You have 3 meetings.", citationsJson: "[]", model: "llama3"
        ))
        let list = store.listConversations()
        XCTAssertEqual(list?.count, 1)
        XCTAssertEqual(list?.first?.id, conversationId)

        let detail = store.getConversation(id: conversationId)
        XCTAssertEqual(detail?.exchanges.count, 1)
        XCTAssertEqual(detail?.exchanges.first?.answer, "You have 3 meetings.")
        XCTAssertEqual(detail?.exchanges.first?.model, "llama3")
    }

    func testSavedCitationRoundTripsThroughTheStore() throws {
        let store = makeStore()
        let conversationId = try XCTUnwrap(store.createConversation(firstQuery: "q"))
        let citation = SavedCitation(
            item: CoreContextItemRef(kind: .transcript, id: 4), label: "Standup · Speaker 1",
            text: "meeting \"Standup\", Speaker 1: ship it", open: .init(kind: "meeting", id: 9)
        )
        let json = try XCTUnwrap(String(data: JSONEncoder().encode([citation]), encoding: .utf8))
        XCTAssertTrue(store.appendConversationExchange(conversationId: conversationId, query: "q", answer: "a", citationsJson: json, model: nil))

        let row = try XCTUnwrap(store.getConversation(id: conversationId)?.exchanges.first)
        let saved = try JSONDecoder().decode([SavedCitation].self, from: Data(row.citationsJson.utf8))
        XCTAssertEqual(saved, [citation])
        XCTAssertNil(row.model)
        let item = ContextDisplayItem.saved(saved[0])
        XCTAssertEqual(item.detailWindowId, "meeting-detail")
        XCTAssertEqual(item.detailWindowValue, 9)
        XCTAssertTrue(item.matchesRef(CoreContextItemRef(kind: .transcript, id: 4)))
    }

    func testRenameStarDelete() {
        let store = makeStore()
        guard let conversationId = store.createConversation(firstQuery: "q") else {
            return XCTFail("createConversation returned nil")
        }
        XCTAssertTrue(store.renameConversation(id: conversationId, newTitle: "Renamed"))
        XCTAssertTrue(store.setConversationStarred(id: conversationId, starred: true))
        XCTAssertEqual(store.listConversations()?.first?.title, "Renamed")
        XCTAssertEqual(store.listConversations()?.first?.starred, true)
        XCTAssertTrue(store.deleteConversation(id: conversationId))
        XCTAssertNil(store.getConversation(id: conversationId))
    }

    /// FR-CHAT-6 design spec: "a stub failure leaves the placeholder title in place."
    /// A garbage Ollama model name against a routing config with no hosted keys
    /// deterministically fails the provider chain -- whether or not an Ollama daemon
    /// happens to be running on the test machine (connection-refused or
    /// model-not-found are both fast, real failures, no live/successful provider
    /// needed) -- without any mock/protocol seam.
    func testGenerateConversationTitleFailureLeavesPlaceholderTitle() {
        let store = makeStore()
        guard let conversationId = store.createConversation(firstQuery: "What's on my calendar this week?") else {
            return XCTFail("createConversation returned nil")
        }
        let placeholderTitle = store.getConversation(id: conversationId)?.title
        XCTAssertNotNil(placeholderTitle)

        let routing = CoreRoutingConfig(
            anthropicKey: nil,
            openaiKey: nil,
            googleKey: nil,
            typesafeKey: nil,
            userDailyCapCents: nil,
            globalDailyCapCents: nil,
            sensitive: false,
            preferredProvider: "ollama",
            preferredModel: "no-such-model",
            customInstructions: "",
            dailyDigestInstructions: "",
            weeklyDigestInstructions: ""
        )
        let generated = store.generateConversationTitle(
            model: "no-such-model-\(UUID().uuidString)",
            query: "What's on my calendar this week?",
            answer: "You have 3 meetings.",
            routing: routing
        )
        XCTAssertNil(generated, "a nonexistent Ollama model must fail the provider chain, not silently succeed")

        // Mirrors TimelineView.generateConversationTitle's own `guard let generated =
        // ... else { return }` -- on a nil result the caller never calls
        // setConversationTitleIfPlaceholder, so the placeholder must be untouched.
        XCTAssertEqual(store.getConversation(id: conversationId)?.title, placeholderTitle)
    }
}
