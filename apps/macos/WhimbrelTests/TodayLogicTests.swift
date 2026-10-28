import XCTest
@testable import Whimbrel

final class TodayLogicTests: XCTestCase {
    private var calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "UTC")!
        return c
    }()

    private func date(_ day: Int, _ hour: Int, _ minute: Int = 0) -> Date {
        calendar.date(from: DateComponents(year: 2026, month: 10, day: day, hour: hour, minute: minute))!
    }

    private func ms(_ d: Date) -> Int64 { Int64(d.timeIntervalSince1970 * 1000) }

    private func up(_ id: String, _ start: Date) -> TodayUpcoming {
        TodayUpcoming(id: id, title: id, start: start)
    }

    // MARK: hero

    func testAnActiveMeetingThatIsInTheRecentListIsTheLiveHero() {
        let hero = TodayLogic.hero(
            activeMeetingId: 7, recentMeetingIds: [9, 7], upcoming: [up("a", date(6, 15))],
            calendarGranted: true, now: date(6, 12), calendar: calendar
        )
        XCTAssertEqual(hero, .live(meetingId: 7))
    }

    func testAnActiveIdMissingFromTheRecentListFallsThroughToTheUpcomingMeeting() {
        let hero = TodayLogic.hero(
            activeMeetingId: 7, recentMeetingIds: [9], upcoming: [up("a", date(6, 15))],
            calendarGranted: true, now: date(6, 12), calendar: calendar
        )
        XCTAssertEqual(hero, .upcoming(eventId: "a"))
    }

    /// A rebuilt Today (skin switch mid-meeting) has not loaded the recent list yet; the live
    /// slot must hold its place rather than flash the upcoming hero or "Start note".
    func testAnActiveMeetingIsLiveBeforeTheRecentListHasLoaded() {
        let hero = TodayLogic.hero(
            activeMeetingId: 7, recentMeetingIds: [], recentLoaded: false, upcoming: [up("a", date(6, 15))],
            calendarGranted: true, now: date(6, 12), calendar: calendar
        )
        XCTAssertEqual(hero, .live(meetingId: 7))
        let empty = TodayLogic.hero(
            activeMeetingId: 7, recentMeetingIds: [], recentLoaded: false, upcoming: [],
            calendarGranted: true, now: date(6, 12), calendar: calendar
        )
        XCTAssertEqual(empty, .live(meetingId: 7), "not the No meetings / Start note empty state")
    }

    func testTheEarliestUpcomingMeetingToday_isTheHero() {
        let hero = TodayLogic.hero(
            activeMeetingId: nil, recentMeetingIds: [], upcoming: [up("late", date(6, 17)), up("soon", date(6, 13))],
            calendarGranted: true, now: date(6, 12), calendar: calendar
        )
        XCTAssertEqual(hero, .upcoming(eventId: "soon"))
    }

    func testAMeetingAtOneMinutePastMidnightTomorrowIsNotTheHero() {
        let hero = TodayLogic.hero(
            activeMeetingId: nil, recentMeetingIds: [1], upcoming: [up("t", date(7, 0, 1))],
            calendarGranted: true, now: date(6, 23, 59), calendar: calendar
        )
        XCTAssertEqual(hero, .none)
    }

    func testOnlyTomorrowsMeetingsAndNothingRecentIsNotTheNoMeetingsEmptyState() {
        let hero = TodayLogic.hero(
            activeMeetingId: nil, recentMeetingIds: [], upcoming: [up("t", date(7, 9))],
            calendarGranted: true, now: date(6, 12), calendar: calendar
        )
        XCTAssertEqual(hero, .none)
    }

    func testNothingAtAllShowsNoCalendarWhenNotGrantedAndNoMeetingsWhenGranted() {
        func hero(granted: Bool) -> TodayHero {
            TodayLogic.hero(
                activeMeetingId: nil, recentMeetingIds: [], upcoming: [],
                calendarGranted: granted, now: date(6, 12), calendar: calendar
            )
        }
        XCTAssertEqual(hero(granted: false), .noCalendar)
        XCTAssertEqual(hero(granted: true), .noMeetings)
    }

    func testRecentMeetingsAndNothingUpcomingHasNoHero() {
        let hero = TodayLogic.hero(
            activeMeetingId: nil, recentMeetingIds: [3], upcoming: [],
            calendarGranted: false, now: date(6, 12), calendar: calendar
        )
        XCTAssertEqual(hero, .none)
    }

    // MARK: agenda

    func testTodayHoldsUpcomingAscendingThenPastDescendingAndEarlierHoldsTheRestNewestFirst() {
        let agenda = TodayLogic.agenda(
            upcoming: [up("b", date(6, 16)), up("a", date(6, 14)), up("tomorrow", date(7, 9))],
            recent: [(id: 3, startUnixMs: ms(date(6, 11))), (id: 2, startUnixMs: ms(date(6, 9))), (id: 1, startUnixMs: ms(date(5, 15)))],
            excludingMeetingId: nil, now: date(6, 12), calendar: calendar, earlierLimit: 5
        )
        XCTAssertEqual(agenda, TodayAgenda(upcomingToday: ["a", "b"], pastToday: [3, 2], earlier: [1]))
    }

    func testTheActiveMeetingIsExcludedSoItDoesNotAppearTwice() {
        let agenda = TodayLogic.agenda(
            upcoming: [], recent: [(id: 3, startUnixMs: ms(date(6, 11))), (id: 2, startUnixMs: ms(date(6, 9)))],
            excludingMeetingId: 3, now: date(6, 12), calendar: calendar, earlierLimit: 5
        )
        XCTAssertEqual(agenda.pastToday, [2])
    }

    func testEarlierIsCappedAtTheLimitKeepingTheNewest() {
        let recent = (1...8).map { (id: Int64($0), startUnixMs: ms(date(5, 8)) - Int64($0) * 60_000) }
        let agenda = TodayLogic.agenda(
            upcoming: [], recent: recent, excludingMeetingId: nil, now: date(6, 12), calendar: calendar, earlierLimit: 3
        )
        XCTAssertEqual(agenda.earlier, [1, 2, 3])
    }

    func testAnEmptyDayGivesAnEmptyAgenda() {
        let agenda = TodayLogic.agenda(
            upcoming: [], recent: [], excludingMeetingId: nil, now: date(6, 12), calendar: calendar, earlierLimit: 5
        )
        XCTAssertEqual(agenda, TodayAgenda(upcomingToday: [], pastToday: [], earlier: []))
    }

    // MARK: recorded calls

    private func rec(_ id: Int64, _ start: Date, _ title: String) -> (id: Int64, startUnixMs: Int64, title: String) {
        (id: id, startUnixMs: ms(start), title: title)
    }

    func testAnUpcomingEventAlreadyRecordedIsDropped() {
        let kept = TodayLogic.upcomingExcludingRecorded(
            upcoming: [TodayUpcoming(id: "e", title: "Design review", start: date(6, 10))],
            recent: [rec(1, date(6, 10), "Design review")]
        )
        XCTAssertEqual(kept, [])
    }

    func testSameTitleDifferentStartIsKept() {
        let u = TodayUpcoming(id: "e", title: "Standup", start: date(6, 10))
        XCTAssertEqual(TodayLogic.upcomingExcludingRecorded(upcoming: [u], recent: [rec(1, date(5, 10), "Standup")]), [u])
    }

    func testSameStartDifferentTitleIsKept() {
        let u = TodayUpcoming(id: "e", title: "Standup", start: date(6, 10))
        XCTAssertEqual(TodayLogic.upcomingExcludingRecorded(upcoming: [u], recent: [rec(1, date(6, 10), "Other")]), [u])
    }

    func testTitleMatchIsExactNotCaseInsensitive() {
        let u = TodayUpcoming(id: "e", title: "Standup", start: date(6, 10))
        XCTAssertEqual(TodayLogic.upcomingExcludingRecorded(upcoming: [u], recent: [rec(1, date(6, 10), "standup")]), [u])
    }

    // MARK: time zones

    private var plus2: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(secondsFromGMT: 7200)!
        return c
    }

    func testAMeetingLateOnTheSixthUTCIsTomorrowInAPlusTwoZone() {
        // 23:30 UTC on the 6th is 01:30 on the 7th at UTC+2; now is 12:00 local on the 6th.
        let now = plus2.date(from: DateComponents(year: 2026, month: 10, day: 6, hour: 12))!
        let hero = TodayLogic.hero(
            activeMeetingId: nil, recentMeetingIds: [1], upcoming: [up("a", date(6, 23, 30))],
            calendarGranted: true, now: now, calendar: plus2
        )
        XCTAssertEqual(hero, .none)
        let agenda = TodayLogic.agenda(
            upcoming: [up("a", date(6, 23, 30))], recent: [], excludingMeetingId: nil, now: now, calendar: plus2, earlierLimit: 5
        )
        XCTAssertEqual(agenda.upcomingToday, [])
    }

    func testAnEarlyUTCMeetingOnTheSeventhIsStillTodayInAPlusTwoZone() {
        // 22:30 UTC on the 6th is 00:30 on the 7th local; at UTC-ish that is the 6th, here it is today (7th).
        let now = plus2.date(from: DateComponents(year: 2026, month: 10, day: 7, hour: 12))!
        let hero = TodayLogic.hero(
            activeMeetingId: nil, recentMeetingIds: [1], upcoming: [up("a", date(6, 22, 30))],
            calendarGranted: true, now: now, calendar: plus2
        )
        XCTAssertEqual(hero, .upcoming(eventId: "a"))
    }

    // MARK: recap from the synced Daily Brief

    private func brief(_ id: Int64, _ name: String = "Daily Brief", status: String = "success", at: Date, origin: String? = nil) -> CoreRoutineRunWithName {
        CoreRoutineRunWithName(
            id: id, routineId: 1, routineName: name, ranAtUnixMs: ms(at), status: status,
            outputSummary: "Brief \(id)", outputJson: "{}", originDeviceId: origin
        )
    }

    func testRecapRunPicksTheNewestSuccessfulDailyBriefFromToday() {
        let runs = [
            brief(1, at: date(13, 8, 30), origin: "dev-a"),
            brief(2, at: date(13, 11)),
            brief(3, at: date(12, 8, 30)),
        ]
        XCTAssertEqual(TodayLogic.recapRun(from: runs, now: date(13, 15), calendar: calendar)?.id, 2)
    }

    func testRecapRunIgnoresOtherRoutinesFailuresAndYesterday() {
        let runs = [
            brief(1, "EOD Recap", at: date(13, 9)),
            brief(2, status: "failed", at: date(13, 9)),
            brief(3, at: date(12, 23, 59)),
        ]
        XCTAssertNil(TodayLogic.recapRun(from: runs, now: date(13, 15), calendar: calendar))
    }

    func testRecapRunJudgesTodayInTheCalendarsOwnTimeZone() {
        var la = Calendar(identifier: .gregorian)
        la.timeZone = TimeZone(identifier: "America/Los_Angeles")!
        let now = la.date(from: DateComponents(year: 2026, month: 10, day: 13, hour: 1))!
        let yesterdayPT = la.date(from: DateComponents(year: 2026, month: 10, day: 12, hour: 23, minute: 30))!
        let todayPT = la.date(from: DateComponents(year: 2026, month: 10, day: 13, hour: 0, minute: 30))!
        XCTAssertNil(TodayLogic.recapRun(from: [brief(1, at: yesterdayPT)], now: now, calendar: la))
        XCTAssertEqual(TodayLogic.recapRun(from: [brief(2, at: todayPT)], now: now, calendar: la)?.id, 2)
    }

    // MARK: greeting and relative start

    func testGreetingUsesTheFirstWordOfTheProfileName() {
        XCTAssertEqual(TodayLogic.greeting(name: "Mike Pointer", now: date(6, 9), calendar: calendar), "Good morning, Mike")
        XCTAssertEqual(TodayLogic.greeting(name: "Mike Pointer", now: date(6, 13), calendar: calendar), "Good afternoon, Mike")
        XCTAssertEqual(TodayLogic.greeting(name: "Mike Pointer", now: date(6, 19), calendar: calendar), "Good evening, Mike")
    }

    func testGreetingIsNilWithoutAUsableName() {
        for name in ["", "   ", "a@b.com"] {
            XCTAssertNil(TodayLogic.greeting(name: name, now: date(6, 9), calendar: calendar), name)
        }
    }

    func testRelativeStart() {
        let now = date(6, 12)
        let end = date(6, 13)
        XCTAssertEqual(TodayLogic.relativeStart(now.addingTimeInterval(9 * 60), end: end, now: now), "In 9m")
        XCTAssertEqual(TodayLogic.relativeStart(now.addingTimeInterval(2 * 3600 + 5 * 60), end: date(6, 16), now: now), "In 2h")
        XCTAssertEqual(TodayLogic.relativeStart(now.addingTimeInterval(59), end: end, now: now), "In 1m")
        XCTAssertEqual(TodayLogic.relativeStart(now.addingTimeInterval(59 * 60 + 30), end: date(6, 15), now: now), "In 1h")
        XCTAssertEqual(TodayLogic.relativeStart(date(6, 11, 30), end: end, now: now), "Now")
        XCTAssertNil(TodayLogic.relativeStart(date(6, 10), end: date(6, 11), now: now))
        XCTAssertNil(TodayLogic.relativeStart(now.addingTimeInterval(13 * 3600), end: date(7, 2), now: now))
    }

    func testAttendeeSummaryShowsTwoNamesAndTheRest() {
        XCTAssertEqual(MeetingRowText.attendeeSummary([]), "")
        XCTAssertEqual(MeetingRowText.attendeeSummary(["A"]), "A")
        XCTAssertEqual(MeetingRowText.attendeeSummary(["A", "B"]), "A, B")
        XCTAssertEqual(MeetingRowText.attendeeSummary(["A", "B", "C", "D", "E"]), "A, B +3")
    }
}
