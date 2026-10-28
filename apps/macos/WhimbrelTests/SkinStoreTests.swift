import AppKit
import XCTest
@testable import Whimbrel

final class SkinStoreTests: XCTestCase {
    private let skinKey = "com.mpointer.kestrel.skinId"
    private let paletteKey = "com.mpointer.kestrel.skinPaletteId"

    /// <repo>, found from this file's path (apps/macos/WhimbrelTests/SkinStoreTests.swift).
    private var repo: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
    }

    private func defaults() -> UserDefaults {
        let suite = "skin-store-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        // Dropping the domain leaves an empty plist in ~/Library/Preferences; remove that too.
        addTeardownBlock {
            d.removePersistentDomain(forName: suite)
            let plist = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Preferences/\(suite).plist")
            try? FileManager.default.removeItem(at: plist)
        }
        return d
    }

    /// The four real themes, as the app installs them.
    private func store(_ d: UserDefaults, extraThemes: [Theme] = [], skins: [Skin]? = nil) -> (SkinStore, ThemeStore) {
        let themes = ThemeStore(defaults: d)
        let loaded = ThemeLoader.loadDirectory(repo.appendingPathComponent("themes"))
        themes.install(loaded + extraThemes)
        let store = SkinStore(defaults: d, themes: themes)
        store.install(skins ?? SkinLoader.loadDirectory(repo.appendingPathComponent("skins")))
        return (store, themes)
    }

    // MARK: migration

    func testAStoredOldSkinIdReadsAsWhimbrelAndIsRewritten() {
        let d = defaults()
        d.set("kestrel", forKey: skinKey)
        let (store, _) = store(d)
        XCTAssertEqual(store.selected.id, SkinDefaults.whimbrel.id)
        XCTAssertEqual(d.string(forKey: skinKey), SkinDefaults.whimbrel.id, "rewritten once, under the live id")
    }

    func test_migrates_each_old_theme_id() {
        for (old, skin, palette) in [("default", "whimbrel", "default"), ("slate", "whimbrel", "slate"),
                                     ("sepia", "whimbrel", "sepia"), ("high-contrast", "high-contrast", "high-contrast")] {
            let d = defaults()
            d.set(old, forKey: ThemeStore.selectedKey)
            let (s, themes) = store(d)
            XCTAssertEqual(s.selected.id, skin, old)
            XCTAssertEqual(s.selectedPaletteId, palette, old)
            XCTAssertEqual(themes.selectedId, palette, old)
            XCTAssertEqual(d.string(forKey: skinKey), skin, old)
            XCTAssertEqual(d.string(forKey: paletteKey), palette, old)
            XCTAssertEqual(d.string(forKey: ThemeStore.selectedKey), old, "the old key is left in place")
        }
    }

    func test_unknown_old_theme_id_falls_back() {
        let d = defaults()
        d.set("neon", forKey: ThemeStore.selectedKey)
        let (s, _) = store(d)
        XCTAssertEqual(s.selected.id, "whimbrel")
        XCTAssertEqual(s.selectedPaletteId, "default")
        XCTAssertEqual(d.string(forKey: skinKey), "whimbrel")
        XCTAssertEqual(d.string(forKey: paletteKey), "default")
    }

    func test_migration_runs_once() {
        let d = defaults()
        d.set("slate", forKey: ThemeStore.selectedKey)
        _ = store(d)
        d.set("sepia", forKey: ThemeStore.selectedKey)
        let (s, _) = store(d)
        XCTAssertEqual(s.selectedPaletteId, "slate", "skinId is set, so the old key is no longer read")
    }

    // MARK: fallbacks

    func test_missing_skin_file_falls_back_but_keeps_stored_id() {
        let d = defaults()
        d.set("gone", forKey: skinKey)
        d.set("slate", forKey: paletteKey)
        let (s, _) = store(d)
        XCTAssertEqual(s.selected.id, "whimbrel")
        XCTAssertEqual(s.selectedPaletteId, "default")
        XCTAssertEqual(d.string(forKey: skinKey), "gone")
    }

    func test_skin_with_no_valid_palettes_uses_default_palette() {
        var broken = SkinDefaults.whimbrel
        broken.id = "broken"
        broken.name = "Broken"
        broken.paletteIds = ["nope"]
        let d = defaults()
        d.set("broken", forKey: skinKey)
        let (s, _) = store(d, skins: [SkinDefaults.whimbrel, broken])
        XCTAssertEqual(s.selected.id, "broken")
        XCTAssertEqual(s.palettes(for: broken).map(\.id), ["default"])
        XCTAssertEqual(s.selectedPaletteId, "default")
    }

    // MARK: select

    func test_select_writes_both_keys_and_theme_id() {
        let d = defaults()
        let (s, themes) = store(d)
        let before = s.revision
        s.select(skinId: "high-contrast", paletteId: "high-contrast")
        XCTAssertEqual(d.string(forKey: skinKey), "high-contrast")
        XCTAssertEqual(d.string(forKey: paletteKey), "high-contrast")
        XCTAssertEqual(d.string(forKey: ThemeStore.selectedKey), "high-contrast")
        XCTAssertEqual(themes.selectedId, "high-contrast")
        XCTAssertGreaterThan(s.revision, before)
        s.select(skinId: "whimbrel", paletteId: nil)
        XCTAssertEqual(s.selectedPaletteId, "default", "nil picks the skin's first palette")
        s.select(skinId: "nope", paletteId: nil)
        XCTAssertEqual(s.selected.id, "whimbrel", "an unknown skin is ignored")
    }

    // MARK: palettes

    func test_dark_only_palette_serves_light_appearance() throws {
        let dark = ThemeDefaults.dark.merging([.backgroundPage: 0x000011]) { _, new in new }
        let json = ##"{"id":"night","name":"Night","dark":{"backgroundPage":"#000011"}}"##
        let night = try XCTUnwrap(ThemeLoader.parse(Data(json.utf8)))
        XCTAssertEqual(night.dark, dark)
        XCTAssertEqual(night.light, night.dark)
        var skin = SkinDefaults.whimbrel
        skin.id = "nightly"
        skin.paletteIds = ["night"]
        let d = defaults()
        d.set("nightly", forKey: skinKey)
        let (s, themes) = store(d, extraThemes: [night], skins: [SkinDefaults.whimbrel, skin])
        XCTAssertEqual(s.selectedPaletteId, "night")
        XCTAssertEqual(themes.hex(.backgroundPage, isDark: false), 0x000011)
    }

    func test_palette_ids_resolve_from_both_directories() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("palettes-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        try Data(##"{"id":"extra","name":"Extra","dark":{"backgroundPage":"#000011"}}"##.utf8)
            .write(to: dir.appendingPathComponent("extra.json"))
        let all = ThemeLoader.loadDirectory(repo.appendingPathComponent("themes")) + ThemeLoader.loadDirectory(dir)
        let themes = ThemeStore(defaults: defaults())
        themes.install(all)
        var skin = SkinDefaults.whimbrel
        skin.paletteIds = ["slate", "extra", "missing"]
        let s = SkinStore(defaults: defaults(), themes: themes)
        XCTAssertEqual(s.palettes(for: skin).map(\.id), ["slate", "extra"])
        // A missing or empty directory is fine.
        XCTAssertEqual(ThemeLoader.loadDirectory(dir.appendingPathComponent("absent")).map(\.id), ["default"])
        XCTAssertTrue(FileManager.default.fileExists(atPath: repo.appendingPathComponent("skins/palettes").path))
    }

    // MARK: scoped override

    func test_with_override_is_scoped_and_restores_on_throw() {
        let d = defaults()
        let (s, themes) = store(d)
        let hc = s.skins.first { $0.id == "high-contrast" }!
        s.withOverride(skin: hc, paletteId: "high-contrast") {
            XCTAssertEqual(s.selected.id, "high-contrast")
            XCTAssertEqual(themes.selectedId, "high-contrast")
        }
        struct Boom: Error {}
        XCTAssertThrowsError(try s.withOverride(skin: hc, paletteId: "slate") { throw Boom() })
        XCTAssertEqual(s.selected.id, "whimbrel")
        XCTAssertEqual(themes.selectedId, "default")
        XCTAssertNotEqual(d.string(forKey: skinKey), "high-contrast", "an override never persists")
    }

    // MARK: appearance

    func test_palette_variants_come_from_the_file() throws {
        func parse(_ json: String) throws -> Theme { try XCTUnwrap(ThemeLoader.parse(Data(json.utf8))) }
        XCTAssertEqual(try parse(##"{"id":"n","name":"N","dark":{"backgroundPage":"#000011"}}"##).variants, .darkOnly)
        XCTAssertEqual(try parse(##"{"id":"n","name":"N","light":{"backgroundPage":"#ffffee"}}"##).variants, .lightOnly)
        XCTAssertEqual(try parse(##"{"id":"n","name":"N","light":{},"dark":{}}"##).variants, .both)
        let palettes = ThemeLoader.loadDirectory(repo.appendingPathComponent("skins/palettes"))
        for id in ["phosphor", "amber", "violet-night", "ice"] {
            XCTAssertEqual(palettes.first { $0.id == id }?.variants, .darkOnly, id)
        }
        XCTAssertEqual(palettes.first { $0.id == "paper" }?.variants, .both)
    }

    /// A one-variant palette forces its appearance, so system-drawn text (labels, placeholders,
    /// pickers, alerts) matches the palette; the user's choice applies to two-variant palettes.
    func test_single_variant_palettes_force_their_appearance() {
        let table: [(ThemeVariants, AppearanceChoice, NSAppearance.Name?)] = [
            (.both, .system, nil), (.both, .light, .aqua), (.both, .dark, .darkAqua),
            (.darkOnly, .system, .darkAqua), (.darkOnly, .light, .darkAqua), (.darkOnly, .dark, .darkAqua),
            (.lightOnly, .system, .aqua), (.lightOnly, .light, .aqua), (.lightOnly, .dark, .aqua),
        ]
        for (variants, choice, expected) in table {
            XCTAssertEqual(AppearanceSettings.appearanceName(choice: choice, variants: variants), expected, "\(variants) \(choice)")
        }
    }

    /// The picker does nothing under a one-variant palette, so Settings says why instead of looking broken.
    func test_locked_caption_shows_only_for_single_variant_palettes() {
        XCTAssertEqual(AppearanceSettings.lockedCaption(variants: .darkOnly), "This palette is dark only, so it always uses Dark.")
        XCTAssertNotNil(AppearanceSettings.lockedCaption(variants: .lightOnly))
        XCTAssertNil(AppearanceSettings.lockedCaption(variants: .both))
    }

    // MARK: icons

    /// Views that used hard-coded symbols now ask the skin; Whimbrel's must be the symbols they used.
    func test_whimbrel_role_symbols_match_the_symbols_they_replace() {
        let icons = SkinDefaults.whimbrel.icons
        XCTAssertEqual(icons[.ask], "arrow.up.circle.fill", "MeetingAskBar send")
        XCTAssertEqual(icons[.meetings], "person.2", "Meetings empty state")
        XCTAssertEqual(icons[.routines], "bolt", "Timeline routine-run chip")
    }
}
