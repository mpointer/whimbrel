import SwiftUI
import WhimbrelUI

/// `LaunchBlockingScreenView` in the redesign: the same text from `LaunchScreenContent`, the same
/// responses, the same confirmations. Quit first, the safest action last as the primary.
struct LaunchBlockingRedesignView: View {
    let screen: LaunchScreen
    var busy = false
    let respond: (LaunchScreenResponse) -> Void
    @State private var confirmRestore = false
    @State private var confirmReset = false

    var body: some View {
        let c = LaunchScreenContent.of(screen)
        WNoticeScreenView(model: Self.model(c, busy: busy, quit: { respond(.quit) }, restore: { confirmRestore = true },
                                            reset: { confirmReset = true }, continueWithout: { respond(.continueWithoutStore) },
                                            retry: { respond(.retry) }))
            .redesignThemed()
            .alert(EncryptionCopy.restoreConfirmTitle, isPresented: $confirmRestore) {
                Button(EncryptionCopy.restoreConfirm) { respond(.restoreRollbackCopy) }
                Button(EncryptionCopy.cancel, role: .cancel) {}
            } message: {
                Text(EncryptionCopy.restoreConfirmBody)
            }
            .alert(EncryptionCopy.resetConfirmTitle, isPresented: $confirmReset) {
                Button(EncryptionCopy.resetConfirm, role: .destructive) { respond(.resetEncryptionStatus) }
                Button(EncryptionCopy.cancel, role: .cancel) {}
            } message: {
                Text(c.resetConfirmBody ?? EncryptionCopy.resetConfirmBody)
            }
    }

    /// Pure: which buttons a blocking screen has, in order. Quit, then the secondary ones, then the primary.
    static func model(_ c: LaunchScreenContent, busy: Bool, quit: @escaping () -> Void, restore: @escaping () -> Void,
                      reset: @escaping () -> Void, continueWithout: @escaping () -> Void, retry: @escaping () -> Void) -> WNoticeScreenModel {
        var m = WNoticeScreenModel()
        m.title = c.title
        m.paragraphs = c.paragraphs
        m.hints = c.hints
        m.errorLine = c.lastAction
        m.busy = busy
        var actions = [WScreenAction(id: "quit", title: EncryptionCopy.quitAction, isEnabled: !busy, run: quit)]
        if c.continueWithoutStore { actions.append(WScreenAction(id: "continueWithoutStore", title: EncryptionCopy.continueWithoutStore, isEnabled: !busy, run: continueWithout)) }
        if c.reset { actions.append(WScreenAction(id: "reset", title: EncryptionCopy.resetStatusEllipsis, isEnabled: !busy, run: reset)) }
        if c.restoreCopy { actions.append(WScreenAction(id: "restore", title: EncryptionCopy.restoreCopyAction, kind: .primary, isEnabled: !busy, run: restore)) }
        if c.retry { actions.append(WScreenAction(id: "retry", title: busy ? EncryptionCopy.trying : EncryptionCopy.tryAgain, kind: .primary, isEnabled: !busy, run: retry)) }
        m.actions = actions
        return m
    }
}
