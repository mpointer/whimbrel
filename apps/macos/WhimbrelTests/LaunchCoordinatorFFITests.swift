import XCTest
@testable import Whimbrel

/// The launch coordinator end to end through the real C ABI and real SQLCipher (X4, Task
/// 11b-1): the decision, resume/precheck/run, the keyed open with the plan's cache, the
/// launch-screen file actions and the speed check. Temp files and a throwaway Keychain
/// service in the test host's in-memory Keychain only; no MCP server is started.
final class LaunchCoordinatorFFITests: XCTestCase {
    private var tempDir: URL!
    private var path: URL { tempDir.appendingPathComponent("core-memory.sqlite") }

    override func setUp() {
        super.setUp()
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try! FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: tempDir)
        tempDir = nil
        super.tearDown()
    }

    final class Settings: LaunchSettingsStore {
        var fastSearch = true
        var migrateAtNextLaunch = false
        var goodLaunches: UInt32 = 0
    }

    /// Answers screens from `responses`, then quits, so a failure cannot hang.
    final class Presenter: LaunchPresenter {
        var screens: [LaunchScreen] = []
        var responses: [LaunchScreenResponse] = []
        var progress: [Double] = []
        let lock = NSLock()
        func present(_ screen: LaunchScreen) async -> LaunchScreenResponse {
            screens.append(screen)
            return responses.isEmpty ? .quit : responses.removeFirst()
        }
        func runOnboarding(existingKey: Data?) async -> Data? { nil }
        func migrationProgress(_ p: CoreEncryptionMigration.Progress) { lock.withLock { progress.append(p.overall) } }
    }

    private func meeting(_ title: String) -> CoreNewMeeting {
        CoreNewMeeting(deviceId: "dev", startUnixMs: 1000, endUnixMs: nil, title: title,
                       attendees: [], trigger: .manual, audioPath: nil, status: .completed)
    }

    private func header() throws -> Data { try FileHandle(forReadingFrom: path).read(upToCount: 16) ?? Data() }

    @MainActor
    private func launch(keys: DbKeyStore, settings: Settings, presenter: Presenter,
                        mcp: @escaping (Data?) -> Void) -> LaunchCoordinator<CoreMemoryStore> {
        var collaborators = LaunchCoordinator<CoreMemoryStore>.Collaborators.core(
            dbPath: path, modelsDir: nil, keys: keys, settings: settings,
            presenter: presenter, recording: { false }, startMcpServer: mcp)
        // A 32 GiB computer, whatever runs the test: Faster search applies.
        collaborators.physicalMemory = { 32 << 30 }
        return LaunchCoordinator(collaborators)
    }

    /// First launch after the Settings opt-in migrates and opens keyed; the second opens keyed.
    @MainActor
    func testAPlainDatabaseMigratesAtTheFirstLaunchAndOpensKeyedAtTheSecond() async throws {
        let id = try XCTUnwrap(CoreMemoryStore(path: path, modelsDir: nil)?.insertMeeting(meeting("Before encryption")))
        let keys = DbKeyStore(service: "launch-coordinator-ffi-tests.\(UUID().uuidString)")
        defer { try? keys.delete() }
        // The Settings opt-in: the setup key stored as db-key-pending and the flag set.
        let key = try keys.createPending()
        let settings = Settings()
        settings.migrateAtNextLaunch = true
        let presenter = Presenter()
        var mcpKeys: [Data?] = []

        do {
            let first = launch(keys: keys, settings: settings, presenter: presenter) { mcpKeys.append($0) }
            guard case let .opened(o) = await first.run() else { return XCTFail("screens: \(presenter.screens)") }
            XCTAssertEqual(o.key, key, "encrypted with the key the recovery setup made")
            XCTAssertEqual(o.store.getMeeting(id: id)?.title, "Before encryption")
            XCTAssertEqual(o.store.dbKey, key)
            XCTAssertEqual(keys.lookup(), .found(key))
            XCTAssertEqual(keys.lookupPending(), .absent, "deleted after the keyed open")
            XCTAssertFalse(settings.migrateAtNextLaunch)
            XCTAssertEqual(settings.goodLaunches, 0, "not counted before the first window")
            await first.firstWindowRendered()
            XCTAssertEqual(settings.goodLaunches, 1)
            XCTAssertEqual(mcpKeys, [key])
            XCTAssertEqual(presenter.progress.last, 1.0)
            XCTAssertTrue(CoreEncryptionMigration.plainBackupExists(dbPath: path))

            // The speed check runs on the store the launch opened (Faster search on: 1 GiB).
            let json = try XCTUnwrap(o.store.speedCheck(openMs: o.openMs, physicalMemory: 32 << 30, progress: nil))
            let v = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
            let report = try XCTUnwrap(v["report"] as? [String: Any])
            XCTAssertEqual(report["encrypted"] as? Bool, true)
            XCTAssertEqual(report["cache_kib"] as? Int, 1_048_576, "the plan's cache reached the main store")
            let reportJSON = String(data: try JSONSerialization.data(withJSONObject: report), encoding: .utf8)!
            let text = try XCTUnwrap(CoreSpeedCheck.text(reportJSON: reportJSON))
            XCTAssertTrue(text.hasPrefix("Whimbrel encryption speed check"))
            XCTAssertFalse(text.contains(tempDir.path) || json.contains(tempDir.path), "no paths")
        } // the first launch's store closes here
        XCTAssertNotEqual(try header(), Data("SQLite format 3\0".utf8), "encrypted on disk")

        let second = launch(keys: keys, settings: settings, presenter: presenter) { mcpKeys.append($0) }
        guard case let .opened(o) = await second.run() else { return XCTFail("screens: \(presenter.screens)") }
        XCTAssertEqual(o.key, key)
        XCTAssertEqual(o.store.getMeeting(id: id)?.title, "Before encryption")
        await second.firstWindowRendered()
        XCTAssertEqual(settings.goodLaunches, 2)
        XCTAssertEqual(second.notes, [])
        XCTAssertEqual(mcpKeys, [key, key])
        XCTAssertEqual(presenter.screens, [])
    }

    /// Encrypted with K; a wrong K' sits in db-key; the user restores K on the recovery screen.
    /// The database opens, K replaces K' only after it opened, K' is kept under
    /// db-key-previous, and the next launch opens with no screen.
    @MainActor
    func testRecoveryOverAWrongDbKeyOpensAndKeepsThePreviousKey() async throws {
        let k = Data((0..<32).map { UInt8(($0 * 13 + 5) % 256) })
        let wrong = Data(repeating: 0x5A, count: 32)
        var id: Int64 = 0
        do {
            let (created, store) = CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(k))
            XCTAssertEqual(created, .opened)
            id = try XCTUnwrap(store?.insertMeeting(meeting("Encrypted with K")))
        }
        let keys = DbKeyStore(service: "launch-coordinator-ffi-tests.\(UUID().uuidString)")
        defer { try? keys.delete() } // Delete everything: db-key, pending and every previous slot
        try keys.store(wrong, account: DbKeyStore.keyAccount)
        let settings = Settings()
        let presenter = Presenter()
        presenter.responses = [.restoredKey(k)]
        var mcpKeys: [Data?] = []

        do {
            let first = launch(keys: keys, settings: settings, presenter: presenter) { mcpKeys.append($0) }
            guard case let .opened(o) = await first.run() else { return XCTFail("screens: \(presenter.screens)") }
            XCTAssertEqual(presenter.screens, [.recovery(.keyRejected)])
            XCTAssertEqual(o.key, k)
            XCTAssertEqual(o.store.getMeeting(id: id)?.title, "Encrypted with K")
            XCTAssertEqual(keys.lookup(), .found(k))
            XCTAssertEqual(keys.lookupPrevious(), [wrong], "the rejected key is kept, never destroyed")
            XCTAssertEqual(first.notes, [])
            XCTAssertEqual(mcpKeys, [k])
        }

        presenter.screens = []
        let second = launch(keys: keys, settings: settings, presenter: presenter) { mcpKeys.append($0) }
        guard case let .opened(o) = await second.run() else { return XCTFail("screens: \(presenter.screens)") }
        XCTAssertEqual(presenter.screens, [], "no screen at the next launch")
        XCTAssertEqual(o.key, k)
        XCTAssertEqual(keys.lookupPrevious(), [wrong])
        // Delete everything leaves no key behind, previous ones included.
        try keys.delete()
        XCTAssertEqual(keys.lookup(), .absent)
        XCTAssertEqual(keys.lookupPrevious(), [])
    }

    /// Task 13c item 4 through the real C ABI: the first open writes the marker and notices
    /// nothing; with the database gone (and no copy) the next launch opens a new empty store
    /// with ONE "library not found" note; the launch after that is quiet.
    @MainActor
    func testALibraryThatVanishedIsNoticedOnceAndANewInstallNever() async throws {
        let keys = DbKeyStore(service: "launch-coordinator-ffi-tests.\(UUID().uuidString)")
        defer { try? keys.delete() }
        try keys.storeKey(Data(repeating: 0x44, count: 32))
        let marker = tempDir.appendingPathComponent("store_existed")
        let presenter = Presenter()

        let first = launch(keys: keys, settings: Settings(), presenter: presenter) { _ in }
        guard case .opened = await first.run() else { return XCTFail("screens: \(presenter.screens)") }
        XCTAssertEqual(first.notes, [], "a new install notices nothing")
        XCTAssertTrue(FileManager.default.fileExists(atPath: marker.path), "written after the open")

        for name in try FileManager.default.contentsOfDirectory(atPath: tempDir.path) where name.hasPrefix("core-memory.sqlite") {
            try FileManager.default.removeItem(at: tempDir.appendingPathComponent(name))
        }
        let second = launch(keys: keys, settings: Settings(), presenter: presenter) { _ in }
        guard case .opened = await second.run() else { return XCTFail("screens: \(presenter.screens)") }
        XCTAssertEqual(second.notes, [.libraryMissing])
        XCTAssertTrue(FileManager.default.fileExists(atPath: marker.path), "written again after the new store opened")

        let third = launch(keys: keys, settings: Settings(), presenter: presenter) { _ in }
        guard case .opened = await third.run() else { return XCTFail("screens: \(presenter.screens)") }
        XCTAssertEqual(third.notes, [], "shown once")
        XCTAssertEqual(presenter.screens, [])
    }

    /// The proof open never creates: a missing database stays missing (real C ABI).
    func testTheNeverCreateOpenLeavesAMissingDatabaseMissing() throws {
        let key = Data(repeating: 0x33, count: 32)
        let (outcome, store) = CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(key), mustExist: true)
        guard case .damaged = outcome else { return XCTFail("\(outcome)") }
        XCTAssertNil(store)
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path))
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: tempDir.path), [])
        do { XCTAssertEqual(CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(key)).0, .opened) }
        XCTAssertEqual(CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(key), mustExist: true).0, .opened)
    }

    /// The launch-screen file actions through the Swift wrappers: codes and counts.
    func testTheFileActionsOverTheCAbi() throws {
        let backup = URL(fileURLWithPath: path.path + ".plain.bak")
        do { _ = CoreMemoryStore(path: backup, modelsDir: nil) }
        for s in ["-wal", "-shm"] { try? FileManager.default.removeItem(atPath: backup.path + s) }
        try Data("stray".utf8).write(to: URL(fileURLWithPath: path.path + "-wal"))
        let restored = CoreEncryptionMigration.restoreRollbackCopy(dbPath: path)
        XCTAssertEqual(restored, CoreRestoreResult(action: .done, movedAside: 1, warning: .none))
        XCTAssertEqual(try header(), Data("SQLite format 3\0".utf8))
        XCTAssertEqual(CoreEncryptionMigration.restoreRollbackCopy(dbPath: path).action, .refused, "the database exists now")
        XCTAssertEqual(CoreEncryptionMigration.resetEncryptionStatus(dbPath: path), .refused, "no state file")
        let (deleted, removed) = CoreEncryptionMigration.deletePlainBackup(dbPath: path, openedWithKeyThisLaunch: true)
        XCTAssertEqual(deleted, .refused, "a plain database")
        XCTAssertEqual(removed, 0)
        XCTAssertEqual(CoreEncryptionFileAction(code: 42), .unknown(42))
        XCTAssertEqual(CoreRestoreWarning(code: 2), .stateNotCleared)
    }
}
