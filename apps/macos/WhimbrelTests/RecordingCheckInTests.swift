import XCTest
@testable import Whimbrel

final class RecordingCheckInTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_000_000)
    private let h: TimeInterval = 3600
    private let m: TimeInterval = 60

    private func make(_ after: TimeInterval? = 2 * 3600) -> RecordingCheckIn {
        RecordingCheckIn(startedAt: t0, firstPromptAfter: after)
    }

    func testNoPromptBeforeThreshold() {
        var c = make()
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(h + 59 * m)), .none)
    }

    func testPromptAtThresholdOnce() {
        var c = make()
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(2 * h)), .showPrompt(deadline: t0.addingTimeInterval(2 * h + 5 * m)))
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(2 * h + m)), .none)
    }

    func testStopsWhenUnansweredOnce() {
        var c = make()
        _ = c.poll(now: t0.addingTimeInterval(2 * h))
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(2 * h + 5 * m)), .stop)
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(2 * h + 6 * m)), .none)
    }

    func testKeepRecordingReasksAfterAnHour() {
        var c = make()
        _ = c.poll(now: t0.addingTimeInterval(2 * h))
        c.keepRecording(now: t0.addingTimeInterval(2 * h + m))
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(3 * h)), .none)
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(3 * h + m)), .showPrompt(deadline: t0.addingTimeInterval(3 * h + 6 * m)))
    }

    func testMaxDurationSuppressedOnlyAfterKeepRecording() {
        var c = make()
        XCTAssertFalse(c.suppressesMaxDuration(now: t0.addingTimeInterval(2 * h - 1)))
        c.keepRecording(now: t0.addingTimeInterval(3 * h + m))
        XCTAssertTrue(c.suppressesMaxDuration(now: t0.addingTimeInterval(4 * h)))
        // From 4 h 01 the next prompt is due, then showing: still suppressed until it stops.
        XCTAssertTrue(c.suppressesMaxDuration(now: t0.addingTimeInterval(4 * h + 7 * m)))
        _ = c.poll(now: t0.addingTimeInterval(4 * h + 7 * m))
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(4 * h + 12 * m)), .stop)
        XCTAssertFalse(c.suppressesMaxDuration(now: t0.addingTimeInterval(4 * h + 12 * m)))
    }

    func testOffNeverPrompts() {
        var c = make(nil)
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(10 * h)), .none)
    }

    private func defaults() -> UserDefaults {
        let suite = "checkin-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return d
    }

    func testSettingDefaultsToTwoHoursAndZeroIsOff() {
        let d = defaults()
        let s = RecordingCheckInSettings(defaults: d)
        XCTAssertEqual(s.hours, 2)
        XCTAssertEqual(s.firstPromptAfter, 2 * 3600)
        s.hours = 0
        XCTAssertNil(RecordingCheckInSettings(defaults: d).firstPromptAfter)
        XCTAssertEqual(RecordingCheckInSettings(defaults: d).hours, 0)
    }

    #if DEBUG
    /// DEBUG shorteners for the manual pass: positive integers override the first prompt
    /// (and re-ask) and the countdown, in seconds; anything else leaves the real values.
    func testDebugOverridesShortenTheCheckIn() {
        let d = defaults()
        d.set(30, forKey: "com.mpointer.kestrel.debugCheckInSeconds")
        d.set("20", forKey: "com.mpointer.kestrel.debugCheckInCountdownSeconds")  // `defaults write` without -int
        var c = RecordingCheckInSettings(defaults: d).makeCheckIn(startedAt: t0)
        XCTAssertEqual(c.firstPromptAfter, 30)
        XCTAssertEqual(c.countdown, 20)
        XCTAssertEqual(c.reaskAfter, 30)
        XCTAssertEqual(c.poll(now: t0 + 30), .showPrompt(deadline: t0 + 50))
        XCTAssertEqual(c.poll(now: t0 + 50), .stop)
        c.keepRecording(now: t0 + 60)
        XCTAssertEqual(c.poll(now: t0 + 90), .showPrompt(deadline: t0 + 110))
    }

    func testDebugOverridesIgnoreZeroNegativeAndJunk() {
        for value in [0, -5, "abc"] as [Any] {
            let d = defaults()
            d.set(value, forKey: "com.mpointer.kestrel.debugCheckInSeconds")
            d.set(value, forKey: "com.mpointer.kestrel.debugCheckInCountdownSeconds")
            let c = RecordingCheckInSettings(defaults: d).makeCheckIn(startedAt: t0)
            XCTAssertEqual(c.firstPromptAfter, 2 * h, "\(value)")
            XCTAssertEqual(c.countdown, 5 * m)
            XCTAssertEqual(c.reaskAfter, h)
        }
    }
    #endif

    func testOutOfRangePersistedValueFallsBackToTwo() {
        let d = defaults()
        d.set(9, forKey: "com.mpointer.kestrel.recordingCheckInHours")
        XCTAssertEqual(RecordingCheckInSettings(defaults: d).hours, 2)
    }

    func testMaxDurationSuppressedWhilePromptShowingUntilStop() {
        var c = make()
        XCTAssertFalse(c.suppressesMaxDuration(now: t0.addingTimeInterval(2 * h - m)))
        _ = c.poll(now: t0.addingTimeInterval(2 * h))
        XCTAssertTrue(c.suppressesMaxDuration(now: t0.addingTimeInterval(2 * h + 4 * m)))
        XCTAssertEqual(c.poll(now: t0.addingTimeInterval(2 * h + 5 * m)), .stop)
        XCTAssertFalse(c.suppressesMaxDuration(now: t0.addingTimeInterval(2 * h + 5 * m)))
    }

    func testMaxDurationSuppressedOnceAPromptIsDueBeforeItsPoll() {
        // Setting at 4 h: core's 4 h max duration must not win the race with the prompt.
        let c = make(4 * 3600)
        XCTAssertFalse(c.suppressesMaxDuration(now: t0.addingTimeInterval(4 * h - 1)))
        XCTAssertTrue(c.suppressesMaxDuration(now: t0.addingTimeInterval(4 * h)))
    }
}
