import XCTest
@testable import Whimbrel

final class ThemeLoaderTests: XCTestCase {
    /// <repo>/themes, found from this file's path (apps/macos/WhimbrelTests/ThemeLoaderTests.swift).
    private var themes: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("themes")
    }

    private func load(_ relative: String) throws -> Data {
        try Data(contentsOf: themes.appendingPathComponent(relative))
    }

    // MARK: corpus

    func testMalformedJsonNoIdNoVariantsAndBadHexAreSkipped() throws {
        for file in ["not-json.json", "no-id.json", "no-variants.json", "bad-hex.json"] {
            XCTAssertNil(ThemeLoader.parse(try load("invalid/\(file)")), "\(file) must be skipped")
        }
    }

    func testALowContrastVariantIsReplacedByDefaultsOtherwiseTheThemeLoads() throws {
        let theme = try XCTUnwrap(ThemeLoader.parse(try load("invalid/low-contrast.json")))
        XCTAssertEqual(theme.light, ThemeDefaults.light, "the failing light variant is replaced by Default's")
        XCTAssertEqual(theme.dark[.backgroundPage], 0x000000, "the passing dark variant is kept")
        XCTAssertEqual(theme.dark[.textPrimary], 0xFFFFFF)
        XCTAssertEqual(theme.dark[.accentPrimary], ThemeDefaults.dark[.accentPrimary], "missing tokens fall back to Default")
    }

    func testMissingTokensFallBackToDefaultAndASingleVariantServesBoth() throws {
        let theme = try XCTUnwrap(ThemeLoader.parse(try load("invalid/missing-tokens.json")))
        var expected = ThemeDefaults.light
        expected[.accentPrimary] = 0x336699
        XCTAssertEqual(theme.light, expected)
        XCTAssertEqual(theme.dark, theme.light, "only a light variant was shipped: it serves dark mode too")
    }

    func testADarkOnlyThemeServesBothAppearances() throws {
        let theme = try XCTUnwrap(ThemeLoader.parse(try load("invalid/single-variant.json")))
        XCTAssertEqual(theme.light, theme.dark)
        XCTAssertEqual(theme.light[.backgroundPage], 0x101010)
        XCTAssertEqual(theme.light[.link], ThemeDefaults.dark[.link], "its missing tokens come from Default's dark variant")
    }

    func testALegacyTwelveTokenThemeLoadsAndUsesDefaultsNewTokens() throws {
        let theme = try XCTUnwrap(ThemeLoader.parse(try load("invalid/legacy-12-token.json")))
        XCTAssertEqual(theme.light[.accentPrimary], 0x1F6F4A)
        for token in [ThemeToken.selection, .highlight, .textOnAccent] {
            XCTAssertEqual(theme.light[token], ThemeDefaults.light[token], "\(token)")
        }
    }

    func testIdsMustBeLowercaseLettersDigitsAndHyphens() {
        let ok = ##"{"id":"my-theme-2","name":"Ok","author":"x","light":{"backgroundPage":"#FFFFFF"}}"##
        let bad = ##"{"id":"My Theme","name":"Bad","author":"x","light":{"backgroundPage":"#FFFFFF"}}"##
        XCTAssertNotNil(ThemeLoader.parse(Data(ok.utf8)))
        XCTAssertNil(ThemeLoader.parse(Data(bad.utf8)))
    }

    // MARK: bundled themes

    func testEveryBundledThemeLoadsAndPassesTheContrastGuardWithAllFifteenTokens() throws {
        for id in ["default", "slate", "sepia", "high-contrast"] {
            let theme = try XCTUnwrap(ThemeLoader.parse(try load("\(id).json")), id)
            XCTAssertEqual(theme.id, id)
            for palette in [theme.light, theme.dark] {
                XCTAssertEqual(Set(palette.keys), Set(ThemeToken.allCases), id)
                XCTAssertEqual(ThemeContrast.failures(palette), [], "\(id) must pass the contrast guard")
            }
        }
    }

    func testLoadDirectoryListsDefaultFirstAndSkipsBrokenFilesAndDuplicateIds() {
        let themes = ThemeLoader.loadDirectory(self.themes)
        XCTAssertEqual(themes.first?.id, "default")
        XCTAssertEqual(Set(themes.map(\.id)), ["default", "slate", "sepia", "high-contrast"], "invalid/ is not scanned")
        XCTAssertEqual(themes.count, Set(themes.map(\.id)).count)
        XCTAssertEqual(ThemeLoader.loadDirectory(URL(fileURLWithPath: "/nonexistent/themes")).map(\.id), ["default"], "a missing folder still yields Default")
    }

    // MARK: contrast

    func testContrastRatioMatchesWcagKnownValues() {
        XCTAssertEqual(ThemeContrast.ratio(0x000000, 0xFFFFFF), 21, accuracy: 0.01)
        XCTAssertEqual(ThemeContrast.ratio(0xFFFFFF, 0xFFFFFF), 1, accuracy: 0.001)
        XCTAssertEqual(ThemeContrast.ratio(0xFFFFFF, 0xC97B54), 3.26, accuracy: 0.02, "today's dark primary button text")
        XCTAssertEqual(ThemeContrast.ratio(0x1C1712, 0xC97B54), 5.46, accuracy: 0.02)
    }

    func testTheGuardNamesTheFailingPairs() {
        var palette = ThemeDefaults.light
        palette[.textOnAccent] = 0xE0E0E0
        let failures = ThemeContrast.failures(palette)
        XCTAssertEqual(failures.count, 1)
        XCTAssertTrue(failures[0].contains("textOnAccent"), failures[0])
    }
}
