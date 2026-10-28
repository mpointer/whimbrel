import XCTest
@testable import Whimbrel

final class AwayOutcomeLabelTests: XCTestCase {
    func test_all_labels() {
        let at = Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 20, hour: 10, minute: 42))!
        let expected: [(AwayOutcome, String)] = [
            (.attended, "Attended by Whimbrel"),
            (.chatNotPosted, "Attended by Whimbrel · chat message not posted"),
            (.removed, "Removed at 10:42 AM"),
            (.waitingRoom, "Couldn't get in: not admitted from the waiting room"),
            (.passcodeRequired, "Couldn't get in: needed a passcode"),
            (.signInRequired, "Couldn't get in: needed a sign-in"),
            (.notMuted, "Left: couldn't confirm mic and camera were off"),
            (.screenLocked, "Couldn't get in: this Mac was locked"),
            (.appFailed, "Couldn't get in: Zoom didn't open"),
            (.missedOverlap, "Missed: another meeting was on at the same time"),
        ]
        for (outcome, text) in expected { XCTAssertEqual(AwayOutcomeLabel.text(outcome, at: at), text) }
    }

    private func meeting(_ trigger: CoreMeetingTrigger, _ outcome: AwayOutcome?) -> CoreMeeting {
        CoreMeeting(
            id: 1, deviceId: "d", startUnixMs: 0, endUnixMs: nil, title: "t", attendees: [], trigger: trigger,
            audioPath: nil, status: .completed, transcriptPath: nil, notes: nil, summaryTldr: nil,
            summaryDecisions: [], summaryActionItems: [], summaryOpenQuestions: [], summaryProvider: nil,
            summaryModel: nil, estimatedSpeakerCount: nil, awayOutcome: outcome
        )
    }

    func test_effective_outcome() {
        XCTAssertEqual(meeting(.away, nil).effectiveAwayOutcome, .attended)
        XCTAssertEqual(meeting(.away, .removed).effectiveAwayOutcome, .removed)
        XCTAssertNil(meeting(.calendar, nil).effectiveAwayOutcome)
    }
}
