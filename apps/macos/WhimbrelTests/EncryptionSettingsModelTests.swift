import XCTest
@testable import Whimbrel

/// Settings, Capture & Privacy, "Encrypt the database" (X4 Task 10): state from the open
/// outcome, Export everything offered first, the pre-checks before any recovery key is made,
/// the setup key handed to the encrypt step, and a way back from a failure.
@MainActor
final class EncryptionSettingsModelTests: XCTestCase {
    final class Log: @unchecked Sendable {
        var exports = 0
        var prechecks = 0
        var begun = 0
        var encrypted: [Data] = []
        var backup = false
        /// What the encrypt step stored as `db-key-pending` (Task 11 does this on Continue).
        var pending: Data?
        var setupKeys: [Data?] = []
        var restarts = 0
        var fresh: UInt8 = 0
    }

    static let key = Data(repeating: 5, count: 32)

    /// Like core: a pending key is reused (same words); otherwise each setup makes a new key.
    private func setupActions(_ log: Log, pending: Data?) -> EncryptionSetupModel.Actions {
        log.setupKeys.append(pending)
        return .init(minimumLength: 12, passphraseLength: { $0.count },
              begin: { _ in
                  log.begun += 1
                  let key: Data
                  if let pending { key = pending } else { log.fresh += 1; key = Data(repeating: log.fresh, count: 32) }
                  return .success(CoreSetupMaterial(key: key, recoveryFile: "FILE", words: (1...24).map { "w\(key[0])-\($0)" }))
              },
              verify: { _, p in p == "correct horse battery" },
              saveFile: { _ in .saved(path: "/Volumes/USB/x", sameDiskAsDatabase: false) }, printWords: { _ in .printed },
              reusesKey: pending != nil)
    }

    private func model(_ log: Log, outcome: CoreOpenOutcome? = .needsMigration,
                       precheck: String? = nil,
                       result: EncryptionSettingsModel.EncryptResult = .encrypted) -> EncryptionSettingsModel {
        EncryptionSettingsModel(outcome: outcome, actions: .init(
            exportEverything: { log.exports += 1 },
            precheck: { log.prechecks += 1; return precheck },
            plainBackupExists: { log.backup },
            pendingKey: { log.pending },
            setup: { pending in self.setupActions(log, pending: pending) },
            restart: { log.restarts += 1 },
            encrypt: { key, progress in
                log.encrypted.append(key)
                log.pending = key
                progress(0.5)
                return result
            }
        ))
    }

    private func finishSetup(_ m: EncryptionSettingsModel) async throws {
        let setup = try XCTUnwrap(m.setupModel)
        setup.passphrase = "correct horse battery"
        setup.confirmation = "correct horse battery"
        await setup.perform(.createRecoveryKey)
        setup.testPassphrase = "correct horse battery"
        await setup.perform(.test)
        await setup.perform(.saveFile)
        setup.savedConfirmed = true
        XCTAssertTrue(setup.canContinue)
        await setup.perform(.continueToEncrypt)
        for _ in 0..<200 {
            if case .encrypting = m.phase { await Task.yield(); continue }
            if m.phase == .settingUp { await Task.yield(); continue }
            break
        }
    }

    func testStateFromEveryOutcome() {
        typealias M = EncryptionSettingsModel
        XCTAssertEqual(M.state(for: .opened), .encrypted)
        XCTAssertEqual(M.state(for: .needsMigration), .notEncrypted)
        XCTAssertEqual(M.state(for: .newInstall), .notEncrypted)
        XCTAssertEqual(M.state(for: nil), .notEncrypted, "before the keyed launch exists, the database is plain")
        XCTAssertEqual(M.state(for: .needsKey), .needsAttention(EncryptionCopy.attentionNeedsKey))
        XCTAssertEqual(M.state(for: .keyRejected("file is not a database")), .needsAttention(EncryptionCopy.attentionKeyRejected))
        XCTAssertEqual(M.state(for: .keyUnavailable("status -25308")), .needsAttention(EncryptionCopy.attentionKeyUnavailable))
        XCTAssertEqual(M.state(for: .migrationInterrupted("plain.bak present")), .needsAttention(EncryptionCopy.attentionMigrationInterrupted))
        XCTAssertEqual(M.state(for: .startupFailed("sweep failed")), .needsAttention(EncryptionCopy.attentionStartupFailed))
        XCTAssertEqual(M.state(for: .damaged("truncated")), .needsAttention(EncryptionCopy.attentionDamaged))
        XCTAssertEqual(M.state(for: .unsupported("no sqlcipher")), .needsAttention(EncryptionCopy.attentionUnsupported))
        XCTAssertEqual(M.state(for: .invalidArgument), .needsAttention(EncryptionCopy.attentionUnexpected(-1)))
        XCTAssertEqual(M.state(for: .unknown(42, "")), .needsAttention(EncryptionCopy.attentionUnexpected(42)))
    }

    func testCoreMessagesNeverReachTheScreen() {
        if case let .needsAttention(text) = EncryptionSettingsModel.state(for: .keyRejected("SECRET-DETAIL")) {
            XCTAssertFalse(text.contains("SECRET-DETAIL"))
        } else { XCTFail() }
    }

    func testExportEverythingIsOfferedBeforeEncrypting() async {
        let log = Log()
        let m = model(log)
        XCTAssertTrue(m.canStartEncrypt)
        await m.continueToSetup()
        XCTAssertEqual(m.phase, .idle, "setup cannot be reached from idle")
        XCTAssertNil(m.setupModel)

        m.startEncrypt()
        XCTAssertEqual(m.phase, .offeringExport)
        m.exportEverything()
        XCTAssertEqual(log.exports, 1)
        XCTAssertTrue(m.exportOffered)
        XCTAssertEqual(log.prechecks, 0)

        await m.continueToSetup()
        XCTAssertEqual(log.prechecks, 1)
        XCTAssertEqual(m.phase, .settingUp)
        XCTAssertNotNil(m.setupModel)
        XCTAssertTrue(log.encrypted.isEmpty, "nothing is encrypted before the recovery key is kept")
    }

    func testAFailedPrecheckStopsBeforeAnyKeyIsMade() async {
        let log = Log()
        let m = model(log, precheck: EncryptionCopy.skipRecording)
        m.startEncrypt()
        await m.continueToSetup()
        XCTAssertEqual(log.prechecks, 1)
        XCTAssertEqual(m.phase, .idle, "stays idle")
        XCTAssertNil(m.setupModel, "no setup sheet")
        XCTAssertEqual(log.begun, 0, "no key, no words, no file")
        XCTAssertEqual(m.notice, EncryptionCopy.skipRecording)
        XCTAssertTrue(m.canStartEncrypt, "can try again")
        m.startEncrypt()
        XCTAssertNil(m.notice)
    }

    func testSkipMessageFromPrecheckOutcome() {
        typealias O = CoreEncryptionMigration.Outcome
        XCTAssertNil(EncryptionSettingsModel.skipMessage(from: O(ok: true, phase: nil, error: nil)))
        let low = O(ok: false, phase: nil, error: .init(kind: "skipped", message: "m",
                    skip: .init(reason: "low_disk", needBytes: 1, freeBytes: 0, phase: nil, attempts: nil, last: nil)))
        XCTAssertEqual(EncryptionSettingsModel.skipMessage(from: low), EncryptionCopy.skipLowDisk)
        XCTAssertEqual(EncryptionSettingsModel.skipMessage(from: nil), EncryptionCopy.encryptFailed)
    }

    func testSetupKeyIsTheKeyEncryptedWithAndTheBadgeUpdates() async throws {
        let log = Log()
        let m = model(log)
        XCTAssertEqual(m.state, .notEncrypted)
        m.startEncrypt()
        await m.continueToSetup()
        try await finishSetup(m)
        XCTAssertEqual(log.encrypted, [Data(repeating: 1, count: 32)], "the key setup made")
        XCTAssertEqual(m.phase, .finished(EncryptionCopy.encryptDone))
        XCTAssertEqual(m.state, .encrypted, "the badge says Encrypted after success")
        XCTAssertNil(m.setupModel)
        m.dismissResult()
        XCTAssertEqual(m.phase, .finished(EncryptionCopy.encryptDone), "only a failure is dismissed")
        XCTAssertFalse(m.canStartEncrypt)
    }

    func testFailureCanBeDismissedBackToIdle() async throws {
        let log = Log()
        let m = model(log, result: .failed(EncryptionCopy.encryptFailed))
        m.startEncrypt()
        await m.continueToSetup()
        try await finishSetup(m)
        XCTAssertEqual(m.phase, .failed(EncryptionCopy.encryptFailed))
        XCTAssertEqual(m.state, .notEncrypted)
        m.dismissResult()
        XCTAssertEqual(m.phase, .idle)
        XCTAssertTrue(m.canStartEncrypt)
    }

    func testNextLaunchResultKeepsStatePlainAndBlocksASecondRun() async throws {
        let log = Log()
        let m = model(log, result: .finishesAtNextLaunch)
        m.startEncrypt()
        await m.continueToSetup()
        try await finishSetup(m)
        XCTAssertEqual(m.phase, .finished(EncryptionCopy.encryptAtNextLaunch))
        XCTAssertEqual(m.state, .notEncrypted)
        XCTAssertTrue(m.pendingNextLaunch)
        m.dismissResult()
        XCTAssertEqual(m.phase, .finished(EncryptionCopy.encryptAtNextLaunch), "not dismissable")
        XCTAssertFalse(m.canStartEncrypt, "no second run, no fresh key")
        m.startEncrypt()
        XCTAssertEqual(m.phase, .finished(EncryptionCopy.encryptAtNextLaunch))
        XCTAssertEqual(log.begun, 1)
        m.restart()
        XCTAssertEqual(log.restarts, 1, "Restart is the only way on")
    }

    func testRestartIsOnlyForAPendingRun() {
        let log = Log()
        let m = model(log)
        m.restart()
        XCTAssertEqual(log.restarts, 0)
    }

    func testRetryAfterAFailureReusesThePendingKeyAndWords() async throws {
        let log = Log()
        let m = model(log, result: .failed(EncryptionCopy.encryptFailed))
        m.startEncrypt()
        await m.continueToSetup()
        let firstWords = try XCTUnwrap(m.setupModel)
        firstWords.passphrase = "correct horse battery"
        firstWords.confirmation = "correct horse battery"
        await firstWords.perform(.createRecoveryKey)
        let words1 = try XCTUnwrap(firstWords.material?.words)
        try await finishSetupAfterCreate(m, firstWords)
        XCTAssertEqual(m.phase, .failed(EncryptionCopy.encryptFailed))
        XCTAssertNotNil(log.pending, "the attempt left a pending key")

        m.dismissResult()
        XCTAssertEqual(m.phase, .idle)
        XCTAssertTrue(m.canStartEncrypt)
        m.startEncrypt()
        await m.continueToSetup()
        let retry = try XCTUnwrap(m.setupModel)
        XCTAssertEqual(log.setupKeys.last ?? nil, log.pending, "the setup was made for the pending key")
        XCTAssertTrue(retry.actions.reusesKey)
        retry.passphrase = "another good passphrase"
        retry.confirmation = "another good passphrase"
        await retry.perform(.createRecoveryKey)
        XCTAssertEqual(retry.material?.words, words1, "same key, same words")
        XCTAssertEqual(retry.material?.key, log.pending)
    }

    private func finishSetupAfterCreate(_ m: EncryptionSettingsModel, _ setup: EncryptionSetupModel) async throws {
        setup.testPassphrase = "correct horse battery"
        await setup.perform(.test)
        await setup.perform(.saveFile)
        setup.savedConfirmed = true
        await setup.perform(.continueToEncrypt)
        for _ in 0..<200 {
            if case .encrypting = m.phase { await Task.yield(); continue }
            if m.phase == .settingUp { await Task.yield(); continue }
            break
        }
    }

    func testAPrecheckAnsweredAfterCancelIsIgnored() async {
        let log = Log()
        var m: EncryptionSettingsModel!
        m = EncryptionSettingsModel(outcome: .needsMigration, actions: .init(
            exportEverything: {},
            precheck: { m.cancel(); return EncryptionCopy.skipRecording },
            plainBackupExists: { false }, pendingKey: { nil },
            setup: { pending in self.setupActions(log, pending: pending) }, restart: {},
            encrypt: { _, _ in .encrypted }))
        m.startEncrypt()
        await m.continueToSetup()
        XCTAssertEqual(m.phase, .idle)
        XCTAssertNil(m.notice, "a cancelled check does not leave its message")
    }

    func testCancelGoesBackWithoutEncrypting() async {
        let log = Log()
        let m = model(log)
        m.startEncrypt()
        m.cancel()
        XCTAssertEqual(m.phase, .idle)
        m.startEncrypt()
        await m.continueToSetup()
        m.cancel()
        XCTAssertEqual(m.phase, .idle)
        XCTAssertNil(m.setupModel)
        XCTAssertTrue(log.encrypted.isEmpty)
    }

    func testOnlyAPlainDatabaseOffersEncrypt() {
        let log = Log()
        XCTAssertFalse(model(log, outcome: .opened).canStartEncrypt)
        XCTAssertFalse(model(log, outcome: .needsKey).canStartEncrypt)
        let m = model(log, outcome: .opened)
        m.startEncrypt()
        XCTAssertEqual(m.phase, .idle)
    }

    func testPlainBackupNoticeFollowsTheFile() async throws {
        let log = Log()
        XCTAssertFalse(model(log).plainBackupPresent)
        log.backup = true
        XCTAssertTrue(model(log, outcome: .opened).plainBackupPresent)
        // Re-read after a run.
        log.backup = false
        let m = model(log)
        m.startEncrypt()
        await m.continueToSetup()
        log.backup = true
        try await finishSetup(m)
        XCTAssertTrue(m.plainBackupPresent)
        XCTAssertEqual(EncryptionCopy.plainBackupNotice,
                       "Whimbrel keeps an unencrypted copy from before encryption until it has started normally three times, then deletes it. Deleting a file does not guarantee its contents are gone from the disk.")
    }

    func testMigrationOutcomeMapping() {
        typealias M = EncryptionSettingsModel
        typealias O = CoreEncryptionMigration.Outcome
        typealias F = CoreEncryptionMigration.Failure
        typealias S = CoreEncryptionMigration.Skip
        func skip(_ reason: String) -> O {
            O(ok: false, phase: nil, error: F(kind: "skipped", message: "m", skip: S(reason: reason, needBytes: nil, freeBytes: nil, phase: nil, attempts: nil, last: nil)))
        }
        XCTAssertEqual(M.result(from: O(ok: true, phase: "done", error: nil)), .encrypted)
        XCTAssertEqual(M.result(from: skip("recording")), .skipped(EncryptionCopy.skipRecording))
        XCTAssertEqual(M.result(from: skip("low_disk")), .skipped(EncryptionCopy.skipLowDisk))
        XCTAssertEqual(M.result(from: skip("open_elsewhere")), .skipped(EncryptionCopy.skipOpenElsewhere))
        // The app's own open store: the run happens at the next launch, before it opens.
        XCTAssertEqual(M.result(from: skip("open_in_this_process")), .finishesAtNextLaunch)
        XCTAssertNil(M.skipMessage(from: skip("open_in_this_process")))
        XCTAssertEqual(M.result(from: skip("failed_before")), .failed(EncryptionCopy.encryptFailed))
        XCTAssertEqual(M.result(from: skip("something_new")), .skipped(EncryptionCopy.skipOther))
        XCTAssertEqual(M.result(from: O(ok: false, phase: nil, error: F(kind: "verify", message: "rows differ", skip: nil))), .failed(EncryptionCopy.encryptFailed))
        XCTAssertEqual(M.result(from: nil), .failed(EncryptionCopy.encryptFailed))
    }

    func testPaneCopySaysWhatIsNotEncrypted() {
        XCTAssertEqual(EncryptionCopy.notEncryptedList,
                       "Not encrypted: recordings, slide images, exports, the Obsidian vault, captured screen text files, settings, logs and diagnostics. These stay ordinary files.")
        XCTAssertEqual(EncryptionCopy.settingsEncryptedBody,
                       "The memory database on this Mac is encrypted with a key kept in your Mac Keychain. A copy of the database file is unreadable without it. Backups made before you turned this on, such as Time Machine, still hold a readable copy.")
    }
}
