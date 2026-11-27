import SwiftUI

// The restyle pass (step 10, docs/ui-redesign/screens/restyle.md): Topics, Automations, the Meetings list and the capture, episode
// and thread windows keep their layout, controls and words, and take the Thread look while the redesign switch is on. The rules
// here are pure: each takes the look and, where the old screen had its own value, that old value, and hands the old value back
// unchanged for `.classic`. The app (`UI/Redesign/Restyle.swift`) feeds in the old values and draws what these return, so a test
// can say "off is the old style, on is Thread" without a view, a skin store or the real defaults.

/// Which look a screen is drawn in.
public enum WLook: Equatable, Sendable {
    /// The old look: the screen as it was before the pass. The redesign switch is off.
    case classic
    /// The Thread look: type roles, shapes, buttons and fields of the design system.
    case thread

    public init(redesign: Bool) { self = redesign ? .thread : .classic }
}

// MARK: Text

/// The old text styles these screens use, each with the Thread role that replaces it.
public enum WRestyleText: CaseIterable, Sendable {
    /// Small gray lines: dates, sources, counts (old caption, 11).
    case caption
    case captionMedium
    /// Body text of a page (old callout, 13).
    case callout
    /// A section heading inside a page (old callout, bold).
    case calloutStrong
    /// A row's own text (old body, 14).
    case body
    case bodyMedium
    /// A window or section title in the skin's display face (old title2, 17).
    case display
    /// A topic's name (old page title, 28 regular; Thread page title, 26).
    case pageTitle

    /// The Thread role for this style.
    public var role: WTextRole {
        switch self {
        case .caption, .captionMedium: return .secondary
        case .callout, .body: return .body
        case .calloutStrong, .bodyMedium: return .bodyStrong
        case .display: return .subtitle
        case .pageTitle: return .pageTitle
        }
    }
}

/// What a text style resolves to: nothing (draw the old font) or a Thread role.
public struct WTextSpec: Equatable, Sendable {
    public var role: WTextRole?
    public init(role: WTextRole? = nil) { self.role = role }
    public var isClassic: Bool { role == nil }
}

// MARK: Shapes the rules return

/// The chrome of a card: the padding inside it and its outline.
public struct WCardSpec: Equatable, Sendable {
    public var padding: CGFloat
    public var cornerRadius: CGFloat
    public var borderWidth: CGFloat
    public var dashed: Bool
    /// The old skins' double outline. Thread has none.
    public var doubled: Bool
    public init(padding: CGFloat, cornerRadius: CGFloat, borderWidth: CGFloat, dashed: Bool = false, doubled: Bool = false) {
        self.padding = padding
        self.cornerRadius = cornerRadius
        self.borderWidth = borderWidth
        self.dashed = dashed
        self.doubled = doubled
    }
}

/// How a list row shows that it is selected: a fill and a leading bar together, so it is not colour alone.
public struct WRowSpec: Equatable, Sendable {
    public enum Fill: Equatable, Sendable { case none, selection }
    public var fill: Fill
    public var leadingBar: Bool
    public init(fill: Fill, leadingBar: Bool) {
        self.fill = fill
        self.leadingBar = leadingBar
    }
}

/// The buttons these screens use.
public enum WRestyleButtonKind: CaseIterable, Sendable { case primary, secondary, quiet }

/// Which button style a button takes.
public enum WButtonSpec: Equatable, Sendable {
    /// The old pill buttons.
    case classic(WRestyleButtonKind)
    /// The Thread button of this kind.
    case thread(WRestyleButtonKind)

    public var kind: WRestyleButtonKind {
        switch self {
        case .classic(let kind), .thread(let kind): return kind
        }
    }

    public var thread: WButtonKind {
        switch kind {
        case .primary: return .primary
        case .secondary: return .secondary
        case .quiet: return .quiet
        }
    }
}

/// The state of a routine run, for the colour and the glyph beside it.
public enum WRunTone: Equatable, Sendable { case success, failure, other }

public enum WRestyle {
    /// The text spec for `text`. `.classic` is "keep the old font"; `.thread` is the role.
    public static func text(_ text: WRestyleText, look: WLook) -> WTextSpec {
        look == .classic ? WTextSpec() : WTextSpec(role: text.role)
    }

    /// The card chrome. `.classic` returns `classic` unchanged (the old padding, radius and outline, which the app reads from
    /// the selected skin); `.thread` takes the theme's card radius and outline and a 16 point padding scaled by its density.
    public static func card(look: WLook, classic: WCardSpec, theme: WTheme) -> WCardSpec {
        switch look {
        case .classic:
            return classic
        case .thread:
            return WCardSpec(
                padding: theme.space(16), cornerRadius: theme.shape.cardRadius, borderWidth: theme.borderWidth,
                dashed: theme.shape.borderStyle == .dashed && !theme.isHighContrast,
                doubled: theme.drawsDoubleBorder(width: theme.borderWidth))
        }
    }

    /// A row. Only the Thread look marks a selected row, with the selection fill and the leading bar.
    public static func row(look: WLook, selected: Bool) -> WRowSpec {
        look == .thread && selected ? WRowSpec(fill: .selection, leadingBar: true) : WRowSpec(fill: .none, leadingBar: false)
    }

    /// A button.
    public static func button(look: WLook, kind: WRestyleButtonKind) -> WButtonSpec {
        look == .classic ? .classic(kind) : .thread(kind)
    }

    /// Which theme color a run's status glyph takes: Thread uses the status colors; the old look keeps its two accent colors.
    /// Always paired with a glyph and a spoken label, never color alone.
    public static func runToneUsesStatusColor(look: WLook, tone: WRunTone) -> Bool {
        look == .thread && tone != .other
    }

    /// The glyph's size in points for a close or chevron button: the old fixed size, or the size of a type role at the text scale.
    public static func glyphSize(look: WLook, classic: CGFloat, role: WTextRole, scale: CGFloat) -> CGFloat {
        look == .classic ? classic : role.size(scale: scale)
    }
}

// MARK: Thread parts the app borrows

/// The Thread button as a style the app can apply.
public struct WThreadButtonStyle: ButtonStyle {
    let kind: WButtonKind
    public init(_ kind: WRestyleButtonKind) {
        switch kind {
        case .primary: self.kind = .primary
        case .secondary: self.kind = .secondary
        case .quiet: self.kind = .quiet
        }
    }

    public func makeBody(configuration: Configuration) -> some View {
        WButtonStyle(kind: kind, firm: true).makeBody(configuration: configuration)
    }
}

/// A text field in the Thread look: the control border, the card fill, a focus ring and the body text role. For a sheet or a page
/// where the old screen drew the system's rounded field. Pass `focus` when the page reads the focus (the field then takes it).
public struct WThreadTextField: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @FocusState private var ownFocus: Bool
    let placeholder: String
    let text: Binding<String>
    let axis: Axis
    let lineLimit: ClosedRange<Int>?
    let focus: FocusState<Bool>.Binding?
    let onSubmit: (() -> Void)?

    public init(
        _ placeholder: String, text: Binding<String>, axis: Axis = .horizontal, lineLimit: ClosedRange<Int>? = nil,
        focus: FocusState<Bool>.Binding? = nil, onSubmit: (() -> Void)? = nil
    ) {
        self.placeholder = placeholder
        self.text = text
        self.axis = axis
        self.lineLimit = lineLimit
        self.focus = focus
        self.onSubmit = onSubmit
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        let focused = focus?.wrappedValue ?? ownFocus
        field
            .textFieldStyle(.plain)
            .wFont(.body)
            .foregroundStyle(theme.textPrimary)
            .padding(.horizontal, theme.space(10))
            .padding(.vertical, theme.space(5))
            .frame(minHeight: WTokens.Size.target * k)
            .background(shape.fill(theme.backgroundCard))
            .overlay(shape.wBorder(theme.controlBorder, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 1)))
            .overlay {
                if focused { shape.strokeBorder(theme.focus, lineWidth: 2.5).padding(-1).allowsHitTesting(false).accessibilityHidden(true) }
            }
    }

    @ViewBuilder private var field: some View {
        let base = TextField(placeholder, text: text, axis: axis).onSubmit { onSubmit?() }
        let limited = Group { if let lineLimit { base.lineLimit(lineLimit) } else { base } }
        if let focus { limited.focused(focus) } else { limited.focused($ownFocus) }
    }
}

/// A menu picker in the Thread look (the choice and a chevron, the control border, a focus ring) for a place where the old screen drew
/// the system's menu picker. Opens the system menu.
public struct WThreadMenuPicker<ID: Hashable>: View {
    let label: String
    let options: [WSettingsOption<ID>]
    let selection: ID?
    let onSelect: (ID) -> Void

    public init(label: String, options: [WSettingsOption<ID>], selection: ID?, onSelect: @escaping (ID) -> Void) {
        self.label = label
        self.options = options
        self.selection = selection
        self.onSelect = onSelect
    }

    public var body: some View {
        WMenuControl(label: label, options: options.map { WSegmentOption(id: $0.id, title: $0.title) }, selection: selection, placeholder: "", onSelect: onSelect)
    }
}

/// A selected row's fill and leading bar, for a list the old screen drew with the system's selection.
public struct WThreadRowBackground: ViewModifier {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let spec: WRowSpec

    public init(_ spec: WRowSpec) { self.spec = spec }

    public func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        content
            .background(shape.fill(spec.fill == .selection ? theme.selection : .clear))
            .overlay(alignment: .leading) {
                if spec.leadingBar {
                    Capsule().fill(theme.accentPrimary).frame(width: 3 * k, height: 18 * k).padding(.leading, theme.space(3)).accessibilityHidden(true)
                }
            }
            .overlay {
                if spec.leadingBar && theme.isHighContrast { shape.strokeBorder(theme.textPrimary, lineWidth: theme.shape.highContrastBorderWidth) }
            }
    }
}

/// A card outline in the Thread look, drawn behind the content it wraps.
public struct WThreadCardBackground: ViewModifier {
    @Environment(\.wTheme) private var theme
    let spec: WCardSpec

    public init(_ spec: WCardSpec) { self.spec = spec }

    public func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: spec.cornerRadius, style: .continuous)
        content
            .padding(spec.padding)
            .background(shape.fill(theme.backgroundCard))
            .clipShape(shape)
            .overlay(shape.wBorder(theme.border, style: StrokeStyle(lineWidth: spec.borderWidth, dash: spec.dashed ? [4, 3] : [])))
            .overlay {
                if theme.isHighContrast { shape.strokeBorder(theme.textPrimary, lineWidth: theme.shape.highContrastBorderWidth) }
            }
    }
}

/// The text role at the theme's scale, for a glyph that was a fixed point size.
public struct WThreadGlyphFont: ViewModifier {
    @Environment(\.wTextScale) private var k
    let classic: CGFloat
    let weight: Font.Weight
    let role: WTextRole
    let look: WLook
    public init(classic: CGFloat, weight: Font.Weight = .regular, role: WTextRole, look: WLook) {
        self.classic = classic
        self.weight = weight
        self.role = role
        self.look = look
    }

    public func body(content: Content) -> some View {
        content.font(.system(size: WRestyle.glyphSize(look: look, classic: classic, role: role, scale: look == .classic ? 1 : k), weight: weight))
    }
}

/// A theme color the app asks for by role, so the app's restyled screens never name a hex.
public enum WRestyleColor: Sendable {
    case success, error, textSecondary, accent, link
}

extension WTheme {
    public func color(_ role: WRestyleColor) -> Color {
        switch role {
        case .success: return success
        case .error: return error
        case .textSecondary: return textSecondary
        case .accent: return accentPrimary
        case .link: return link
        }
    }
}
