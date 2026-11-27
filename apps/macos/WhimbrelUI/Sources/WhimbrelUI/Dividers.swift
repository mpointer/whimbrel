import SwiftUI

// Dividers (richer skins row 6, docs/superpowers/plans/2026-11-25-richer-skins.md, section 3). A skin's `decoration.dividers`
// draws the rules between Home's sections, between the rows of the ruled time line, and as the spine of the line time line.
// Nothing else in the app reads it. A divider is decoration: it is hidden from VoiceOver. Thread draws none.

/// One line of a divider, measured across it from the divider's first edge.
public struct WDividerStroke: Equatable, Sendable {
    public var offset: CGFloat
    public var width: CGFloat
    public var opacity: Double
    /// Dash lengths along the line. Empty is a continuous line.
    public var dash: [CGFloat]
    public var roundCaps: Bool
}

/// The strokes of each divider style, in points.
public enum WDividerGeometry {
    /// `none` draws nothing. `hairline` is one 1 point stroke. `chalk` is one 2 point stroke at 60 percent opacity with round
    /// caps. `double` is two 1 point strokes with a 2 point gap. `dotted` is 1 point dots at 3 point spacing. Increase Contrast
    /// draws every stroke at full opacity.
    public static func strokes(_ style: WDividerStyle, highContrast: Bool = false) -> [WDividerStroke] {
        switch style {
        case .none:
            return []
        case .hairline:
            return [WDividerStroke(offset: 0, width: 1, opacity: 1, dash: [], roundCaps: false)]
        case .chalk:
            return [WDividerStroke(offset: 0, width: 2, opacity: highContrast ? 1 : 0.6, dash: [], roundCaps: true)]
        case .double:
            return [
                WDividerStroke(offset: 0, width: 1, opacity: 1, dash: [], roundCaps: false),
                WDividerStroke(offset: 3, width: 1, opacity: 1, dash: [], roundCaps: false),
            ]
        case .dotted:
            return [WDividerStroke(offset: 0, width: 1, opacity: 1, dash: [1, 2], roundCaps: false)]
        }
    }

    /// How much room a divider takes across its line.
    public static func thickness(_ style: WDividerStyle) -> CGFloat {
        strokes(style).map { $0.offset + $0.width }.max() ?? 0
    }
}

private struct WSpineDividersKey: EnvironmentKey { static let defaultValue = false }

extension EnvironmentValues {
    /// True inside Home's line time line, the only spine a skin's dividers draw. Other screens that use a spine keep theirs.
    var wSpineDividers: Bool {
        get { self[WSpineDividersKey.self] }
        set { self[WSpineDividersKey.self] = newValue }
    }
}

enum WDividerAxis { case horizontal, vertical }

/// One stroke of a divider as filled geometry: a bar the stroke's width across, with a gap pattern along it when it has dashes.
/// Filled, not stroked, so a 1 point dot lands on whole pixels instead of being left to the dash engine.
private struct WDividerLine: Shape {
    let axis: WDividerAxis
    let stroke: WDividerStroke

    func path(in rect: CGRect) -> Path {
        var p = Path()
        let length = axis == .horizontal ? rect.width : rect.height
        let pitch = stroke.dash.reduce(0, +)
        var segments: [(CGFloat, CGFloat)] = []
        if stroke.dash.count >= 2, pitch > 0 {
            var at: CGFloat = 0
            while at < length {
                segments.append((at, min(stroke.dash[0], length - at)))
                at += pitch
            }
        } else {
            segments = [(0, length)]
        }
        for (start, run) in segments where run > 0 {
            let bar: CGRect
            switch axis {
            case .horizontal: bar = CGRect(x: rect.minX + start, y: rect.minY + stroke.offset, width: run, height: stroke.width)
            case .vertical: bar = CGRect(x: rect.minX + stroke.offset, y: rect.minY + start, width: stroke.width, height: run)
            }
            if stroke.roundCaps { p.addRoundedRect(in: bar, cornerSize: CGSize(width: stroke.width / 2, height: stroke.width / 2)) } else { p.addRect(bar) }
        }
        return p
    }
}

/// A divider's strokes, drawn along one axis in `color`. Decorative: hidden from VoiceOver.
struct WDividerStrokes: View {
    @Environment(\.wTheme) private var theme
    let axis: WDividerAxis
    let color: Color
    /// A rule uses the style's own opacity (chalk is 60 percent). A spine keeps the opacity of its connector, which says before
    /// or after the present, and takes only the style's width and pattern.
    var usesStrokeOpacity = true

    var body: some View {
        let strokes = WDividerGeometry.strokes(theme.shape.dividers, highContrast: theme.isHighContrast)
        let thickness = WDividerGeometry.thickness(theme.shape.dividers)
        ZStack(alignment: .topLeading) {
            ForEach(Array(strokes.enumerated()), id: \.offset) { _, s in
                WDividerLine(axis: axis, stroke: s).fill(color.opacity(usesStrokeOpacity ? s.opacity : 1))
            }
        }
        .frame(width: axis == .vertical ? thickness : nil, height: axis == .horizontal ? thickness : nil)
        .frame(maxWidth: axis == .horizontal ? .infinity : nil, maxHeight: axis == .vertical ? .infinity : nil)
        .accessibilityHidden(true)
    }
}

/// The horizontal rule a skin draws between Home's sections and between rows of the ruled time line. It is drawn only when the
/// skin's dividers are not `none`; callers check `theme.shape.dividers` first so Thread's layout does not change.
struct WDivider: View {
    @Environment(\.wTheme) private var theme
    var body: some View { WDividerStrokes(axis: .horizontal, color: theme.border) }
}

/// Where the ruled time line draws a rule: above a row, between it and the one before. None above the first row, and none
/// next to the hero, which is already a framed block of its own.
enum WHomeRuledRules {
    static func ruleAbove(_ rows: [WHomeLineRow]) -> [Bool] {
        func isHero(_ r: WHomeLineRow) -> Bool { if case .hero = r { return true } else { return false } }
        return rows.indices.map { $0 > 0 && !isHero(rows[$0]) && !isHero(rows[$0 - 1]) }
    }
}
