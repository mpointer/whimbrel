import XCTest
@testable import Whimbrel

/// Smoke tests for the CoreBridge wrappers over core/api::connectors; the decoding,
/// merge, fold and mapping cases are Rust tests. These pin the QualifyingMeeting JSON
/// round trip (dates as Unix milliseconds, enum raw values) and the request/apply
/// plumbing against a real temp store.
final class ConnectorBridgeTests: XCTestCase {
    private func meeting(id: String, title: String, start: Date, source: MeetingSource) -> QualifyingMeeting {
        QualifyingMeeting(
            id: id, title: title, startDate: start, endDate: start.addingTimeInterval(1800),
            attendees: ["alice@x.com"], conferenceLink: nil, source: source, appBundleID: nil, organizerEmail: nil, organizerName: nil, selfIsOrganizer: false, selfResponse: nil
        )
    }

    func testMergeMeetingsRoundTripsThroughTheBridge() {
        let start = Date(timeIntervalSince1970: 1_754_413_200)
        let ek = meeting(id: "ek-1", title: "Standup", start: start, source: .calendar)
        let oauth = meeting(id: "oauth-1", title: "STANDUP", start: start, source: .googleCalendar)
        let merged = CoreMemoryStore.mergeMeetings(eventKit: [ek], oauth: [oauth])
        XCTAssertEqual(merged, [oauth])
    }

    /// The Google Calendar connector keeps every future occurrence of a recurring
    /// meeting (out to years ahead); EventKit only covers the rest of today. The merged
    /// list feeds the Daily Brief's "Today:" line, so it must stop at today's end too.
    func testTodaysMeetingsDropsOAuthMeetingsAfterToday() {
        let cal = Calendar.current
        let now = cal.date(bySettingHour: 9, minute: 0, second: 0, of: Date(timeIntervalSince1970: 1_761_000_000))!
        let laterToday = meeting(id: "o1", title: "Partner Connect", start: now.addingTimeInterval(3600), source: .googleCalendar)
        let tomorrow = meeting(id: "o2", title: "Partner Connect", start: now.addingTimeInterval(86_400), source: .googleCalendar)
        let in2040 = meeting(id: "o3", title: "Partner Connect", start: now.addingTimeInterval(14 * 365 * 86_400), source: .googleCalendar)
        let ek = meeting(id: "ek-1", title: "Standup", start: now.addingTimeInterval(1800), source: .calendar)
        XCTAssertEqual(AppState.todaysMeetings(eventKit: [ek], oauth: [in2040, tomorrow, laterToday], now: now), [ek, laterToday])
    }

    func testCalendarEventItemAndLinkDetectionThroughTheBridge() {
        let m = meeting(id: "evt-123", title: "Planning", start: Date(timeIntervalSince1970: 1_000), source: .calendar)
        XCTAssertEqual(CoreMemoryStore.calendarEventItem(for: m, deviceId: "dev-1"), NewCalendarEventItemInput(
            deviceId: "dev-1", source: "apple_calendar", externalId: "evt-123#1000", tsUnixMs: 1_000_000, title: "Planning", actors: ["alice@x.com"]
        ))
        XCTAssertNil(CoreMemoryStore.calendarEventItem(for: meeting(id: "d", title: "D", start: Date(), source: .detected), deviceId: "dev-1"))
        XCTAssertEqual(
            MeetingLinkDetector.detectLink(location: "join at https://zoom.us/j/123 (password 4567).", notes: nil, url: nil),
            DetectedConferenceLink(provider: .zoom, url: URL(string: "https://zoom.us/j/123")!)
        )
        XCTAssertNil(MeetingLinkDetector.detectLink(location: "Room 4", notes: nil, url: nil))
    }

    func testCalendarAndGmailRequestApplyAgainstATempStore() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = CoreMemoryStore(path: dir.appendingPathComponent("test.sqlite"), modelsDir: nil)!

        XCTAssertEqual(store.calendarSyncRequest(accountEmail: "a@x.com")?.argsJson, "{}")
        let result = """
        {"items": [{"id": "evt-1", "summary": "Active", "start": {"dateTime": "2100-01-01T10:00:00Z"}, "end": {"dateTime": "2100-01-01T11:00:00Z"}, "hangoutLink": "https://meet.google.com/abc"}], "nextSyncToken": "tok-1"}
        """
        let outcome = try XCTUnwrap(store.applyCalendarSync(accountEmail: "a@x.com", resultJSON: result, existing: [], nowUnixMs: 0))
        XCTAssertTrue(outcome.tokenSaved)
        XCTAssertEqual(outcome.meetings.map(\.id), ["evt-1"])
        XCTAssertEqual(outcome.meetings.first?.conferenceLink?.provider, .googleMeet)
        XCTAssertTrue(store.calendarSyncRequest(accountEmail: "a@x.com")?.argsJson.contains("tok-1") ?? false)

        XCTAssertEqual(store.gmailSyncRequest(accountEmail: "a@x.com")?.tool, "sync_messages")
        let gmail = try XCTUnwrap(store.applyGmailSync(accountEmail: "a@x.com", deviceId: "dev-1", resultJSON: #"{"messages": [{"id": "m1", "threadId": "t1", "from": "a@x.com", "to": "me@x.com", "subject": "Hi", "internalDate": "1700000000000", "bodyText": "Hello"}], "nextHistoryId": "105"}"#))
        XCTAssertEqual(gmail.upserted, 1)
        XCTAssertTrue(gmail.historySaved)
    }
}
