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
}
