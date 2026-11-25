import SwiftUI
import WhimbrelUI

/// `EncryptionRecoveryView` in the redesign: the same model, the same `perform` calls, the same
/// secrets cleared when the screen goes away.
struct RecoveryRedesignView: View {
    @ObservedObject var model: EncryptionRecoveryModel
    /// Quit, from the launch window. Nil leaves it out.
    var quit: (() -> Void)?

    var body: some View {
        WRecoveryView(
            model: Self.content(model, quit: quit, unlock: { Task { await model.perform(model.method == .file ? .restoreFromFile : .restoreFromWords) } },
                                chooseFile: { Task { await model.perform(.chooseFile) } }),
            method: Binding(get: { model.method == .file ? .file : .words }, set: { model.method = $0 == .file ? .file : .words }),
            passphrase: $model.passphrase, words: $model.words)
            .redesignThemed()
            .onDisappear { model.clearSecrets() }
    }

    static func content(_ model: EncryptionRecoveryModel, quit: (() -> Void)?, unlock: @escaping () -> Void, chooseFile: @escaping () -> Void) -> WRecoveryModel {
        var m = WRecoveryModel()
        m.title = LibraryCopyRedesign.map(EncryptionCopy.recoveryTitle)
        m.reason = LibraryCopyRedesign.map(model.explanation)
        m.unchanged = EncryptionCopy.recoveryUnchanged
        m.methodLabel = String(localized: "Recovery method", comment: "Accessibility label of the switch between a recovery file and 24 words.")
        m.fileMethodTitle = EncryptionCopy.methodFile
        m.wordsMethodTitle = EncryptionCopy.methodWords
        m.chooseFileTitle = EncryptionCopy.chooseFile
        m.chosenFileName = model.chosenFileName
        m.passphraseLabel = EncryptionCopy.passphraseField
        m.wordsLabel = EncryptionCopy.methodWords
        m.wordsHelp = EncryptionCopy.wordsHelp
        m.unlockTitle = model.working ? EncryptionCopy.unlocking : (model.method == .file ? EncryptionCopy.unlockWithFile : EncryptionCopy.unlockWithWords)
        m.canUnlock = model.canRestore
        m.working = model.working
        m.errorLine = model.error.map(LibraryCopyRedesign.map)
        m.restoredLine = model.restored ? LibraryCopyRedesign.map(EncryptionCopy.restored) : nil
        m.finePrint = [EncryptionCopy.recoveryNeverDeletes, EncryptionCopy.recoveryLostBoth].map(LibraryCopyRedesign.map)
        m.quitTitle = quit == nil ? nil : EncryptionCopy.quitAction
        m.quitEnabled = !model.working
        m.onChooseFile = chooseFile
        m.onUnlock = unlock
        m.onQuit = quit ?? {}
        return m
    }
}
