import XCTest
@testable import Whimbrel

final class ReminderPlannerTests: XCTestCase {
    private var calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "America/New_York")!
        return c
    }()

    /// A local time in the fixed zone; `day` is the day of November 2026.
    private func at(_ hour: Int, _ minute: Int = 0, day: Int = 11) -> Date {
        calendar.date(from: DateComponents(year: 2026, month: 11, day: day, hour: hour, minute: minute))!
    }

    private func ms(_ date: Date) -> Int64 { Int64(date.timeIntervalSince1970 * 1000) }

    private func event(_ id: String, _ title: String, _ start: Date, rowId: Int64? = nil) -> PhoneCalendarEvent {
        PhoneCalendarEvent(externalId: id, source: "google_calendar", title: title, startUnixMs: ms(start), attendees: [], rowId: rowId)
    }

    private func snapshot(key: String = "topic-1", name: String = "Northwind", next: PhoneTopicSnapshot.NextMeeting? = nil,
                          owe: [PhoneTopicSnapshot.Step] = []) -> PhoneTopicSnapshot {
        PhoneTopicSnapshot(topicKey: key, name: name, nextMeeting: next, youOwe: owe)
    }

    private func step(_ text: String, due: String?) -> PhoneTopicSnapshot.Step {
        PhoneTopicSnapshot.Step(text: text, sourceKind: "meeting", dueDate: due)
    }

    private func plan(events: [PhoneCalendarEvent] = [], snapshots: [PhoneTopicSnapshot] = [], settings: ReminderSettings = ReminderSettings(),
                      isCloud: Bool = true, now: Date? = nil) -> [PlannedReminder] {
        ReminderPlanner.plan(events: events, snapshots: snapshots, settings: settings, isCloud: isCloud, now: now ?? at(8, 30), calendar: calendar)
    }

    func testMeetingReminderFiresLeadMinutesBefore() {
        let planned = plan(events: [event("e1", "Northwind weekly", at(10))])
        XCTAssertEqual(planned.count, 1)
        XCTAssertEqual(planned[0].id, "meeting:e1")
        XCTAssertEqual(planned[0].fireAt, at(9, 50))
        XCTAssertEqual(planned[0].title, "Northwind weekly in 10 minutes")
        XCTAssertEqual(planned[0].route, .today)
    }

    func testMeetingReminderAddsTopicName() {
        let next = PhoneTopicSnapshot.NextMeeting(title: "Northwind weekly", startUnixMs: ms(at(10)), attendeesByOrg: [], me: 1)
        let planned = plan(events: [event("e1", "Northwind weekly", at(10))], snapshots: [snapshot(next: next)])
        XCTAssertEqual(planned[0].title, "Northwind weekly in 10 minutes · Northwind")
        XCTAssertEqual(planned[0].route, .topic(key: "topic-1"))
    }

    func testMeetingReminderIgnoresTopicWithDifferentStart() {
        let next = PhoneTopicSnapshot.NextMeeting(title: "Northwind weekly", startUnixMs: ms(at(11)), attendeesByOrg: [], me: 1)
        let planned = plan(events: [event("e1", "Northwind weekly", at(10))], snapshots: [snapshot(next: next)])
        XCTAssertEqual(planned[0].title, "Northwind weekly in 10 minutes")
        XCTAssertEqual(planned[0].route, .today)
    }

    func testDedupeKeepsNewestRowWithin24h() {
        let kept = ReminderPlanner.dedupe([
            event("a", "Standup", at(10), rowId: 1),
            event("b", "Standup", at(14), rowId: 2),
        ])
        XCTAssertEqual(kept.map(\.externalId), ["b"])
    }

    func testDedupeKeepsDailyRepeats() {
        let kept = ReminderPlanner.dedupe([
            event("a", "Standup", at(10), rowId: 1),
            event("b", "Standup", at(10, day: 12), rowId: 2),
        ])
        XCTAssertEqual(Set(kept.map(\.externalId)), ["a", "b"])
    }

    func testDedupeTreatsMissingRowIdAsZeroAndKeepsInputOrderOnTies() {
        let kept = ReminderPlanner.dedupe([
            event("a", "Standup", at(10)),
            event("b", "Standup", at(11)),
            event("c", "Other", at(11), rowId: nil),
        ])
        XCTAssertEqual(Set(kept.map(\.externalId)), ["a", "c"])
    }

    func testSkipsPastFireTimes() {
        XCTAssertEqual(plan(events: [event("e1", "Soon", at(8, 35))]), [])
    }

    func testCapsMeetingReminders() {
        let events = (0..<70).map { event("e\($0)", "Meeting \($0)", at(10).addingTimeInterval(Double($0) * 60)) }
        let planned = plan(events: events.reversed(), settings: ReminderSettings(digestEnabled: false))
        XCTAssertEqual(planned.count, 60)
        XCTAssertEqual(planned.first?.id, "meeting:e0")
        XCTAssertEqual(planned.last?.id, "meeting:e59")
    }

    func testDigestListsDueAndOverdueSteps() {
        let snap = snapshot(owe: [step("Send deck", due: "2026-11-11"), step("Call Sam", due: "2026-11-09"), step("Later", due: "2026-11-12")])
        let planned = plan(snapshots: [snap], now: at(7))
        XCTAssertEqual(planned.count, 1)
        XCTAssertEqual(planned[0].id, "digest")
        XCTAssertEqual(planned[0].fireAt, at(8))
        XCTAssertEqual(planned[0].title, "2 steps due today")
        XCTAssertEqual(planned[0].body, "Send deck\nCall Sam")
        XCTAssertEqual(planned[0].route, .topics)
    }

    func testDigestAfterTimeTargetsTomorrow() {
        let planned = plan(snapshots: [snapshot(owe: [step("Send deck", due: "2026-11-12")])])
        XCTAssertEqual(planned.count, 1)
        XCTAssertEqual(planned[0].fireAt, at(8, day: 12))
    }

    func testIgnoresMalformedDueDates() {
        let snap = snapshot(owe: [step("a", due: ""), step("b", due: "soon"), step("c", due: "2026-11"), step("d", due: nil)])
        XCTAssertEqual(plan(snapshots: [snap]), [])
    }

    func testSingleStepUsesSingular() {
        let planned = plan(snapshots: [snapshot(owe: [step("Send deck", due: "2026-11-11")])], now: at(7))
        XCTAssertEqual(planned[0].title, "1 step due today")
    }

    func testNothingOffCloud() {
        let snap = snapshot(owe: [step("Send deck", due: "2026-11-11")])
        XCTAssertEqual(plan(events: [event("e1", "Soon", at(10))], snapshots: [snap], isCloud: false, now: at(7)), [])
    }

    func testDisabledKindsAreSkipped() {
        let events = [event("e1", "Soon", at(10))]
        let snap = snapshot(owe: [step("Send deck", due: "2026-11-11")])
        let noMeetings = plan(events: events, snapshots: [snap], settings: ReminderSettings(meetingEnabled: false), now: at(7))
        XCTAssertEqual(noMeetings.map(\.id), ["digest"])
        let noDigest = plan(events: events, snapshots: [snap], settings: ReminderSettings(digestEnabled: false), now: at(7))
        XCTAssertEqual(noDigest.map(\.id), ["meeting:e1"])
    }

    func testMeetingsComeBeforeDigest() {
        let snap = snapshot(owe: [step("Send deck", due: "2026-11-12")])
        let planned = plan(events: [event("e1", "Soon", at(10))], snapshots: [snap])
        XCTAssertEqual(planned.map(\.id), ["meeting:e1", "digest"])
    }
}
