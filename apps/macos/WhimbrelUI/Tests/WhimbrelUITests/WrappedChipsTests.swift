import AppKit
import SwiftUI
import Testing
@testable import WhimbrelUI

/// A meeting row's attendee chips sit in a flow layout that wraps. Retro Terminal (a monospaced face, compact spacing) drew two
/// chips whose widths and gap added up to the row's width within a rounding hair; the last chip wrapped onto a second line the
/// card had not measured, so the next card drew over it. The lines a layout draws and the height it reports must agree.
@MainActor
@Suite struct WrappedChipsTests {
    /// The widths of Retro Terminal's "Jo Park" and "Rae Kim" chips at the gallery's size, and the gap its compact density gives.
    private let chip: CGFloat = 85.60000000000001
    private let gap: CGFloat = 4.8
    private let chipHeight: CGFloat = 24.8

    @Test func twoChipsInARowExactlyTheirWidthShareALine() {
        // Placement adds the chips to the row's left edge, so the running end carries that edge's rounding too.
        let edge: CGFloat = 180.4
        let used = (edge + chip + gap) - edge
        #expect(WFlowLayout.fits(used: used, width: chip, container: 2 * chip + gap))
        #expect(WFlowLayout.fits(used: 0, width: 500, container: 100), "the first child always takes the line")
    }

    @Test func aChipThatReallyDoesNotFitWraps() {
        #expect(!WFlowLayout.fits(used: chip + gap, width: chip, container: 2 * chip + gap - 1))
    }

    private func height(chips: Int, container: CGFloat, leading: CGFloat = 180.4) -> (height: CGFloat, maxY: CGFloat) {
        final class Box { var maxY: CGFloat = 0 }
        let box = Box()
        let view = WFlowLayout(spacing: gap, lineSpacing: gap) {
            ForEach(0..<chips, id: \.self) { _ in
                Color.clear.frame(width: chip, height: chipHeight)
                    .background(GeometryReader { geo in
                        Color.clear.onAppear { box.maxY = max(box.maxY, geo.frame(in: .named("flow")).maxY) }
                    })
            }
        }
        .coordinateSpace(name: "flow")
        .frame(width: container, alignment: .leading)
        .padding(.leading, leading)
        let host = NSHostingView(rootView: view)
        host.frame = CGRect(x: 0, y: 0, width: container + leading, height: 400)
        let window = NSWindow(contentRect: host.frame, styleMask: [], backing: .buffered, defer: false)
        window.contentView = host
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        return (host.fittingSize.height, box.maxY)
    }

    /// The row sits at some distance from the card's edge, and placement adds the chips to that distance, so how the sum rounds
    /// depends on it. Sweep it: no distance may wrap a chip that fits.
    @Test func aRowExactlyAsWideAsItsChipsStaysOnOneLineAtAnyOffset() {
        for step in 0..<400 {
            let leading = CGFloat(step) * 0.7 + 0.1
            let r = height(chips: 2, container: 176.0, leading: leading)
            #expect(r.height < chipHeight + gap, "at \(leading): height \(r.height) holds more than one line")
            #expect(r.maxY < chipHeight + gap, "at \(leading): the last chip was drawn at \(r.maxY)")
        }
    }

    @Test func aWrappedChipGrowsTheRowAndNeverDrawsBelowIt() {
        let r = height(chips: 2, container: 150)
        #expect(r.height > 2 * chipHeight, "height \(r.height) holds fewer than two lines")
        #expect(r.maxY <= r.height + 0.01, "the last chip was drawn at \(r.maxY), under a row \(r.height) high")
    }
}
