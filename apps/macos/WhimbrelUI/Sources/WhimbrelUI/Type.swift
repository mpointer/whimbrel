import AppKit
import CoreText
import SwiftUI

/// What the installed fonts can do, so a skin that names a font this Mac lacks, or small caps a face does not have, still draws
/// readable text.
public enum WFontSupport {
    /// Whether a named font (a family such as "Avenir Next" or a font name) is installed.
    public static func isInstalled(_ name: String) -> Bool {
        NSFont(name: name, size: 12) != nil || NSFontManager.shared.availableFontFamilies.contains(name)
    }

    /// Whether the face has its own small capitals (the OpenType `smcp` feature, "lowercase small caps"). A named font that is
    /// not installed draws as the system font, so it answers for the system font.
    public static func hasSmallCaps(_ family: WFontFamily) -> Bool {
        let font = nsFont(family.resolved, size: 12)
        guard let features = CTFontCopyFeatures(font) as? [[String: Any]] else { return false }
        for feature in features {
            guard (feature[kCTFontFeatureTypeIdentifierKey as String] as? Int) == kLowerCaseType else { continue }
            let selectors = feature[kCTFontFeatureTypeSelectorsKey as String] as? [[String: Any]] ?? []
            if selectors.contains(where: { ($0[kCTFontFeatureSelectorIdentifierKey as String] as? Int) == kLowerCaseSmallCapsSelector }) {
                return true
            }
        }
        return false
    }

    static func nsFont(_ family: WFontFamily, size: CGFloat) -> NSFont {
        let base = NSFont.systemFont(ofSize: size, weight: .regular)
        func designed(_ design: NSFontDescriptor.SystemDesign) -> NSFont {
            if let descriptor = base.fontDescriptor.withDesign(design), let font = NSFont(descriptor: descriptor, size: size) {
                return font
            }
            return base
        }
        switch family {
        case .system: return base
        case .serif: return designed(.serif)
        case .rounded: return designed(.rounded)
        case .monospaced: return designed(.monospaced)
        case .named(let name): return NSFont(name: name, size: size) ?? base
        }
    }
}

extension WFontFamily {
    /// A named font this Mac does not have draws as the system face, as the Windows app does for an unknown family.
    public var resolved: WFontFamily {
        if case .named(let name) = self, !WFontSupport.isInstalled(name) { return .system }
        return self
    }
}

extension WWeight {
    public init(_ weight: Font.Weight) {
        let all: [(Font.Weight, WWeight)] = [
            (.ultraLight, .ultraLight), (.thin, .thin), (.light, .light), (.regular, .regular), (.medium, .medium),
            (.semibold, .semibold), (.bold, .bold), (.heavy, .heavy), (.black, .black),
        ]
        self = all.first { $0.0 == weight }?.1 ?? .regular
    }
}

/// One text role resolved against a skin: what the font is made of. Tests compare these; `font` draws them.
public struct WFontSpec: Sendable, Equatable {
    public var face: WFaceKind
    public var family: WFontFamily
    public var weight: WWeight
    public var italic: Bool
    public var size: CGFloat
    public var smallCaps: Bool
    public var monospacedDigits: Bool

    public var font: Font {
        var font: Font
        switch family {
        case .system: font = .system(size: size, weight: weight.font, design: .default)
        case .serif: font = .system(size: size, weight: weight.font, design: .serif)
        case .rounded: font = .system(size: size, weight: weight.font, design: .rounded)
        case .monospaced: font = .system(size: size, weight: weight.font, design: .monospaced)
        case .named(let name): font = Font.custom(name, size: size).weight(weight.font)
        }
        if italic { font = font.italic() }
        if smallCaps { font = font.smallCaps() }
        if monospacedDigits { font = font.monospacedDigit() }
        return font
    }
}

/// The type roles from the tokens. UI roles use the system font, `reading` and `readingTitle` use
/// New York (the system serif), and `time` uses monospaced digits. A skin's type style (family, weight, italic) applies to
/// every role except reading text.
public enum WTextRole: CaseIterable, Sendable {
    case caption, secondary, body, bodyStrong, time, subtitle, title, pageTitle, reading, readingTitle

    public var token: WTokens.TypeRole {
        switch self {
        case .caption: return WTokens.TypeScale.caption
        case .secondary: return WTokens.TypeScale.secondary
        case .body: return WTokens.TypeScale.body
        case .bodyStrong: return WTokens.TypeScale.bodyStrong
        case .time: return WTokens.TypeScale.time
        case .subtitle: return WTokens.TypeScale.subtitle
        case .title: return WTokens.TypeScale.title
        case .pageTitle: return WTokens.TypeScale.pageTitle
        case .reading: return WTokens.TypeScale.reading
        case .readingTitle: return WTokens.TypeScale.readingTitle
        }
    }

    public var isReading: Bool { self == .reading || self == .readingTitle }
    public var isDisplay: Bool { self == .subtitle || self == .title || self == .pageTitle }

    /// The face this role is set in: display (titles), numerals (the time), label (captions), body (all other interface text)
    /// or reading (notes, fixed).
    public var faceKind: WFaceKind {
        if isReading { return .reading }
        if isDisplay { return .display }
        switch self {
        case .time: return .numerals
        case .caption: return .label
        default: return .body
        }
    }

    public func size(scale: CGFloat = 1) -> CGFloat { token.size * scale }
    public func lineHeight(scale: CGFloat = 1) -> CGFloat { token.lineHeight * scale }

    /// The family a skin's type style assigns to this role.
    public func family(in style: WTypeStyle) -> WFontFamily {
        switch faceKind {
        case .reading: return style.readingFamily
        case .display: return style.displayFamily
        case .numerals: return style.numeralsFamily
        case .label: return style.labelFamily
        case .body: return style.bodyFamily
        }
    }

    /// The weight for this role. A skin that sets none keeps the Thread scale's weight. Display, numerals and label take the
    /// skin's weight as set; body takes it for regular text and two steps up for `bodyStrong`, capped at 800.
    public func weight(in style: WTypeStyle) -> WWeight {
        let token = WWeight(self.token.weight)
        switch faceKind {
        case .reading: return token
        case .display: return style.displayWeight ?? token
        case .numerals: return style.numeralsWeight ?? token
        case .label: return style.labelWeight ?? token
        case .body:
            guard let body = style.bodyWeight else { return token }
            return self == .bodyStrong ? body.emphasis : body
        }
    }

    /// Italic applies to display and label only.
    public func italic(in style: WTypeStyle) -> Bool {
        switch faceKind {
        case .display: return style.displayItalic
        case .label: return style.labelItalic
        default: return false
        }
    }

    /// The role resolved against a skin's type style.
    public func spec(scale: CGFloat = 1, style: WTypeStyle = .thread) -> WFontSpec {
        WFontSpec(
            face: faceKind, family: family(in: style).resolved, weight: weight(in: style), italic: italic(in: style),
            size: size(scale: scale), smallCaps: false, monospacedDigits: self == .time)
    }

    /// A section label: the label face at the secondary size, in the skin's label weight (semibold in Thread, which pins it),
    /// italic and caps. Small caps are the face's own; a face without them draws capitals at 85 percent size.
    public static func sectionLabelSpec(scale: CGFloat = 1, style: WTypeStyle = .thread, caps: WLabelCaps? = nil) -> WFontSpec {
        var size = WTextRole.secondary.size(scale: scale)
        var smallCaps = false
        if (caps ?? style.labelCaps) == .small {
            if WFontSupport.hasSmallCaps(style.labelFamily) { smallCaps = true } else { size *= 0.85 }
        }
        return WFontSpec(
            face: .label, family: style.labelFamily.resolved, weight: style.sectionLabelWeight, italic: style.labelItalic,
            size: size, smallCaps: smallCaps, monospacedDigits: false)
    }

    /// The scoreboard's big digits: the numerals face at 44 points, bold (as Thread has drawn them) unless the skin sets a
    /// numerals weight. Never italic.
    public static func scoreboardSpec(scale: CGFloat = 1, style: WTypeStyle = .thread) -> WFontSpec {
        WFontSpec(
            face: .numerals, family: style.numeralsFamily.resolved, weight: style.numeralsWeight ?? .bold, italic: false,
            size: 44 * scale, smallCaps: false, monospacedDigits: true)
    }

    public func font(scale: CGFloat = 1, style: WTypeStyle = .thread) -> Font {
        spec(scale: scale, style: style).font
    }

    /// Extra line spacing needed so a line is `lineHeight` tall, given the font's own natural height.
    public func lineSpacing(scale: CGFloat = 1, style: WTypeStyle = .thread) -> CGFloat {
        Self.extraSpacing(lineHeight: lineHeight(scale: scale), size: size(scale: scale), family: family(in: style))
    }

    static func extraSpacing(lineHeight: CGFloat, size: CGFloat, family: WFontFamily) -> CGFloat {
        let font = WFontSupport.nsFont(family, size: size)
        let natural = ceil(font.ascender - font.descender + font.leading)
        return max(0, lineHeight - natural)
    }
}

private struct WTextModifier: ViewModifier {
    @Environment(\.wTextScale) private var scale
    @Environment(\.wTheme) private var theme
    let role: WTextRole

    func body(content: Content) -> some View {
        content
            .font(role.font(scale: scale, style: theme.type))
            .lineSpacing(role.lineSpacing(scale: scale, style: theme.type))
    }
}

private struct WSectionLabelModifier: ViewModifier {
    var caps: WLabelCaps?
    @Environment(\.wTextScale) private var scale
    @Environment(\.wTheme) private var theme

    func body(content: Content) -> some View {
        content
            .font(theme.sectionLabelFont(scale: scale, caps: caps))
            .lineSpacing(WTextRole.extraSpacing(
                lineHeight: WTextRole.secondary.lineHeight(scale: scale), size: WTextRole.secondary.size(scale: scale),
                family: theme.type.labelFamily))
    }
}

extension View {
    /// Applies a type role at the current text scale.
    public func wFont(_ role: WTextRole) -> some View {
        modifier(WTextModifier(role: role))
    }

    /// Applies the skin's section label type (label face, weight, italic, caps) at the current text scale.
    public func wSectionLabel(caps: WLabelCaps? = nil) -> some View {
        modifier(WSectionLabelModifier(caps: caps))
    }
}
