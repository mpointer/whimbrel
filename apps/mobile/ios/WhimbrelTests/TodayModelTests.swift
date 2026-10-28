import XCTest
@testable import Whimbrel

@MainActor
final class TodayModelTests: XCTestCase {
    private var calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "UTC")!
        return c
    }()
    private lazy var now = calendar.date(from: DateComponents(year: 2026, month: 10, day: 19, hour: 9, minute: 0))!
    private func ms(_ d: Date) -> Int64 { Int64(d.timeIntervalSince1970 * 1000) }

    private func run(_ id: Int64, _ name: String, _ status: String, _ at: Date) -> PhoneRoutineRun {
        PhoneRoutineRun(id: id, routineName: name, ranAtUnixMs: ms(at), status: status, outputSummary: "out \(id)", originDeviceId: nil)
    }

    private func model(store: FakeStore?) -> (TodayModel, SyncModel) {
        let sync = SyncModel(store: store, credentials: FakeCredentials())
        return (TodayModel(), sync)
    }

    func testBriefRunPicksTheNewestSuccessfulDailyBriefOfToday() {
        let runs = [
            run(1, "Daily Brief", "success", now.addingTimeInterval(-3600)),
            run(2, "Daily Brief", "success", now.addingTimeInterval(-600)),
            run(3, "Daily Brief", "failed", now.addingTimeInterval(-60)),
            run(4, "Meeting Prep", "success", now.addingTimeInterval(-30)),
            run(5, "Daily Brief", "success", now.addingTimeInterval(-86_400)),
        ]
        XCTAssertEqual(TodayModel.briefRun(from: runs, now: now, calendar: calendar)?.id, 2)
        XCTAssertNil(TodayModel.briefRun(from: [runs[4]], now: now, calendar: calendar))
    }

    func testReloadFillsUpNextPrepAndRecentFromTheStore() async throws {
        let store = FakeStore()
        store.meetings = (1...12).map { PhoneMeeting(id: Int64($0), deviceId: "d", startUnixMs: ms(now) - Int64($0) * 3_600_000, title: "M\($0)") }
        store.events = [
            PhoneCalendarEvent(externalId: "later", source: "google_calendar", title: "Later", startUnixMs: ms(now.addingTimeInterval(7200)), attendees: []),
            PhoneCalendarEvent(externalId: "soon", source: "google_calendar", title: "Soon", startUnixMs: ms(now.addingTimeInterval(1800)), attendees: ["a@x.com"]),
            PhoneCalendarEvent(externalId: "tomorrow", source: "google_calendar", title: "Tomorrow", startUnixMs: ms(now.addingTimeInterval(86_400)), attendees: []),
        ]
        store.preps["soon"] = try JSONDecoder().decode(PhoneMeetingPrep.self, from: Data(#"{"event_id":"soon","title":"Soon","start_unix_ms":1,"brief_json":"{}"}"#.utf8))
        store.runs = [run(7, "Daily Brief", "success", now.addingTimeInterval(-60))]
        let (model, sync) = model(store: store)
        await model.reload(sync: sync, now: now, calendar: calendar)
        XCTAssertEqual(model.upNext?.externalId, "soon", "the soonest event today; tomorrow's is not Up next")
        XCTAssertEqual(model.prep?.eventId, "soon")
        XCTAssertEqual(model.recent.count, 10)
        XCTAssertEqual(model.brief?.id, 7)
        XCTAssertTrue(model.loaded)
    }

    func testReloadWithNoStoreLeavesEverythingEmpty() async {
        let (model, sync) = model(store: nil)
        await model.reload(sync: sync, now: now, calendar: calendar)
        XCTAssertNil(model.upNext)
        XCTAssertNil(model.brief)
        XCTAssertTrue(model.recent.isEmpty)
        XCTAssertTrue(model.loaded)
    }
}
