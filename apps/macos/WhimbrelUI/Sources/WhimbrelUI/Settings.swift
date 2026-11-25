import SwiftUI

// MARK: Settings row

/// A label, an optional description and a control slot. The convenience initializers cover the variants: toggle (switch
/// or checkbox), value with chevron, button and destructive.
///
/// Every row can be unavailable (`isEnabled: false`) with a `disabledReason` shown under the description, and can be
/// `highlighted` for the brief flash after a search jump. The row reflows: when the label would be squeezed, the
/// control drops under it.
public struct WSettingsRow<Control: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let label: String
    let detail: String?
    let rowAction: (() -> Void)?
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool
    var controlSpeaksLabel = false
    var leadingControl = false
    let control: Control

    public init(
        label: String, description: String? = nil, rowAction: (() -> Void)? = nil,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false,
        @ViewBuilder control: () -> Control
    ) {
        self.label = label
        self.detail = description
        self.rowAction = rowAction
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
        self.control = control()
    }

    public var body: some View {
        let inner = WSettingsRowFrame(
            label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
            controlSpeaksLabel: controlSpeaksLabel, leading: leadingControl
        ) { control }

        if let rowAction {
            // Not `.disabled`: that would dim the reason line too. A disabled row ignores the press and dims only its control.
            Button { if isEnabled { rowAction() } } label: { inner }
                .buttonStyle(.plain)
                .wFocusRing(radius: 0)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(label)
                .accessibilityHint([detail, disabledReason].compactMap { $0 }.joined(separator: " "))
        } else {
            inner
        }
    }
}

public struct WSettingsToggleControl: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let label: String
    let isOn: Bool
    let style: WToggleStyle
    let onChange: (Bool) -> Void

    init(label: String, isOn: Bool, style: WToggleStyle = .switch, onChange: @escaping (Bool) -> Void) {
        self.label = label
        self.isOn = isOn
        self.style = style
        self.onChange = onChange
    }

    public var body: some View {
        switch style {
        case .switch:
            WSwitch(label: label, isOn: isOn, onChange: onChange)
        case .checkbox:
            WCheckbox(label, isOn: isOn, showsLabel: false, onChange: onChange)
        }
    }
}

public struct WSettingsValueControl: View {
    @Environment(\.wTheme) private var theme
    let value: String
    public var body: some View {
        HStack(spacing: theme.space(6)) {
            Text(value).wFont(.body).foregroundStyle(theme.textSecondary)
            WGlyph(name: "chevron.right", size: 11, weight: .semibold).foregroundStyle(theme.textSecondary)
        }
    }
}

extension WSettingsRow where Control == WSettingsToggleControl {
    /// A toggle row. `style: .checkbox` draws a checkbox before the label (a list of independent choices);
    /// the switch stays on the trailing edge (a setting that turns something on).
    public init(
        label: String, description: String? = nil, isOn: Bool, style: WToggleStyle = .switch,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false,
        onChange: @escaping (Bool) -> Void
    ) {
        self.init(label: label, description: description, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted) {
            WSettingsToggleControl(label: label, isOn: isOn, style: style, onChange: onChange)
        }
        self.controlSpeaksLabel = true
        self.leadingControl = style == .checkbox
    }
}

extension WSettingsRow where Control == WSettingsValueControl {
    public init(label: String, description: String? = nil, value: String, isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false, action: @escaping () -> Void) {
        self.init(label: label, description: description, rowAction: action, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted) {
            WSettingsValueControl(value: value)
        }
    }
}

extension WSettingsRow where Control == WButton {
    public init(label: String, description: String? = nil, buttonTitle: String, destructive: Bool = false, isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false, action: @escaping () -> Void) {
        self.init(label: label, description: description, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted) {
            WButton(buttonTitle, kind: destructive ? .destructive : .secondary, action: action)
        }
    }
}

// MARK: Empty, loading, error

public struct WEmptyState: View {
    @Environment(\.wTheme) private var theme
    let glyph: String
    let title: String
    let help: String
    let actionTitle: String?
    let action: (() -> Void)?

    public init(glyph: String, title: String, help: String, actionTitle: String? = nil, action: (() -> Void)? = nil) {
        self.glyph = glyph
        self.title = title
        self.help = help
        self.actionTitle = actionTitle
        self.action = action
    }

    public var body: some View {
        VStack(spacing: theme.space(8)) {
            WGlyph(name: glyph, size: 28, weight: .light).foregroundStyle(theme.textSecondary)
            Text(title).wFont(.subtitle).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
            Text(help).wFont(.body).foregroundStyle(theme.textSecondary).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
            if let actionTitle, let action {
                WButton(actionTitle, kind: .secondary, action: action).padding(.top, theme.space(4))
            }
        }
        .padding(theme.space(24))
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .contain)
    }
}

/// A quiet bar. No spinner. With Reduce Motion it stays still.
public struct WLoadingState: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var dim = false
    let title: String?
    let help: String?

    public init(title: String? = nil, help: String? = nil) {
        self.title = title
        self.help = help
    }

    public var body: some View {
        let title = title ?? strings.loading
        return VStack(spacing: theme.space(10)) {
            Text(title).wFont(.subtitle).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
            if let help { Text(help).wFont(.body).foregroundStyle(theme.textSecondary) }
            Capsule()
                .fill(theme.border.opacity(theme.isHighContrast ? 1 : 0.6))
                .frame(width: 160 * k, height: 4 * k)
                .overlay(alignment: .leading) {
                    Capsule().fill(theme.accentPrimary).frame(width: 64 * k, height: 4 * k).opacity(dim ? 0.45 : 1)
                }
                .accessibilityHidden(true)
        }
        .padding(theme.space(24))
        .frame(maxWidth: .infinity)
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.easeInOut(duration: WTokens.Motion.slow * 3).repeatForever(autoreverses: true)) { dim = true }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
    }
}

public struct WErrorState: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let title: String
    /// A line under the title; nil or empty draws nothing.
    let help: String?
    let onRetry: () -> Void

    public init(title: String, help: String? = nil, onRetry: @escaping () -> Void) {
        self.title = title
        self.help = help
        self.onRetry = onRetry
    }

    public var body: some View {
        VStack(spacing: theme.space(8)) {
            WGlyph(name: "exclamationmark.triangle", size: 26, weight: .regular).foregroundStyle(theme.error)
            Text(title).wFont(.subtitle).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
            if let help, !help.isEmpty {
                Text(help).wFont(.body).foregroundStyle(theme.textSecondary).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
            }
            WButton(strings.tryAgain, glyph: "arrow.clockwise", kind: .secondary, action: onRetry).padding(.top, theme.space(4))
        }
        .padding(theme.space(24))
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .contain)
    }
}
