import XCTest
@testable import Whimbrel

final class MeetingPageLogicTests: XCTestCase {
    private var calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "America/New_York")!
        return c
    }()
    private let locale = Locale(identifier: "en_US")

    private func ms(_ y: Int, _ m: Int, _ d: Int, _ h: Int, _ min: Int = 0) -> Int64 {
        let date = calendar.date(from: DateComponents(year: y, month: m, day: d, hour: h, minute: min))!
        return Int64(date.timeIntervalSince1970 * 1000)
    }

    func testMeetingsGroupUnderTodayYesterdayWeekdayAndDate() {
        let now = Date(timeIntervalSince1970: Double(ms(2026, 10, 6, 12)) / 1000) // Tue Oct 6
        let groups = MeetingDayGrouping.groups([
            (id: 1, startUnixMs: ms(2026, 10, 6, 9)),
            (id: 2, startUnixMs: ms(2026, 10, 6, 8)),
            (id: 3, startUnixMs: ms(2026, 10, 5, 23)),
            (id: 4, startUnixMs: ms(2026, 10, 2, 17)),
            (id: 5, startUnixMs: ms(2026, 9, 20, 10)),
        ], now: now, calendar: calendar, locale: locale)
        XCTAssertEqual(groups, [
            MeetingDayGroup(title: "Today", ids: [1, 2]),
            MeetingDayGroup(title: "Yesterday", ids: [3]),
            MeetingDayGroup(title: "Friday", ids: [4]),
            MeetingDayGroup(title: "Sep 20, 2026", ids: [5]),
        ])
    }

    func testTheDayBoundaryFollowsTheCalendarTimeZoneNotUTC() {
        // 11:30 PM Eastern on Oct 5 is already Oct 6 in UTC; it must still be "Yesterday".
        let now = Date(timeIntervalSince1970: Double(ms(2026, 10, 6, 7)) / 1000)
        let groups = MeetingDayGrouping.groups([(id: 1, startUnixMs: ms(2026, 10, 5, 23, 30))], now: now, calendar: calendar, locale: locale)
        XCTAssertEqual(groups.map(\.title), ["Yesterday"])
    }

    func testGroupsKeepTheInputOrderWithinADay() {
        let now = Date(timeIntervalSince1970: Double(ms(2026, 10, 6, 12)) / 1000)
        let groups = MeetingDayGrouping.groups([(id: 9, startUnixMs: ms(2026, 10, 6, 11)), (id: 3, startUnixMs: ms(2026, 10, 6, 10))], now: now, calendar: calendar, locale: locale)
        XCTAssertEqual(groups.first?.ids, [9, 3])
    }

    func testAskIsDisabledUntilTheMeetingIsCompletedAndTranscribed() {
        XCTAssertNotNil(MeetingAskAvailability.disabledReason(status: .recording, isProcessing: false, segmentCount: 0))
        XCTAssertNotNil(MeetingAskAvailability.disabledReason(status: .completed, isProcessing: true, segmentCount: 0))
        XCTAssertNotNil(MeetingAskAvailability.disabledReason(status: .completed, isProcessing: false, segmentCount: 0))
        XCTAssertNil(MeetingAskAvailability.disabledReason(status: .completed, isProcessing: false, segmentCount: 12))
    }

    func testOnlyTranscriptCitationsBecomeSegmentIdsInOrderWithoutDuplicates() {
        let refs = [
            CoreContextItemRef(kind: .transcript, id: 40),
            CoreContextItemRef(kind: .capture, id: 7),
            CoreContextItemRef(kind: .transcript, id: 12),
            CoreContextItemRef(kind: .transcript, id: 40),
        ]
        XCTAssertEqual(TranscriptCitations.segmentIds(from: refs), [40, 12])
        XCTAssertEqual(TranscriptCitations.segmentIds(from: []), [])
    }

    func testAskIsAllowedWhileRecordingOnceLiveSegmentsExist() {
        XCTAssertNil(MeetingAskAvailability.disabledReason(status: .recording, isProcessing: false, segmentCount: 3))
        XCTAssertEqual(
            MeetingAskAvailability.disabledReason(status: .recording, isProcessing: false, segmentCount: 0),
            "Listening. The transcript starts appearing after the first minute."
        )
    }
}
