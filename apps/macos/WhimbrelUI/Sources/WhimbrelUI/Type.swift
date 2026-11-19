import AppKit
import SwiftUI

/// The type roles from the tokens. UI roles use the system font, `reading` and `readingTitle` use
/// New York (the system serif), and `time` uses monospaced digits.
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

    public func size(scale: CGFloat = 1) -> CGFloat { token.size * scale }
    public func lineHeight(scale: CGFloat = 1) -> CGFloat { token.lineHeight * scale }

    /// The family a skin's type style assigns to this role.
    public func family(in style: WTypeStyle) -> WFontFamily {
        if isReading { return style.readingFamily }
        if isDisplay { return style.displayFamily }
        return style.bodyFamily
    }

    public func font(scale: CGFloat = 1, style: WTypeStyle = .thread) -> Font {
        let size = size(scale: scale)
        var font: Font
        switch family(in: style) {
        case .system: font = .system(size: size, weight: token.weight, design: .default)
        case .serif: font = .system(size: size, weight: token.weight, design: .serif)
        case .rounded: font = .system(size: size, weight: token.weight, design: .rounded)
        case .monospaced: font = .system(size: size, weight: token.weight, design: .monospaced)
        case .named(let name): font = Font.custom(name, size: size).weight(token.weight)
        }
        if self == .time { font = font.monospacedDigit() }
        return font
    }

    /// Extra line spacing needed so a line is `lineHeight` tall, given the font's own natural height.
    public func lineSpacing(scale: CGFloat = 1, style: WTypeStyle = .thread) -> CGFloat {
        let size = size(scale: scale)
        let base = NSFont.systemFont(ofSize: size, weight: .regular)
        func designed(_ design: NSFontDescriptor.SystemDesign) -> NSFont {
            if let descriptor = base.fontDescriptor.withDesign(design), let font = NSFont(descriptor: descriptor, size: size) {
                return font
            }
            return base
        }
        let font: NSFont
        switch family(in: style) {
        case .system: font = base
        case .serif: font = designed(.serif)
        case .rounded: font = designed(.rounded)
        case .monospaced: font = designed(.monospaced)
        case .named(let name): font = NSFont(name: name, size: size) ?? base
        }
        let natural = ceil(font.ascender - font.descender + font.leading)
        return max(0, lineHeight(scale: scale) - natural)
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

extension View {
    /// Applies a type role at the current text scale.
    public func wFont(_ role: WTextRole) -> some View {
        modifier(WTextModifier(role: role))
    }
}
