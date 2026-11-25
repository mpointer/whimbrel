import SwiftUI
import WhimbrelUI

// The restyle pass (step 10, docs/ui-redesign/screens/restyle.md) on the Mac: Topics, Automations, the Meetings list and the capture,
// episode and thread windows keep their layout, controls and words and take the Thread look while the hidden redesign switch is on.
//
// One mechanism. A restyled screen's root calls `.restyledScreen(redesign:)`, which with the switch on gives it the app's theme
// (`redesignThemed`) and the `threadLook` environment value, and with it off returns the view untouched. Everything below reads
// `threadLook` and, with it `.classic` (the default, and the only value anywhere with the switch off), draws exactly what the old
// code drew. The rules (which role, which radius, which button) are pure and live in the WhimbrelUI package (`WRestyle`), where
// they are tested.

private struct ThreadLookKey: EnvironmentKey {
    static let defaultValue = WLook.classic
}

extension EnvironmentValues {
    /// `.thread` inside a restyled screen with the redesign switch on; `.classic` everywhere else.
    var threadLook: WLook {
        get { self[ThreadLookKey.self] }
        set { self[ThreadLookKey.self] = newValue }
    }
}

private struct RestyledScreenRoot: ViewModifier {
    let redesign: Bool

    @ViewBuilder func body(content: Content) -> some View {
        if redesign {
            content.redesignThemed().environment(\.threadLook, .thread)
        } else {
            content
        }
    }
}

extension View {
    /// The root of a screen that takes the Thread look. Pass `appState.redesignEnabled`. With the switch off this is the view itself.
    func restyledScreen(redesign: Bool) -> some View { modifier(RestyledScreenRoot(redesign: redesign)) }
}

// MARK: Text

extension WRestyleText {
    /// The font the screen had before the pass.
    var classicFont: Font {
        switch self {
        case .caption: return .whimbrelBody(.caption)
        case .captionMedium: return .whimbrelBody(.caption).weight(.medium)
        case .callout: return .whimbrelBody(.callout)
        case .calloutStrong: return .whimbrelBody(.callout).bold()
        case .body: return .whimbrelBody(.body)
        case .bodyMedium: return .whimbrelBody(.body).weight(.medium)
        case .display: return .whimbrelDisplay()
        case .pageTitle: return .whimbrelPageTitle
        }
    }
}

private struct RestyledFont: ViewModifier {
    @Environment(\.threadLook) private var look
    let text: WRestyleText

    @ViewBuilder func body(content: Content) -> some View {
        if let role = WRestyle.text(text, look: look).role {
            content.wFont(role)
        } else {
            content.font(text.classicFont)
        }
    }
}

extension View {
    /// `.font(old)` for a screen that takes the Thread type roles: the old font off, the role on.
    func restyledFont(_ text: WRestyleText) -> some View { modifier(RestyledFont(text: text)) }

    /// A fixed-size glyph (a close button, a chevron): the old size off, the size of a type role on.
    func restyledGlyph(classicSize: CGFloat, weight: Font.Weight = .regular, role: WTextRole = .title) -> some View {
        modifier(RestyledGlyph(classicSize: classicSize, weight: weight, role: role))
    }
}

private struct RestyledGlyph: ViewModifier {
    @Environment(\.threadLook) private var look
    let classicSize: CGFloat
    let weight: Font.Weight
    let role: WTextRole

    func body(content: Content) -> some View {
        content.modifier(WThreadGlyphFont(classic: classicSize, weight: weight, role: role, look: look))
    }
}

// MARK: Card, rows, buttons, fields

private struct RestyledCard: ViewModifier {
    @Environment(\.threadLook) private var look
    @Environment(\.wTheme) private var theme

    @ViewBuilder func body(content: Content) -> some View {
        if look == .thread {
            content.modifier(WThreadCardBackground(WRestyle.card(look: look, classic: Self.classicSpec, theme: theme)))
        } else {
            content.whimbrelCard()
        }
    }

    /// What `.whimbrelCard()` draws, read from the selected skin the same way.
    private static var classicSpec: WCardSpec {
        let shape = SkinStore.shared.selected.shape
        return WCardSpec(padding: Spacing.lg * shape.density.scale, cornerRadius: shape.radius, borderWidth: shape.borderWidth)
    }
}

private struct RestyledRowBackground: ViewModifier {
    @Environment(\.threadLook) private var look
    let selected: Bool

    @ViewBuilder func body(content: Content) -> some View {
        if look == .thread {
            content.modifier(WThreadRowBackground(WRestyle.row(look: look, selected: selected)))
        } else {
            content
        }
    }
}

/// A list row's background in a `List`: the selection fill and leading bar with the Thread look, the system's with it off.
private struct RestyledListRow: ViewModifier {
    @Environment(\.threadLook) private var look
    let selected: Bool

    @ViewBuilder func body(content: Content) -> some View {
        if look == .thread {
            content.listRowBackground(Color.clear.modifier(WThreadRowBackground(WRestyle.row(look: look, selected: selected))))
        } else {
            content
        }
    }
}

private struct RestyledButton: ViewModifier {
    @Environment(\.threadLook) private var look
    let kind: WRestyleButtonKind

    @ViewBuilder func body(content: Content) -> some View {
        switch WRestyle.button(look: look, kind: kind) {
        case .classic(let kind):
            switch kind {
            case .primary: content.buttonStyle(.whimbrelPrimary)
            case .secondary: content.buttonStyle(.whimbrelSecondary)
            case .quiet: content.buttonStyle(.whimbrelQuiet)
            }
        case .thread(let kind):
            content.buttonStyle(WThreadButtonStyle(kind))
        }
    }
}

extension View {
    /// `.whimbrelCard()` with the Thread card on.
    func restyledCard() -> some View { modifier(RestyledCard()) }

    /// A row that shows it is selected by a fill and a leading bar (Thread look only).
    func restyledRowBackground(selected: Bool) -> some View { modifier(RestyledRowBackground(selected: selected)) }

    /// A `List` row's background (Thread look only).
    func restyledListRow(selected: Bool) -> some View { modifier(RestyledListRow(selected: selected)) }

    /// `.buttonStyle(.whimbrelPrimary / .whimbrelSecondary / .whimbrelQuiet)` with the Thread button on.
    func restyledButton(_ kind: WRestyleButtonKind) -> some View { modifier(RestyledButton(kind: kind)) }
}

/// A single line or multi-line text field: the system's rounded field with the switch off, the Thread field (control border, focus
/// ring) with it on. Same label, same binding, same limits.
struct RestyledTextField: View {
    @Environment(\.threadLook) private var look
    let placeholder: LocalizedStringResource
    @Binding var text: String
    var axis: Axis = .horizontal
    var lineLimit: ClosedRange<Int>? = nil
    var focus: FocusState<Bool>.Binding? = nil
    var onSubmit: (() -> Void)? = nil

    var body: some View {
        if look == .thread {
            WThreadTextField(String(localized: placeholder), text: $text, axis: axis, lineLimit: lineLimit, focus: focus, onSubmit: onSubmit)
        } else {
            classic
        }
    }

    /// Built from the same calls the old screens made, in the same order; `onSubmit` is only attached when given.
    @ViewBuilder private var classic: some View {
        let field = TextField(String(localized: placeholder), text: $text, axis: axis).textFieldStyle(.roundedBorder)
        let limited = Group { if let lineLimit { field.lineLimit(lineLimit) } else { field } }
        let focused = Group { if let focus { limited.focused(focus) } else { limited } }
        if let onSubmit { focused.onSubmit(onSubmit) } else { focused }
    }
}

/// A list section: the system's titled section with the switch off; with it on, a header in the theme's section label style.
struct RestyledSection<Content: View>: View {
    @Environment(\.threadLook) private var look
    @Environment(\.wTheme) private var theme
    let title: String
    @ViewBuilder let content: () -> Content

    @ViewBuilder var body: some View {
        if look == .thread {
            Section {
                content()
            } header: {
                Text(theme.sectionLabel(title)).wFont(.caption).fontWeight(.semibold).foregroundStyle(theme.textSecondary)
            }
        } else {
            Section(title) { content() }
        }
    }
}

/// A window's loading state: the system's spinner with the switch off, the Thread quiet bar with it on.
struct RestyledProgress: View {
    @Environment(\.threadLook) private var look

    @ViewBuilder var body: some View {
        if look == .thread { WLoadingState() } else { ProgressView() }
    }
}

private struct RestyledList: ViewModifier {
    @Environment(\.threadLook) private var look
    @Environment(\.wTheme) private var theme

    @ViewBuilder func body(content: Content) -> some View {
        if look == .thread {
            content.scrollContentBackground(.hidden).background(theme.backgroundPage)
        } else {
            content
        }
    }
}

extension View {
    /// A `List` that draws the theme's page color behind its rows (Thread look only).
    func restyledList() -> some View { modifier(RestyledList()) }
}

/// A menu picker over a list of accounts: the system's menu picker with the switch off, the Thread picker (control border, focus
/// ring) with it on. Same label, same choices, same binding.
struct RestyledAccountPicker: View {
    @Environment(\.threadLook) private var look
    let label: LocalizedStringResource
    let emails: [String]
    @Binding var selection: String

    @ViewBuilder var body: some View {
        if look == .thread {
            WThreadMenuPicker(
                label: String(localized: label), options: emails.map { WSettingsOption(id: $0, title: $0) }, selection: selection,
                onSelect: { selection = $0 })
        } else {
            Picker(String(localized: label), selection: $selection) {
                ForEach(emails, id: \.self) { Text($0).tag($0) }
            }
            .pickerStyle(.menu)
            .font(.whimbrelBody(.caption))
        }
    }
}
