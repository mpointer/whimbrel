import XCTest
@testable import Whimbrel

final class DeviceComparisonCodeTests: XCTestCase {
    func testPinnedCodePassesThroughCoreBridge() {
        // Key = bytes 0..<32; vector computed independently (see core/sync comparison_code tests).
        XCTAssertEqual(CoreMemoryStore.deviceComparisonCode(publicKeyBase64: "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="), "CC · 6W · TA · B6 · RG")
    }

    func testInvalidKeyIsNil() {
        XCTAssertNil(CoreMemoryStore.deviceComparisonCode(publicKeyBase64: "not base64!"))
    }
}

final class DeviceCodeCollisionTests: XCTestCase {
    func testSharedPendingCodesCollide() {
        let ids = collidingPendingDeviceIDs([("a", "pending", "X"), ("b", "pending", "X"), ("c", "pending", "Y")])
        XCTAssertEqual(ids, ["a", "b"])
    }

    func testNilCodesAndNonPendingNeverCollide() {
        let ids = collidingPendingDeviceIDs([("a", "pending", nil), ("b", "pending", nil), ("c", "approved", "X"), ("d", "pending", "X")])
        XCTAssertTrue(ids.isEmpty)
    }
}
