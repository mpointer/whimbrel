import XCTest
@testable import Whimbrel

final class DeviceLabelsTests: XCTestCase {
    private func record(_ key: String, name: String?) -> SyncDeviceRecord {
        SyncDeviceRecord(id: "id-\(key)", accountId: "a", publicKey: key, name: name, platform: "macos", status: "approved", enrolledAtUnixMs: 0, revokedAtUnixMs: nil)
    }

    func testNamesComeFromTheDeviceListKeyedByPublicKey() {
        let names = DeviceLabels.names(from: [record("keyA", name: " Mike's MacBook "), record("keyB", name: nil), record("keyC", name: "  ")])
        XCTAssertEqual(names, ["keyA": "Mike's MacBook"])
    }

    func testLabelPrefersTheNameThenThisPhoneThenAShortId() {
        let labels = DeviceLabels(names: ["keyA": "MacBook"], ownDeviceId: "keyOwn")
        XCTAssertEqual(labels.label(for: "keyA"), "MacBook")
        XCTAssertEqual(labels.label(for: "keyOwn"), "This iPhone")
        XCTAssertEqual(labels.label(for: "Hp7AHkXxyz"), "Device Hp7AHk")
    }
}
