import SwiftUI

// The small floating windows (docs/ui-redesign/screens/menu-bar.md, "Small windows"): one compact frame with the app
// mark and name, a title, one or two lines and an action row with the main action first. The windows are hosted in a
// panel by the app; this file draws what is inside.

/// The frame: a solid card (never glass, since it holds text), the app mark and name above the title.
public struct WPanelFrame<Content: View, Actions: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let width: CGFloat
    let mark: Image?
    let title: String
    let content: Content
    let actions: Actions

    public init(width: CGFloat, mark: Image? = nil, title: String, @ViewBuilder content: () -> Content, @ViewBuilder actions: () -> Actions) {
        self.width = width
        self.mark = mark
        self.title = title
        self.content = content()
        self.actions = actions()
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            HStack(spacing: theme.space(8)) {
                markView
                Text(strings.menu.appName).wFont(.secondary).fontWeight(.semibold).foregroundStyle(theme.textSecondary)
                Spacer(minLength: 0)
            }
            .accessibilityHidden(true)
            Text(title).wFont(.subtitle).foregroundStyle(theme.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            content
            actions
        }
        .padding(theme.space(16))
        .frame(width: width * min(k, 1.6), alignment: .leading)
        .wCard(radius: .surface, elevation: WTokens.Elevation.level2)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.menu.alertWindowName)
    }

    @ViewBuilder
    private var markView: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        Group {
            if let mark {
                mark.resizable().scaledToFit().clipShape(shape)
            } else {
                shape.fill(theme.accentPrimary).overlay(WGlyph(name: "waveform", size: 12, weight: .bold).foregroundStyle(theme.textOnAccent))
            }
        }
        .frame(width: 22 * k, height: 22 * k)
    }
}

/// The meeting detected window and the check-in window: the same frame, a model each. Return runs the main action only when
/// that action is marked safe (`WAlertAction.isSafeDefault`); Escape is the app's.
public struct WAlertPanel: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let model: WAlertPanelModel
    let width: CGFloat
    let mark: Image?
    /// The app the meeting runs in, drawn beside the subject. Nil draws nothing.
    let subjectIcon: Image?

    public init(model: WAlertPanelModel, width: CGFloat = 440, mark: Image? = nil, subjectIcon: Image? = nil) {
        self.model = model
        self.width = width
        self.mark = mark
        self.subjectIcon = subjectIcon
    }

    public var body: some View {
        WPanelFrame(width: width, mark: mark, title: model.title) {
            VStack(alignment: .leading, spacing: theme.space(6)) {
                if let subject = model.subject {
                    HStack(spacing: theme.space(8)) {
                        if let subjectIcon { subjectIcon.resizable().scaledToFit().frame(width: 20 * k, height: 20 * k).accessibilityHidden(true) }
                        Text(subject).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).lineLimit(2).fixedSize(horizontal: false, vertical: true)
                    }
                }
                ForEach(Array(model.lines.enumerated()), id: \.offset) { _, line in
                    Text(line).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
                if let countdown = model.countdown { countdownRow(countdown) }
            }
        } actions: {
            actionRow
        }
    }

    private func countdownRow(_ countdown: WAlertCountdown) -> some View {
        HStack(spacing: theme.space(8)) {
            WGlyph(name: "timer", size: 12, weight: .semibold).foregroundStyle(theme.textPrimary)
            Text(countdown.text).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).monospacedDigit()
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: theme.space(6))
            if let cancel = countdown.cancel {
                WSpokenButton(title: cancel.title, spokenName: cancel.spokenName, kind: .quiet, action: cancel.run)
            }
        }
        .padding(.vertical, theme.space(2))
    }

    /// The main button. Return runs it from anywhere in the window only when it is safe to run by accident (Keep recording);
    /// a button that starts, resumes or stops a recording, or changes how one is kept, runs on Return only when it has focus.
    @ViewBuilder
    private var mainButton: some View {
        let button = WSpokenButton(title: model.main.title, spokenName: model.main.spokenName, glyph: model.main.glyph, kind: .primary, help: model.main.help, action: model.main.run)
        if model.main.isSafeDefault {
            button.keyboardShortcut(.defaultAction)
        } else {
            button
        }
    }

    /// The main action first, then the others. The buttons wrap onto a second line instead of squeezing the words.
    private var actionRow: some View {
        WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
            HStack(spacing: theme.space(4)) {
                mainButton
                if let alternate = model.alternate {
                    Menu {
                        Button(alternate.title, action: alternate.run)
                    } label: {
                        WGlyph(name: "chevron.down", size: 11, weight: .semibold).foregroundStyle(theme.textPrimary)
                            .frame(width: WTokens.Size.target * k, height: WTokens.Size.target * k)
                            .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.backgroundCard))
                            .overlay(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).strokeBorder(theme.firmEdge, lineWidth: max(1, theme.borderWidth)))
                            .contentShape(Rectangle())
                    }
                    .menuStyle(.button)
                    .buttonStyle(.plain)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .accessibilityLabel(alternate.spokenName ?? alternate.title)
                }
            }
            ForEach(model.others) { action in
                WSpokenButton(title: action.title, spokenName: action.spokenName, glyph: action.glyph, kind: .secondary, help: action.help, action: action.run)
            }
        }
    }
}
