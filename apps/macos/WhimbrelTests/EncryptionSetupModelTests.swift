import XCTest
@testable import Whimbrel

/// The recovery-key setup screen's model (X4 Task 10). Every collaborator is a closure, so no
/// Keychain, store or core call runs here.
@MainActor
final class EncryptionSetupModelTests: XCTestCase {
    final class Log {
        var begun: [String] = []
        var verified: [(String, String)] = []
        var saved: [String] = []
        var printed: [[String]] = []
        var continued: [Data] = []
    }

    static let words = (1...24).map { "word\($0)" }
    static let key = Data(repeating: 7, count: 32)
    static let file = "whimbrel-recovery 1\nkdf argon2id m=65536 t=3 p=1\nsalt AAAA\nv1:token\n"
    static let pass = "correct horse battery"

    private func model(_ log: Log,
                       save: EncryptionSetupModel.SaveResult = .saved(path: "/Volumes/USB/Whimbrel recovery key.txt", sameDiskAsDatabase: false),
                       print: EncryptionSetupModel.PrintResult = .printed, reusesKey: Bool = false) -> EncryptionSetupModel {
        EncryptionSetupModel(actions: .init(
            minimumLength: 12,
            passphraseLength: { $0.count },
            begin: { p in
                log.begun.append(p)
                return p.count < 12 ? .failure(.passphraseTooShort)
                    : .success(CoreSetupMaterial(key: Self.key, recoveryFile: Self.file, words: Self.words))
            },
            verify: { f, p in log.verified.append((f, p)); return p == Self.pass },
            saveFile: { contents in log.saved.append(contents); return save },
            printWords: { log.printed.append($0); return print },
            reusesKey: reusesKey
        ), continueTitle: "Encrypt", onContinue: { log.continued.append($0) })
    }

    private func created(_ log: Log, save: EncryptionSetupModel.SaveResult = .saved(path: "/Volumes/USB/x.txt", sameDiskAsDatabase: false),
                         print: EncryptionSetupModel.PrintResult = .printed, reusesKey: Bool = false) async -> EncryptionSetupModel {
        let m = model(log, save: save, print: print, reusesKey: reusesKey)
        m.passphrase = Self.pass
        m.confirmation = Self.pass
        await m.perform(.createRecoveryKey)
        return m
    }

    private func passTest(_ m: EncryptionSetupModel) async {
        m.testPassphrase = Self.pass
        await m.perform(.test)
        XCTAssertTrue(m.testPassed)
    }

    // MARK: Passphrase

    func testPassphraseMinimumIsTwelveCharacters() async {
        let log = Log()
        let m = model(log)
        m.passphrase = "elevenchars"
        m.confirmation = "elevenchars"
        XCTAssertEqual(m.passphraseCount, 11)
        XCTAssertFalse(m.meetsMinimum)
        XCTAssertFalse(m.canCreate)
        XCTAssertEqual(m.passphraseProblem, EncryptionCopy.passphraseTooShort(12))
        await m.perform(.createRecoveryKey)
        XCTAssertTrue(log.begun.isEmpty, "core is not asked with a short passphrase")
        XCTAssertNil(m.material)

        m.passphrase = "twelve chars"
        m.confirmation = "twelve chars"
        XCTAssertTrue(m.meetsMinimum)
        XCTAssertTrue(m.canCreate)
        XCTAssertNil(m.passphraseProblem)
    }

    func testCoreRefusalForLengthIsShown() async {
        let m = EncryptionSetupModel(actions: .init(
            minimumLength: 12, passphraseLength: { _ in 12 },
            begin: { _ in .failure(.passphraseTooShort) }, verify: { _, _ in false },
            saveFile: { _ in .cancelled }, printWords: { _ in .cancelled }
        ), continueTitle: "Encrypt", onContinue: { _ in })
        m.passphrase = "abcdefghijkl"
        m.confirmation = "abcdefghijkl"
        await m.perform(.createRecoveryKey)
        XCTAssertNil(m.material)
        XCTAssertEqual(m.createError, EncryptionCopy.passphraseTooShort(12))
    }

    func testMismatchedConfirmationBlocksCreate() async {
        let log = Log()
        let m = model(log)
        m.passphrase = Self.pass
        m.confirmation = "correct horse batterz"
        XCTAssertFalse(m.confirmationMatches)
        XCTAssertFalse(m.canCreate)
        XCTAssertEqual(m.passphraseProblem, EncryptionCopy.passphrasesDiffer)
        await m.perform(.createRecoveryKey)
        XCTAssertTrue(log.begun.isEmpty)
        m.confirmation = ""
        XCTAssertNil(m.passphraseProblem)
        XCTAssertFalse(m.canCreate)
    }

    func testCreateGivesFileWordsAndOneKey() async {
        let log = Log()
        let m = await created(log)
        XCTAssertEqual(log.begun, [Self.pass])
        XCTAssertEqual(m.material?.words, Self.words)
        XCTAssertEqual(m.material?.recoveryFile, Self.file)
        XCTAssertNotNil(m.qrImage, "a QR of the words")
        await m.perform(.saveFile)
        XCTAssertEqual(log.saved, [Self.file], "the saved file is the sealed text, never the words")
        XCTAssertEqual(m.savedPath, "/Volumes/USB/x.txt")
        await m.perform(.printWords)
        XCTAssertEqual(log.printed, [Self.words])
        XCTAssertTrue(m.printed)
    }

    // MARK: Locked passphrase and Start over

    func testThePassphraseIsLockedOnceTheKeyExists() async {
        let log = Log()
        let m = await created(log)
        await passTest(m)
        await m.perform(.saveFile)
        XCTAssertTrue(m.passphraseLocked)
        m.passphrase = "another passphrase here"
        m.confirmation = "another passphrase here"
        XCTAssertEqual(m.passphrase, Self.pass, "edits are refused while the key exists")
        XCTAssertEqual(m.confirmation, Self.pass)
        XCTAssertEqual(m.material?.key, Self.key, "the key, file and checks stay")
        XCTAssertTrue(m.testPassed)
        XCTAssertNotNil(m.savedPath)
    }

    func testStartOverAsksThenDiscardsEverything() async {
        let log = Log()
        let m = await created(log)
        await passTest(m)
        await m.perform(.saveFile)
        m.wroteDownTicked = true
        m.savedConfirmed = true

        await m.perform(.startOver)
        XCTAssertTrue(m.startOverPending, "asks first")
        XCTAssertNotNil(m.material, "nothing is discarded until confirmed")
        XCTAssertTrue(EncryptionCopy.startOverBody.contains("Any recovery file or printout you already made will not work with the new one."))

        await m.perform(.confirmStartOver)
        XCTAssertFalse(m.startOverPending)
        XCTAssertNil(m.material)
        XCTAssertNil(m.savedPath)
        XCTAssertFalse(m.printed)
        XCTAssertFalse(m.wroteDownTicked)
        XCTAssertFalse(m.testPassed)
        XCTAssertFalse(m.savedConfirmed)
        XCTAssertEqual(m.passphrase, "")
        XCTAssertEqual(m.confirmation, "")
        XCTAssertFalse(m.passphraseLocked)
        XCTAssertFalse(m.canContinue)
    }

    func testStartOverWordingForAFreshKey() async {
        let m = await created(Log())
        XCTAssertEqual(m.startOverTitle, EncryptionCopy.startOverTitle)
        XCTAssertEqual(m.startOverBody, "Any recovery file or printout you already made will not work with the new one.")
        XCTAssertFalse(EncryptionSetupModel.Actions.live().reusesKey)
    }

    func testStartOverWordingWhenTheKeyIsReused() async {
        let m = await created(Log(), reusesKey: true)
        XCTAssertEqual(m.startOverTitle, "Change the passphrase for your recovery file?")
        XCTAssertEqual(m.startOverBody, "Your 24 words stay the same. A recovery file you already saved still opens with its old passphrase.")
        XCTAssertTrue(EncryptionSetupModel.Actions.live(existingKey: Data(count: 32)).reusesKey)
    }

    func testSaveAsPDFDoesNotCountAndWarns() async {
        let log = Log()
        let m = await created(log, print: .savedAsFile)
        await passTest(m)
        await m.perform(.printWords)
        XCTAssertFalse(m.printed)
        XCTAssertTrue(m.printWentToFile)
        XCTAssertFalse(m.canConfirmSaved, "a PDF on this Mac is not a kept copy")
        XCTAssertEqual(EncryptionCopy.printAsFileRefused,
                       "Saving the page as a PDF would leave your recovery words as an ordinary file on this Mac. Print it on paper instead, or write the words down.")
    }

    func testPrintResultFromThePrintOperation() {
        XCTAssertEqual(EncryptionPrint.result(ran: true, disposition: .spool), .printed)
        XCTAssertEqual(EncryptionPrint.result(ran: true, disposition: .save), .savedAsFile)
        XCTAssertEqual(EncryptionPrint.result(ran: true, disposition: .preview), .savedAsFile)
        XCTAssertEqual(EncryptionPrint.result(ran: false, disposition: .spool), .cancelled)
        XCTAssertEqual(EncryptionPrint.result(ran: false, disposition: .save), .cancelled)
    }

    func testStartOverDoesNothingBeforeAKeyExists() async {
        let log = Log()
        let m = model(log)
        await m.perform(.startOver)
        XCTAssertFalse(m.startOverPending)
    }

    // MARK: The test and "I have kept it"

    func testConfirmIsDisabledWithNoRoute() async {
        let log = Log()
        let m = await created(log)
        await passTest(m)
        XCTAssertFalse(m.keptSomewhere)
        XCTAssertFalse(m.canConfirmSaved, "the test alone proves nothing was kept")
        m.savedConfirmed = true
        XCTAssertFalse(m.savedConfirmed)
    }

    func testConfirmViaSavedFile() async {
        let log = Log()
        let m = await created(log)
        await passTest(m)
        await m.perform(.saveFile)
        XCTAssertTrue(m.canConfirmSaved)
        m.savedConfirmed = true
        XCTAssertTrue(m.savedConfirmed)
    }

    func testConfirmViaPrint() async {
        let log = Log()
        let m = await created(log)
        await passTest(m)
        await m.perform(.printWords)
        XCTAssertTrue(m.canConfirmSaved)
        m.savedConfirmed = true
        XCTAssertTrue(m.savedConfirmed)
    }

    func testCancelledPrintIsNotARoute() async {
        let log = Log()
        let m = await created(log, print: .cancelled)
        await passTest(m)
        await m.perform(.printWords)
        XCTAssertFalse(m.printed)
        XCTAssertFalse(m.canConfirmSaved)
    }

    func testConfirmViaWrittenDown() async {
        let log = Log()
        let m = await created(log)
        await passTest(m)
        m.wroteDownTicked = true
        XCTAssertTrue(m.canConfirmSaved)
        m.savedConfirmed = true
        XCTAssertTrue(m.savedConfirmed)
        // Unticking the only route unticks the confirmation.
        m.wroteDownTicked = false
        XCTAssertFalse(m.savedConfirmed)
    }

    func testEachRouteAloneWithoutThePassphraseTestIsDisabled() async {
        for route in ["file", "print", "written"] {
            let log = Log()
            let m = await created(log)
            switch route {
            case "file": await m.perform(.saveFile)
            case "print": await m.perform(.printWords)
            default: m.wroteDownTicked = true
            }
            XCTAssertTrue(m.keptSomewhere, route)
            XCTAssertFalse(m.canConfirmSaved, route)
            m.savedConfirmed = true
            XCTAssertFalse(m.savedConfirmed, route)
            // A failed test does not count either.
            m.testPassphrase = "wrong passphrase!!"
            await m.perform(.test)
            XCTAssertTrue(m.testFailed, route)
            XCTAssertFalse(m.canConfirmSaved, route)
        }
    }

    func testContinueNeedsTestRouteAndConfirmation() async {
        let log = Log()
        let m = await created(log)
        await m.perform(.continueToEncrypt)
        XCTAssertTrue(log.continued.isEmpty)
        await passTest(m)
        m.wroteDownTicked = true
        XCTAssertFalse(m.canContinue, "not until confirmed")
        await m.perform(.continueToEncrypt)
        XCTAssertTrue(log.continued.isEmpty)
        m.savedConfirmed = true
        XCTAssertTrue(m.canContinue)
        await m.perform(.continueToEncrypt)
        XCTAssertEqual(log.continued, [Self.key], "continues with the key setup made, not a new one")
        // The words, QR, file text and passphrases are gone from the model afterwards.
        XCTAssertNil(m.material)
        XCTAssertNil(m.qrImage)
        XCTAssertEqual(m.passphrase, "")
        XCTAssertEqual(m.confirmation, "")
        XCTAssertEqual(m.testPassphrase, "")
        XCTAssertFalse(m.canContinue)
    }

    func testEditingTheTestPassphraseClearsTheResult() async {
        let log = Log()
        let m = await created(log)
        await passTest(m)
        m.wroteDownTicked = true
        m.savedConfirmed = true
        m.testPassphrase = "correct horse batter"
        XCTAssertFalse(m.testPassed)
        XCTAssertFalse(m.savedConfirmed)
    }

    func testSaveResults() async {
        let log = Log()
        let failed = await created(log, save: .failed)
        await failed.perform(.saveFile)
        XCTAssertTrue(failed.saveFailed)
        XCTAssertNil(failed.savedPath)
        let cancelled = await created(log, save: .cancelled)
        await cancelled.perform(.saveFile)
        XCTAssertFalse(cancelled.saveFailed)
        XCTAssertNil(cancelled.savedPath)
        // Same disk as the database: saved, counts as a route, but warns.
        let same = await created(log, save: .saved(path: "/Users/me/Documents/x.txt", sameDiskAsDatabase: true))
        await same.perform(.saveFile)
        XCTAssertTrue(same.savedOnSameDisk)
        XCTAssertEqual(same.savedPath, "/Users/me/Documents/x.txt")
        await passTest(same)
        XCTAssertTrue(same.canConfirmSaved)
    }

    func testClearSecretsDropsWordsAndPassphrases() async {
        let log = Log()
        let m = await created(log)
        await passTest(m)
        m.clearSecrets()
        XCTAssertNil(m.material)
        XCTAssertNil(m.qrImage)
        XCTAssertEqual(m.passphrase, "")
        XCTAssertEqual(m.testPassphrase, "")
        XCTAssertFalse(m.testPassed)
    }

    // MARK: Actions

    func testThereIsNoSkipOrCopy() {
        XCTAssertEqual(Set(EncryptionSetupModel.Action.allCases), [
            .createRecoveryKey, .saveFile, .printWords, .test, .startOver, .confirmStartOver, .continueToEncrypt,
        ])
        // The live collaborators: none puts anything on the pasteboard.
        let labels = Mirror(reflecting: EncryptionSetupModel.Actions.live()).children.compactMap { $0.label?.lowercased() }
        XCTAssertEqual(Set(labels), ["minimumlength", "passphraselength", "begin", "verify", "savefile", "printwords", "reuseskey"])
    }
}
