import Security
import XCTest
@testable import Whimbrel

/// The launch coordinator runs core's launch plan (X4, Task 11b-1) through injected fakes:
/// no Keychain, no store, no MCP server, no files. Each test scripts the decision's answers
/// (the plans core gives for one row of the decision table) and asserts the exact sequence
/// of collaborator calls, and that forbidden calls never happen.
@MainActor
final class LaunchCoordinatorTests: XCTestCase {
    // MARK: - Keys (labels in the call log)

    nonisolated static let K = Data(repeating: 1, count: 32)
    nonisolated static let P = Data(repeating: 2, count: 32)
    nonisolated static let R = Data(repeating: 3, count: 32)
    nonisolated static let N = Data(repeating: 4, count: 32)
    /// A recovery key that does not open the file.
    nonisolated static let W = Data(repeating: 5, count: 32)

    nonisolated static func label(_ key: Data?) -> String {
        switch key {
        case nil: return "nil"
        case K?: return "K"
        case P?: return "P"
        case R?: return "R"
        case N?: return "N"
        case W?: return "W"
        default: return "?"
        }
    }

    // MARK: - Plans (core's JSON shape)

    typealias Step = [String: Any]
    static let openPlain: Step = ["code": 0, "action": "open_plain"]
    static let openPlainRecord: Step = ["code": 1, "action": "open_plain_record_key_unavailable"]
    static let hardStop: Step = ["code": 2, "action": "hard_stop_no_key_storage"]
    static let keyUnavailable: Step = ["code": 3, "action": "show_key_unavailable_screen"]
    static func recovery(_ reason: String) -> Step { ["code": 4, "action": "show_recovery_screen", "reason": reason] }
    static let interrupted: Step = ["code": 5, "action": "show_migration_interrupted"]
    static func damaged(_ reason: String) -> Step { ["code": 6, "action": "show_damaged", "reason": reason] }
    static func failedBefore(_ n: Int) -> Step { ["code": 7, "action": "show_failed_before", "attempts": n] }
    static func skipped(_ reason: String) -> Step { ["code": 8, "action": "show_migration_skipped", "reason": reason] }
    static let migrationFailed: Step = ["code": 9, "action": "show_migration_failed"]
    static func onboarding(reuse: Bool) -> Step { ["code": 10, "action": "new_install_onboarding", "reuse_pending_key": reuse] }
    static let resume: Step = ["code": 11, "action": "finish_undo_via_resume"]
    static func precheck(_ rec: Bool = false) -> Step { ["code": 12, "action": "precheck_migration", "recording": rec] }
    static func run(_ source: String, _ rec: Bool = false) -> Step {
        ["code": 13, "action": "run_migration", "key_source": source, "recording": rec]
    }
    static func openKeyed(_ source: String, cache: Int? = nil) -> Step {
        ["code": 14, "action": "open_keyed", "key_source": source, "cache_kib": cache.map { $0 as Any } ?? NSNull()]
    }
    static let promote: Step = ["code": 15, "action": "promote_pending_key"]
    static let deletePending: Step = ["code": 16, "action": "delete_pending_key"]
    static let clearFlag: Step = ["code": 17, "action": "clear_migrate_flag"]
    static let startCounter: Step = ["code": 18, "action": "start_cleanup_counter"]
    static func recordGood(_ n: Int) -> Step { ["code": 19, "action": "record_good_launch", "count": n] }
    static func cleanup(_ n: Int) -> Step { ["code": 20, "action": "cleanup_plain_backup", "good_launches": n] }
    static let resetCounter: Step = ["code": 21, "action": "reset_cleanup_counter"]
    static let mcp: Step = ["code": 22, "action": "start_mcp_server"]
    static let encryptionAvailable: Step = ["code": 23, "action": "show_encryption_available"]
    static func fileMissing(backup: Bool, copy: Bool) -> Step {
        ["code": 24, "action": "show_encrypted_file_missing", "backup_present": backup, "encrypted_copy_present": copy]
    }
    static let backupKept: Step = ["code": 25, "action": "show_plain_backup_kept"]
    static let libraryMissing: Step = ["code": 26, "action": "notice_library_missing"]

    static func plan(_ steps: Step...) -> String {
        let data = try! JSONSerialization.data(withJSONObject: ["steps": steps])
        return String(data: data, encoding: .utf8)!
    }

    // MARK: - The fake world

    final class World: LaunchKeyStore, LaunchSettingsStore, LaunchMigrationRunner, LaunchPresenter {
        var log: [String] = []
        var plans: [String] = []
        /// When set, every decision answers this (the cap test).
        var always: String?
        var inputs: [[String: Any]] = []

        var dbKey: DbKeyLookup = .absent
        var pendingKey: DbKeyLookup = .absent
        var keyWriteFails = false
        var recoveredKeyRejected = false

        var fastSearch = false
        var migrateAtNextLaunch = false { didSet { log.append("setMigrateFlag(\(migrateAtNextLaunch))") } }
        var goodLaunches: UInt32 = 0 { didSet { log.append("setGoodLaunches(\(goodLaunches))") } }
        var recordingNow = false

        var resumeOutcome: CoreEncryptionMigration.Outcome? = .init(ok: true, phase: nil, error: nil)
        var precheckOutcome: CoreEncryptionMigration.Outcome? = .init(ok: true, phase: nil, error: nil)
        var runOutcome: CoreEncryptionMigration.Outcome? = .init(ok: true, phase: "done", error: nil)
        var cleanupRemoves = true
        var restoreResults: [CoreRestoreResult] = []
        var resetResult: CoreEncryptionFileAction = .done
        var deleteResult: CoreEncryptionFileAction = .done

        var keyedOutcomes: [CoreOpenOutcome] = []
        var plainOpens = true

        var responses: [LaunchScreenResponse] = []
        var screens: [LaunchScreen] = []
        var onboardingKey: Data? = N
        var progress: [Double] = []

        // Keys: the same never-replace rule as the live DbKeyStore. Only replaceAfterRecovery
        // with the proof replaces db-key, and it keeps the old key under `previous`.
        var previous: [Data] = []

        private func neverReplace(_ current: inout DbKeyLookup, with key: Data, account: String) throws {
            if keyWriteFails { throw DbKeyStoreError.writeFailed(errSecNotAvailable) }
            switch current {
            case let .found(existing) where existing == key: return
            case .found: throw DbKeyStoreError.mismatch(account)
            case let .unavailable(m): throw DbKeyStoreError.readFailed(errSecInteractionNotAllowed, m)
            case .absent: current = .found(key)
            }
        }

        func lookup() -> DbKeyLookup { log.append("lookup"); return dbKey }
        func lookupPending() -> DbKeyLookup { log.append("lookupPending"); return pendingKey }
        func storePending(_ key: Data) throws {
            log.append("storePending(\(label(key)))")
            try neverReplace(&pendingKey, with: key, account: "db-key-pending")
        }
        func storeKey(_ key: Data) throws {
            log.append("storeKey(\(label(key)))")
            try neverReplace(&dbKey, with: key, account: "db-key")
        }
        /// The Keychain reports the write as done but the read-back does not confirm it.
        var unverifiedWrite = false

        func replaceAfterRecovery(proven key: Data, replacing rejected: Data?) throws {
            log.append("replace(\(label(key)),rejected:\(label(rejected)))")
            try replace(key, rejected: rejected)
        }
        private func replace(_ key: Data, rejected: Data?) throws {
            if keyWriteFails { throw DbKeyStoreError.writeFailed(errSecNotAvailable) }
            switch dbKey {
            case .absent:
                _ = rejected // nothing stored: a plain store, as live
                if unverifiedWrite { throw DbKeyStoreError.storeUnverified("db-key did not read back") }
                dbKey = .found(key)
            case let .found(old) where old == key: return
            case let .found(old):
                guard let rejected else { throw DbKeyStoreError.notRejectedThisLaunch }
                guard rejected == old else { throw DbKeyStoreError.rejectedKeyChanged }
                if !previous.contains(old) { previous.append(old) }
                if unverifiedWrite { throw DbKeyStoreError.storeUnverified("db-key did not read back") }
                dbKey = .found(key)
            case let .unavailable(m): throw DbKeyStoreError.readFailed(errSecInteractionNotAllowed, m)
            }
        }
        func promotePendingKey(openedWith opened: Data, replacing rejected: Data?) throws {
            log.append("promotePending(opened:\(label(opened)),rejected:\(label(rejected)))")
            guard case let .found(p) = pendingKey else { throw DbKeyStoreError.noPendingKey }
            guard p == opened else { throw DbKeyStoreError.pendingNotOpened }
            try replace(p, rejected: rejected)
            pendingKey = .absent
        }
        func deletePending() throws { log.append("deletePending"); pendingKey = .absent }
        func migrationWrite(_ key: Data, account: String) -> OSStatus {
            log.append("migrationWrite(\(account),\(label(key)))")
            return errSecSuccess
        }

        // Migration
        func resume() async -> CoreEncryptionMigration.Outcome? {
            log.append("resume")
            // Like core: the original comes back from <db>.plain.bak, no run left.
            if useCore && resumeOutcome?.ok == true && files.contains(#""plain_backup_exists":true"#) { files = Self.plainNone }
            return resumeOutcome
        }
        func precheck(recording: Bool) async -> CoreEncryptionMigration.Outcome? {
            log.append("precheck(recording:\(recording))")
            return precheckOutcome
        }
        func run(key: Data, recording: Bool, storeKey: @escaping (String, Data) -> OSStatus,
                 progress: @escaping @Sendable (CoreEncryptionMigration.Progress) -> Void) async -> CoreEncryptionMigration.Outcome? {
            log.append("run(\(label(key)),recording:\(recording))")
            _ = storeKey("db-key-pending", key)
            progress(.init(step: 1, stepFraction: 1, overall: 0.5, note: nil))
            if runOutcome?.phase == "done" {
                _ = storeKey("db-key", key)
                dbKey = .found(key)
                if useCore { files = Self.notPlainDone; fileKey = key }
            }
            return runOutcome
        }
        static func onMainThread() -> Bool { Thread.isMainThread }
        var slowCleanup = false
        var cleanupRanOnMain: Bool?
        var ticked = false
        var tickedDuringCleanup = false
        /// Set while the slow cleanup is inside its sleep (the note-action test waits for it);
        /// written off the main thread, so behind a lock.
        private let flagLock = NSLock()
        private var _cleanupInProgress = false
        var cleanupInProgress: Bool {
            get { flagLock.withLock { _cleanupInProgress } }
            set { flagLock.withLock { _cleanupInProgress = newValue } }
        }
        /// Runs inside a keyed open, before its outcome (the file can vanish there).
        var beforeKeyedOpen: (() -> Void)?
        func cleanupPlainBackup(goodLaunches: UInt32) async -> Bool {
            log.append("cleanupPlainBackup(\(goodLaunches))")
            cleanupRanOnMain = Self.onMainThread()
            if slowCleanup {
                cleanupInProgress = true
                try? await Task.sleep(nanoseconds: 200_000_000)
                tickedDuringCleanup = ticked
                cleanupInProgress = false
            }
            return cleanupRemoves
        }
        func resetFailures() -> CoreEncryptionMigration.Outcome? { log.append("resetFailures"); return .init(ok: true, phase: nil, error: nil) }
        func restoreRollbackCopy() -> CoreRestoreResult {
            log.append("restoreRollbackCopy")
            if useCore && (restoreResults.first.map { $0.action == .done } ?? true) { files = Self.plainNone }
            return restoreResults.isEmpty ? CoreRestoreResult(action: .done, movedAside: 0, warning: .none) : restoreResults.removeFirst()
        }
        func resetEncryptionStatus() -> CoreEncryptionFileAction {
            log.append("resetEncryptionStatus")
            if useCore && resetResult == .done { files = Self.missingNone }
            return resetResult
        }
        func deletePlainBackup(openedWithKeyThisLaunch: Bool) -> (CoreEncryptionFileAction, removed: UInt32) {
            log.append("deletePlainBackup(opened:\(openedWithKeyThisLaunch))")
            return (deleteResult, 1)
        }

        // Presenter
        /// What changes in the world while a screen is up (the user unlocks the Mac, ...).
        var onPresent: ((LaunchScreen) -> Void)?

        func present(_ screen: LaunchScreen) async -> LaunchScreenResponse {
            log.append("present")
            screens.append(screen)
            onPresent?(screen)
            return responses.isEmpty ? .quit : responses.removeFirst()
        }
        func runOnboarding(existingKey: Data?) async -> Data? {
            log.append("onboarding(existing:\(label(existingKey)))")
            return onboardingKey
        }
        func migrationProgress(_ progress: CoreEncryptionMigration.Progress) { self.progress.append(progress.overall) }

        // The files and the decision. With `useCore` the plans come from the REAL core
        // decision (`whimbrel_launch_decision`) over `files`, not from a script.
        var useCore = false
        var files = #"{"db_file":"plain"}"#
        static let notPlain =
            #"{"db_file":"not_plain","plain_backup_exists":false,"encrypted_tmp_exists":false,"migration":{"kind":"none"}}"#
        static let notPlainDone =
            #"{"db_file":"not_plain","plain_backup_exists":true,"encrypted_tmp_exists":false,"migration":{"kind":"done"}}"#
        static let plainNone =
            #"{"db_file":"plain","plain_backup_exists":false,"encrypted_tmp_exists":false,"migration":{"kind":"none"}}"#
        static let missingNone =
            #"{"db_file":"missing","plain_backup_exists":false,"encrypted_tmp_exists":false,"migration":{"kind":"none"}}"#
        static let missingBackup =
            #"{"db_file":"missing","plain_backup_exists":true,"encrypted_tmp_exists":false,"migration":{"kind":"none"}}"#
        static let missingBackupInProgress =
            #"{"db_file":"missing","plain_backup_exists":true,"encrypted_tmp_exists":false,"migration":{"kind":"in_progress","phase":"swapped"}}"#
        /// A keyed open with the right key whose startup sweep fails.
        var startupFails = false
        static let missingInProgress =
            #"{"db_file":"missing","plain_backup_exists":false,"encrypted_tmp_exists":false,"migration":{"kind":"in_progress","phase":"verified"}}"#

        /// When set, a keyed open succeeds only with these bytes (else key rejected).
        var fileKey: Data?

        func decide(_ json: String) -> String? {
            log.append("decide")
            inputs.append(try! JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: Any])
            if useCore { return CoreLaunch.decide(inputJSON: json) }
            if let always { return always }
            return plans.isEmpty ? nil : plans.removeFirst()
        }
    }

    final class FakeStore {
        let keyed: Bool
        init(keyed: Bool) { self.keyed = keyed }
    }

    var world: World!
    var coordinator: LaunchCoordinator<FakeStore>!

    override func setUp() async throws {
        world = World()
        coordinator = makeCoordinator(world)
    }

    func makeCoordinator(_ w: World) -> LaunchCoordinator<FakeStore> {
        LaunchCoordinator(collaborators(w))
    }

    func collaborators(_ w: World) -> LaunchCoordinator<FakeStore>.Collaborators {
        LaunchCoordinator<FakeStore>.Collaborators(
            observeFiles: { w.log.append("observe"); return w.files },
            decide: { w.decide($0) },
            keys: w,
            settings: w,
            physicalMemory: { 32 * 1024 * 1024 * 1024 },
            recording: { w.recordingNow },
            migration: w,
            openPlain: {
                w.log.append("openPlain")
                return w.plainOpens ? FakeStore(keyed: false) : nil
            },
            openKeyed: { key, cache, mustExist in
                w.log.append("openKeyed(\(Self.label(key)),cache:\(cache.map(String.init) ?? "nil")\(mustExist ? ",existing" : ""))")
                w.beforeKeyedOpen?()
                let outcome: CoreOpenOutcome
                if w.files.contains(#""db_file":"missing""#) {
                    // Like core: a never-create open refuses; a normal open CREATES a database.
                    outcome = mustExist ? .damaged("no database file") : .opened
                    if !mustExist { w.log.append("created(\(Self.label(key)))") }
                } else if let fileKey = w.fileKey {
                    outcome = key == fileKey ? (w.startupFails ? .startupFailed("sweep") : .opened) : .keyRejected("wrong key")
                } else {
                    outcome = w.keyedOutcomes.isEmpty ? .opened : w.keyedOutcomes.removeFirst()
                }
                return (outcome, outcome == .opened ? FakeStore(keyed: true) : nil)
            },
            startMcpServer: { key in w.log.append("startMcp(\(Self.label(key)))") },
            presenter: w,
            now: { 1000 }
        )
    }

    /// One "ask": gather the inputs, then the decision.
    let ask = ["observe", "lookup", "lookupPending", "decide"]

    private func opened(_ outcome: LaunchOutcome<FakeStore>, file: StaticString = #filePath, line: UInt = #line) -> LaunchOpened<FakeStore>? {
        guard case let .opened(o) = outcome else {
            XCTFail("expected an open, got \(outcome)", file: file, line: line)
            return nil
        }
        return o
    }

    private func thisLaunch(_ n: Int) -> [String: Any] { world.inputs[n]["this_launch"] as! [String: Any] }

    private let forbiddenOnAScreen = ["openPlain", "openKeyed", "startMcp", "run(", "storeKey", "storePending", "promotePending",
                                      "deletePending", "setGoodLaunches", "cleanupPlainBackup", "setMigrateFlag"]

    private func assertNever(_ prefixes: [String], file: StaticString = #filePath, line: UInt = #line) {
        for p in prefixes {
            XCTAssertFalse(world.log.contains { $0.hasPrefix(p) }, "\(p) must not be called: \(world.log)", file: file, line: line)
        }
    }

    // MARK: - Plain file

    func testPlainWithNoKeyOpensPlainThenTheMcpServer() async {
        world.plans = [Self.plan(Self.openPlain, Self.mcp)]
        let o = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["openPlain", "startMcp(nil)"])
        XCTAssertEqual(o?.key, nil)
        XCTAssertEqual(coordinator.notes, [])
        // The input carried presence only, settings, RAM and an empty this-launch record.
        let input = world.inputs[0]
        XCTAssertEqual(input["db_key"] as? String, "absent")
        XCTAssertEqual(input["pending_key"] as? String, "absent")
        XCTAssertEqual((input["files"] as? [String: Any])?["db_file"] as? String, "plain", "files passed through untouched")
        XCTAssertEqual(input["physical_ram_bytes"] as? UInt64, 32 * 1024 * 1024 * 1024)
        XCTAssertEqual(thisLaunch(0)["resume"] as? String, "not_run")
        XCTAssertEqual(thisLaunch(0)["migration"] as? String, "not_tried")
        XCTAssertEqual(Set(input.keys), ["files", "db_key", "pending_key", "migrate_at_next_launch", "recording", "settings",
                                         "physical_ram_bytes", "good_launches", "this_launch"])
        XCTAssertEqual(Set(thisLaunch(0).keys), ["resume", "migration", "db_key_rejected", "pending_key_rejected", "key_store_failed"])
        assertNever(["openKeyed", "resume", "precheck", "run(", "present", "onboarding"])
    }

    func testPlainWithTheKeyStoreUnavailableOpensPlainAndRecordsTheNote() async {
        world.dbKey = .unavailable("locked (-25308)")
        world.plans = [Self.plan(Self.openPlainRecord, Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["openPlain", "startMcp(nil)"])
        XCTAssertEqual(world.inputs[0]["db_key"] as? String, "unavailable")
        XCTAssertEqual(coordinator.notes, [.keyStoreUnavailable(LaunchDiagnostic(diagnosticOnly: "locked (-25308)"))])
        assertNever(["openKeyed", "run(", "storeKey", "storePending"])
    }

    func testPlainWithDbKeyAndNoOptInOffersEncryption() async {
        world.dbKey = .found(Self.K)
        world.plans = [Self.plan(Self.openPlain, Self.encryptionAvailable, Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["openPlain", "startMcp(nil)"], "a plain store never gets the key")
        XCTAssertEqual(coordinator.notes, [.encryptionAvailable])
    }

    // MARK: - Library missing (Task 13c item 4)

    /// A coordinator whose store-marker calls go to the log.
    private func coordinatorWithMarker() -> LaunchCoordinator<FakeStore> {
        var c = collaborators(world)
        c.markStoreExisted = { [world] in world!.log.append("markStoreExisted") }
        c.clearStoreExisted = { [world] in world!.log.append("clearStoreExisted") }
        return LaunchCoordinator(c)
    }

    func testTheNoticeStepDecodesAndOnlyComesAfterAnOpen() {
        let steps = LaunchStep.plan(json: Self.plan(Self.openKeyed("db_key"), Self.libraryMissing, Self.mcp))
        XCTAssertEqual(steps, [.openKeyed(.dbKey, cacheKib: nil), .noticeLibraryMissing, .startMcpServer])
        XCTAssertEqual(LaunchStep.noticeLibraryMissing.code, 26)
        XCTAssertNil(LaunchCoordinator<FakeStore>.validate([.openKeyed(.dbKey, cacheKib: nil), .noticeLibraryMissing, .startMcpServer]))
        XCTAssertEqual(LaunchCoordinator<FakeStore>.validate([.noticeLibraryMissing, .openKeyed(.dbKey, cacheKib: nil)]),
                       .stepOutOfOrder(code: 26))
    }

    /// The notice step clears the marker and adds a launch-only note; the marker is written
    /// again once the store opened (so the next launch, which finds the new database, is quiet).
    func testTheLibraryMissingNoticeShowsOnceAndTheMarkerComesBackAfterTheOpen() async {
        world.dbKey = .found(Self.K)
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.libraryMissing, Self.mcp), Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        let c = coordinatorWithMarker()
        _ = opened(await c.run())
        XCTAssertEqual(world.log, ask + ["openKeyed(K,cache:nil)", "clearStoreExisted", "startMcp(K)", "markStoreExisted"])
        XCTAssertEqual(c.notes, [.libraryMissing])
        XCTAssertTrue(LaunchNote.libraryMissing.isLaunchOnly, "the decision cannot repeat it: the database exists now")
        // A note action asks the decision again: the launch-only notice stays until dismissed.
        XCTAssertEqual(c.deletePlainBackup(), .done)
        XCTAssertEqual(c.notes, [.libraryMissing])
        c.dismissLibraryMissingNotice()
        XCTAssertEqual(c.notes, [])
    }

    /// Fix (C): "Reset encryption status and start empty" is the user's own choice of an empty
    /// start, so the store marker goes with it and no "library not found" notice follows.
    /// A reset that did not happen keeps the marker.
    func testResetAndStartEmptyClearsTheStoreMarker() async {
        world.dbKey = .found(Self.K)
        world.plans = [Self.plan(Self.fileMissing(backup: false, copy: false)), Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        world.responses = [.resetEncryptionStatus]
        _ = opened(await coordinatorWithMarker().run())
        let cleared = world.log.firstIndex(of: "clearStoreExisted")
        XCTAssertNotNil(cleared, "\(world.log)")
        XCTAssertLessThan(cleared ?? 0, world.log.firstIndex(of: "openKeyed(K,cache:nil)") ?? 0, "cleared before the next decision")
        XCTAssertEqual(world.log.last, "markStoreExisted")

        world = World()
        world.resetResult = .refused
        world.plans = [Self.plan(Self.fileMissing(backup: false, copy: false))]
        world.responses = [.resetEncryptionStatus, .quit]
        guard case .quit = await coordinatorWithMarker().run() else { return XCTFail() }
        XCTAssertFalse(world.log.contains("clearStoreExisted"))
    }

    /// Every open records that a store existed here; a launch that goes on without the store
    /// (or quits) does not.
    func testTheMarkerIsWrittenAfterEveryOpenAndNeverWithoutAStore() async {
        world.plans = [Self.plan(Self.openPlain, Self.mcp)]
        _ = opened(await coordinatorWithMarker().run())
        XCTAssertEqual(world.log.last, "markStoreExisted")
        XCTAssertEqual(world.log.filter { $0 == "markStoreExisted" }.count, 1)

        world = World()
        world.plainOpens = false
        world.plans = [Self.plan(Self.openPlain, Self.mcp)]
        world.responses = [.continueWithoutStore]
        guard case .openedWithoutStore = await coordinatorWithMarker().run() else { return XCTFail() }
        XCTAssertFalse(world.log.contains("markStoreExisted"))

        world = World()
        world.plans = [Self.plan(Self.hardStop)]
        world.responses = [.quit]
        guard case .quit = await coordinatorWithMarker().run() else { return XCTFail() }
        XCTAssertFalse(world.log.contains("markStoreExisted"))
    }

    // MARK: - Missing file

    func testMissingFileWithTheKeyStoreUnavailableIsAHardStopWithNothingCreated() async {
        world.dbKey = .unavailable("denied")
        world.plans = [Self.plan(Self.hardStop), Self.plan(Self.hardStop)]
        world.responses = [.retry, .quit]
        let outcome = await coordinator.run()
        guard case .quit = outcome else { return XCTFail("\(outcome)") }
        XCTAssertEqual(world.screens, [.keyStorageUnavailable, .keyStorageUnavailable])
        XCTAssertEqual(world.log, ask + ["present"] + ask + ["present"], "Retry asks again")
        assertNever(forbiddenOnAScreen + ["onboarding"])
    }

    func testNewInstallRunsTheSetupStoresThePendingKeyThenDbKeyThenCreatesTheEncryptedStore() async {
        world.plans = [Self.plan(Self.onboarding(reuse: false)),
                       Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        let o = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["onboarding(existing:nil)", "storePending(N)", "storeKey(N)"]
                       + ask + ["openKeyed(N,cache:nil)", "startMcp(N)"])
        XCTAssertEqual(o?.key, Self.N)
        XCTAssertEqual(o?.store.keyed, true)
    }

    func testNewInstallWithAPendingKeyReusesItAndNeverMakesAnother() async {
        world.pendingKey = .found(Self.P)
        world.onboardingKey = Self.P
        world.plans = [Self.plan(Self.onboarding(reuse: true)), Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["onboarding(existing:P)", "storeKey(P)"] + ask + ["openKeyed(P,cache:nil)", "startMcp(P)"],
                       "the pending key is already stored: only db-key is written")
    }

    /// The coordinator itself refuses a fresh key while a key exists or cannot be read, even
    /// if a plan asked for one: no setup UI, no key write.
    func testAFreshKeyIsNeverMadeWhileAKeyExistsOrCannotBeRead() async {
        for (db, pending) in [(DbKeyLookup.absent, DbKeyLookup.found(Self.P)), (.found(Self.K), .absent),
                              (.absent, .unavailable("x")), (.unavailable("x"), .absent)] {
            world = World()
            world.dbKey = db
            world.pendingKey = pending
            world.plans = [Self.plan(Self.onboarding(reuse: false))]
            guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
            XCTAssertEqual(world.screens, [.error(.freshKeyRefused)], "\(db) \(pending)")
            assertNever(forbiddenOnAScreen + ["onboarding"])
        }
    }

    /// The setup ran and only the Keychain write failed: Retry stores the same key, without
    /// a second setup (the words the user saved stay valid).
    func testAFailedOnboardingWriteIsRetriedWithTheSameKey() async {
        world.keyWriteFails = true
        world.plans = [Self.plan(Self.onboarding(reuse: false)), Self.plan(Self.hardStop),
                       Self.plan(Self.onboarding(reuse: false)), Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        world.responses = [.retry]
        let w = world!
        w.onPresent = { _ in w.keyWriteFails = false }
        let o = opened(await coordinator.run())
        XCTAssertEqual(w.log.filter { $0.hasPrefix("onboarding") }, ["onboarding(existing:nil)"], "one setup only")
        XCTAssertEqual(w.log.filter { $0.hasPrefix("store") }, ["storePending(N)", "storePending(N)", "storeKey(N)"])
        XCTAssertEqual(o?.key, Self.N)
    }

    func testOnboardingThatHandsBackADifferentKeyThanThePendingOneIsRefused() async {
        world.pendingKey = .found(Self.P)
        world.onboardingKey = Self.N
        world.plans = [Self.plan(Self.onboarding(reuse: true))]
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.onboardingKeyMismatch)])
        assertNever(forbiddenOnAScreen)
    }

    func testNewInstallKeyStoreFailureRecordsItAndAsksAgain() async {
        world.keyWriteFails = true
        world.plans = [Self.plan(Self.onboarding(reuse: false)), Self.plan(Self.hardStop), Self.plan(Self.hardStop)]
        world.responses = [.retry, .quit]
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(thisLaunch(0)["key_store_failed"] as? Bool, false)
        XCTAssertEqual(thisLaunch(1)["key_store_failed"] as? Bool, true)
        XCTAssertEqual(thisLaunch(2)["key_store_failed"] as? Bool, false, "Retry clears it")
        XCTAssertEqual(world.screens, [.keyStorageUnavailable, .keyStorageUnavailable])
        assertNever(["openPlain", "openKeyed", "startMcp", "storeKey"])
    }

    // MARK: - Encrypted file

    func testEncryptedWithTheKeyStoreUnavailableShowsTheRetryScreenAndRetryAsksAgain() async {
        world.dbKey = .unavailable("interaction not allowed")
        world.plans = [Self.plan(Self.keyUnavailable), Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        world.responses = [.retry]
        // The Keychain answers by the time the user presses Retry.
        let w = world!
        w.onPresent = { _ in w.dbKey = .found(Self.K) }
        let o = opened(await coordinator.run())
        XCTAssertEqual(w.screens, [.keyUnavailable(LaunchDiagnostic(diagnosticOnly: "interaction not allowed"))])
        XCTAssertEqual(w.log, ask + ["present"] + ask + ["openKeyed(K,cache:nil)", "startMcp(K)"])
        XCTAssertEqual(w.inputs[1]["db_key"] as? String, "found", "the keys are read again")
        XCTAssertEqual(o?.key, Self.K)
    }

    /// No db-key at all: the restored key is tried by the plan's own open and stored only
    /// after it opened (a plain add, nothing to replace).
    func testEncryptedWithNoKeyShowsRecoveryAndARestoredKeyIsStoredOnlyAfterItOpened() async {
        world.useCore = true
        world.files = World.notPlain
        world.fileKey = Self.R
        world.responses = [.restoredKey(Self.R)]
        let o = opened(await coordinator.run())
        XCTAssertEqual(world.screens, [.recovery(.needsKey)])
        XCTAssertEqual(world.log, ask + ["present"] + ask + ["openKeyed(R,cache:nil,existing)", "replace(R,rejected:nil)", "startMcp(R)"])
        XCTAssertEqual(world.inputs[1]["db_key"] as? String, "found", "the candidate stands in for db-key")
        XCTAssertEqual(world.dbKey, .found(Self.R))
        XCTAssertEqual(o?.key, Self.R)
    }

    /// The real decision, a wrong db-key K in the Keychain, the file opened only by R. A wrong
    /// restored key W goes back to the recovery screen with ZERO Keychain writes; R opens,
    /// and only then replaces K, which is kept under db-key-previous.
    func testRecoveryOverARejectedKeyProvesBeforeItPersists() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.responses = [.restoredKey(Self.W), .restoredKey(Self.R)]
        let o = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["openKeyed(K,cache:nil)"]
                       + ask + ["present"] + ask + ["openKeyed(W,cache:nil,existing)"]
                       + ask + ["present"] + ask + ["openKeyed(R,cache:nil,existing)", "replace(R,rejected:K)", "startMcp(R)"],
                       "nothing is written before R opened; the rejected plans' MCP server never starts")
        XCTAssertEqual(world.screens, [.recovery(.keyRejected), .recovery(.keyRejected)])
        XCTAssertEqual(world.dbKey, .found(Self.R))
        XCTAssertEqual(world.previous, [Self.K], "the old key is never destroyed")
        XCTAssertEqual(o?.key, Self.R)
        XCTAssertEqual(coordinator.notes, [])

        // The next launch converges: no screen, no write.
        world.log = []
        world.screens = []
        _ = opened(await makeCoordinator(world).run())
        XCTAssertEqual(world.log, ask + ["openKeyed(R,cache:nil)", "startMcp(R)"])
    }

    /// The restored key opened the store but the Keychain refused it: a note, not an error.
    func testAProvenKeyThatCannotBeStoredIsANoteAndTheStoreIsOpen() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.keyWriteFails = true
        world.responses = [.restoredKey(Self.R)]
        let o = opened(await coordinator.run())
        XCTAssertEqual(o?.key, Self.R)
        XCTAssertEqual(world.log.suffix(3), ["openKeyed(R,cache:nil,existing)", "replace(R,rejected:K)", "startMcp(R)"])
        guard case .recoveredKeyNotStored(.keychainUnavailable, _)? = coordinator.notes.first else {
            return XCTFail("\(coordinator.notes)")
        }
        XCTAssertEqual(world.dbKey, .found(Self.K), "nothing replaced")
    }

    func testThePendingKeyOpensAndIsPromoted() async {
        world.pendingKey = .found(Self.P)
        world.plans = [Self.plan(Self.openKeyed("pending_key"), Self.promote, Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["openKeyed(P,cache:nil)", "promotePending(opened:P,rejected:nil)", "startMcp(P)"])
        XCTAssertEqual(world.dbKey, .found(Self.P))
    }

    /// The real decision: db-key K is rejected, the pending key P opens and is promoted over
    /// K (kept under db-key-previous), and the next launch opens with db-key and no note.
    func testARejectedDbKeyWithAPendingKeyThatOpensConverges() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.pendingKey = .found(Self.P)
        world.fileKey = Self.P
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["openKeyed(K,cache:nil)"]
                       + ask + ["openKeyed(P,cache:nil)", "promotePending(opened:P,rejected:K)", "startMcp(P)"])
        XCTAssertEqual(coordinator.notes, [])
        XCTAssertEqual(world.dbKey, .found(Self.P))
        XCTAssertEqual(world.pendingKey, .absent)
        XCTAssertEqual(world.previous, [Self.K])

        world.log = []
        let next = makeCoordinator(world)
        _ = opened(await next.run())
        XCTAssertEqual(world.log, ask + ["openKeyed(P,cache:nil)", "startMcp(P)"], "converged: no promotion again")
        XCTAssertEqual(next.notes, [])
    }

    /// The pending key is deleted only when it is the very key the store opened with.
    func testADifferentPendingKeyIsNeverDeletedAndTheNoteSurvivesARefresh() async {
        world.dbKey = .found(Self.K)
        world.pendingKey = .found(Self.P)
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.deletePending, Self.backupKept, Self.mcp),
                       Self.plan(Self.openKeyed("db_key"), Self.deletePending, Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertFalse(world.log.contains("deletePending"))
        XCTAssertEqual(world.pendingKey, .found(Self.P))
        XCTAssertEqual(coordinator.notes, [.pendingKeyNotDeleted(.pendingDiffers, LaunchDiagnostic(diagnosticOnly: "")), .plainBackupKept])
        _ = coordinator.deletePlainBackup()
        XCTAssertEqual(coordinator.notes, [.pendingKeyNotDeleted(.pendingDiffers, LaunchDiagnostic(diagnosticOnly: ""))],
                       "a note about what this launch did stays after a refresh")
    }

    func testAKeyMissingForAPlannedStepIsASpecificErrorNotALoop() async {
        world.plans = [Self.plan(Self.openKeyed("pending_key"), Self.mcp)]
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.keyMissing(.pendingKey))])
        XCTAssertEqual(world.log.filter { $0 == "decide" }.count, 1)

        world = World()
        world.plans = [Self.plan(Self.precheck(), Self.run("db_key"))]
        guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.keyMissing(.dbKey))])
        assertNever(["run("])
    }

    func testDamagedOffersOnlyQuitAndNoAccessOffersRetry() async {
        world.plans = [Self.plan(Self.damaged("damaged"))]
        world.responses = [.retry, .restoreRollbackCopy, .quit]
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.screens.count, 3, "an answer the screen does not offer shows it again")
        XCTAssertEqual(world.screens[0], .damaged(.damaged))
        XCTAssertEqual(world.log.filter { $0 == "decide" }.count, 1)
        assertNever(forbiddenOnAScreen + ["restoreRollbackCopy"])

        // Permissions: Retry looks at the files again.
        world = World()
        world.plans = [Self.plan(Self.damaged("no_access")), Self.plan(Self.openPlain, Self.mcp)]
        world.responses = [.retry]
        _ = opened(await makeCoordinator(world).run())
        XCTAssertEqual(world.screens, [.damaged(.noAccess)])
        XCTAssertEqual(world.log, ask + ["present"] + ask + ["openPlain", "startMcp(nil)"])

        // Another program holds the file (Windows): in use, not damaged; Retry looks again.
        world = World()
        world.plans = [Self.plan(Self.damaged("open_elsewhere")), Self.plan(Self.openPlain, Self.mcp)]
        world.responses = [.retry]
        _ = opened(await makeCoordinator(world).run())
        XCTAssertEqual(world.screens, [.damaged(.openElsewhere)])
        XCTAssertEqual(world.log, ask + ["present"] + ask + ["openPlain", "startMcp(nil)"])
    }

    // MARK: - Migration

    func testMigrationResumesThenPrechecksThenRunsThenOpensKeyedAndCountsOnlyAfterTheFirstWindow() async {
        world.pendingKey = .found(Self.P)
        world.migrateAtNextLaunch = true
        world.fastSearch = true
        world.log = []
        world.plans = [
            Self.plan(Self.resume),
            Self.plan(Self.precheck(), Self.run("pending_key")),
            Self.plan(Self.openKeyed("db_key", cache: 1_048_576), Self.deletePending, Self.clearFlag, Self.startCounter, Self.mcp),
        ]
        let o = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["resume"]
                       + ask + ["precheck(recording:false)", "run(P,recording:false)",
                                "migrationWrite(db-key-pending,P)", "migrationWrite(db-key,P)"]
                       + ask + ["openKeyed(P,cache:1048576)", "deletePending", "setMigrateFlag(false)", "startMcp(P)"])
        XCTAssertEqual(thisLaunch(1)["resume"] as? String, "ran")
        XCTAssertEqual(thisLaunch(2)["migration"] as? String, "done")
        XCTAssertEqual((world.inputs[0]["settings"] as? [String: Any])?["fast_search"] as? Bool, true)
        XCTAssertEqual(world.inputs[0]["migrate_at_next_launch"] as? Bool, true)
        XCTAssertEqual(world.progress, [0.5])
        XCTAssertEqual(world.goodLaunches, 0, "never counted at the open")
        XCTAssertEqual(o?.key, Self.P)

        world.log = []
        await coordinator.firstWindowRendered()
        await coordinator.firstWindowRendered()
        XCTAssertEqual(world.log, ["setGoodLaunches(1)"], "once, after the first window")
    }

    func testTheCounterStepsRunOnlyAfterTheFirstWindow() async {
        world.dbKey = .found(Self.K)
        world.goodLaunches = 3
        world.log = []
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.cleanup(3), Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["openKeyed(K,cache:nil)", "startMcp(K)"])
        await coordinator.firstWindowRendered()
        XCTAssertEqual(world.log.suffix(2), ["cleanupPlainBackup(3)", "setGoodLaunches(0)"])

        for (step, expected) in [(Self.recordGood(2), "setGoodLaunches(2)"), (Self.resetCounter, "setGoodLaunches(0)")] {
            world = World()
            world.dbKey = .found(Self.K)
            world.plans = [Self.plan(Self.openKeyed("db_key"), step, Self.mcp)]
            let c = makeCoordinator(world)
            _ = opened(await c.run())
            XCTAssertFalse(world.log.contains(expected))
            await c.firstWindowRendered()
            XCTAssertEqual(world.log.last, expected)
        }
    }

    func testAFailedCleanupKeepsTheCounter() async {
        world.dbKey = .found(Self.K)
        world.cleanupRemoves = false
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.cleanup(3), Self.mcp)]
        _ = opened(await coordinator.run())
        await coordinator.firstWindowRendered()
        XCTAssertEqual(world.log.last, "cleanupPlainBackup(3)")
    }

    func testAPrecheckSkipEndsThePlanAndTheNoteCarriesItsReason() async {
        world.dbKey = .found(Self.K)
        world.migrateAtNextLaunch = true
        let skip = CoreEncryptionMigration.Skip(reason: "low_disk", needBytes: 10, freeBytes: 1, phase: nil, attempts: nil, last: nil)
        world.precheckOutcome = .init(ok: false, phase: nil, error: .init(kind: "skipped", message: "m", skip: skip))
        world.log = []
        world.plans = [Self.plan(Self.precheck(), Self.run("db_key")),
                       Self.plan(Self.openPlain, Self.skipped("precheck"), Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["precheck(recording:false)"] + ask + ["openPlain", "startMcp(nil)"])
        XCTAssertEqual(thisLaunch(1)["migration"] as? String, "skipped")
        XCTAssertEqual(coordinator.notes, [.migrationSkipped(.precheck, skip: skip)])
        // A note's button refreshes the notes: the skip detail is kept.
        world.plans = [Self.plan(Self.openPlain, Self.skipped("precheck"), Self.mcp)]
        _ = coordinator.deletePlainBackup()
        XCTAssertEqual(coordinator.notes, [.migrationSkipped(.precheck, skip: skip)])
    }

    /// The cleanup deletes a file: it never blocks the main actor.
    /// While the deferred cleanup runs (it may delete the same backup), the note actions do
    /// nothing; afterwards they work again.
    func testNoteActionsDoNothingWhileTheCleanupRuns() async {
        world.dbKey = .found(Self.K)
        world.slowCleanup = true
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.cleanup(3), Self.backupKept, Self.mcp),
                       Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        _ = opened(await coordinator.run())
        let w = world!
        let c = coordinator!
        let rendering = Task { await c.firstWindowRendered() }
        let deadline = Date().addingTimeInterval(5)
        while !w.cleanupInProgress && Date() < deadline { try? await Task.sleep(nanoseconds: 1_000_000) }
        guard w.cleanupInProgress else { await rendering.value; return XCTFail("the cleanup never started within 5 s") }
        XCTAssertTrue(c.cleanupRunning)
        XCTAssertEqual(c.deletePlainBackup(), .refused)
        XCTAssertEqual(c.resetEncryptionStatus(), .refused)
        XCTAssertFalse(c.retryAfterFailedBefore())
        XCTAssertFalse(w.log.contains { $0.hasPrefix("deletePlainBackup") || $0 == "resetEncryptionStatus" || $0 == "resetFailures" })
        XCTAssertEqual(w.log.filter { $0 == "decide" }.count, 1, "no notes refresh meanwhile")
        await rendering.value
        XCTAssertFalse(c.cleanupRunning)
        XCTAssertEqual(c.deletePlainBackup(), .done)
        XCTAssertTrue(w.log.contains("deletePlainBackup(opened:true)"))
    }

    /// The pending key is promoted only right after the store opened with it.
    func testThePlanCannotPromoteAPendingKeyTheStoreDidNotOpenWith() async {
        world.dbKey = .found(Self.K)
        world.pendingKey = .found(Self.P)
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.promote, Self.mcp)]
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.stepOutOfOrder(code: 15))])
        assertNever(["openKeyed", "promotePending", "startMcp"])
    }

    /// The file vanished between the recovery screen and the open, and stays missing through
    /// several Retries: the same screen every time, nothing decided, opened or written, and
    /// never a creating open with the restored key or the rejected K.
    func testWhileTheFileIsMissingRetryShowsTheSameScreenAndOpensNothing() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.responses = [.restoredKey(Self.R), .retry, .retry, .quit]
        let w = world!
        w.onPresent = { screen in if case .recovery = screen { w.files = World.missingNone } }
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(w.screens, [.recovery(.keyRejected)] + Array(repeating: .error(.databaseMissingForRecoveredKey), count: 3))
        XCTAssertEqual(w.log.filter { $0.hasPrefix("openKeyed") }, ["openKeyed(K,cache:nil)"], "nothing opens while the file is missing")
        XCTAssertEqual(w.log.filter { $0 == "decide" }.count, 2, "nothing is decided while the restored key waits")
        assertNever(["created(", "replace(", "storeKey", "storePending", "startMcp", "onboarding"])
        XCTAssertEqual(w.dbKey, .found(Self.K))
    }

    /// The file comes back: Retry tries the kept restored key on it, and it is proven there.
    func testWhenTheFileIsBackTheKeptRestoredKeyIsTriedAndProven() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.responses = [.restoredKey(Self.R), .retry]
        let w = world!
        w.onPresent = { screen in
            switch screen {
            case .recovery: w.files = World.missingNone
            case .error(.databaseMissingForRecoveredKey): w.files = World.notPlain
            default: break
            }
        }
        let o = opened(await coordinator.run())
        XCTAssertEqual(w.screens, [.recovery(.keyRejected), .error(.databaseMissingForRecoveredKey)])
        XCTAssertEqual(w.log.suffix(3), ["openKeyed(R,cache:nil,existing)", "replace(R,rejected:K)", "startMcp(R)"])
        XCTAssertEqual(o?.key, Self.R)
        XCTAssertEqual(w.previous, [Self.K])
        assertNever(["created("])
    }

    /// A restored key is waiting, and the file is missing with its unencrypted copy beside it:
    /// the key is dropped, core decides as before, and its restore screen works.
    func testWithACandidateAMissingFileWithItsBackupStillReachesTheRestoreScreen() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.responses = [.restoredKey(Self.R), .restoreRollbackCopy]
        let w = world!
        w.onPresent = { screen in if case .recovery = screen { w.files = World.missingBackup } }
        _ = opened(await coordinator.run())
        XCTAssertEqual(w.screens, [.recovery(.keyRejected),
                                   .encryptedFileMissing(backupPresent: true, encryptedCopyPresent: false, lastAction: nil)])
        XCTAssertTrue(w.log.contains("restoreRollbackCopy"))
        XCTAssertEqual(w.log.suffix(2), ["openPlain", "startMcp(nil)"])
        assertNever(["replace(", "storeKey", "storePending", "created("])
        XCTAssertEqual(w.dbKey, .found(Self.K))
    }

    /// A restored key is waiting, and the file is missing with an unfinished run: the key is
    /// dropped and resume runs, as core decides.
    func testWithACandidateAMissingFileWithAnUnfinishedRunStillResumes() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.responses = [.restoredKey(Self.R)]
        let w = world!
        w.onPresent = { screen in if case .recovery = screen { w.files = World.missingBackupInProgress } }
        _ = opened(await coordinator.run())
        XCTAssertTrue(w.log.contains("resume"), "\(w.log)")
        XCTAssertEqual(w.log.suffix(2), ["openPlain", "startMcp(nil)"])
        assertNever(["replace(", "storeKey", "storePending", "created("])
    }

    /// The file comes back PLAIN with the opt-in set: the unproven restored key never reaches
    /// the migration; the migration uses the Keychain's own db-key.
    func testWithACandidateAFileThatReturnsPlainMigratesWithTheKeychainKey() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.migrateAtNextLaunch = true
        world.responses = [.restoredKey(Self.R)]
        let w = world!
        w.onPresent = { screen in if case .recovery = screen { w.files = World.plainNone } }
        guard case .opened = await coordinator.run() else { return XCTFail("\(w.screens) \(w.log)") }
        XCTAssertTrue(w.log.contains("run(K,recording:false)"), "\(w.log)")
        XCTAssertFalse(w.log.contains { $0.contains("R") && ($0.hasPrefix("migrationWrite") || $0.hasPrefix("run(")) },
                       "db-key-pending is never written with the restored key")
        assertNever(["replace("])
    }

    /// The file comes back as a DIFFERENT encrypted file: the restored key is tried and
    /// rejected, nothing is stored. K is not tried again in this launch (it was rejected
    /// earlier; known limitation, the recovery screen shows).
    func testWithACandidateADifferentEncryptedFileRejectsItAndStoresNothing() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.responses = [.restoredKey(Self.R), .quit]
        let w = world!
        w.onPresent = { screen in if case .recovery = screen { w.fileKey = Self.N } }
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(w.screens, [.recovery(.keyRejected), .recovery(.keyRejected)])
        XCTAssertTrue(w.log.contains("openKeyed(R,cache:nil,existing)"))
        assertNever(["replace(", "storeKey", "storePending", "startMcp"])
    }

    /// The restored key opened the file but the startup sweep failed: the key is proven, so it
    /// is stored rather than lost, and the error shows.
    func testAStartupFailureAfterTheRestoredKeyOpenedStillStoresIt() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.startupFails = true
        world.responses = [.restoredKey(Self.R), .quit]
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.screens.last, .error(.keyedOpenFailed(.startupFailed("sweep"))))
        XCTAssertTrue(world.log.contains("replace(R,rejected:K)"))
        XCTAssertEqual(world.dbKey, .found(Self.R))
        XCTAssertEqual(world.previous, [Self.K])
    }

    /// The race: the file is there when looked at and gone by the time of the open. The proof
    /// open never creates; the key is kept and the missing-file screen shows.
    func testAFileThatVanishesDuringTheProofOpenIsNeverCreated() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.responses = [.restoredKey(Self.R), .quit]
        let w = world!
        w.beforeKeyedOpen = { if w.log.last == "openKeyed(R,cache:nil,existing)" { w.files = World.missingNone } }
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(w.screens, [.recovery(.keyRejected), .error(.databaseMissingForRecoveredKey)])
        assertNever(["created(", "replace(", "storeKey", "startMcp"])
    }

    /// The db-key changed between its rejection and the replace: nothing replaced, a note.
    func testADbKeyThatChangedSinceItWasRejectedIsNotReplaced() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.responses = [.restoredKey(Self.R)]
        let w = world!
        w.onPresent = { _ in w.dbKey = .found(Self.N) } // something else wrote db-key meanwhile
        _ = opened(await coordinator.run())
        XCTAssertTrue(w.log.contains("replace(R,rejected:K)"))
        XCTAssertEqual(w.dbKey, .found(Self.N))
        XCTAssertEqual(w.previous, [])
        guard case .recoveredKeyNotStored(.keyChangedMeanwhile, _)? = coordinator.notes.first else {
            return XCTFail("\(coordinator.notes)")
        }
    }

    /// The Keychain says done but the read-back does not show it: its own note.
    func testAnUnverifiedStoreIsItsOwnNote() async {
        world.useCore = true
        world.files = World.notPlain
        world.dbKey = .found(Self.K)
        world.fileKey = Self.R
        world.unverifiedWrite = true
        world.responses = [.restoredKey(Self.R)]
        _ = opened(await coordinator.run())
        guard case .recoveredKeyStoreUnverified? = coordinator.notes.first else { return XCTFail("\(coordinator.notes)") }
    }

    func testTheBackupCleanupRunsOffTheMainActor() async {
        world.dbKey = .found(Self.K)
        world.slowCleanup = true
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.cleanup(3), Self.mcp)]
        _ = opened(await coordinator.run())
        let w = world!
        let ticker = Task { @MainActor in w.ticked = true }
        await coordinator.firstWindowRendered()
        await ticker.value
        XCTAssertEqual(w.cleanupRanOnMain, false)
        XCTAssertTrue(w.tickedDuringCleanup, "the main actor ran other work while the cleanup was busy")
        XCTAssertEqual(w.goodLaunches, 0)
    }

    /// The real decision: an unfinished run whose database and copies are gone; resume
    /// fails, the user resets the status, and the next ask is NOT the same screen again.
    func testResetAfterAFailedResumeDoesNotShowTheSameScreenAgain() async {
        world.useCore = true
        world.files = World.missingInProgress
        world.resumeOutcome = .init(ok: false, phase: nil, error: .init(kind: "recovery", message: "gone", skip: nil))
        world.responses = [.resetEncryptionStatus]
        world.onboardingKey = nil // the new-install setup comes next; the test stops there
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.encryptedFileMissing(backupPresent: false, encryptedCopyPresent: false, lastAction: nil)])
        XCTAssertEqual(world.log, ask + ["resume"] + ask + ["present", "resetEncryptionStatus"] + ask + ["onboarding(existing:nil)"])
        XCTAssertEqual(thisLaunch(2)["resume"] as? String, "not_run")
    }

    /// The real decision through a migration: opt-in, precheck, run, keyed open, then the
    /// pending key (the same bytes) deleted, the flag cleared and the counter started.
    func testAMigrationWithTheRealDecision() async {
        world.useCore = true
        world.files = World.plainNone
        world.pendingKey = .found(Self.P)
        world.migrateAtNextLaunch = true
        world.log = []
        let o = opened(await coordinator.run())
        XCTAssertEqual(world.log, ask + ["precheck(recording:false)", "run(P,recording:false)",
                                         "migrationWrite(db-key-pending,P)", "migrationWrite(db-key,P)"]
                       + ask + ["openKeyed(P,cache:nil)", "deletePending", "setMigrateFlag(false)", "startMcp(P)"])
        XCTAssertEqual(o?.key, Self.P)
        await coordinator.firstWindowRendered()
        XCTAssertEqual(world.goodLaunches, 1)
    }

    func testAnUnreadableStateNoteOffersResetAndResetNeverOpensAgain() async {
        world.dbKey = .found(Self.K)
        world.plans = [Self.plan(Self.openPlain, Self.skipped("state_unreadable"), Self.mcp),
                       Self.plan(Self.precheck(), Self.run("db_key"))]
        _ = opened(await coordinator.run())
        XCTAssertEqual(coordinator.notes, [.migrationSkipped(.stateUnreadable, skip: nil)])
        world.log = []
        XCTAssertEqual(coordinator.resetEncryptionStatus(), .done)
        XCTAssertEqual(world.log, ["resetEncryptionStatus"] + ask, "the notes are asked again, nothing is run")
        XCTAssertEqual(coordinator.notes, [])
        assertNever(["precheck", "run(", "openPlain", "openKeyed", "startMcp"])
    }

    func testAFailedRunShowsTheFailedNote() async {
        world.dbKey = .found(Self.K)
        world.runOutcome = .init(ok: false, phase: nil, error: .init(kind: "verify", message: "m", skip: nil))
        world.plans = [Self.plan(Self.precheck(), Self.run("db_key")),
                       Self.plan(Self.openPlain, Self.migrationFailed, Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(thisLaunch(1)["migration"] as? String, "failed")
        XCTAssertEqual(coordinator.notes, [.migrationFailed])
    }

    func testFailedBeforeShowsTheNoteAndItsRetryResetsTheFailuresWithoutOpeningAgain() async {
        world.dbKey = .found(Self.K)
        world.plans = [Self.plan(Self.openPlain, Self.failedBefore(2), Self.mcp),
                       Self.plan(Self.precheck(), Self.run("db_key"))]
        _ = opened(await coordinator.run())
        XCTAssertEqual(coordinator.notes, [.failedBefore(attempts: 2)])
        world.log = []
        XCTAssertTrue(coordinator.retryAfterFailedBefore())
        XCTAssertEqual(world.log, ["resetFailures", "setMigrateFlag(true)"] + ask)
        XCTAssertEqual(coordinator.notes, [], "the migration runs at the next launch")
        assertNever(["precheck", "run(", "openPlain", "openKeyed", "startMcp"])
    }

    func testAnInterruptedMigrationShowsRetryAndRetryResumesAgain() async {
        world.plans = [Self.plan(Self.resume),
                       Self.plan(Self.interrupted),
                       Self.plan(Self.resume),
                       Self.plan(Self.openPlain, Self.mcp)]
        world.resumeOutcome = .init(ok: false, phase: nil, error: .init(kind: "recovery", message: "busy", skip: nil))
        world.responses = [.retry]
        let w = world!
        w.onPresent = { _ in w.resumeOutcome = .init(ok: true, phase: "copy_written", error: nil) }
        _ = opened(await coordinator.run())
        XCTAssertEqual(w.screens, [.migrationInterrupted])
        XCTAssertEqual(w.log, ask + ["resume"] + ask + ["present"] + ask + ["resume"] + ask + ["openPlain", "startMcp(nil)"])
        XCTAssertEqual(thisLaunch(1)["resume"] as? String, "failed")
        XCTAssertEqual(thisLaunch(2)["resume"] as? String, "not_run", "Retry runs resume again")
        XCTAssertEqual(thisLaunch(3)["resume"] as? String, "ran")
    }

    func testARecordingInProgressNeverStartsAMigration() async {
        world.dbKey = .found(Self.K)
        world.migrateAtNextLaunch = true
        world.recordingNow = true
        world.log = []
        // A plan that (wrongly or from a stale read) says no recording.
        world.plans = [Self.plan(Self.precheck(false), Self.run("db_key", false)),
                       Self.plan(Self.openPlain, Self.skipped("precheck"), Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.inputs[0]["recording"] as? Bool, true)
        XCTAssertEqual(world.log, ask + ["precheck(recording:true)"] + ask + ["openPlain", "startMcp(nil)"])
        XCTAssertEqual(thisLaunch(1)["migration"] as? String, "skipped")
        assertNever(["run("])
    }

    func testARecordingThatStartsAfterThePrecheckStillStopsTheRun() async {
        world.dbKey = .found(Self.K)
        let w = world!
        w.plans = [Self.plan(Self.precheck(), Self.run("db_key")), Self.plan(Self.openPlain, Self.skipped("precheck"), Self.mcp)]
        // Not recording when the inputs and the precheck are read; recording by the time the
        // run would start.
        let c = LaunchCoordinator(LaunchCoordinator<FakeStore>.Collaborators(
            observeFiles: { w.log.append("observe"); return "{}" },
            decide: { w.decide($0) }, keys: w, settings: w, physicalMemory: { 1 },
            recording: { w.log.contains { $0.hasPrefix("precheck") } },
            migration: w, openPlain: { w.log.append("openPlain"); return FakeStore(keyed: false) },
            openKeyed: { _, _, _ in (.opened, FakeStore(keyed: true)) },
            startMcpServer: { w.log.append("startMcp(\(Self.label($0)))") }, presenter: w, now: { 0 }))
        _ = opened(await c.run())
        XCTAssertTrue(w.log.contains("precheck(recording:false)"))
        XCTAssertFalse(w.log.contains { $0.hasPrefix("run(") }, "\(w.log)")
        XCTAssertEqual(w.inputs.count, 2)
        XCTAssertEqual((w.inputs[1]["this_launch"] as! [String: Any])["migration"] as? String, "skipped")
    }

    // MARK: - Encrypted file missing

    func testEncryptedFileMissingWithTheBackupRestoresOnConfirmThenOpensPlain() async {
        world.dbKey = .found(Self.K)
        world.plans = [Self.plan(Self.fileMissing(backup: true, copy: false)),
                       Self.plan(Self.openPlain, Self.encryptionAvailable, Self.mcp)]
        world.restoreResults = [CoreRestoreResult(action: .done, movedAside: 2, warning: .stateNotMovedAside)]
        world.responses = [.restoreRollbackCopy]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.screens, [.encryptedFileMissing(backupPresent: true, encryptedCopyPresent: false, lastAction: nil)])
        XCTAssertEqual(world.log, ask + ["present", "restoreRollbackCopy"] + ask + ["openPlain", "startMcp(nil)"])
        XCTAssertEqual(coordinator.notes, [.restoredCopy(movedAside: 2, warning: .stateNotMovedAside), .encryptionAvailable])
    }

    func testAPartlyDoneRestoreIsShownOnTheScreenAndCanBePressedAgain() async {
        world.plans = [Self.plan(Self.fileMissing(backup: true, copy: false)),
                       Self.plan(Self.fileMissing(backup: true, copy: false)),
                       Self.plan(Self.openPlain, Self.mcp)]
        let partly = CoreRestoreResult(action: .partlyDone, movedAside: 1, warning: .none)
        world.restoreResults = [partly, CoreRestoreResult(action: .done, movedAside: 1, warning: .none)]
        world.responses = [.restoreRollbackCopy, .restoreRollbackCopy]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.screens, [
            .encryptedFileMissing(backupPresent: true, encryptedCopyPresent: false, lastAction: nil),
            .encryptedFileMissing(backupPresent: true, encryptedCopyPresent: false, lastAction: .restore(partly)),
        ])
        XCTAssertEqual(world.log.filter { $0 == "restoreRollbackCopy" }.count, 2)
    }

    func testEncryptedFileMissingWithNoCopyOffersResetAndOnlyCopyOffersNothing() async {
        world.plans = [Self.plan(Self.fileMissing(backup: false, copy: false)), Self.plan(Self.onboarding(reuse: false))]
        world.responses = [.restoreRollbackCopy, .resetEncryptionStatus]
        world.onboardingKey = nil
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.log.filter { $0 == "restoreRollbackCopy" }, [], "not offered without a backup")
        XCTAssertEqual(world.log.filter { $0 == "resetEncryptionStatus" }.count, 1)

        world = World()
        world.plans = [Self.plan(Self.fileMissing(backup: false, copy: true))]
        world.responses = [.restoreRollbackCopy, .resetEncryptionStatus, .retry, .quit]
        guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
        XCTAssertEqual(world.screens.count, 4, "the support case offers no file action")
        assertNever(forbiddenOnAScreen + ["restoreRollbackCopy", "resetEncryptionStatus"])
    }

    // MARK: - Plain backup kept

    func testPlainBackupKeptOffersDeleteWhichPassesTheKeyedOpenAndNeverOpensAgain() async {
        world.dbKey = .found(Self.K)
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.backupKept, Self.mcp),
                       Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(coordinator.notes, [.plainBackupKept])
        world.log = []
        XCTAssertEqual(coordinator.deletePlainBackup(), .done)
        XCTAssertEqual(world.log, ["deletePlainBackup(opened:true)"] + ask)
        XCTAssertEqual(coordinator.notes, [])
        assertNever(["openKeyed", "openPlain", "startMcp"])
    }

    func testDeletePlainBackupAfterAPlainOpenSaysTheStoreWasNotOpenedWithItsKey() async {
        world.plans = [Self.plan(Self.openPlain, Self.mcp), Self.plan(Self.openPlain, Self.mcp)]
        _ = opened(await coordinator.run())
        _ = coordinator.deletePlainBackup()
        XCTAssertTrue(world.log.contains("deletePlainBackup(opened:false)"))
    }

    // MARK: - Guards

    func testTheReaskCapShowsAnErrorInsteadOfLooping() async {
        world.always = Self.plan(Self.resume)
        world.responses = [.quit]
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.log.filter { $0 == "decide" }.count, LaunchCoordinator<FakeStore>.maxDecisions)
        XCTAssertEqual(LaunchCoordinator<FakeStore>.maxDecisions, 8)
        XCTAssertEqual(world.screens, [.error(.tooManyDecisions)])
        assertNever(forbiddenOnAScreen)
    }

    func testAUserRetryStartsAFreshBudget() async {
        world.always = Self.plan(Self.keyUnavailable)
        world.dbKey = .unavailable("x")
        world.responses = Array(repeating: .retry, count: 12) + [.quit]
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.screens.count, 13)
        XCTAssertFalse(world.screens.contains(.error(.tooManyDecisions)), "presses of Retry are not a loop")
    }

    func testTheMcpServerStartsLastAndNeverBeforeAnOpen() async {
        world.dbKey = .found(Self.K)
        world.pendingKey = .found(Self.P)
        world.migrateAtNextLaunch = true
        world.log = []
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.deletePending, Self.clearFlag, Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.log.last, "startMcp(K)")

        world = World()
        world.plans = [Self.plan(Self.mcp, Self.openPlain)]
        guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.stepOutOfOrder(code: 22))])
        assertNever(["startMcp", "openPlain"])

        // Nothing that changes files or keys runs before an open either.
        for step in [Self.promote, Self.deletePending, Self.clearFlag, Self.startCounter, Self.encryptionAvailable] {
            world = World()
            world.plans = [Self.plan(step, Self.openPlain, Self.mcp)]
            guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
            assertNever(forbiddenOnAScreen)
        }
        // A migration step after an open is refused too, before anything of the plan runs.
        world = World()
        world.plans = [Self.plan(Self.openPlain, Self.precheck(), Self.run("db_key"))]
        guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
        assertNever(["openPlain", "precheck", "run(", "startMcp"])
        // So is an unknown step at the end of a plan: nothing before it runs either.
        world = World()
        world.pendingKey = .found(Self.P)
        world.plans = [Self.plan(Self.openKeyed("pending_key"), Self.promote, ["code": 77, "action": "later"])]
        guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.unknownStep(code: 77))])
        assertNever(["openKeyed", "promotePending"])
    }

    func testFastSearchPassesTheCacheToTheMainStoreOnly() async {
        world.dbKey = .found(Self.K)
        world.fastSearch = true
        world.plans = [Self.plan(Self.openKeyed("db_key", cache: 1_048_576), Self.mcp)]
        _ = opened(await coordinator.run())
        XCTAssertEqual(world.log.suffix(2), ["openKeyed(K,cache:1048576)", "startMcp(K)"],
                       "the MCP server gets the key and nothing else")
        XCTAssertEqual((world.inputs[0]["settings"] as? [String: Any])?["fast_search"] as? Bool, true)
    }

    func testUnknownStepsAMissingDecisionAndAFailedOpenStopWithAnError() async {
        world.plans = [#"{"steps":[{"code":99,"action":"from_the_future"}]}"#]
        guard case .quit = await coordinator.run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.unknownStep(code: 99))])

        world = World()
        world.plans = []
        guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.decisionUnavailable)])

        world = World()
        world.plainOpens = false
        world.plans = [Self.plan(Self.openPlain, Self.mcp)]
        guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.plainOpenFailed)])
        assertNever(["startMcp"])

        world = World()
        world.dbKey = .found(Self.K)
        world.keyedOutcomes = [.damaged("x")]
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.keyedOpenFailed(.damaged("x")))])
        assertNever(["startMcp"])
    }

    /// A failed PLAIN open (Task 11b-2 fix round 1): Try again opens again; "Continue
    /// without the memory database" returns an outcome with no store, a persistent note,
    /// no MCP server and no good-launch bookkeeping.
    func testAFailedPlainOpenCanContinueWithoutTheDatabaseAndNeverStartsTheMcpServer() async {
        world.plainOpens = false
        world.plans = [Self.plan(Self.openPlain, Self.mcp), Self.plan(Self.openPlain, Self.mcp)]
        world.responses = [.retry, .continueWithoutStore]
        let c = makeCoordinator(world)
        guard case .openedWithoutStore = await c.run() else { return XCTFail("\(world.log)") }
        XCTAssertEqual(world.screens, [.error(.plainOpenFailed), .error(.plainOpenFailed)])
        XCTAssertEqual(world.log.filter { $0 == "openPlain" }.count, 2, "Try again opened again")
        XCTAssertEqual(c.notes, [.memoryDatabaseUnavailable])
        await c.firstWindowRendered()
        assertNever(["startMcp", "setGoodLaunches", "cleanupPlainBackup", "openKeyed"])
        XCTAssertTrue(LaunchNote.memoryDatabaseUnavailable.isLaunchOnly, "kept when the notes are asked again")
    }

    /// Every other stop stays blocking: the answer is not offered, so the screen shows again.
    func testAKeyedFailureNeverContinuesWithoutTheDatabase() async {
        world.dbKey = .found(Self.K)
        world.keyedOutcomes = [.damaged("x")]
        world.plans = [Self.plan(Self.openKeyed("db_key"), Self.mcp)]
        world.responses = [.continueWithoutStore]
        guard case .quit = await makeCoordinator(world).run() else { return XCTFail() }
        XCTAssertEqual(world.screens, [.error(.keyedOpenFailed(.damaged("x"))), .error(.keyedOpenFailed(.damaged("x")))])
        assertNever(["startMcp"])
    }

    /// No screen outcome ever reaches the store opener, the MCP server or a key write.
    func testTheStoreOpenerIsNeverCalledOnAScreenOutcome() async {
        let screens: [Step] = [Self.hardStop, Self.keyUnavailable, Self.recovery("needs_key"), Self.recovery("key_rejected"),
                               Self.interrupted, Self.damaged("damaged"), Self.damaged("no_access"), Self.damaged("open_elsewhere"),
                               Self.fileMissing(backup: true, copy: false), Self.fileMissing(backup: false, copy: false),
                               Self.fileMissing(backup: false, copy: true)]
        for screen in screens {
            world = World()
            world.plans = [Self.plan(screen)]
            guard case .quit = await makeCoordinator(world).run() else { return XCTFail("\(screen)") }
            XCTAssertEqual(world.screens.count, 1, "\(screen)")
            assertNever(forbiddenOnAScreen)
        }
    }

    // MARK: - Settings

    func testTheShellKeepsTheCounterAndFlagInItsOwnSettings() {
        let suite = "launch-coordinator-tests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let s = UserDefaultsLaunchSettings(defaults: defaults)
        XCTAssertFalse(s.fastSearch)
        XCTAssertFalse(s.migrateAtNextLaunch)
        XCTAssertEqual(s.goodLaunches, 0)
        defaults.set(true, forKey: "storage.fastSearch")
        s.migrateAtNextLaunch = true
        s.goodLaunches = 3
        let again = UserDefaultsLaunchSettings(defaults: defaults)
        XCTAssertTrue(again.fastSearch, "core's pinned key (api::settings::FAST_SEARCH_KEY)")
        XCTAssertTrue(again.migrateAtNextLaunch)
        XCTAssertEqual(again.goodLaunches, 3)
        XCTAssertEqual(UserDefaultsLaunchSettings.fastSearchKey, "storage.fastSearch")
    }
}
