import XCTest
@testable import Whimbrel

final class StorePathTests: XCTestCase {
    /// The decrypted store stays out of iCloud/Finder backups and is readable after first
    /// unlock (review fix: plaintext store in backups).
    func testStoreDirectoryIsExcludedFromBackupWithFileProtection() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("store-\(UUID().uuidString)/Sync", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir.deletingLastPathComponent()) }
        try CoreMemoryStore.prepareStoreDirectory(dir)
        var isDirectory: ObjCBool = false
        XCTAssertTrue(FileManager.default.fileExists(atPath: dir.path, isDirectory: &isDirectory) && isDirectory.boolValue)
        XCTAssertEqual(try dir.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
        XCTAssertEqual(CoreMemoryStore.storeDirectoryAttributes[.protectionKey] as? FileProtectionType, .completeUntilFirstUserAuthentication)
        // The simulator may not report data protection; assert it where it does.
        if let protection = try FileManager.default.attributesOfItem(atPath: dir.path)[.protectionKey] as? FileProtectionType {
            XCTAssertEqual(protection, .completeUntilFirstUserAuthentication)
        }
        // Idempotent: a second launch re-applies the flags without failing.
        try CoreMemoryStore.prepareStoreDirectory(dir)
        XCTAssertEqual(try dir.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
    }

    func testDefaultStoreLivesInTheSyncDirectory() throws {
        let path = try CoreMemoryStore.defaultPath()
        XCTAssertEqual(path.lastPathComponent, "memory.sqlite")
        XCTAssertEqual(path.deletingLastPathComponent().lastPathComponent, "Sync")
        XCTAssertEqual(try path.deletingLastPathComponent().resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
    }
}
