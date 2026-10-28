import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

/// Skins draw their approved images: header and badge in every Today archetype (background too, except
/// `standard`), the header on top-level screens, nothing for a missing file; and `SkinImageResolver`.
@MainActor
final class SkinImageryPlacementTests: XCTestCase {
    private let repo = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()

    /// One shipped skin per archetype (Whimbrel for `standard`).
    private func skinPerArchetype() -> [SkinArchetype: Skin] {
        var found: [SkinArchetype: Skin] = [.standard: SkinDefaults.whimbrel]
        for skin in SkinLoader.loadDirectory(repo.appendingPathComponent("skins")) where found[skin.archetype] == nil {
            found[skin.archetype] = skin
        }
        return found
    }

    // MARK: placements

    func testEveryArchetypeDrawsHeaderAndBadge() throws {
        let fixture = try SkinImageryFixture(self)
        let skins = skinPerArchetype()
        XCTAssertEqual(Set(skins.keys), Set(SkinArchetype.allCases), "a shipped skin for every archetype")
        for (archetype, base) in skins {
            let skin = try fixture.skin(from: base)
            for dark in [false, true] {
                let expected: Set<SkinImagePlacement> = archetype == .standard ? [.header, .badge] : [.header, .background, .badge]
                let drawn = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
                    SkinImageryFixture.drawnPlacements(SkinImageryFixture.chrome(), expecting: expected, dark: dark)
                }
                XCTAssertEqual(drawn, expected, "\(archetype) \(dark ? "dark" : "light")")
            }
        }
    }

    /// A text-free background sits under Today's content, so it is not drawn there.
    func testTextFreeBackgroundIsNotDrawnUnderToday() throws {
        let fixture = try SkinImageryFixture(self)
        for (archetype, base) in skinPerArchetype() where archetype != .standard {
            let skin = try fixture.skin(from: base, textFreeBackground: true)
            let drawn = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
                SkinImageryFixture.drawnPlacements(SkinImageryFixture.chrome(), expecting: [.header, .badge])
            }
            XCTAssertEqual(drawn, [.header, .badge], "\(archetype)")
        }
    }

    func testMissingImageDrawsNothing() throws {
        let fixture = try SkinImageryFixture(self)
        for (archetype, base) in skinPerArchetype() {
            var skin = try fixture.skin(from: base)
            skin.decoration.imagery = skin.decoration.imagery.map { SkinImage(asset: "gone-\($0.asset)", placement: $0.placement, textFree: false) }
            let drawn = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
                SkinImageryFixture.drawnPlacements(SkinImageryFixture.chrome(), expecting: [])
            }
            XCTAssertEqual(drawn, [], "\(archetype)")
        }
    }

    /// Top-level screens get the header image above the skin's band.
    func testTopLevelScreenDrawsHeaderOnly() throws {
        let fixture = try SkinImageryFixture(self)
        let skin = try fixture.skin(from: try XCTUnwrap(skinPerArchetype()[.glass]))
        let drawn = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
            SkinImageryFixture.drawnPlacements(SkinHeaderArea { Text("screen") }, expecting: [.header])
        }
        XCTAssertEqual(drawn, [.header])
    }

    /// Header and badge sit beside the text (above Today, above a screen's band, beside the greeting), never under
    /// it, so they draw the image as it is. The background is under Today's text and keeps its scrim.
    func testHeaderAndBadgeDrawWithoutScrimBackgroundKeepsIt() throws {
        let fixture = try SkinImageryFixture(self)
        let skin = try fixture.skin(from: try XCTUnwrap(skinPerArchetype()[.glass]))
        let size = CGSize(width: 400, height: 120)
        func brightness(_ placement: SkinImagePlacement, dark: Bool) throws -> CGFloat {
            // A point in the image's white half (the 48 pt badge is centred: x 176-224); the dark page scrim would pull it far down.
            let x = placement == .badge ? 185 : 50
            let color = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
                SkinImageryFixture.pixel(SkinImagerySlot(placement: placement).frame(width: 400, height: 120), size: size, dark: dark, x: x, y: 60)
            }
            return try XCTUnwrap(color).brightnessComponent
        }
        for dark in [false, true] {
            XCTAssertGreaterThan(try brightness(.header, dark: dark), 0.98, "header unscrimmed \(dark)")
            XCTAssertGreaterThan(try brightness(.badge, dark: dark), 0.98, "badge unscrimmed \(dark)")
        }
        XCTAssertLessThan(try brightness(.background, dark: true), 0.5, "the background keeps the dark page scrim")
        XCTAssertFalse(SkinImageryView.hasScrim(.header))
        XCTAssertFalse(SkinImageryView.hasScrim(.badge))
        XCTAssertTrue(SkinImageryView.hasScrim(.background))
    }

    /// Whimbrel with no imagery renders exactly as before: `TodayChrome` adds nothing around the standard layout,
    /// and it still matches the standard Today baselines.
    func testWhimbrelSkinWithoutImageryUnchanged() throws {
        let size = CGSize(width: 900, height: 1400)
        try SkinStore.shared.withOverride(skin: SkinDefaults.whimbrel, paletteId: "default") {
            XCTAssertTrue(SkinDefaults.whimbrel.decoration.imagery.isEmpty)
            // Built inside the override: the composer stand-in reads the skin's fonts when it is made.
            let standard = StandardTodayLayout(
                content: .sample, actions: .noop,
                slots: TodaySlots(live: nil, decisionCount: TodayContent.sample.pendingApprovalCount, actions: .noop, composer: TodayFixtures.composer)
            )
            for scheme in [ColorScheme.light, .dark] {
                let chrome = try XCTUnwrap(OffscreenRender.bitmap(of: SkinImageryFixture.chrome(), size: size, colorScheme: scheme, scale: 1))
                let plain = try XCTUnwrap(OffscreenRender.bitmap(of: standard, size: size, colorScheme: scheme, scale: 1))
                XCTAssertEqual(chrome.tiffRepresentation, plain.tiffRepresentation, "\(scheme)")
            }
            try assertSnapshot(SkinImageryFixture.chrome(), named: "standard-sample-light", size: size, colorScheme: .light, tolerance: 0.0001)
            try assertSnapshot(SkinImageryFixture.chrome(), named: "standard-sample-dark", size: size, colorScheme: .dark, tolerance: 0.0001)
        }
    }

    // MARK: resolver

    func testResolverReadsTheSkinsImagesFolder() throws {
        let fixture = try SkinImageryFixture(self)
        try fixture.write(asset: "a.png", skinId: "user-r")
        let skin = Skin(id: "user-r", base: SkinDefaults.whimbrel)
        let image = try XCTUnwrap(SkinImageResolver.image(for: SkinImage(asset: "a.png", placement: .header, textFree: false), skin: skin))
        XCTAssertEqual(image.representations.first?.pixelsWide, 8)
    }

    func testResolverRejectsTraversalNonImageAndMissing() throws {
        let fixture = try SkinImageryFixture(self)
        // A real image one level up and in a sibling skin: reachable only by escaping `images/`.
        try FileManager.default.createDirectory(at: fixture.root.appendingPathComponent("user-r"), withIntermediateDirectories: true)
        try SkinImageryFixture.png.write(to: fixture.root.appendingPathComponent("user-r/up.png"))
        try fixture.write(asset: "other.png", skinId: "user-other")
        try fixture.write(Data("not an image".utf8), asset: "text.png", skinId: "user-r")
        let skin = Skin(id: "user-r", base: SkinDefaults.whimbrel)
        for asset in ["../up.png", "..", ".", "", "../../user-other/images/other.png", "sub/a.png", "/etc/hosts", "missing.png", "text.png"] {
            XCTAssertNil(SkinImageResolver.image(for: SkinImage(asset: asset, placement: .header, textFree: false), skin: skin), asset)
        }
    }

    /// A symlink (the file, or the `images/` folder) can point anywhere, so neither is followed.
    func testResolverDoesNotFollowSymlinks() throws {
        let fixture = try SkinImageryFixture(self)
        let fm = FileManager.default
        let outside = fixture.root.appendingPathComponent("outside")
        try fm.createDirectory(at: outside, withIntermediateDirectories: true)
        try SkinImageryFixture.png.write(to: outside.appendingPathComponent("secret.png"))
        // File symlink inside a real images/ folder.
        try fixture.write(asset: "real.png", skinId: "user-r")
        let images = fixture.store.imageURL(asset: "x", skinId: "user-r").deletingLastPathComponent()
        try fm.createSymbolicLink(at: images.appendingPathComponent("link.png"), withDestinationURL: outside.appendingPathComponent("secret.png"))
        // images/ itself a symlink.
        try fm.createDirectory(at: fixture.root.appendingPathComponent("user-s"), withIntermediateDirectories: true)
        try fm.createSymbolicLink(at: fixture.root.appendingPathComponent("user-s/images"), withDestinationURL: outside)
        func resolve(_ asset: String, _ skinId: String) -> NSImage? {
            SkinImageResolver.image(for: SkinImage(asset: asset, placement: .header, textFree: false), skin: Skin(id: skinId, base: SkinDefaults.whimbrel))
        }
        XCTAssertNotNil(resolve("real.png", "user-r"), "a plain file still resolves")
        XCTAssertNil(resolve("link.png", "user-r"))
        XCTAssertNil(resolve("secret.png", "user-s"))
    }

    /// The scrim is worked out once per image and colours, not on every draw.
    func testScrimIsComputedOncePerImage() {
        let image = NSImage(size: NSSize(width: 4, height: 4))
        image.lockFocus(); NSColor.white.setFill(); NSRect(x: 0, y: 0, width: 4, height: 4).fill(); image.unlockFocus()
        let (text, page): (UInt32, UInt32) = (0xF2F2F2, 0x101010)
        let first = SkinImageResolver.scrimOpacity(for: image, text: [text], scrim: page)
        XCTAssertGreaterThan(first, 0, "light text needs a scrim over white")
        // Repainting the same image black would need none; the cached value stands.
        image.lockFocus(); NSColor.black.setFill(); NSRect(x: 0, y: 0, width: 4, height: 4).fill(); image.unlockFocus()
        XCTAssertEqual(SkinImageResolver.scrimOpacity(for: image, text: [text], scrim: page), first)
        XCTAssertEqual(SkinImageResolver.scrimOpacity(for: NSImage(data: SkinImageryFixture.png)!, text: [text], scrim: page), first, "a new image is measured afresh (white half)")
        SkinImageResolver.invalidate()
        XCTAssertEqual(SkinImageResolver.scrimOpacity(for: image, text: [text], scrim: page), 0, "dropped with the image cache")
    }

    func testResolverCachesUntilTheSkinsReload() throws {
        let fixture = try SkinImageryFixture(self)
        try fixture.write(asset: "a.png", skinId: "user-r")
        let skin = Skin(id: "user-r", base: SkinDefaults.whimbrel)
        let entry = SkinImage(asset: "a.png", placement: .header, textFree: false)
        let first = try XCTUnwrap(SkinImageResolver.image(for: entry, skin: skin))
        XCTAssertTrue(first === SkinImageResolver.image(for: entry, skin: skin), "decoded once")
        try fixture.write(Data(), asset: "a.png", skinId: "user-r")
        XCTAssertTrue(first === SkinImageResolver.image(for: entry, skin: skin), "still the cached image")
        // Installing skins (every reload goes through it) drops the cache.
        let suite = "skin-imagery-tests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        addTeardownBlock {
            defaults.removePersistentDomain(forName: suite)
            try? FileManager.default.removeItem(at: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Preferences/\(suite).plist"))
        }
        SkinStore(defaults: defaults, themes: ThemeStore(defaults: defaults)).install([])
        XCTAssertNil(SkinImageResolver.image(for: entry, skin: skin), "re-read after the reload: the file is now empty")
    }
}

private extension Skin {
    /// `base` under another id.
    init(id: String, base: Skin) {
        self = base
        self.id = id
    }
}
