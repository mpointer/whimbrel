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
public enum WBorderStyle: String, Sendable, CaseIterable { case solid, dashed, none }

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

    public init(
        controlRadius: CGFloat = WTokens.Radius.control,
        cardRadius: CGFloat = WTokens.Radius.card,
        surfaceRadius: CGFloat = WTokens.Radius.surface,
        borderWidth: CGFloat = 0.5,
        highContrastBorderWidth: CGFloat = 1.5,
        borderStyle: WBorderStyle = .solid,
        density: WDensity = .regular
    ) {
        self.controlRadius = controlRadius
        self.cardRadius = cardRadius
        self.surfaceRadius = surfaceRadius
        self.borderWidth = borderWidth
        self.highContrastBorderWidth = highContrastBorderWidth
        self.borderStyle = borderStyle
        self.density = density
    }

    public static let thread = WShape()
}

/// A font family a skin can name.
public enum WFontFamily: Sendable, Equatable {
    case system, serif, rounded, monospaced
    case named(String)
}

/// Type a skin can set. Display roles are subtitle, title and page title; body roles are the rest of
/// the interface; reading roles are the user's notes and the note title.
public struct WTypeStyle: Sendable, Equatable {
    public var displayFamily: WFontFamily
    public var bodyFamily: WFontFamily
    public var readingFamily: WFontFamily
    /// Whether section labels are set in capitals.
    public var sectionLabelCaps: Bool

    public init(
        displayFamily: WFontFamily = .system,
        bodyFamily: WFontFamily = .system,
        readingFamily: WFontFamily = .serif,
        sectionLabelCaps: Bool = true
    ) {
        self.displayFamily = displayFamily
        self.bodyFamily = bodyFamily
        self.readingFamily = readingFamily
        self.sectionLabelCaps = sectionLabelCaps
    }

    public static let thread = WTypeStyle()
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

    public init(palette: [String: UInt32], variant: WVariant, shape: WShape = .thread, type: WTypeStyle = .thread) {
        self.shape = shape
        self.type = type
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
        type: WTypeStyle = .thread
    ) {
        self.init(palette: palette, variant: WVariant(colorScheme: colorScheme, contrast: contrast), shape: shape, type: type)
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

    /// Stroke style for the skin's borders.
    public var borderStroke: StrokeStyle {
        StrokeStyle(lineWidth: borderWidth, lineCap: .round, lineJoin: .round, dash: shape.borderStyle == .dashed && !isHighContrast ? [4, 3] : [])
    }

    /// A section label in the skin's case.
    public func sectionLabel(_ text: String) -> String {
        type.sectionLabelCaps ? text.uppercased() : text
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
