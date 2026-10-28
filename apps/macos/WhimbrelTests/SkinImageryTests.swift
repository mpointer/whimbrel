import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

@MainActor
final class SkinImageryTests: XCTestCase {
    private func solid(_ color: NSColor) -> NSImage {
        let image = NSImage(size: NSSize(width: 1, height: 1))
        image.lockFocus()
        color.setFill()
        NSRect(x: 0, y: 0, width: 1, height: 1).fill()
        image.unlockFocus()
        return image
    }

    private func gradient() -> NSImage {
        let image = NSImage(size: NSSize(width: 64, height: 64))
        image.lockFocus()
        NSGradient(starting: .white, ending: .black)!.draw(in: NSRect(x: 0, y: 0, width: 64, height: 64), angle: 0)
        image.unlockFocus()
        return image
    }

    /// Dark text on a light page and the reverse, as the real palettes pair them.
    private let pairs: [(text: UInt32, page: UInt32)] = [(0x111111, 0xFAFAFA), (0xF2F2F2, 0x101010), (0x1A1A1A, 0xF4EDE0)]

    func test_scrim_reaches_minimum_over_white_and_black_images() {
        for (text, page) in pairs {
            for image in [solid(.white), solid(.black), gradient()] {
                let (lightest, darkest) = SkinImageryView.extremes(of: image, page: page)
                let opacity = SkinImageryView.scrimOpacity(text: [text], scrim: page, lightest: lightest, darkest: darkest)
                XCTAssertGreaterThanOrEqual(opacity, 0)
                if image.size.width > 1 { XCTAssertGreaterThan(opacity, 0, "a gradient has an end the text cannot read on") }
                XCTAssertLessThanOrEqual(opacity, 1)
                for under in [lightest, darkest] {
                    let shown = SkinImageryView.blend(under, over: page, opacity: opacity)
                    XCTAssertGreaterThanOrEqual(ThemeContrast.ratio(text, shown), ThemeContrast.minimum, "text \(String(text, radix: 16)) over \(String(under, radix: 16))")
                }
                // The smallest such value: one step less fails for the worse of the two.
                if opacity > 0.01 {
                    let less = opacity - 0.01
                    let worst = [lightest, darkest].map { ThemeContrast.ratio(text, SkinImageryView.blend($0, over: page, opacity: less)) }.min()!
                    XCTAssertLessThan(worst, ThemeContrast.minimum)
                }
            }
        }
    }

    func test_extremes_of_white_and_black() {
        XCTAssertEqual(SkinImageryView.extremes(of: solid(.white), page: 0x808080).lightest, 0xFFFFFF)
        XCTAssertEqual(SkinImageryView.extremes(of: solid(.black), page: 0x808080).darkest, 0x000000)
        let g = SkinImageryView.extremes(of: gradient(), page: 0x808080)
        XCTAssertGreaterThan(ThemeContrast.relativeLuminance(g.lightest), 0.8)
        XCTAssertLessThan(ThemeContrast.relativeLuminance(g.darkest), 0.2)
    }

    func test_textFree_image_never_under_text() {
        let free = SkinImage(asset: "a.png", placement: .background, textFree: true)
        let plain = SkinImage(asset: "a.png", placement: .background, textFree: false)
        XCTAssertFalse(SkinImageryView.isDrawn(free), "a text-free background sits under the content")
        XCTAssertTrue(SkinImageryView.isDrawn(plain))
        // Header and badge sit beside the text, never under it.
        for placement in [SkinImagePlacement.header, .badge] {
            XCTAssertTrue(SkinImageryView.isDrawn(SkinImage(asset: "a.png", placement: placement, textFree: true)))
        }
    }

    func test_header_band_draws_nothing_only_for_none() {
        XCTAssertFalse(SkinHeaderBandView.draws(.none))
        for band in [SkinHeaderBand.rule, .block, .stripesDiagonal] { XCTAssertTrue(SkinHeaderBandView.draws(band)) }
    }

    func test_transparent_pixels_count_as_the_page_colour() {
        let clear = NSImage(size: NSSize(width: 4, height: 4))
        clear.lockFocus()
        NSColor.clear.setFill()
        NSRect(x: 0, y: 0, width: 4, height: 4).fill(using: .copy)
        clear.unlockFocus()
        let e = SkinImageryView.extremes(of: clear, page: 0x203040)
        XCTAssertEqual(e.lightest, 0x203040)
        XCTAssertEqual(e.darkest, 0x203040)
    }

    func test_scrim_caps_at_one_when_contrast_is_unreachable() {
        XCTAssertEqual(SkinImageryView.scrimOpacity(text: [0x808080], scrim: 0x808080, lightest: 0xFFFFFF, darkest: 0x000000), 1)
    }

    /// The defaults must follow the appearance: light text on a dark page in dark mode.
    func test_defaults_resolve_for_the_current_appearance() {
        func palette(text: UInt32, page: UInt32) -> ThemePalette {
            var p = ThemeDefaults.light
            p[.textPrimary] = text
            p[.backgroundPage] = page
            return p
        }
        let theme = Theme(id: "imagery-test", name: "t", author: "t", light: palette(text: 0x111111, page: 0xFAFAFA), dark: palette(text: 0xF2F2F2, page: 0x101010))
        let store = ThemeStore.shared
        let before = store.themes
        store.install(before + [theme])
        defer { store.install(before) }
        store.withOverride(paletteId: theme.id) {
            XCTAssertTrue(SkinImageryView.resolvedColors(isDark: false) == (0x111111, 0xFAFAFA))
            XCTAssertTrue(SkinImageryView.resolvedColors(isDark: true) == (0xF2F2F2, 0x101010))
            let view = SkinImageryView(skinImage: SkinImage(asset: "a.png", placement: .header, textFree: false), image: solid(.white))
            XCTAssertEqual(view.opacity(isDark: false), 0, "dark text reads on white")
            XCTAssertGreaterThan(view.opacity(isDark: true), 0, "light text needs a scrim over white")
        }
    }
}
