import XCTest
@testable import Whimbrel

final class SilenceWatchTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_000_000)
    private let quiet = -80.0
    private let loud = -20.0

    /// Feeds a constant level every second from `from` to `to` seconds (inclusive); returns the events.
    private func run(_ w: inout SilenceWatch, _ from: Int, _ to: Int, system: Double?, mic: Double) -> [SilenceWatch.Event] {
        (from...to).compactMap { w.feed(at: t0.addingTimeInterval(Double($0)), systemDB: system, micDB: mic) }
    }

    func testThreeMinutesOfSilenceWarnsOnce() {
        var w = SilenceWatch()
        XCTAssertEqual(run(&w, 0, 179, system: quiet, mic: quiet), [])
        XCTAssertEqual(run(&w, 180, 600, system: quiet, mic: quiet), [.warn])
    }

    func testSoundJustBeforeThreeMinutesPreventsWarning() {
        var w = SilenceWatch()
        XCTAssertEqual(run(&w, 0, 178, system: quiet, mic: quiet), [])
        XCTAssertEqual(run(&w, 179, 179, system: quiet, mic: loud), [])
        XCTAssertEqual(run(&w, 180, 358, system: quiet, mic: quiet), [], "the silence clock restarted at 2:59")
    }

    func testSoundAfterWarningClears() {
        var w = SilenceWatch()
        XCTAssertEqual(run(&w, 0, 200, system: quiet, mic: quiet), [.warn])
        XCTAssertEqual(run(&w, 201, 201, system: loud, mic: quiet), [.clear])
    }

    func testNoRewarnWithoutThirtySecondsOfSound() {
        var w = SilenceWatch()
        _ = run(&w, 0, 200, system: quiet, mic: quiet)
        XCTAssertEqual(run(&w, 201, 220, system: loud, mic: quiet), [.clear])   // 20 s of sound only
        XCTAssertEqual(run(&w, 221, 800, system: quiet, mic: quiet), [], "not re-armed")
    }

    func testRewarnsAfterThirtySecondsOfSoundThenThreeMinutesSilence() {
        var w = SilenceWatch()
        _ = run(&w, 0, 200, system: quiet, mic: quiet)
        XCTAssertEqual(run(&w, 201, 231, system: loud, mic: quiet), [.clear])   // 30 s of sound
        XCTAssertEqual(run(&w, 232, 411, system: quiet, mic: quiet), [])
        XCTAssertEqual(run(&w, 412, 412, system: quiet, mic: quiet), [.warn])
    }

    func testSystemTrackAbsentUsesMicAlone() {
        var w = SilenceWatch()
        XCTAssertEqual(run(&w, 0, 179, system: nil, mic: quiet), [])
        XCTAssertEqual(run(&w, 180, 180, system: nil, mic: quiet), [.warn])
        var w2 = SilenceWatch()
        XCTAssertEqual(run(&w2, 0, 400, system: nil, mic: loud), [], "mic sound keeps it quiet")
    }

    func testEitherTrackAudibleCountsAsSound() {
        var w = SilenceWatch()
        XCTAssertEqual(run(&w, 0, 400, system: loud, mic: quiet), [])
    }

    func testDecibelsFromPeak() {
        XCTAssertEqual(SilenceWatch.decibels(fromPeak: 1), 0, accuracy: 0.001)
        XCTAssertEqual(SilenceWatch.decibels(fromPeak: 0.1), -20, accuracy: 0.001)
        XCTAssertLessThan(SilenceWatch.decibels(fromPeak: 0), -100)
    }
}
