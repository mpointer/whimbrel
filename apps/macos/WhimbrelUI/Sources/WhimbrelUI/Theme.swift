import SwiftUI

/// The four looks every color has: light, dark, and a high contrast version of each.
public enum WVariant: String, CaseIterable, Sendable {
    case light, dark, highContrastLight, highContrastDark

    public init(colorScheme: ColorScheme, contrast: ColorSchemeContrast) {
        let dark = colorScheme == .dark
        let high = contrast == .increased
        switch (dark, high) {
        case (true, true): self = .highContrastDark
        case (true, false): self = .dark
        case (false, true): self = .highContrastLight
        case (false, false): self = .light
        }
    }

    public var isDark: Bool { self == .dark || self == .highContrastDark }
    public var isHighContrast: Bool { self == .highContrastLight || self == .highContrastDark }
    public var colorScheme: ColorScheme { isDark ? .dark : .light }
    public var contrast: ColorSchemeContrast { isHighContrast ? .increased : .standard }

    /// The Thread palette for this variant, straight from the generated tokens.
    public var threadPalette: [String: UInt32] {
        switch self {
        case .light: return WThreadPalette.light
        case .dark: return WThreadPalette.dark
        case .highContrastLight: return WThreadPalette.highContrastLight
        case .highContrastDark: return WThreadPalette.highContrastDark
        }
    }
}

extension WFixedColor {
    public func value(for variant: WVariant) -> UInt32 {
        switch variant {
        case .light: return light
        case .dark: return dark
        case .highContrastLight: return highContrastLight
        case .highContrastDark: return highContrastDark
        }
    }
}

extension Color {
    /// A color from 0xRRGGBB.
    public init(wHex value: UInt32) {
        self.init(
            .sRGB,
            red: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255,
            opacity: 1
        )
    }
}

/// How the border of a card or control is drawn.
public enum WBorderStyle: String, Sendable, CaseIterable { case solid, dashed, double, none }

/// How a skin draws the rules between Home's sections and in its ruled and line time lines (`decoration.dividers`).
public enum WDividerStyle: String, Sendable, CaseIterable { case none, hairline, chalk, double, dotted }

/// How much room a skin gives content. Multiplies padding and gaps, not text.
public enum WDensity: String, Sendable, CaseIterable {
    case compact, regular, roomy
    public var factor: CGFloat {
        switch self {
        case .compact: return 0.8
        case .regular: return 1
        case .roomy: return 1.25
        }
    }
}

/// Shape a skin can set. Defaults are the Thread values from `WTokens`.
public struct WShape: Sendable, Equatable {
    public var controlRadius: CGFloat
    public var cardRadius: CGFloat
    public var surfaceRadius: CGFloat
    /// Hairline width for cards and controls at standard contrast. High contrast uses `highContrastBorderWidth`.
    public var borderWidth: CGFloat
    public var highContrastBorderWidth: CGFloat
    public var borderStyle: WBorderStyle
    public var density: WDensity
    /// The rules on Home. Thread draws none.
    public var dividers: WDividerStyle

    public init(
        controlRadius: CGFloat = WTokens.Radius.control,
        cardRadius: CGFloat = WTokens.Radius.card,
        surfaceRadius: CGFloat = WTokens.Radius.surface,
        borderWidth: CGFloat = 0.5,
        highContrastBorderWidth: CGFloat = 1.5,
        borderStyle: WBorderStyle = .solid,
        density: WDensity = .regular,
        dividers: WDividerStyle = .none
    ) {
        self.controlRadius = controlRadius
        self.cardRadius = cardRadius
        self.surfaceRadius = surfaceRadius
        self.borderWidth = borderWidth
        self.highContrastBorderWidth = highContrastBorderWidth
        self.borderStyle = borderStyle
        self.density = density
        self.dividers = dividers
    }

    public static let thread = WShape()
}

/// The motion a skin gives the interface, in seconds and points (docs/superpowers/plans/2026-11-25-richer-skins.md, section 6). One set
/// for both OSes (`skins/testdata/expected/motion.json`). `.thread` and `none` are all zero: nothing new animates, so Thread draws as
/// it always did. Only interface transitions read this. The recording indicator, the capture control, the consent banner and approvals
/// never do.
public struct WMotion: Sendable, Equatable {
    /// A Home section fades in and rises `rise` points, once, on first show.
    public var section: Double
    public var rise: CGFloat
    /// A button, link or the ask bar changes state.
    public var control: Double
    /// The "now" marker moves to its new place.
    public var marker: Double
    /// A card or panel opens.
    public var panel: Double

    public init(section: Double = 0, rise: CGFloat = 0, control: Double = 0, marker: Double = 0, panel: Double = 0) {
        self.section = section
        self.rise = rise
        self.control = control
        self.marker = marker
        self.panel = panel
    }

    public static let thread = WMotion()
    public static let none = WMotion()
    public static let subtle = WMotion(section: 0.15, rise: 4, control: 0.12, marker: 0.2, panel: 0.15)

    /// What a skin's motion resolves to. When the system asks for less motion the answer is `none` whatever the skin says.
    public static func resolve(subtle isSubtle: Bool, reduceMotion: Bool) -> WMotion {
        isSubtle && !reduceMotion ? .subtle : .none
    }

    /// The same motion, or none when the system asks for less. A view applies this to the theme's motion, since the system setting
    /// can change while a window is open.
    public func respecting(reduceMotion: Bool) -> WMotion { reduceMotion ? .none : self }

    /// The animation for a duration, or nil for none (nil leaves the view exactly as it was before motion existed).
    public static func animation(_ seconds: Double) -> Animation? {
        seconds > 0 ? .easeInOut(duration: seconds) : nil
    }
}

/// A font family a skin can name.
public enum WFontFamily: Sendable, Equatable {
    case system, serif, rounded, monospaced
    case named(String)
}

/// A font weight as a number, so "two steps up" is arithmetic (the CSS scale the Windows app uses).
public enum WWeight: Int, Sendable, CaseIterable, Comparable {
    case ultraLight = 100, thin = 200, light = 300, regular = 400, medium = 500
    case semibold = 600, bold = 700, heavy = 800, black = 900

    public static func < (a: WWeight, b: WWeight) -> Bool { a.rawValue < b.rawValue }

    public var font: Font.Weight {
        switch self {
        case .ultraLight: return .ultraLight
        case .thin: return .thin
        case .light: return .light
        case .regular: return .regular
        case .medium: return .medium
        case .semibold: return .semibold
        case .bold: return .bold
        case .heavy: return .heavy
        case .black: return .black
        }
    }

    /// Body emphasis: two steps (200) above this weight, up to 800. A body already heavier than 800 keeps its own weight,
    /// so emphasis is never lighter than the text it sits in. Same rule as `emphasisWeight` in the Windows `skin.ts`.
    public var emphasis: WWeight {
        WWeight(rawValue: max(rawValue, min(rawValue + 200, WWeight.heavy.rawValue))) ?? self
    }
}

/// How a skin sets its section labels: as typed, in capitals, or in small capitals.
public enum WLabelCaps: Sendable, Equatable {
    case none, upper, small
}

/// The faces every Thread text role belongs to. Reading text is fixed and never takes a skin's face.
public enum WFaceKind: Sendable, Equatable {
    case display, body, numerals, label, reading
}

/// Type a skin can set. Display roles are subtitle, title and page title; numerals is the time; label is the caption
/// and section labels; body roles are the rest of the interface; reading roles are the user's notes and the note title.
///
/// A weight of `nil` means the role keeps the weight the Thread type scale gives it, so `.thread` draws as it always did.
/// Italic applies to display and label only: body italic is ignored (long italic text reads slowly) and so is numerals
/// italic. Weights follow docs/superpowers/plans/2026-11-25-richer-skins.md, decision 6: display, numerals and label use
/// the skin weight as set; body uses it for regular text and two steps up (capped at 800) for emphasis.
public struct WTypeStyle: Sendable, Equatable {
    public var displayFamily: WFontFamily
    public var bodyFamily: WFontFamily
    public var readingFamily: WFontFamily
    public var numeralsFamily: WFontFamily
    public var labelFamily: WFontFamily
    public var displayWeight: WWeight?
    public var bodyWeight: WWeight?
    public var numeralsWeight: WWeight?
    public var labelWeight: WWeight?
    public var displayItalic: Bool
    public var labelItalic: Bool
    /// How section labels are cased.
    public var labelCaps: WLabelCaps

    /// Whether section labels are set in capitals.
    public var sectionLabelCaps: Bool { labelCaps == .upper }

    public init(
        displayFamily: WFontFamily = .system,
        bodyFamily: WFontFamily = .system,
        readingFamily: WFontFamily = .serif,
        sectionLabelCaps: Bool = true,
        numeralsFamily: WFontFamily? = nil,
        labelFamily: WFontFamily? = nil,
        displayWeight: WWeight? = nil,
        bodyWeight: WWeight? = nil,
        numeralsWeight: WWeight? = nil,
        labelWeight: WWeight? = nil,
        displayItalic: Bool = false,
        labelItalic: Bool = false,
        labelCaps: WLabelCaps? = nil
    ) {
        self.displayFamily = displayFamily
        self.bodyFamily = bodyFamily
        self.readingFamily = readingFamily
        // Numerals and labels followed the body family before skins could name them, so that stays the default.
        self.numeralsFamily = numeralsFamily ?? bodyFamily
        self.labelFamily = labelFamily ?? bodyFamily
        self.displayWeight = displayWeight
        self.bodyWeight = bodyWeight
        self.numeralsWeight = numeralsWeight
        self.labelWeight = labelWeight
        self.displayItalic = displayItalic
        self.labelItalic = labelItalic
        self.labelCaps = labelCaps ?? (sectionLabelCaps ? .upper : .none)
    }

    public static let thread = WTypeStyle()

    /// Section labels: the skin's label weight, or semibold when it sets none. Thread's label weight is regular (captions),
    /// but its section labels have always been semibold, so Thread pins them there; one label weight cannot be both.
    public var sectionLabelWeight: WWeight { labelWeight ?? .semibold }

    /// The boldest rules (counts, badges, initials): Thread stays bold (700); any other skin takes its body weight plus
    /// two steps, capped at 800.
    public var strongWeight: WWeight { bodyWeight?.emphasis ?? .bold }
}

/// The colors a component can use: the 15 skinnable colors plus the fixed ones, resolved for one variant.
/// Built by injection (the SwiftUI environment), never read from a singleton.
public struct WTheme: Sendable, Equatable {
    public static let skinnableKeys: [String] = [
        "backgroundPage", "backgroundCard", "border", "textPrimary", "textSecondary",
        "accentPrimary", "accentSecondary", "accentHighlight", "error", "warning",
        "success", "link", "selection", "highlight", "textOnAccent",
    ]

    public static let fixedColors: [String: WFixedColor] = [
        "recording": WFixed.recording, "onRecording": WFixed.onRecording,
        "recordingTint": WFixed.recordingTint, "aiInk": WFixed.aiInk, "aiTint": WFixed.aiTint,
        "focus": WFixed.focus, "paused": WFixed.paused, "scrim": WFixed.scrim,
        "appBlue": WFixed.appBlue, "appGreen": WFixed.appGreen,
        "appPurple": WFixed.appPurple, "appAmber": WFixed.appAmber,
    ]

    public let variant: WVariant
    /// The 15 skinnable colors as 0xRRGGBB. A key missing from the input palette falls back to Thread.
    public let palette: [String: UInt32]
    /// The 12 fixed colors resolved for `variant`.
    public let fixed: [String: UInt32]
    /// Radii, border and density. Components read these, never literals.
    public let shape: WShape
    /// Font families and label caps.
    public let type: WTypeStyle
    /// Durations for interface transitions.
    public let motion: WMotion

    /// The page texture the skin asks for. Drawn only where `drawsTexture` says.
    public let texture: WTexture

    public init(palette: [String: UInt32], variant: WVariant, shape: WShape = .thread, type: WTypeStyle = .thread, motion: WMotion = .thread, texture: WTexture = .none) {
        self.shape = shape
        self.type = type
        self.motion = motion
        self.texture = texture
        var resolved = variant.threadPalette
        for key in Self.skinnableKeys {
            if let value = palette[key] { resolved[key] = value }
        }
        self.variant = variant
        self.palette = resolved
        self.fixed = Self.fixedColors.mapValues { $0.value(for: variant) }
    }

    /// For an app palette (a `[String: UInt32]` keyed by theme token raw names).
    public init(
        palette: [String: UInt32],
        colorScheme: ColorScheme,
        contrast: ColorSchemeContrast,
        shape: WShape = .thread,
        type: WTypeStyle = .thread,
        motion: WMotion = .thread,
        texture: WTexture = .none
    ) {
        self.init(palette: palette, variant: WVariant(colorScheme: colorScheme, contrast: contrast), shape: shape, type: type, motion: motion, texture: texture)
    }

    public static func thread(variant: WVariant) -> WTheme {
        WTheme(palette: variant.threadPalette, variant: variant)
    }

    /// A length scaled by the skin's density. Use for padding and gaps.
    public func space(_ value: CGFloat) -> CGFloat { value * shape.density.factor }

    /// The border width in effect: thicker in high contrast, zero when the skin has no borders.
    public var borderWidth: CGFloat {
        shape.borderStyle == .none && !isHighContrast ? 0 : (isHighContrast ? shape.highContrastBorderWidth : shape.borderWidth)
    }

    /// Whether a border of this width draws as two lines: the skin's border is double, the width is a point or more (finer
    /// draws solid), and high contrast is off (high contrast draws one firm line).
    public func drawsDoubleBorder(width: CGFloat) -> Bool {
        shape.borderStyle == .double && !isHighContrast && width >= 1
    }

    /// Whether the page draws the skin's texture: it asks for one, and high contrast (Increase Contrast, or the High Contrast
    /// palette) is off. Texture is for the page background only.
    public var drawsTexture: Bool { texture != .none && !isHighContrast }

    /// Stroke style for the skin's borders.
    public var borderStroke: StrokeStyle {
        StrokeStyle(lineWidth: borderWidth, lineCap: .round, lineJoin: .round, dash: shape.borderStyle == .dashed && !isHighContrast ? [4, 3] : [])
    }

    /// A section label in the skin's case.
    public func sectionLabel(_ text: String) -> String {
        switch type.labelCaps {
        case .none: return text
        case .upper: return text.uppercased()
        // The face's own small caps when it has them; otherwise capitals (drawn smaller, see `sectionLabelFont`).
        case .small: return WFontSupport.hasSmallCaps(type.labelFamily) ? text : text.uppercased()
        }
    }

    /// The font for a section label, in the skin's label face and caps.
    public func sectionLabelFont(scale: CGFloat = 1, caps: WLabelCaps? = nil) -> Font {
        WTextRole.sectionLabelSpec(scale: scale, style: type, caps: caps).font
    }

    public static func thread(colorScheme: ColorScheme, contrast: ColorSchemeContrast) -> WTheme {
        thread(variant: WVariant(colorScheme: colorScheme, contrast: contrast))
    }

    public var isDark: Bool { variant.isDark }
    public var isHighContrast: Bool { variant.isHighContrast }

    private func skin(_ key: String) -> Color { Color(wHex: palette[key] ?? 0) }
    private func fix(_ key: String) -> Color { Color(wHex: fixed[key] ?? 0) }

    // Skinnable
    public var backgroundPage: Color { skin("backgroundPage") }
    public var backgroundCard: Color { skin("backgroundCard") }
    public var border: Color { skin("border") }
    /// The edge of a control that must be told from the card behind it (3 to 1 or more): the secondary ink, softened. In high
    /// contrast it is the border itself. The menu and the small windows use it; the shared `border` is unchanged.
    var firmEdge: Color { isHighContrast ? border : textSecondary.opacity(0.7) }
    public var textPrimary: Color { skin("textPrimary") }
    public var textSecondary: Color { skin("textSecondary") }
    public var accentPrimary: Color { skin("accentPrimary") }
    public var accentSecondary: Color { skin("accentSecondary") }
    public var accentHighlight: Color { skin("accentHighlight") }
    public var error: Color { skin("error") }
    public var warning: Color { skin("warning") }
    public var success: Color { skin("success") }
    public var link: Color { skin("link") }
    public var selection: Color { skin("selection") }
    public var highlight: Color { skin("highlight") }
    public var textOnAccent: Color { skin("textOnAccent") }

    // Fixed
    public var recording: Color { fix("recording") }
    public var onRecording: Color { fix("onRecording") }
    public var recordingTint: Color { fix("recordingTint") }
    public var aiInk: Color { fix("aiInk") }
    public var aiTint: Color { fix("aiTint") }
    public var focus: Color { fix("focus") }
    public var paused: Color { fix("paused") }
    public var scrim: Color { fix("scrim") }

    public func appColor(_ app: WAppKind) -> Color {
        switch app {
        case .blue: return fix("appBlue")
        case .green: return fix("appGreen")
        case .purple: return fix("appPurple")
        case .amber: return fix("appAmber")
        }
    }
}

/// The dot color a seen app carries. Always paired with a glyph and a name.
public enum WAppKind: String, CaseIterable, Sendable {
    case blue, green, purple, amber
}

extension EnvironmentValues {
    /// The theme every component reads. Set it with `wThemed()` or `.environment(\.wTheme, ...)`.
    @Entry public var wTheme: WTheme = .thread(variant: .light)
    /// Multiplies every font size, line height and fixed text container. macOS has no Dynamic Type.
    @Entry public var wTextScale: CGFloat = 1
}

private struct WThemedModifier: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.colorSchemeContrast) private var contrast
    let palette: [String: UInt32]?
    let shape: WShape
    let type: WTypeStyle

    func body(content: Content) -> some View {
        let theme = WTheme(
            palette: palette ?? WVariant(colorScheme: colorScheme, contrast: contrast).threadPalette,
            colorScheme: colorScheme, contrast: contrast, shape: shape, type: type
        )
        return content.environment(\.wTheme, theme)
    }
}

extension View {
    /// Resolves the theme from the system appearance and contrast. Pass the app's own palette to skin it.
    public func wThemed(palette: [String: UInt32]? = nil, shape: WShape = .thread, type: WTypeStyle = .thread) -> some View {
        modifier(WThemedModifier(palette: palette, shape: shape, type: type))
    }
}
