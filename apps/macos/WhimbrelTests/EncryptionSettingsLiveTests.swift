import Combine
import XCTest
@testable import Whimbrel

/// The Settings encryption card's live closures (X4 Task 11b-2): the pre-check, the key the
/// setup wraps, the encrypt step that only prepares the next launch, the launch notes and
/// their buttons, Faster search and the local speed check. Temp files, a throwaway Keychain
/// service and a `UserDefaults` suite only.
@MainActor
final class EncryptionSettingsLiveTests: XCTestCase {
    private var dir: URL!
    private var db: URL { dir.appendingPathComponent("core-memory.sqlite") }
    private var keys: DbKeyStore!
    private var suiteName: String!
    private var defaults: UserDefaults!
    private var settings: UserDefaultsLaunchSettings!

    override func setUp() {
        super.setUp()
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("enc-settings-\(UUID().uuidString)", isDirectory: true)
        try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        keys = DbKeyStore(service: "enc-settings-tests.\(UUID().uuidString)")
        suiteName = "enc-settings-tests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)!
        settings = UserDefaultsLaunchSettings(defaults: defaults)
    }

    override func tearDown() {
        try? keys.delete()
        defaults.removePersistentDomain(forName: suiteName)
        try? FileManager.default.removeItem(at: dir)
        super.tearDown()
    }

    /// The setup collaborators record which key they were asked to wrap.
    private var wrapped: [Data?] = []

    private func actions(keys: LaunchKeyStore? = nil, recording: Bool = false,
                         precheck: ((URL, Bool) async -> CoreEncryptionMigration.Outcome?)? = nil) -> EncryptionSettingsModel.Actions {
        EncryptionSettingsLive.actions(
            keys: keys ?? self.keys, settings: settings, dbPath: db, recording: { recording },
            precheck: precheck ?? { CoreEncryptionMigration.precheck(dbPath: $0, recording: $1) },
            setup: { [unowned self] existing in
                wrapped.append(existing)
                return .init(minimumLength: 12, passphraseLength: { $0.count },
                             begin: { _ in .failure(.invalidArgument) }, verify: { _, _ in false },
                             saveFile: { _ in .cancelled }, printWords: { _ in .cancelled })
            },
            exportEverything: {}, restart: {})
    }

    private func outcome(skip reason: String) -> CoreEncryptionMigration.Outcome {
        .init(ok: false, phase: nil, error: .init(kind: "skipped", message: "x",
                                                  skip: .init(reason: reason, needBytes: nil, freeBytes: nil, phase: nil, attempts: nil, last: nil)))
    }

    // MARK: Pre-check

    /// With the app's own store open, core's lock test says "open elsewhere" every time: that
    /// is this app, and the launch's own pre-check runs before the store opens. Any other
    /// reason stops the setup before a key is made; the open store keeps working.
    func testThePrecheckIgnoresOnlyTheAppsOwnOpenStore() async throws {
        let store = try XCTUnwrap(CoreMemoryStore(path: db, modelsDir: nil))
        _ = store.insertMeeting(CoreNewMeeting(deviceId: "d", startUnixMs: 1, endUnixMs: nil, title: "Open", attendees: [],
                                               trigger: .manual, audioPath: nil, status: .completed))
        let raw = CoreEncryptionMigration.precheck(dbPath: db, recording: false)
        XCTAssertEqual(raw?.error?.skip?.reason, "open_in_this_process", "core names the app's own handle")
        let ok = await actions().precheck()
        XCTAssertNil(ok, "the app's own open store does not stop the setup")
        XCTAssertNotNil(store.insertMeeting(CoreNewMeeting(deviceId: "d", startUnixMs: 2, endUnixMs: nil, title: "Still works",
                                                           attendees: [], trigger: .manual, audioPath: nil, status: .completed)))
        let recording = await actions(recording: true).precheck()
        XCTAssertEqual(recording, EncryptionCopy.skipRecording)
        let lowDisk = await actions(precheck: { _, _ in self.outcome(skip: "low_disk") }).precheck()
        XCTAssertEqual(lowDisk, EncryptionCopy.skipLowDisk)
    }

    /// Another program holding the database (a real second process) is a real refusal.
    func testAnotherProgramHoldingTheDatabaseStopsTheSetup() async throws {
        do { _ = try XCTUnwrap(CoreMemoryStore(path: db, modelsDir: nil)) } // made, then closed
        let holder = Process()
        holder.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
        holder.arguments = [db.path]
        let input = Pipe()
        let output = Pipe()
        holder.standardInput = input
        holder.standardOutput = output
        try holder.run()
        defer {
            try? input.fileHandleForWriting.close()
            holder.terminate()
            holder.waitUntilExit()
        }
        // A read leaves the other process's connection holding the WAL index.
        try input.fileHandleForWriting.write(contentsOf: Data("SELECT 'held', count(*) FROM sqlite_master;\n".utf8))
        _ = output.fileHandleForReading.availableData
        let raw = CoreEncryptionMigration.precheck(dbPath: db, recording: false)
        XCTAssertEqual(raw?.error?.skip?.reason, "open_elsewhere", "\(String(describing: raw))")
        let message = await actions().precheck()
        XCTAssertEqual(message, EncryptionCopy.skipOpenElsewhere)
    }

    func testThePrecheckPassesOnlyForTheAppsOwnHandle() async {
        let own = await actions(precheck: { _, _ in self.outcome(skip: "open_in_this_process") }).precheck()
        XCTAssertNil(own)
        let other = await actions(precheck: { _, _ in self.outcome(skip: "open_elsewhere") }).precheck()
        XCTAssertEqual(other, EncryptionCopy.skipOpenElsewhere)
    }

    func testThePrecheckPassesTheRecordingFlagToCore() async {
        var seen: [Bool] = []
        _ = await actions(recording: true, precheck: { _, r in seen.append(r); return .init(ok: true, phase: nil, error: nil) }).precheck()
        XCTAssertEqual(seen, [true])
    }

    func testAnUnreadableKeychainStopsTheSetupBeforeAnyKeyIsMade() async {
        final class Locked: LaunchKeyStore {
            func lookup() -> DbKeyLookup { .unavailable("locked") }
            func lookupPending() -> DbKeyLookup { .absent }
            func storePending(_ key: Data) throws { XCTFail("no write") }
            func storeKey(_ key: Data) throws { XCTFail("no write") }
            func replaceAfterRecovery(proven key: Data, replacing rejected: Data?) throws {}
            func promotePendingKey(openedWith opened: Data, replacing rejected: Data?) throws {}
            func deletePending() throws {}
            func migrationWrite(_ key: Data, account: String) -> OSStatus { 0 }
        }
        let message = await actions(keys: Locked(), precheck: { _, _ in .init(ok: true, phase: nil, error: nil) }).precheck()
        XCTAssertEqual(message, EncryptionCopy.precheckKeychainUnavailable)
        let encrypt = await actions(keys: Locked()).encrypt(Data(repeating: 1, count: 32)) { _ in }
        XCTAssertEqual(encrypt, .failed(EncryptionCopy.encryptFailed))
        XCTAssertFalse(settings.migrateAtNextLaunch)
    }

    // MARK: The key the setup wraps

    func testTheSetupWrapsTheExistingKeyAndNeverAFreshOneBesideIt() throws {
        let a = actions()
        _ = a.setup(a.pendingKey())
        XCTAssertEqual(wrapped.last ?? Data(), nil, "no key yet: a fresh one")
        let pending = try keys.createPending()
        XCTAssertEqual(a.pendingKey(), pending)
        _ = a.setup(a.pendingKey())
        XCTAssertEqual(wrapped.last, pending, "a pending key is reused: same words")
        let dbKey = Data(repeating: 3, count: 32)
        try keys.store(dbKey, account: DbKeyStore.keyAccount)
        _ = a.setup(a.pendingKey())
        XCTAssertEqual(wrapped.last, dbKey, "db-key wins: the next launch migrates with it")
    }

    // MARK: Encrypt only prepares the next launch

    func testEncryptStoresThePendingKeySetsTheFlagAndNeverTouchesTheOpenStore() async throws {
        let store = try XCTUnwrap(CoreMemoryStore(path: db, modelsDir: nil))
        let before = try Data(contentsOf: db)
        let key = Data(repeating: 8, count: 32)
        let r = await actions().encrypt(key) { _ in }
        XCTAssertEqual(r, .finishesAtNextLaunch)
        XCTAssertEqual(keys.lookupPending(), .found(key))
        XCTAssertEqual(keys.lookup(), .absent, "db-key is the migration's to write")
        XCTAssertTrue(settings.migrateAtNextLaunch)
        XCTAssertEqual(try Data(contentsOf: db).prefix(16), before.prefix(16), "still plain: nothing migrated now")
        XCTAssertFalse(FileManager.default.fileExists(atPath: db.path + ".plain.bak"))
        XCTAssertFalse(FileManager.default.fileExists(atPath: db.path + ".enc.tmp"))
        withExtendedLifetime(store) {}
        // Again with the same key (a retry): fine, nothing replaced.
        let again = await actions().encrypt(key) { _ in }
        XCTAssertEqual(again, .finishesAtNextLaunch)
    }

    func testEncryptNeverReplacesAKeyWithAnotherOne() async throws {
        let pending = try keys.createPending()
        let other = Data(repeating: 2, count: 32)
        let r = await actions().encrypt(other) { _ in }
        XCTAssertEqual(r, .failed(EncryptionCopy.encryptFailed))
        XCTAssertEqual(keys.lookupPending(), .found(pending))
        XCTAssertFalse(settings.migrateAtNextLaunch)
        // With db-key: encrypting with it writes no pending key beside it.
        try keys.deletePending()
        let dbKey = Data(repeating: 3, count: 32)
        try keys.store(dbKey, account: DbKeyStore.keyAccount)
        let ok = await actions().encrypt(dbKey) { _ in }
        XCTAssertEqual(ok, .finishesAtNextLaunch)
        XCTAssertEqual(keys.lookupPending(), .absent)
        XCTAssertTrue(settings.migrateAtNextLaunch)
    }

    // MARK: The pane after a restart is pending

    func testAPendingRunFromAnEarlierSessionShowsRestartOnly() {
        let m = EncryptionSettingsModel(outcome: nil, actions: actions(), pendingNextLaunch: true)
        XCTAssertTrue(m.pendingNextLaunch)
        XCTAssertEqual(m.phase, .finished(EncryptionCopy.encryptAtNextLaunch))
        XCTAssertFalse(m.canStartEncrypt)
        XCTAssertFalse(EncryptionSettingsModel(outcome: nil, actions: actions()).pendingNextLaunch)
    }

    /// "Finishes at the next launch" only when core will retry then: not after too many
    /// failures (Retry first) or with an unreadable state file (Reset first), and never
    /// once the store opened encrypted.
    func testPendingNextLaunchOnlyWhenTheNextLaunchWillTry() {
        XCTAssertTrue(EncryptionSettingsLive.pendingNextLaunch(flag: true, openedWithKey: false, notes: []))
        XCTAssertTrue(EncryptionSettingsLive.pendingNextLaunch(flag: true, openedWithKey: false, notes: [.migrationFailed]))
        XCTAssertFalse(EncryptionSettingsLive.pendingNextLaunch(flag: false, openedWithKey: false, notes: []))
        XCTAssertFalse(EncryptionSettingsLive.pendingNextLaunch(flag: true, openedWithKey: true, notes: []))
        XCTAssertFalse(EncryptionSettingsLive.pendingNextLaunch(flag: true, openedWithKey: false, notes: [.failedBefore(attempts: 3)]))
        XCTAssertFalse(EncryptionSettingsLive.pendingNextLaunch(flag: true, openedWithKey: false,
                                                               notes: [.migrationSkipped(.stateUnreadable, skip: nil)]))
    }

    /// The automatic-deletion notice ("until it has started normally three times") is wrong
    /// while the "copy kept" note shows (that cleanup never runs), and goes once the copy is.
    func testThePlainBackupNoticeFollowsTheKeptNoteAndTheFile() {
        var backup = true
        var a = actions()
        a.plainBackupExists = { backup }
        let m = EncryptionSettingsModel(outcome: .opened, actions: a)
        XCTAssertTrue(m.showsPlainBackupNotice)
        m.plainBackupKeptByNote = true
        XCTAssertFalse(m.showsPlainBackupNotice)
        m.plainBackupKeptByNote = false
        backup = false
        m.refreshPlainBackup()
        XCTAssertFalse(m.showsPlainBackupNotice)
    }

    func testTheBadgeFollowsWhatTheLaunchOpened() {
        XCTAssertEqual(EncryptionSettingsLive.outcome(openedWithKey: true), .opened)
        XCTAssertNil(EncryptionSettingsLive.outcome(openedWithKey: false))
    }

    // MARK: Notes

    private static let allNotes: [LaunchNote] = [
        .keyStoreUnavailable(LaunchDiagnostic(diagnosticOnly: "SECRET")),
        .failedBefore(attempts: 3),
        .migrationSkipped(.precheck, skip: .init(reason: "recording", needBytes: nil, freeBytes: nil, phase: nil, attempts: nil, last: "SECRET")),
        .migrationSkipped(.precheck, skip: .init(reason: "low_disk", needBytes: 1, freeBytes: 0, phase: nil, attempts: nil, last: nil)),
        .migrationSkipped(.precheck, skip: .init(reason: "open_elsewhere", needBytes: nil, freeBytes: nil, phase: nil, attempts: nil, last: nil)),
        .migrationSkipped(.precheck, skip: .init(reason: "open_in_this_process", needBytes: nil, freeBytes: nil, phase: nil, attempts: nil, last: nil)),
        .migrationSkipped(.precheck, skip: nil),
        .migrationSkipped(.stateUnreadable, skip: nil),
        .migrationFailed, .encryptionAvailable, .plainBackupKept,
        .restoredCopy(movedAside: 2, warning: .stateNotMovedAside),
        .restoredCopy(movedAside: 0, warning: .none),
        .pendingKeyNotPromoted(.keychainUnavailable, LaunchDiagnostic(diagnosticOnly: "SECRET")),
        .pendingKeyNotDeleted(.pendingDiffers, LaunchDiagnostic(diagnosticOnly: "SECRET")),
        .recoveredKeyNotStored(.noPreviousSlot, LaunchDiagnostic(diagnosticOnly: "SECRET")),
        .recoveredKeyNotStored(.keychainUnavailable, LaunchDiagnostic(diagnosticOnly: "SECRET")),
        .recoveredKeyStoreUnverified(LaunchDiagnostic(diagnosticOnly: "SECRET")),
    ]

    func testEveryNoteHasCopyAndOnlyItsOwnButtonAndNeverRawText() {
        for note in Self.allNotes {
            let row = EncryptionNoteRow(note)
            XCTAssertFalse(row.text.isEmpty, "\(note)")
            XCTAssertFalse(row.text.joined().contains("SECRET"), "\(note)")
            switch note {
            case .failedBefore: XCTAssertEqual(row.action, .retryAfterFailedBefore)
            case .migrationSkipped(.stateUnreadable, _): XCTAssertEqual(row.action, .resetEncryptionStatus)
            case .plainBackupKept: XCTAssertEqual(row.action, .deletePlainBackup)
            default: XCTAssertNil(row.action, "\(note)")
            }
        }
        XCTAssertEqual(EncryptionNoteRow(.failedBefore(attempts: 3)).text.first, EncryptionCopy.encryptFailed)
        // At launch nothing is open yet, so the app's own handle cannot be the reason: the
        // generic note, never "another program".
        let own = EncryptionNoteRow(.migrationSkipped(.precheck, skip: .init(reason: "open_in_this_process", needBytes: nil,
                                                                             freeBytes: nil, phase: nil, attempts: nil, last: nil)))
        XCTAssertEqual(own.text, [EncryptionCopy.noteSkippedOther])
        XCTAssertTrue(EncryptionNoteRow(.plainBackupKept).text.contains(EncryptionCopy.deleteNotGuaranteed))
    }

    func testNoteButtonsCallTheCoordinatorAndWaitForTheCleanup() {
        let notes = CurrentValueSubject<[LaunchNote], Never>([.plainBackupKept, .failedBefore(attempts: 3)])
        let cleanup = CurrentValueSubject<Bool, Never>(true)
        var calls: [String] = []
        let model = EncryptionNotesModel(notes: notes.eraseToAnyPublisher(), cleanupRunning: cleanup.eraseToAnyPublisher(),
                                         actions: .init(deletePlainBackup: { calls.append("delete"); return .failed },
                                                        resetEncryptionStatus: { calls.append("reset"); return .done },
                                                        retryAfterFailedBefore: { calls.append("retry"); return true }))
        XCTAssertEqual(model.rows.count, 2)
        XCTAssertTrue(model.cleanupRunning)
        model.perform(.deletePlainBackup)
        XCTAssertEqual(calls, [], "buttons do nothing while the cleanup runs")
        cleanup.send(false)
        model.perform(.deletePlainBackup)
        model.perform(.retryAfterFailedBefore)
        XCTAssertEqual(calls, ["delete", "retry"])
        XCTAssertEqual(model.feedback[.deletePlainBackup], EncryptionCopy.deletePlainBackupFailed)
        XCTAssertEqual(model.feedback[.retryAfterFailedBefore], EncryptionCopy.noteRetryScheduled)
        notes.send([])
        XCTAssertTrue(model.rows.isEmpty)
    }

    // MARK: Faster search

    func testFasterSearchIsOfferedOnlyWithEnoughMemoryAndStaysOnThisMac() {
        XCTAssertNil(FastSearchSetting(defaults: defaults, physicalMemory: 8 << 30), "8 GB: not offered")
        let s = FastSearchSetting(defaults: defaults, physicalMemory: 32 << 30)
        XCTAssertNotNil(s)
        XCTAssertEqual(s?.enabled, false, "off by default")
        s?.enabled = true
        XCTAssertTrue(defaults.bool(forKey: UserDefaultsLaunchSettings.fastSearchKey))
        XCTAssertTrue(settings.fastSearch, "the launch reads the same key")
        XCTAssertEqual(UserDefaultsLaunchSettings.fastSearchKey, "storage.fastSearch")
    }

    // MARK: The speed check

    static let reportJSON = """
    {"ok":true,"report":{"open_ms":42,"keyword_queries":50,"keyword_p50_ms":3.25,"keyword_p95_ms":9.5,"semantic_queries":50,\
    "semantic_p50_ms":40.0,"semantic_p95_ms":81.25,"semantic_unavailable":false,"too_little_data":false,"page_size":16384,\
    "encrypted":true,"cache_kib":1048576,"cpu_arch":"aarch64","os":"macos","ram_gib":32,"db_size_gib":1.5}}
    """

    func testTheSpeedCheckShowsTimingsAndSavesTheTextLocally() async {
        var saved: [String] = []
        var progressSeen = false
        let model = SpeedCheckModel(actions: .init(
            run: { progress in progress(0.5); progressSeen = true; return Self.reportJSON },
            text: { report in "TEXT for " + report },
            save: { text in saved.append(text); return URL(fileURLWithPath: "/tmp/Whimbrel encryption speed.txt") }))
        await model.check()
        XCTAssertTrue(progressSeen)
        guard case let .done(lines) = model.phase else { return XCTFail("\(model.phase)") }
        XCTAssertEqual(lines, [
            EncryptionCopy.speedEncrypted, EncryptionCopy.speedOpen("42"),
            EncryptionCopy.speedKeyword("3.2", "9.5"), EncryptionCopy.speedSemantic("40.0", "81.2"),
            EncryptionCopy.speedComputer("aarch64", "32", "macos"),
        ])
        model.saveReport()
        XCTAssertEqual(saved.count, 1)
        XCTAssertTrue(saved[0].hasPrefix("TEXT for {"), "the report object goes to core's text")
        XCTAssertFalse(saved[0].contains("\"ok\""))
        XCTAssertEqual(model.savedPath, "/tmp/Whimbrel encryption speed.txt")
    }

    func testAFailedOrTooSmallSpeedCheck() async {
        let failed = SpeedCheckModel(actions: .init(run: { _ in "{\"ok\":false,\"error\":\"a search failed\"}" },
                                                    text: { _ in nil }, save: { _ in nil }))
        await failed.check()
        XCTAssertEqual(failed.phase, .failed)
        let small = SpeedCheckModel(actions: .init(run: { _ in
            "{\"ok\":true,\"report\":{\"open_ms\":null,\"keyword_queries\":0,\"semantic_queries\":0,\"semantic_unavailable\":true,\"too_little_data\":true,\"page_size\":4096,\"encrypted\":false,\"cache_kib\":-2000,\"cpu_arch\":\"aarch64\",\"os\":\"macos\",\"ram_gib\":null,\"db_size_gib\":0.0}}"
        }, text: { _ in nil }, save: { _ in nil }))
        await small.check()
        guard case let .done(lines) = small.phase else { return XCTFail("\(small.phase)") }
        XCTAssertEqual(lines, [EncryptionCopy.speedNotEncrypted, EncryptionCopy.speedOpen(EncryptionCopy.speedNotMeasured),
                               EncryptionCopy.speedNotEnoughData,
                               EncryptionCopy.speedComputer("aarch64", EncryptionCopy.speedNotMeasured, "macos")])
    }

    /// The real C ABI on a temp store: the report holds no meeting text and no path.
    static let seededWords = ["zanzibarquokka", "plumbvortex", "quillmarrow", "brindlecask", "fenwickhollow", "tarnlantern",
                              "gorsemeadow", "vellumspire", "copperwren", "saltmarrow", "thistlebeck", "emberquay",
                              "lichenvault", "harrowfinch", "mossgallery", "pewterlark"]

    func testTheRealSpeedCheckHoldsNoContentOrPaths() async throws {
        let store = try XCTUnwrap(CoreMemoryStore(path: db, modelsDir: nil))
        // Captures with distinct words, so core draws real searches (not "too little data").
        let jsonl = dir.appendingPathComponent("seed.jsonl")
        let records = Self.seededWords.enumerated().map { i, word in
            String(decoding: try! JSONSerialization.data(withJSONObject: [
                "timestamp": String(format: "2026-09-10T12:%02d:00Z", i), "bundle_id": "com.example.notes",
                "window_title": "\(word) notes", "text": "\(word) \(Self.seededWords[(i + 1) % Self.seededWords.count]) budget",
                "source": "ax",
            ]), as: UTF8.self)
        }
        try records.joined(separator: "\n").write(to: jsonl, atomically: true, encoding: .utf8)
        XCTAssertEqual(store.ingestJSONL(path: jsonl, deviceId: "d")?.itemsInserted, Self.seededWords.count)
        var saved: [String] = []
        let model = SpeedCheckModel(actions: EncryptionSettingsLive.speedActions(
            store: { store }, openMs: { 7 }, physicalMemory: 16 << 30, save: { saved.append($0); return nil }))
        await model.check()
        guard case let .done(lines) = model.phase else { return XCTFail("\(model.phase)") }
        XCTAssertFalse(lines.contains(EncryptionCopy.speedNotEnoughData), "searches ran: \(lines)")
        model.saveReport()
        let text = try XCTUnwrap(saved.first)
        XCTAssertTrue(text.contains("Whimbrel encryption speed check"), text)
        XCTAssertTrue(text.contains("Keyword search ("), "searches were timed: \(text)")
        for shown in [text, lines.joined(separator: "\n")] {
            let lower = shown.lowercased()
            for word in Self.seededWords + ["budget", "notes", "com.example"] {
                XCTAssertFalse(lower.contains(word), "\(word) in \(shown)")
            }
            XCTAssertFalse(shown.contains(dir.path))
        }
    }

    // MARK: Fix round 1: speed check guard and cancel, two keys, copy

    func testTheSpeedCheckDoesNotStartWhileRecording() async {
        var runs = 0
        let model = SpeedCheckModel(actions: .init(run: { _ in runs += 1; return Self.reportJSON }, text: { $0 }, save: { _ in nil },
                                                   isRecording: { true }))
        await model.check()
        XCTAssertEqual(runs, 0)
        XCTAssertEqual(model.phase, .blockedByRecording)
    }

    func testCancelReturnsAtOnceAndDropsTheResult() async {
        var finish: CheckedContinuation<String?, Never>?
        let model = SpeedCheckModel(actions: .init(run: { _ in await withCheckedContinuation { finish = $0 } },
                                                   text: { $0 }, save: { _ in nil }))
        let check = Task { await model.check() }
        for _ in 0..<2000 where finish == nil { try? await Task.sleep(nanoseconds: 1_000_000) }
        XCTAssertEqual(model.phase, .running(0))
        model.cancel()
        XCTAssertEqual(model.phase, .idle)
        finish?.resume(returning: Self.reportJSON)
        await check.value
        XCTAssertEqual(model.phase, .idle, "a cancelled check's numbers are not shown")
        model.saveReport()
        XCTAssertNil(model.savedPath)
    }

    /// After Cancel the button stays off until the dropped run returns, so no second run
    /// queues behind its leftover queries on the serial access queue.
    func testCancelKeepsTheCheckOffUntilTheDroppedRunReturns() async {
        var runs = 0
        var finish: CheckedContinuation<String?, Never>?
        let model = SpeedCheckModel(actions: .init(run: { _ in
            runs += 1
            return await withCheckedContinuation { finish = $0 }
        }, text: { $0 }, save: { _ in nil }))
        let first = Task { await model.check() }
        for _ in 0..<2000 where finish == nil { try? await Task.sleep(nanoseconds: 1_000_000) }
        model.cancel()
        XCTAssertTrue(model.draining)
        XCTAssertFalse(model.canCheck)
        await model.check()
        XCTAssertEqual(runs, 1, "no second run while the first still holds the queue")
        let f = finish
        finish = nil
        f?.resume(returning: nil)
        await first.value
        XCTAssertFalse(model.draining)
        XCTAssertTrue(model.canCheck)
        let second = Task { await model.check() }
        for _ in 0..<2000 where finish == nil { try? await Task.sleep(nanoseconds: 1_000_000) }
        XCTAssertEqual(runs, 2)
        finish?.resume(returning: Self.reportJSON)
        await second.value
    }

    func testThePrivacyLineSaysTheKindOfMac() {
        XCTAssertTrue(EncryptionCopy.speedCheckPrivacy.contains("timings and the kind of Mac only, no content"),
                      EncryptionCopy.speedCheckPrivacy)
    }

    /// db-key and a different pending key: core would migrate with db-key while the pending
    /// key is left beside it, and the run's key store refuses every launch. Refused up front.
    func testTwoDifferentKeysAreRefusedBeforeAnySetup() async throws {
        let dbKey = Data(repeating: 3, count: 32)
        try keys.store(dbKey, account: DbKeyStore.keyAccount)
        _ = try keys.createPending()
        let message = await actions(precheck: { _, _ in .init(ok: true, phase: nil, error: nil) }).precheck()
        XCTAssertEqual(message, EncryptionCopy.twoKeysFound)
        let r = await actions().encrypt(dbKey) { _ in }
        XCTAssertEqual(r, .failed(EncryptionCopy.twoKeysFound))
        XCTAssertFalse(settings.migrateAtNextLaunch, "no flag that would fail every launch")
    }

    func testAStoreThatDidNotOpenShowsInThePane() {
        let m = EncryptionSettingsModel(outcome: nil, actions: actions(), attention: EncryptionCopy.noteStoreUnavailable)
        XCTAssertEqual(m.state, .needsAttention(EncryptionCopy.noteStoreUnavailable))
        XCTAssertFalse(m.canStartEncrypt)
        // The pane says "could not be opened" once; the note adds only what to do.
        let row = EncryptionNoteRow(.memoryDatabaseUnavailable)
        XCTAssertEqual(row.text, [EncryptionCopy.noteStoreUnavailableHint])
        XCTAssertTrue(EncryptionCopy.noteStoreUnavailableHint.hasPrefix("Whimbrel is running without the memory database"))
        XCTAssertNil(row.action)
        // No store: no speed check and no Faster search on the card.
        var live = EncryptionSettingsLive(outcome: nil, attention: EncryptionCopy.noteStoreUnavailable, pendingNextLaunch: false,
                                          actions: actions(),
                                          notes: EncryptionNotesModel(notes: Just([]).eraseToAnyPublisher(),
                                                                      cleanupRunning: Just(false).eraseToAnyPublisher(),
                                                                      actions: .init(deletePlainBackup: { .done },
                                                                                     resetEncryptionStatus: { .done },
                                                                                     retryAfterFailedBefore: { true })),
                                          fastSearch: FastSearchSetting(defaults: defaults, physicalMemory: 32 << 30),
                                          speedCheck: SpeedCheckModel(actions: .init(run: { _ in nil }, text: { _ in nil }, save: { _ in nil })))
        XCTAssertFalse(live.showsStoreTools)
        live.attention = nil
        XCTAssertTrue(live.showsStoreTools)
    }

    func testTheSettingsCardMakesNoNetworkCalls() throws {
        let app = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Whimbrel")
        for file in ["UI/Settings/EncryptionSettingsLive.swift", "UI/Settings/EncryptionSettingsPane.swift", "UI/LaunchScreens.swift"] {
            let text = try String(contentsOf: app.appendingPathComponent(file), encoding: .utf8)
            for banned in ["URLSession", "URLRequest", "NWConnection", "http://", "https://", "NSPasteboard"] {
                XCTAssertFalse(text.contains(banned), "\(banned) in \(file)")
            }
        }
    }
}
