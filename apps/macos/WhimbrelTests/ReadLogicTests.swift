import XCTest
@testable import Whimbrel

final class ReadLogicTests: XCTestCase {
    private var calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "UTC")!
        return c
    }()

    private func date(_ hour: Int, _ minute: Int = 0) -> Date {
        calendar.date(from: DateComponents(year: 2026, month: 10, day: 6, hour: hour, minute: minute))!
    }

    func testDaypart() {
        XCTAssertEqual(ReadLogic.daypart(now: date(5), calendar: calendar), "morning")
        XCTAssertEqual(ReadLogic.daypart(now: date(11, 59), calendar: calendar), "morning")
        XCTAssertEqual(ReadLogic.daypart(now: date(12), calendar: calendar), "afternoon")
        XCTAssertEqual(ReadLogic.daypart(now: date(17), calendar: calendar), "evening")
    }

    func testGreetingUsesTheFirstNameOrIsNil() {
        XCTAssertEqual(ReadLogic.greeting(name: "Mike Pointer", now: date(9), calendar: calendar), "Good morning, Mike")
        XCTAssertNil(ReadLogic.greeting(name: "", now: date(9), calendar: calendar))
        XCTAssertNil(ReadLogic.greeting(name: "a@b.com", now: date(9), calendar: calendar))
    }

    func testRelativeStart() {
        let now = date(12)
        XCTAssertEqual(ReadLogic.relativeStart(now.addingTimeInterval(9 * 60), end: date(13), now: now), "In 9m")
        XCTAssertEqual(ReadLogic.relativeStart(date(11, 30), end: date(13), now: now), "Now")
        XCTAssertNil(ReadLogic.relativeStart(date(10), end: date(11), now: now))
    }

    func testAttendeeSummary() {
        XCTAssertEqual(ReadLogic.attendeeSummary(["A", "B", "C", "D"]), "A, B +2")
    }

    func testSyncLine() {
        let now = date(12)
        XCTAssertEqual(ReadLogic.syncLine(lastSyncedAt: nil, now: now, isSyncing: false, error: nil), "Not synced yet")
        XCTAssertEqual(ReadLogic.syncLine(lastSyncedAt: now.addingTimeInterval(-20), now: now, isSyncing: false, error: nil), "Synced just now")
        XCTAssertEqual(ReadLogic.syncLine(lastSyncedAt: now.addingTimeInterval(-120), now: now, isSyncing: false, error: nil), "Synced 2 min ago")
        XCTAssertEqual(ReadLogic.syncLine(lastSyncedAt: now.addingTimeInterval(-3 * 3600), now: now, isSyncing: false, error: nil), "Synced 3 h ago")
        XCTAssertEqual(ReadLogic.syncLine(lastSyncedAt: now, now: now, isSyncing: true, error: nil), "Syncing…")
        XCTAssertEqual(ReadLogic.syncLine(lastSyncedAt: now, now: now, isSyncing: false, error: "Couldn't sync."), "Couldn't sync.")
    }
}
