import SwiftUI

// MARK: Settings row

/// A label, an optional description and a control slot. The convenience initializers cover the four
/// variants: toggle, value with chevron, button and destructive.
public struct WSettingsRow<Control: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let label: String
    let detail: String?
    let rowAction: (() -> Void)?
    let control: Control

    public init(label: String, description: String? = nil, rowAction: (() -> Void)? = nil, @ViewBuilder control: () -> Control) {
        self.label = label
        self.detail = description
        self.rowAction = rowAction
        self.control = control()
    }

    public var body: some View {
        let inner = HStack(alignment: .center, spacing: theme.space(16)) {
            VStack(alignment: .leading, spacing: theme.space(2)) {
                Text(label).wFont(.body).foregroundStyle(theme.textPrimary)
                if let detail {
                    Text(detail).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: theme.space(8))
            control
        }
        .padding(.horizontal, theme.space(14))
        .padding(.vertical, theme.space(10))
        .frame(minHeight: 44 * k)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())

        if let rowAction {
            Button(action: rowAction) { inner }
                .buttonStyle(.plain)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(label)
                .accessibilityHint(detail ?? "")
        } else {
            inner.accessibilityElement(children: .contain)
        }
    }
}

public struct WSettingsToggleControl: View {
    @Environment(\.wTheme) private var theme
    let label: String
    let isOn: Bool
    let onChange: (Bool) -> Void
    public var body: some View {
        Toggle(label, isOn: Binding(get: { isOn }, set: onChange))
            .labelsHidden()
            .toggleStyle(.switch)
            .tint(theme.accentPrimary)
            .accessibilityLabel(label)
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
    public init(label: String, description: String? = nil, isOn: Bool, onChange: @escaping (Bool) -> Void) {
        self.init(label: label, description: description) {
            WSettingsToggleControl(label: label, isOn: isOn, onChange: onChange)
        }
    }
}

extension WSettingsRow where Control == WSettingsValueControl {
    public init(label: String, description: String? = nil, value: String, action: @escaping () -> Void) {
        self.init(label: label, description: description, rowAction: action) {
            WSettingsValueControl(value: value)
        }
    }
}

extension WSettingsRow where Control == WButton {
    public init(label: String, description: String? = nil, buttonTitle: String, destructive: Bool = false, action: @escaping () -> Void) {
        self.init(label: label, description: description) {
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
    let help: String
    let onRetry: () -> Void

    public init(title: String, help: String, onRetry: @escaping () -> Void) {
        self.title = title
        self.help = help
        self.onRetry = onRetry
    }

    public var body: some View {
        VStack(spacing: theme.space(8)) {
            WGlyph(name: "exclamationmark.triangle", size: 26, weight: .regular).foregroundStyle(theme.error)
            Text(title).wFont(.subtitle).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
            Text(help).wFont(.body).foregroundStyle(theme.textSecondary).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
            WButton(strings.tryAgain, glyph: "arrow.clockwise", kind: .secondary, action: onRetry).padding(.top, theme.space(4))
        }
        .padding(theme.space(24))
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .contain)
    }
}
