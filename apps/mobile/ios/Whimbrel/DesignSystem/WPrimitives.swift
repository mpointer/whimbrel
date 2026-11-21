import SwiftUI

private struct WStringsKey: EnvironmentKey { static let defaultValue = WStrings.english }

extension EnvironmentValues {
    var wStrings: WStrings {
        get { self[WStringsKey.self] }
        set { self[WStringsKey.self] = newValue }
    }
}

// MARK: - Surfaces

/// A white card on the page. In dark mode and with Increase Contrast a border stands in for the shadow.
private struct WCardModifier: ViewModifier {
    @Environment(\.colorScheme) private var scheme
    @Environment(\.colorSchemeContrast) private var contrast
    let level: WTokens.Shadow
    let tint: Color?

    func body(content: Content) -> some View {
        let flat = scheme == .dark || contrast == .increased
        content
            .background(tint ?? Color.theme(.backgroundCard), in: RoundedRectangle(cornerRadius: WTokens.Radius.card, style: .continuous))
            .overlay {
                if flat {
                    RoundedRectangle(cornerRadius: WTokens.Radius.card, style: .continuous)
                        .strokeBorder(Color.theme(.border), lineWidth: contrast == .increased ? 2 : 1)
                }
            }
            .shadow(color: flat ? .clear : WFixed.scrim.color.opacity(level.opacity), radius: level.blur / 2, y: level.y)
    }
}

extension View {
    func wCard(level: WTokens.Shadow = WTokens.Elevation.level1, tint: Color? = nil) -> some View {
        modifier(WCardModifier(level: level, tint: tint))
    }
}

// MARK: - Buttons

enum WButtonKind { case primary, secondary, quiet, destructive, recording }

struct WButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    @Environment(\.colorSchemeContrast) private var contrast
    let kind: WButtonKind
    var fill = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .wType(WTokens.TypeScale.bodyStrong)
            .multilineTextAlignment(.center)
            .padding(.horizontal, WTokens.Space.lg)
            .padding(.vertical, WTokens.Space.sm)
            .frame(maxWidth: fill ? .infinity : nil, minHeight: WTokens.Size.target)
            .foregroundStyle(foreground)
            .background(background, in: RoundedRectangle(cornerRadius: WTokens.Radius.control, style: .continuous))
            .overlay {
                if kind == .secondary || contrast == .increased {
                    RoundedRectangle(cornerRadius: WTokens.Radius.control, style: .continuous).strokeBorder(Color.theme(.border), lineWidth: 1)
                }
            }
            .opacity(enabled ? (configuration.isPressed ? 0.7 : 1) : 0.45)
            .contentShape(Rectangle())
    }

    private var foreground: Color {
        switch kind {
        case .primary: Color.theme(.textOnAccent)
        case .recording: WFixed.onRecording.color
        case .secondary: Color.theme(.textPrimary)
        case .quiet: Color.theme(.accentPrimary)
        case .destructive: Color.theme(.error)
        }
    }

    private var background: Color {
        switch kind {
        case .primary: Color.theme(.accentPrimary)
        case .recording: WFixed.recording.color
        case .secondary: Color.theme(.backgroundCard)
        case .quiet, .destructive: .clear
        }
    }
}

extension ButtonStyle where Self == WButtonStyle {
    static func w(_ kind: WButtonKind, fill: Bool = false) -> WButtonStyle { WButtonStyle(kind: kind, fill: fill) }
}

// MARK: - Chips

/// Initials avatar and a name.
struct WSpeakerChip: View {
    let name: String

    static func initials(_ name: String) -> String {
        let parts = name.split(separator: " ").prefix(2)
        return parts.compactMap { $0.first.map(String.init) }.joined().uppercased()
    }

    var body: some View {
        HStack(spacing: WTokens.Space.xs) {
            Text(Self.initials(name))
                .font(.system(size: 10, weight: .bold))
                .frame(width: 22, height: 22)
                .background(Color.theme(.border), in: Circle())
                .accessibilityHidden(true)
            Text(name).wType(WTokens.TypeScale.secondary).fontWeight(.semibold)
        }
        .padding(.trailing, WTokens.Space.sm)
        .padding(WTokens.Space.xxs)
        .foregroundStyle(Color.theme(.textPrimary))
        .background(Color.theme(.backgroundPage), in: Capsule())
    }
}

/// A selectable filter (People, Apps, Dates). Selected is shown by a check mark as well as color.
struct WFilterChip: View {
    let label: String
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: WTokens.Space.xs) {
                if selected { Image(systemName: "checkmark").font(.caption.weight(.bold)) }
                Text(label).wType(WTokens.TypeScale.secondary).fontWeight(.semibold)
            }
            .padding(.horizontal, WTokens.Space.md)
            .frame(minHeight: WTokens.Size.target)
            .foregroundStyle(Color.theme(.textPrimary))
            .background(selected ? Color.theme(.selection) : Color.theme(.backgroundCard), in: Capsule())
            .overlay(Capsule().strokeBorder(selected ? Color.theme(.accentPrimary) : Color.theme(.border), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// The search field as a button: tapping it opens search. Typing happens on the search screen.
struct WSearchBar: View {
    @Environment(\.wStrings) private var strings
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: WTokens.Space.sm) {
                Image(systemName: "magnifyingglass").accessibilityHidden(true)
                Text(strings.searchPlaceholder).wType(WTokens.TypeScale.body)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, WTokens.Space.lg)
            .frame(minHeight: WTokens.Size.target)
            .foregroundStyle(Color.theme(.textSecondary))
            .background(Color.theme(.backgroundCard), in: Capsule())
            .overlay(Capsule().strokeBorder(Color.theme(.border), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(.isSearchField)
    }
}

// MARK: - States

/// Empty, loading and error share one shape: a glyph, a one-line title, one line of help, an optional action.
struct WStateView: View {
    enum Kind { case empty, loading, error }
    @Environment(\.wStrings) private var strings
    let kind: Kind
    let title: String
    var help: String?
    var actionTitle: String?
    var action: (() -> Void)?

    var body: some View {
        VStack(spacing: WTokens.Space.md) {
            switch kind {
            case .loading: ProgressView().controlSize(.regular)
            case .empty: Image(systemName: "tray").font(.title).foregroundStyle(Color.theme(.textSecondary)).accessibilityHidden(true)
            case .error: Image(systemName: "exclamationmark.triangle").font(.title).foregroundStyle(Color.theme(.error)).accessibilityHidden(true)
            }
            Text(title).wType(WTokens.TypeScale.subtitle).foregroundStyle(Color.theme(.textPrimary))
            if let help { Text(help).wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary)) }
            if let action {
                Button(actionTitle ?? strings.tryAgain, action: action).buttonStyle(.w(.secondary))
            }
        }
        .multilineTextAlignment(.center)
        .padding(WTokens.Space.xl)
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .combine)
    }
}

/// A line across the top of a screen when there is no connection. Synced content stays usable below it.
struct WOfflineBanner: View {
    @Environment(\.wStrings) private var strings

    var body: some View {
        Label(strings.offline, systemImage: "wifi.slash")
            .wType(WTokens.TypeScale.secondary)
            .foregroundStyle(Color.theme(.textPrimary))
            .padding(WTokens.Space.md)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.theme(.highlight), in: RoundedRectangle(cornerRadius: WTokens.Radius.control, style: .continuous))
    }
}
