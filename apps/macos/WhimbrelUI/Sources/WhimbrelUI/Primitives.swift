import SwiftUI

/// A person shown as a speaker, owner or attendee.
public struct WPerson: Identifiable, Equatable, Sendable {
    public let name: String
    public var id: String { name }
    public init(name: String) { self.name = name }

    /// Up to two letters from the capitalized words of the name ("Jo at Northwind" gives "JN").
    public var initials: String {
        let words = name.split(whereSeparator: { $0 == " " || $0 == "," }).filter { $0.first?.isUppercase == true }
        let letters = words.count >= 2 ? [words.first!, words.last!] : Array(words.prefix(1))
        let text = letters.compactMap { $0.first }.map(String.init).joined()
        return text.isEmpty ? String(name.prefix(1)).uppercased() : text
    }
}

/// An icon from SF Symbols, sized with the text scale and hidden from accessibility.
struct WGlyph: View {
    @Environment(\.wTextScale) private var k
    let name: String
    var size: CGFloat = 13
    var weight: Font.Weight = .regular

    var body: some View {
        Image(systemName: name)
            .font(.system(size: size * k, weight: weight))
            .accessibilityHidden(true)
    }
}

/// A small keyboard hint, such as "⌘K".
struct WKeycap: View {
    @Environment(\.wTheme) private var theme
    let text: String
    var onFill: Color? = nil

    var body: some View {
        Text(text)
            .wFont(.caption)
            .foregroundStyle(onFill ?? theme.textSecondary)
            .padding(.horizontal, theme.space(5))
            .padding(.vertical, theme.space(1))
            .background(
                RoundedRectangle(cornerRadius: theme.shape.controlRadius * 0.5, style: .continuous)
                    .fill((onFill ?? theme.textSecondary).opacity(0.14))
            )
            .accessibilityHidden(true)
    }
}

// MARK: Speaker chip

public enum WChipSize: Sendable { case small, regular }

/// Initials avatar and name.
public struct WSpeakerChip: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let person: WPerson
    let size: WChipSize

    public init(_ person: WPerson, size: WChipSize = .regular) {
        self.person = person
        self.size = size
    }

    public var body: some View {
        let avatar: CGFloat = (size == .small ? 20 : 22) * k
        HStack(spacing: theme.space(6)) {
            Text(person.initials)
                .wFont(.caption)
                .fontWeight(.bold)
                .minimumScaleFactor(0.5)
                .lineLimit(1)
                .foregroundStyle(theme.textPrimary)
                .padding(.horizontal, theme.isHighContrast ? 2.5 : 1)
                .frame(width: avatar, height: avatar)
                .background(Circle().fill(theme.border.opacity(theme.isHighContrast ? 0 : 0.55)))
                .overlay(Circle().strokeBorder(theme.border, lineWidth: theme.isHighContrast ? 1 : 0))
            Text(person.name)
                .wFont(size == .small ? .secondary : .body)
                .fontWeight(.medium)
                .foregroundStyle(theme.textPrimary)
                .lineLimit(1)
        }
        .padding(.leading, theme.space(3))
        .padding(.trailing, theme.space(size == .small ? 8 : 10))
        .padding(.vertical, theme.space(3))
        .background(Capsule().fill(theme.backgroundPage))
        .overlay(RoundedRectangle(cornerRadius: WTokens.Radius.pill, style: .continuous).wBorder(theme.border.opacity(0.7), style: theme.borderStroke))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(person.name)
    }
}

// MARK: Source mark

/// The small numbered circle on an AI line. A button: it selects the matching source.
public struct WSourceMark: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let number: Int
    let isSelected: Bool
    /// Spoken after "Source 3": the speaker and the time ("Jo", "18:25").
    let spokenDetail: [String]
    /// The source's own words, spoken as the mark's value, so a screen reader hears the source once, at its line.
    let spokenQuote: String?
    /// With `onOpen`: the name of the extra action that opens the source ("Open in transcript at 18:25").
    let openTitle: String?
    let onOpen: (() -> Void)?
    let action: () -> Void

    public init(
        number: Int, isSelected: Bool = false, spokenDetail: [String] = [], spokenQuote: String? = nil, openTitle: String? = nil,
        onOpen: (() -> Void)? = nil, action: @escaping () -> Void
    ) {
        self.number = number
        self.isSelected = isSelected
        self.spokenDetail = spokenDetail
        self.spokenQuote = spokenQuote
        self.openTitle = openTitle
        self.onOpen = onOpen
        self.action = action
    }

    public var body: some View {
        let diameter = 18 * k
        Button(action: action) {
            Text("\(number)")
                .wFont(.caption)
                .fontWeight(.semibold)
                .foregroundStyle(isSelected ? theme.backgroundCard : theme.aiInk)
                .frame(width: diameter, height: diameter)
                .background(Circle().fill(isSelected ? theme.aiInk : Color.clear))
                .overlay(Circle().strokeBorder(theme.aiInk, lineWidth: theme.isHighContrast ? 2 : 1.25))
                .frame(width: max(diameter, WTokens.Size.target * k), height: max(diameter, WTokens.Size.target * k))
                .contentShape(Rectangle())
                .padding(-(max(diameter, WTokens.Size.target * k) - diameter) / 2)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(strings.joinDetails([strings.sourceNumber(number)] + spokenDetail))
        .accessibilityValue(spokenQuote.map(strings.quoted) ?? "")
        .accessibilityHint(strings.selectsThisSource)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .accessibilityActions {
            if let onOpen, let openTitle { Button(openTitle, action: onOpen) }
        }
    }
}

// MARK: Button

/// `destructive` is the loud filled red button, for the one button that does the damage. `destructiveText` is plain red
/// text on no fill: for a row's Remove or Revoke, which still ask before they do anything, so a page of rows is not a page
/// of red buttons.
public enum WButtonKind: Sendable { case primary, secondary, quiet, destructive, destructiveText }

public struct WDisabledButtonLookKey: EnvironmentKey { public static let defaultValue = false }

public extension EnvironmentValues {
    /// True inside the Settings window: a disabled button keeps readable text on a quiet fill. Everywhere else a disabled
    /// button is the same button at 45% strength, as it always was. A button with a `reason` opts in by itself.
    var wDisabledButtonLook: Bool {
        get { self[WDisabledButtonLookKey.self] }
        set { self[WDisabledButtonLookKey.self] = newValue }
    }
}

private struct WButtonBody: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.wDisabledButtonLook) private var settingsLook
    let kind: WButtonKind
    /// This button was given a reason for being off, so it uses the readable disabled look wherever it is.
    var hasReason = false
    /// A secondary button whose edge must stand out from the card it sits on (the menu and the small windows).
    let firm: Bool
    let configuration: ButtonStyleConfiguration

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        // A disabled button keeps readable text: secondary ink on a quiet fill, not the same button at half strength (which
        // left white on a pale blue in dark and high contrast).
        let readable = settingsLook || hasReason
        let solid = kind == .primary || kind == .destructive
        let fill: Color = {
            if readable && !isEnabled && solid { return theme.selection }
            switch kind {
            case .primary: return theme.accentPrimary
            case .secondary: return theme.backgroundCard
            case .quiet, .destructiveText: return configuration.isPressed ? theme.selection : .clear
            case .destructive: return theme.error
            }
        }()
        let ink: Color = {
            // In high contrast the secondary ink is as dark as the primary one, so a button that is off is dimmed too: it must not read as on.
            if readable && !isEnabled { return theme.isHighContrast ? theme.textSecondary.opacity(0.55) : theme.textSecondary }
            switch kind {
            case .primary, .destructive: return theme.textOnAccent
            case .secondary: return theme.textPrimary
            case .quiet: return theme.accentPrimary
            case .destructiveText: return theme.error
            }
        }()
        configuration.label
            .wFont(.bodyStrong)
            .foregroundStyle(ink)
            .padding(.horizontal, theme.space(12))
            .frame(minHeight: WTokens.Size.target * k)
            .background(shape.fill(fill))
            .overlay {
                if kind == .secondary || (readable && !isEnabled && solid) {
                    shape.wBorder(firm ? theme.firmEdge : theme.border, style: theme.borderStroke.widened(theme.isHighContrast ? (readable && !isEnabled ? 0.75 : 1.5) : 0.75))
                } else if theme.isHighContrast {
                    shape.strokeBorder(theme.textPrimary, lineWidth: theme.shape.highContrastBorderWidth)
                }
            }
            .overlay(shape.fill(Color.black.opacity(configuration.isPressed && kind != .quiet && kind != .destructiveText ? 0.12 : 0)))
            .opacity(readable || isEnabled ? 1 : 0.45)
            .contentShape(shape)
    }
}

struct WButtonStyle: ButtonStyle {
    let kind: WButtonKind
    var firm = false
    var hasReason = false
    func makeBody(configuration: Configuration) -> some View {
        WButtonBody(kind: kind, hasReason: hasReason, firm: firm, configuration: configuration)
    }
}

/// Primary, secondary, quiet or destructive, with an optional glyph and shortcut hint.
public struct WButton: View {
    @Environment(\.wTheme) private var theme
    let title: String
    let glyph: String?
    let shortcutHint: String?
    let kind: WButtonKind
    /// Why the button is off, when the caller turns it off: the hover help and the spoken hint. Nil when it is on.
    let reason: String?
    let action: () -> Void

    public init(_ title: String, glyph: String? = nil, shortcutHint: String? = nil, kind: WButtonKind = .secondary, reason: String? = nil, action: @escaping () -> Void) {
        self.reason = reason
        self.title = title
        self.glyph = glyph
        self.shortcutHint = shortcutHint
        self.kind = kind
        self.action = action
    }

    public var body: some View {
        let hintInk: Color? = (kind == .primary || kind == .destructive) ? theme.textOnAccent : nil
        Button(action: action) {
            HStack(spacing: theme.space(6)) {
                if let glyph { WGlyph(name: glyph, size: 12, weight: .semibold) }
                Text(title).lineLimit(1)
                if let shortcutHint { WKeycap(text: shortcutHint, onFill: hintInk) }
            }
        }
        .buttonStyle(WButtonStyle(kind: kind, hasReason: reason != nil))
        .accessibilityLabel(title)
        .modifier(WReasonHint(reason: reason))
    }
}

/// The hover help and the spoken hint, only when there is a reason: no button carries an empty one.
private struct WReasonHint: ViewModifier {
    let reason: String?
    @ViewBuilder func body(content: Content) -> some View {
        if let reason, !reason.isEmpty { content.help(reason).accessibilityHint(reason) } else { content }
    }
}
