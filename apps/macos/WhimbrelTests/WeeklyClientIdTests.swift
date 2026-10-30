import XCTest
@testable import Whimbrel

final class WeeklyClientIdTests: XCTestCase {
    private func freshDefaults() -> UserDefaults {
        let name = "WeeklyClientIdTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        addTeardownBlock { defaults.removePersistentDomain(forName: name) }
        return defaults
    }

    private func date(_ iso: String) -> Date { ISO8601DateFormatter().date(from: iso)! }

    func test_same_week_same_id() {
        let defaults = freshDefaults()
        let monday = WeeklyClientId.current(now: date("2026-10-26T00:00:00Z"), defaults: defaults)
        let sunday = WeeklyClientId.current(now: date("2026-11-01T23:59:59Z"), defaults: defaults)
        XCTAssertEqual(monday, sunday)
        XCTAssertEqual(defaults.string(forKey: "com.mpointer.kestrel.weeklyClientIdWeek"), "2026-W44")
    }

    func test_new_week_new_id() {
        let defaults = freshDefaults()
        let first = WeeklyClientId.current(now: date("2026-11-01T23:59:59Z"), defaults: defaults)
        let next = WeeklyClientId.current(now: date("2026-11-02T00:00:00Z"), defaults: defaults)
        XCTAssertNotEqual(first, next)
        XCTAssertEqual(defaults.string(forKey: "com.mpointer.kestrel.weeklyClientId"), next)
    }

    func test_id_is_32_lowercase_hex() {
        let id = WeeklyClientId.current(now: Date(), defaults: freshDefaults())
        XCTAssertNotNil(id.range(of: "^[0-9a-f]{32}$", options: .regularExpression))
    }

    /// The ISO year differs from the calendar year around New Year, and the week rolls at UTC midnight.
    func test_iso_week_uses_the_iso_year_in_utc() {
        XCTAssertEqual(WeeklyClientId.week(of: date("2027-01-01T12:00:00Z")), "2026-W53")
    }
}
