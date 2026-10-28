import XCTest
@testable import Whimbrel

final class ElapsedLabelTests: XCTestCase {
    func testFormats() {
        XCTAssertEqual(ElapsedLabel.text(0), "0:00")
        XCTAssertEqual(ElapsedLabel.text(83 * 60), "1:23")
        XCTAssertEqual(ElapsedLabel.text(15 * 3600 + 5 * 60), "15:05")
        XCTAssertEqual(ElapsedLabel.text(59), "0:00")
    }
}
