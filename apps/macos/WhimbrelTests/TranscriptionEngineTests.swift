import XCTest
@testable import Whimbrel

final class TranscriptionEngineTests: XCTestCase {
    private var defaults: UserDefaults!

    override func setUp() {
        defaults = UserDefaults(suiteName: "TranscriptionEngineTests")!
        defaults.removePersistentDomain(forName: "TranscriptionEngineTests")
    }

    private func clip() throws -> URL {
        try XCTUnwrap(EngineBenchmark.clip)
    }

    func testExistingInstallStaysOnItsModelAndNewInstallGetsAutomatic() {
        TranscriptionQuality.migrate(defaults: defaults, modelExists: true)
        XCTAssertEqual(TranscriptionQuality.current(defaults: defaults), .fast)
        defaults.removePersistentDomain(forName: "TranscriptionEngineTests")
        TranscriptionQuality.migrate(defaults: defaults, modelExists: false)
        XCTAssertEqual(TranscriptionQuality.current(defaults: defaults), .automatic)
    }

    func testMigrationRunsOnceAndKeepsTheChoice() {
        defaults.set("better", forKey: TranscriptionQuality.key)
        TranscriptionQuality.migrate(defaults: defaults, modelExists: true)
        XCTAssertEqual(TranscriptionQuality.current(defaults: defaults), .better)
        // A new install's model landing later doesn't make it existing.
        defaults.set("automatic", forKey: TranscriptionQuality.key)
        TranscriptionQuality.migrate(defaults: defaults, modelExists: true)
        XCTAssertEqual(TranscriptionQuality.current(defaults: defaults), .automatic)
    }

    func testTheOfferShowsOnceAndOnlyWhenItChangesSomething() {
        let offer = { (dismissed: Bool, quality: TranscriptionQuality, custom: Bool, speech: Bool, turbo: Bool) in
            TranscriptionQuality.shouldOffer(dismissed: dismissed, quality: quality, customModel: custom, speechAnalyzer: speech, offerTurbo: turbo)
        }
        XCTAssertTrue(offer(false, .fast, false, true, false))
        XCTAssertTrue(offer(false, .automatic, false, false, true))
        XCTAssertFalse(offer(true, .fast, false, true, false))
        XCTAssertFalse(offer(false, .automatic, false, true, false))
        XCTAssertFalse(offer(false, .fast, true, true, false))
        XCTAssertFalse(offer(false, .fast, false, false, false))
    }

    func testChooserRoundTripsThroughCore() throws {
        let inputs = CoreEngineInputs(quality: .automatic, customModel: false, speechAnalyzer: false, turboPresent: true,
                                      basePresent: true, detecting: false, turboSpeed: 2.5)
        let choice = try XCTUnwrap(CoreMemoryStore.chooseEngine(inputs))
        XCTAssertEqual(choice.batch, .turbo)
        XCTAssertEqual(choice.live, .base)
    }

    func testASpanCoversEveryWordOfItsResult() {
        let span = SpeechAnalyzerEngine.Span(text: " Thanks everyone.", ranges: [(0, 0.3), (0.3, 0.9), (0.9, 1.5)])
        XCTAssertEqual(span?.start, 0)
        XCTAssertEqual(span?.end, 1.5)
        XCTAssertNil(SpeechAnalyzerEngine.Span(text: "no times", ranges: []))
    }

    func testSegmentsTrimOffsetAndDropEmptyText() throws {
        let spans = [
            try XCTUnwrap(SpeechAnalyzerEngine.Span(text: " Hello there. ", ranges: [(1.5, 2.25)])),
            try XCTUnwrap(SpeechAnalyzerEngine.Span(text: "  ", ranges: [(3, 4)]))
        ]
        let segments = SpeechAnalyzerEngine.segments(from: spans, offsetCs: 1000)
        XCTAssertEqual(segments.count, 1)
        XCTAssertEqual(segments[0].text, "Hello there.")
        XCTAssertEqual(segments[0].startCs, 1150)
        XCTAssertEqual(segments[0].endCs, 1225)
    }

    func testLocaleUsesTheRegionOfTheAppLanguage() {
        XCTAssertEqual(SpeechAnalyzerEngine.locale(for: "en", uiTag: "en-GB").identifier, "en-GB")
        XCTAssertEqual(SpeechAnalyzerEngine.locale(for: "es", uiTag: "en-US").identifier, "es")
        XCTAssertEqual(SpeechAnalyzerEngine.locale(for: "fr", uiTag: "fr-CA").identifier, "fr-CA")
    }

    private final class Fake: TranscriptionEngine {
        var batchResult: (segments: [CoreTranscriptSegment], language: String?)?
        var liveResult: String?
        var prepares = true
        var prepared = 0
        var isReady = true
        var needsLanguage = false
        func transcribe(wav: URL, language: String?) -> (segments: [CoreTranscriptSegment], language: String?)? { batchResult }
        func prepareLive(detectLanguage: Bool) -> Bool { prepared += 1; return prepares }
        func transcribe(samples: [Float], startOffsetCs: Int64, threads: Int32) -> String? { liveResult }
    }

    func testAFailedPrimaryFallsBackForABatch() {
        let primary = Fake(), fallback = Fake()
        fallback.batchResult = ([], "en")
        let engine = FallbackEngine(primary: primary, fallback: fallback)
        XCTAssertEqual(engine.transcribe(wav: URL(fileURLWithPath: "/x"), language: "en")?.language, "en")
    }

    func testAFailedLiveChunkMovesToTheFallbackForGood() {
        let primary = Fake(), fallback = Fake()
        fallback.liveResult = "[]"
        let engine = FallbackEngine(primary: primary, fallback: fallback)
        XCTAssertTrue(engine.prepareLive(detectLanguage: false))
        XCTAssertEqual(engine.transcribe(samples: [0], startOffsetCs: 0, threads: 1), "[]")
        primary.liveResult = "primary"
        XCTAssertEqual(engine.transcribe(samples: [0], startOffsetCs: 0, threads: 1), "[]")
        XCTAssertEqual(fallback.prepared, 1)
    }

    func testAFallbackThatCantLoadFailsTheChunk() {
        let primary = Fake(), fallback = Fake()
        fallback.prepares = false
        let engine = FallbackEngine(primary: primary, fallback: fallback)
        _ = engine.prepareLive(detectLanguage: false)
        XCTAssertNil(engine.transcribe(samples: [0], startOffsetCs: 0, threads: 1))
    }

    func testEngineWithNoModelIsNotReady() {
        XCTAssertFalse(WhisperEngine(model: URL(fileURLWithPath: "/missing")).isReady)
    }

    func testBenchmarkClipReadsAsElevenSeconds() throws {
        XCTAssertEqual(EngineBenchmark.samples(in: try clip())?.count, 176_000)
    }

    func testBenchmarkRunsOnceAndNeverWithSpeechAnalyzer() {
        XCTAssertFalse(EngineBenchmark.needed(defaults: defaults, speechAnalyzer: true))
        XCTAssertTrue(EngineBenchmark.needed(defaults: defaults, speechAnalyzer: false))
        defaults.set(3.0, forKey: TranscriptionQuality.turboSpeedKey)
        XCTAssertFalse(EngineBenchmark.needed(defaults: defaults, speechAnalyzer: false))
    }

    func testSamplesStopAtTheLimit() throws {
        XCTAssertEqual(EngineBenchmark.samples(in: try clip(), seconds: 2)?.count, 32_000)
    }

    func testSpeechAnalyzerTranscribesTheClipWhenThisMacHasIt() throws {
        try XCTSkipUnless(SpeechAnalyzerEngine.isAvailable)
        let wav = try clip()
        let done = expectation(description: "transcribed")
        nonisolated(unsafe) var result: (segments: [CoreTranscriptSegment], language: String?)?
        DispatchQueue.global().async {
            result = SpeechAnalyzerEngine().transcribe(wav: wav, language: "en")
            done.fulfill()
        }
        wait(for: [done], timeout: 120)
        try XCTSkipIf(result == nil, "language asset unavailable here")
        let text = result!.segments.map(\.text).joined(separator: " ").lowercased()
        XCTAssertTrue(text.contains("country"), text)
    }

    func testSpeechAnalyzerWithoutALanguageLeavesItToTheFallback() {
        XCTAssertNil(SpeechAnalyzerEngine().transcribe(wav: URL(fileURLWithPath: "/x"), language: nil))
    }
}
