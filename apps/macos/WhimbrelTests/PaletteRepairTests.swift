import XCTest
@testable import Whimbrel

final class PaletteRepairTests: XCTestCase {
    /// SplitMix64, so the 200 palettes are the same on every run.
    private struct Seeded: RandomNumberGenerator {
        var state: UInt64
        mutating func next() -> UInt64 {
            state &+= 0x9E37_79B9_7F4A_7C15
            var z = state
            z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
            z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
            return z ^ (z >> 31)
        }
    }

    /// 8-bit chroma (max - min channel); below about 120 the hue of a rounded colour can drift over 1°.
    private func chroma(_ c: UInt32) -> UInt32 {
        let channels = [(c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF]
        return channels.max()! - channels.min()!
    }

    func testRepairedPalettesAlwaysPass() {
        var rng = Seeded(state: 2026_10_21)
        var huesChecked = 0
        for _ in 0..<200 {
            var palette: ThemePalette = [:]
            for token in ThemeToken.allCases { palette[token] = UInt32.random(in: 0...0xFFFFFF, using: &rng) }
            let (repaired, changed) = PaletteRepair.repair(palette)
            XCTAssertTrue(ArchetypeContrast.passes(repaired), ArchetypeContrast.failures(repaired).joined(separator: "; "))
            for token in ThemeToken.allCases where !changed.contains(token) {
                XCTAssertEqual(repaired[token], palette[token], "\(token) changed without being reported")
            }
            for token in changed where chroma(palette[token]!) >= 120 && chroma(repaired[token]!) >= 120 {
                let before = PaletteRepair.hsl(palette[token]!).h, after = PaletteRepair.hsl(repaired[token]!).h
                let diff = abs(before - after)
                XCTAssertLessThanOrEqual(min(diff, 360 - diff), 1, "\(token) hue moved")
                huesChecked += 1
            }
        }
        XCTAssertGreaterThan(huesChecked, 50, "too few changed colours kept enough chroma to measure hue")
    }

    /// The spec's example: Steelers gold (#FFB612) as the accent on a white page is about 1.8:1, yet passes
    /// `ThemeContrast.requiredPairs`. Repair must reach 4.5:1 on every pair any archetype draws, keeping the hue.
    func testSteelersGoldOnWhiteIsRepairedOnEveryPair() throws {
        var palette = ThemeDefaults.theme.light
        palette[.backgroundPage] = 0xFFFFFF
        palette[.backgroundCard] = 0xFFFFFF
        palette[.accentPrimary] = 0xFFB612
        XCTAssertFalse(ArchetypeContrast.passes(palette))
        let (repaired, changed) = PaletteRepair.repair(palette)
        XCTAssertEqual(ArchetypeContrast.failures(repaired), [])
        XCTAssertTrue(changed.contains(.accentPrimary))
        let gold = try XCTUnwrap(repaired[.accentPrimary])
        for bg in [ThemeToken.backgroundPage, .backgroundCard] {
            XCTAssertGreaterThanOrEqual(ThemeContrast.ratio(gold, try XCTUnwrap(repaired[bg])), ThemeContrast.minimum)
        }
        let diff = abs(PaletteRepair.hsl(gold).h - PaletteRepair.hsl(0xFFB612).h)
        XCTAssertLessThanOrEqual(min(diff, 360 - diff), 2, "still gold")
    }

    /// Default passes the theme guard (not every archetype: its secondary text on Glass's glow is about 3.4:1).
    func testPassingPaletteUnchanged() {
        let guardPairs = ThemeContrast.requiredPairs.map { ArchetypeContrast.plain($0.0, on: $0.1) }
        for palette in [ThemeDefaults.theme.light, ThemeDefaults.theme.dark] {
            let (repaired, changed) = PaletteRepair.repair(palette, pairs: guardPairs)
            XCTAssertTrue(changed.isEmpty)
            XCTAssertEqual(repaired, palette)
        }
    }
}
