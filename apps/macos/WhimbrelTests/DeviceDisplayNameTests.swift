import XCTest
@testable import Whimbrel

final class DeviceDisplayNameTests: XCTestCase {
    func testNameWins() {
        XCTAssertEqual(DeviceDisplayName.name("Mike's Fold", platform: "android"), "Mike's Fold")
    }

    func testUnnamedAndroidIsAndroidPhone() {
        XCTAssertEqual(DeviceDisplayName.name(nil, platform: "android"), "Android phone")
        XCTAssertEqual(DeviceDisplayName.name("  ", platform: "android"), "Android phone")
    }

    func testOtherPlatformsAreNil() {
        XCTAssertNil(DeviceDisplayName.name(nil, platform: "mac"))
        XCTAssertNil(DeviceDisplayName.name(nil, platform: "ios"))
        XCTAssertNil(DeviceDisplayName.name(nil, platform: nil))
    }
}
