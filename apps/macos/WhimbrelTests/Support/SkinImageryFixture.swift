import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

private final class PlacementsBox { var placements: Set<SkinImagePlacement> = [] }

/// A temp user-skins folder that `SkinImageResolver` reads for the duration of a test, and skins
/// re-id'd as `user-` copies holding a header, background and badge image in it.
@MainActor
final class SkinImageryFixture {
    let root: URL
    let store: UserSkinStore

    init(_ test: XCTestCase) throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent("skin-imagery-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        store = UserSkinStore(root: root)
        let previous = SkinImageResolver.userSkins
        SkinImageResolver.userSkins = store
        SkinImageResolver.invalidate()
        let root = root
        test.addTeardownBlock { @MainActor in
            SkinImageResolver.userSkins = previous
            SkinImageResolver.invalidate()
            try? FileManager.default.removeItem(at: root)
        }
    }

    /// An 8x8 PNG: the left half white, the right half black, so the scrim has both ends to cover.
    nonisolated static let png: Data = {
        let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: 8, pixelsHigh: 8, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
            isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        )!
        let context = NSGraphicsContext(bitmapImageRep: rep)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        NSColor.white.setFill()
        NSRect(x: 0, y: 0, width: 4, height: 8).fill()
        NSColor.black.setFill()
        NSRect(x: 4, y: 0, width: 4, height: 8).fill()
        NSGraphicsContext.restoreGraphicsState()
        return rep.representation(using: .png, properties: [:])!
    }()

    static var image: NSImage { NSImage(data: png)! }

    /// Writes `data` as `images/<asset>` of `skinId`.
    func write(_ data: Data = SkinImageryFixture.png, asset: String, skinId: String) throws {
        let url = store.imageURL(asset: asset, skinId: skinId)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: url)
    }

    /// `base` as a user skin whose header, background (not text-free) and badge images exist.
    func skin(from base: Skin, textFreeBackground: Bool = false) throws -> Skin {
        var skin = base
        skin.id = "user-imagery-\(base.id)"
        skin.decoration.imagery = [
            SkinImage(asset: "header.png", placement: .header, textFree: false),
            SkinImage(asset: "background.png", placement: .background, textFree: textFreeBackground),
            SkinImage(asset: "badge.png", placement: .badge, textFree: false),
        ]
        for image in skin.decoration.imagery { try write(asset: image.asset, skinId: skin.id) }
        return skin
    }

    /// The placements whose `SkinImageryView` `view` draws, read from a hosting view. Preferences arrive
    /// asynchronously, so this polls (layout + run loop) until the set equals `expected` or 2 s pass; when
    /// nothing is expected it watches the whole 0.5 s so a late image still shows up.
    static func drawnPlacements(
        _ view: some View, expecting expected: Set<SkinImagePlacement>, width: CGFloat = 760, dark: Bool = false
    ) -> Set<SkinImagePlacement> {
        let box = PlacementsBox()
        let host = NSHostingView(rootView: view.onPreferenceChange(SkinImageryPlacementsKey.self) { box.placements = $0 })
        host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        host.frame = CGRect(x: 0, y: 0, width: width, height: 2000)
        let deadline = Date().addingTimeInterval(expected.isEmpty ? 0.5 : 2)
        repeat {
            host.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        } while Date() < deadline && (expected.isEmpty || box.placements != expected)
        return box.placements
    }

    /// The colour at (`x`, `y`) points of `view` rendered at `size`, in sRGB.
    static func pixel(_ view: some View, size: CGSize, dark: Bool, x: Int, y: Int) -> NSColor? {
        OffscreenRender.bitmap(of: view, size: size, colorScheme: dark ? .dark : .light, scale: 1)?
            .colorAt(x: x, y: y)?.usingColorSpace(.sRGB)
    }

    static func chrome(_ content: TodayContent = .sample) -> some View {
        TodayChrome(
            content: content, actions: .noop,
            slots: TodaySlots(live: nil, decisionCount: content.pendingApprovalCount, actions: .noop, composer: TodayFixtures.composer)
        )
    }
}
