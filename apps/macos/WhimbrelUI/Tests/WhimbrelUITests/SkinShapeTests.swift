import AppKit
import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

/// Richer skins, row 5 (docs/superpowers/plans/2026-11-25-richer-skins.md): the Mac draws a double border as two lines with a gap
/// of the same width, inside the shape's edge, and Thread draws as before.
@Suite struct SkinShapeTests {
    private func theme(_ shape: WShape, variant: WVariant = .light) -> WTheme {
        WTheme(palette: [:], variant: variant, shape: shape)
    }

    @Test func doubleIsABorderStyle() {
        #expect(WBorderStyle.allCases.contains(.double))
        #expect(WBorderStyle(rawValue: "double") == .double)
    }

    @Test func aDoubleBorderDrawsTwoLinesWithAGapOfTheSameWidth() {
        let t = theme(WShape(borderWidth: 1, borderStyle: .double))
        #expect(t.drawsDoubleBorder(width: t.borderWidth))
        #expect(t.borderStroke.dash.isEmpty)
        #expect(WBorderGeometry.insets(width: 1, double: true) == [0, 1, 2, 3])
        #expect(WBorderGeometry.insets(width: 2, double: true) == [0, 2, 4, 6])
        #expect(WBorderGeometry.thickness(width: 1.5, double: true) == 4.5)
    }

    @Test func aSolidBorderIsOneLineAsBefore() {
        #expect(WBorderGeometry.insets(width: 1, double: false) == [0, 1])
        #expect(WBorderGeometry.thickness(width: 2, double: false) == 2)
        for style in [WBorderStyle.solid, .dashed, .none] {
            let t = theme(WShape(borderWidth: 2, borderStyle: style))
            #expect(!t.drawsDoubleBorder(width: 2), "\(style)")
        }
        #expect(!WTheme.thread(variant: .light).drawsDoubleBorder(width: WTheme.thread(variant: .light).borderWidth))
        #expect(WShape.thread.borderStyle == .solid)
    }

    @Test func aBorderUnderOnePointDrawsSolid() {
        let t = theme(WShape(borderWidth: 0.5, borderStyle: .double))
        #expect(!t.drawsDoubleBorder(width: 0.5))
        #expect(WBorderGeometry.insets(width: 0.5, double: true) == [0, 0.5])
    }

    @Test func highContrastDrawsOneFirmLine() {
        let t = theme(WShape(borderWidth: 1, highContrastBorderWidth: 1.5, borderStyle: .double), variant: .highContrastLight)
        #expect(!t.drawsDoubleBorder(width: t.borderWidth))
        #expect(t.borderWidth == 1.5)
    }

    @Test func doubleKeepsItsWidthAndDensity() {
        let t = theme(WShape(controlRadius: 1, cardRadius: 2, surfaceRadius: 3, borderWidth: 1, borderStyle: .double, density: .roomy))
        #expect(t.borderWidth == 1)
        #expect(t.shape.surfaceRadius == 3)
        #expect(t.space(10) == 12.5)
    }

    // MARK: Pixels

    /// Renders a 24 point square border at scale 1 and reads the alpha along the middle row, left edge to centre.
    @MainActor private func alphaAcrossLeftEdge(_ shape: WShape, width: CGFloat) -> [CGFloat] {
        let t = theme(shape)
        let view = Rectangle().wBorder(.black, style: StrokeStyle(lineWidth: width))
            .frame(width: 24, height: 24)
            .environment(\.wTheme, t)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1
        guard let cg = renderer.cgImage else { return [] }
        let rep = NSBitmapImageRep(cgImage: cg)
        return (0..<8).map { rep.colorAt(x: $0, y: 12)?.alphaComponent ?? -1 }
    }

    @Test @MainActor func aDoubleBorderShowsAGapBetweenTwoLines() {
        let alpha = alphaAcrossLeftEdge(WShape(borderWidth: 1, borderStyle: .double), width: 1)
        #expect(alpha.count == 8)
        // line, gap, line, then the inside
        #expect(alpha[0] > 0.9)
        #expect(alpha[1] < 0.1)
        #expect(alpha[2] > 0.9)
        #expect(alpha[3] < 0.1)
        #expect(alpha[4] < 0.1)
    }

    @Test func aDoubleBorderIsCappedAtSixPointsInAll() {
        // thin settings are exactly as before: line, gap, line of one width each
        #expect(WBorderGeometry.insets(width: 1, double: true) == [0, 1, 2, 3])
        #expect(WBorderGeometry.insets(width: 1.5, double: true) == [0, 1.5, 3, 4.5])
        #expect(WBorderGeometry.insets(width: 2, double: true) == [0, 2, 4, 6])
        // past two points each band and the gap stop at two
        #expect(WBorderGeometry.insets(width: 3, double: true) == [0, 2, 4, 6])
        #expect(WBorderGeometry.insets(width: 4, double: true) == [0, 2, 4, 6])
        for width in stride(from: CGFloat(1), through: 4, by: 0.25) {
            #expect(WBorderGeometry.thickness(width: width, double: true) <= 6)
        }
    }

    @Test @MainActor func aFourPointDoubleBorderDrawsTwoPointLinesAndStopsAtSix() {
        let alpha = alphaAcrossLeftEdge(WShape(borderWidth: 4, borderStyle: .double), width: 4)
        #expect(alpha.count == 8)
        // 2 point line, 2 point gap, 2 point line, then the inside
        #expect(alpha[0] > 0.9 && alpha[1] > 0.9)
        #expect(alpha[2] < 0.1 && alpha[3] < 0.1)
        #expect(alpha[4] > 0.9 && alpha[5] > 0.9)
        #expect(alpha[6] < 0.1 && alpha[7] < 0.1)
    }

    @Test @MainActor func aSolidBorderIsOneLine() {
        let alpha = alphaAcrossLeftEdge(.thread, width: 1)
        #expect(alpha.count == 8)
        #expect(alpha[0] > 0.9)
        for a in alpha.dropFirst() { #expect(a < 0.1) }
    }
}
