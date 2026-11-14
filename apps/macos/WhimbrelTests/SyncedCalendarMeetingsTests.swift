import XCTest
@testable import Whimbrel

/// Calendar events synced from the user's other devices show in Today, At a Glance, Upcoming and
/// Meeting Prep next to this Mac's own, and never reach the meeting trigger. Which synced rows
/// count (own device, removed, duplicates) is core's `synced_meetings_json`, tested in Rust; these
/// pin the Mac side: the merge with the local list, the window cuts, the panel rules and the
/// trigger exclusion.
@MainActor
final class SyncedCalendarMeetingsTests: XCTestCase {
    private var now: Date {
        Calendar.current.date(bySettingHour: 8, minute: 0, second: 0, of: Date(timeIntervalSince1970: 1_761_000_000))!
    }

    private func meeting(id: String, title: String, start: Date, source: MeetingSource = .calendar, response: SelfResponse? = nil, allDay: Bool = false) -> QualifyingMeeting {
        QualifyingMeeting(
            id: id, title: title, startDate: start, endDate: start.addingTimeInterval(1800),
            attendees: ["Ana"], conferenceLink: nil, source: source, appBundleID: nil, organizerEmail: nil, organizerName: nil,
            selfIsOrganizer: false, selfResponse: response, isAllDay: allDay
        )
    }

    func testTodayShowsASyncedMeetingBesideLocalOnesInStartOrder() {
        let local = meeting(id: "ek-1", title: "Standup", start: now.addingTimeInterval(3 * 3600))
        let synced = meeting(id: "g1", title: "Genesys Architecture Follow Up", start: now.addingTimeInterval(3600))
        let result = AppState.todaysMeetings(eventKit: [local], oauth: [], now: now, forPanel: true, synced: [synced])
        XCTAssertEqual(result.map(\.id), ["g1", "ek-1"])
        XCTAssertEqual(AppState.todaysMeetings(eventKit: [local], oauth: [], now: now, synced: [synced]).map(\.id), ["g1", "ek-1"])
    }

    func testASyncedMeetingAfterTodayIsNotInTodaysList() {
        let tomorrow = meeting(id: "g2", title: "Tomorrow", start: now.addingTimeInterval(30 * 3600))
        XCTAssertTrue(AppState.todaysMeetings(eventKit: [], oauth: [], now: now, forPanel: true, synced: [tomorrow]).isEmpty)
    }

    func testAlreadyStartedSyncedMeetingStaysLikeALocalOne() {
        let started = meeting(id: "g3", title: "Early", start: now.addingTimeInterval(-1800))
        XCTAssertEqual(AppState.todaysMeetings(eventKit: [], oauth: [], now: now, synced: [started]).map(\.id), ["g3"])
    }

    func testUpcomingGainsSyncedMeetingsInTheWeekWindowOnly() {
        let week = CalendarService.weekWindow(now: now)
        let inside = meeting(id: "g4", title: "Inside", start: week.lowerBound.addingTimeInterval(3600))
        let outside = meeting(id: "g5", title: "Outside", start: week.upperBound.addingTimeInterval(3600))
        let local = meeting(id: "wk-1", title: "Planning", start: week.lowerBound.addingTimeInterval(7200))
        let result = AppState.weekMeetings(eventKit: [local], oauth: [], now: now, synced: [inside, outside])
        XCTAssertEqual(result.map(\.id), ["g4", "wk-1"])
    }

    func testNoSyncedListChangesNothing() {
        let local = meeting(id: "ek-1", title: "Standup", start: now.addingTimeInterval(3600))
        XCTAssertEqual(AppState.todaysMeetings(eventKit: [local], oauth: [], now: now, forPanel: true), [local])
        XCTAssertEqual(AppState.weekMeetings(eventKit: [local], oauth: [], now: now), [])
    }

    /// The trigger reads `triggerMeetings`, which has no synced input: a meeting that exists only
    /// on another device is inside its lead window and still raises no prompt.
    func testTheTriggerNeverPromptsForASyncedOnlyMeeting() {
        let suite = "SyncedCalendarMeetingsTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let synced = meeting(id: "g6", title: "Genesys Architecture Follow Up", start: now)

        let display = AppState.todaysMeetings(eventKit: [], oauth: [], now: now, synced: [synced])
        XCTAssertEqual(display.map(\.id), ["g6"], "the panel and prep see it")

        let service = MeetingTriggerService(defaults: defaults)
        service.evaluate(meetings: AppState.triggerMeetings(eventKit: [], oauth: [], now: now), now: now)
        XCTAssertNil(service.pendingPrompt, "the trigger does not")

        // Control: the same meeting from this Mac's own calendar does prompt.
        let own = MeetingTriggerService(defaults: defaults)
        own.evaluate(meetings: AppState.triggerMeetings(eventKit: [synced], oauth: [], now: now), now: now)
        XCTAssertEqual(own.pendingPrompt?.id, "g6")
    }

    // MARK: - Through the bridge, against a real temp store

    private func tempStore() throws -> (CoreMemoryStore, URL) {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return (try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("test.sqlite"), modelsDir: nil)), dir)
    }

    private func item(device: String, source: String, id: String, start: Date, title: String) -> NewCalendarEventItemInput {
        NewCalendarEventItemInput(deviceId: device, source: source, externalId: "\(id)#\(Int(start.timeIntervalSince1970))", tsUnixMs: Int64(start.timeIntervalSince1970 * 1000), title: title, actors: ["Ana"])
    }

    func testSyncedMeetingsThroughTheBridgeDedupeAndSkipThisDevice() throws {
        let (store, dir) = try tempStore()
        defer { try? FileManager.default.removeItem(at: dir) }
        let nine = now.addingTimeInterval(3600), half = now.addingTimeInterval(2.5 * 3600)
        XCTAssertTrue(store.upsertCalendarEventItems([
            item(device: "mac-genesys", source: "apple_calendar", id: "EK-9", start: nine, title: "Genesys Architecture Follow Up"),
            item(device: "mac-genesys", source: "apple_calendar", id: "EK-10", start: half, title: "FMA Partner Connect"),
            item(device: "mac-genesys", source: "google_calendar", id: "GG-10", start: half, title: "FMA Partner Connect"),
            item(device: "mac-here", source: "apple_calendar", id: "EK-11", start: nine.addingTimeInterval(7200), title: "Mine"),
            item(device: "mac-genesys", source: "apple_calendar", id: "EK-12", start: nine.addingTimeInterval(9000), title: "Standup"),
        ]))
        let local = meeting(id: "L-12", title: "STANDUP", start: nine.addingTimeInterval(9000))
        let found = try XCTUnwrap(store.syncedMeetings(
            excludeDeviceId: "mac-here", from: Calendar.current.startOfDay(for: now), until: now.addingTimeInterval(86_400), local: [local]
        ))
        XCTAssertEqual(found.map(\.title), ["Genesys Architecture Follow Up", "FMA Partner Connect"], "own row and the locally shown Standup are out; the twin pair is one row")
        XCTAssertEqual(found[1].source, .googleCalendar)
        XCTAssertEqual(found[0].id, "EK-9")
        XCTAssertEqual(found[0].endDate.timeIntervalSince(found[0].startDate), 1800, accuracy: 0.5)
        XCTAssertEqual(found[0].attendees, ["Ana"])
    }
}
