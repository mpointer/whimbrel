import Security
import XCTest
@testable import Whimbrel

/// The app links SQLCipher (core/ffi built with `--features sqlcipher`) and opens a keyed
/// store end to end through the C ABI (X4 encryption at rest). Temp files only.
final class KeyedStoreFFITests: XCTestCase {
    private var tempDir: URL!
    private var path: URL { tempDir.appendingPathComponent("core-memory.sqlite") }
    private let key = Data((0..<32).map { UInt8(($0 * 37 + 11) % 256) })

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

    private func meeting(_ title: String) -> CoreNewMeeting {
        CoreNewMeeting(deviceId: "dev", startUnixMs: 1000, endUnixMs: nil, title: title,
                       attendees: [], trigger: .manual, audioPath: nil, status: .completed)
    }

    private func header(_ url: URL) throws -> Data {
        try FileHandle(forReadingFrom: url).read(upToCount: 16) ?? Data()
    }

    func testKeyedStoreOpensWritesAndReopens() throws {
        var id: Int64 = 0
        do {
            let (created, store) = CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(key))
            if case let .unsupported(message) = created { return XCTFail("the app must link SQLCipher: \(message)") }
            XCTAssertEqual(created, .opened)
            let primary = try XCTUnwrap(store)
            id = try XCTUnwrap(primary.insertMeeting(meeting("Encrypted standup")))
            let slow = CoreMemoryStore(secondaryTo: primary, path: path)
            XCTAssertNotNil(slow, "the slow lane opens with the primary's key")
            XCTAssertEqual(slow?.getMeeting(id: id)?.title, "Encrypted standup")
        } // both connections close here
        XCTAssertNotEqual(try header(path), Data("SQLite format 3\0".utf8), "the file on disk is encrypted")
        XCTAssertNil(CoreMemoryStore(path: path, modelsDir: nil), "the plain open cannot read it")

        let (again, reopened) = CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(key))
        XCTAssertEqual(again, .opened)
        XCTAssertEqual(reopened?.getMeeting(id: id)?.title, "Encrypted standup")
    }

    func testWrongKeyIsKeyRejectedAndLeavesTheFileUnchanged() throws {
        do {
            let (created, store) = CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(key))
            XCTAssertEqual(created, .opened)
            _ = store?.insertMeeting(meeting("x"))
        }
        let before = try Data(contentsOf: path)
        let (outcome, store) = CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(Data(repeating: 9, count: 32)))
        guard case .keyRejected = outcome else { return XCTFail("got \(outcome)") }
        XCTAssertEqual(outcome.code, WHIMBREL_OPEN_KEY_REJECTED)
        XCTAssertNil(store)
        XCTAssertEqual(try Data(contentsOf: path), before)
        let (needsKey, none) = CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .absent)
        XCTAssertEqual(needsKey, .needsKey)
        XCTAssertNil(none)
        XCTAssertEqual(try Data(contentsOf: path), before)
    }

    func testEveryOutcomeCodeMapsBothWays() {
        let all: [(Int32, CoreOpenOutcome)] = [
            (WHIMBREL_OPEN_OPENED, .opened),
            (WHIMBREL_OPEN_NEW_INSTALL, .newInstall),
            (WHIMBREL_OPEN_NEEDS_KEY, .needsKey),
            (WHIMBREL_OPEN_KEY_UNAVAILABLE, .keyUnavailable("m")),
            (WHIMBREL_OPEN_KEY_REJECTED, .keyRejected("m")),
            (WHIMBREL_OPEN_NEEDS_MIGRATION, .needsMigration),
            (WHIMBREL_OPEN_DAMAGED, .damaged("m")),
            (WHIMBREL_OPEN_STARTUP_FAILED, .startupFailed("m")),
            (WHIMBREL_OPEN_UNSUPPORTED, .unsupported("m")),
            (WHIMBREL_OPEN_MIGRATION_INTERRUPTED, .migrationInterrupted("m")),
            (WHIMBREL_OPEN_INVALID_ARGUMENT, .invalidArgument),
        ]
        XCTAssertEqual(all.map(\.0), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, -1], "the pinned C codes")
        for (code, outcome) in all {
            XCTAssertEqual(CoreOpenOutcome(code: code, message: "m"), outcome, "code \(code)")
            XCTAssertEqual(outcome.code, code, "\(outcome)")
        }
        // A code this build does not know is never mistaken for a known outcome.
        let unknown = CoreOpenOutcome(code: 42, message: "from a newer core")
        XCTAssertEqual(unknown, .unknown(42, "from a newer core"))
        XCTAssertNotEqual(unknown, .invalidArgument)
        XCTAssertEqual(unknown.code, 42)
        XCTAssertEqual(CoreOpenOutcome(code: -7, message: nil), .unknown(-7, ""))
    }

    func testLookupStatesWithNoFile() {
        XCTAssertEqual(CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .absent).0, .newInstall)
        XCTAssertEqual(CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .unavailable("locked")).0, .keyUnavailable("locked"))
        XCTAssertEqual(CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(Data(count: 31))).0, .invalidArgument)
        XCTAssertFalse(FileManager.default.fileExists(atPath: path.path), "nothing created")
    }

    func testPlainDatabaseNeedsMigrationThenMigratesThroughTheKeychainCallback() throws {
        var id: Int64 = 0
        do { id = try XCTUnwrap(CoreMemoryStore(path: path, modelsDir: nil)?.insertMeeting(meeting("Plain meeting"))) }
        XCTAssertEqual(CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: .found(key)).0, .needsMigration)

        let keys = DbKeyStore(service: "keyed-store-ffi-tests.\(UUID().uuidString)")
        // Progress arrives on core's threads (a helper thread during the copy): collect it
        // behind a lock, as a caller that updates UI would hop to the main queue.
        final class Collected: @unchecked Sendable {
            private let lock = NSLock()
            private var values: [Double] = []
            func add(_ v: Double) { lock.withLock { values.append(v) } }
            var all: [Double] { lock.withLock { values } }
        }
        let overall = Collected()
        let result = CoreEncryptionMigration.run(dbPath: path, recording: false, key: key, keyStore: keys) { progress in
            overall.add(progress.overall)
        }
        XCTAssertEqual(result?.ok, true, "\(String(describing: result))")
        XCTAssertEqual(result?.phase, "done")
        XCTAssertEqual(keys.lookupPending(), .found(key), "pending stays until the shell deletes it")
        XCTAssertEqual(keys.lookup(), .found(key))
        XCTAssertEqual(overall.all.last, 1.0)
        XCTAssertTrue(CoreEncryptionMigration.plainBackupExists(dbPath: path))

        let (outcome, store) = CoreMemoryStore.openKeyed(path: path, modelsDir: nil, lookup: keys.lookup())
        XCTAssertEqual(outcome, .opened)
        XCTAssertEqual(store?.getMeeting(id: id)?.title, "Plain meeting")
        XCTAssertEqual(CoreEncryptionMigration.resume(dbPath: path)?.ok, true)
    }

    func testFailedKeychainWriteStopsTheMigration() throws {
        do { _ = CoreMemoryStore(path: path, modelsDir: nil)?.insertMeeting(meeting("Plain meeting")) }
        let before = try Data(contentsOf: path)
        var keychain = DbKeychain.live
        keychain.add = { _ in errSecNotAvailable }
        let keys = DbKeyStore(service: "keyed-store-ffi-tests.\(UUID().uuidString)", keychain: keychain)
        let result = CoreEncryptionMigration.run(dbPath: path, recording: false, key: key, keyStore: keys, progress: nil)
        XCTAssertEqual(result?.ok, false)
        XCTAssertEqual(result?.error?.kind, "key_store")
        XCTAssertEqual(try header(path), Data("SQLite format 3\0".utf8))
        XCTAssertEqual(try Data(contentsOf: path), before)
    }
}
