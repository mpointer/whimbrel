import SwiftUI

// Small parts Home and its element choices share: text links, the action that is a button or a link,
// and the section label in its three styles. They read the skin's choice from the environment, so a
// component like `WMeetingCard` follows it without a new parameter at each call.

extension EnvironmentValues {
    /// Whether actions draw as buttons or as text links (`today.options.actions`).
    @Entry public var wActionStyle: WActionStyle = .buttons
    /// How section labels are drawn (`today.options.labels`).
    @Entry public var wLabelStyle: WLabelStyle = .plain
}

/// An action drawn as text: the link color on the page, underlined, with a hit area of at least 24 by 24.
public struct WTextLink: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.isEnabled) private var isEnabled
    let title: String
    let action: () -> Void

    /// The smallest click area, in points before the text scale.
    public static let minimumTarget: CGFloat = 24

    public init(_ title: String, action: @escaping () -> Void) {
        self.title = title
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            Text(title)
                .wFont(.bodyStrong)
                .underline()
                .foregroundStyle(theme.link)
                .lineLimit(1)
                .frame(minWidth: Self.minimumTarget * k, minHeight: Self.minimumTarget * k)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .opacity(isEnabled ? 1 : 0.45)
        .accessibilityLabel(title)
        .accessibilityAddTraits(.isLink)
    }
}

/// A button or a text link, by the skin's choice. Same name, same action either way.
struct WAction: View {
    @Environment(\.wActionStyle) private var style
    let title: String
    let glyph: String?
    let shortcutHint: String?
    let kind: WButtonKind
    let spokenName: String?
    let action: () -> Void

    init(_ title: String, glyph: String? = nil, shortcutHint: String? = nil, kind: WButtonKind = .secondary, spokenName: String? = nil, action: @escaping () -> Void) {
        self.title = title
        self.glyph = glyph
        self.shortcutHint = shortcutHint
        self.kind = kind
        self.spokenName = spokenName
        self.action = action
    }

    var body: some View {
        Group {
            if style == .links {
                WTextLink(title, action: action)
            } else {
                WButton(title, glyph: glyph, shortcutHint: shortcutHint, kind: kind, action: action)
            }
        }
        .accessibilityLabel(spokenName ?? title)
    }
}

/// A label over a part of Home: plain (the skin's own case), bracketed with a rule, or small caps.
public struct WSectionLabel: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wLabelStyle) private var style
    let text: String

    public init(_ text: String) { self.text = text }

    public var body: some View {
        Group {
            switch style {
            case .plain:
                Text(theme.sectionLabel(text))
                    .wSectionLabel()
                    .foregroundStyle(theme.textSecondary)
            case .bracketed:
                HStack(spacing: theme.space(8)) {
                    Text("[").wFont(.secondary).foregroundStyle(theme.textSecondary)
                    Text(text.uppercased()).wSectionLabel(caps: .upper).foregroundStyle(theme.textPrimary)
                    Text("]").wFont(.secondary).foregroundStyle(theme.textSecondary)
                    Rectangle().fill(theme.border).frame(height: max(1, theme.borderWidth)).frame(maxWidth: .infinity)
                }
            case .smallCaps:
                VStack(alignment: .leading, spacing: theme.space(4)) {
                    Text(text)
                        .font(WTextRole.subtitle.font(scale: k, style: theme.type).smallCaps())
                        .foregroundStyle(theme.textPrimary)
                    Rectangle().fill(theme.border).frame(height: max(1, theme.borderWidth))
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(text)
        .accessibilityAddTraits(.isHeader)
    }
}
