import XCTest
import SwiftUI
@testable import Whimbrel

final class MeetingBriefViewTests: XCTestCase {
    func testMeetingBriefViewInstantiatesWithAScheduledMeeting() {
        let meeting = CoreMeeting(
            id: 1, deviceId: "d1", startUnixMs: 1_700_000_000_000, endUnixMs: nil, title: "Design review",
            attendees: ["a@x.com", "b@x.com"], trigger: .calendar, audioPath: nil, status: .scheduled,
            transcriptPath: nil, notes: nil, summaryTldr: nil, summaryDecisions: [], summaryActionItems: [],
            summaryOpenQuestions: [], summaryProvider: nil, summaryModel: nil, estimatedSpeakerCount: nil
        )
        let view = MeetingBriefView(model: MeetingBriefModel(meeting: meeting), relatedThreads: [], suggestedQuestions: [], onStart: {})
        XCTAssertNotNil(view.body)
    }
}
