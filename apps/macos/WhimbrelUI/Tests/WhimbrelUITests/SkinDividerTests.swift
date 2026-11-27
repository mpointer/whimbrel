import AppKit
import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

/// Richer skins, row 6 (docs/superpowers/plans/2026-11-25-richer-skins.md, section 3): a skin's dividers draw between Home's
/// sections and in its time lines, are hidden from VoiceOver, and the ruled line skips the hero. Thread draws none.
@Suite struct SkinDividerTests {
    private func theme(_ dividers: WDividerStyle, variant: WVariant = .light) -> WTheme {
        WTheme(palette: [:], variant: variant, shape: WShape(dividers: dividers))
    }

    @Test func theFiveStylesAreTheFormatsFive() {
        #expect(WDividerStyle.allCases.map(\.rawValue) == ["none", "hairline", "chalk", "double", "dotted"])
        #expect(WShape.thread.dividers == .none)
        #expect(WTheme.thread(variant: .light).shape.dividers == .none)
    }

    @Test func eachStyleHasItsStrokes() {
        #expect(WDividerGeometry.strokes(.none).isEmpty)
        #expect(WDividerGeometry.thickness(.none) == 0)
        let hairline = WDividerGeometry.strokes(.hairline)
        #expect(hairline.count == 1 && hairline[0].width == 1 && hairline[0].opacity == 1 && hairline[0].dash.isEmpty)
        let chalk = WDividerGeometry.strokes(.chalk)
        #expect(chalk.count == 1 && chalk[0].width == 2 && chalk[0].opacity == 0.6 && chalk[0].roundCaps)
        let double = WDividerGeometry.strokes(.double)
        #expect(double.map(\.width) == [1, 1])
        #expect(double.map(\.offset) == [0, 3]) // two 1 point strokes with a 2 point gap
        #expect(WDividerGeometry.thickness(.double) == 4)
        let dotted = WDividerGeometry.strokes(.dotted)
        #expect(dotted.count == 1 && dotted[0].width == 1 && dotted[0].dash == [1, 2]) // 1 point dots, 3 point pitch
        #expect(WDividerGeometry.thickness(.hairline) == 1 && WDividerGeometry.thickness(.chalk) == 2 && WDividerGeometry.thickness(.dotted) == 1)
    }

    @Test func increaseContrastDrawsChalkAtFullOpacity() {
        #expect(WDividerGeometry.strokes(.chalk, highContrast: true)[0].opacity == 1)
    }

    // MARK: The ruled line skips the hero

    private func rows(_ kinds: [String]) -> [WHomeLineRow] {
        kinds.map { $0 == "hero" ? WHomeLineRow.hero : WHomeLineRow.stop(.notice($0)) }
    }

    @Test func theRuledLineDrawsNoRuleNextToTheHeroOrAboveTheFirstRow() {
        #expect(WHomeRuledRules.ruleAbove(rows(["a", "b", "hero", "c", "d"])) == [false, true, false, false, true])
        #expect(WHomeRuledRules.ruleAbove(rows(["hero", "a", "b"])) == [false, false, true])
        #expect(WHomeRuledRules.ruleAbove(rows(["a", "b", "c"])) == [false, true, true])
        #expect(WHomeRuledRules.ruleAbove([]).isEmpty)
    }

    // MARK: Hidden from VoiceOver, and drawn

    @MainActor private func render(_ view: some View, size: CGSize) -> NSBitmapImageRep? {
        let renderer = ImageRenderer(content: view.frame(width: size.width, height: size.height, alignment: .topLeading))
        renderer.scale = 1
        return renderer.cgImage.map(NSBitmapImageRep.init(cgImage:))
    }

    @Test @MainActor func aDoubleDividerDrawsTwoLinesWithAGap() {
        let t = theme(.double)
        let view = WDividerStrokes(axis: .horizontal, color: .black).environment(\.wTheme, t)
        guard let rep = render(view, size: CGSize(width: 20, height: 6)) else { Issue.record("no image"); return }
        let alpha = (0..<6).map { rep.colorAt(x: 10, y: $0)?.alphaComponent ?? -1 }
        #expect(alpha[0] > 0.9)
        #expect(alpha[1] < 0.1 && alpha[2] < 0.1)
        #expect(alpha[3] > 0.9)
        #expect(alpha[4] < 0.1)
    }

    @Test @MainActor func aDottedDividerDrawsOnePointDotsThreePointsApart() {
        let t = theme(.dotted)
        let view = WDividerStrokes(axis: .horizontal, color: .black).environment(\.wTheme, t)
        guard let rep = render(view, size: CGSize(width: 30, height: 3)) else { Issue.record("no image"); return }
        let row = (0..<12).map { rep.colorAt(x: $0, y: 0)?.alphaComponent ?? -1 }
        let on = row.enumerated().filter { $0.element > 0.5 }.map(\.offset)
        #expect(on.count >= 3)
        for pair in zip(on, on.dropFirst()) { #expect(pair.1 - pair.0 == 3) }
    }

    @Test @MainActor func aChalkDividerIsSixtyPercentOpaque() {
        let view = WDividerStrokes(axis: .horizontal, color: .black).environment(\.wTheme, theme(.chalk))
        guard let rep = render(view, size: CGSize(width: 30, height: 4)) else { Issue.record("no image"); return }
        let a = rep.colorAt(x: 15, y: 0)?.alphaComponent ?? -1
        #expect(abs(a - 0.6) < 0.05)
        #expect((rep.colorAt(x: 15, y: 3)?.alphaComponent ?? 1) < 0.1)
    }

    @Test @MainActor func theSpineKeepsItsConnectorsOpacityAndTakesTheStylesWidth() {
        // chalk is 2 points wide; the connector's own 45 percent stays, it is not multiplied by chalk's 60
        let view = WDividerStrokes(axis: .vertical, color: Color.black.opacity(0.45), usesStrokeOpacity: false).environment(\.wTheme, theme(.chalk))
        guard let rep = render(view, size: CGSize(width: 2, height: 20)) else { Issue.record("no image"); return }
        let a = rep.colorAt(x: 1, y: 10)?.alphaComponent ?? -1
        #expect(abs(a - 0.45) < 0.05)
    }

    @Test @MainActor func aDividerIsHiddenFromTheAccessibilityTree() {
        // Every divider is built from WDividerStrokes, which is marked hidden; the plain rule of a skin with no divider style
        // is hidden in the ruled line too. The checks read the source so a new divider view cannot forget it.
        let file = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/WhimbrelUI/Dividers.swift")
        let source = (try? String(contentsOf: file, encoding: .utf8)) ?? ""
        #expect(source.contains(".accessibilityHidden(true)"))
        let home = file.deletingLastPathComponent().appendingPathComponent("HomeViews.swift")
        let homeSource = (try? String(contentsOf: home, encoding: .utf8)) ?? ""
        #expect(homeSource.contains(".fill(theme.border).frame(height: max(1, theme.borderWidth)).accessibilityHidden(true)"))
    }
}
