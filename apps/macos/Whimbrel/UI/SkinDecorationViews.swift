import AppKit
import SwiftUI

/// The skin's header band at the top of a top-level screen. `none` (Whimbrel, High Contrast) draws nothing,
/// so those screens are unchanged. (Named `...View` because `SkinHeaderBand` is the manifest enum.) Reads the
/// skin and colours when drawn; `ThemedRoot`'s `.id(revision)` rebuilds it when the skin or palette changes.
struct SkinHeaderBandView: View {
    static func draws(_ band: SkinHeaderBand) -> Bool { band != .none }

    var body: some View {
        switch SkinStore.shared.selected.decoration.headerBand {
        case .none:
            EmptyView()
        case .rule:
            Rectangle().fill(Color.whimbrelAccentPrimary).frame(height: 2)
        case .block:
            Rectangle().fill(Color.whimbrelAccentPrimary).frame(height: 12)
        case .stripesDiagonal:
            DiagonalStripes().frame(height: 8).clipped().accessibilityHidden(true)
        }
    }
}

/// A top-level screen under the skin's header image and header band, both outside the screen's own
/// padding and scrolling so they stay put. A skin with neither (Whimbrel, High Contrast) adds nothing.
struct SkinHeaderArea<Screen: View>: View {
    @ViewBuilder let screen: () -> Screen

    var body: some View {
        VStack(spacing: 0) {
            SkinImagerySlot(placement: .header)
            SkinHeaderBandView()
            screen()
        }
    }
}

/// The placements whose image is on screen; read by tests from a hosting view.
struct SkinImageryPlacementsKey: PreferenceKey {
    static var defaultValue: Set<SkinImagePlacement> = []
    static func reduce(value: inout Set<SkinImagePlacement>, nextValue: () -> Set<SkinImagePlacement>) { value.formUnion(nextValue()) }
}

/// The selected skin's image for `placement`, sized for it: `header` a full-width 120 pt band, `badge`
/// 48 pt square, `background` whatever it is given. Draws nothing (and takes no space) when the skin has
/// no such image, the file is missing, or it is a text-free background.
struct SkinImagerySlot: View {
    static let headerHeight: CGFloat = 120
    static let badgeSide: CGFloat = 48

    let placement: SkinImagePlacement

    /// The entry and its bitmap, when it would be drawn.
    static func resolved(_ placement: SkinImagePlacement) -> (entry: SkinImage, image: NSImage)? {
        let skin = SkinStore.shared.selected
        guard let entry = skin.decoration.imagery.first(where: { $0.placement == placement }), SkinImageryView.isDrawn(entry),
              let image = SkinImageResolver.image(for: entry, skin: skin) else { return nil }
        return (entry, image)
    }

    var body: some View {
        if let (entry, image) = Self.resolved(placement) {
            let frame: (width: CGFloat?, height: CGFloat?) = switch placement {
            case .header: (nil, Self.headerHeight)
            case .badge: (Self.badgeSide, Self.badgeSide)
            case .background: (nil, nil)
            }
            Color.clear
                .frame(width: frame.width, height: frame.height)
                .frame(maxWidth: placement == .header ? .infinity : nil)
                .overlay { SkinImageryView(skinImage: entry, image: image) }
                .clipped()
                .accessibilityHidden(true)
        }
    }
}

extension View {
    /// `self` (a Today greeting) with the skin's badge image beside it; unchanged when there is none.
    @ViewBuilder
    func skinBadge() -> some View {
        if SkinImagerySlot.resolved(.badge) != nil {
            HStack(alignment: .top, spacing: Spacing.md) {
                self
                SkinImagerySlot(placement: .badge)
            }
        } else {
            self
        }
    }
}

/// A skin image; a background gets a scrim over it so every text colour on top reads at 4.5:1.
struct SkinImageryView: View {
    let skinImage: SkinImage
    let image: NSImage
    @Environment(\.colorScheme) private var colorScheme

    /// The text and scrim (page) colours for an appearance.
    static func resolvedColors(isDark: Bool) -> (textPrimary: UInt32, scrim: UInt32) {
        (ThemeStore.shared.hex(.textPrimary, isDark: isDark), ThemeStore.shared.hex(.backgroundPage, isDark: isDark))
    }

    /// Every colour Today draws text in (`foregroundStyle` in UI/Today); the scrim is sized for the worst of them.
    static let textTokens: [ThemeToken] = [.textPrimary, .textSecondary, .accentPrimary, .error, .link]

    /// Glass lays its glow (`GlassStyle.glow`) over the background, so its scrim is sized for the text on that too.
    func opacity(isDark: Bool) -> Double {
        let text = Self.textTokens.map { ThemeStore.shared.hex($0, isDark: isDark) }
        let glow = SkinStore.shared.selected.archetype == .glass
            ? (accent: ThemeStore.shared.hex(.accentPrimary, isDark: isDark), text: [ThemeToken.textPrimary, .textSecondary].map { ThemeStore.shared.hex($0, isDark: isDark) })
            : nil
        return SkinImageResolver.scrimOpacity(for: image, text: text, scrim: Self.resolvedColors(isDark: isDark).scrim, glow: glow)
    }

    /// Only the background has text over it. Header and badge sit beside the text (above Today, above a screen's
    /// band, beside the greeting: `SkinImagerySlot`'s callers), so they draw the image as it is.
    static func hasScrim(_ placement: SkinImagePlacement) -> Bool { placement == .background }

    /// Imagery marked text-free is never placed under text, and a background is under the text.
    static func isDrawn(_ image: SkinImage) -> Bool { !(image.textFree && image.placement == .background) }

    var body: some View {
        if Self.isDrawn(skinImage) {
            let isDark = colorScheme == .dark
            Image(nsImage: image)
                .resizable()
                .scaledToFill()
                .overlay(Color(nsColor: NSColor(srgbHex: Self.resolvedColors(isDark: isDark).scrim)).opacity(Self.hasScrim(skinImage.placement) ? opacity(isDark: isDark) : 0))
                .clipped()
                .preference(key: SkinImageryPlacementsKey.self, value: [skinImage.placement])
        }
    }

    /// `over` shown at `opacity` on top of `under`, per sRGB channel.
    static func blend(_ under: UInt32, over: UInt32, opacity: Double) -> UInt32 {
        func mix(_ shift: UInt32) -> UInt32 {
            let u = Double((under >> shift) & 0xFF), o = Double((over >> shift) & 0xFF)
            return UInt32((u * (1 - opacity) + o * opacity).rounded()) << shift
        }
        return mix(16) | mix(8) | mix(0)
    }

    /// The smallest opacity (in 1% steps) at which every `text` colour reaches 4.5:1 over the lightest and the darkest
    /// colour under the scrim, and with a `glow` (Glass) its text (the greeting and date: primary and secondary; the
    /// rest sits in cards) on every colour of the glow laid over those too; 1 when even a solid scrim falls short.
    static func scrimOpacity(text: [UInt32], scrim: UInt32, lightest: UInt32, darkest: UInt32, glow: (accent: UInt32, text: [UInt32])? = nil) -> Double {
        for step in 0...100 {
            let opacity = Double(step) / 100
            if [lightest, darkest].allSatisfy({ under in
                func reads(_ colours: [UInt32], on surface: UInt32) -> Bool { colours.allSatisfy { ThemeContrast.ratio($0, surface) >= ThemeContrast.minimum } }
                let shown = blend(under, over: scrim, opacity: opacity)
                guard reads(text, on: shown) else { return false }
                guard let glow else { return true }
                return GlassStyle.glow(accent: glow.accent, page: scrim, over: shown).allSatisfy { reads(glow.text, on: $0) }
            }) {
                return opacity
            }
        }
        return 1
    }

    /// The lightest and darkest colours (by luminance) in a 64x64 sample of the image, transparent pixels counting as `page`.
    static func extremes(of image: NSImage, page: UInt32) -> (lightest: UInt32, darkest: UInt32) {
        let n = 64
        guard let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: n, pixelsHigh: n, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
            isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        ), let context = NSGraphicsContext(bitmapImageRep: rep) else { return (0xFFFFFF, 0x000000) }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        context.imageInterpolation = .none
        NSColor(srgbHex: page).setFill()
        NSRect(x: 0, y: 0, width: n, height: n).fill()
        image.draw(in: NSRect(x: 0, y: 0, width: n, height: n), from: .zero, operation: .sourceOver, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
        var lightest: (UInt32, Double) = (0, -1), darkest: (UInt32, Double) = (0, 2)
        for y in 0..<n {
            for x in 0..<n {
                guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                let hex = UInt32((c.redComponent * 255).rounded()) << 16 | UInt32((c.greenComponent * 255).rounded()) << 8 | UInt32((c.blueComponent * 255).rounded())
                let l = ThemeContrast.relativeLuminance(hex)
                if l > lightest.1 { lightest = (hex, l) }
                if l < darkest.1 { darkest = (hex, l) }
            }
        }
        return (lightest.0, darkest.0)
    }
}
