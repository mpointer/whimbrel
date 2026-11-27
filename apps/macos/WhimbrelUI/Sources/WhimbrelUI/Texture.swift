import AppKit
import SwiftUI

/// The page texture a skin asks for. It is drawn by code on the page background only, never on cards or inputs.
public enum WTexture: String, Sendable, CaseIterable { case none, paper, grain, grid }

/// How a texture is drawn and how strong it may be (richer skins row 9, docs/superpowers/plans/2026-11-25-richer-skins.md).
/// A texture is a faint layer of black or white ink over the page color, blended the way the screen blends it (in the encoded
/// sRGB values). Its strength is capped per page color so that, at the strongest point of the noise or at a grid line, the
/// page's relative luminance (the one contrast ratios use) moves by no more than `maxLuminanceShift`. That keeps every
/// contrast pair repair checked valid.
public enum WTextureSpec {
    /// The most a texture may move the page color, in relative luminance (0 is black, 1 is white). Four percent.
    public static let maxLuminanceShift: Double = 0.04

    /// What the strength is solved for, a little under the cap so 8-bit rounding cannot cross it.
    static let targetShift: Double = 0.039

    /// Spacing of the grid's lines, in points.
    public static let gridSpacing: CGFloat = 24

    /// The ink's strongest alpha before the cap: how strong the texture is on a page that has room for it. A light page has the
    /// least room (black ink on white moves luminance fastest), so there the cap lowers it.
    public static func nominalAlpha(_ texture: WTexture) -> Double {
        switch texture {
        case .none: return 0
        case .paper: return 0.05
        case .grain: return 0.06
        case .grid: return 0.05
        }
    }

    /// Size of one speck of noise, in points. Paper is fine, grain is coarser.
    static func cell(_ texture: WTexture) -> Int { texture == .grain ? 2 : 1 }

    /// Relative luminance (WCAG) of encoded sRGB channels, each 0 to 1.
    static func luminance(r: Double, g: Double, b: Double) -> Double {
        func lin(_ c: Double) -> Double { c <= 0.03928 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4) }
        return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    /// WCAG relative luminance of 0xRRGGBB.
    public static func luminance(_ hex: UInt32) -> Double {
        luminance(r: Double((hex >> 16) & 0xFF) / 255, g: Double((hex >> 8) & 0xFF) / 255, b: Double(hex & 0xFF) / 255)
    }

    /// White ink on a dark page, black on a light one.
    public static func usesWhiteInk(page: UInt32) -> Bool { luminance(page) < 0.5 }

    /// How far ink of `alpha` moves `page`'s luminance, blended in encoded sRGB as the screen does.
    public static func luminanceShift(alpha: Double, page: UInt32) -> Double {
        let ink: Double = usesWhiteInk(page: page) ? 1 : 0
        func mix(_ shift: UInt32) -> Double { let v = Double((page >> shift) & 0xFF) / 255; return v * (1 - alpha) + ink * alpha }
        return abs(luminance(r: mix(16), g: mix(8), b: mix(0)) - luminance(page))
    }

    /// The strongest alpha `texture` may draw on `page`: its nominal alpha, lowered where the page has too little room for it
    /// to stay inside `maxLuminanceShift`.
    public static func peakAlpha(_ texture: WTexture, page: UInt32) -> Double {
        let nominal = nominalAlpha(texture)
        if nominal == 0 || luminanceShift(alpha: nominal, page: page) <= targetShift { return nominal }
        var low = 0.0, high = nominal
        for _ in 0..<40 {
            let mid = (low + high) / 2
            if luminanceShift(alpha: mid, page: page) <= targetShift { low = mid } else { high = mid }
        }
        return low
    }

    /// The most the texture can move `page`'s luminance, at its strongest point.
    public static func luminanceShift(_ texture: WTexture, page: UInt32) -> Double {
        luminanceShift(alpha: peakAlpha(texture, page: page), page: page)
    }

    // MARK: Tiles

    /// A tile of noise as a bitmap, one speck per `cell` points, each with a deterministic alpha from 0 to 1 (the view scales it
    /// to the page's `peakAlpha`).
    static func noiseTile(_ texture: WTexture, whiteInk: Bool) -> NSImage {
        let points = 128
        let scale = 2
        let px = points * scale
        let cellPx = cell(texture) * scale
        var state: UInt64 = texture == .paper ? 0x9E37_79B9_7F4A_7C15 : 0xD1B5_4A32_D192_ED03
        func next() -> Double {
            state = state &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            return Double((state >> 33) & 0x7FFF_FFFF) / Double(0x7FFF_FFFF)
        }
        var data = [UInt8](repeating: 0, count: px * px * 4)
        for cy in 0..<(px / cellPx) {
            for cx in 0..<(px / cellPx) {
                // paper is soft: the mean of two draws leans to the middle
                let r = texture == .paper ? (next() + next()) / 2 : next()
                let a = UInt8((r * 255).rounded())
                let c: UInt8 = whiteInk ? 255 : 0
                for y in 0..<cellPx {
                    for x in 0..<cellPx {
                        let i = ((cy * cellPx + y) * px + cx * cellPx + x) * 4
                        data[i] = c; data[i + 1] = c; data[i + 2] = c; data[i + 3] = a
                    }
                }
            }
        }
        let image = NSImage(size: NSSize(width: points, height: points))
        if let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
            isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: px * 4, bitsPerPixel: 32), let buffer = rep.bitmapData {
            data.withUnsafeBufferPointer { buffer.update(from: $0.baseAddress!, count: data.count) }
            rep.size = NSSize(width: points, height: points)
            image.addRepresentation(rep)
        }
        return image
    }

    /// The largest alpha in a noise tile, 0 to 1, for tests.
    static func peakAlpha(ofTile image: NSImage) -> Double {
        guard let rep = image.representations.first as? NSBitmapImageRep, let data = rep.bitmapData else { return 0 }
        var most: UInt8 = 0
        for i in stride(from: 3, to: rep.bytesPerRow * rep.pixelsHigh, by: 4) { most = max(most, data[i]) }
        return Double(most) / 255
    }
}

/// The tiles, made once on the main actor (a view's body runs there).
@MainActor private enum WTextureTiles {
    private static var cache: [String: NSImage] = [:]
    static func tile(_ texture: WTexture, whiteInk: Bool) -> NSImage {
        let key = "\(texture.rawValue)-\(whiteInk)"
        if let hit = cache[key] { return hit }
        let made = WTextureSpec.noiseTile(texture, whiteInk: whiteInk)
        cache[key] = made
        return made
    }
}

/// The texture layer, for behind the content of a page. Draws nothing when the theme draws no texture.
struct WTextureView: View {
    @Environment(\.wTheme) private var theme

    var body: some View {
        let texture = theme.texture
        let page = theme.palette["backgroundPage"] ?? 0
        let white = WTextureSpec.usesWhiteInk(page: page)
        let alpha = WTextureSpec.peakAlpha(texture, page: page)
        switch texture {
        case .none:
            EmptyView()
        case .paper, .grain:
            Image(nsImage: WTextureTiles.tile(texture, whiteInk: white))
                .resizable(resizingMode: .tile)
                .interpolation(.none)
                .opacity(alpha)
                .accessibilityHidden(true)
                .allowsHitTesting(false)
        case .grid:
            Canvas { context, size in
                let ink = Color(white: white ? 1 : 0).opacity(alpha)
                var path = Path()
                var x: CGFloat = 0
                while x <= size.width { path.addRect(CGRect(x: x, y: 0, width: 1, height: size.height)); x += WTextureSpec.gridSpacing }
                var y: CGFloat = 0
                while y <= size.height { path.addRect(CGRect(x: 0, y: y, width: size.width, height: 1)); y += WTextureSpec.gridSpacing }
                context.fill(path, with: .color(ink))
            }
            .accessibilityHidden(true)
            .allowsHitTesting(false)
        }
    }
}

private struct WPageBackground: ViewModifier {
    @Environment(\.wTheme) private var theme

    @ViewBuilder func body(content: Content) -> some View {
        if theme.drawsTexture {
            // The page color stays the outer background (so it fills as it always did); the texture sits between it and the content.
            content.background { WTextureView() }.background(theme.backgroundPage)
        } else {
            content.background(theme.backgroundPage)
        }
    }
}

extension View {
    /// The page background: the page color, and the skin's texture over it when the theme draws one. Never used for cards or inputs.
    func wPageBackground() -> some View { modifier(WPageBackground()) }
}
