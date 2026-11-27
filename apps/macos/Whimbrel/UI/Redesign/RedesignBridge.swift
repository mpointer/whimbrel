import AppKit
import SwiftUI
import WhimbrelUI

/// Turns the app's skin and palette into the `WTheme` the WhimbrelUI views read, so a user's skin
/// applies to the redesign screens. The Thread skin keeps the package's own Thread shape and type.
enum RedesignBridge {
    static func palette(_ theme: Theme, dark: Bool) -> [String: UInt32] {
        let source = dark ? theme.dark : theme.light
        var out: [String: UInt32] = [:]
        for (token, value) in source { out[token.rawValue] = value }
        return out
    }

    static func family(_ family: SkinFamily) -> WFontFamily {
        switch family {
        case .sfPro: return .system
        case .sfProRounded: return .rounded
        case .sfMono: return .monospaced
        case .newYork: return .serif
        default: return .named(family.rawValue)
        }
    }

    static func shape(_ skin: Skin) -> WShape {
        if skin.id == "thread" { return .thread }
        let border: WBorderStyle
        switch skin.shape.border {
        case .none: border = .none
        case .dashed: border = .dashed
        case .solid: border = .solid
        case .double: border = .double
        }
        let density: WDensity = skin.shape.density == .compact ? .compact : (skin.shape.density == .airy ? .roomy : .regular)
        let dividers: WDividerStyle = WDividerStyle(rawValue: skin.decoration.dividers.rawValue) ?? .none
        return WShape(
            controlRadius: skin.shape.controlRadius, cardRadius: skin.shape.radius, surfaceRadius: skin.shape.radius * 1.5,
            borderWidth: skin.shape.borderWidth, borderStyle: border, density: density, dividers: dividers)
    }

    static func type(_ skin: Skin) -> WTypeStyle {
        if skin.id == "thread" { return .thread }
        let t = skin.typography
        let caps: WLabelCaps = t.caps == .upper ? .upper : (t.caps == .small ? .small : .none)
        return WTypeStyle(
            displayFamily: family(t.display.family), bodyFamily: family(t.body.family), readingFamily: .serif,
            numeralsFamily: family(t.numerals.family), labelFamily: family(t.label.family),
            displayWeight: WWeight(t.display.weight), bodyWeight: WWeight(t.body.weight),
            numeralsWeight: WWeight(t.numerals.weight), labelWeight: WWeight(t.label.weight),
            displayItalic: t.display.italic, labelItalic: t.label.italic, labelCaps: caps)
    }

    /// The skin's motion. Thread keeps the motion it always had (nothing new). Reduce Motion is applied where it is drawn
    /// (`WMotion.respecting`), since the setting can change while a window is open.
    static func motion(_ skin: Skin) -> WMotion {
        if skin.id == "thread" { return .thread }
        return WMotion.resolve(subtle: skin.motion == .subtle, reduceMotion: false)
    }

    /// The page texture a skin asks for. Thread draws none, and so does the High Contrast skin (its look is plain by design).
    static func texture(_ skin: Skin) -> WTexture {
        if skin.id == "thread" || skin.id == "high-contrast" { return .none }
        switch skin.decoration.texture {
        case .none: return .none
        case .paper: return .paper
        case .grain: return .grain
        case .grid: return .grid
        }
    }

    static func theme(colorScheme: ColorScheme, contrast: ColorSchemeContrast) -> WTheme {
        let dark = colorScheme == .dark
        // With Increase Contrast on, every screen draws Thread's high contrast colors whatever skin is picked (the stored choice is
        // not changed), so text and backgrounds agree with Thread's own high contrast parts. Shape and type stay the skin's.
        let colors = WContrastPalette.palette(skin: palette(ThemeStore.shared.selected, dark: dark), colorScheme: colorScheme, contrast: contrast)
        return WTheme(
            palette: colors, colorScheme: colorScheme, contrast: contrast,
            shape: shape(SkinStore.shared.selected), type: type(SkinStore.shared.selected), motion: motion(SkinStore.shared.selected),
            texture: texture(SkinStore.shared.selected))
    }
}

private struct RedesignThemed: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.colorSchemeContrast) private var contrast

    func body(content: Content) -> some View {
        content
            .environment(\.wTheme, RedesignBridge.theme(colorScheme: colorScheme, contrast: contrast))
            .environment(\.wStrings, WhimbrelUIStrings.make())
    }
}

extension View {
    /// Gives a WhimbrelUI view the app's theme (skin and palette) and words.
    func redesignThemed() -> some View { modifier(RedesignThemed()) }
}

/// The switch at a call site: the redesign view when the hidden flag is on, the old view otherwise.
/// `enabled` overrides the environment where it is not set (the Settings window).
struct RedesignSwitch<New: View, Old: View>: View {
    @Environment(\.redesignEnabled) private var environmentOn
    var enabled: Bool?
    @ViewBuilder let new: () -> New
    @ViewBuilder let old: () -> Old

    var body: some View {
        if enabled ?? environmentOn { new() } else { old() }
    }
}
