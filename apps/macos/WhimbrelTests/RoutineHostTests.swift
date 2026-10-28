import XCTest
@testable import Whimbrel

@MainActor
final class RoutineHostTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let suite = "routine-host-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return d
    }

    // MARK: which gate this device gets

    func testAnUnsetPrimaryMeansEveryDeviceRunsEverything() {
        XCTAssertEqual(RoutineHost.gate(primaryId: "", ownId: "me", approvedDeviceIds: ["me", "other"]), .full)
        XCTAssertEqual(RoutineHost.gate(primaryId: "  ", ownId: "me", approvedDeviceIds: ["me", "other"]), .full)
    }

    func testTheChosenDeviceRunsEverythingAndAnotherApprovedDeviceStandsBy() {
        XCTAssertEqual(RoutineHost.gate(primaryId: "me", ownId: "me", approvedDeviceIds: ["me", "other"]), .full)
        XCTAssertEqual(RoutineHost.gate(primaryId: "other", ownId: "me", approvedDeviceIds: ["me", "other"]), .failover)
    }

    func testARevokedOrUnknownPrimaryNeverStopsRoutines() {
        XCTAssertEqual(RoutineHost.gate(primaryId: "gone", ownId: "me", approvedDeviceIds: ["me"]), .full, "revoked or unknown behaves as unset")
    }

    func testTheFallbackDeviceIdIsNeverTreatedAsAnotherDevice() {
        XCTAssertEqual(RoutineHost.gate(primaryId: "other", ownId: "spike-device", approvedDeviceIds: ["other"]), .full)
    }

    func testTheGateBytesMatchTheCoreEncoding() {
        XCTAssertEqual(RoutineGate.off.rawValue, 0)
        XCTAssertEqual(RoutineGate.full.rawValue, 1)
        XCTAssertEqual(RoutineGate.failover.rawValue, 2)
    }

    // MARK: setting and sync

    func testThePrimaryPersistsAndUsesAStableSyncKey() {
        let d = defaults()
        let a = RoutineHostSettings(defaults: d)
        XCTAssertEqual(a.primaryDeviceId, "")
        a.primaryDeviceId = "dev-1"
        XCTAssertEqual(RoutineHostSettings(defaults: d).primaryDeviceId, "dev-1")
        XCTAssertEqual(RoutineHostSettings.fieldKey, "routines.primaryDeviceId")
    }

    func testASyncedValueIsAppliedAndAnotherKeyIsRejected() {
        let s = RoutineHostSettings(defaults: defaults())
        XCTAssertTrue(s.apply(fieldKey: "routines.primaryDeviceId", value: "dev-2"))
        XCTAssertEqual(s.primaryDeviceId, "dev-2")
        XCTAssertTrue(s.apply(fieldKey: "routines.primaryDeviceId", value: ""))
        XCTAssertEqual(s.primaryDeviceId, "")
        XCTAssertFalse(s.apply(fieldKey: "routines.other", value: "x"))
    }

    // MARK: runs from other devices

    func testARunDecodesWithAndWithoutItsOriginDevice() throws {
        let base = #"{"id":1,"routine_id":2,"routine_name":"Daily Brief","ran_at_unix_ms":3,"status":"success","output_summary":"s","output_json":"{}""#
        let local = try JSONDecoder().decode(CoreRoutineRunWithName.self, from: Data((base + "}").utf8))
        XCTAssertNil(local.originDeviceId)
        let synced = try JSONDecoder().decode(CoreRoutineRunWithName.self, from: Data((base + #","origin_device_id":"dev-9"}"#).utf8))
        XCTAssertEqual(synced.originDeviceId, "dev-9")
    }
}
