import Foundation
import Testing
@testable import WhimbrelUI

/// One test per row of the state table in docs/ui-redesign/screens/home.md.
@Suite struct CapturePauseTests {
    private var calendar: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "America/New_York")!
        return c
    }

    private func date(_ y: Int, _ m: Int, _ d: Int, _ h: Int, _ min: Int = 0) -> Date {
        calendar.date(from: DateComponents(year: y, month: m, day: d, hour: h, minute: min))!
    }

    private func step(_ state: CapturePauseState, _ at: Date?, _ now: Date, _ event: CapturePauseEvent) -> CapturePauseResult {
        CapturePauseModel.next(state: state, resumeAt: at, now: now, event: event, calendar: calendar)
    }

    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    @Test func pauseForATimeFromOnSetsTheTimeAndStopsCapture() {
        let r = step(.on, nil, now, .pause(.fifteenMinutes))
        #expect(r == CapturePauseResult(state: .paused, resumeAt: now.addingTimeInterval(900), action: .stop))
        let h = step(.on, nil, now, .pause(.oneHour))
        #expect(h.resumeAt == now.addingTimeInterval(3600))
    }

    @Test func pauseForATimeFromPausedReplacesTheTime() {
        let r = step(.paused, now.addingTimeInterval(60), now, .pause(.oneHour))
        #expect(r.state == .paused)
        #expect(r.resumeAt == now.addingTimeInterval(3600))
    }

    @Test func pauseFromOffStaysOffWithNoTime() {
        let r = step(.off, nil, now, .pause(.oneHour))
        #expect(r == CapturePauseResult(state: .off, resumeAt: nil, action: .none))
    }

    @Test func resumeNowFromPausedTurnsOnAndClears() {
        let r = step(.paused, now.addingTimeInterval(600), now, .resumeNow)
        #expect(r == CapturePauseResult(state: .on, resumeAt: nil, action: .start))
    }

    @Test func resumeNowDoesNothingFromOffOrOn() {
        #expect(step(.off, nil, now, .resumeNow) == CapturePauseResult(state: .off, resumeAt: nil, action: .none))
        #expect(step(.on, nil, now, .resumeNow) == CapturePauseResult(state: .on, resumeAt: nil, action: .none))
    }

    @Test func turnOffFromOnOrPausedClears() {
        #expect(step(.on, nil, now, .turnOff) == CapturePauseResult(state: .off, resumeAt: nil, action: .stop))
        #expect(step(.paused, now.addingTimeInterval(600), now, .turnOff) == CapturePauseResult(state: .off, resumeAt: nil, action: .stop))
        #expect(step(.off, nil, now, .turnOff) == CapturePauseResult(state: .off, resumeAt: nil, action: .none))
    }

    @Test func turnOnFromOffHasNoTime() {
        #expect(step(.off, nil, now, .turnOn) == CapturePauseResult(state: .on, resumeAt: nil, action: .start))
    }

    @Test func timeReachedWhileRunningTurnsOnAndClears() {
        let at = now.addingTimeInterval(-1)
        #expect(step(.paused, at, now, .timeReached) == CapturePauseResult(state: .on, resumeAt: nil, action: .start))
        // Exactly at the time counts as reached.
        #expect(step(.paused, now, now, .timeReached).state == .on)
    }

    @Test func aTimerFiringEarlyDoesNotEndThePause() {
        let at = now.addingTimeInterval(30)
        #expect(step(.paused, at, now, .timeReached) == CapturePauseResult(state: .paused, resumeAt: at, action: .none))
    }

    @Test func timeReachedWhileNotRunningIsOnAtTheNextStart() {
        let at = now.addingTimeInterval(-3600)
        #expect(step(.paused, at, now, .appStarted) == CapturePauseResult(state: .on, resumeAt: nil, action: .start))
    }

    @Test func startWithTimeNotReachedKeepsThePause() {
        let at = now.addingTimeInterval(3600)
        #expect(step(.paused, at, now, .appStarted) == CapturePauseResult(state: .paused, resumeAt: at, action: .none))
    }

    @Test func deleteEverythingEndsInOffFromAnyState() {
        for state in [CapturePauseState.on, .paused, .off] {
            let r = step(state, now.addingTimeInterval(600), now, .deleteEverything)
            #expect(r.state == .off)
            #expect(r.resumeAt == nil)
        }
    }

    @Test func wakeFromSleepComparesTheClock() {
        let at = now.addingTimeInterval(600)
        // Slept through the end: on, cleared.
        #expect(step(.paused, at, now.addingTimeInterval(3600), .wokeFromSleep) == CapturePauseResult(state: .on, resumeAt: nil, action: .start))
        // Woke before the end: still paused, time kept.
        #expect(step(.paused, at, now, .wokeFromSleep) == CapturePauseResult(state: .paused, resumeAt: at, action: .none))
    }

    @Test func expiryNeverTurnsOnWhatThePersonTurnedOff() {
        for event in [CapturePauseEvent.timeReached, .appStarted, .wokeFromSleep] {
            let r = step(.off, now.addingTimeInterval(-60), now, event)
            #expect(r == CapturePauseResult(state: .off, resumeAt: nil, action: .none))
        }
    }

    @Test func stateComesFromCaptureAndStoredTime() {
        #expect(CapturePauseModel.state(captureOn: true, resumeAt: nil) == .on)
        #expect(CapturePauseModel.state(captureOn: false, resumeAt: now) == .paused)
        #expect(CapturePauseModel.state(captureOn: false, resumeAt: nil) == .off)
    }

    @Test func untilTomorrowIsFourAMOnTheNextCalendarDay() {
        // A Tuesday afternoon, 4:00 AM sharp and a late night before midnight all end at 4 AM on the next date.
        let wed4 = date(2026, 11, 18, 4)
        #expect(CapturePauseModel.resumeTime(for: .untilTomorrow, now: date(2026, 11, 17, 15, 10), calendar: calendar) == wed4)
        #expect(CapturePauseModel.resumeTime(for: .untilTomorrow, now: date(2026, 11, 17, 23, 59), calendar: calendar) == wed4)
        #expect(CapturePauseModel.resumeTime(for: .untilTomorrow, now: date(2026, 11, 17, 4, 0), calendar: calendar) == wed4)
        // Between midnight and 4 AM a late night is still today: 4 AM the same date.
        #expect(CapturePauseModel.resumeTime(for: .untilTomorrow, now: date(2026, 11, 18, 0, 30), calendar: calendar) == wed4)
        #expect(CapturePauseModel.resumeTime(for: .untilTomorrow, now: date(2026, 11, 18, 3, 59), calendar: calendar) == wed4)
        #expect(CapturePauseModel.resumeTime(for: .untilTomorrow, now: date(2026, 11, 18, 0, 0), calendar: calendar) == wed4)
    }

    @Test func untilTomorrowHoldsAcrossADaylightSavingChange() {
        // The US clocks go back on 2026-11-01: 4 AM that day is still 4 AM on the wall.
        let r = CapturePauseModel.resumeTime(for: .untilTomorrow, now: date(2026, 10, 31, 22), calendar: calendar)
        let parts = calendar.dateComponents([.year, .month, .day, .hour, .minute], from: r)
        #expect(parts.day == 1 && parts.month == 11 && parts.hour == 4 && parts.minute == 0)
    }
}
