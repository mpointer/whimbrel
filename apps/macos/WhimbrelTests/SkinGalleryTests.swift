import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

@MainActor
final class SkinGalleryTests: XCTestCase {
    /// <repo>, found from this file's path (apps/macos/WhimbrelTests/SkinGalleryTests.swift).
    private let repo = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()

    private struct Boom: Error {}

    private func defaults() -> UserDefaults {
        let suite = "skin-gallery-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        addTeardownBlock {
            d.removePersistentDomain(forName: suite)
            let plist = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Preferences/\(suite).plist")
            try? FileManager.default.removeItem(at: plist)
        }
        return d
    }

    private var themes: [Theme] {
        ThemeLoader.loadDirectory(repo.appendingPathComponent("themes")) + ThemeLoader.loadDirectory(repo.appendingPathComponent("skins/palettes"))
    }

    /// Stores of their own (nothing persisted to the app's defaults) holding the real skins and palettes. The shared theme
    /// store gets the palettes for the duration, since the render reads colors through it and the test host loads none.
    private func stores() -> (SkinStore, ThemeStore) {
        let themeStore = ThemeStore(defaults: defaults())
        themeStore.install(themes)
        let skinStore = SkinStore(defaults: defaults(), themes: themeStore)
        skinStore.install(SkinLoader.loadDirectory(repo.appendingPathComponent("skins")))
        let shared = ThemeStore.shared
        let before = shared.themes
        shared.install(before + themes)
        addTeardownBlock { shared.install(before) }
        return (skinStore, themeStore)
    }

    func test_override_is_scoped() throws {
        let (store, _) = stores()
        let other = try XCTUnwrap(store.skins.first { $0.id != store.selected.id })
        let originalSkin = store.selected.id
        let originalPalette = store.selectedPaletteId
        let palette = try XCTUnwrap(store.palettes(for: other).first).id
        store.withOverride(skin: other, paletteId: palette) {
            XCTAssertEqual(store.selected.id, other.id)
            XCTAssertEqual(store.selectedPaletteId, palette)
        }
        XCTAssertEqual(store.selected.id, originalSkin)
        XCTAssertEqual(store.selectedPaletteId, originalPalette)
        XCTAssertThrowsError(try store.withOverride(skin: other, paletteId: palette) { throw Boom() })
        XCTAssertEqual(store.selected.id, originalSkin)
        XCTAssertEqual(store.selectedPaletteId, originalPalette)
    }

    func test_preview_does_not_change_live_selection() {
        let (skins, themeStore) = stores()
        let cache = SkinPreviewCache(skins: skins, themes: themeStore)
        let shared = (SkinStore.shared.selected.id, SkinStore.shared.selectedPaletteId, SkinStore.shared.revision, ThemeStore.shared.revision)
        let mine = (skins.selected.id, skins.selectedPaletteId, skins.revision)
        for skin in skins.skins {
            _ = cache.image(skinId: skin.id, paletteId: skins.palettes(for: skin)[0].id, colorScheme: .dark)
        }
        XCTAssertEqual(SkinStore.shared.selected.id, shared.0)
        XCTAssertEqual(SkinStore.shared.selectedPaletteId, shared.1)
        XCTAssertEqual(SkinStore.shared.revision, shared.2)
        XCTAssertEqual(ThemeStore.shared.revision, shared.3)
        XCTAssertEqual(skins.selected.id, mine.0)
        XCTAssertEqual(skins.selectedPaletteId, mine.1)
        XCTAssertEqual(skins.revision, mine.2)
    }

    func test_cache_invalidates_on_install() throws {
        let (skins, themeStore) = stores()
        let cache = SkinPreviewCache(skins: skins, themes: themeStore)
        let whimbrel = SkinDefaults.whimbrel
        _ = cache.image(skinId: whimbrel.id, paletteId: "default", colorScheme: .light)
        _ = cache.image(skinId: whimbrel.id, paletteId: "default", colorScheme: .light)
        XCTAssertEqual(cache.renderCount, 1, "second request is a cache hit")
        _ = cache.image(skinId: whimbrel.id, paletteId: "default", colorScheme: .dark)
        XCTAssertEqual(cache.renderCount, 2, "appearance is part of the key")
        skins.select(skinId: "whimbrel", paletteId: "slate")
        _ = cache.image(skinId: whimbrel.id, paletteId: "default", colorScheme: .light)
        XCTAssertEqual(cache.renderCount, 2, "selecting does not invalidate")

        // Whimbrel is always installed as the built-in, so change another skin instead.
        let other = try XCTUnwrap(skins.skins.first { $0.id != whimbrel.id })
        let palette = skins.palettes(for: other)[0].id
        _ = cache.image(skinId: other.id, paletteId: palette, colorScheme: .light)
        let before = cache.renderCount
        var edited = other
        edited.shape.radius += 6
        skins.install(skins.skins.filter { $0.id != other.id } + [edited])
        _ = cache.image(skinId: other.id, paletteId: palette, colorScheme: .light)
        XCTAssertEqual(cache.renderCount, before + 1, "an installed skin that changed is drawn again")
    }

    func test_every_installed_skin_renders_a_preview() throws {
        let (skins, themeStore) = stores()
        XCTAssertGreaterThan(skins.skins.count, 1)
        let cache = SkinPreviewCache(skins: skins, themes: themeStore)
        var seen: [String] = []
        for skin in skins.skins {
            let palette = skins.palettes(for: skin)[0].id
            for scheme in [ColorScheme.light, .dark] {
                let image = cache.image(skinId: skin.id, paletteId: palette, colorScheme: scheme)
                XCTAssertEqual(image.size, CGSize(width: 225, height: 300), "\(skin.id)")
                let rep = try XCTUnwrap(NSBitmapImageRep(data: try XCTUnwrap(image.tiffRepresentation)), skin.id)
                var colors = Set<UInt32>()
                for y in stride(from: 0, to: rep.pixelsHigh, by: 3) {
                    for x in stride(from: 0, to: rep.pixelsWide, by: 3) {
                        let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB)
                        colors.insert(UInt32((c?.redComponent ?? 0) * 255) << 16 | UInt32((c?.greenComponent ?? 0) * 255) << 8 | UInt32((c?.blueComponent ?? 0) * 255))
                    }
                }
                XCTAssertGreaterThan(colors.count, 8, "\(skin.id) \(scheme) preview looks blank")
            }
            seen.append(skin.id)
        }
        XCTAssertEqual(seen, skins.skins.map(\.id))
    }

    func test_render_does_not_yield_to_the_run_loop() {
        let (skins, themeStore) = stores()
        let cache = SkinPreviewCache(skins: skins, themes: themeStore)
        var ran = false
        DispatchQueue.main.async { ran = true }
        _ = cache.image(skinId: "whimbrel", paletteId: "default", colorScheme: .light)
        XCTAssertFalse(ran, "the main run loop must not turn while the override is active")
    }

    func test_render_never_starts_inside_another_render() throws {
        let (skins, themeStore) = stores()
        let cache = SkinPreviewCache(skins: skins, themes: themeStore)
        let other = try XCTUnwrap(skins.skins.first { $0.id != "whimbrel" })
        var insideRendering = false
        var countInside = -1
        var probed = false
        cache.onRender = {
            guard !probed else { return }
            probed = true
            insideRendering = cache.isRendering
            _ = cache.image(skinId: other.id, paletteId: skins.palettes(for: other)[0].id, colorScheme: .light)
            countInside = cache.renderCount
        }
        _ = cache.image(skinId: "whimbrel", paletteId: "default", colorScheme: .light)
        XCTAssertTrue(insideRendering)
        XCTAssertEqual(countInside, 0, "the nested request did not render")
        XCTAssertEqual(cache.renderCount, 1)
        XCTAssertFalse(cache.isRendering)
        XCTAssertNil(cache.cachedImage(skinId: other.id, paletteId: skins.palettes(for: other)[0].id, colorScheme: .light), "a placeholder is never cached")
        cache.onRender = nil
        _ = cache.image(skinId: other.id, paletteId: skins.palettes(for: other)[0].id, colorScheme: .light)
        XCTAssertEqual(cache.renderCount, 2, "it renders once the first is done")
    }

    func test_selection_outside_override_is_unchanged_after_render() {
        let (skins, themeStore) = stores()
        let cache = SkinPreviewCache(skins: skins, themes: themeStore)
        let before = (SkinStore.shared.selected.id, SkinStore.shared.selectedPaletteId, ThemeStore.shared.selectedId, SkinStore.shared.revision, ThemeStore.shared.revision)
        for skin in skins.skins { _ = cache.image(skinId: skin.id, paletteId: skins.palettes(for: skin)[0].id, colorScheme: .dark) }
        let after = (SkinStore.shared.selected.id, SkinStore.shared.selectedPaletteId, ThemeStore.shared.selectedId, SkinStore.shared.revision, ThemeStore.shared.revision)
        XCTAssertEqual(before.0, after.0)
        XCTAssertEqual(before.1, after.1)
        XCTAssertEqual(before.2, after.2)
        XCTAssertEqual(before.3, after.3)
        XCTAssertEqual(before.4, after.4)
    }

    func test_reinstalling_a_changed_palette_invalidates_the_cache() throws {
        let (skins, themeStore) = stores()
        let cache = SkinPreviewCache(skins: skins, themes: themeStore)
        _ = cache.image(skinId: "whimbrel", paletteId: "slate", colorScheme: .light)
        XCTAssertEqual(cache.renderCount, 1)
        let slate = try XCTUnwrap(themeStore.themes.first { $0.id == "slate" })
        var light = slate.light
        light[.accentPrimary] = 0x123456
        let changed = Theme(id: slate.id, name: slate.name, author: slate.author, light: light, dark: slate.dark)
        themeStore.install(themeStore.themes.filter { $0.id != "slate" } + [changed])
        _ = cache.image(skinId: "whimbrel", paletteId: "slate", colorScheme: .light)
        XCTAssertEqual(cache.renderCount, 2)
    }

    func test_clicking_the_selected_card_keeps_its_palette() {
        XCTAssertEqual(SkinGallery.paletteToSelect(isSelected: true, current: "slate", first: "default"), "slate")
        XCTAssertEqual(SkinGallery.paletteToSelect(isSelected: false, current: "slate", first: "default"), "default")
    }

    /// A rebuilt card starts from the cached preview, so it never flashes a placeholder.
    func test_card_preview_seeds_from_the_cache() throws {
        let (skins, themeStore) = stores()
        let cache = SkinPreviewCache(skins: skins, themes: themeStore)
        let other = try XCTUnwrap(skins.skins.first { $0.id != skins.selected.id })
        XCTAssertNil(SkinGallery.cachedPreview(for: other, skins: skins, colorScheme: .light, cache: cache), "nothing drawn yet")
        let drawn = cache.image(skinId: other.id, paletteId: skins.palettes(for: other)[0].id, colorScheme: .light)
        let count = cache.renderCount
        XCTAssertTrue(SkinGallery.cachedPreview(for: other, skins: skins, colorScheme: .light, cache: cache) === drawn)
        XCTAssertNil(SkinGallery.cachedPreview(for: other, skins: skins, colorScheme: .dark, cache: cache), "appearance is part of the key")
        XCTAssertEqual(cache.renderCount, count, "seeding never draws")
        // The selected skin previews the palette in force.
        skins.select(skinId: "whimbrel", paletteId: "slate")
        let slate = cache.image(skinId: "whimbrel", paletteId: "slate", colorScheme: .dark)
        XCTAssertTrue(SkinGallery.cachedPreview(for: SkinDefaults.whimbrel, skins: skins, colorScheme: .dark, cache: cache) === slate)
    }

    /// Only a gallery click makes the rebuilt Settings scroll back to the card, and only once.
    func test_gallery_scroll_target_is_taken_once() {
        let (skins, _) = stores()
        skins.select(skinId: "reference", paletteId: nil)
        XCTAssertNil(skins.takeGalleryScrollTarget(), "an ordinary selection or Settings open does not scroll")
        skins.selectFromGallery(skinId: "retro-terminal", paletteId: nil)
        XCTAssertEqual(skins.selected.id, "retro-terminal")
        XCTAssertEqual(skins.takeGalleryScrollTarget(), "skin-card-retro-terminal")
        XCTAssertNil(skins.takeGalleryScrollTarget())
    }
}
