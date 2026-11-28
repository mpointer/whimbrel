import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

/// Skins draw their approved images: header and badge on Home for every skin (the background placement is dropped),
/// the header on top-level screens, nothing for a missing file or a bad name; and `SkinImageResolver`.
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

    func testHomeDrawsHeaderAndBadgeForEveryArchetypesSkin() throws {
        let fixture = try SkinImageryFixture(self)
        let skins = skinPerArchetype()
        XCTAssertEqual(Set(skins.keys), Set(SkinArchetype.allCases), "a shipped skin for every archetype")
        for (archetype, base) in skins {
            let skin = try fixture.skin(from: base)
            for dark in [false, true] {
                let expected: Set<SkinImagePlacement> = [.header, .badge]
                let drawn = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
                    SkinImageryFixture.drawnPlacements(SkinHomeImagesView(), expecting: expected, dark: dark)
                }
                XCTAssertEqual(drawn, expected, "\(archetype) \(dark ? "dark" : "light")")
            }
        }
    }

    /// The background placement is dropped: a skin that has one (text-free or not) draws nothing for it.
    func testBackgroundIsNeverDrawn() throws {
        let fixture = try SkinImageryFixture(self)
        for textFree in [false, true] {
            for (archetype, base) in skinPerArchetype() {
                let skin = try fixture.skin(from: base, textFreeBackground: textFree)
                let drawn = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
                    SkinImageryFixture.drawnPlacements(SkinHomeImagesView(), expecting: [.header, .badge])
                }
                XCTAssertEqual(drawn, [.header, .badge], "\(archetype) textFree \(textFree)")
            }
        }
    }

    /// Thread's Home draws the header image and the badge, and a skin with neither adds nothing.
    func testHomeDrawsHeaderAndBadge() throws {
        let fixture = try SkinImageryFixture(self)
        let skin = try fixture.skin(from: try XCTUnwrap(skinPerArchetype()[.glass]))
        let drawn = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
            SkinImageryFixture.drawnPlacements(SkinHomeImagesView(), expecting: [.header, .badge])
        }
        XCTAssertEqual(drawn, [.header, .badge])
        let none = SkinStore.shared.withOverride(skin: SkinDefaults.whimbrel, paletteId: "default") {
            SkinImageryFixture.drawnPlacements(SkinHomeImagesView(), expecting: [])
        }
        XCTAssertEqual(none, [])
    }

    func testMissingImageDrawsNothing() throws {
        let fixture = try SkinImageryFixture(self)
        for (archetype, base) in skinPerArchetype() {
            var skin = try fixture.skin(from: base)
            skin.decoration.imagery = skin.decoration.imagery.map { SkinImage(asset: "gone-\($0.asset)", placement: $0.placement, textFree: false) }
            let drawn = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
                SkinImageryFixture.drawnPlacements(SkinHomeImagesView(), expecting: [])
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

    /// A header image has the page-colour scrim over it (so the title row against it reads at 4.5:1); a text-free one and the
    /// badge draw the image as it is.
    func testHeaderHasTheScrimUnlessTextFreeAndBadgeNever() throws {
        let fixture = try SkinImageryFixture(self)
        let base = try XCTUnwrap(skinPerArchetype()[.glass])
        let size = CGSize(width: 400, height: 120)
        func brightness(_ placement: SkinImagePlacement, textFree: Bool, dark: Bool) throws -> CGFloat {
            var skin = try fixture.skin(from: base)
            skin.decoration.imagery = skin.decoration.imagery.map { SkinImage(asset: $0.asset, placement: $0.placement, textFree: $0.placement == .header ? textFree : $0.textFree) }
            // A point in the image's white half (the 40 pt badge is centred: x 180-220); the dark page scrim would pull it far down.
            let x = placement == .badge ? 185 : 50
            let color = SkinStore.shared.withOverride(skin: skin, paletteId: "default") {
                SkinImageryFixture.pixel(SkinImagerySlot(placement: placement).frame(width: 400, height: 120), size: size, dark: dark, x: x, y: 60)
            }
            return try XCTUnwrap(color).brightnessComponent
        }
        XCTAssertLessThan(try brightness(.header, textFree: false, dark: true), 0.5, "the header keeps the dark page scrim")
        for dark in [false, true] {
            XCTAssertGreaterThan(try brightness(.header, textFree: true, dark: dark), 0.98, "a text-free header is unscrimmed \(dark)")
            XCTAssertGreaterThan(try brightness(.badge, textFree: false, dark: dark), 0.98, "badge unscrimmed \(dark)")
        }
    }

    /// Every image is decorative: the slot is hidden from VoiceOver, so it has no label and nothing to read.
    func testImagesAreHiddenFromAccessibility() throws {
        let src = try String(contentsOf: repo.appendingPathComponent("apps/macos/Whimbrel/UI/SkinDecorationViews.swift"), encoding: .utf8)
        let slot = try XCTUnwrap(src.range(of: "struct SkinImagerySlot"))
        let slotBody = src[slot.lowerBound...].prefix(2000)
        XCTAssertTrue(slotBody.contains(".accessibilityHidden(true)"))
        XCTAssertFalse(slotBody.contains(".accessibilityLabel"))
        XCTAssertTrue(src.contains("struct SkinHomeImagesView"))
        let home = try XCTUnwrap(src.range(of: "struct SkinHomeImagesView"))
        XCTAssertTrue(src[home.lowerBound...].prefix(900).contains(".accessibilityHidden(true)"))
    }

    // MARK: Home header band

    private func homeBandSkin(_ band: SkinHeaderBand) -> Skin {
        var skin = SkinDefaults.whimbrel
        skin.decoration.headerBand = band
        return skin
    }

    /// Home as the shell draws it: the sample day under the skin in force, with the band area around it when `banded`.
    @ViewBuilder
    private func homeChrome(banded: Bool) -> some View {
        if banded {
            TodayHomeBandArea { SkinPreviewHome() }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        } else {
            SkinPreviewHome().frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
    }

    /// With no band Home is pixel-identical whether or not it asks for one, so `none` cannot move anything.
    func testHomeBandNoneLeavesHomeUnchanged() throws {
        let size = CGSize(width: 900, height: 1400)
        try SkinStore.shared.withOverride(skin: homeBandSkin(.none), paletteId: "default") {
            for scheme in [ColorScheme.light, .dark] {
                let plain = try XCTUnwrap(OffscreenRender.bitmap(of: homeChrome(banded: false), size: size, colorScheme: scheme, scale: 1))
                let banded = try XCTUnwrap(OffscreenRender.bitmap(of: homeChrome(banded: true), size: size, colorScheme: scheme, scale: 1))
                XCTAssertEqual(plain.tiffRepresentation, banded.tiffRepresentation, "\(scheme)")
            }
        }
    }

    /// The band is the first thing on Home, at the height the other tabs use (2, 12 and 8 points), and Home sits
    /// under it exactly as it was.
    func testHomeBandSitsAtTheTopOfHome() throws {
        let size = CGSize(width: 900, height: 600)
        for (band, height) in [(SkinHeaderBand.rule, 2), (.block, 12), (.stripesDiagonal, 8)] {
            try SkinStore.shared.withOverride(skin: homeBandSkin(band), paletteId: "default") {
                let banded = try XCTUnwrap(OffscreenRender.bitmap(of: homeChrome(banded: true), size: size, colorScheme: .light, scale: 1))
                let plain = try XCTUnwrap(OffscreenRender.bitmap(of: homeChrome(banded: false), size: size, colorScheme: .light, scale: 1))
                let xs = Array(stride(from: 0, to: 900, by: 7))
                let drawsBand = xs.contains { x in banded.colorAt(x: x, y: 0) != plain.colorAt(x: x, y: 0) }
                XCTAssertTrue(drawsBand, "\(band) draws in the first row")
                // The row just under the band is Home's own page background; Home is the same, moved down by the band.
                for y in [0, 40, 200] {
                    for x in xs { XCTAssertEqual(banded.colorAt(x: x, y: y + height), plain.colorAt(x: x, y: y), "\(band) x \(x) y \(y)") }
                }
            }
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
