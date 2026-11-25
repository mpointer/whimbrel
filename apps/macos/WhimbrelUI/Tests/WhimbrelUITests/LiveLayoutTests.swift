import SwiftUI
import Testing
@testable import WhimbrelUI

/// The live screen's column sizes: the transcript is about 38 percent of the width, between 320 and 560.
@Suite struct LiveLayoutTests {
    private let gap: CGFloat = 16
    private func arrangement(_ w: CGFloat, stops: Bool = true, transcript: Bool = true, k: CGFloat = 1) -> WLiveLayout.Arrangement {
        WLiveLayout.arrangement(width: w, hasStops: stops, showsTranscript: transcript, gap: gap, k: k)
    }
    private func side(_ w: CGFloat, k: CGFloat = 1, stops: Bool = true) -> CGFloat {
        WLiveLayout.sideWidth(available: w, timeline: WLiveLayout.timelineWidth(hasStops: stops, spine: 96, gap: gap, k: k), gap: gap, k: k)
    }

    // 112 (time line) + 440 (main) + 16 (gap) + 320 (transcript minimum) = 888
    @Test func threeColumnsStartAtTheMinimumTranscriptWidth() {
        #expect(arrangement(887) == .stacked)
        #expect(arrangement(888) == .three)
    }

    @Test func stackedAndStackedWithoutTimelineThresholdsAreUnchanged() {
        #expect(arrangement(552) == .stacked)
        #expect(arrangement(551) == .stackedWithoutTimeline)
    }

    @Test func withoutTranscriptOrTimelineTheThresholdsMove() {
        #expect(arrangement(552, transcript: false) == .three)
        #expect(arrangement(776, stops: false) == .three)
        #expect(arrangement(775, stops: false) == .stacked)
    }

    @Test func textScaleScalesTheThresholds() {
        #expect(arrangement(1000, k: 1.5) == .stacked)
        #expect(arrangement(1000 + 400, k: 1.5) == .three)
    }

    @Test func theTranscriptIsAboutThirtyEightPercentOfTheWidth() {
        #expect(abs(side(1200) - 456) < 0.001)
        #expect(abs(side(1000) - 380) < 0.001)
    }

    @Test func theTranscriptNeverGoesUnderTheMinimumOrOverTheMaximum() {
        #expect(side(888) == 320)
        #expect(side(600) == 320)
        #expect(side(1600) == 560)
        #expect(side(3000) == 560)
    }

    @Test func theMainColumnKeepsItsMinimumAtEveryWidthThatFitsThreeColumns() {
        for w in stride(from: CGFloat(888), through: 2400, by: 8) {
            #expect(arrangement(w) == .three)
            let main = w - 112 - 16 - side(w)
            #expect(main >= 440, "width \(w) leaves \(main)")
            #expect(side(w) >= 320 && side(w) <= 560)
        }
    }

    @Test func theTranscriptLimitsGrowWithTextSize() {
        #expect(side(3000, k: 1.5) == 560 * 1.5)
        #expect(side(1400, k: 1.5) >= 320 * 1.5)
    }

    // MARK: The highlighted line

    private func entry(_ id: String, _ state: WTranscriptLineState = .normal) -> WTranscriptEntry {
        WTranscriptEntry(id: id, time: "10:01 AM", speaker: WPerson(name: "Sam"), text: "Hello", state: state)
    }

    @Test func theHighlightedEntryShowsAsCitedAndOthersKeepTheirState() {
        #expect(WTranscriptView.shownState(entry("live-7"), highlightedId: "live-7") == .cited)
        #expect(WTranscriptView.shownState(entry("live-8"), highlightedId: "live-7") == .normal)
        #expect(WTranscriptView.shownState(entry("live-8", .live), highlightedId: "live-7") == .live)
        #expect(WTranscriptView.shownState(entry("live-8", .live), highlightedId: nil) == .live)
    }

    @Test func theLiveModelCarriesTheHighlightedId() {
        #expect(WLiveMeetingModel(title: "t").highlightedEntryId == nil)
        #expect(WLiveMeetingModel(title: "t", highlightedEntryId: "live-3").highlightedEntryId == "live-3")
    }
}
