import SwiftUI
import WhimbrelUI

/// `EncryptionSetupView` in the redesign: the same model and the same `perform` calls, the same Start over
/// confirmation, the same secrets cleared when the screen goes away.
/// The sentences of the sheet that say what the encryption protects. `.library` says "your library" (decision 3); `.classic` is
/// `EncryptionCopy`'s own wording, kept for a caller that wants it. `EncryptionCopy` itself is not changed.
struct KeySetupStrings {
    var intro: String
    var wordsAreTheKey: String
    var lostBoth: String
    var sameDiskWarning: String
    var encryptContinue: String

    static let library = KeySetupStrings(
        intro: LibraryCopyRedesign.setupIntro, wordsAreTheKey: LibraryCopyRedesign.wordsAreTheKey, lostBoth: LibraryCopyRedesign.lostBoth,
        sameDiskWarning: LibraryCopyRedesign.sameDiskWarning, encryptContinue: LibraryCopyRedesign.encryptContinue)
    static let classic = KeySetupStrings(
        intro: EncryptionCopy.setupIntro, wordsAreTheKey: EncryptionCopy.wordsAreTheKey, lostBoth: EncryptionCopy.lostBoth,
        sameDiskWarning: EncryptionCopy.sameDiskWarning, encryptContinue: EncryptionCopy.encryptContinue)

    /// The button's title: the setup model asks for "Encrypt the memory database" by name; any other title (Continue) stays.
    func continueTitle(_ modelTitle: String) -> String { modelTitle == EncryptionCopy.encryptContinue ? encryptContinue : modelTitle }
}

struct KeySetupRedesignView: View {
    @ObservedObject var model: EncryptionSetupModel
    /// The words that name what is protected. Every caller is the redesign, so the default says "your library".
    var strings: KeySetupStrings = .library
    /// Quit (launch window) or Cancel (Settings sheet). Nil leaves it out.
    var leave: Leave?

    struct Leave {
        var title: String
        /// Escape triggers it (Cancel in the Settings sheet, never Quit).
        var isEscape: Bool
        var isEnabled: Bool
        var run: () -> Void
    }

    var body: some View {
        WKeySetupView(
            model: Self.content(model, leave: leave, strings: strings),
            passphrase: $model.passphrase, confirmation: $model.confirmation, testPassphrase: $model.testPassphrase,
            wroteDown: $model.wroteDownTicked, savedConfirmed: $model.savedConfirmed)
            .redesignThemed()
            .onDisappear { model.clearSecrets() }
            .alert(model.startOverTitle, isPresented: $model.startOverPending) {
                Button(EncryptionCopy.startOverConfirm, role: .destructive) { Task { await model.perform(.confirmStartOver) } }
                Button(EncryptionCopy.keepThisKey, role: .cancel) {}
            } message: {
                Text(model.startOverBody)
            }
    }

    static func content(_ model: EncryptionSetupModel, leave: Leave?, strings: KeySetupStrings = .library) -> WKeySetupModel {
        var m = WKeySetupModel()
        m.title = EncryptionCopy.setupTitle
        m.intro = strings.intro
        m.passphraseHeading = EncryptionCopy.stepPassphrase
        m.passphraseHelp = EncryptionCopy.passphraseHelp
        m.passphraseLabel = EncryptionCopy.passphraseField
        m.confirmLabel = EncryptionCopy.confirmField
        m.passphraseCount = EncryptionCopy.passphraseCount(model.passphraseCount, model.minimumLength)
        m.meetsMinimum = model.meetsMinimum
        m.passphraseProblem = model.passphraseProblem ?? model.createError
        m.passphraseLockedHint = EncryptionCopy.passphraseLockedHint
        m.passphraseLocked = model.passphraseLocked
        m.hasKey = model.material != nil
        m.words = model.material?.words ?? []
        m.qrImage = model.qrImage
        m.qrLabel = EncryptionCopy.qrLabel
        m.fileHeading = EncryptionCopy.stepFile
        m.fileHelp = EncryptionCopy.fileHelp
        m.saveFileTitle = EncryptionCopy.saveFile
        m.savedLine = model.savedPath.map(EncryptionCopy.savedTo)
        m.sameDiskWarning = model.savedPath != nil && model.savedOnSameDisk ? strings.sameDiskWarning : nil
        m.saveFailedLine = model.saveFailed ? EncryptionCopy.saveFailed : nil
        m.wordsHeading = EncryptionCopy.stepWords
        m.wordsNotice = strings.wordsAreTheKey
        m.printTitle = EncryptionCopy.printWords
        m.printedNote = model.printed ? EncryptionCopy.printedNote : nil
        m.printToFileWarning = model.printWentToFile ? EncryptionCopy.printAsFileRefused : nil
        m.wroteDownLabel = EncryptionCopy.wroteDown
        m.testHeading = EncryptionCopy.stepTest
        m.testHelp = EncryptionCopy.testHelp
        m.testLabel = EncryptionCopy.passphraseField
        m.testButtonTitle = model.testing ? EncryptionCopy.testing : EncryptionCopy.testButton
        m.canTest = model.canTest
        m.testPassedLine = model.testPassed ? EncryptionCopy.testPassed : nil
        m.testFailedLine = !model.testPassed && model.testFailed ? EncryptionCopy.testFailed : nil
        m.savedItLabel = EncryptionCopy.iSavedIt
        m.savedItLockedHint = EncryptionCopy.iSavedItLocked
        m.canConfirmSaved = model.canConfirmSaved
        m.finePrint = strings.lostBoth
        m.createTitle = model.creating ? EncryptionCopy.creatingKey : EncryptionCopy.createKey
        m.canCreate = model.canCreate
        m.creating = model.creating
        m.continueTitle = strings.continueTitle(model.continueTitle)
        m.canContinue = model.canContinue
        m.startOverTitle = EncryptionCopy.startOver
        m.leaveTitle = leave?.title
        m.leaveIsEscape = leave?.isEscape ?? false
        m.leaveEnabled = leave?.isEnabled ?? true
        m.onLeave = leave?.run ?? {}
        m.onCreate = { Task { await model.perform(.createRecoveryKey) } }
        m.onSaveFile = { Task { await model.perform(.saveFile) } }
        m.onPrint = { Task { await model.perform(.printWords) } }
        m.onTest = { Task { await model.perform(.test) } }
        m.onStartOver = { Task { await model.perform(.startOver) } }
        m.onContinue = { Task { await model.perform(.continueToEncrypt) } }
        return m
    }
}
