import XCTest
@testable import Whimbrel

final class LocalOnlyLineTests: XCTestCase {
    func testOnText() {
        XCTAssertEqual(LocalOnlyLine.text(localOnly: true), "Local-only mode: On")
    }

    func testOffText() {
        XCTAssertEqual(LocalOnlyLine.text(localOnly: false), "Local-only mode: Off")
    }
}
