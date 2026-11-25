import SwiftUI
import WhimbrelUI

/// The hotkey panel's state as the plain values the redesigned panel draws. Pure: the reading line and the destination
/// line come through unchanged from the model, and are shown whenever it has them, before anything is sent.
enum HotkeyPanelAdapter {
    static let accessibilityActionId = "open-accessibility"
    static let localModelActionId = "install-local-model"

    static func model(state: HotkeyPanelModel.State, context: HotkeyPanelModel.ContextLine?, partialAnswer: String) -> WHotkeyModel {
        let phase: WHotkeyPhase
        switch state {
        case .reading: phase = .reading
        case .ready: phase = .ready
        case .blocked(let reason): phase = .blocked(message: reason.message, action: action(for: reason))
        case .answering: phase = .answering(partial: partialAnswer)
        case .answered(let answer): phase = .answered(answer)
        case .failed(let message): phase = .failed(message)
        }
        return WHotkeyModel(
            phase: phase, readingLine: context?.reading, destinationLine: context?.destination,
            readingProgress: String(localized: "Reading this window…"),
            copyTitle: String(localized: "Copy"),
            // Its own key: "Copied" is the recording notice's word, with its own comment.
            copiedTitle: String(localized: "hotkey.copied", defaultValue: "Copied", comment: "Shown on the Copy button of the Ask about this window panel after the answer was copied."))
    }

    /// The button a blocked state offers, as before: System Settings for the permission, the local model install for the
    /// two that need one. The other two have none.
    static func action(for reason: HotkeyPanelModel.Reason) -> WHotkeyAction? {
        switch reason {
        case .notPermitted: return WHotkeyAction(id: accessibilityActionId, title: String(localized: "Open System Settings"))
        case .localModelMissing, .clientNeedsLocalModel: return WHotkeyAction(id: localModelActionId, title: String(localized: "Install a local model"))
        case .excluded, .unreadable: return nil
        }
    }
}

/// `HotkeyPanelView` in the redesign: the same model, the same asks, the same blocked buttons.
struct HotkeyPanelRedesignView: View {
    @ObservedObject var model: HotkeyPanelModel
    /// Opens Settings, AI Providers, where a local model is installed.
    let openAIProviders: () -> Void
    /// Escape, when the field editor keeps it from reaching the panel.
    let close: () -> Void
    @State private var question = ""

    var body: some View {
        WHotkeyPanel(
            model: HotkeyPanelAdapter.model(state: model.state, context: model.context, partialAnswer: model.partialAnswer),
            question: $question,
            actions: WHotkeyActions(
                submit: { text in Task { await model.ask(text) } },
                close: close,
                copy: { model.copyAnswer() },
                perform: { id in
                    switch id {
                    case HotkeyPanelAdapter.accessibilityActionId: NSWorkspace.shared.open(PermissionKind.accessibility.systemSettingsURL)
                    case HotkeyPanelAdapter.localModelActionId: openAIProviders()
                    default: break
                    }
                },
                questionChanged: { model.questionChanged($0) }),
            width: HotkeyPanelController.panelWidth)
            // Room for the card's shadow inside the panel.
            .padding(14)
            .redesignThemed()
            .onExitCommand(perform: close)
            .onChange(of: model.state) { _, state in
                if state == .reading { question = "" }
            }
    }
}
