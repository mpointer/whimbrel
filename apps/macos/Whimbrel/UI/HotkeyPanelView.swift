import SwiftUI

/// The hotkey panel's content: the question field, the context and destination lines
/// (always shown before anything is sent), and the answer with Copy. Return asks.
struct HotkeyPanelView: View {
    @ObservedObject var model: HotkeyPanelModel
    /// Opens Settings → AI Providers, where a local model is installed.
    let openAIProviders: () -> Void
    /// Escape, when the field editor keeps it from reaching the panel.
    let close: () -> Void
    @State private var question = ""
    @FocusState private var fieldFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            TextField("Ask about this window", text: $question)
                .textFieldStyle(.plain)
                .font(.whimbrelBody(.title3))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .focused($fieldFocused)
                .onSubmit { submit() }

            Divider().overlay(Color.whimbrelBorder)

            if let context = model.context {
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    Text(verbatim: context.reading)
                        .foregroundStyle(Color.whimbrelTextPrimary)
                    Text(verbatim: context.destination)
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
                .font(.whimbrelBody(.caption))
                .lineLimit(2)
            }

            stateBody
        }
        .padding(Spacing.lg)
        .frame(width: HotkeyPanelController.panelWidth, alignment: .topLeading)
        .whimbrelCard()
        .onAppear { fieldFocused = true }
        .onExitCommand(perform: close)
        .onChange(of: model.state) { _, state in
            if state == .reading { question = "" }
            fieldFocused = true
        }
    }

    private func submit() {
        let text = question
        Task { await model.ask(text) }
    }

    @ViewBuilder
    private var stateBody: some View {
        switch model.state {
        case .reading:
            HStack(spacing: Spacing.xs) {
                ProgressView().controlSize(.small)
                Text("Reading this window…")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
        case .ready:
            EmptyView()
        case .blocked(let reason):
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text(verbatim: reason.message)
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelWarning)
                switch reason {
                case .notPermitted:
                    Button("Open System Settings") { NSWorkspace.shared.open(PermissionKind.accessibility.systemSettingsURL) }
                        .buttonStyle(.whimbrelSecondary)
                case .localModelMissing:
                    Button("Install a local model") { openAIProviders() }
                        .buttonStyle(.whimbrelSecondary)
                case .excluded, .unreadable:
                    EmptyView()
                }
            }
        case .answering:
            if model.partialAnswer.isEmpty {
                ProgressView().controlSize(.small)
            } else {
                answerText(model.partialAnswer)
            }
        case .answered(let answer):
            VStack(alignment: .leading, spacing: Spacing.sm) {
                answerText(answer)
                Button("Copy") { model.copyAnswer() }
                    .buttonStyle(.whimbrelSecondary)
            }
        case .failed(let message):
            Text(verbatim: message)
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelWarning)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func answerText(_ text: String) -> some View {
        ScrollView {
            Text(verbatim: text)
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxHeight: 320)
    }
}
