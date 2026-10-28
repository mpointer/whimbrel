import Foundation

/// Makes a palette pass `ArchetypeContrast` (every pair any archetype draws, by default) without dropping it
/// (Skins Release 2: a generated palette is repaired, not replaced). Each text colour of a failing pair moves
/// its HSL lightness in 2% steps, hue and saturation kept, to the nearest value that passes on all of its
/// surfaces. When none does (its surfaces sit at both ends), the text goes to the end that suits most of them
/// and each surface still failing has its base colour step away from it the same way.
enum PaletteRepair {
    private static let step = 0.02

    static func repair(_ palette: ThemePalette, pairs: [ArchetypeContrast.Pair] = ArchetypeContrast.all) -> (palette: ThemePalette, changed: [ThemeToken]) {
        var p = palette
        var changed: [ThemeToken] = []
        func mark(_ token: ThemeToken) { if !changed.contains(token) { changed.append(token) } }
        func passes(_ text: UInt32, _ pair: ArchetypeContrast.Pair, in palette: ThemePalette) -> Bool {
            pair.surface(palette).map { ThemeContrast.ratio(text, $0) >= ThemeContrast.minimum } ?? true
        }

        var texts: [ThemeToken] = []
        for pair in pairs where !texts.contains(pair.text) { texts.append(pair.text) }
        // A colour moved for one pair can break another (the accent is text and a surface); later rounds settle it.
        for _ in 0..<12 where !ArchetypeContrast.passes(p, pairs) {
            for text in texts {
                guard let colour = p[text] else { continue }
                let mine = pairs.filter { $0.text == text }
                if mine.allSatisfy({ passes(colour, $0, in: p) }) { continue }
                // Hue and saturation always come from the requested colour, so repeated moves never drift it.
                let requested = palette[text] ?? colour
                if let fixed = nearest(requested, where: { c in mine.allSatisfy { passes(c, $0, in: p) } }) {
                    p[text] = fixed
                    mark(text)
                    continue
                }
                let black = withLightness(requested, 0), white = withLightness(requested, 1)
                let darkText = mine.filter { passes(black, $0, in: p) }.count >= mine.filter { passes(white, $0, in: p) }.count
                let end = darkText ? black : white
                p[text] = end
                mark(text)
                for pair in mine where !passes(end, pair, in: p) {
                    guard let b = p[pair.base] else { continue }
                    p[pair.base] = away(b, hue: palette[pair.base] ?? b, darker: !darkText) { candidate in
                        var q = p
                        q[pair.base] = candidate
                        return passes(end, pair, in: q)
                    }
                    mark(pair.base)
                }
            }
        }
        return (p, changed)
    }

    /// The colour at the nearest lightness (2% steps either way) that satisfies `ok`, or nil.
    private static func nearest(_ colour: UInt32, where ok: (UInt32) -> Bool) -> UInt32? {
        let l = hsl(colour).l
        for k in 1...Int(1 / step) {
            for target in [l - Double(k) * step, l + Double(k) * step] {
                let c = withLightness(colour, min(max(target, 0), 1))
                if ok(c) { return c }
            }
        }
        return nil
    }

    /// Steps `colour`'s lightness toward black (`darker`) or white, in `hue`'s hue and saturation, until `ok`; the end
    /// itself always passes against the opposite end.
    private static func away(_ colour: UInt32, hue: UInt32, darker: Bool, until ok: (UInt32) -> Bool) -> UInt32 {
        var l = hsl(colour).l
        while true {
            l = darker ? max(l - step, 0) : min(l + step, 1)
            let c = withLightness(hue, l)
            if ok(c) || l == 0 || l == 1 { return c }
        }
    }

    static func withLightness(_ colour: UInt32, _ l: Double) -> UInt32 {
        let (h, s, _) = hsl(colour)
        return rgb(h: h, s: s, l: l)
    }

    static func hsl(_ colour: UInt32) -> (h: Double, s: Double, l: Double) {
        let r = Double((colour >> 16) & 0xFF) / 255, g = Double((colour >> 8) & 0xFF) / 255, b = Double(colour & 0xFF) / 255
        let mx = max(r, g, b), mn = min(r, g, b), l = (mx + mn) / 2, d = mx - mn
        guard d > 0 else { return (0, 0, l) }
        let s = min(d / (1 - abs(2 * l - 1)), 1)
        var h: Double
        if mx == r { h = (g - b) / d } else if mx == g { h = (b - r) / d + 2 } else { h = (r - g) / d + 4 }
        h *= 60
        if h < 0 { h += 360 }
        return (h, s, l)
    }

    private static func rgb(h: Double, s: Double, l: Double) -> UInt32 {
        let c = (1 - abs(2 * l - 1)) * s
        let x = c * (1 - abs((h / 60).truncatingRemainder(dividingBy: 2) - 1))
        let m = l - c / 2
        let parts: (Double, Double, Double)
        switch Int(h / 60) % 6 {
        case 0: parts = (c, x, 0)
        case 1: parts = (x, c, 0)
        case 2: parts = (0, c, x)
        case 3: parts = (0, x, c)
        case 4: parts = (x, 0, c)
        default: parts = (c, 0, x)
        }
        func byte(_ v: Double) -> UInt32 { UInt32(min(max(((v + m) * 255).rounded(), 0), 255)) }
        return byte(parts.0) << 16 | byte(parts.1) << 8 | byte(parts.2)
    }
}

/// Every text-on-surface pair Today draws, per archetype: `ThemeContrast.requiredPairs`, `error` on the page
/// and cards everywhere, accent (and Reference's link) text, and the blended surfaces of Glass (its card over
/// the material, its glow) and Field (the striped pitch). Release 1 proved these only for the bundled palettes;
/// their archetype tests and the user-palette repair (`all`, since a user skin can take any archetype) share it.
enum ArchetypeContrast {
    struct Pair {
        let text: ThemeToken
        /// The palette colour the surface is built on: the one a repair may move.
        let base: ThemeToken
        let name: String
        let surface: (ThemePalette) -> UInt32?
    }

    static func plain(_ text: ThemeToken, on background: ThemeToken) -> Pair {
        Pair(text: text, base: background, name: background.rawValue) { $0[background] }
    }

    private static func on(_ texts: [ThemeToken], _ backgrounds: [ThemeToken]) -> [Pair] {
        texts.flatMap { t in backgrounds.map { plain(t, on: $0) } }
    }

    private static let common = ThemeContrast.requiredPairs.map { plain($0.0, on: $0.1) } + on([.error], [.backgroundPage, .backgroundCard])

    /// Glass: the card is `backgroundCard` at 0.9 over a material that shows any backdrop (white and black bound
    /// it); the greeting and date sit straight on the glow.
    private static let glass: [Pair] = {
        let steps = 11  // `GlassStyle.glow`'s colours
        let backdrops: [(name: String, count: Int, colour: (ThemePalette, Int) -> UInt32?)] = [
            ("page", 1, { p, _ in p[.backgroundPage] }),
            ("glow", steps, { p, i in glow(p)?[i] }),
            ("accent", 1, { p, _ in p[.accentPrimary] }),
            ("white", 1, { _, _ in 0xFFFFFF }),
            ("black", 1, { _, _ in 0 }),
        ]
        var pairs: [Pair] = []
        for backdrop in backdrops {
            for i in 0..<backdrop.count {
                for text in [ThemeToken.textPrimary, .textSecondary, .error] {
                    let name = backdrop.count == 1 ? "glass card over \(backdrop.name)" : "glass card over \(backdrop.name) \(i)"
                    pairs.append(Pair(text: text, base: .backgroundCard, name: name) { p in
                        guard let under = backdrop.colour(p, i), let card = p[.backgroundCard] else { return nil }
                        return GlassStyle.composite(card, alpha: GlassStyle.cardOpacity, over: under)
                    })
                }
            }
        }
        for i in 0..<steps {
            for text in [ThemeToken.textPrimary, .textSecondary] {
                pairs.append(Pair(text: text, base: .backgroundPage, name: "glass glow \(i)") { glow($0)?[i] })
            }
        }
        return pairs
    }()

    private static func glow(_ p: ThemePalette) -> [UInt32]? {
        guard let accent = p[.accentPrimary], let page = p[.backgroundPage] else { return nil }
        return GlassStyle.glow(accent: accent, page: page, over: page)
    }

    /// Field: labels on the pitch (`backgroundCard` under accent stripes), where a chalk line is the worst case.
    private static let field: [Pair] = [ThemeToken.textPrimary, .textSecondary].flatMap { text in
        [
            Pair(text: text, base: .backgroundCard, name: "pitch stripe") { p in
                guard let card = p[.backgroundCard], let accent = p[.accentPrimary] else { return nil }
                return GlassStyle.composite(accent, alpha: FieldStyle.stripeOpacity, over: card)
            },
            // The chalk line dominates this surface, so a repair moves it.
            Pair(text: text, base: .border, name: "pitch stripe+line") { p in
                guard let card = p[.backgroundCard], let accent = p[.accentPrimary], let border = p[.border] else { return nil }
                let stripe = GlassStyle.composite(accent, alpha: FieldStyle.stripeOpacity, over: card)
                return GlassStyle.composite(border, alpha: FieldStyle.lineOpacity, over: stripe)
            },
        ]
    }

    static func pairs(for archetype: SkinArchetype) -> [Pair] {
        switch archetype {
        case .standard, .almanac, .terminal: common + on([.accentPrimary], [.backgroundPage, .backgroundCard])
        case .reference: common + on([.accentPrimary, .link], [.backgroundPage, .backgroundCard])
        case .glass: common + glass
        case .field: common + field
        }
    }

    /// Every archetype's pairs, once each.
    static let all: [Pair] = {
        var seen = Set<String>()
        return SkinArchetype.allCases.flatMap(pairs(for:)).filter { seen.insert("\($0.text.rawValue) on \($0.name)").inserted }
    }()

    /// One line per failing pair, e.g. `accentPrimary on backgroundPage = 1.73`.
    static func failures(_ palette: ThemePalette, _ pairs: [Pair] = all) -> [String] {
        pairs.compactMap { pair in
            guard let text = palette[pair.text], let surface = pair.surface(palette) else { return "\(pair.text.rawValue) on \(pair.name) = missing" }
            let r = ThemeContrast.ratio(text, surface)
            return r < ThemeContrast.minimum ? "\(pair.text.rawValue) on \(pair.name) = \(String(format: "%.2f", r))" : nil
        }
    }

    static func passes(_ palette: ThemePalette, _ pairs: [Pair] = all) -> Bool { failures(palette, pairs).isEmpty }
}
