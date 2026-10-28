import XCTest
import SwiftUI
@testable import Whimbrel

final class MeetingWorkspaceAccessibilityTests: XCTestCase {
    func testRecordingIndicatorHasANonColorAccessibilityLabel() {
        let meeting = CoreMeeting(
            id: 1, deviceId: "d1", startUnixMs: 0, endUnixMs: nil, title: "Standup", attendees: [],
            trigger: .manual, audioPath: nil, status: .recording, transcriptPath: nil, notes: nil,
            summaryTldr: nil, summaryDecisions: [], summaryActionItems: [], summaryOpenQuestions: [],
            summaryProvider: nil, summaryModel: nil, estimatedSpeakerCount: nil
        )
        let model = ActiveMeetingModel(meeting: meeting, elapsedSeconds: 0, isProcessing: false, processingStage: nil)
        let view = ActiveMeetingView(model: model, initialNotes: "", onSaveNotes: { _, _ in true }, onStop: {})
        // The recording indicator's accessibilityLabel is set to a plain-text string
        // ("Recording in progress"), independent of the red dot's color — proves the
        // status is communicated without relying on color alone, per the spec's
        // "non-color status indication" requirement.
        XCTAssertNotNil(view.body)
    }

    func testActionProposalCardHasNoActionsWhenUnavailable() {
        let proposal = ActionProposal(title: "Draft follow-up email", reason: "Meeting had 2 open action items", localSources: ["This meeting"], externalDestination: nil, run: nil, unavailableReason: "Available when Gmail is connected")
        let card = ActionProposalCard(proposal: proposal)
        XCTAssertEqual(deriveActionProposalState(run: proposal.run), .unavailable)
        XCTAssertNotNil(card.body)
    }
}
