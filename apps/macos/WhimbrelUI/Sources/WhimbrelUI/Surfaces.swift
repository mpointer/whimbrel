import SwiftUI

enum WSurfaceKind { case sidebar, floating }

/// Which of the skin's radii a shape uses.
enum WRadiusKind { case control, card, surface }

extension WTheme {
    func radius(_ kind: WRadiusKind) -> CGFloat {
        switch kind {
        case .control: return shape.controlRadius
        case .card: return shape.cardRadius
        case .surface: return shape.surfaceRadius
        }
    }
}

extension StrokeStyle {
    /// The same stroke with the line width raised to at least `minimum` (used where a solid fill
    /// replaces glass and needs a visible edge). A zero-width stroke stays zero.
    func widened(_ minimum: CGFloat) -> StrokeStyle {
        var copy = self
        if copy.lineWidth > 0 { copy.lineWidth = max(copy.lineWidth, minimum) }
        return copy
    }
}

/// Glass for navigation and controls only, never for content. macOS 26 gets `glassEffect`; earlier
/// systems get a material. Reduce Transparency and high contrast get a solid fill with a border.
private struct WGlassSurface: ViewModifier {
    @Environment(\.wTheme) private var theme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    let kind: WSurfaceKind

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.surfaceRadius, style: .continuous)
        let solid = kind == .sidebar ? theme.backgroundPage : theme.backgroundCard
        if reduceTransparency || theme.isHighContrast {
            content
                .background(shape.fill(solid))
                .overlay(shape.wBorder(theme.border, style: theme.borderStroke.widened(1)))
        } else if #available(macOS 26, *) {
            content
                .glassEffect(.regular, in: shape)
                .wShadow(kind == .floating ? WTokens.Elevation.level2 : WTokens.Elevation.level0)
        } else {
            content
                .background(.regularMaterial, in: shape)
                .overlay(shape.wBorder(theme.border.opacity(0.6), style: theme.borderStroke))
                .wShadow(kind == .floating ? WTokens.Elevation.level2 : WTokens.Elevation.level0)
        }
    }
}

private struct WShadowModifier: ViewModifier {
    @Environment(\.wTheme) private var theme
    let level: WTokens.Shadow

    func body(content: Content) -> some View {
        if theme.isHighContrast || level.opacity == 0 {
            content
        } else {
            content.shadow(color: theme.scrim.opacity(level.opacity), radius: level.blur / 2, x: 0, y: level.y)
        }
    }
}

private struct WCardModifier: ViewModifier {
    @Environment(\.wTheme) private var theme
    let radiusKind: WRadiusKind
    let fill: Color?
    let elevation: WTokens.Shadow

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: theme.radius(radiusKind), style: .continuous)
        content
            .background(shape.fill(fill ?? theme.backgroundCard))
            .overlay(shape.wBorder(theme.border.opacity(theme.isHighContrast ? 1 : 0.55), style: theme.borderStroke))
            .modifier(WShadowModifier(level: elevation))
    }
}

extension View {
    func wGlass(kind: WSurfaceKind) -> some View {
        modifier(WGlassSurface(kind: kind))
    }

    func wShadow(_ level: WTokens.Shadow) -> some View {
        modifier(WShadowModifier(level: level))
    }

    /// A content card: solid fill, hairline border, level 1 shadow (a border in high contrast).
    func wCard(radius: WRadiusKind = .card, fill: Color? = nil, elevation: WTokens.Shadow = WTokens.Elevation.level1) -> some View {
        modifier(WCardModifier(radiusKind: radius, fill: fill, elevation: elevation))
    }
}

private struct WSimulatedFocus: ViewModifier {
    @Environment(\.wTheme) private var theme
    let on: Bool
    let radiusKind: WRadiusKind

    func body(content: Content) -> some View {
        content.overlay {
            if on {
                RoundedRectangle(cornerRadius: theme.radius(radiusKind) + 2, style: .continuous)
                    .strokeBorder(theme.focus, lineWidth: 2.5)
                    .padding(-2)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
        }
    }
}

extension View {
    /// Draws a stand-in for the system focus ring. Only the gallery uses it, because keyboard focus
    /// cannot be shown in an offscreen render. The components themselves use the system ring.
    func wSimulatedFocus(_ on: Bool, radius: WRadiusKind = .control) -> some View {
        modifier(WSimulatedFocus(on: on, radiusKind: radius))
    }
}

/// Lays children out left to right and wraps them onto new lines.
struct WFlowLayout: Layout {
    var spacing: CGFloat = 8
    var lineSpacing: CGFloat = 8

    /// A child is measured at its ideal size, as before. Only a child wider than the container is proposed the container's width, so a long
    /// chip truncates inside it instead of running past the trailing edge.
    static func proposal(idealWidth: CGFloat, container: CGFloat?) -> ProposedViewSize {
        guard let container, container.isFinite, container > 0, idealWidth > container else { return .unspecified }
        return ProposedViewSize(width: container, height: nil)
    }

    /// How far past the container's edge a line may run before the next child wraps. A row that is exactly as wide as its children
    /// (a card that sizes itself to its chips) is given that width back, and sums of fractional widths (a monospaced face, a compact
    /// skin's spacing) can come out a hair above it. Without this slack the last chip wraps onto a second line the card never
    /// measured, and the next row draws over it.
    static let wrapTolerance: CGFloat = 0.01

    /// Whether a child of `width` fits on the line that already holds `used` points (the end of the last child plus its spacing).
    /// `sizeThatFits` and `placeSubviews` both decide through this, so a card always has the height of the lines it draws.
    static func fits(used: CGFloat, width: CGFloat, container: CGFloat) -> Bool {
        used == 0 || used + width <= container + wrapTolerance
    }

    private func measured(_ subview: LayoutSubview, container: CGFloat?) -> CGSize {
        let ideal = subview.sizeThatFits(.unspecified)
        let proposal = Self.proposal(idealWidth: ideal.width, container: container)
        guard proposal.width != nil else { return ideal }
        return subview.sizeThatFits(proposal)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let maxWidth = proposal.width ?? .infinity
        var x: CGFloat = 0, y: CGFloat = 0, rowHeight: CGFloat = 0, width: CGFloat = 0
        for subview in subviews {
            let size = measured(subview, container: proposal.width)
            if !Self.fits(used: x, width: size.width, container: maxWidth) {
                y += rowHeight + lineSpacing
                x = 0
                rowHeight = 0
            }
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
            width = max(width, x - spacing)
        }
        return CGSize(width: width, height: y + rowHeight)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, rowHeight: CGFloat = 0
        for subview in subviews {
            let size = measured(subview, container: bounds.width)
            if !Self.fits(used: x - bounds.minX, width: size.width, container: bounds.width) {
                y += rowHeight + lineSpacing
                x = bounds.minX
                rowHeight = 0
            }
            // Ideal size, as measured: a proposal of the measured size can fall a hair short of it and truncate the last word. Only a child
            // wider than the container is given the container's width.
            subview.place(at: CGPoint(x: x, y: y), anchor: .topLeading, proposal: Self.proposal(idealWidth: subview.sizeThatFits(.unspecified).width, container: bounds.width))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
    }
}

/// Where the edges of a border sit, measured inward from the outer edge of the shape. Consecutive pairs are the filled bands.
public enum WBorderGeometry {
    /// The most a band or the gap of a double border takes (Mike, 2026-11-25): six points in all, line, gap, line.
    public static let doubleBandLimit: CGFloat = 2

    /// A solid border is one band of `width`. A double border is two bands with a gap of the same size between them, each
    /// `min(width, 2)`, so it is at most six points thick, and all of it sits inside the shape's edge: the layout box does not
    /// grow. A width under one point is too fine to show two lines, so it draws solid.
    public static func insets(width: CGFloat, double: Bool) -> [CGFloat] {
        if double && width >= 1 {
            let band = min(width, doubleBandLimit)
            return [0, band, 2 * band, 3 * band]
        }
        return [0, width]
    }

    /// The thickness a border takes inside its shape.
    public static func thickness(width: CGFloat, double: Bool) -> CGFloat { insets(width: width, double: double).last ?? 0 }
}

private struct WBorderView<S: Shape>: View {
    @Environment(\.wTheme) private var theme
    let shape: S
    let color: Color
    let style: StrokeStyle

    var body: some View {
        GeometryReader { proxy in
            let rect = CGRect(origin: .zero, size: proxy.size)
            let w = style.lineWidth
            if style.dash.isEmpty {
                let insets = WBorderGeometry.insets(width: w, double: theme.drawsDoubleBorder(width: w))
                Path { p in
                    for inset in insets { p.addPath(shape.path(in: rect.insetBy(dx: inset, dy: inset))) }
                }
                .fill(color, style: FillStyle(eoFill: true))
            } else {
                shape.path(in: rect.insetBy(dx: w / 2, dy: w / 2)).stroke(color, style: style)
            }
        }
        .allowsHitTesting(false)
    }
}

extension Shape {
    /// An inside border. A solid border is one even-odd fill between the outer shape and an inset
    /// copy, so a translucent color does not double up where a stroke starts and ends (that left
    /// ticks at the ends of capsules). When the skin's border is double the same fill has two bands (see `WBorderGeometry`).
    /// A dashed border is an inset stroke.
    func wBorder(_ color: Color, style: StrokeStyle) -> some View {
        WBorderView(shape: self, color: color, style: style)
    }
}
