import XCTest
@testable import Whimbrel

final class WhimbrelShellNavigationTests: XCTestCase {
    func testAllFiveTabsAreDistinct() {
        let tabs: [WhimbrelTab] = [.today, .meetings, .library, .automations, .settings]
        XCTAssertEqual(Set(tabs.map(\.rawValue)).count, tabs.count)
    }
}
