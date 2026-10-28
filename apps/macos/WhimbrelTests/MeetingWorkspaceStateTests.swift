import XCTest
@testable import Whimbrel

/// Smoke tests for `deriveMeetingWorkspaceState`, now a wrapper over
/// core/api::meeting_workspace::derive_state; the state table's cases live in Rust
/// (`core/api/src/meeting_workspace.rs`). These pin the tag decoding of every state and
/// the payloads the views read.
final class MeetingWorkspaceStateTests: XCTestCase {
    private func makeMeeting(id: Int64 = 1, status: CoreMeetingStatus, summaryTldr: String? = nil) -> CoreMeeting {
        CoreMeeting(
            id: id, deviceId: "d1", startUnixMs: 1000, endUnixMs: status == .completed ? 2000 : nil,
            title: "Standup", attendees: ["a@x.com"], trigger: .manual, audioPath: nil, status: status,
            transcriptPath: nil, notes: nil, summaryTldr: summaryTldr, summaryDecisions: [],
            summaryActionItems: [], summaryOpenQuestions: [], summaryProvider: nil, summaryModel: nil,
            estimatedSpeakerCount: nil
        )
    }

    func testScheduledMeetingProducesBeforeState() {
        let state = deriveMeetingWorkspaceState(
            meeting: makeMeeting(status: .scheduled), activeMeetingId: nil, processingMeetingId: nil,
            processingStage: nil, draft: nil, actionItems: []
        )
        guard case .before = state else { return XCTFail("expected .before, got \(state)") }
    }

    func testRecordingMeetingWithMatchingActiveIdProducesDuringState() {
        let state = deriveMeetingWorkspaceState(
            meeting: makeMeeting(id: 42, status: .recording), activeMeetingId: 42, processingMeetingId: nil,
            processingStage: nil, draft: nil, actionItems: []
        )
        guard case .during = state else { return XCTFail("expected .during, got \(state)") }
    }

    func testRecordingMeetingWithMismatchedActiveIdFallsBackToBefore() {
        let state = deriveMeetingWorkspaceState(
            meeting: makeMeeting(id: 42, status: .recording), activeMeetingId: 99, processingMeetingId: nil,
            processingStage: nil, draft: nil, actionItems: []
        )
        guard case .before = state else { return XCTFail("expected .before, got \(state)") }
    }

    func testCompletedMeetingStillProcessingProducesAfterStateWithProcessingStage() {
        let state = deriveMeetingWorkspaceState(
            meeting: makeMeeting(id: 7, status: .completed, summaryTldr: nil), activeMeetingId: nil,
            processingMeetingId: 7, processingStage: "Summarizing…", draft: nil, actionItems: []
        )
        guard case let .after(model) = state else { return XCTFail("expected .after, got \(state)") }
        XCTAssertEqual(model.processingStage, "Summarizing…")
    }

    func testCancelledMeetingProducesItsOwnCancelledCase() {
        let state = deriveMeetingWorkspaceState(
            meeting: makeMeeting(status: .cancelled), activeMeetingId: nil, processingMeetingId: nil,
            processingStage: nil, draft: nil, actionItems: []
        )
        guard case .cancelled = state else { return XCTFail("expected .cancelled, got \(state)") }
    }

    func testAfterStatePrefersDraftSummaryOverGenerated() {
        let draft = CoreMeetingNoteDraft(
            meetingId: 1, summaryEdited: "My edited summary", decisionsEdited: nil,
            openQuestionsEdited: nil, personalNotes: nil, lastEditedUnixMs: 5000, lastRegeneratedSeenUnixMs: nil
        )
        let state = deriveMeetingWorkspaceState(
            meeting: makeMeeting(status: .completed, summaryTldr: "Generated summary"), activeMeetingId: nil,
            processingMeetingId: nil, processingStage: nil, draft: draft, actionItems: []
        )
        guard case let .after(model) = state else { return XCTFail("expected .after, got \(state)") }
        XCTAssertEqual(model.displaySummary, "My edited summary")
        XCTAssertEqual(model.generatedSummary, "Generated summary")
        XCTAssertTrue(model.summaryIsUserEdited)
    }

    func testDuringStateElapsedSecondsIsNonNegative() {
        let meeting = makeMeeting(id: 5, status: .recording)
        let state = deriveMeetingWorkspaceState(
            meeting: meeting, activeMeetingId: 5, processingMeetingId: nil, processingStage: nil,
            draft: nil, actionItems: [], nowUnixMs: meeting.startUnixMs + 60_000
        )
        guard case let .during(model) = state else { return XCTFail("expected .during, got \(state)") }
        XCTAssertEqual(model.elapsedSeconds, 60)
    }
}
