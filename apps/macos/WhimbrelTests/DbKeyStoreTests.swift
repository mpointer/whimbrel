import Security
import XCTest
@testable import Whimbrel

/// The database key in the Keychain (X4 encryption at rest, design 3.2). In the test host
/// every call lands in `KeychainAccess`'s in-memory store; each test uses its own service so
/// tests never share items. Failures are injected through `DbKeychain`.
final class DbKeyStoreTests: XCTestCase {
    private func store(keychain: DbKeychain = .live) -> DbKeyStore {
        DbKeyStore(service: "db-key-store-tests.\(UUID().uuidString)", keychain: keychain)
    }

    func testCreateThenLookupReturnsSameKey() throws {
        XCTAssertTrue(KeychainAccess.isTestHost, "must never reach the real Keychain")
        let keys = store()
        XCTAssertEqual(keys.lookup(), .absent)
        let created = try keys.create()
        XCTAssertEqual(keys.lookup(), .found(created))
        XCTAssertEqual(keys.lookupPending(), .absent, "create() writes db-key only")
    }

    /// A Keychain that answers every read with `status`, handing back `data` on success.
    private func reading(_ status: OSStatus, data: Data? = nil) -> DbKeychain {
        var keychain = DbKeychain.live
        keychain.copyMatching = { _, result in
            if let data { result?.pointee = data as NSData }
            return status
        }
        return keychain
    }

    func testLookupReportsUnavailableOnKeychainError() {
        let errors: [(String, OSStatus)] = [
            ("errSecInteractionNotAllowed", errSecInteractionNotAllowed),
            ("errSecAuthFailed", errSecAuthFailed),
            ("errSecNoSuchKeychain", errSecNoSuchKeychain),
            ("errSecMissingEntitlement", errSecMissingEntitlement),
            ("errSecUserCanceled", errSecUserCanceled),
            ("errSecNotAvailable", errSecNotAvailable),
        ]
        XCTAssertEqual(errors.map(\.1), [-25308, -25293, -25294, -34018, -128, -25291])
        for (name, status) in errors {
            let keys = store(keychain: reading(status))
            for (account, result) in [("db-key", keys.lookup()), ("db-key-pending", keys.lookupPending())] {
                guard case let .unavailable(message) = result else {
                    XCTFail("\(name) on \(account) must be unavailable, never absent; got \(result)")
                    continue
                }
                XCTAssertTrue(message.contains("\(status)"), "\(name): \(message)")
            }
        }
        // Success that is not a 32-byte key: no data, too short, too long.
        for (label, keychain) in [("nil data", reading(errSecSuccess)),
                                  ("16 bytes", reading(errSecSuccess, data: Data(repeating: 1, count: 16))),
                                  ("33 bytes", reading(errSecSuccess, data: Data(repeating: 1, count: 33)))] {
            let keys = store(keychain: keychain)
            for result in [keys.lookup(), keys.lookupPending()] {
                guard case .unavailable = result else { XCTFail("\(label) must be unavailable; got \(result)"); continue }
            }
        }
        // Only item-not-found is absent.
        let missing = store(keychain: reading(errSecItemNotFound))
        XCTAssertEqual(missing.lookup(), .absent)
        XCTAssertEqual(missing.lookupPending(), .absent)
    }

    /// The C key-store callback's answer: 0, or the real Keychain status.
    func testMigrationWriteReturnsTheRealKeychainStatus() throws {
        let key = Data(repeating: 3, count: 32)
        let keys = store()
        XCTAssertEqual(keys.migrationWrite(key, account: "db-key-pending"), errSecSuccess)
        XCTAssertEqual(keys.lookupPending(), .found(key))
        XCTAssertEqual(keys.migrationWrite(key, account: "db-key-pending"), errSecSuccess, "same bytes again is fine")

        var refusingAdd = DbKeychain.live
        refusingAdd.add = { _ in errSecNotAvailable }
        XCTAssertEqual(store(keychain: refusingAdd).migrationWrite(key, account: "db-key"), errSecNotAvailable)

        for status in [errSecAuthFailed, errSecInteractionNotAllowed, errSecMissingEntitlement] {
            XCTAssertEqual(store(keychain: reading(status)).migrationWrite(key, account: "db-key"), status,
                           "a failed read is reported with its own status, not a stand-in")
        }
        // Documented stand-ins where there is no Keychain status to pass on.
        XCTAssertEqual(store(keychain: reading(errSecSuccess, data: Data(count: 5))).migrationWrite(key, account: "db-key"),
                       errSecDecode, "a stored value that is not a key")
        XCTAssertEqual(keys.migrationWrite(Data(repeating: 4, count: 32), account: "db-key-pending"), errSecDuplicateItem,
                       "a different key already there: never replaced")
        XCTAssertEqual(keys.lookupPending(), .found(key))
        XCTAssertEqual(keys.migrationWrite(Data(count: 31), account: "db-key"), errSecParam)
        XCTAssertEqual(keys.migrationWrite(key, account: "other"), errSecParam)
    }

    func testStoredValueOfTheWrongLengthIsUnavailableNotAbsent() throws {
        let keys = store()
        try keys.store(Data(repeating: 1, count: 32), account: DbKeyStore.keyAccount)
        var keychain = DbKeychain.live
        keychain.copyMatching = { query, result in
            result?.pointee = Data(repeating: 1, count: 16) as NSData
            return errSecSuccess
        }
        guard case .unavailable = DbKeyStore(service: keys.service, keychain: keychain).lookup() else {
            return XCTFail("a 16-byte value is not a key")
        }
    }

    func testCreateFailureIsSurfacedNotSwallowed() {
        var keychain = DbKeychain.live
        keychain.add = { _ in errSecNotAvailable }
        let keys = store(keychain: keychain)
        XCTAssertThrowsError(try keys.create()) { error in
            XCTAssertEqual(error as? DbKeyStoreError, .writeFailed(errSecNotAvailable))
        }
        XCTAssertThrowsError(try keys.createPending())
        XCTAssertEqual(keys.lookup(), .absent, "nothing half-written")

        let noRandom = DbKeyStore(service: keys.service, keychain: .live, random: { _ in throw DbKeyStoreError.randomFailed(-1) })
        XCTAssertThrowsError(try noRandom.create()) { error in
            XCTAssertEqual(error as? DbKeyStoreError, .randomFailed(-1))
        }
        XCTAssertEqual(noRandom.lookup(), .absent)
    }

    func testCreateNeverOverwritesAnExistingKey() throws {
        let keys = store()
        let first = try keys.create()
        XCTAssertThrowsError(try keys.create()) { error in
            XCTAssertEqual(error as? DbKeyStoreError, .alreadyExists(DbKeyStore.keyAccount))
        }
        XCTAssertEqual(keys.lookup(), .found(first), "the database's key is never replaced")
        let pending = try keys.createPending()
        XCTAssertThrowsError(try keys.createPending())
        XCTAssertEqual(keys.lookupPending(), .found(pending))
    }

    func testStoreIsIdempotentForTheSameKeyAndRefusesADifferentOne() throws {
        let keys = store()
        let key = Data(repeating: 7, count: 32)
        try keys.store(key, account: DbKeyStore.pendingAccount)
        try keys.store(key, account: DbKeyStore.pendingAccount)
        XCTAssertThrowsError(try keys.store(Data(repeating: 8, count: 32), account: DbKeyStore.pendingAccount))
        XCTAssertThrowsError(try keys.store(Data(repeating: 7, count: 31), account: DbKeyStore.keyAccount))
        XCTAssertEqual(keys.lookupPending(), .found(key))
    }

    func testPendingThenPromoteMovesTheKey() throws {
        let keys = store()
        let pending = try keys.createPending()
        XCTAssertEqual(keys.lookup(), .absent)
        XCTAssertEqual(try keys.promotePending(), pending)
        XCTAssertEqual(keys.lookup(), .found(pending))
        XCTAssertEqual(keys.lookupPending(), .absent)
        XCTAssertThrowsError(try keys.promotePending()) { error in
            XCTAssertEqual(error as? DbKeyStoreError, .noPendingKey)
        }
    }

    func testPromoteKeepsPendingWhenTheWriteFails() throws {
        let keys = store()
        let pending = try keys.createPending()
        var keychain = DbKeychain.live
        keychain.add = { _ in errSecNotAvailable }
        XCTAssertThrowsError(try DbKeyStore(service: keys.service, keychain: keychain).promotePending())
        XCTAssertEqual(keys.lookupPending(), .found(pending), "pending is deleted only after db-key reads back")
        XCTAssertEqual(keys.lookup(), .absent)
    }

    // MARK: Recovery over a rejected key (a key is never destroyed)

    private let old = Data(repeating: 0x0A, count: 32)
    private let restored = Data(repeating: 0x0B, count: 32)

    private let other = Data(repeating: 0x0D, count: 32)

    /// The proof is bytes: the caller names the db-key the database rejected, and it must be
    /// the db-key stored now.
    func testReplaceAfterRecoveryIsRefusedWithoutProofThatTheCurrentDbKeyWasRejected() throws {
        let keys = store()
        try keys.store(old, account: DbKeyStore.keyAccount)
        XCTAssertThrowsError(try keys.replaceAfterRecovery(proven: restored, replacing: nil)) { error in
            XCTAssertEqual(error as? DbKeyStoreError, .notRejectedThisLaunch)
        }
        XCTAssertThrowsError(try keys.replaceAfterRecovery(proven: restored, replacing: other)) { error in
            XCTAssertEqual(error as? DbKeyStoreError, .rejectedKeyChanged, "the db-key changed since it was rejected")
        }
        XCTAssertEqual(keys.lookup(), .found(old))
        XCTAssertEqual(keys.lookupPrevious(), [], "nothing written")
    }

    func testReplaceAfterRecoveryKeepsTheOldKeyUnderDbKeyPrevious() throws {
        let keys = store()
        try keys.store(old, account: DbKeyStore.keyAccount)
        try keys.replaceAfterRecovery(proven: restored, replacing: old)
        XCTAssertEqual(keys.lookup(), .found(restored))
        XCTAssertEqual(keys.lookupPrevious(), [old])
        // The same key again is a no-op; a later recovery over another rejected key keeps both.
        try keys.replaceAfterRecovery(proven: restored, replacing: old)
        XCTAssertEqual(keys.lookupPrevious(), [old])
        let third = Data(repeating: 0x0C, count: 32)
        try keys.replaceAfterRecovery(proven: third, replacing: restored)
        XCTAssertEqual(keys.lookup(), .found(third))
        XCTAssertEqual(keys.lookupPrevious(), [old, restored], "db-key-previous is never overwritten")
        XCTAssertEqual(DbKeyStore.previousAccount, "db-key-previous")
    }

    func testReplaceAfterRecoveryWithNoDbKeyIsAPlainStore() throws {
        let keys = store()
        try keys.replaceAfterRecovery(proven: restored, replacing: nil)
        XCTAssertEqual(keys.lookup(), .found(restored))
        XCTAssertEqual(keys.lookupPrevious(), [])
        // The rejected key has gone meanwhile and nothing is stored: storing the proven key
        // replaces nothing, so it is a plain store.
        let keys2 = store()
        try keys2.replaceAfterRecovery(proven: restored, replacing: old)
        XCTAssertEqual(keys2.lookup(), .found(restored))
        XCTAssertEqual(keys2.lookupPrevious(), [])
    }

    /// The add path (no db-key): the Keychain says done but nothing reads back.
    func testAnUnverifiedAddIsReportedAsUnverified() throws {
        let keys = store()
        var silent = DbKeychain.live
        silent.add = { _ in errSecSuccess } // claims success, stores nothing
        XCTAssertThrowsError(try DbKeyStore(service: keys.service, keychain: silent)
            .replaceAfterRecovery(proven: restored, replacing: nil)) { error in
            guard case .storeUnverified? = error as? DbKeyStoreError else { return XCTFail("\(error)") }
            XCTAssertEqual(LaunchKeyFailure(error), .unverified)
        }
    }

    /// The plain promotion's read-back is unverified the same way, and the pending key stays.
    func testAnUnverifiedPromotionIsReportedAsUnverifiedAndKeepsThePendingKey() throws {
        let keys = store()
        try keys.store(restored, account: DbKeyStore.pendingAccount)
        var silent = DbKeychain.live
        silent.add = { _ in errSecSuccess }
        XCTAssertThrowsError(try DbKeyStore(service: keys.service, keychain: silent).promotePending()) { error in
            guard case .storeUnverified? = error as? DbKeyStoreError else { return XCTFail("\(error)") }
        }
        XCTAssertEqual(keys.lookupPending(), .found(restored))
    }

    func testAChangedRejectedKeyHasItsOwnTypedReason() {
        XCTAssertEqual(LaunchKeyFailure(DbKeyStoreError.rejectedKeyChanged), .keyChangedMeanwhile)
        XCTAssertNotEqual(LaunchKeyFailure(DbKeyStoreError.rejectedKeyChanged), .differentKeyStored)
    }

    /// A crash or failure between the steps leaves either the old key intact (with its copy
    /// under db-key-previous) or the restored key plus the previous one: never neither.
    func testAFailureBetweenTheStepsNeverLosesAKey() throws {
        let keys = store()
        try keys.store(old, account: DbKeyStore.keyAccount)
        var keychain = DbKeychain.live
        keychain.update = { _, _ in errSecNotAvailable }
        XCTAssertThrowsError(try DbKeyStore(service: keys.service, keychain: keychain)
            .replaceAfterRecovery(proven: restored, replacing: old))
        XCTAssertEqual(keys.lookup(), .found(old), "db-key untouched")
        XCTAssertEqual(keys.lookupPrevious(), [old], "the copy was made first")
        // Pressing it again finishes it (the copy is already kept).
        try keys.replaceAfterRecovery(proven: restored, replacing: old)
        XCTAssertEqual(keys.lookup(), .found(restored))
        XCTAssertEqual(keys.lookupPrevious(), [old])

        // The copy itself failing: nothing replaced.
        let keys2 = store()
        try keys2.store(old, account: DbKeyStore.keyAccount)
        var failingAdd = DbKeychain.live
        failingAdd.add = { _ in errSecNotAvailable }
        XCTAssertThrowsError(try DbKeyStore(service: keys2.service, keychain: failingAdd)
            .replaceAfterRecovery(proven: restored, replacing: old))
        XCTAssertEqual(keys2.lookup(), .found(old))
        XCTAssertEqual(keys2.lookupPrevious(), [])
    }

    /// The update reported success but the read-back does not show the new key: the key may
    /// or may not be stored, which is not the same as "not stored".
    func testAnUnverifiedUpdateIsReportedAsUnverified() throws {
        let keys = store()
        try keys.store(old, account: DbKeyStore.keyAccount)
        var lying = DbKeychain.live
        lying.update = { _, _ in errSecSuccess } // claims success, changes nothing
        XCTAssertThrowsError(try DbKeyStore(service: keys.service, keychain: lying)
            .replaceAfterRecovery(proven: restored, replacing: old)) { error in
            guard case .storeUnverified? = error as? DbKeyStoreError else { return XCTFail("\(error)") }
        }
        XCTAssertEqual(keys.lookupPrevious(), [old], "the old key was kept first")
    }

    /// Every previous slot taken: refused before anything changes, with its own reason.
    func testAtTheSlotCeilingNothingIsReplaced() throws {
        let keys = store()
        for slot in 1...DbKeyStore.maxPreviousSlots {
            XCTAssertEqual(KeychainAccess.add([kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: keys.service,
                                               kSecAttrAccount as String: DbKeyStore.previousAccount(slot: slot),
                                               kSecValueData as String: Data(repeating: UInt8(0x40 + slot), count: 32)] as CFDictionary,
                                              nil), errSecSuccess)
        }
        try keys.store(old, account: DbKeyStore.keyAccount)
        XCTAssertThrowsError(try keys.replaceAfterRecovery(proven: restored, replacing: old)) { error in
            XCTAssertEqual(error as? DbKeyStoreError, .noPreviousSlot)
            XCTAssertEqual(LaunchKeyFailure(error), .noPreviousSlot)
        }
        XCTAssertEqual(keys.lookup(), .found(old))
        XCTAssertEqual(keys.lookupPrevious().count, DbKeyStore.maxPreviousSlots)
        try keys.delete() // Delete everything frees the slots
        XCTAssertEqual(keys.lookupPrevious(), [])
    }

    func testPromotePendingOverARejectedDbKeyKeepsTheOldKey() throws {
        let keys = store()
        try keys.store(old, account: DbKeyStore.keyAccount)
        try keys.store(restored, account: DbKeyStore.pendingAccount)
        XCTAssertThrowsError(try keys.promotePendingKey(openedWith: restored, replacing: nil), "never replaces unproven")
        XCTAssertThrowsError(try keys.promotePendingKey(openedWith: restored, replacing: other), "not the current db-key")
        XCTAssertThrowsError(try keys.promotePendingKey(openedWith: other, replacing: old)) { error in
            XCTAssertEqual(error as? DbKeyStoreError, .pendingNotOpened, "the store did not open with the pending key")
        }
        XCTAssertEqual(keys.lookup(), .found(old))
        XCTAssertEqual(keys.lookupPending(), .found(restored))
        try keys.promotePendingKey(openedWith: restored, replacing: old)
        XCTAssertEqual(keys.lookup(), .found(restored))
        XCTAssertEqual(keys.lookupPending(), .absent, "the same bytes now live under db-key")
        XCTAssertEqual(keys.lookupPrevious(), [old])
        // The usual promotion (no db-key) is unchanged.
        let keys2 = store()
        try keys2.store(restored, account: DbKeyStore.pendingAccount)
        try keys2.promotePendingKey(openedWith: restored, replacing: nil)
        XCTAssertEqual(keys2.lookup(), .found(restored))
        XCTAssertEqual(keys2.lookupPending(), .absent)
    }

    /// Delete everything erases db-key, db-key-pending and every db-key-previous* slot.
    func testDeleteRemovesTheKeyThePendingKeyAndEveryPreviousKey() throws {
        let keys = store()
        _ = try keys.create()
        _ = try keys.createPending()
        for slot in [1, 2, DbKeyStore.maxPreviousSlots] {
            XCTAssertEqual(KeychainAccess.add([kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: keys.service,
                                               kSecAttrAccount as String: DbKeyStore.previousAccount(slot: slot),
                                               kSecValueData as String: Data(repeating: UInt8(slot), count: 32)] as CFDictionary,
                                              nil), errSecSuccess)
        }
        XCTAssertEqual(keys.lookupPrevious().count, 3)
        try keys.delete()
        XCTAssertEqual(keys.lookup(), .absent)
        XCTAssertEqual(keys.lookupPending(), .absent)
        XCTAssertEqual(keys.lookupPrevious(), [], "every previous slot")
        try keys.delete() // harmless when there is nothing
        _ = try keys.createPending()
        try keys.deletePending()
        XCTAssertEqual(keys.lookupPending(), .absent)
    }

    /// Delete everything reports EVERY account that could not be deleted (not just the first),
    /// and still deletes the others.
    func testDeleteAllReportsEverySlotThatWouldNotGo() throws {
        let refused: Set<String> = [DbKeyStore.pendingAccount, DbKeyStore.previousAccount(slot: 2)]
        var keychain = DbKeychain.live
        keychain.delete = { query in
            let account = (query as NSDictionary)[kSecAttrAccount as String] as? String ?? ""
            return refused.contains(account) ? errSecInteractionNotAllowed : KeychainAccess.delete(query)
        }
        let keys = store(keychain: keychain)
        _ = try keys.create()
        _ = try keys.createPending()
        let failed = keys.deleteAll()
        XCTAssertEqual(Set(failed.map(\.account)), refused)
        XCTAssertTrue(failed.allSatisfy { $0.status == errSecInteractionNotAllowed })
        XCTAssertEqual(keys.lookup(), .absent, "the others still go")
        XCTAssertThrowsError(try keys.delete()) { error in
            XCTAssertEqual(error as? DbKeyStoreError, .deleteFailed(refused.sorted()))
        }
    }

    func testKeyIsThirtyTwoBytesAndRandom() throws {
        let a = try store().create()
        let b = try store().create()
        XCTAssertEqual(a.count, 32)
        XCTAssertEqual(b.count, 32)
        XCTAssertNotEqual(a, b)
        XCTAssertNotEqual(a, Data(count: 32))
    }

    /// Proves what is sent to the Keychain, not what macOS enforces: the login keychain
    /// ignores the this-device-only accessibility (it follows a migrated login keychain);
    /// non-synchronizable keeps the key out of iCloud Keychain.
    func testItemsAreNeverSynchronizableAndRequestThisDeviceOnlyAccessibility() {
        var added: [String: Any] = [:]
        var keychain = DbKeychain.live
        keychain.add = { attributes in
            added = attributes as! [String: Any]
            return errSecSuccess
        }
        _ = try? store(keychain: keychain).create()
        XCTAssertEqual(added[kSecAttrAccessible as String] as? String, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
        XCTAssertEqual(added[kSecAttrSynchronizable as String] as? Bool, false)
        XCTAssertEqual(added[kSecAttrAccount as String] as? String, "db-key")
        XCTAssertEqual(added[kSecClass as String] as? String, kSecClassGenericPassword as String)
    }

    func testDefaultServiceIsTheSyncKeysService() {
        XCTAssertEqual(DbKeyStore().service, (Bundle.main.bundleIdentifier ?? "com.mpointer.kestrel") + ".sync")
        XCTAssertEqual(DbKeyStore.keyAccount, "db-key")
        XCTAssertEqual(DbKeyStore.pendingAccount, "db-key-pending")
    }
}
