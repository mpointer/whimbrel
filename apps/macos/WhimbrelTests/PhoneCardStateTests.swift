import XCTest
@testable import Whimbrel

final class PhoneCardStateTests: XCTestCase {
    private func device(_ platform: String?, status: String) -> SyncDeviceRecord {
        SyncDeviceRecord(id: UUID().uuidString, accountId: "a", publicKey: "k", name: nil, platform: platform,
                         status: status, enrolledAtUnixMs: 0, revokedAtUnixMs: nil)
    }

    func testApprovedIphoneIsConnected() {
        XCTAssertEqual(PhoneCardState.state(devices: [device("mac", status: "approved"), device("ios", status: "approved")]), .connected)
    }

    func testApprovedAndroidIsConnected() {
        XCTAssertEqual(PhoneCardState.state(devices: [device("android", status: "approved")]), .connected)
    }

    func testPendingPhoneStaysBeta() {
        XCTAssertEqual(PhoneCardState.state(devices: [device("ios", status: "pending")]), .beta)
    }

    func testRevokedPhoneStaysBeta() {
        XCTAssertEqual(PhoneCardState.state(devices: [device("android", status: "revoked")]), .beta)
    }

    func testApprovedMacOnlyStaysBeta() {
        XCTAssertEqual(PhoneCardState.state(devices: [device("mac", status: "approved"), device(nil, status: "approved")]), .beta)
    }

    func testTheCardWaitsForTheFirstDeviceListOnlyWhileSyncIsReady() {
        XCTAssertTrue(PhoneCardState.isLoading(ready: true, loaded: false, waitedOut: false))
        XCTAssertFalse(PhoneCardState.isLoading(ready: true, loaded: true, waitedOut: false))
        XCTAssertFalse(PhoneCardState.isLoading(ready: true, loaded: false, waitedOut: true))
        XCTAssertFalse(PhoneCardState.isLoading(ready: false, loaded: false, waitedOut: false))
    }

    func testNoDevicesIsBeta() {
        XCTAssertEqual(PhoneCardState.state(devices: []), .beta)
    }
}
