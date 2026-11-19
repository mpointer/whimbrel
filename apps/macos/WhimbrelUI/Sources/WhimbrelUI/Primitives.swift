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
        let avatar: CGFloat = (size == .small ? 18 : 22) * k
        HStack(spacing: theme.space(6)) {
            Text(person.initials)
                .wFont(.caption)
                .fontWeight(.bold)
                .minimumScaleFactor(0.6)
                .lineLimit(1)
                .foregroundStyle(theme.textPrimary)
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
    let action: () -> Void

    public init(number: Int, isSelected: Bool = false, action: @escaping () -> Void) {
        self.number = number
        self.isSelected = isSelected
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
        .accessibilityLabel(strings.sourceNumber(number))
        .accessibilityHint(strings.selectsThisSource)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

// MARK: Button

public enum WButtonKind: Sendable { case primary, secondary, quiet, destructive }

private struct WButtonBody: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.isEnabled) private var isEnabled
    let kind: WButtonKind
    let configuration: ButtonStyleConfiguration

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        let fill: Color = {
            switch kind {
            case .primary: return theme.accentPrimary
            case .secondary: return theme.backgroundCard
            case .quiet: return configuration.isPressed ? theme.selection : .clear
            case .destructive: return theme.error
            }
        }()
        let ink: Color = {
            switch kind {
            case .primary, .destructive: return theme.textOnAccent
            case .secondary: return theme.textPrimary
            case .quiet: return theme.accentPrimary
            }
        }()
        configuration.label
            .wFont(.bodyStrong)
            .foregroundStyle(ink)
            .padding(.horizontal, theme.space(12))
            .frame(minHeight: WTokens.Size.target * k)
            .background(shape.fill(fill))
            .overlay {
                if kind == .secondary {
                    shape.wBorder(theme.isHighContrast ? theme.border : theme.border, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 0.75))
                } else if theme.isHighContrast {
                    shape.strokeBorder(theme.textPrimary, lineWidth: theme.shape.highContrastBorderWidth)
                }
            }
            .overlay(shape.fill(Color.black.opacity(configuration.isPressed && kind != .quiet ? 0.12 : 0)))
            .opacity(isEnabled ? 1 : 0.45)
            .contentShape(shape)
    }
}

struct WButtonStyle: ButtonStyle {
    let kind: WButtonKind
    func makeBody(configuration: Configuration) -> some View {
        WButtonBody(kind: kind, configuration: configuration)
    }
}

/// Primary, secondary, quiet or destructive, with an optional glyph and shortcut hint.
public struct WButton: View {
    @Environment(\.wTheme) private var theme
    let title: String
    let glyph: String?
    let shortcutHint: String?
    let kind: WButtonKind
    let action: () -> Void

    public init(_ title: String, glyph: String? = nil, shortcutHint: String? = nil, kind: WButtonKind = .secondary, action: @escaping () -> Void) {
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
        .buttonStyle(WButtonStyle(kind: kind))
        .accessibilityLabel(title)
    }
}
