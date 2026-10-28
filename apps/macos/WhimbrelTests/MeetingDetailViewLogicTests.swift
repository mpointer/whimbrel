import XCTest
@testable import Whimbrel

final class MeetingDetailViewLogicTests: XCTestCase {
    // MARK: - nextActionItemOrderIndex

    func testNextOrderIndexIsZeroWhenListIsEmpty() {
        XCTAssertEqual(nextActionItemOrderIndex(existing: []), 0)
    }

    func testNextOrderIndexAfterMidListDeleteDoesNotCollideWithSurvivor() {
        // [0, 1, 2] -> delete the row at order_index 1 -> survivors are [0, 2].
        // `.count` (2) would collide with the surviving order_index-2 row; the
        // max-based fix must not.
        let survivors = [
            CoreMeetingActionItem(id: 1, meetingId: 1, text: "a", completed: false, orderIndex: 0, source: "generated", createdUnixMs: 0, updatedUnixMs: 0),
            CoreMeetingActionItem(id: 3, meetingId: 1, text: "c", completed: false, orderIndex: 2, source: "generated", createdUnixMs: 0, updatedUnixMs: 0),
        ]
        XCTAssertEqual(nextActionItemOrderIndex(existing: survivors), 3)
    }

    // MARK: - mergeUnsavedEditsIntoDraft

    func testNoLocalEditAndNoPersistedDraftStaysNil() {
        let merged = mergeUnsavedEditsIntoDraft(
            persisted: nil, meetingId: 1,
            localSummary: "Generated tldr", generatedSummary: "Generated tldr",
            localDecisions: ["d1"], generatedDecisions: ["d1"],
            localOpenQuestions: [], generatedOpenQuestions: []
        )
        XCTAssertNil(merged)
    }

    func testUnsavedLocalEditIsSurfacedEvenWithNoPersistedDraft() {
        // The debounce-window race: user typed an edit, autosave hasn't landed yet, so
        // `persisted` is still nil -- the merge must still report a structured override.
        let merged = mergeUnsavedEditsIntoDraft(
            persisted: nil, meetingId: 1,
            localSummary: "My edited tldr", generatedSummary: "Generated tldr",
            localDecisions: ["d1"], generatedDecisions: ["d1"],
            localOpenQuestions: [], generatedOpenQuestions: []
        )
        XCTAssertEqual(merged?.summaryEdited, "My edited tldr")
        XCTAssertNil(merged?.decisionsEdited)
    }

    func testPersistedOverrideIsPreservedWhenLocalMatchesGenerated() {
        let persisted = CoreMeetingNoteDraft(
            meetingId: 1, summaryEdited: "Saved edit", decisionsEdited: nil, openQuestionsEdited: nil,
            personalNotes: nil, lastEditedUnixMs: 1000, lastRegeneratedSeenUnixMs: nil
        )
        let merged = mergeUnsavedEditsIntoDraft(
            persisted: persisted, meetingId: 1,
            localSummary: "Generated tldr", generatedSummary: "Generated tldr",
            localDecisions: [], generatedDecisions: [],
            localOpenQuestions: [], generatedOpenQuestions: []
        )
        XCTAssertEqual(merged?.summaryEdited, "Saved edit")
    }

    /// End-to-end regression for the reported bug: a fast "type an edit, immediately
    /// click Regenerate" sequence, with no persisted draft yet, must still trip
    /// `regenerationNeedsConflictPrompt` once the merge is applied.
    func testDebounceWindowRaceIsClosed() {
        let merged = mergeUnsavedEditsIntoDraft(
            persisted: nil, meetingId: 1,
            localSummary: "My edited tldr", generatedSummary: "Generated tldr",
            localDecisions: [], generatedDecisions: [],
            localOpenQuestions: [], generatedOpenQuestions: []
        )
        XCTAssertTrue(regenerationNeedsConflictPrompt(existingDraft: merged, newGenerationUnixMs: 5000))
    }

    // MARK: - actionItemsHaveUserInteraction

    private func makeItem(source: String = "generated", completed: Bool = false, created: Int64 = 1000, updated: Int64 = 1000) -> CoreMeetingActionItem {
        CoreMeetingActionItem(id: 1, meetingId: 1, text: "a", completed: completed, orderIndex: 0, source: source, createdUnixMs: created, updatedUnixMs: updated)
    }

    func testFreshlySeededGeneratedItemsHaveNoInteraction() {
        XCTAssertFalse(actionItemsHaveUserInteraction(existing: [makeItem(), makeItem(created: 2000, updated: 2000)]))
    }

    func testEmptyListHasNoInteraction() {
        XCTAssertFalse(actionItemsHaveUserInteraction(existing: []))
    }

    func testUserAddedItemCountsAsInteraction() {
        XCTAssertTrue(actionItemsHaveUserInteraction(existing: [makeItem(source: "user")]))
    }

    func testCompletedGeneratedItemCountsAsInteraction() {
        XCTAssertTrue(actionItemsHaveUserInteraction(existing: [makeItem(completed: true)]))
    }

    func testTouchedGeneratedItemCountsAsInteraction() {
        // `updatedUnixMs != createdUnixMs` is the proxy for "edited or reordered after
        // seeding" -- no per-item "edited" flag exists in the schema.
        XCTAssertTrue(actionItemsHaveUserInteraction(existing: [makeItem(created: 1000, updated: 5000)]))
    }

    // MARK: - rejoined dividers

    private func part(_ i: Int64, _ startMs: Int64, transcript: String? = nil) -> CoreMeetingPart {
        CoreMeetingPart(partIndex: i, startUnixMs: startMs, endUnixMs: nil, audioPath: nil, transcriptPath: transcript, transcribed: transcript != nil)
    }

    private func hm(_ ms: Int64) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "HH:mm"
        return f.string(from: Date(timeIntervalSince1970: Double(ms) / 1000))
    }

    func testDividersSitAtEachLaterPartsOffset() {
        let parts = [part(1, 1_000), part(2, 61_000), part(3, 121_000)]
        let d = MeetingDetailLogic.dividers(parts: parts)
        XCTAssertEqual(d.map(\.beforeStartCs), [6_000, 12_000])
        XCTAssertEqual(d.map(\.label), ["\u{2014} rejoined \(hm(61_000)) \u{2014}", "\u{2014} rejoined \(hm(121_000)) \u{2014}"])
    }

    func testSinglePartHasNoDividers() {
        XCTAssertTrue(MeetingDetailLogic.dividers(parts: [part(1, 1_000)]).isEmpty)
    }

    func testDividerLabelsLandBeforeFirstSegmentAtOrPastTheOffset() {
        let d = [(beforeStartCs: Int64(6_000), label: "A"), (beforeStartCs: Int64(12_000), label: "B")]
        let labels = MeetingDetailLogic.dividerLabels(segmentStartCs: [100, 5_999, 6_000, 6_500, 13_000], dividers: d)
        XCTAssertEqual(labels, [2: "A", 4: "B"])
    }

    func testDividerWithNoLaterSegmentIsOmittedAndSimultaneousOnesKeepTheLatest() {
        let d = [(beforeStartCs: Int64(6_000), label: "A"), (beforeStartCs: Int64(7_000), label: "B"), (beforeStartCs: Int64(99_000), label: "C")]
        let labels = MeetingDetailLogic.dividerLabels(segmentStartCs: [100, 8_000], dividers: d)
        XCTAssertEqual(labels, [1: "B"])
    }

    func testJoinedTranscriptPutsADividerBetweenPartFiles() {
        let parts = [part(1, 1_000, transcript: "/a"), part(2, 61_000, transcript: "/b")]
        let text = MeetingDetailLogic.joinedTranscript(parts: parts) { ["/a": "[00:00:01] Me: hi", "/b": "[00:01:01] Me: back"][$0] }
        XCTAssertEqual(text, "[00:00:01] Me: hi\n\u{2014} rejoined \(hm(61_000)) \u{2014}\n[00:01:01] Me: back")
    }

    func testJoinedTranscriptSkipsUnreadableParts() {
        let parts = [part(1, 1_000, transcript: "/a"), part(2, 61_000, transcript: "/missing")]
        XCTAssertEqual(MeetingDetailLogic.joinedTranscript(parts: parts) { $0 == "/a" ? "x" : nil }, "x")
        XCTAssertNil(MeetingDetailLogic.joinedTranscript(parts: parts) { _ in nil })
    }

    func testSplitAtDividersSeparatesBodies() {
        let text = "[00:00:01] Me: hi\n\u{2014} rejoined 10:05 \u{2014}\n[00:01:01] Me: back"
        let chunks = MeetingDetailLogic.splitAtDividers(text)
        XCTAssertEqual(chunks.map(\.label), [nil, "\u{2014} rejoined 10:05 \u{2014}"])
        XCTAssertEqual(chunks.map(\.body), ["[00:00:01] Me: hi", "[00:01:01] Me: back"])
    }
}
