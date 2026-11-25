import SwiftUI
import WhimbrelUI

/// `EncryptionKeyUnavailableView` in the redesign: the same words, Try again calls the same `perform(.retry)`.
struct KeyUnavailableRedesignView: View {
    @ObservedObject var model: EncryptionKeyUnavailableModel
    /// Quit, from the launch window. Nil leaves it out.
    var quit: (() -> Void)?

    var body: some View {
        WNoticeScreenView(model: Self.content(model, quit: quit, retry: { Task { await model.perform(.retry) } })).redesignThemed()
    }

    static func content(_ model: EncryptionKeyUnavailableModel, quit: (() -> Void)?, retry: @escaping () -> Void) -> WNoticeScreenModel {
        var m = WNoticeScreenModel()
        m.title = EncryptionCopy.unavailableTitle
        m.paragraphs = [LibraryCopyRedesign.map(EncryptionCopy.unavailableBody)]
        m.hints = [EncryptionCopy.unavailableHint, EncryptionCopy.unavailableTouchesNothing]
        if !model.detail.isEmpty { m.hints.append(EncryptionCopy.unavailableDetail(model.detail)) }
        m.errorLine = model.stillUnavailable ? EncryptionCopy.stillUnavailable : nil
        m.busy = model.retrying
        var actions: [WScreenAction] = []
        if let quit { actions.append(WScreenAction(id: "quit", title: EncryptionCopy.quitAction, isEnabled: !model.retrying, run: quit)) }
        actions.append(WScreenAction(id: "retry", title: model.retrying ? EncryptionCopy.trying : EncryptionCopy.tryAgain, kind: .primary, isEnabled: !model.retrying, run: retry))
        m.actions = actions
        return m
    }
}
