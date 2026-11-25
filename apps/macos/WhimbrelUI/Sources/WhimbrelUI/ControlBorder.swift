import SwiftUI

/// Color math for the control edge. The edge colors themselves are `WThreadOnly.controlBorder`, generated from
/// `threadOnly.controlBorder` in `design/tokens/tokens.json` (order: light, dark, high contrast light, high contrast dark).
/// The edge of an input, select, number box and segmented choice must be at least 3:1 on the card and on the page (WCAG 1.4.11),
/// unlike `border`, the hairline between cards. Buttons keep their own edges; this is not used for them.
enum WControlEdge {
    static func mix(_ fg: UInt32, over bg: UInt32, alpha: Double) -> UInt32 {
        func channel(_ shift: UInt32) -> UInt32 {
            let f = Double((fg >> shift) & 0xFF), b = Double((bg >> shift) & 0xFF)
            return UInt32((alpha * f + (1 - alpha) * b).rounded())
        }
        return channel(16) << 16 | channel(8) << 8 | channel(0)
    }

    static func contrast(_ a: UInt32, _ b: UInt32) -> Double {
        func lum(_ hex: UInt32) -> Double {
            func c(_ v: UInt32) -> Double {
                let x = Double(v) / 255
                return x <= 0.03928 ? x / 12.92 : pow((x + 0.055) / 1.055, 2.4)
            }
            return 0.2126 * c((hex >> 16) & 0xFF) + 0.7152 * c((hex >> 8) & 0xFF) + 0.0722 * c(hex & 0xFF)
        }
        let (x, y) = (lum(a), lum(b))
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }
}

extension WControlEdge {
    /// Whether words in `ink` are readable on `fill`: 4.5 to 1 or better, the least for text this size (WCAG 1.4.3).
    static func readable(_ ink: UInt32, on fill: UInt32) -> Bool { contrast(ink, fill) >= 4.5 }
}

extension WTheme {
    /// The ink for words on the selection fill (the chosen filter or order chip). A skin's accent is not always readable on its
    /// selection color (3.7 to 4.3 to 1 in Classic, Game Day and Hypermodern palettes), so the accent is used only where it
    /// is, as in Thread, and the text color otherwise. The fill and the weight still mark the chosen chip.
    public var accentOnSelection: Color {
        guard let accent = palette["accentPrimary"], let fill = palette["selection"], WControlEdge.readable(accent, on: fill) else { return textPrimary }
        return accentPrimary
    }

    /// Whether the palette in force is Thread's own: its hairline and card are Thread's for this variant. A skin that sets its
    /// own palette keeps its own edge (`firmEdge`), as Windows falls back to the skin's border.
    var usesThreadPalette: Bool {
        let thread = variant.threadPalette
        return palette["border"] == thread["border"] && palette["backgroundCard"] == thread["backgroundCard"]
            && palette["backgroundPage"] == thread["backgroundPage"]
    }

    /// The hex of the edge a text box, picker or number box draws. With Thread's palette, whichever of the control border token and
    /// the old `firmEdge` (the secondary ink at 70% over the card) has the higher contrast on the card, so no control gets a weaker
    /// edge than before. In light and dark the old edge wins on the card; in high contrast the token does.
    var controlBorderValue: UInt32? {
        guard usesThreadPalette else { return nil }
        let token = WThreadOnly.controlBorder.value(for: variant)
        guard !isHighContrast, let ink = palette["textSecondary"], let card = palette["backgroundCard"] else { return token }
        let old = WControlEdge.mix(ink, over: card, alpha: 0.7)
        return WControlEdge.contrast(old, card) > WControlEdge.contrast(token, card) ? old : token
    }

    /// The edge of a text box, search field, picker, number box or segmented choice. Thread: `controlBorderValue` (3:1 or better
    /// on the card and the page, never weaker than the old `firmEdge`), and the border itself in high contrast, as before. Another palette:
    /// `firmEdge`, its own softened ink.
    public var controlBorder: Color {
        if let value = controlBorderValue { return Color(wHex: value) }
        return firmEdge
    }
}
