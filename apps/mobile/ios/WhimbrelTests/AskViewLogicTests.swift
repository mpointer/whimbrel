import XCTest
@testable import Whimbrel

final class AskViewLogicTests: XCTestCase {
    private func card(kind: String, id: Int64 = 1, title: String = "Acme pricing sync", subtitle: String = "Meeting", ts: Int64? = nil,
                      meetingId: Int64? = nil, deviceId: String? = nil, found: Bool = true) -> CitationCard {
        CitationCard(kind: kind, id: id, title: title, subtitle: subtitle, tsUnixMs: ts, meetingId: meetingId, deviceId: deviceId, found: found)
    }

    func test_target_for_transcript_with_meeting_is_meeting() {
        XCTAssertEqual(CitationTarget.for(card(kind: "transcript", meetingId: 7)), .meeting(id: 7))
        XCTAssertEqual(CitationTarget.for(card(kind: "snapshot", meetingId: 8)), .meeting(id: 8))
        XCTAssertEqual(CitationTarget.for(card(kind: "transcript")), .none)
    }

    func test_target_for_capture_and_routine_run() {
        XCTAssertEqual(CitationTarget.for(card(kind: "capture", id: 5)), .capture(id: 5))
        XCTAssertEqual(CitationTarget.for(card(kind: "routine_run", id: 6)), .routineRun(id: 6))
        XCTAssertEqual(CitationTarget.for(card(kind: "episode", id: 9)), .none)
    }

    func test_target_for_not_found_is_none() {
        XCTAssertEqual(CitationTarget.for(card(kind: "transcript", meetingId: 7, found: false)), .none)
        XCTAssertEqual(CitationTarget.for(card(kind: "capture", found: false)), .none)
        XCTAssertEqual(CitationTarget.for(card(kind: "routine_run", found: false)), .none)
    }

    func test_chip_label_format() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let locale = Locale(identifier: "en_US")
        // Tue 2026-10-27 12:00 UTC; "now" is Thu 2026-10-29.
        let tuesday: Int64 = 1_793_102_400_000
        let now = Date(timeIntervalSince1970: 1_793_102_400 + 2 * 86_400)
        func label(_ c: CitationCard, device: String? = nil) -> String {
            CitationChipLabel.text(for: c, deviceName: device, now: now, calendar: calendar, locale: locale)
        }
        XCTAssertEqual(label(card(kind: "transcript", ts: tuesday)), "Meeting · Acme pricing sync · Tue")
        XCTAssertEqual(label(card(kind: "capture", title: "Pricing.xlsx", subtitle: "Screen", ts: tuesday), device: "MacBook"),
                       "Screen · MacBook · Pricing.xlsx · Tue")
        // No date, no device, no subtitle: those parts are left out.
        XCTAssertEqual(label(card(kind: "transcript", subtitle: "")), "Acme pricing sync")
        // Older than a week: month and day.
        let old = tuesday - 20 * 86_400_000
        XCTAssertEqual(label(card(kind: "transcript", ts: old)), "Meeting · Acme pricing sync · Oct 7")
        // Not found: the kind stays, so "Meeting" or "Screen" still shows.
        XCTAssertEqual(label(card(kind: "capture", title: "Pricing.xlsx", subtitle: "Screen", ts: tuesday, found: false), device: "MacBook"), "Screen · Not on this iPhone")
        XCTAssertEqual(label(card(kind: "transcript", subtitle: "", found: false)), "Not on this iPhone")
    }

    func test_ask_stops_only_when_leaving_the_ask_tab() {
        XCTAssertTrue(RootView.shouldStopAsk(old: 1, new: 0))
        XCTAssertTrue(RootView.shouldStopAsk(old: 1, new: 2))
        XCTAssertFalse(RootView.shouldStopAsk(old: 0, new: 1))
        XCTAssertFalse(RootView.shouldStopAsk(old: 2, new: 3))
    }
}
