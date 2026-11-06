import XCTest
@testable import Whimbrel

final class CoreLLMErrorDecodingTests: XCTestCase {
    private func decode<T: Decodable>(_ json: String, as type: T.Type) -> T? {
        try? JSONDecoder().decode(T.self, from: Data(json.utf8))
    }

    func testDecodesSpendCapUserScope() {
        let json = #"{"kind":"spend_cap","scope":"user","spent_cents":42,"cap_cents":200}"#
        let error = decode(json, as: CoreLLMError.self)
        XCTAssertEqual(error, .spendCap(scope: .user, spentCents: 42, capCents: 200))
    }

    func testDecodesSpendCapGlobalScope() {
        let json = #"{"kind":"spend_cap","scope":"global","spent_cents":5000,"cap_cents":5000}"#
        let error = decode(json, as: CoreLLMError.self)
        XCTAssertEqual(error, .spendCap(scope: .global, spentCents: 5000, capCents: 5000))
    }

    func testNoTranscriptDecodes() {
        let error = decode(#"{"kind":"no_transcript"}"#, as: CoreLLMError.self)
        XCTAssertEqual(error, .noTranscript)
        XCTAssertFalse(error?.userFacingMessage.isEmpty ?? true)
    }

    func testDecodesProviderFailure() {
        let json = #"{"kind":"provider_failure"}"#
        let error = decode(json, as: CoreLLMError.self)
        XCTAssertEqual(error, .providerFailure)
    }

    func testDecodesProviderFailureReasonAndSaysWhy() {
        let missing = decode(#"{"kind":"provider_failure","reason":{"kind":"ollama_model_missing","model":"gemma4:12b"}}"#, as: CoreLLMError.self)
        XCTAssertEqual(missing, .providerFailureBecause(.ollamaModelMissing(model: "gemma4:12b")))
        XCTAssertEqual(missing?.userFacingMessage, "Ollama doesn't have the model gemma4:12b. Pick one you have in Settings → AI Providers, or run: ollama pull gemma4:12b")
        XCTAssertEqual(missing?.opensAIProvidersSettings, true)
        let unauthorized = decode(#"{"kind":"provider_failure","reason":{"kind":"unauthorized","provider":"anthropic"}}"#, as: CoreLLMError.self)
        XCTAssertEqual(unauthorized?.userFacingMessage, "Anthropic rejected the API key. Check it in Settings → AI Providers.")
        let notRunning = decode(#"{"kind":"provider_failure","reason":{"kind":"ollama_not_running"}}"#, as: CoreLLMError.self)
        XCTAssertEqual(notRunning, .providerFailureBecause(.ollamaNotRunning))
        // A reason with no message of its own keeps the generic text.
        let other = decode(#"{"kind":"provider_failure","reason":{"kind":"other","provider":"openai","status":500}}"#, as: CoreLLMError.self)
        XCTAssertEqual(other, .providerFailure)
    }

    func testUnrecognizedKindFallsBackToProviderFailure() {
        let json = #"{"kind":"something_a_future_build_might_add"}"#
        let error = decode(json, as: CoreLLMError.self)
        XCTAssertEqual(error, .providerFailure)
    }

    func testDecodesOkTrueEnvelope() {
        let json = #"{"ok":true,"data":{"text":"hello","citations":[]}}"#
        let envelope = decode(json, as: CoreLLMEnvelope<CoreChatAnswer>.self)
        XCTAssertEqual(envelope?.ok, true)
        XCTAssertEqual(envelope?.data?.text, "hello")
        XCTAssertNil(envelope?.error)
    }

    func testUserFacingMessagesAreDistinctPerCase() {
        let spendCapUser = CoreLLMError.spendCap(scope: .user, spentCents: 42, capCents: 200)
        let spendCapGlobal = CoreLLMError.spendCap(scope: .global, spentCents: 5000, capCents: 5000)
        let providerFailure = CoreLLMError.providerFailure
        XCTAssertTrue(spendCapUser.userFacingMessage.contains("42¢/200¢"))
        XCTAssertTrue(spendCapGlobal.userFacingMessage.contains("5000¢/5000¢"))
        XCTAssertFalse(providerFailure.userFacingMessage.contains("¢"))
    }

    // MARK: - CoreUserMessage (Task 4 codes rendered from the String Catalog)

    private func userMessage(_ json: String) throws -> CoreUserMessage {
        try JSONDecoder().decode(CoreUserMessage.self, from: Data(json.utf8))
    }

    func test_device_limit_code_renders_catalog_text() throws {
        let m = try userMessage(#"{"code":"device_limit","params":{},"message":"English from core"}"#)
        XCTAssertEqual(m.localized(), "Your free sync plan includes one computer and one phone, and they're already in use.")
    }

    func test_unknown_code_falls_back_to_message() throws {
        XCTAssertEqual(try userMessage(#"{"code":"brand_new_code","params":{},"message":"From core"}"#).localized(), "From core")
        XCTAssertEqual(try userMessage(#"{"message":"No code"}"#).localized(), "No code")
    }

    func test_approve_on_other_device_lists_names() throws {
        let m = try userMessage(#"{"code":"approve_on_other_device","params":{"devices":["Mac mini","iPhone"]},"message":"x"}"#)
        XCTAssertEqual(m.localized(locale: Locale(identifier: "en_US")), "Approve this computer on Mac mini or iPhone; it was active in the last week.")
    }

    func test_params_render_and_missing_param_falls_back() throws {
        XCTAssertEqual(try userMessage(#"{"code":"server_error","params":{"status":502},"message":"x"}"#).localized(),
                       "Sync is temporarily unavailable (server error 502).")
        XCTAssertEqual(try userMessage(#"{"code":"key_refused","params":{},"message":"core text"}"#).localized(), "core text")
    }

    /// The automatic summary keeps its failure for the meeting page, as Regenerate shows it.
    func testSummaryFailuresKeepTheLastFailureUntilASuccessOrAClear() {
        let missing = CoreLLMError.providerFailureBecause(.ollamaModelMissing(model: "gemma4:12b"))
        var failures = SummaryFailures()
        failures.record(Result<Int, CoreLLMError>?.some(.failure(.providerFailure)), meetingId: 1)
        failures.record(Result<Int, CoreLLMError>?.some(.failure(missing)), meetingId: 1)
        XCTAssertEqual(failures[1], missing, "a failure replaces the last one")
        failures.record(Result<Int, CoreLLMError>?.none, meetingId: 1)
        XCTAssertEqual(failures[1], missing, "a call that never ran keeps it")
        failures.record(Result<Int, CoreLLMError>?.some(.failure(.providerFailure)), meetingId: 2)
        failures.record(Result<Int, CoreLLMError>?.some(.success(1)), meetingId: 1)
        XCTAssertNil(failures[1], "a success clears it")
        XCTAssertEqual(failures[2], .providerFailure, "per meeting")
        failures.clear(meetingId: 2)
        XCTAssertNil(failures[2], "a provider or speaker-name change, or forgetting the meeting, clears it")
        XCTAssertTrue(missing.opensAIProvidersSettings)
        XCTAssertFalse(CoreLLMError.providerFailureBecause(.rateLimited(provider: "openai")).opensAIProvidersSettings)
    }
}
