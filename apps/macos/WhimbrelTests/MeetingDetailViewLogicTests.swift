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

    // MARK: - Localized display (stored text stays English)

    func test_rejoined_divider_still_splits_when_rendered_localized() {
        let label = MeetingDetailLogic.dividerLabel(startUnixMs: 1_761_000_000_000)
        let text = "a\n\(label)\nb"
        let chunks = MeetingDetailLogic.splitAtDividers(text)
        XCTAssertEqual(chunks.map(\.label), [nil, label])
        XCTAssertEqual(chunks.map(\.body), ["a", "b"])
        XCTAssertTrue(MeetingDetailLogic.renderedDivider(label).contains("rejoined"))
        XCTAssertTrue(MeetingDetailLogic.renderedDivider(label).hasPrefix("\u{2014} "))
    }

    func test_speakerLabel_maps_for_display_only() {
        let stored = "[10:00:00] Speaker 2: hi"
        XCTAssertEqual(MeetingDetailLogic.displaySpeaker("Speaker 2"), "Speaker 2")
        XCTAssertEqual(MeetingDetailLogic.displaySpeaker("Me"), "Me")
        XCTAssertEqual(MeetingDetailLogic.displaySpeaker("Alice"), "Alice")
        XCTAssertEqual(stored, "[10:00:00] Speaker 2: hi")
    }

    // MARK: - Speaker names

    func testDisplaySpeakerShowsGuessMark() {
        XCTAssertEqual(MeetingDetailLogic.displaySpeaker("Sam", guess: true), "Sam?")
        XCTAssertEqual(MeetingDetailLogic.displaySpeaker("Sam", guess: false), "Sam")
        XCTAssertEqual(MeetingDetailLogic.displaySpeaker("Speaker 2", guess: false), "Speaker 2")
    }

    func testSpeakerIdIsTheLabelNumberMinusOne() {
        XCTAssertEqual(MeetingDetailLogic.speakerId(rawLabel: "Speaker 1"), 0)
        XCTAssertEqual(MeetingDetailLogic.speakerId(rawLabel: "Speaker 10"), 9)
        XCTAssertNil(MeetingDetailLogic.speakerId(rawLabel: "Speaker 0"))
        XCTAssertNil(MeetingDetailLogic.speakerId(rawLabel: "Me"))
        XCTAssertNil(MeetingDetailLogic.speakerId(rawLabel: "Others"))
    }

    func testMenuOnlyForSpeakerLabelsInSinglePartMeetings() {
        XCTAssertTrue(MeetingDetailLogic.speakerMenuOffered(rawLabel: "Speaker 1", partCount: 1))
        XCTAssertTrue(MeetingDetailLogic.speakerMenuOffered(rawLabel: "Speaker 1", partCount: 0))
        XCTAssertFalse(MeetingDetailLogic.speakerMenuOffered(rawLabel: "Speaker 1", partCount: 2))
        XCTAssertFalse(MeetingDetailLogic.speakerMenuOffered(rawLabel: "Me", partCount: 1))
        XCTAssertFalse(MeetingDetailLogic.speakerMenuOffered(rawLabel: "Others", partCount: 1))
    }

    func testMenuListsInviteesPlusSomeoneElse() {
        let ana = CoreAttendee(name: "Ana", email: "ana@x.com"), bo = CoreAttendee(name: "Bo", email: nil)
        let items = MeetingDetailLogic.speakerMenuItems(shown: CoreDisplayLabel(label: "Speaker 1", guess: false), invitees: [ana, bo])
        XCTAssertEqual(items, [.invitee(ana), .invitee(bo), .someoneElse])
    }

    func testConfirmItemOnlyForGuess() {
        let samA = CoreAttendee(name: "Sam", email: "sam@a.com")
        let samB = CoreAttendee(name: "Sam", email: "sam@b.com")
        // Two invitees named Sam: the guess confirms the Sam it is (B), not the first one listed.
        let guessed = MeetingDetailLogic.speakerMenuItems(
            shown: CoreDisplayLabel(label: "Sam", guess: true, personKey: "sam@b.com"), invitees: [samA, samB])
        XCTAssertEqual(guessed, [.confirm(name: "Sam", personKey: "sam@b.com"), .invitee(samA), .invitee(samB), .someoneElse])
        let confirmed = MeetingDetailLogic.speakerMenuItems(
            shown: CoreDisplayLabel(label: "Sam", guess: false, personKey: "sam@b.com"), invitees: [samA])
        XCTAssertEqual(confirmed, [.invitee(samA), .someoneElse])
        // No person behind the label: nothing to confirm.
        let unnamed = MeetingDetailLogic.speakerMenuItems(shown: CoreDisplayLabel(label: "Lee", guess: true), invitees: [samA])
        XCTAssertEqual(unnamed, [.invitee(samA), .someoneElse])
    }

    func testDisplayLabelDecodesPersonKey() throws {
        let json = Data(#"[{"label":"Sam","guess":true,"person_key":"sam@b.com"},{"label":"Me","guess":false,"person_key":null}]"#.utf8)
        let labels = try JSONDecoder().decode([CoreDisplayLabel].self, from: json)
        XCTAssertEqual(labels, [CoreDisplayLabel(label: "Sam", guess: true, personKey: "sam@b.com"), CoreDisplayLabel(label: "Me", guess: false)])
    }

    func testCopyTextMarksGuesses() {
        let segments = [
            CoreMeetingTranscriptSegment(id: 1, meetingId: 1, meetingTitle: "", startCs: 500, speakerLabel: "Speaker 1", text: "hi"),
            CoreMeetingTranscriptSegment(id: 2, meetingId: 1, meetingTitle: "", startCs: 6_100, speakerLabel: "Speaker 2", text: "hey"),
            CoreMeetingTranscriptSegment(id: 3, meetingId: 1, meetingTitle: "", startCs: 7_000, speakerLabel: "Me", text: "yo"),
        ]
        let names = [
            "Speaker 1": CoreDisplayLabel(label: "Sam", guess: true),
            "Speaker 2": CoreDisplayLabel(label: "Ana", guess: false),
            "Me": CoreDisplayLabel(label: "Me", guess: false),
        ]
        let expected = "Sam? (0:05): hi\nAna (1:01): hey\nMe (1:10): yo"
        XCTAssertEqual(MeetingDetailLogic.transcriptCopyText(fileText: "[00:00:05] Speaker 1: hi", segments: segments, names: names), expected)
        // No names on the page: the transcript file, as before.
        XCTAssertEqual(MeetingDetailLogic.transcriptCopyText(fileText: "file", segments: segments, names: [:]), "file")
        XCTAssertEqual(
            MeetingDetailLogic.transcriptCopyText(fileText: nil, segments: Array(segments.prefix(1)), names: [:]),
            "Speaker 1 (0:05): hi")
    }

    func test_placeholder_title_renders_verbatim() {
        XCTAssertEqual(MeetingDetailLogic.displayTitle("Meeting"), "Meeting")
    }
}
