import XCTest
@testable import Whimbrel

final class RegenerationConflictTests: XCTestCase {
    func testNoDraftMeansNoConflictPrompt() {
        XCTAssertFalse(regenerationNeedsConflictPrompt(existingDraft: nil, newGenerationUnixMs: 5000))
    }

    func testDraftNeverReconciledMeansConflictPrompt() {
        let draft = CoreMeetingNoteDraft(
            meetingId: 1, summaryEdited: "Edited", decisionsEdited: nil, openQuestionsEdited: nil,
            personalNotes: nil, lastEditedUnixMs: 1000, lastRegeneratedSeenUnixMs: nil
        )
        XCTAssertTrue(regenerationNeedsConflictPrompt(existingDraft: draft, newGenerationUnixMs: 5000))
    }

    func testAlreadyReconciledGenerationMeansNoPrompt() {
        let draft = CoreMeetingNoteDraft(
            meetingId: 1, summaryEdited: "Edited", decisionsEdited: nil, openQuestionsEdited: nil,
            personalNotes: nil, lastEditedUnixMs: 1000, lastRegeneratedSeenUnixMs: 5000
        )
        XCTAssertFalse(regenerationNeedsConflictPrompt(existingDraft: draft, newGenerationUnixMs: 5000))
    }

    func testDraftWithNoStructuredOverridesNeedsNoPrompt() {
        // Only personal_notes set -- nothing to conflict with a regenerated summary.
        let draft = CoreMeetingNoteDraft(
            meetingId: 1, summaryEdited: nil, decisionsEdited: nil, openQuestionsEdited: nil,
            personalNotes: "just my own notes", lastEditedUnixMs: 1000, lastRegeneratedSeenUnixMs: nil
        )
        XCTAssertFalse(regenerationNeedsConflictPrompt(existingDraft: draft, newGenerationUnixMs: 5000))
    }
}
