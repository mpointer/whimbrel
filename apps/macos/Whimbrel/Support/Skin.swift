import SwiftUI

/// A skin: the typography, shape, icons, decoration, voice and Today layout that restyle every
/// screen. Parsed from `skins/<id>.json` by `SkinLoader`; see
/// docs/superpowers/specs/2026-10-20-skins-design.md. Mac-only.
struct Skin: Identifiable, Equatable {
    var id: String
    var name: String
    var author: String
    /// Theme ids, the first is the default colour option.
    var paletteIds: [String]
    var typography: SkinTypography
    var shape: SkinShape
    var icons: SkinIcons
    var decoration: SkinDecoration
    var voice: SkinVoice
    var archetype: SkinArchetype
    var archetypeOptions: [String: String]
    var motion: SkinMotion
}

/// The only fonts a skin may name (raw value = the manifest spelling). Nothing is bundled.
enum SkinFamily: String, CaseIterable {
    case sfPro = "SF Pro", sfProRounded = "SF Pro Rounded", sfMono = "SF Mono", newYork = "New York"
    case georgia = "Georgia", courierNew = "Courier New", menlo = "Menlo"
    case helveticaNeue = "Helvetica Neue", avenirNext = "Avenir Next"
}

/// `"Family"`, `"Family/weight"` or `"Family/weight/italic"` in a manifest.
struct SkinFace: Equatable {
    var family: SkinFamily
    var weight: Font.Weight = .regular
    var italic = false

    static let weights: [String: Font.Weight] = [
        "ultraLight": .ultraLight, "thin": .thin, "light": .light, "regular": .regular, "medium": .medium,
        "semibold": .semibold, "bold": .bold, "heavy": .heavy, "black": .black,
    ]

    /// nil when the family is not allowed, the weight is unknown or there is trailing junk.
    init?(_ spec: String) {
        let parts = spec.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
        guard (1...3).contains(parts.count), let family = SkinFamily(rawValue: parts[0]) else { return nil }
        self.family = family
        if parts.count > 1 {
            guard let weight = Self.weights[parts[1]] else { return nil }
            self.weight = weight
        }
        if parts.count > 2 {
            guard parts[2] == "italic" else { return nil }
            italic = true
        }
    }

    init(_ family: SkinFamily, _ weight: Font.Weight = .regular, italic: Bool = false) {
        self.family = family; self.weight = weight; self.italic = italic
    }

    /// The system design a family maps to; nil for the named (installed) fonts.
    private var design: Font.Design? {
        switch family {
        case .sfPro: return .default
        case .sfProRounded: return .rounded
        case .sfMono: return .monospaced
        case .newYork: return .serif
        default: return nil
        }
    }

    func font(size: CGFloat) -> Font {
        let base = design.map { Font.system(size: size, weight: weight, design: $0) }
            ?? Font.custom(family.rawValue, size: size).weight(weight)
        return italic ? base.italic() : base
    }

    /// Same face at a text style: system families keep Dynamic-Type-style sizing, others use the macOS scale.
    func font(style: Font.TextStyle) -> Font {
        guard let design else { return font(size: Font.whimbrelPointSize(for: style)) }
        let base = Font.system(style, design: design).weight(weight)
        return italic ? base.italic() : base
    }
}

enum SkinCaps: String, CaseIterable {
    case none, upper, small

    /// The text a section label shows; `small` is drawn with small-cap glyphs, so the text itself is unchanged.
    func apply(_ text: String) -> String { self == .upper ? text.uppercased() : text }
}

struct SkinTypography: Equatable {
    var display, body, numerals, label: SkinFace
    var caps: SkinCaps
}

enum SkinBorder: String, CaseIterable { case none, solid, dashed, double }

enum SkinDensity: String, CaseIterable {
    case compact, regular, airy
    var scale: CGFloat { self == .compact ? 0.75 : self == .regular ? 1 : 1.25 }
}

struct SkinShape: Equatable {
    static let radiusRange: ClosedRange<CGFloat> = 0...24
    static let widthRange: ClosedRange<CGFloat> = 0...4
    /// Card corner radius.
    var radius: CGFloat
    var border: SkinBorder
    var borderWidth: CGFloat
    var density: SkinDensity
    /// Inputs, editors and banners: two thirds of the card radius (12 -> 8).
    var controlRadius: CGFloat { (radius * 2 / 3).rounded() }
}

enum SkinIconRole: String, CaseIterable { case today, meetings, routines, settings, ask, person, time, prep, recap }

struct SkinIcons: Equatable {
    var symbols: [SkinIconRole: String]
    subscript(_ role: SkinIconRole) -> String { symbols[role] ?? "questionmark" }
}

/// Header band, divider and texture styles each archetype knows how to draw.
enum SkinHeaderBand: String, CaseIterable { case none, rule, block, stripesDiagonal = "stripes-diagonal" }
enum SkinDivider: String, CaseIterable { case none, hairline, chalk, double, dotted }
enum SkinTexture: String, CaseIterable { case none, paper, grain, grid }
enum SkinImagePlacement: String, CaseIterable { case header, background, badge }

struct SkinImage: Equatable {
    var asset: String
    var placement: SkinImagePlacement
    /// Imagery marked text-free is never placed under text.
    var textFree: Bool
}

struct SkinDecoration: Equatable {
    var headerBand: SkinHeaderBand = .none
    var dividers: SkinDivider = .none
    var texture: SkinTexture = .none
    var imagery: [SkinImage] = []
}

/// The closed catalogue of strings a skin may reword (raw value = the manifest key).
enum SkinVoiceKey: String, CaseIterable {
    case greetingMorning, greetingAfternoon, greetingEvening
    case upNextHeader, upcomingHeader, earlierHeader, recapHeader
    case emptyDay, emptyEarlier, askPlaceholder, tagline

    /// Greetings are measured after `{name}` is filled with a 30-character name.
    static let measuredNameLength = 30

    var limit: Int {
        switch self {
        // 48: Whimbrel's own "Good afternoon, {name}" is 46 characters with a 30-character name.
        case .greetingMorning, .greetingAfternoon, .greetingEvening: return 48
        case .askPlaceholder: return 40
        case .upNextHeader, .upcomingHeader, .earlierHeader, .recapHeader: return 24
        case .emptyDay, .emptyEarlier: return 80
        case .tagline: return 60
        }
    }

    func fits(_ value: String) -> Bool {
        !value.isEmpty && value.replacingOccurrences(of: "{name}", with: String(repeating: "x", count: Self.measuredNameLength)).count <= limit
    }
}

struct SkinVoice: Equatable {
    var values: [SkinVoiceKey: String]
    func text(_ key: SkinVoiceKey, name: String) -> String {
        (values[key] ?? "").replacingOccurrences(of: "{name}", with: name)
    }
}

enum SkinArchetype: String, CaseIterable { case standard, terminal, glass, field, almanac, reference }

enum SkinMotion: String, CaseIterable { case none, subtle }

/// The Whimbrel skin: today's look. `skins/whimbrel.json` mirrors it (guarded by a test).
enum SkinDefaults {
    static let whimbrel = Skin(
        id: "whimbrel", name: "Whimbrel", author: "Whimbrel",
        paletteIds: ["default", "slate", "sepia"],
        typography: SkinTypography(
            display: SkinFace(.newYork, .semibold), body: SkinFace(.sfPro),
            numerals: SkinFace(.sfPro), label: SkinFace(.sfPro), caps: .upper),
        shape: SkinShape(radius: 12, border: .solid, borderWidth: 1, density: .regular),
        icons: SkinIcons(symbols: [
            .today: "sun.max", .meetings: "person.2", .routines: "bolt", .settings: "gearshape",
            .ask: "arrow.up.circle.fill", .person: "person", .time: "clock", .prep: "doc.text", .recap: "text.alignleft",
        ]),
        decoration: SkinDecoration(),
        voice: SkinVoice(values: [
            .greetingMorning: "Good morning, {name}", .greetingAfternoon: "Good afternoon, {name}",
            .greetingEvening: "Good evening, {name}",
            .upNextHeader: "Up next", .upcomingHeader: "Today", .earlierHeader: "Earlier", .recapHeader: "Daily recap",
            .emptyDay: "No meetings yet", .emptyEarlier: "Nothing yet today",
            .askPlaceholder: "Ask Whimbrel about your meetings", .tagline: "Your meetings, remembered.",
        ]),
        archetype: .standard, archetypeOptions: [:], motion: .none)
}
