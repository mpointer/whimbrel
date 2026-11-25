import SwiftUI

// The hotkey panel, "Ask about this window" (docs/ui-redesign/screens/menu-bar.md, "Hotkey panel"). Restyled only: the
// field, the line saying what is read, the line saying where the text will be sent (shown before anything is sent), the
// blocked states with their buttons, the streamed answer and Copy. Return asks; the app closes the panel on Escape.

public struct WHotkeyPanel: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @FocusState private var fieldFocused: Bool
    @State private var copied = false
    let model: WHotkeyModel
    @Binding var question: String
    let actions: WHotkeyActions
    let width: CGFloat

    public init(model: WHotkeyModel, question: Binding<String>, actions: WHotkeyActions, width: CGFloat = 520) {
        self.model = model
        self._question = question
        self.actions = actions
        self.width = width
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            fieldRow
            Rectangle().fill(theme.border.opacity(theme.isHighContrast ? 1 : 0.7)).frame(height: max(1, theme.borderWidth)).accessibilityHidden(true)
            if model.readingLine != nil || model.destinationLine != nil { contextLines }
            stateBody
        }
        .padding(theme.space(16))
        .frame(width: width * min(k, 1.6), alignment: .topLeading)
        .wCard(radius: .surface, elevation: WTokens.Elevation.level2)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.menu.hotkeyWindowName)
        .onAppear { fieldFocused = true }
        // Keyed on the kind of phase, not the phase: a streamed answer changes its text many times and the field must
        // not take the keyboard back on each one.
        .onChange(of: model.phase.kind) { _, kind in
            fieldFocused = true
            if kind != .answered { copied = false }
        }
    }

    private var fieldRow: some View {
        HStack(spacing: theme.space(10)) {
            WGlyph(name: "sparkles", size: 15, weight: .medium).foregroundStyle(theme.accentPrimary)
            // The placeholder is drawn here, in the theme's own secondary ink, so it holds its contrast in every palette.
            ZStack(alignment: .leading) {
                if question.isEmpty {
                    Text(strings.menu.askAboutWindow).wFont(.title).foregroundStyle(theme.textSecondary)
                        .allowsHitTesting(false).accessibilityHidden(true)
                }
                TextField("", text: $question)
                    .accessibilityLabel(strings.menu.askAboutWindow)
                    .textFieldStyle(.plain)
                    .wFont(.title)
                    .foregroundStyle(theme.textPrimary)
                    .focused($fieldFocused)
                    .onSubmit { actions.submit(question) }
                    .onChange(of: question) { _, text in actions.questionChanged(text) }
            }
            Button(action: actions.close) {
                WGlyph(name: "xmark", size: 11, weight: .semibold).foregroundStyle(theme.textSecondary)
                    .frame(width: WTokens.Size.target * k, height: WTokens.Size.target * k)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(strings.menu.close)
            .accessibilityLabel(strings.menu.close)
        }
    }

    /// What is read and where it goes. Both are shown before anything is sent, with a glyph each so neither rests on color.
    private var contextLines: some View {
        VStack(alignment: .leading, spacing: theme.space(4)) {
            if let reading = model.readingLine {
                HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                    WGlyph(name: "doc.text", size: 11).foregroundStyle(theme.textSecondary).frame(width: 16 * k)
                    Text(verbatim: reading).wFont(.secondary).foregroundStyle(theme.textPrimary).lineLimit(2)
                }
                .accessibilityElement(children: .combine)
            }
            if let destination = model.destinationLine {
                HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                    WGlyph(name: "paperplane", size: 11).foregroundStyle(theme.textSecondary).frame(width: 16 * k)
                    Text(verbatim: destination).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(2)
                }
                .accessibilityElement(children: .combine)
            }
        }
    }

    @ViewBuilder
    private var stateBody: some View {
        switch model.phase {
        case .reading:
            HStack(spacing: theme.space(8)) {
                ProgressView().controlSize(.small)
                Text(verbatim: model.readingProgress).wFont(.secondary).foregroundStyle(theme.textSecondary)
            }
        case .ready:
            EmptyView()
        case .blocked(let message, let action):
            VStack(alignment: .leading, spacing: theme.space(8)) {
                WWarningLine(text: message)
                if let action {
                    WSpokenButton(title: action.title, kind: .secondary) { actions.perform(action.id) }
                }
            }
        case .answering(let partial):
            if partial.isEmpty {
                ProgressView().controlSize(.small).accessibilityLabel(strings.menu.answering)
            } else {
                answerText(partial)
            }
        case .answered(let answer):
            VStack(alignment: .leading, spacing: theme.space(10)) {
                answerText(answer)
                WSpokenButton(title: copied ? model.copiedTitle : model.copyTitle, spokenName: copied ? model.copiedTitle : model.copyTitle, glyph: copied ? "checkmark" : "doc.on.doc") {
                    actions.copy()
                    copied = true
                    AccessibilityNotification.Announcement(model.copiedTitle).post()
                }
            }
        case .failed(let message):
            WErrorLine(text: message)
        }
    }

    private func answerText(_ text: String) -> some View {
        ScrollView {
            Text(verbatim: text)
                .wFont(.body)
                .foregroundStyle(theme.textPrimary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxHeight: 320 * k)
    }
}
