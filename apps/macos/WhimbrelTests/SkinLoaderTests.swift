import AppKit
import XCTest
@testable import Whimbrel

final class SkinLoaderTests: XCTestCase {
    private let base = SkinDefaults.whimbrel

    /// <repo>/skins, found from this file's path (apps/macos/WhimbrelTests/SkinLoaderTests.swift).
    private var skins: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("skins")
    }

    private func load(_ relative: String) throws -> Data {
        try Data(contentsOf: skins.appendingPathComponent(relative))
    }

    private func parse(_ json: String) -> (Skin, repairs: [String])? {
        SkinLoader.parse(Data(json.utf8), base: base)
    }

    private func corpus(_ file: String) throws -> (Skin, repairs: [String]) {
        try XCTUnwrap(SkinLoader.parse(try load("invalid/\(file)"), base: base), "\(file) must load")
    }

    // MARK: full manifest

    func test_parses_full_manifest() throws {
        let json = """
        {
          "id": "game-day", "name": "Game Day", "author": "Whimbrel",
          "palettes": ["game-day-navy-red", "game-day-black-gold"],
          "typography": { "display": "SF Pro/heavy/italic", "body": "Avenir Next", "numerals": "SF Mono", "label": "Menlo/bold", "caps": "upper" },
          "shape": { "radius": 4, "border": "dashed", "borderWidth": 2, "density": "airy" },
          "icons": { "today": "sportscourt", "ask": "megaphone", "person": "figure.run" },
          "decoration": { "headerBand": "stripes-diagonal", "dividers": "chalk", "texture": "grid",
                          "imagery": [{ "asset": "pitch", "placement": "background", "textFree": true }] },
          "voice": { "greetingMorning": "Game day, {name}.", "upcomingHeader": "Fixtures", "askPlaceholder": "Ask the coach…" },
          "today": { "archetype": "field", "options": { "digits": "flip" } },
          "motion": "subtle"
        }
        """
        let (skin, repairs) = try XCTUnwrap(parse(json))
        XCTAssertEqual(repairs, [])
        XCTAssertEqual(skin.id, "game-day")
        XCTAssertEqual(skin.paletteIds, ["game-day-navy-red", "game-day-black-gold"])
        XCTAssertEqual(skin.typography.display, SkinFace(.sfPro, .heavy, italic: true))
        XCTAssertEqual(skin.typography.label, SkinFace(.menlo, .bold))
        XCTAssertEqual(skin.typography.caps, .upper)
        XCTAssertEqual(skin.shape, SkinShape(radius: 4, border: .dashed, borderWidth: 2, density: .airy))
        XCTAssertEqual(skin.shape.controlRadius, 3)
        XCTAssertEqual(skin.icons[.ask], "megaphone")
        XCTAssertEqual(skin.icons[.meetings], base.icons[.meetings], "unset icons keep Whimbrel's")
        XCTAssertEqual(skin.decoration.headerBand, .stripesDiagonal)
        XCTAssertEqual(skin.decoration.imagery, [SkinImage(asset: "pitch", placement: .background, textFree: true)])
        XCTAssertEqual(skin.voice.text(.greetingMorning, name: "Mike"), "Game day, Mike.")
        XCTAssertEqual(skin.voice.text(.earlierHeader, name: "Mike"), "Earlier", "unset voice keeps Whimbrel's")
        XCTAssertEqual(skin.archetype, .field)
        XCTAssertEqual(skin.archetypeOptions, ["digits": "flip"])
        XCTAssertEqual(skin.motion, .subtle)
    }

    // MARK: skipped files

    private let invalidSkipped = ["not-json.json", "no-id.json", "no-name.json", "bad-id.json"]

    func test_skips_bad_json_no_id_bad_id_duplicate_id() throws {
        for file in invalidSkipped {
            XCTAssertNil(SkinLoader.parse(try load("invalid/\(file)"), base: base), "\(file) must be skipped")
        }
        let loaded = SkinLoader.loadDirectory(skins.appendingPathComponent("invalid"), base: base)
        XCTAssertEqual(loaded.filter { $0.id == "dup" }.map(\.name), ["Dup A"], "the second file with an id is skipped")
        XCTAssertFalse(loaded.contains { $0.id == "game day" || $0.id == "no-name" })
    }

    // MARK: repairs

    func test_repairs_font_outside_allowlist() throws {
        let (skin, repairs) = try corpus("bad-font.json")
        XCTAssertEqual(skin.typography.display, base.typography.display)
        XCTAssertEqual(skin.typography.body, SkinFace(.sfMono, .light, italic: true))
        XCTAssertEqual(skin.typography.numerals, base.typography.numerals)
        XCTAssertEqual(repairs.count, 2)
        XCTAssertEqual(SkinFamily.allCases.count, 9)
    }

    func test_repairs_voice_over_limit() throws {
        let (skin, repairs) = try corpus("long-voice.json")
        XCTAssertEqual(skin.voice.values[.upNextHeader], base.voice.values[.upNextHeader])
        XCTAssertEqual(skin.voice.values[.tagline], base.voice.values[.tagline], "an empty value is repaired too")
        XCTAssertEqual(skin.voice.text(.greetingMorning, name: "Mike"), "Hello, Mike")
        XCTAssertEqual(repairs.count, 2)
    }

    func test_repairs_unknown_archetype_to_standard() throws {
        let (skin, repairs) = try corpus("bad-archetype.json")
        XCTAssertEqual(skin.archetype, .standard)
        XCTAssertEqual(skin.decoration.headerBand, .none)
        XCTAssertEqual(repairs.count, 2)
    }

    func test_clamps_radius_and_width() throws {
        let (skin, repairs) = try corpus("out-of-range.json")
        XCTAssertEqual(skin.shape.radius, 24)
        XCTAssertEqual(skin.shape.borderWidth, 0)
        XCTAssertEqual(repairs.count, 2)
    }

    func test_unknown_fields_ignored() throws {
        let (skin, repairs) = try corpus("unknown-fields.json")
        XCTAssertEqual(skin.shape.radius, 4)
        XCTAssertEqual(repairs, [])
    }

    func test_unknown_icon_symbol_falls_back() throws {
        XCTAssertNil(NSImage(systemSymbolName: "not.a.real.symbol.zzz", accessibilityDescription: nil))
        let (skin, repairs) = try corpus("bad-symbol.json")
        XCTAssertEqual(skin.icons[.today], base.icons[.today])
        XCTAssertEqual(skin.icons[.meetings], "list.bullet.rectangle")
        XCTAssertEqual(repairs.count, 1)
    }

    // MARK: corpus table and bundled skin

    /// The README's table: every corpus file is listed there and has an outcome here.
    func test_corpus_readme_lists_every_file() throws {
        let readme = String(decoding: try load("invalid/README.md"), as: UTF8.self)
        let files = try FileManager.default.contentsOfDirectory(atPath: skins.appendingPathComponent("invalid").path)
            .filter { $0.hasSuffix(".json") }
        XCTAssertEqual(files.count, 12)
        for file in files { XCTAssertTrue(readme.contains("`\(file)`"), "\(file) is missing from the README") }
    }

    func test_loadDirectory_reads_top_level_only() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("skins-\(UUID().uuidString)")
        let nested = dir.appendingPathComponent("invalid")
        try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        try Data(#"{"id":"top","name":"Top","palettes":["default"]}"#.utf8).write(to: dir.appendingPathComponent("top.json"))
        try Data(#"{"id":"nested","name":"Nested","palettes":["default"]}"#.utf8).write(to: nested.appendingPathComponent("nested.json"))
        XCTAssertEqual(SkinLoader.loadDirectory(dir, base: base).map(\.id), ["whimbrel", "top"])
        let real = SkinLoader.loadDirectory(skins, base: base).map(\.id)
        XCTAssertTrue(real.contains("whimbrel"))
        XCTAssertFalse(real.contains("dup") || real.contains("bad-font"), "skins/invalid/ is not descended into")
    }

    func test_imagery_asset_must_be_a_plain_file_name() throws {
        let assets = ["../x.png", "a/b.png", ".hidden.png", "", "ok.png"]
        let imagery = assets.map { #"{"asset":"\#($0)","placement":"header"}"# }.joined(separator: ",")
        let (skin, repairs) = try XCTUnwrap(parse(#"{"id":"img","name":"Img","palettes":["default"],"decoration":{"imagery":[\#(imagery)]}}"#))
        XCTAssertEqual(skin.decoration.imagery.map(\.asset), ["ok.png"])
        XCTAssertEqual(repairs.count, 4)
    }

    func test_boolean_is_not_a_number() throws {
        let (skin, repairs) = try XCTUnwrap(parse(#"{"id":"b","name":"B","palettes":["default"],"shape":{"radius":true,"borderWidth":false}}"#))
        XCTAssertEqual(skin.shape.radius, base.shape.radius)
        XCTAssertEqual(skin.shape.borderWidth, base.shape.borderWidth)
        XCTAssertEqual(repairs.count, 2)
    }

    func test_bundled_high_contrast_json_loads_without_repairs() throws {
        let (skin, repairs) = try XCTUnwrap(SkinLoader.parse(try load("high-contrast.json"), base: base))
        XCTAssertEqual(repairs, [])
        XCTAssertEqual(skin.id, "high-contrast")
        XCTAssertEqual(skin.paletteIds, ["high-contrast"])
        XCTAssertEqual(skin.archetype, .standard)
    }

    func test_bundled_whimbrel_json_equals_SkinDefaults_whimbrel() throws {
        let (skin, repairs) = try XCTUnwrap(SkinLoader.parse(try load("whimbrel.json"), base: base))
        XCTAssertEqual(repairs, [])
        XCTAssertEqual(skin, SkinDefaults.whimbrel)
        XCTAssertEqual(SkinDefaults.whimbrel.shape.controlRadius, 8)
        for role in SkinIconRole.allCases {
            XCTAssertNotNil(NSImage(systemSymbolName: SkinDefaults.whimbrel.icons[role], accessibilityDescription: nil), "\(role)")
        }
        for key in SkinVoiceKey.allCases {
            XCTAssertTrue(key.fits(SkinDefaults.whimbrel.voice.values[key] ?? ""), "\(key)")
        }
    }
}
