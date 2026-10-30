import XCTest
@testable import Whimbrel

@MainActor
final class DeviceApprovalTests: XCTestCase {
    var backend: FakeBackend!
    var phone: CoreSyncDeviceKeypair!
    var groupKey: String!

    override func setUp() async throws {
        backend = FakeBackend()
        let service = (Bundle.main.bundleIdentifier ?? "com.mpointer.kestrel") + ".sync"
        for account in ["device-secret-key", "group-key"] {
            KeychainAccess.delete([kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: account] as CFDictionary)
        }
        phone = CoreMemoryStore.generateSyncDeviceKeypair()!
        groupKey = CoreMemoryStore.generateSyncGroupKey()!
        try SyncKeychain.saveDeviceSecretKey(phone.secretKeyBase64)
        try SyncKeychain.saveGroupKey(groupKey)
    }

    private func record(_ id: String, key: String, status: String = "pending", name: String? = nil, platform: String? = "mac") -> SyncDeviceRecord {
        SyncDeviceRecord(id: id, accountId: "acct", publicKey: key, name: name, platform: platform, status: status, enrolledAtUnixMs: 0, revokedAtUnixMs: nil)
    }

    private func makeModel(sleeper: FakeSleeper = FakeSleeper()) -> DeviceApprovalModel {
        DeviceApprovalModel(backend: backend, sleep: { try await sleeper.sleep($0) })
    }

    func test_pending_devices_listed_with_code() async throws {
        let newMac = CoreMemoryStore.generateSyncDeviceKeypair()!
        backend.devices = [record("me", key: phone.publicKeyBase64, status: "approved"),
                           record("new", key: newMac.publicKeyBase64, name: "iMac")]
        let model = makeModel()
        await model.refresh()
        XCTAssertEqual(model.rows.map(\.device.id), ["new"], "the phone itself is not listed")
        let code = try XCTUnwrap(CoreMemoryStore.deviceComparisonCode(publicKeyBase64: newMac.publicKeyBase64))
        XCTAssertEqual(model.rows[0].codeLine, "Check this matches the code on the new device: \(code)")
        XCTAssertTrue(model.rows[0].canApprove)
        XCTAssertEqual(model.pendingCount, 1)
    }

    func test_approve_wraps_key_for_target_public_key() async throws {
        let target = CoreMemoryStore.generateSyncDeviceKeypair()!
        backend.devices = [record("new", key: target.publicKeyBase64)]
        let model = makeModel()
        await model.refresh()
        await model.approve(model.rows[0].device)
        let sent = try XCTUnwrap(backend.approvals.first)
        XCTAssertEqual(sent.targetDeviceId, "new")
        // Only the target (with this phone's public key) can open it, and it holds the phone's group key.
        XCTAssertEqual(CoreMemoryStore.unwrapSyncGroupKey(recipientSecretKeyBase64: target.secretKeyBase64, senderPublicKeyBase64: phone.publicKeyBase64, wrappedGroupKeyBase64: sent.wrappedGroupKey), groupKey)
        XCTAssertNil(model.actionError)
    }

    func test_code_collision_warns_like_mac() async throws {
        let same = CoreMemoryStore.generateSyncDeviceKeypair()!.publicKeyBase64
        backend.devices = [record("a", key: same), record("b", key: same)]
        let model = makeModel()
        await model.refresh()
        XCTAssertEqual(model.rows.map(\.collides), [true, true])
        XCTAssertEqual(model.rows.map(\.canApprove), [false, false])
        XCTAssertEqual(DeviceApprovalModel.collisionWarning, "Another pending device shows the same code — don't approve it")
        await model.approve(model.rows[0].device)
        XCTAssertTrue(backend.approvals.isEmpty, "a colliding device is never approved")
    }

    func test_approve_while_in_flight_is_ignored() async throws {
        let target = CoreMemoryStore.generateSyncDeviceKeypair()!
        backend.devices = [record("new", key: target.publicKeyBase64)]
        let model = makeModel()
        await model.refresh()
        let device = model.rows[0].device
        async let a: Void = model.approve(device)
        async let b: Void = model.approve(device)
        _ = await (a, b)
        XCTAssertEqual(backend.approvals.count, 1)
        XCTAssertFalse(model.approving)
    }

    func test_resetting_device_shows_resetting_and_no_approve() async throws {
        backend.devices = [record("c", key: CoreMemoryStore.generateSyncDeviceKeypair()!.publicKeyBase64, status: "resetting")]
        let model = makeModel()
        await model.refresh()
        XCTAssertEqual(model.rows[0].statusLine, "Resetting…")
        XCTAssertFalse(model.rows[0].canApprove)
        XCTAssertEqual(model.pendingCount, 0)
    }

    func test_polling_sleeps_5_seconds_between_refreshes_until_the_sleep_throws() async throws {
        let sleeper = FakeSleeper()
        sleeper.limit = 2
        let model = makeModel(sleeper: sleeper)
        await model.run()
        XCTAssertEqual(sleeper.intervals, [5, 5, 5])
    }
}
