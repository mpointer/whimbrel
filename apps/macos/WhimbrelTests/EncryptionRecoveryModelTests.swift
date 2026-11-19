import XCTest
@testable import Whimbrel

/// The key-gone recovery screen's model (X4 Task 10): restore from the file and passphrase or
/// from the 24 words, and nothing else.
@MainActor
final class EncryptionRecoveryModelTests: XCTestCase {
    final class Log {
        var fileRestores: [(String, String)] = []
        var wordRestores: [String] = []
        var used: [Data] = []
    }

    static let key = Data(repeating: 3, count: 32)

    private func model(_ log: Log, reason: EncryptionRecoveryModel.Reason = .keyMissing,
                       chosen: EncryptionRecoveryModel.ChosenFile = .chosen(name: "Whimbrel recovery key.txt", contents: "FILE"),
                       use: EncryptionRecoveryModel.UseKeyResult = .opened) -> EncryptionRecoveryModel {
        EncryptionRecoveryModel(reason: reason, actions: .init(
            chooseFile: { chosen },
            restoreFromFile: { f, p in
                log.fileRestores.append((f, p))
                return p == "right passphrase" ? .success(Self.key) : .failure(.wrongPassphraseOrDamaged)
            },
            restoreFromWords: { w in
                log.wordRestores.append(w)
                return w.lowercased().contains("abandon") ? .success(Self.key) : .failure(.badWords)
            },
            useKey: { k in log.used.append(k); return use }
        ))
    }

    func testOnlyRestoreActionsExist() {
        XCTAssertEqual(Set(EncryptionRecoveryModel.Action.allCases), [.chooseFile, .restoreFromFile, .restoreFromWords])
        // The live collaborators: choose a file, two restores, and use the key. Nothing else.
        let labels = Mirror(reflecting: EncryptionRecoveryModel.Actions.live(useKey: { _ in .opened })).children.compactMap { $0.label }
        XCTAssertEqual(Set(labels), ["chooseFile", "restoreFromFile", "restoreFromWords", "useKey"])
        let names = EncryptionRecoveryModel.Action.allCases.map { "\($0)".lowercased() }
        for banned in ["fresh", "new", "create", "delete", "reset", "erase", "remove", "skip"] {
            XCTAssertFalse(names.contains { $0.contains(banned) }, "\(banned) in \(names)")
        }
        // The screen's own words never offer it either.
        let copy = [EncryptionCopy.recoveryTitle, EncryptionCopy.recoveryKeyMissing, EncryptionCopy.recoveryKeyRejected,
                    EncryptionCopy.recoveryUnchanged, EncryptionCopy.recoveryLostBoth, EncryptionCopy.recoveryNeverDeletes]
            .joined(separator: " ").lowercased()
        for banned in ["start fresh", "start over", "new database", "create a new"] {
            XCTAssertFalse(copy.contains(banned), banned)
        }
    }

    func testWordsRestore() async {
        let log = Log()
        let m = model(log)
        m.method = .words
        XCTAssertFalse(m.canRestore)
        m.words = "  Abandon ABILITY able \n about"
        XCTAssertTrue(m.canRestore)
        await m.perform(.restoreFromWords)
        XCTAssertEqual(log.wordRestores, ["  Abandon ABILITY able \n about"], "pasted as typed; core cleans spacing and capitals")
        XCTAssertEqual(log.used, [Self.key])
        XCTAssertTrue(m.restored)
        XCTAssertNil(m.error)
        XCTAssertEqual(m.words, "", "the words are dropped once they worked")
    }

    func testBadWordsMessage() async {
        let log = Log()
        let m = model(log)
        m.method = .words
        m.words = "nope nope"
        await m.restore()
        XCTAssertEqual(m.error, EncryptionCopy.badWords)
        XCTAssertTrue(log.used.isEmpty)
        XCTAssertFalse(m.restored)
    }

    func testFileRestore() async {
        let log = Log()
        let m = model(log)
        m.method = .file
        XCTAssertFalse(m.canRestore, "needs a file")
        await m.perform(.chooseFile)
        XCTAssertEqual(m.chosenFileName, "Whimbrel recovery key.txt")
        XCTAssertFalse(m.canRestore, "needs a passphrase")
        m.passphrase = "right passphrase"
        XCTAssertTrue(m.canRestore)
        await m.restore()
        XCTAssertEqual(log.fileRestores.first?.0, "FILE")
        XCTAssertEqual(log.fileRestores.first?.1, "right passphrase")
        XCTAssertEqual(log.used, [Self.key])
        XCTAssertTrue(m.restored)
        // Once unlocked, the passphrase and file text are dropped.
        XCTAssertEqual(m.passphrase, "")
        XCTAssertNil(m.chosenFileName)
    }

    func testWrongPassphraseMessage() async {
        let log = Log()
        let m = model(log)
        m.method = .file
        await m.perform(.chooseFile)
        m.passphrase = "wrong passphrase"
        await m.restore()
        XCTAssertEqual(m.error, EncryptionCopy.wrongPassphrase)
        XCTAssertTrue(log.used.isEmpty)
        XCTAssertFalse(m.restored)
        // Typing again clears the message.
        m.passphrase = "right passphrase"
        XCTAssertNil(m.error)
    }

    func testNotARecoveryFileAndUnreadableFile() async {
        let log = Log()
        let m = EncryptionRecoveryModel(reason: .keyMissing, actions: .init(
            chooseFile: { .chosen(name: "notes.txt", contents: "hello") },
            restoreFromFile: { _, _ in .failure(.malformed) },
            restoreFromWords: { _ in .failure(.badWords) },
            useKey: { log.used.append($0); return .opened }
        ))
        await m.perform(.chooseFile)
        m.passphrase = "anything at all"
        await m.restore()
        XCTAssertEqual(m.error, EncryptionCopy.notARecoveryFile)

        let u = model(log, chosen: .unreadable)
        await u.perform(.chooseFile)
        XCTAssertNil(u.chosenFileName)
        XCTAssertEqual(u.error, EncryptionCopy.fileUnreadable)
    }

    func testAKeyForAnotherDatabaseIsSaidPlainly() async {
        let log = Log()
        let m = model(log, use: .keyRejected)
        m.method = .words
        m.words = "abandon"
        await m.restore()
        XCTAssertEqual(log.used, [Self.key])
        XCTAssertFalse(m.restored)
        XCTAssertEqual(m.error, EncryptionCopy.keyForAnotherDatabase)
    }

    func testReasonPicksTheExplanation() {
        let log = Log()
        XCTAssertEqual(model(log, reason: .keyMissing).explanation, EncryptionCopy.recoveryKeyMissing)
        XCTAssertEqual(model(log, reason: .keyRejected).explanation, EncryptionCopy.recoveryKeyRejected)
    }
}

/// The "can't read the key right now" screen (Keychain read failed): Retry, nothing else.
@MainActor
final class EncryptionKeyUnavailableModelTests: XCTestCase {
    func testRetryIsTheOnlyAction() async {
        XCTAssertEqual(EncryptionKeyUnavailableModel.Action.allCases, [.retry])
        var calls = 0
        let m = EncryptionKeyUnavailableModel(detail: "") { calls += 1; return true }
        await m.perform(.retry)
        XCTAssertEqual(calls, 1)
    }

    func testRetryCallsBackAndReportsStillUnavailable() async {
        var calls = 0
        let m = EncryptionKeyUnavailableModel(detail: "Keychain read failed (status -25308)") {
            calls += 1
            return false
        }
        XCTAssertEqual(m.detail, "Keychain read failed (status -25308)")
        XCTAssertFalse(m.stillUnavailable)
        await m.retry()
        XCTAssertEqual(calls, 1)
        XCTAssertFalse(m.retrying)
        XCTAssertTrue(m.stillUnavailable)
        await m.retry()
        XCTAssertEqual(calls, 2)
    }

    func testRetrySuccessClearsTheMessage() async {
        let m = EncryptionKeyUnavailableModel(detail: "x") { true }
        await m.retry()
        XCTAssertFalse(m.stillUnavailable)
    }

    func testCopySaysThisIsNotALostKey() {
        let text = EncryptionCopy.unavailableBody.lowercased()
        XCTAssertTrue(text.contains("not the same as a lost key"), text)
        XCTAssertTrue(EncryptionCopy.unavailableTouchesNothing.lowercased().contains("has not opened, changed or deleted"))
    }
}
