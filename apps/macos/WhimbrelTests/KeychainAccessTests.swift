import Security
import XCTest
@testable import Whimbrel

/// `KeychainAccess` is the only door to the Keychain: in a unit-test host it must never
/// reach the real one (every test run launches a freshly built, differently signed app,
/// and the real Keychain would ask for the user's password each time).
final class KeychainAccessTests: XCTestCase {
    private func query(_ service: String, _ account: String, returnData: Bool = false) -> CFDictionary {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        if returnData { q[kSecReturnData as String] = true }
        return q as CFDictionary
    }

    private func add(_ store: InMemoryKeychain, _ service: String, _ account: String, _ value: String) -> OSStatus {
        let attributes: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecValueData as String: Data(value.utf8),
        ]
        return store.add(attributes as CFDictionary, nil)
    }

    func testAddThenReadRoundTripsAndIsPerServiceAndAccount() {
        let store = InMemoryKeychain()
        XCTAssertEqual(add(store, "svc", "a", "one"), errSecSuccess)
        XCTAssertEqual(add(store, "svc", "b", "two"), errSecSuccess)
        var result: CFTypeRef?
        XCTAssertEqual(store.copyMatching(query("svc", "a", returnData: true), &result), errSecSuccess)
        XCTAssertEqual((result as? Data).flatMap { String(data: $0, encoding: .utf8) }, "one")
        XCTAssertEqual(store.copyMatching(query("svc", "b"), nil), errSecSuccess, "an existence check needs no result")
        XCTAssertEqual(store.copyMatching(query("other", "a"), nil), errSecItemNotFound)
    }

    func testAddingADuplicateFailsLikeTheRealKeychain() {
        let store = InMemoryKeychain()
        XCTAssertEqual(add(store, "svc", "a", "one"), errSecSuccess)
        XCTAssertEqual(add(store, "svc", "a", "two"), errSecDuplicateItem)
    }

    func testUpdateReplacesTheValueAndReportsAMissingItem() {
        let store = InMemoryKeychain()
        XCTAssertEqual(store.update(query("svc", "a"), [kSecValueData as String: Data("x".utf8)] as CFDictionary), errSecItemNotFound)
        XCTAssertEqual(add(store, "svc", "a", "one"), errSecSuccess)
        XCTAssertEqual(store.update(query("svc", "a"), [kSecValueData as String: Data("two".utf8)] as CFDictionary), errSecSuccess)
        var result: CFTypeRef?
        XCTAssertEqual(store.copyMatching(query("svc", "a", returnData: true), &result), errSecSuccess)
        XCTAssertEqual((result as? Data).flatMap { String(data: $0, encoding: .utf8) }, "two")
    }

    func testDeleteRemovesTheItemAndIsHarmlessWhenMissing() {
        let store = InMemoryKeychain()
        XCTAssertEqual(add(store, "svc", "a", "one"), errSecSuccess)
        XCTAssertEqual(store.delete(query("svc", "a")), errSecSuccess)
        XCTAssertEqual(store.copyMatching(query("svc", "a"), nil), errSecItemNotFound)
        XCTAssertEqual(store.delete(query("svc", "a")), errSecItemNotFound)
    }

    func testTheTestHostNeverUsesTheRealKeychain() {
        XCTAssertTrue(KeychainAccess.isTestHost, "this suite runs inside the XCTest host")
        // A write through the funnel must land in memory: a unique account that is then readable and removable.
        let account = "keychain-access-tests-\(UUID().uuidString)"
        let attributes: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "com.mpointer.kestrel.tests",
            kSecAttrAccount as String: account,
            kSecValueData as String: Data("v".utf8),
        ]
        XCTAssertEqual(KeychainAccess.add(attributes as CFDictionary, nil), errSecSuccess)
        XCTAssertEqual(KeychainAccess.delete(query("com.mpointer.kestrel.tests", account)), errSecSuccess)
    }

    /// Guards the funnel: no source file outside `KeychainAccess.swift` may call the SecItem
    /// API directly, or a test run could reach the real Keychain again.
    func testNoSourceFileOutsideTheFunnelCallsSecItemDirectly() throws {
        let sources = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel")
        guard let files = FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil) else {
            return XCTFail("cannot enumerate \(sources.path)")
        }
        var offenders: [String] = []
        for case let url as URL in files where url.pathExtension == "swift" && url.lastPathComponent != "KeychainAccess.swift" {
            let text = try String(contentsOf: url, encoding: .utf8)
            let calls = ["SecItemAdd(", "SecItemCopyMatching(", "SecItemUpdate(", "SecItemDelete("]
            if calls.contains(where: { text.contains($0) }) { offenders.append(url.lastPathComponent) }
        }
        XCTAssertEqual(offenders, [], "call KeychainAccess instead of SecItem* in: \(offenders)")
    }
}
