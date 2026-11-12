import XCTest
@testable import Whimbrel

final class WindowAskTests: XCTestCase {
    private var dir: URL!
    private var store: CoreMemoryStore!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        store = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("t.sqlite"), modelsDir: nil))
    }

    override func tearDownWithError() throws {
        store = nil
        try? FileManager.default.removeItem(at: dir)
    }

    private func routing(provider: String = "ollama") -> CoreRoutingConfig {
        CoreRoutingConfig(
            anthropicKey: nil, openaiKey: nil, googleKey: nil, typesafeKey: nil,
            userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: false,
            preferredProvider: provider, preferredModel: "gemma4:12b",
            customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: ""
        )
    }

    func testWindowContextEncodesSnakeCase() throws {
        let ctx = WindowContext(appName: "Notes", title: "Plan", text: "hi", isSelection: true)
        let json = try XCTUnwrap(String(data: JSONEncoder().encode(ctx), encoding: .utf8))
        XCTAssertTrue(json.contains("\"app_name\":\"Notes\""), json)
        XCTAssertTrue(json.contains("\"is_selection\":true"), json)
    }

    func testPrepareWindowAskBuildsAPromptWithTheWindowTextForLocalRouting() throws {
        let ctx = WindowContext(appName: "Notes", title: "Plan", text: "ship the zebra on Friday", isSelection: false)
        let result = try XCTUnwrap(store.prepareWindowAsk(ctx, query: "what ships?", history: [], routing: routing()))
        let prepared = try result.get()
        XCTAssertTrue(prepared.prompt.contains("ship the zebra on Friday"))
        XCTAssertFalse(prepared.prepareId.isEmpty)
        XCTAssertFalse(prepared.truncated)
        XCTAssertEqual(prepared.destination, .local)
    }

    func testPrepareWindowAskReportsHostedDestination() throws {
        let ctx = WindowContext(appName: "Notes", title: "Plan", text: "some text", isSelection: false)
        var config = routing(provider: "anthropic")
        config = CoreRoutingConfig(
            anthropicKey: "sk-test", openaiKey: nil, googleKey: nil, typesafeKey: nil,
            userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: false,
            preferredProvider: "anthropic", preferredModel: "m",
            customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: "")
        let prepared = try XCTUnwrap(store.prepareWindowAsk(ctx, query: "q", history: [], routing: config)).get()
        XCTAssertEqual(prepared.destination, .hosted(provider: "anthropic"))
    }

    func testBlankWindowTextIsNoText() throws {
        let ctx = WindowContext(appName: "Notes", title: "Plan", text: "  \n", isSelection: false)
        let result = try XCTUnwrap(store.prepareWindowAsk(ctx, query: "q", history: [], routing: routing()))
        guard case .failure(let failure) = result else { return XCTFail("expected failure") }
        XCTAssertEqual(failure, .noText)
    }

    func testFailureKindsDecode() throws {
        let noText = try JSONDecoder().decode(CoreLLMError.self, from: Data(#"{"kind":"no_text"}"#.utf8))
        XCTAssertEqual(noText, .noText)
        let missing = try JSONDecoder().decode(CoreLLMError.self, from: Data(#"{"kind":"local_model_missing"}"#.utf8))
        XCTAssertEqual(missing, .localModelMissing)
    }
}
