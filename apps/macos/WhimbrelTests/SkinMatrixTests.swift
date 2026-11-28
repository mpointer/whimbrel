import AppKit
import SwiftUI
import WhimbrelUI
import XCTest
@testable import Whimbrel

private final class WidthBox { var width: CGFloat = 0 }
private struct WidthKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// Test-only mirror of `translate.py pseudo` (en-XA): accented, 40% longer, bracketed; placeholders and
/// `{tokens}` unchanged. The matrix applies it to skin voice, so it needs no catalog at runtime.
enum Pseudo {
    private static let placeholder = try! NSRegularExpression(
        pattern: #"%(?:\d+\$)?[-+ #0]*\d*(?:\.\d+)?(?:hh|h|ll|l|q|z|t|j|L)?[@dDiuUxXoOfFeEgGcCsSpaA]|\{\w+\}"#)
    private static let accents = Dictionary(uniqueKeysWithValues: zip("aceinousyACEINOUSY", "åçéîñöûšýÅÇÉÎÑÖÛŠÝ"))

    static func apply(_ s: String) -> String {
        let ns = s as NSString
        var out = "", last = 0
        func accent(_ part: String) -> String { String(part.map { accents[$0] ?? $0 }) }
        for m in placeholder.matches(in: s, range: NSRange(location: 0, length: ns.length)) {
            out += accent(ns.substring(with: NSRange(location: last, length: m.range.location - last))) + ns.substring(with: m.range)
            last = m.range.location + m.range.length
        }
        out += accent(ns.substring(from: last))
        return "[" + out + String(repeating: "~", count: (s.count * 2 + 4) / 5) + "]"
    }
}

/// Every installed skin x each of its palettes x light/dark, against Home's content and widths.
@MainActor
final class SkinMatrixTests: XCTestCase {
    private let repo = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()

    /// Home's content width at the 900pt window (660 strict, 672 ideal sidebar, 692 minimum sidebar), the 700 and 712
    /// seams, and 760.
    private let widths: [CGFloat] = [660, 672, 692, 700, 712, 760]

    private var skins: [Skin] {
        var seen = Set<String>()
        return ([SkinDefaults.whimbrel] + SkinLoader.loadDirectory(repo.appendingPathComponent("skins"))).filter { seen.insert($0.id).inserted }
    }

    /// Every (skin, palette) with the real palettes installed in the shared theme store for the duration.
    private func combos() -> [(skin: Skin, palette: Theme)] {
        let palettes = ThemeLoader.loadDirectory(repo.appendingPathComponent("themes")) + ThemeLoader.loadDirectory(repo.appendingPathComponent("skins/palettes"))
        let store = ThemeStore.shared
        let before = store.themes
        store.install(before + palettes)
        addTeardownBlock { store.install(before) }
        let all = before + palettes
        return skins.flatMap { skin -> [(Skin, Theme)] in
            XCTAssertFalse(skin.paletteIds.isEmpty, "\(skin.id) names no palette")
            let found = skin.paletteIds.compactMap { id in all.last { $0.id == id } }
            XCTAssertEqual(found.count, skin.paletteIds.count, "\(skin.id): a manifest palette does not resolve")
            return found.map { (skin, $0) }
        }
    }

    /// The full sweep (every palette at every width) only with WHIMBREL_FULL_MATRIX=1 (pass TEST_RUNNER_WHIMBREL_FULL_MATRIX=1).
    private var fullMatrix: Bool { ProcessInfo.processInfo.environment["WHIMBREL_FULL_MATRIX"] == "1" }
    private let contents = [TodayContent.sample, .worstCase, .empty]

    /// The real Home (`WHomeView`, the model the Home screen draws) for `content`, under the skin and palette in force.
    private func home(_ content: TodayContent) -> some View {
        let skin = SkinStore.shared.selected
        let model = HomeRedesignAdapter.model(
            content: content, data: HomeRedesignData(), memory: .on, elapsedSeconds: nil, now: content.now, voice: skin.voice,
            elements: WHomeElements.resolve(options: skin.archetypeOptions, archetype: skin.archetype.rawValue))
        return WHomeView(model: model, actions: WHomeActions()).redesignThemed()
    }

    private func renderedWidth<V: View>(_ view: V, proposing width: CGFloat, dark: Bool) -> CGFloat {
        let box = WidthBox()
        let probe = view
            .fixedSize(horizontal: false, vertical: true)
            .background(GeometryReader { Color.clear.preference(key: WidthKey.self, value: $0.size.width) })
            .frame(width: width, alignment: .leading)
            .onPreferenceChange(WidthKey.self) { box.width = $0 }
        let host = NSHostingView(rootView: probe)
        host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        host.frame = CGRect(x: 0, y: 0, width: width, height: 6000)
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        host.layoutSubtreeIfNeeded()
        return box.width
    }

    func test_matrix_covers_every_skin_and_palette() {
        let c = combos()
        XCTAssertEqual(Set(c.map(\.skin.id)).count, 8, "every shipped skin, Thread included")
        XCTAssertGreaterThan(c.count, 10)
    }

    /// Palette and appearance cannot change a measured width, so: each skin at every width once, then each palette in
    /// light and dark at the two extreme widths. The full cross product runs with WHIMBREL_FULL_MATRIX=1.
    func test_matrix_no_overflow() throws {
        let all = combos()
        var cells = 0
        func check(_ skin: Skin, _ palette: Theme, dark: Bool, width: CGFloat) {
            SkinStore.shared.withOverride(skin: skin, paletteId: palette.id) {
                for content in contents {
                    let measured = renderedWidth(home(content), proposing: width, dark: dark)
                    XCTAssertGreaterThan(measured, 0, "the probe measured nothing")
                    XCTAssertLessThanOrEqual(measured, width + 0.5, "\(skin.id)/\(palette.id) \(dark ? "dark" : "light") at \(width)")
                    cells += 1
                }
            }
        }
        if fullMatrix {
            for (skin, palette) in all { for dark in [false, true] { for width in widths { check(skin, palette, dark: dark, width: width) } } }
            XCTAssertEqual(cells, all.count * 2 * widths.count * contents.count)
        } else {
            for skin in skins { for width in widths { check(skin, all.first { $0.skin.id == skin.id }!.palette, dark: false, width: width) } }
            for (skin, palette) in all { for dark in [false, true] { for width in [widths.first!, widths.last!] { check(skin, palette, dark: dark, width: width) } } }
            XCTAssertEqual(cells, skins.count * widths.count * contents.count + all.count * 2 * 2 * contents.count)
        }
        XCTAssertEqual(all.count, skins.map(\.paletteIds.count).reduce(0, +))
    }

    func test_pseudo_apply_matches_translate_py() {
        XCTAssertEqual(Pseudo.apply("Good morning, {name}"), "[Gööd mörñîñg, {name}~~~~~~~~]")
        XCTAssertEqual(Pseudo.apply("%lld days"), "[%lld dåýš~~~~]")
    }

    /// The locale axis: en (above) and en-XA, every skin's voice pseudo-localized, at the two extreme widths.
    func test_pseudo_locale_fits_every_skin() {
        let all = combos()
        for base in skins {
            var skin = base
            skin.voice.values = skin.voice.values.mapValues(Pseudo.apply)
            skin.voice.localized = [:]
            let palette = all.first { $0.skin.id == base.id }!.palette
            SkinStore.shared.withOverride(skin: skin, paletteId: palette.id) {
                for width in [widths.first!, widths.last!] {
                    for content in contents {
                        let measured = renderedWidth(home(content), proposing: width, dark: false)
                        XCTAssertGreaterThan(measured, 0, "the probe measured nothing")
                        XCTAssertLessThanOrEqual(measured, width + 0.5, "en-XA \(skin.id) at \(width)")
                    }
                }
            }
        }
    }

    /// The real translations: every skin's voice in each shipped language at the two extreme widths.
    func test_translated_voice_fits_every_skin() {
        let all = combos()
        for lang in ["es", "fr", "fr-CA", "de", "pt"] {
            for base in skins {
                var skin = base
                skin.voice.values = Dictionary(uniqueKeysWithValues: SkinVoiceKey.allCases.map { ($0, base.voice.template($0, lang: lang)) })
                skin.voice.localized = [:]
                let palette = all.first { $0.skin.id == base.id }!.palette
                SkinStore.shared.withOverride(skin: skin, paletteId: palette.id) {
                    for width in [widths.first!, widths.last!] {
                        for content in contents {
                            let measured = renderedWidth(home(content), proposing: width, dark: false)
                            XCTAssertLessThanOrEqual(measured, width + 0.5, "\(lang) \(skin.id) at \(width)")
                        }
                    }
                }
            }
        }
    }

    /// Every skin and palette as a user skin with a header, background and badge image, light and dark, at every
    /// width: still no overflow, and every text colour over the fixture image reaches 4.5:1 under its scrim.
    func test_matrix_no_overflow_with_imagery() throws {
        let fixture = try SkinImageryFixture(self)
        let all = combos()
        var cells = 0
        for (base, palette) in all {
            let skin = try fixture.skin(from: base)
            for dark in [false, true] {
                SkinStore.shared.withOverride(skin: skin, paletteId: palette.id) {
                    for width in widths {
                        for content in contents {
                            let measured = renderedWidth(home(content), proposing: width, dark: dark)
                            XCTAssertGreaterThan(measured, 0, "the probe measured nothing")
                            XCTAssertLessThanOrEqual(measured, width + 0.5, "\(skin.id)/\(palette.id) with imagery \(dark ? "dark" : "light") at \(width)")
                            cells += 1
                        }
                    }
                    let colors = SkinImageryView.resolvedColors(isDark: dark)
                    let image = SkinImageryFixture.image
                    let opacity = SkinImageryView(skinImage: skin.decoration.imagery[0], image: image).opacity(isDark: dark)
                    let (lightest, darkest) = SkinImageryView.extremes(of: image, page: colors.scrim)
                    // Every colour Home draws text in, not just textPrimary.
                    for token in [ThemeToken.textPrimary, .textSecondary, .accentPrimary, .error, .link] {
                        let text = ThemeStore.shared.hex(token, isDark: dark)
                        for under in [lightest, darkest] {
                            let shown = SkinImageryView.blend(under, over: colors.scrim, opacity: opacity)
                            XCTAssertGreaterThanOrEqual(
                                ThemeContrast.ratio(text, shown), ThemeContrast.minimum, "\(skin.id)/\(palette.id) \(dark ? "dark" : "light") \(token) over the image"
                            )
                        }
                    }
                }
            }
        }
        XCTAssertEqual(cells, all.count * 2 * widths.count * contents.count)
    }

    /// Every catalogue string at its length limit, in wide glyphs with word breaks. Greetings are `<filler>, {name}`,
    /// which is the limit once `{name}` holds a 30-character name.
    static let longestVoice: SkinVoice = {
        func filler(_ count: Int) -> String { String(String(repeating: "MWMW ", count: count).prefix(count)) }
        var values: [SkinVoiceKey: String] = [:]
        for key in SkinVoiceKey.allCases {
            switch key {
            case .greetingMorning, .greetingAfternoon, .greetingEvening:
                values[key] = filler(key.limit - SkinVoiceKey.measuredNameLength - 2) + ", {name}"
            default:
                values[key] = filler(key.limit)
            }
        }
        return SkinVoice(values: values)
    }()

    /// A 30-character user name and each skin's voice replaced by the longest values the catalogue allows, at every width.
    func test_matrix_no_overflow_with_longest_name_and_voice() throws {
        let all = combos()
        let name = "Bartholomew Featherstonehaughs"
        XCTAssertEqual(name.count, SkinVoiceKey.measuredNameLength)
        for key in SkinVoiceKey.allCases {
            let value = try XCTUnwrap(Self.longestVoice.values[key])
            XCTAssertTrue(key.fits(value), "\(key) must respect its limit")
            XCTAssertEqual(Self.longestVoice.text(key, name: name).count, key.limit, "\(key) must be at its limit")
        }
        let contents = [TodayContent.sample, .worstCase, .empty].map { content -> TodayContent in
            var named = content
            named.name = name
            return named
        }
        var cells = 0
        for skin in skins {
            var stressed = skin
            stressed.voice = Self.longestVoice
            let palette = try XCTUnwrap(all.first { $0.skin.id == skin.id }).palette
            SkinStore.shared.withOverride(skin: stressed, paletteId: palette.id) {
                for width in widths {
                    for content in contents {
                        let measured = renderedWidth(home(content), proposing: width, dark: false)
                        XCTAssertGreaterThan(measured, 0, "the probe measured nothing")
                        XCTAssertLessThanOrEqual(measured, width + 0.5, "\(skin.id) with the longest voice at \(width)")
                        cells += 1
                    }
                }
            }
        }
        XCTAssertEqual(cells, skins.count * widths.count * contents.count)
    }

    /// The widest face a manifest may name: every family at its regular, bold and black weights, measured once.
    private static let widestFace: SkinFace = {
        let sample = "MWMW Wednesday 10:45 AM"
        var best = (face: SkinFace(.sfPro), width: CGFloat(0))
        for family in SkinFamily.allCases {
            for weight in [Font.Weight.regular, .bold, .black] {
                let face = SkinFace(family, weight)
                let width = NSHostingView(rootView: Text(sample).font(face.font(size: 20)).fixedSize()).fittingSize.width
                if width > best.width { best = (face, width) }
            }
        }
        return best.face
    }()

    /// The spec's worst-case user manifest: the longest voice, the widest face everywhere, upper-case labels, airy
    /// density and the thickest double border, as each archetype, with a 30-character name, at every width.
    func test_matrix_no_overflow_with_worst_case_user_manifest() throws {
        let palette = try XCTUnwrap(combos().first).palette
        let name = "Bartholomew Featherstonehaughs"
        let contents = [TodayContent.sample, .worstCase, .empty].map { content -> TodayContent in
            var named = content
            named.name = name
            return named
        }
        let face = Self.widestFace
        var cells = 0
        for archetype in SkinArchetype.allCases {
            var skin = SkinDefaults.whimbrel
            skin.id = "user-worst-\(archetype.rawValue)"
            skin.archetype = archetype
            skin.voice = Self.longestVoice
            skin.typography = SkinTypography(display: face, body: face, numerals: face, label: face, caps: .upper)
            skin.shape = SkinShape(radius: SkinShape.radiusRange.upperBound, border: .double, borderWidth: SkinShape.widthRange.upperBound, density: .airy)
            SkinStore.shared.withOverride(skin: skin, paletteId: palette.id) {
                for width in widths {
                    for content in contents {
                        let measured = renderedWidth(home(content), proposing: width, dark: false)
                        XCTAssertGreaterThan(measured, 0, "the probe measured nothing")
                        XCTAssertLessThanOrEqual(measured, width + 0.5, "worst-case \(archetype) in \(face.family.rawValue) at \(width)")
                        cells += 1
                    }
                }
            }
        }
        XCTAssertEqual(cells, SkinArchetype.allCases.count * widths.count * contents.count)
    }

    func test_every_skin_palette_passes_contrast() {
        for (skin, palette) in combos() {
            XCTAssertEqual(ThemeContrast.failures(palette.light), [], "\(skin.id)/\(palette.id) light")
            XCTAssertEqual(ThemeContrast.failures(palette.dark), [], "\(skin.id)/\(palette.id) dark")
        }
    }
}
