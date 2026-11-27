import AppKit
import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

/// Richer skins, row 9 (docs/superpowers/plans/2026-11-25-richer-skins.md): a skin's texture is drawn by code on the page
/// background, moves the page color by at most 4 percent in luminance on every built-in palette, and is off in high contrast.
@Suite struct SkinTextureTests {
    /// Every built-in palette in `skins/palettes`: the page color of each variant it ships.
    private func builtInPageColors() throws -> [(palette: String, variant: String, page: UInt32)] {
        // .../apps/macos/WhimbrelUI/Tests/WhimbrelUITests/<this file> up to the repo root
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let dir = root.appendingPathComponent("skins/palettes")
        let files = try FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil).filter { $0.pathExtension == "json" }
        var out: [(String, String, UInt32)] = []
        for file in files {
            let json = try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any] ?? [:]
            for variant in ["light", "dark"] {
                guard let colors = json[variant] as? [String: Any], let hex = colors["backgroundPage"] as? String else { continue }
                out.append((file.deletingPathExtension().lastPathComponent, variant, UInt32(hex.dropFirst(), radix: 16) ?? 0))
            }
        }
        return out
    }

    private let textures: [WTexture] = [.paper, .grain, .grid]

    @Test func everyBuiltInPaletteHasPagesToTest() throws {
        let pages = try builtInPageColors()
        #expect(Set(pages.map(\.palette)).count >= 12)
        #expect(pages.count >= 12)
    }

    @Test func noTextureMovesAnyBuiltInPalettePageByMoreThanFourPercent() throws {
        for (palette, variant, page) in try builtInPageColors() {
            for texture in textures {
                let shift = WTextureSpec.luminanceShift(texture, page: page)
                #expect(shift <= WTextureSpec.maxLuminanceShift, "\(palette) \(variant) \(texture)")
                #expect(shift > 0, "\(palette) \(variant) \(texture) draws something")
            }
        }
    }

    @Test func theCapIsFourPercentAndNoTextureIsStrongerThanIt() {
        #expect(WTextureSpec.maxLuminanceShift == 0.04)
        #expect(WTextureSpec.peakAlpha(.none, page: 0xFFFFFF) == 0)
        // even the worst page (mid grey of either ink) cannot move by more than the cap
        for hex in stride(from: 0, through: 255, by: 5) {
            let page = UInt32(hex) << 16 | UInt32(hex) << 8 | UInt32(hex)
            for texture in textures { #expect(WTextureSpec.luminanceShift(texture, page: page) <= 0.04) }
        }
    }

    @Test func aLightPageGetsLessInkThanADarkOneAndNeverMoreThanNominal() {
        for texture in textures {
            let white = WTextureSpec.peakAlpha(texture, page: 0xFFFFFF)
            let dark = WTextureSpec.peakAlpha(texture, page: 0x101418)
            #expect(white > 0 && white < dark)
            #expect(dark <= WTextureSpec.nominalAlpha(texture))
        }
    }

    @Test @MainActor func theNoiseTilesAreUnitNoiseThatTheViewScalesDown() {
        for texture in [WTexture.paper, .grain] {
            for white in [true, false] {
                let peak = WTextureSpec.peakAlpha(ofTile: WTextureSpec.noiseTile(texture, whiteInk: white))
                #expect(peak > 0.9 && peak <= 1, "\(texture) \(white)")
            }
        }
    }

    @Test func inkIsTheExtremeFartherFromThePage() {
        #expect(WTextureSpec.usesWhiteInk(page: 0x101418))
        #expect(!WTextureSpec.usesWhiteInk(page: 0xFFFFFF))
        #expect(!WTextureSpec.usesWhiteInk(page: 0xF7F2E7))
    }

    // MARK: Off where it must be off

    private func theme(_ variant: WVariant, texture: WTexture) -> WTheme {
        WTheme(palette: [:], variant: variant, texture: texture)
    }

    @Test func textureIsOffInHighContrastAndWhenThereIsNone() {
        for texture in textures {
            #expect(theme(.light, texture: texture).drawsTexture)
            #expect(theme(.dark, texture: texture).drawsTexture)
            #expect(!theme(.highContrastLight, texture: texture).drawsTexture)
            #expect(!theme(.highContrastDark, texture: texture).drawsTexture)
        }
        #expect(!theme(.light, texture: .none).drawsTexture)
    }

    @Test func increaseContrastTurnsTextureOff() {
        let on = WTheme(palette: [:], colorScheme: .light, contrast: .standard, texture: .grid)
        let increased = WTheme(palette: [:], colorScheme: .light, contrast: .increased, texture: .grid)
        #expect(on.drawsTexture)
        #expect(!increased.drawsTexture)
    }

    @Test func threadHasNoTexture() {
        #expect(WTheme.thread(variant: .light).texture == .none)
        #expect(!WTheme.thread(variant: .dark).drawsTexture)
    }

    // MARK: Pixels

    /// Renders a 96 point square page background and returns the relative luminance of every pixel.
    @MainActor private func pageLuminances(_ t: WTheme) -> [Double] {
        let view = Color.clear.frame(width: 96, height: 96).wPageBackground().environment(\.wTheme, t)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1
        guard let cg = renderer.cgImage else { return [] }
        let rep = NSBitmapImageRep(cgImage: cg)
        var out: [Double] = []
        for y in 0..<rep.pixelsHigh {
            for x in 0..<rep.pixelsWide {
                guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                let hex = UInt32(c.redComponent * 255 + 0.5) << 16 | UInt32(c.greenComponent * 255 + 0.5) << 8 | UInt32(c.blueComponent * 255 + 0.5)
                out.append(WTextureSpec.luminance(hex))
            }
        }
        return out
    }

    @Test @MainActor func aTexturedPageStaysWithinFourPercentOfThePageColorInEveryPixel() throws {
        for (palette, variant, page) in try builtInPageColors() {
            // the same page rendered with no texture is the baseline, so a color space difference in the renderer cancels out
            let flat = pageLuminances(WTheme(palette: ["backgroundPage": page], variant: variant == "dark" ? .dark : .light))
            let base = flat.first ?? 0
            #expect(flat.allSatisfy { abs($0 - base) < 0.0005 }, "\(palette) \(variant) is flat with no texture")
            for texture in textures {
                let t = WTheme(palette: ["backgroundPage": page], variant: variant == "dark" ? .dark : .light, texture: texture)
                let pixels = pageLuminances(t)
                #expect(pixels.count == 96 * 96, "\(palette) \(variant) \(texture)")
                let worst = pixels.map { abs($0 - base) }.max() ?? 1
                #expect(worst <= WTextureSpec.maxLuminanceShift + 0.001, "\(palette) \(variant) \(texture): \(worst)")
            }
        }
    }

    @Test @MainActor func aTexturedPageDrawsSomethingAndAPlainPageIsFlat() {
        let page: UInt32 = 0xF7F2E7
        let flat = pageLuminances(WTheme(palette: ["backgroundPage": page], variant: .light))
        let base = flat.first ?? 0
        #expect(flat.count == 96 * 96)
        for texture in textures {
            let pixels = pageLuminances(WTheme(palette: ["backgroundPage": page], variant: .light, texture: texture))
            #expect(pixels.contains { abs($0 - base) > 0.0005 }, "\(texture)")
        }
        #expect(flat.allSatisfy { abs($0 - base) < 0.0005 })
        // high contrast draws flat even when the skin asks for texture
        let hc = pageLuminances(WTheme(palette: ["backgroundPage": page], variant: .highContrastLight, texture: .grain))
        #expect(hc.allSatisfy { abs($0 - base) < 0.0005 })
    }
}
