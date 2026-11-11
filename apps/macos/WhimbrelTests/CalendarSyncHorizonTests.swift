import XCTest
@testable import Whimbrel

/// The phones get the next 48 hours of calendar events, while the Mac's own surfaces
/// (Today, the menu bar, the meeting trigger) keep their today-only window.
final class CalendarSyncHorizonTests: XCTestCase {
    private func meeting(id: String, title: String, start: Date, source: MeetingSource, response: SelfResponse? = nil, allDay: Bool = false) -> QualifyingMeeting {
        QualifyingMeeting(
            id: id, title: title, startDate: start, endDate: start.addingTimeInterval(1800),
            attendees: ["alice@x.com"], conferenceLink: nil, source: source, appBundleID: nil, organizerEmail: nil, organizerName: nil, selfIsOrganizer: false, selfResponse: response, isAllDay: allDay
        )
    }

    private var now: Date {
        Calendar.current.date(bySettingHour: 8, minute: 0, second: 0, of: Date(timeIntervalSince1970: 1_761_000_000))!
    }

    func testIncludesTomorrowWithin48h() {
        let today = meeting(id: "ek-1", title: "Standup", start: now.addingTimeInterval(2 * 3600), source: .calendar)
        let tomorrow = meeting(id: "wk-1", title: "Planning", start: now.addingTimeInterval(25 * 3600), source: .calendar)
        let result = AppState.syncMeetings(eventKit: [today], week: [tomorrow], oauth: [], now: now)
        XCTAssertEqual(Set(result.map(\.id)), ["ek-1", "wk-1"])
    }

    func testExcludesBeyond48h() {
        let past = meeting(id: "wk-2", title: "Offsite", start: now.addingTimeInterval(49 * 3600), source: .calendar)
        let pastOAuth = meeting(id: "o-2", title: "Partner Connect", start: now.addingTimeInterval(49 * 3600), source: .googleCalendar)
        let inside = meeting(id: "wk-3", title: "Review", start: now.addingTimeInterval(47 * 3600), source: .calendar)
        let result = AppState.syncMeetings(eventKit: [], week: [past, inside], oauth: [pastOAuth], now: now)
        XCTAssertEqual(result.map(\.id), ["wk-3"])
    }

    func testKeepsTodaysAlreadyStartedMeetings() {
        let started = meeting(id: "ek-4", title: "Early call", start: now.addingTimeInterval(-1800), source: .calendar)
        XCTAssertEqual(AppState.syncMeetings(eventKit: [started], week: [], oauth: [], now: now).map(\.id), ["ek-4"])
    }

    func testMergesDuplicatesAcrossSources() {
        let start = now.addingTimeInterval(3600)
        let ek = meeting(id: "ek-5", title: "Standup", start: start, source: .calendar)
        let oauth = meeting(id: "o-5", title: "STANDUP", start: start, source: .googleCalendar)
        XCTAssertEqual(AppState.syncMeetings(eventKit: [ek], week: [], oauth: [oauth], now: now).count, 1)
    }

    func testExcludesDeclinedMeetings() {
        let declined = meeting(id: "wk-6", title: "Skipped", start: now.addingTimeInterval(3 * 3600), source: .calendar, response: .declined)
        let accepted = meeting(id: "wk-7", title: "Going", start: now.addingTimeInterval(5 * 3600), source: .calendar, response: .accepted)
        let declinedOAuth = meeting(id: "o-6", title: "Skipped too", start: now.addingTimeInterval(26 * 3600), source: .googleCalendar, response: .declined)
        let result = AppState.syncMeetings(eventKit: [], week: [declined, accepted], oauth: [declinedOAuth], now: now)
        XCTAssertEqual(result.map(\.id), ["wk-7"])
    }

    func testKeepsTentativeAndUnknownResponses() {
        let tentative = meeting(id: "ek-8", title: "Maybe", start: now.addingTimeInterval(3600), source: .calendar, response: .tentative)
        let unknown = meeting(id: "ek-9", title: "No reply", start: now.addingTimeInterval(7200), source: .calendar, response: nil)
        let result = AppState.syncMeetings(eventKit: [tentative, unknown], week: [], oauth: [], now: now)
        XCTAssertEqual(Set(result.map(\.id)), ["ek-8", "ek-9"])
    }

    func testDedupesSameEventInEventKitAndWeek() {
        let start = now.addingTimeInterval(3600)
        let inEventKit = meeting(id: "ek-10", title: "Standup", start: start, source: .calendar)
        let inWeek = meeting(id: "ek-10", title: "Standup", start: start, source: .calendar)
        XCTAssertEqual(AppState.syncMeetings(eventKit: [inEventKit], week: [inWeek], oauth: [], now: now).map(\.id), ["ek-10"])
    }

    func testKeepsRecurringOccurrencesOnDifferentDays() {
        let todayStart = now.addingTimeInterval(2 * 3600)
        let tomorrowStart = todayStart.addingTimeInterval(24 * 3600)
        let todayOccurrence = meeting(id: "ek-11", title: "Standup", start: todayStart, source: .calendar)
        let tomorrowOccurrence = meeting(id: "ek-11", title: "Standup", start: tomorrowStart, source: .calendar)
        let result = AppState.syncMeetings(eventKit: [todayOccurrence], week: [tomorrowOccurrence], oauth: [], now: now)
        XCTAssertEqual(result.map(\.startDate), [todayStart, tomorrowStart])
    }

    // MARK: - Apple Calendar presence

    func testNoReconcileWithoutASuccessfulFetch() {
        let m = meeting(id: "ek-20", title: "Standup", start: now.addingTimeInterval(3600), source: .calendar)
        XCTAssertNil(AppState.eventKitPresence(eventKit: [m], week: [], now: now, fetchSucceeded: false))
        XCTAssertNil(AppState.eventKitPresence(eventKit: [], week: [], now: now, fetchSucceeded: false))
    }

    func testPresenceIsTakenBeforeMergingWithGoogle() {
        let start = now.addingTimeInterval(3600)
        let ek = meeting(id: "ek-21", title: "Standup", start: start, source: .calendar)
        let twin = meeting(id: "o-21", title: "STANDUP", start: start, source: .googleCalendar)
        // The merge keeps only the Google copy, but the meeting is still in Apple Calendar.
        XCTAssertEqual(AppState.syncMeetings(eventKit: [ek], week: [], oauth: [twin], now: now).map(\.id), ["o-21"])
        let present = AppState.eventKitPresence(eventKit: [ek], week: [], now: now, fetchSucceeded: true)
        XCTAssertEqual(present?.count, 1)
        XCTAssertEqual(present?.first?.startUnixMs, Int64(start.timeIntervalSince1970 * 1000))
    }

    func testPresenceDropsDeclinedMeetings() {
        let declined = meeting(id: "ek-22", title: "Skipped", start: now.addingTimeInterval(3600), source: .calendar, response: .declined)
        let kept = meeting(id: "ek-23", title: "Going", start: now.addingTimeInterval(7200), source: .calendar, response: .accepted)
        let beyond = meeting(id: "ek-24", title: "Later", start: now.addingTimeInterval(49 * 3600), source: .calendar)
        let dup = meeting(id: "ek-23", title: "Going", start: now.addingTimeInterval(7200), source: .calendar, response: .accepted)
        let present = AppState.eventKitPresence(eventKit: [declined, kept], week: [dup, beyond], now: now, fetchSucceeded: true)
        XCTAssertEqual(present?.count, 1)
    }

    func testEmptySuccessfulFetchIsAnEmptyPresenceList() {
        let present = AppState.eventKitPresence(eventKit: [], week: [], now: now, fetchSucceeded: true)
        XCTAssertNotNil(present)
        XCTAssertEqual(present?.count, 0)
    }

    private func present(_ n: Int) -> [PresentEvent] {
        (0..<n).map { PresentEvent(externalId: "e\($0)", startUnixMs: Int64($0)) }
    }

    func testEmptyAfterNonEmptyNeedsASecondRefresh() {
        let first = AppState.confirmedPresence(current: [], previousCount: 3, emptyStreak: 0, generation: 5, lastGeneration: 4)
        XCTAssertNil(first.present)
        XCTAssertEqual(first.emptyStreak, 1)
        let second = AppState.confirmedPresence(current: [], previousCount: 3, emptyStreak: first.emptyStreak, generation: 6, lastGeneration: first.lastGeneration)
        XCTAssertEqual(second.present?.count, 0)
        // A non-empty list in between resets the streak.
        let back = AppState.confirmedPresence(current: present(2), previousCount: 3, emptyStreak: 1, generation: 6, lastGeneration: 5)
        XCTAssertEqual(back.present?.count, 2)
        XCTAssertEqual(back.emptyStreak, 0)
        // A failed fetch passes nil and resets the streak.
        let failed = AppState.confirmedPresence(current: nil, previousCount: 3, emptyStreak: 1, generation: 6, lastGeneration: 5)
        XCTAssertNil(failed.present)
        XCTAssertEqual(failed.emptyStreak, 0)
    }

    func testRepeatPersistOfTheSameRefreshDoesNotConfirmAnEmptyList() {
        let first = AppState.confirmedPresence(current: [], previousCount: 3, emptyStreak: 0, generation: 5, lastGeneration: 4)
        XCTAssertNil(first.present)
        let again = AppState.confirmedPresence(current: [], previousCount: 3, emptyStreak: first.emptyStreak, generation: 5, lastGeneration: first.lastGeneration)
        XCTAssertNil(again.present)
        XCTAssertEqual(again.emptyStreak, 1)
    }

    func testEmptyFirstReconcileNeedsASecondRefresh() {
        // Nothing known yet (a launch): the first empty read is withheld, even so.
        let first = AppState.confirmedPresence(current: [], previousCount: nil, emptyStreak: 0, generation: 1, lastGeneration: 0)
        XCTAssertNil(first.present)
        XCTAssertEqual(first.emptyStreak, 1)
        let second = AppState.confirmedPresence(current: [], previousCount: nil, emptyStreak: first.emptyStreak, generation: 2, lastGeneration: first.lastGeneration)
        XCTAssertEqual(second.present?.count, 0)
        // Same for a known-empty previous list.
        let zero = AppState.confirmedPresence(current: [], previousCount: 0, emptyStreak: 0, generation: 1, lastGeneration: 0)
        XCTAssertNil(zero.present)
    }

    func testFetchWithNoCalendarsIsNotASuccess() {
        XCTAssertFalse(CalendarService.fetchSucceeded(accessible: true, calendarCount: 0))
        XCTAssertFalse(CalendarService.fetchSucceeded(accessible: false, calendarCount: 4))
        XCTAssertTrue(CalendarService.fetchSucceeded(accessible: true, calendarCount: 1))
    }

    // MARK: Today panel filter (#657)

    func testPanelHidesDeclinedAndAllDayButTheTriggerListKeepsThem() {
        let t = now.addingTimeInterval(3600)
        let declined = meeting(id: "d", title: "Declined", start: t, source: .calendar, response: .declined)
        let allDay = meeting(id: "a", title: "Offsite", start: now, source: .calendar, allDay: true)
        let kept = meeting(id: "k", title: "Standup", start: t.addingTimeInterval(3600), source: .calendar, response: .tentative)
        let declinedOAuth = meeting(id: "od", title: "Skipped", start: t.addingTimeInterval(7200), source: .googleCalendar, response: .declined)
        let all = AppState.todaysMeetings(eventKit: [declined, allDay, kept], oauth: [declinedOAuth], now: now)
        XCTAssertEqual(Set(all.map(\.id)), ["d", "a", "k", "od"])
        let panel = AppState.todaysMeetings(eventKit: [declined, allDay, kept], oauth: [declinedOAuth], now: now, forPanel: true)
        XCTAssertEqual(panel.map(\.id), ["k"])
    }

    // MARK: DST-safe window (#657)

    private func newYork() -> Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "America/New_York")!
        return c
    }

    private func ny(_ cal: Calendar, _ y: Int, _ m: Int, _ d: Int, _ h: Int, _ min: Int = 0) -> Date {
        cal.date(from: DateComponents(year: y, month: m, day: d, hour: h, minute: min))!
    }

    func testWindowEndOnTheEveningBeforeSpringForwardIsTheStartOfTomorrow() {
        let cal = newYork()
        let end = CalendarService.todayWindowEnd(now: ny(cal, 2026, 3, 7, 23, 30), calendar: cal)
        XCTAssertEqual(end, ny(cal, 2026, 3, 8, 0))
    }

    func testWindowEndJustAfterMidnightOnFallBackDayIsTheStartOfTomorrow() {
        let cal = newYork()
        let now = ny(cal, 2026, 11, 1, 0, 30)
        let end = CalendarService.todayWindowEnd(now: now, calendar: cal)
        XCTAssertEqual(end, ny(cal, 2026, 11, 2, 0))
        XCTAssertGreaterThan(end, now)
    }

    func testTriggerListKeepsTheLaterTodayMeetingJustAfterMidnightOnFallBack() {
        let cal = newYork()
        let now = ny(cal, 2026, 11, 1, 0, 30)
        let evening = meeting(id: "e", title: "Evening", start: ny(cal, 2026, 11, 1, 23, 45), source: .googleCalendar)
        let tomorrow = meeting(id: "t", title: "Tomorrow", start: ny(cal, 2026, 11, 2, 0, 15), source: .googleCalendar)
        let result = AppState.todaysMeetings(eventKit: [], oauth: [evening, tomorrow], now: now, calendar: cal)
        XCTAssertEqual(result.map(\.id), ["e"])
    }

    func testTriggerListDoesNotReachIntoTheDayAfterTomorrowOnSpringForwardEve() {
        let cal = newYork()
        let now = ny(cal, 2026, 3, 7, 23, 30)
        let today = meeting(id: "a", title: "Late", start: ny(cal, 2026, 3, 7, 23, 45), source: .googleCalendar)
        let tomorrow = meeting(id: "b", title: "Tomorrow", start: ny(cal, 2026, 3, 8, 0, 30), source: .googleCalendar)
        let result = AppState.todaysMeetings(eventKit: [], oauth: [today, tomorrow], now: now, calendar: cal)
        XCTAssertEqual(result.map(\.id), ["a"])
    }
}
