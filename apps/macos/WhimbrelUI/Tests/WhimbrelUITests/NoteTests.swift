import Foundation
import Testing
@testable import WhimbrelUI

private func seg(_ id: String, _ start: Int64, _ key: String, end: Int64? = nil, part: Int = 0, divider: String? = nil) -> WNoteSegment {
    WNoteSegment(id: id, startCs: start, endCs: end, time: WLaneMath.clock(cs: start), speakerKey: key, speaker: key, text: "t", partIndex: part, dividerBefore: divider)
}

@Suite struct NoteTests {
    // MARK: Span rule

    @Test func aStoredEndWins() {
        #expect(WLaneMath.spanEnd(start: 100, storedEnd: 450, nextStart: 200) == 450)
    }

    @Test func noEndRunsToTheNextLine() {
        #expect(WLaneMath.spanEnd(start: 100, storedEnd: nil, nextStart: 900) == 900)
    }

    @Test func noEndIsCappedAtThirtySeconds() {
        #expect(WLaneMath.spanEnd(start: 100, storedEnd: nil, nextStart: 100_000) == 100 + WLaneMath.capCs)
        #expect(WLaneMath.capCs == 3000)
    }

    @Test func aStoredEndBeforeTheStartIsIgnored() {
        #expect(WLaneMath.spanEnd(start: 500, storedEnd: 400, nextStart: 800) == 800)
        #expect(WLaneMath.spanEnd(start: 500, storedEnd: 500, nextStart: nil) == 500 + WLaneMath.lastLineCs)
    }

    @Test func theLastLineHasAShortSpan() {
        #expect(WLaneMath.spanEnd(start: 1000, storedEnd: nil, nextStart: nil) == 1500)
    }

    // MARK: Lanes

    @Test func oneLanePerSpeakerInOrderOfFirstSpeech() {
        let groups = WLaneMath.groups(segments: [seg("a", 0, "Sam"), seg("b", 500, "Jo"), seg("c", 900, "Sam")], marks: [])
        #expect(groups.count == 1)
        #expect(groups[0].lanes.map(\.speaker) == ["Sam", "Jo"])
        #expect(groups[0].lanes[0].spans.map(\.id) == ["a", "c"])
        #expect(groups[0].lanes[0].spans[0].endCs == 500)
    }

    @Test func twoSpeakersWithTheSameShownNameStayApartByKey() {
        var a = seg("a", 0, "Speaker 1"), b = seg("b", 100, "Speaker 2")
        a.speaker = "Sam"; b.speaker = "Sam"
        #expect(WLaneMath.groups(segments: [a, b], marks: [])[0].lanes.count == 2)
    }

    @Test func aContinuedMeetingHasOneGroupPerPartWithItsDivider() {
        let groups = WLaneMath.groups(
            segments: [seg("a", 0, "Me"), seg("b", 600, "Me"), seg("c", 9000, "Speaker 1", part: 1, divider: "rejoined"), seg("d", 9400, "Speaker 2", part: 1)],
            marks: [])
        #expect(groups.count == 2)
        #expect(groups[0].startCs == 0)
        #expect(groups[1].startCs == 9000)
        #expect(groups[1].divider == "rejoined")
        #expect(groups[1].lanes.map(\.speaker) == ["Speaker 1", "Speaker 2"])
    }

    @Test func marksBecomeTicksInTheirPart() {
        let mark = WNoteMark(id: "k", atCs: 300, clock: "0:03", label: "")
        let late = WNoteMark(id: "l", atCs: 9200, clock: "1:32", label: "")
        let groups = WLaneMath.groups(segments: [seg("a", 0, "Me", end: 600), seg("b", 9000, "Me", end: 9600, part: 1)], marks: [mark, late])
        #expect(groups[0].ticks.map(\.id) == ["k"])
        #expect(groups[1].ticks.map(\.id) == ["l"])
    }

    @Test func aMarkOpensTheNearestLine() {
        let lines = [seg("a", 0, "Me"), seg("b", 1000, "Me")]
        #expect(WLaneMath.nearestSegment(to: 700, in: lines) == "b")
        #expect(WLaneMath.nearestSegment(to: 100, in: lines) == "a")
        #expect(WLaneMath.nearestSegment(to: 5, in: []) == nil)
    }

    @Test func clockIsMinutesAndSecondsEvenPastAnHour() {
        // The format the transcript has always shown, so every time on the page agrees.
        #expect(WLaneMath.clock(cs: 110_500) == "18:25")
        #expect(WLaneMath.clock(cs: 0) == "0:00")
        #expect(WLaneMath.clock(cs: 372_300) == "62:03")
        #expect(WLaneMath.clock(cs: -5) == "0:00")
    }

    @Test func aMarkAfterTheLastSpanKeepsItsTickAndStretchesTheRange() {
        let tail = WNoteMark(id: "t", atCs: 5000, clock: "0:50", label: "")
        let groups = WLaneMath.groups(segments: [seg("a", 0, "Me", end: 600)], marks: [tail])
        #expect(groups[0].ticks.map(\.id) == ["t"])
        #expect(groups[0].endCs > 5000)
    }

    @Test func aMarkInTheGapBetweenPartsBelongsToTheEarlierPartOnly() {
        let gap = WNoteMark(id: "g", atCs: 4000, clock: "0:40", label: "")
        let groups = WLaneMath.groups(
            segments: [seg("a", 0, "Me", end: 600), seg("b", 9000, "Me", end: 9600, part: 1)], marks: [gap])
        #expect(groups[0].ticks.map(\.id) == ["g"])
        #expect(groups[1].ticks.isEmpty)
        #expect(groups[0].endCs > 4000)
        #expect(groups.flatMap(\.ticks).count == 1)
    }

    @Test func everyMarkOfTheMeetingIsATickExactlyOnce() {
        let marks = (0..<12).map { WNoteMark(id: "m\($0)", atCs: Int64($0) * 1300, clock: "", label: "") }
        let groups = WLaneMath.groups(
            segments: [seg("a", 100, "Me", end: 600), seg("b", 3000, "Me", end: 3600, part: 1), seg("c", 9000, "Me", end: 9600, part: 2)], marks: marks)
        #expect(groups.flatMap(\.ticks).map(\.id).sorted() == marks.map(\.id).sorted())
        #expect(groups.flatMap(\.ticks).count == marks.count)
    }

    @Test func aMarkBeforeTheFirstLineStillGetsATickInTheFirstPart() {
        let early = WNoteMark(id: "e", atCs: 0, clock: "0:00", label: "")
        let groups = WLaneMath.groups(segments: [seg("a", 400, "Me", end: 600)], marks: [early])
        #expect(groups[0].ticks.map(\.id) == ["e"])
    }

    @Test func aClickLandsOnTheSpanThatHoldsTheTimeOrTheNearestWithinReach() {
        let lane = WLane(id: "l", speaker: "Me", spans: [
            WLaneSpan(id: "a", startCs: 100, endCs: 200, speaker: "Me", time: "0:01"), WLaneSpan(id: "b", startCs: 1000, endCs: 1100, speaker: "Me", time: "0:10"),
        ])
        #expect(WLaneMath.span(at: 150, in: lane, toleranceCs: 0)?.id == "a")
        #expect(WLaneMath.span(at: 230, in: lane, toleranceCs: 50)?.id == "a")
        #expect(WLaneMath.span(at: 600, in: lane, toleranceCs: 50) == nil)
        #expect(WLaneMath.span(at: 960, in: lane, toleranceCs: 50)?.id == "b")
    }

    @Test func aScreenReaderStepsThroughOneSpeakersSpansAndStopsAtTheEnds() {
        let lane = WLane(id: "l", speaker: "Me", spans: ["a", "b", "c"].enumerated().map {
            WLaneSpan(id: $0.element, startCs: Int64($0.offset) * 100, endCs: Int64($0.offset) * 100 + 50, speaker: "Me", time: "t")
        })
        #expect(WLaneMath.step(in: lane, from: nil, by: 1) == "a")
        #expect(WLaneMath.step(in: lane, from: nil, by: -1) == "c")
        #expect(WLaneMath.step(in: lane, from: "a", by: 1) == "b")
        #expect(WLaneMath.step(in: lane, from: "c", by: 1) == "c")
        #expect(WLaneMath.step(in: lane, from: "a", by: -1) == "a")
        #expect(WLaneMath.step(in: WLane(id: "e", speaker: "x", spans: []), from: nil, by: 1) == nil)
    }

    @Test func arrowKeysMoveAlongInTimeAndStayAtTheEnds() {
        let groups = WLaneMath.groups(segments: [seg("a", 0, "Sam"), seg("b", 500, "Jo"), seg("c", 900, "Sam")], marks: [])
        #expect(WLaneMath.neighbor(of: nil, by: 1, in: groups) == "a")
        #expect(WLaneMath.neighbor(of: "a", by: 1, in: groups) == "b")
        #expect(WLaneMath.neighbor(of: "c", by: 1, in: groups) == "c")
        #expect(WLaneMath.neighbor(of: "a", by: -1, in: groups) == "a")
        #expect(WLaneMath.otherLane(from: "a", by: 1, in: groups) == "b")
        #expect(WLaneMath.otherLane(from: "b", by: -1, in: groups) == "c")
        #expect(WLaneMath.otherLane(from: "b", by: 1, in: groups) == "b")
    }

    // MARK: Margin

    @Test func marginCardsSitLevelWithTheirLinesAndNeverOverlap() {
        let tops = WMarginLayout.tops(anchors: [100, 120, 400], heights: [60, 60, 60], minTop: 40, gap: 8)
        #expect(tops == [100, 168, 400])
    }

    @Test func aCardNeverRidesUpIntoTheIntroLine() {
        #expect(WMarginLayout.tops(anchors: [0, 0], heights: [50, 50], minTop: 70, gap: 10) == [70, 130])
    }

    @Test func theSelectedCardHoldsItsPlaceAndItsNeighboursAboveMakeRoom() {
        // Realistic: lines about 40 apart, compact cards 50 tall, the selected card 90 tall.
        let anchors: [CGFloat] = [100, 140, 180, 300]
        let heights: [CGFloat] = [50, 50, 90, 50]
        let tops = WMarginLayout.tops(anchors: anchors, heights: heights, minTop: 40, gap: 6, pinned: 2)
        #expect(tops[2] == 180)
        let aboveSelected: CGFloat = 180 - 6 - 50
        #expect(tops[1] == aboveSelected)
        #expect(tops[0] == aboveSelected - 6 - 50)
        #expect(tops[3] == 300)
        for index in 1..<tops.count { #expect(tops[index] >= tops[index - 1] + heights[index - 1] + 6) }
    }

    @Test func aCardAlwaysSitsAtItsLineWhenThereIsRoom() {
        let anchors: [CGFloat] = [100, 300, 500]
        let tops = WMarginLayout.tops(anchors: anchors, heights: [60, 60, 60], minTop: 40, gap: 6, pinned: 1)
        #expect(tops == anchors)
    }

    @Test func neighboursNeverRideUpIntoTheIntroWhenTheSelectedCardIsNearTheTop() {
        let tops = WMarginLayout.tops(anchors: [60, 80, 100], heights: [50, 50, 90], minTop: 56, gap: 6, pinned: 2)
        #expect(tops[0] >= 56)
        #expect(tops[1] >= tops[0] + 50 + 6)
        #expect(tops[2] >= tops[1] + 50 + 6)
    }

    @Test func driftDoesNotBuildUpAcrossCardsWhoseLinesHaveRoomBetweenThem() {
        // Three one-line decisions then a gap before the action items: the drift ends where the gap begins.
        let anchors: [CGFloat] = [100, 139, 178, 400, 470]
        let tops = WMarginLayout.tops(anchors: anchors, heights: Array(repeating: 46, count: 5), minTop: 40, gap: 6)
        #expect(tops[3] == 400)
        #expect(tops[4] == 470)
    }

    @Test func theMarginFoldsAwayWhenNarrow() {
        #expect(WMarginLayout.isWide(width: 1200, scale: 1))
        #expect(!WMarginLayout.isWide(width: 600, scale: 1))
        #expect(!WMarginLayout.isWide(width: 1000, scale: 2))
    }

    // MARK: Model

    @Test func sourcesAreInPageOrderAndAnEditedLineHasNone() {
        let src = WNoteSource(number: 1, segmentId: "s", speaker: "Jo", time: "1:00", quote: "q")
        let edited = WNoteLine(id: "e", text: "mine", isAI: false, source: src)
        #expect(edited.source == nil)
        let model = WNoteModel(title: "t", decisions: [WNoteLine(id: "d", text: "x", source: src)], openQuestions: [edited])
        #expect(model.sources.map(\.number) == [1])
        #expect(model.hasAILines)
        #expect(model.source(id: src.id)?.segmentId == "s")
    }

    @Test func aNoteWithNoSourcesShowsOnlyTheRegenerateLine() {
        let model = WNoteModel(title: "t", decisions: [WNoteLine(id: "d", text: "x")])
        #expect(model.showsMargin)
        #expect(model.showsSourcesHint)
        #expect(!model.showsMarginIntro)
    }

    @Test func aNoteWithSourcesShowsTheIntroAndNotTheRegenerateLine() {
        let src = WNoteSource(number: 1, segmentId: "s", speaker: "Jo", time: "1:00", quote: "q", lineText: "x")
        let model = WNoteModel(title: "t", decisions: [WNoteLine(id: "d", text: "x", source: src)])
        #expect(model.showsMarginIntro)
        #expect(!model.showsSourcesHint)
    }

    @Test func aNoteWithNoAILinesHasNoMarginAtAll() {
        let model = WNoteModel(title: "t", decisions: [WNoteLine(id: "d", text: "mine", isAI: false)])
        #expect(!model.showsMargin)
        #expect(!model.showsMarginIntro)
        #expect(!model.showsSourcesHint)
    }

    @Test func editIsOfferedWheneverTheMeetingIsFinishedWhatEverTheState() {
        var model = WNoteModel(title: "t")
        #expect(model.offersEdit)
        model.state = .processing(stage: "Transcribing")
        #expect(model.offersEdit)
        model.state = .failed("x")
        #expect(model.offersEdit)
        model.state = .noSummary("none")
        #expect(model.offersEdit)
        model.canEdit = false
        #expect(!model.offersEdit)
    }

    @Test func aSourceKeepsItsIdWhenTheNumbersShift() {
        let before = WNoteSource(number: 3, segmentId: "s9", speaker: "Jo", time: "1:00", quote: "q", lineText: "A decision.")
        var after = before
        after.number = 1
        #expect(before.id == after.id)
        let other = WNoteSource(number: 3, segmentId: "s9", speaker: "Jo", time: "1:00", quote: "q", lineText: "A different decision.")
        #expect(before.id != other.id)
        var model = WNoteModel(title: "t", decisions: [WNoteLine(id: "x", text: "A decision.", source: after)])
        model.selectedSource = before.id
        #expect(model.selectedSourceIndex == 0)
        model.selectedSource = other.id
        #expect(model.selectedSourceIndex == nil)
    }

    @Test func theReadingSizeSettingScalesInProportionToItsDefault() {
        #expect(WReadingScale.scale(fontSize: 13) == 1)
        #expect(abs(WReadingScale.scale(fontSize: 20) - 20.0 / 13.0) < 0.0001)
        #expect(abs(WReadingScale.scale(fontSize: 11) - 11.0 / 13.0) < 0.0001)
        #expect(WReadingScale.scale(fontSize: 40) == WReadingScale.scale(fontSize: 20))
        #expect(WReadingScale.scale(fontSize: 2) == WReadingScale.scale(fontSize: 11))
        #expect(WReadingScale.scale(fontSize: .nan) == 1)
        #expect(WNoteModel(title: "t").readingScale == 1)
    }

    @Test func aTranscriptLineKeepsItsOwnIdAndTheStringsAreNamedForAScreenReader() {
        let strings = WStrings.english
        #expect(strings.joinDetails([strings.sourceNumber(3), "Jo", "18:25"]) == "Source 3, Jo, 18:25")
        #expect(strings.note.openInTranscriptAt("18:25") == "Open in transcript at 18:25")
    }
}
