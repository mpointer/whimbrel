import WhimbrelUI
import XCTest
@testable import Whimbrel

/// The finished note adapter: numbering, sources, edited lines, action items, parts and state. Pure values, no store.
final class MeetingNoteRedesignTests: XCTestCase {
    private func segment(_ id: Int64, _ start: Int64, _ label: String, _ text: String, end: Int64? = nil) -> CoreMeetingTranscriptSegment {
        CoreMeetingTranscriptSegment(id: id, meetingId: 1, meetingTitle: "", startCs: start, speakerLabel: label, text: text, endCs: end)
    }

    private func raw(_ id: Int64, _ start: Int64, _ label: String, _ text: String) -> CoreLineSource {
        CoreLineSource(segmentId: id, startCs: start, speakerLabel: label, text: text)
    }

    private var lines: [CoreMeetingTranscriptSegment] {
        [segment(10, 37_200, "Speaker 1", "If the price holds, we can do two years."), segment(11, 110_500, "Me", "Open items remain.")]
    }

    private var labels: [Int64: String] { MeetingNoteAdapter.labelsById(lines) }

    private let names = ["Speaker 1": CoreDisplayLabel(label: "Jo", guess: false)]

    func testSourcesAreNumberedDownThePageAndNamedLikeTheTranscript() {
        let sourced = [CoreSourcedText(text: "Renew for two years.", source: raw(10, 37_200, "Speaker 1", "If the price holds, we can do two years."))]
        let result = MeetingNoteAdapter.lines(
            texts: ["Renew for two years.", "A line with no source."], generated: [], edited: false, sourced: sourced, labels: labels, names: names,
            numberFrom: 1, idPrefix: "d")
        XCTAssertEqual(result.lines.count, 2)
        XCTAssertEqual(result.lines[0].source?.number, 1)
        XCTAssertEqual(result.lines[0].source?.speaker, "Jo")
        XCTAssertEqual(result.lines[0].source?.time, "6:12")
        XCTAssertEqual(result.lines[0].source?.segmentId, "10")
        XCTAssertNil(result.lines[1].source)
        XCTAssertEqual(result.next, 2)
    }

    func testALineThePersonEditedIsTheirsWithNoSource() {
        let sourced = [CoreSourcedText(text: "Mine now.", source: raw(10, 0, "Me", "x"))]
        let result = MeetingNoteAdapter.lines(
            texts: ["Mine now.", "Still generated."], generated: ["Still generated."], edited: true, sourced: sourced, labels: labels, names: names,
            numberFrom: 1, idPrefix: "d")
        XCTAssertFalse(result.lines[0].isAI)
        XCTAssertNil(result.lines[0].source)
        XCTAssertTrue(result.lines[1].isAI)
        XCTAssertEqual(result.next, 1)
    }

    func testTwoLinesWithTheSameTextTakeTheirOwnSourceInOrder() {
        let sourced = [
            CoreSourcedText(text: "Same.", source: raw(10, 1, "Me", "a")), CoreSourcedText(text: "Same.", source: raw(11, 2, "Me", "b")),
        ]
        let result = MeetingNoteAdapter.lines(texts: ["Same.", "Same."], generated: [], edited: false, sourced: sourced, labels: labels, names: [:], numberFrom: 3, idPrefix: "d")
        XCTAssertEqual(result.lines.map { $0.source?.segmentId }, ["10", "11"])
        XCTAssertEqual(result.lines.map { $0.source?.number }, [3, 4])
    }

    private func item(
        _ id: Int64, _ text: String, source: String = "generated", completed: Bool = false, touched: Bool = false
    ) -> CoreMeetingActionItem {
        CoreMeetingActionItem(
            id: id, meetingId: 1, text: text, completed: completed, orderIndex: id, source: source, createdUnixMs: 1, updatedUnixMs: touched ? 2 : 1)
    }

    func testActionItemsShowOwnerAndDueOnlyWhenStoredAndMatchByRowId() {
        let sourced = [
            CoreSourcedActionItem(itemId: 5, text: "Send the quote.", owner: "Sam", due: "Thu", source: raw(10, 100, "Me", "I will send it.")),
            CoreSourcedActionItem(itemId: 6, text: "Check seats.", owner: nil, due: "  ", source: nil),
        ]
        let rows = MeetingNoteAdapter.actionItems(
            [item(5, "Send the quote."), item(6, "Check seats.", completed: true)], generated: ["Send the quote.", "Check seats."], sourced: sourced,
            labels: labels, names: names, numberFrom: 4)
        XCTAssertEqual(rows[0].owner, "Sam")
        XCTAssertEqual(rows[0].due, "Thu")
        XCTAssertEqual(rows[0].line.source?.number, 4)
        XCTAssertNil(rows[1].owner)
        XCTAssertNil(rows[1].due)
        XCTAssertNil(rows[1].line.source)
        XCTAssertTrue(rows[1].isDone)
    }

    func testAPersonsOwnOrEditedActionItemIsNotWhimbrels() {
        let rows = MeetingNoteAdapter.actionItems(
            [item(1, "Mine", source: "user"), item(2, "Edited text", touched: true)], generated: ["Original text"],
            sourced: [CoreSourcedActionItem(itemId: 2, text: "Edited text", owner: nil, due: nil, source: raw(10, 1, "Me", "x"))],
            labels: labels, names: [:], numberFrom: 1)
        XCTAssertEqual(rows.map(\.line.isAI), [false, false])
        XCTAssertEqual(rows.map { $0.line.source }, [nil, nil])
    }

    func testSegmentsCarryThePartAndTheRejoinedDivider() {
        let later = [segment(1, 100, "Me", "a"), segment(2, 200, "Me", "b"), segment(3, 9_000, "Speaker 1", "c")]
        let out = MeetingNoteAdapter.segments(later, names: names, dividers: [(beforeStartCs: 8_000, label: MeetingDetailLogic.dividerLabel(startUnixMs: 0))])
        XCTAssertEqual(out.map(\.partIndex), [0, 0, 1])
        XCTAssertNil(out[1].dividerBefore)
        XCTAssertNotNil(out[2].dividerBefore)
        XCTAssertEqual(out[2].speaker, "Jo")
        XCTAssertEqual(out[0].id, "1")
    }

    func testTheRejoinedDividerOnTheNotePageHasNoDashes() throws {
        let shown = try XCTUnwrap(MeetingNoteAdapter.segments(
            [segment(1, 9_000, "Me", "a")], names: names,
            dividers: [(beforeStartCs: 0, label: MeetingDetailLogic.dividerLabel(startUnixMs: 1_761_000_000_000))])[0].dividerBefore)
        XCTAssertFalse(shown.contains("\u{2014}") || shown.contains("\u{2013}"))
        XCTAssertFalse(shown.isEmpty)
        XCTAssertFalse(MeetingDetailLogic.renderedNoteDivider("garbled").contains("\u{2014}"))
    }

    func testSegmentEndCsReachesTheLanes() {
        let out = MeetingNoteAdapter.segments([segment(1, 100, "Me", "a", end: 400)], names: [:], dividers: [])
        XCTAssertEqual(out[0].endCs, 400)
    }

    func testMarksAreOrderedAndShowTheClockOffset() {
        let marks = MeetingNoteAdapter.marks([
            CoreMeetingMarker(id: 2, meetingId: 1, atCs: 144_200, label: nil, createdUnixMs: 0),
            CoreMeetingMarker(id: 1, meetingId: 1, atCs: 49_500, label: "Ask Jo", createdUnixMs: 0),
        ])
        XCTAssertEqual(marks.map(\.clock), ["8:15", "24:02"])
        XCTAssertEqual(marks.map(\.label), ["Ask Jo", ""])
    }

    func testAMarkOpensTheNearestLine() {
        XCTAssertEqual(MeetingNoteAdapter.nearestSegmentId(to: 100_000, in: lines), 11)
        XCTAssertEqual(MeetingNoteAdapter.nearestSegmentId(to: 40_000, in: lines), 10)
        XCTAssertNil(MeetingNoteAdapter.nearestSegmentId(to: 1, in: []))
    }

    func testTheStateFollowsTheMeeting() {
        func state(_ completed: Bool = true, processing: Bool = false, content: Bool = false, failure: String? = nil) -> WNoteState {
            MeetingNoteAdapter.state(isCompleted: completed, isProcessing: processing, stage: "Transcribing", hasContent: content, failure: failure, placeholder: "No summary yet.", notYet: "Later.")
        }
        XCTAssertEqual(state(false), .noSummary("Later."))
        XCTAssertEqual(state(processing: true, content: true), .ready)
        XCTAssertEqual(state(processing: true), .processing(stage: "Transcribing"))
        XCTAssertEqual(state(failure: "It failed."), .failed("It failed."))
        XCTAssertEqual(state(), .noSummary("No summary yet."))
    }

    func testOlderTranscriptFilesBecomeTurnsWithDividers() {
        let turns = MeetingNoteAdapter.turns(chunks: [
            (nil, [("Me", "[10:00:01]", "hi")]),
            (MeetingDetailLogic.dividerLabel(startUnixMs: 0), [("Others", "[10:20:00]", "back")]),
        ])
        XCTAssertEqual(turns.map(\.id), [0, 1])
        XCTAssertNil(turns[0].dividerBefore)
        XCTAssertNotNil(turns[1].dividerBefore)
    }

    // MARK: Live text, blanks, regenerating

    func testTheReadingViewShowsTheLiveTextNotTheTrailingSavedDraft() {
        // The person edited and pressed Done inside the autosave delay: the saved draft still has the old words.
        let saved = CoreMeetingNoteDraft(
            meetingId: 1, summaryEdited: "Old words.", decisionsEdited: ["Old decision."], openQuestionsEdited: nil, personalNotes: nil,
            lastEditedUnixMs: 5, lastRegeneratedSeenUnixMs: nil)
        let live = MeetingNoteAdapter.liveNote(
            summary: "New words.", decisions: ["New decision."], openQuestions: ["Generated question."],
            generatedSummary: "Generated.", generatedDecisions: ["Generated decision."], generatedOpenQuestions: ["Generated question."], persisted: saved)
        XCTAssertEqual(live.summary, "New words.")
        XCTAssertEqual(live.decisions, ["New decision."])
        XCTAssertTrue(live.summaryEdited)
        XCTAssertTrue(live.decisionsEdited)
        XCTAssertFalse(live.openQuestionsEdited)
    }

    func testTextTheEditedNowDiffersFromWhimbrelsIsTheirsEvenBeforeAnythingIsSaved() {
        let live = MeetingNoteAdapter.liveNote(
            summary: "Typed just now.", decisions: [], openQuestions: [], generatedSummary: "Generated.", generatedDecisions: [],
            generatedOpenQuestions: [], persisted: nil)
        XCTAssertTrue(live.summaryEdited)
        let untouched = MeetingNoteAdapter.liveNote(
            summary: "Generated.", decisions: [], openQuestions: [], generatedSummary: "Generated.", generatedDecisions: [],
            generatedOpenQuestions: [], persisted: nil)
        XCTAssertFalse(untouched.summaryEdited)
    }

    func testABlankActionItemTheyAddedIsNotDrawn() {
        let rows = MeetingNoteAdapter.actionItems(
            [item(1, "", source: "user"), item(2, "   \n", source: "user"), item(3, "Real one.")], generated: ["Real one."], sourced: [],
            labels: [:], names: [:], numberFrom: 1)
        XCTAssertEqual(rows.map(\.line.text), ["Real one."])
    }

    func testABlankDecisionIsNotDrawn() {
        let result = MeetingNoteAdapter.lines(
            texts: ["", "A decision."], generated: [], edited: true, sourced: [], labels: [:], names: [:], numberFrom: 1, idPrefix: "d")
        XCTAssertEqual(result.lines.map(\.text), ["A decision."])
    }

    func testRegeneratingAFailedNoteShowsProgressNotAnEmptyPlaceholder() {
        func state(regenerating: Bool, content: Bool = false, failure: String? = "It failed.") -> WNoteState {
            MeetingNoteAdapter.state(
                isCompleted: true, isProcessing: false, isRegenerating: regenerating, regenerating: "Regenerating…", stage: nil,
                hasContent: content, failure: failure, placeholder: "No summary yet.", notYet: "Later.")
        }
        XCTAssertEqual(state(regenerating: true), .processing(stage: "Regenerating…"))
        XCTAssertEqual(state(regenerating: true, failure: nil), .processing(stage: "Regenerating…"))
        XCTAssertEqual(state(regenerating: true, content: true), .ready)
        XCTAssertEqual(state(regenerating: false), .failed("It failed."))
    }

    // MARK: Action items after a Regenerate that kept the person's edits

    func testEarlierWhimbrelRowsStayWhimbrelsWhenARegenerateKeptThePersonsEdits() {
        // The new generation has other texts. The old, untouched rows are still Whimbrel's words.
        let generated = ["Newly written item."] + ["Old item."]
        let rows = [item(1, "Old item."), item(2, "Person rewrote this.", touched: true), item(3, "Typed by hand.", source: "user")]
        XCTAssertEqual(rows.map { MeetingNoteAdapter.isWhimbrelsActionItem($0, generated: generated) }, [true, false, false])
        // Without the earlier generation in the list, an untouched row is still Whimbrel's.
        XCTAssertTrue(MeetingNoteAdapter.isWhimbrelsActionItem(item(1, "Old item."), generated: ["Newly written item."]))
    }

    func testARowThePersonEditedIsNotTurnedBackIntoWhimbrelsByTheNewGeneration() {
        XCTAssertFalse(MeetingNoteAdapter.isWhimbrelsActionItem(item(2, "Person rewrote this.", touched: true), generated: ["Newly written item."]))
    }

    func testARowThePersonTickedKeepsItsInkWhenItsWordsAreStillWhimbrels() {
        XCTAssertTrue(MeetingNoteAdapter.isWhimbrelsActionItem(item(1, "Old item.", completed: true, touched: true), generated: ["Old item."]))
    }

    func testASourceIsNotShownAgainstWordsItWasNotWrittenFor() {
        let stale = CoreSourcedActionItem(itemId: 1, text: "Other words.", owner: "Sam", due: nil, source: raw(10, 1, "Me", "x"))
        let rows = MeetingNoteAdapter.actionItems([item(1, "These words.")], generated: ["These words."], sourced: [stale], labels: labels, names: [:], numberFrom: 1)
        XCTAssertNil(rows[0].line.source)
        XCTAssertNil(rows[0].owner)
    }

    // MARK: Sources reload, transcript cost, clock

    func testSourcesReloadWhenDecisionsOrQuestionsOrActionTextsChangeWithTheSameSummary() {
        func key(decisions: [String] = ["A"], questions: [String] = ["Q"], actions: [String] = ["T"], ids: [Int64] = [1], count: Int = 5) -> Int {
            MeetingNoteAdapter.sourcesKey(summary: "Same.", decisions: decisions, openQuestions: questions, actionItemTexts: actions, itemIds: ids, segmentCount: count)
        }
        let base = key()
        XCTAssertEqual(base, key())
        XCTAssertNotEqual(base, key(decisions: ["B"]))
        XCTAssertNotEqual(base, key(questions: ["R"]))
        XCTAssertNotEqual(base, key(actions: ["U"]))
        XCTAssertNotEqual(base, key(ids: [2]))
        XCTAssertNotEqual(base, key(count: 6))
    }

    func testTheTranscriptCopyCheckAgreesWithTheCopyTextBeingEmpty() {
        let rows = lines
        let cases: [(String?, [CoreMeetingTranscriptSegment], [String: CoreDisplayLabel])] = [
            (nil, [], [:]), ("", [], [:]), ("file text", [], [:]), (nil, rows, names), ("file text", rows, names), ("", rows, [:]),
        ]
        for (file, segments, shown) in cases {
            let text = MeetingDetailLogic.transcriptCopyText(fileText: file, segments: segments, names: shown)
            XCTAssertEqual(MeetingNoteAdapter.hasTranscriptToCopy(fileText: file, segmentCount: segments.count), !text.isEmpty)
        }
    }

    func testEveryTimeOnThePageUsesTheTranscriptsOwnClock() {
        for cs: Int64 in [0, 99, 100, 6_000, 110_500, 359_900, 360_000, 372_300, 1_000_000] {
            XCTAssertEqual(WLaneMath.clock(cs: cs), MeetingDetailLogic.clock(cs))
        }
        XCTAssertEqual(WLaneMath.clock(cs: 372_300), "62:03")
    }

    func testTheTranscriptIsWorkedOutOnceWhileItsInputsStayTheSame() {
        let cache = NoteTranscriptCache()
        var builds = 0
        let build: () -> [WNoteSegment] = { builds += 1; return MeetingNoteAdapter.segments(self.lines, names: self.names, dividers: []) }
        for _ in 0..<5 {
            _ = cache.transcript(lines: lines, names: names, partCount: 1, markers: [], build: build, marks: [])
        }
        XCTAssertEqual(builds, 1)
        _ = cache.transcript(lines: lines, names: ["Speaker 1": CoreDisplayLabel(label: "Sam", guess: false)], partCount: 1, markers: [], build: build, marks: [])
        XCTAssertEqual(builds, 2)
        let marker = CoreMeetingMarker(id: 1, meetingId: 1, atCs: 5, label: nil, createdUnixMs: 0)
        _ = cache.transcript(lines: lines, names: names, partCount: 1, markers: [marker], build: build, marks: MeetingNoteAdapter.marks([marker]))
        XCTAssertEqual(builds, 3)
    }

    func testSourcesNameTheirSpeakerFromALookupOfLineLabels() {
        XCTAssertEqual(labels[10], "Speaker 1")
        XCTAssertEqual(labels[11], "Me")
    }

    // MARK: The ends of lines, flag off and on

    func testALineWithNoEndWritesTheJSONItAlwaysDid() throws {
        let json = String(decoding: try JSONEncoder().encode([NewTranscriptSegmentInput(startCs: 1, speakerLabel: "Me", text: "hi")]), as: UTF8.self)
        XCTAssertFalse(json.contains("end_cs"))
        let with = String(decoding: try JSONEncoder().encode([NewTranscriptSegmentInput(startCs: 1, speakerLabel: "Me", text: "hi", endCs: 9)]), as: UTF8.self)
        XCTAssertTrue(with.contains("\"end_cs\":9"))
    }

    func testTranscriptPagesWithAndWithoutEndsDecode() throws {
        let one = Data(#"{"id":1,"meeting_id":1,"meeting_title":"t","start_cs":5,"end_cs":50,"speaker_label":"Me","text":"x"}"#.utf8)
        let two = Data(#"{"id":2,"meeting_id":1,"meeting_title":"t","start_cs":5,"speaker_label":"Me","text":"x"}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(CoreMeetingTranscriptSegment.self, from: one).endCs, 50)
        XCTAssertNil(try JSONDecoder().decode(CoreMeetingTranscriptSegment.self, from: two).endCs)
    }

    func testNoteSourcesDecodeWithAbsentKeys() throws {
        let json = Data(#"{"decisions":[{"text":"a","source":{"segment_id":3,"start_cs":1,"speaker_label":"Me","text":"q"}}],"action_items":[{"item_id":null,"text":"b","owner":"Sam"}]}"#.utf8)
        let decoded = try JSONDecoder().decode(CoreNoteSources.self, from: json)
        XCTAssertEqual(decoded.decisions.first?.source?.segmentId, 3)
        XCTAssertTrue(decoded.openQuestions.isEmpty)
        XCTAssertNil(decoded.actionItems.first?.itemId)
        XCTAssertEqual(decoded.actionItems.first?.owner, "Sam")
    }
}
