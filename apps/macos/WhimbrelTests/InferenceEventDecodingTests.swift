import XCTest
@testable import Whimbrel

/// Guards the Rust/Swift JSON contract for `llm::streaming::InferenceEvent`
/// (core/llm/src/streaming.rs, docs/adr/0002) — each literal below is the exact shape
/// serde's `#[serde(tag = "kind", rename_all = "snake_case")]` derive produces. A
/// silent drift here (a renamed Rust field, a changed tag) would otherwise only be
/// caught by a human eyeballing a broken Streaming Preview at runtime.
final class InferenceEventDecodingTests: XCTestCase {
    private func decode(_ json: String) -> InferenceEvent? {
        try? JSONDecoder().decode(InferenceEvent.self, from: Data(json.utf8))
    }

    func testDecodesStateEvent() {
        let json = #"{"kind":"state","run_id":"run-1","state":"streaming","at_unix_ms":1000,"provider":"ollama","model":"gemma4","attempt":1}"#
        XCTAssertEqual(
            decode(json),
            .state(runId: "run-1", state: .streaming, atUnixMs: 1000, provider: "ollama", model: "gemma4", attempt: 1)
        )
    }

    func testDecodesStateEventWithNullProviderAndModel() {
        let json = #"{"kind":"state","run_id":"run-1","state":"connecting","at_unix_ms":1000,"provider":null,"model":null,"attempt":1}"#
        XCTAssertEqual(
            decode(json),
            .state(runId: "run-1", state: .connecting, atUnixMs: 1000, provider: nil, model: nil, attempt: 1)
        )
    }

    func testDecodesTextDeltaEvent() {
        let json = #"{"kind":"text_delta","run_id":"run-1","sequence":3,"text":"hello"}"#
        XCTAssertEqual(decode(json), .textDelta(runId: "run-1", sequence: 3, text: "hello"))
    }

    func testDecodesProgressEvent() {
        let json = #"{"kind":"progress","run_id":"run-1","label":"warming up"}"#
        XCTAssertEqual(decode(json), .progress(runId: "run-1", label: "warming up"))
    }

    func testDecodesCompletedTerminalEvent() {
        let json = #"{"kind":"terminal","run_id":"run-1","outcome":{"kind":"completed"},"at_unix_ms":2000,"final_text":"hi there","usage":{"input_tokens":5,"output_tokens":3}}"#
        XCTAssertEqual(
            decode(json),
            .terminal(runId: "run-1", outcome: .completed, atUnixMs: 2000, finalText: "hi there", usage: .init(inputTokens: 5, outputTokens: 3))
        )
    }

    func testDecodesCancelledTerminalEventWithEmptyText() {
        let json = #"{"kind":"terminal","run_id":"run-1","outcome":{"kind":"cancelled"},"at_unix_ms":2000,"final_text":"","usage":{"input_tokens":null,"output_tokens":null}}"#
        XCTAssertEqual(
            decode(json),
            .terminal(runId: "run-1", outcome: .cancelled, atUnixMs: 2000, finalText: "", usage: .init())
        )
    }

    func testDecodesFailedTerminalEventWithClassAndMessage() {
        let json = #"{"kind":"terminal","run_id":"run-1","outcome":{"kind":"failed","class":"network","message":"connection refused"},"at_unix_ms":2000,"final_text":"partial","usage":{"input_tokens":null,"output_tokens":null}}"#
        XCTAssertEqual(
            decode(json),
            .terminal(runId: "run-1", outcome: .failed(classification: "network", message: "connection refused"), atUnixMs: 2000, finalText: "partial", usage: .init())
        )
    }

    /// Mirrors `CoreLLMError`'s own forward-compat convention (see
    /// `CoreLLMErrorDecodingTests.testUnrecognizedKindFallsBackToProviderFailure`): a
    /// terminal outcome kind this build doesn't recognize yet must not crash decoding.
    func testUnrecognizedTerminalOutcomeKindFallsBackToFailed() {
        let json = #"{"kind":"terminal","run_id":"run-1","outcome":{"kind":"some_future_outcome"},"at_unix_ms":2000,"final_text":"","usage":{"input_tokens":null,"output_tokens":null}}"#
        guard case .terminal(_, let outcome, _, _, _) = decode(json) else {
            return XCTFail("expected a decoded terminal event")
        }
        guard case .failed = outcome else {
            return XCTFail("expected an unrecognized outcome kind to fall back to .failed")
        }
    }

    func testRunIdAccessorReturnsTheCorrectRunIdForEveryVariant() {
        XCTAssertEqual(
            InferenceEvent.state(runId: "a", state: .queued, atUnixMs: 0, provider: nil, model: nil, attempt: 1).runId, "a"
        )
        XCTAssertEqual(InferenceEvent.textDelta(runId: "b", sequence: 1, text: "x").runId, "b")
        XCTAssertEqual(InferenceEvent.progress(runId: "c", label: "x").runId, "c")
        XCTAssertEqual(
            InferenceEvent.terminal(runId: "d", outcome: .completed, atUnixMs: 0, finalText: "", usage: .init()).runId, "d"
        )
    }
}
