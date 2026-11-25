import SwiftUI
import Testing
@testable import WhimbrelUI

/// Whether the live transcript follows its newest line.
@Suite struct TranscriptFollowTests {
    @Test func followsFromTheStartAndScrollsToANewLine() {
        var f = WTranscriptFollow()
        #expect(f.following)
        let n = f.newLine(); #expect(n)
    }

    @Test func scrollingUpStopsFollowingAndNewLinesStayPut() {
        var f = WTranscriptFollow()
        f.position(distanceBelow: 300)
        #expect(!f.following)
        let n = f.newLine(); #expect(!n)
    }

    @Test func scrollingBackToTheBottomResumes() {
        var f = WTranscriptFollow()
        f.position(distanceBelow: 300)
        f.position(distanceBelow: 10)
        #expect(f.following)
        let n = f.newLine(); #expect(n)
    }

    @Test func nearTheBottomCountsAsTheBottom() {
        var f = WTranscriptFollow()
        f.position(distanceBelow: WTranscriptFollow.tolerance)
        #expect(f.following)
        f.position(distanceBelow: WTranscriptFollow.tolerance + 1)
        // The first report after nothing pending is believed at once.
        #expect(!f.following)
    }

    @Test func theGrowthOfANewLineIsNotTakenForScrollingUp() {
        var f = WTranscriptFollow()
        let n = f.newLine(); #expect(n)
        f.position(distanceBelow: 60)   // content grew, the scroll has not caught up
        #expect(f.following)
        f.position(distanceBelow: 0)    // caught up
        #expect(f.following)
    }

    @Test func aPersonScrollingUpRightAfterALineIsBelievedAfterTheGrowthReports() {
        var f = WTranscriptFollow()
        let n = f.newLine(); #expect(n)
        for _ in 0..<WTranscriptFollow.growthReports { f.position(distanceBelow: 400) }
        #expect(f.following)
        f.position(distanceBelow: 400)
        #expect(!f.following)
    }

    @Test func aHighlightPausesFollowingWhileItLasts() {
        var f = WTranscriptFollow()
        f.highlight(active: true)
        let n = f.newLine(); #expect(!n)
        f.highlight(active: false)
        let m = f.newLine(); #expect(m)
    }

    @Test func afterAJumpFollowingResumesOnlyBackAtTheBottom() {
        var f = WTranscriptFollow()
        f.highlight(active: true)
        f.position(distanceBelow: 900)   // the jump moved the view up
        f.highlight(active: false)
        let n = f.newLine(); #expect(!n)
        f.position(distanceBelow: 0)
        let m = f.newLine(); #expect(m)
    }

    @Test func manyLinesWhileNotFollowingNeverScroll() {
        var f = WTranscriptFollow()
        f.position(distanceBelow: 500)
        for _ in 0..<5 { let n = f.newLine(); #expect(!n) }
    }
}
