import XCTest
@testable import Whimbrel

final class LiveTranscriptLogicTests: XCTestCase {
    /// A skin switch rebuilds the window (`ThemedRoot`'s `.id`), which recreates every view's state and
    /// used to cancel a live Ask still streaming. The Ask lives on the service now: an answer that
    /// lands while no bar is attached still finalizes, and the rebuilt bar finds it; the meeting
    /// ending (`stop()`) is what tears it down.
    @MainActor
    func testLiveAskSurvivesAViewRebuildAndEndsWithTheMeeting() {
        let live = LiveTranscriptionService(store: { nil }, modelPath: { "" }, systemURL: { nil }, micURL: { nil })
        let session = live.askSession
        session.exchanges = [MeetingAskExchange(question: "What did we decide?")]
        session.pending = ("prep-1", 0, "model")
        session.streaming.apply(.textDelta(runId: "r1", sequence: 0, text: "Ship"))

        // The old bar is gone and no new one is attached yet; the run finishes anyway.
        session.streaming.apply(.terminal(runId: "r1", outcome: .completed, atUnixMs: 0, finalText: "Ship it.", usage: .init()))
        XCTAssertTrue(live.askSession === session, "the rebuilt bar attaches to the same session")
        XCTAssertEqual(session.exchanges.map(\.answer), ["Ship it."])
        XCTAssertNil(session.pending)
        XCTAssertFalse(session.isBusy)

        // A second question still in flight when the meeting ends is dropped with it.
        session.exchanges.append(MeetingAskExchange(question: "Who owns it?"))
        session.pending = ("prep-2", 1, "model")
        live.stop()
        XCTAssertNil(session.pending)
        XCTAssertTrue(session.exchanges.isEmpty)
    }

    func testAppendKeepsAbsoluteIndexesAndTakeAdvancesThem() {
        var buffer = LiveChannelBuffer()
        buffer.append(Array(repeating: 0.1, count: 1_000))
        XCTAssertEqual(buffer.startSample, 0)
        let taken = buffer.take(count: 400, keepOverlap: 0)
        XCTAssertEqual(taken.samples.count, 400)
        XCTAssertEqual(taken.startSample, 0)
        XCTAssertEqual(buffer.startSample, 400)
        XCTAssertEqual(buffer.samples.count, 600)
    }

    func testTakeWithOverlapKeepsTheTailForTheNextChunk() {
        var buffer = LiveChannelBuffer()
        buffer.append((0..<1_000).map { Float($0) })
        let taken = buffer.take(count: 800, keepOverlap: 100)
        XCTAssertEqual(taken.samples.count, 800)
        XCTAssertEqual(buffer.startSample, 700, "the last 100 samples stay at the front")
        XCTAssertEqual(buffer.samples.first, 700)
        XCTAssertEqual(buffer.samples.count, 300)
    }

    func testBacklogBeyondTheCapKeepsOnlyTheMostRecentAudio() {
        var buffer = LiveChannelBuffer(maxBacklogSamples: 1_000)
        buffer.append((0..<1_500).map { Float($0) })
        XCTAssertEqual(buffer.samples.count, 1_000)
        XCTAssertEqual(buffer.startSample, 500, "the oldest 500 samples were dropped and the index advanced")
        XCTAssertEqual(buffer.samples.first, 500)
    }

    func testCentisecondsForAnAbsoluteSampleIndex() {
        XCTAssertEqual(LiveChannelBuffer.centiseconds(forSample: 16_000, sampleRate: 16_000), 100)
        XCTAssertEqual(LiveChannelBuffer.centiseconds(forSample: 8_000, sampleRate: 16_000), 50)
    }

    func testPausePolicy() {
        XCTAssertFalse(LivePausePolicy.shouldPause(lowPowerMode: false, thermal: .nominal))
        XCTAssertFalse(LivePausePolicy.shouldPause(lowPowerMode: false, thermal: .fair))
        XCTAssertTrue(LivePausePolicy.shouldPause(lowPowerMode: true, thermal: .nominal))
        XCTAssertTrue(LivePausePolicy.shouldPause(lowPowerMode: false, thermal: .serious))
        XCTAssertTrue(LivePausePolicy.shouldPause(lowPowerMode: false, thermal: .critical))
    }

    func testCadenceStretchesWhenAChunkIsSlowAndRecoversWhenFast() {
        // A 30 s chunk that took 20 s (more than half) doubles the interval, capped at 2x base.
        XCTAssertEqual(LiveCadence.nextInterval(base: 5, current: 5, chunkSeconds: 30, tookSeconds: 20), 10)
        XCTAssertEqual(LiveCadence.nextInterval(base: 5, current: 10, chunkSeconds: 30, tookSeconds: 25), 10)
        // A fast chunk steps back toward the base.
        XCTAssertEqual(LiveCadence.nextInterval(base: 5, current: 10, chunkSeconds: 30, tookSeconds: 3), 5)
        XCTAssertEqual(LiveCadence.nextInterval(base: 5, current: 5, chunkSeconds: 30, tookSeconds: 3), 5)
    }

    func testThreadCountIsHalfTheCoresAtLeastOne() {
        XCTAssertEqual(LiveCadence.threads(forCores: 10), 5)
        XCTAssertEqual(LiveCadence.threads(forCores: 1), 1)
        XCTAssertEqual(LiveCadence.threads(forCores: 0), 1)
    }

    func testTakeNeverReturnsMoreThanTheBufferHolds() {
        var buffer = LiveChannelBuffer()
        buffer.append([1, 2, 3])
        let taken = buffer.take(count: 10, keepOverlap: 0)
        XCTAssertEqual(taken.samples.count, 3)
        XCTAssertTrue(buffer.samples.isEmpty)
    }
}
