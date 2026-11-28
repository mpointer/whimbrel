import SwiftUI

// Colour numbers the palette repair and the skin header band still use. They come from the Glass and Field skins, whose
// whole-page Today layouts are gone.

/// The numbers behind the glass look, shared with the contrast test.
enum GlassStyle {
    /// How much of `backgroundCard` covers the material. `.ultraThinMaterial` shows what is behind
    /// it, so the tint is mostly opaque: text contrast is then set by the palette, not the backdrop.
    static let cardOpacity = 0.9
    /// Strength of `accentPrimary` at the top of the gradient (it fades to `backgroundPage`).
    static let glowOpacity = 0.3

    /// `fg` at `alpha` over opaque `bg`, as 0xRRGGBB.
    static func composite(_ fg: UInt32, alpha: Double, over bg: UInt32) -> UInt32 {
        func channel(_ shift: UInt32) -> UInt32 {
            UInt32((Double((fg >> shift) & 0xFF) * alpha + Double((bg >> shift) & 0xFF) * (1 - alpha)).rounded())
        }
        return channel(16) << 16 | channel(8) << 8 | channel(0)
    }

    /// The glow's colours, top to bottom in tenths, over opaque `under` (the page, or a scrimmed background
    /// image): `accent` at `glowOpacity` fading to opaque `page`, interpolated unpremultiplied (its middle
    /// carries a little more accent than its top).
    static func glow(accent: UInt32, page: UInt32, over under: UInt32) -> [UInt32] {
        (0...10).map { step in
            let t = Double(step) / 10
            let colour = composite(page, alpha: t, over: accent)
            return composite(colour, alpha: glowOpacity + (1 - glowOpacity) * t, over: under)
        }
    }
}

/// The numbers behind the pitch drawing, shared with the contrast test.
enum FieldStyle {
    /// Strength of the `accentPrimary` mowing stripes over the pitch (`backgroundCard`).
    static let stripeOpacity = 0.10
    /// Strength of the `border` chalk lines over the pitch. Labels may cross one.
    static let lineOpacity = 0.45
}

/// Diagonal stripes in the accent colour, for the header band.
struct DiagonalStripes: View {
    var body: some View {
        Canvas { context, size in
            var path = Path()
            for x in stride(from: -size.height, to: size.width, by: 10) {
                path.move(to: CGPoint(x: x, y: size.height)); path.addLine(to: CGPoint(x: x + size.height, y: 0))
            }
            context.stroke(path, with: .color(Color.whimbrelAccentPrimary), lineWidth: 3)
        }
    }
}
