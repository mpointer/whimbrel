import XCTest
@testable import Whimbrel

/// Smoke tests for the CoreBridge wrappers over core/transcribe::postprocess. The
/// algorithm's cases (bleed filter, same-speaker merge, turn parsing) are tested in
/// Rust (`core/transcribe/src/postprocess.rs`); these only prove the Swift JSON
/// keys match the Rust ones in both directions.
final class TranscriptBridgeTests: XCTestCase {
    private typealias Line = TranscriptionService.Line

    func testPostprocessDropsBleedAndMergesThroughTheBridge() {
        let system = [
            Line(speakerLabel: "Speaker 1", startCs: 100, text: "So it is something the company started"),
            Line(speakerLabel: "Speaker 1", startCs: 150, text: "around COVID and it continues today"),
        ]
        let mic = [Line(speakerLabel: "Me", startCs: 152, text: "around COVID and it continues today")]
        guard let result = CoreMemoryStore.postprocessTranscript(micLines: mic, systemLines: system) else {
            return XCTFail("bridge returned nil")
        }
        XCTAssertEqual(result.micLinesKept, 0)
        XCTAssertEqual(result.lines, [
            Line(speakerLabel: "Speaker 1", startCs: 100, text: "So it is something the company started around COVID and it continues today"),
        ])
    }

    func testPostprocessWithNoSystemLinesMergesMicOnly() {
        let mic = [
            Line(speakerLabel: "Me", startCs: 100, text: "Hello"),
            Line(speakerLabel: "Me", startCs: 150, text: "there"),
        ]
        let result = CoreMemoryStore.postprocessTranscript(micLines: mic, systemLines: [])
        XCTAssertEqual(result?.lines, [Line(speakerLabel: "Me", startCs: 100, text: "Hello there")])
        XCTAssertEqual(result?.micLinesKept, 2)
    }

    func testParseTurnsThroughTheBridge() {
        let raw = """
        [09:02:19] Sarah: Legal's still reviewing the new terms.
        [09:02:20] Sarah: Should have an answer by Thursday.
        [09:05:00] Mike: He said: let's go with option B.
        """
        XCTAssertEqual(CoreMemoryStore.parseTranscriptTurns(from: raw), [
            TranscriptTurn(speaker: "Sarah", timestamp: "09:02:19", text: "Legal's still reviewing the new terms. Should have an answer by Thursday."),
            TranscriptTurn(speaker: "Mike", timestamp: "09:05:00", text: "He said: let's go with option B."),
        ])
        XCTAssertEqual(CoreMemoryStore.parseTranscriptTurns(from: "[  123.4s] Others: legacy format"), [])
    }
}
