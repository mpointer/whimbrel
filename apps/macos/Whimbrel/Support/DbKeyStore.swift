import Foundation
import Security

/// What reading the database key found (mirrors core's `api::store_key::KeyLookup`).
/// `unavailable` means the Keychain itself could not answer (locked, denied, failing):
/// never treated as `absent`, which would look like a fresh install.
enum DbKeyLookup: Equatable {
    case found(Data)
    case absent
    case unavailable(String)
}

enum DbKeyStoreError: Error, Equatable {
    /// `SecRandomCopyBytes` failed.
    case randomFailed(OSStatus)
    /// The Keychain refused the write (or the delete).
    case writeFailed(OSStatus)
    /// The Keychain could not be read, so nothing was written. The status is the Keychain's
    /// own, or `errSecDecode` when the read succeeded but held no 32-byte key.
    case readFailed(OSStatus, String)
    /// The account already holds a key; a key is never replaced.
    case alreadyExists(String)
    /// The account holds a different key than the one being stored.
    case mismatch(String)
    /// A key must be 32 bytes.
    case invalidKey
    /// `promotePending` found no pending key.
    case noPendingKey
    /// A different `db-key` exists and the caller cannot show it was rejected this launch:
    /// it is never replaced then.
    case notRejectedThisLaunch
    /// Every `db-key-previous*` slot is taken; nothing was replaced (never overwritten). The
    /// recovery screen then shows at every launch until a slot is freed (Delete everything
    /// frees them all; a Settings "remove old keys" is a follow-up).
    case noPreviousSlot
    /// The rejected key the caller names is not the `db-key` stored now: it changed since the
    /// database refused it, so nothing is replaced.
    case rejectedKeyChanged
    /// The pending key is not the key the store just opened with.
    case pendingNotOpened
    /// The Keychain reported the write as done, but the read-back did not show the key: it
    /// may or may not be stored.
    case storeUnverified(String)
    /// Delete everything: these accounts (sorted) could not be erased; the others were.
    case deleteFailed([String])
}

/// The Keychain calls `DbKeyStore` makes. Production goes through `KeychainAccess` (the
/// in-memory store in a test host); tests swap single calls to inject failures.
struct DbKeychain {
    var add: (CFDictionary) -> OSStatus
    var copyMatching: (CFDictionary, UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus
    var delete: (CFDictionary) -> OSStatus
    /// Used only by `replaceAfterRecovery`, the one in-place replacement of `db-key`.
    var update: (CFDictionary, CFDictionary) -> OSStatus

    static let live = DbKeychain(
        add: { KeychainAccess.add($0, nil) },
        copyMatching: { KeychainAccess.copyMatching($0, $1) },
        delete: { KeychainAccess.delete($0) },
        update: { KeychainAccess.update($0, $1) }
    )
}

/// The SQLCipher database key in the Keychain (X4 encryption at rest, design 3.2): 32 bytes
/// from the system random source, beside the sync keys (service `<bundle id>.sync`), account
/// `db-key`, or `db-key-pending` while an encryption migration has not swapped the file yet.
/// Items are never synchronizable (`kSecAttrSynchronizable = false`), so never in iCloud
/// Keychain. They also request `AfterFirstUnlockThisDeviceOnly`, set for forward
/// compatibility and for iOS-style (data-protection) keychains; on macOS the login keychain
/// ignores it, so the key follows a migrated or restored login keychain (Migration
/// Assistant, Time Machine). Core never touches the Keychain; it gets these
/// bytes from the shell.
///
/// Keys are added, never updated in place: a key that already exists is never replaced (the
/// database and the user's recovery key depend on it). Storing the same bytes again is a
/// no-op, so a rerun of the migration can write the same key twice.
///
/// Key bytes travel as `Data` and are not wiped from memory after use: Swift gives no
/// reliable way to zero a `Data`'s storage (copies are implicit), and SQLCipher keeps the
/// key in the process for as long as the store is open anyway. Core wipes its own copies.
struct DbKeyStore {
    static let keyAccount = "db-key"
    static let pendingAccount = "db-key-pending"
    /// Where a `db-key` that the database rejected is kept when a recovered key replaces it
    /// (`replaceAfterRecovery`). Then `db-key-previous-2`, `-3`, ... up to
    /// `maxPreviousSlots`. Never overwritten, never deleted by the launch (a key is never
    /// destroyed: it may belong to another copy of the data).
    static let previousAccount = "db-key-previous"
    static let maxPreviousSlots = 16

    static func previousAccount(slot: Int) -> String {
        slot <= 1 ? previousAccount : "\(previousAccount)-\(slot)"
    }
    static let defaultService = (Bundle.main.bundleIdentifier ?? "com.mpointer.kestrel") + ".sync"
    static let keyLength = 32

    let service: String
    var keychain: DbKeychain
    var random: (Int) throws -> Data

    init(service: String = DbKeyStore.defaultService, keychain: DbKeychain = .live,
         random: @escaping (Int) throws -> Data = DbKeyStore.systemRandom) {
        self.service = service
        self.keychain = keychain
        self.random = random
    }

    /// `count` bytes from the system CSPRNG.
    static func systemRandom(_ count: Int) throws -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        let status = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        guard status == errSecSuccess else { throw DbKeyStoreError.randomFailed(status) }
        return Data(bytes)
    }

    /// The key under `db-key`.
    func lookup() -> DbKeyLookup { read(Self.keyAccount) }

    /// The key under `db-key-pending` (an encryption migration in progress or interrupted).
    func lookupPending() -> DbKeyLookup { read(Self.pendingAccount) }

    /// A new random key stored as `db-key` (new install). Throws, and stores nothing, if
    /// `db-key` exists or the Keychain cannot be read or written.
    func create() throws -> Data { try createNew(account: Self.keyAccount) }

    /// A new random key stored as `db-key-pending` (before an encryption migration).
    func createPending() throws -> Data { try createNew(account: Self.pendingAccount) }

    /// Stores `key` under `account` (the migration's key writes, `db-key-pending` then
    /// `db-key`). The same bytes already there is success; different bytes throw `mismatch`.
    func store(_ key: Data, account: String) throws {
        guard key.count == Self.keyLength,
              account == Self.keyAccount || account == Self.pendingAccount else { throw DbKeyStoreError.invalidKey }
        switch readItem(account) {
        case let .found(existing):
            if existing == key { return }
            throw DbKeyStoreError.mismatch(account)
        case let .failed(status, message):
            throw DbKeyStoreError.readFailed(status, message)
        case .absent:
            try add(key, account: account)
        }
    }

    /// Copies the pending key to `db-key` (checked by reading it back), then deletes
    /// `db-key-pending`. Returns the key. If any step fails the pending key stays.
    @discardableResult
    func promotePending() throws -> Data {
        let key: Data
        switch readItem(Self.pendingAccount) {
        case let .found(k): key = k
        case .absent: throw DbKeyStoreError.noPendingKey
        case let .failed(status, message): throw DbKeyStoreError.readFailed(status, message)
        }
        try store(key, account: Self.keyAccount)
        // A write reported done that does not read back may or may not be stored.
        try verify(key, at: Self.keyAccount)
        try deletePending()
        return key
    }

    /// Stores `key`, which has just OPENED the encrypted database (the caller proves it), as
    /// `db-key`. A key is never destroyed:
    /// - no `db-key`: a plain add; the same bytes already there: nothing to do;
    /// - a different `db-key`: only when `rejected` holds exactly the bytes stored as `db-key`
    ///   now (the key the database refused during this launch; passed only by the launch
    ///   coordinator on that path). Nil: `notRejectedThisLaunch`; other bytes:
    ///   `rejectedKeyChanged`. The old key is first copied to the first free
    ///   `db-key-previous*` slot (skipped when a slot already holds it) and read back, then
    ///   `db-key` is replaced in place (`SecItemUpdate`) and read back.
    /// A failure or crash between the steps leaves either the old `db-key` intact (its copy
    /// possibly already kept) or the new key with the old one under `db-key-previous*`.
    func replaceAfterRecovery(proven key: Data, replacing rejected: Data?) throws {
        guard key.count == Self.keyLength else { throw DbKeyStoreError.invalidKey }
        let old: Data
        switch readItem(Self.keyAccount) {
        case .absent:
            // Nothing stored (also when the rejected key has gone meanwhile): a plain add of
            // the proven key replaces nothing, so nothing can be destroyed.
            _ = rejected
            try add(key, account: Self.keyAccount)
            try verify(key, at: Self.keyAccount)
            return
        case let .failed(status, message):
            throw DbKeyStoreError.readFailed(status, message)
        case let .found(existing):
            if existing == key { return }
            old = existing
        }
        // The proof is bytes: exactly the db-key stored now is the one the database rejected.
        guard let rejected else { throw DbKeyStoreError.notRejectedThisLaunch }
        guard rejected == old else { throw DbKeyStoreError.rejectedKeyChanged }
        try keepPrevious(old)
        let status = keychain.update(query(Self.keyAccount) as CFDictionary,
                                     [kSecValueData as String: key] as CFDictionary)
        guard status == errSecSuccess else { throw DbKeyStoreError.writeFailed(status) }
        try verify(key, at: Self.keyAccount)
    }

    /// The read-back after a write the Keychain reported as done: a mismatch or a failed read
    /// means the key may or may not be stored (`storeUnverified`), not "not stored".
    private func verify(_ key: Data, at account: String) throws {
        do { try expect(key, at: account) } catch {
            throw DbKeyStoreError.storeUnverified("\(account): \(error)")
        }
    }

    /// `PromotePendingKey`: the pending key opened the database. The stored pending key must
    /// be `opened` (the bytes the store opened with), else `pendingNotOpened`. No `db-key`, or
    /// the same bytes: `promotePending()`. A different `db-key`: `replaceAfterRecovery` with
    /// the pending key and the same `rejected` proof, then the pending copy is deleted (its
    /// bytes now live under `db-key`).
    func promotePendingKey(openedWith opened: Data, replacing rejected: Data?) throws {
        let pending: Data
        switch readItem(Self.pendingAccount) {
        case let .found(k): pending = k
        case .absent: throw DbKeyStoreError.noPendingKey
        case let .failed(status, message): throw DbKeyStoreError.readFailed(status, message)
        }
        // Only the very key the store just opened with is promoted.
        guard pending == opened else { throw DbKeyStoreError.pendingNotOpened }
        if case let .found(current) = readItem(Self.keyAccount), current != pending {
            try replaceAfterRecovery(proven: pending, replacing: rejected)
            try deletePending()
            return
        }
        try promotePending()
    }

    /// The keys kept under `db-key-previous*`, oldest first.
    func lookupPrevious() -> [Data] {
        (1...Self.maxPreviousSlots).compactMap { slot in
            if case let .found(k) = readItem(Self.previousAccount(slot: slot)) { return k }
            return nil
        }
    }

    /// Copies `old` to the first free `db-key-previous*` slot, unless a slot already holds
    /// it. Never overwrites a slot.
    private func keepPrevious(_ old: Data) throws {
        for slot in 1...Self.maxPreviousSlots {
            let account = Self.previousAccount(slot: slot)
            switch readItem(account) {
            case let .found(k) where k == old: return
            case .found: continue
            case let .failed(status, message): throw DbKeyStoreError.readFailed(status, message)
            case .absent:
                try add(old, account: account)
                try expect(old, at: account)
                return
            }
        }
        throw DbKeyStoreError.noPreviousSlot
    }

    private func expect(_ key: Data, at account: String) throws {
        switch readItem(account) {
        case let .found(back) where back == key: return
        case let .failed(status, message): throw DbKeyStoreError.readFailed(status, message)
        default: throw DbKeyStoreError.readFailed(errSecDecode, "\(account) did not read back")
        }
    }

    /// Deletes `db-key-pending` (after a migration finished). Missing is fine.
    func deletePending() throws { try remove(account: Self.pendingAccount) }

    /// Every account Delete everything erases: `db-key`, `db-key-pending`, then
    /// `db-key-previous` and `db-key-previous-2` to `-16`.
    static var allAccounts: [String] {
        [keyAccount, pendingAccount] + (1...maxPreviousSlots).map { previousAccount(slot: $0) }
    }

    /// An account Delete everything could not erase, with the Keychain's status.
    struct DeleteFailure: Equatable {
        let account: String
        let status: OSStatus
    }

    /// Delete everything (only after the database files are gone: `YourData.runPendingDelete`):
    /// erases every account in `allAccounts`, trying them all, and returns EVERY one that could
    /// not be erased (empty: all gone). A missing item is not a failure. (The launch itself
    /// never deletes a key.)
    func deleteAll() -> [DeleteFailure] {
        Self.allAccounts.compactMap { account in
            let status = keychain.delete(query(account) as CFDictionary)
            return status == errSecSuccess || status == errSecItemNotFound ? nil : DeleteFailure(account: account, status: status)
        }
    }

    /// `deleteAll()`, throwing `deleteFailed` with every account that could not be erased.
    func delete() throws {
        let failed = deleteAll()
        if !failed.isEmpty { throw DbKeyStoreError.deleteFailed(failed.map(\.account).sorted()) }
    }

    /// The key-store callback the core migration calls (`CoreEncryptionMigration.run`):
    /// 0 on success, else an OSStatus, which core puts in its `key_store` error message.
    /// A failed Keychain read or write returns the Keychain's own status. Where there is no
    /// Keychain status, a stand-in: `errSecDecode` for a stored value that is not a 32-byte
    /// key, `errSecDuplicateItem` for a different key already under the account (never
    /// replaced), `errSecParam` for a key that is not 32 bytes or an unknown account.
    func migrationWrite(_ key: Data, account: String) -> OSStatus {
        do {
            try store(key, account: account)
            return errSecSuccess
        } catch let DbKeyStoreError.writeFailed(status) {
            return status
        } catch let DbKeyStoreError.readFailed(status, _) {
            return status
        } catch DbKeyStoreError.mismatch, DbKeyStoreError.alreadyExists {
            return errSecDuplicateItem
        } catch {
            return errSecParam
        }
    }

    // MARK: - Keychain

    private func query(_ account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    /// One read, keeping the Keychain's status on failure.
    private enum ReadResult {
        case found(Data)
        case absent
        case failed(OSStatus, String)
    }

    private func read(_ account: String) -> DbKeyLookup {
        switch readItem(account) {
        case let .found(data): return .found(data)
        case .absent: return .absent
        case let .failed(_, message): return .unavailable(message)
        }
    }

    /// Only `errSecItemNotFound` is absent; every other status, and a success that holds no
    /// 32-byte key (`errSecDecode`), is a failure.
    private func readItem(_ account: String) -> ReadResult {
        var q = query(account)
        q[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = keychain.copyMatching(q as CFDictionary, &result)
        switch status {
        case errSecSuccess:
            guard let data = result as? Data else {
                return .failed(errSecDecode, "the Keychain returned no data for \(account)")
            }
            guard data.count == Self.keyLength else {
                return .failed(errSecDecode, "the \(account) item is \(data.count) bytes, not \(Self.keyLength)")
            }
            return .found(data)
        case errSecItemNotFound:
            return .absent
        default:
            return .failed(status, "Keychain read of \(account) failed (status \(status))")
        }
    }

    private func createNew(account: String) throws -> Data {
        switch readItem(account) {
        case .found: throw DbKeyStoreError.alreadyExists(account)
        case let .failed(status, message): throw DbKeyStoreError.readFailed(status, message)
        case .absent: break
        }
        let key = try random(Self.keyLength)
        guard key.count == Self.keyLength else { throw DbKeyStoreError.invalidKey }
        try add(key, account: account)
        return key
    }

    /// Adds a new item; never updates one in place.
    private func add(_ key: Data, account: String) throws {
        var attributes = query(account)
        attributes[kSecValueData as String] = key
        // Accessibility: set for forward compatibility and for iOS-style (data-protection)
        // keychains; the macOS login keychain ignores it, so the item follows a migrated or
        // restored login keychain. Synchronizable false: never synced to iCloud Keychain.
        attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        attributes[kSecAttrSynchronizable as String] = false
        let status = keychain.add(attributes as CFDictionary)
        switch status {
        case errSecSuccess:
            return
        case errSecDuplicateItem:
            throw DbKeyStoreError.alreadyExists(account)
        default:
            throw DbKeyStoreError.writeFailed(status)
        }
    }

    private func remove(account: String) throws {
        let status = keychain.delete(query(account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw DbKeyStoreError.writeFailed(status)
        }
    }
}
