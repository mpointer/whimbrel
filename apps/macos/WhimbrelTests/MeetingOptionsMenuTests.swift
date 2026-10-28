import XCTest
import SwiftUI
@testable import Whimbrel

private extension CoreMeeting {
    /// Minimal test-only fixture — no `previewFixture`/`PreviewData` helper already
    /// exists for `CoreMeeting` in this target (checked via grep before adding this).
    static func previewFixture(status: CoreMeetingStatus) -> CoreMeeting {
        CoreMeeting(
            id: 1, deviceId: "d1", startUnixMs: 1000, endUnixMs: status == .completed ? 2000 : nil,
            title: "Standup", attendees: ["a@x.com"], trigger: .manual, audioPath: nil, status: status,
            transcriptPath: nil, notes: nil, summaryTldr: nil, summaryDecisions: [],
            summaryActionItems: [], summaryOpenQuestions: [], summaryProvider: nil, summaryModel: nil,
            estimatedSpeakerCount: nil
        )
    }
}

final class MeetingOptionsMenuTests: XCTestCase {
    func testForgetIsDisabledWhileRecording() {
        let recording = CoreMeeting.previewFixture(status: .recording)
        XCTAssertTrue(MeetingOptionsMenu.isForgetDisabled(for: recording))
    }

    func testForgetIsEnabledWhenNotRecording() {
        let completed = CoreMeeting.previewFixture(status: .completed)
        XCTAssertFalse(MeetingOptionsMenu.isForgetDisabled(for: completed))
    }

    func testSplitOfferedOnlyForCompletedMeetingWithTwoOrMoreParts() {
        let done = CoreMeeting.previewFixture(status: .completed)
        XCTAssertTrue(MeetingOptionsMenu.canOfferSplit(meeting: done, partCount: 2))
        XCTAssertTrue(MeetingOptionsMenu.canOfferSplit(meeting: done, partCount: 3))
        XCTAssertFalse(MeetingOptionsMenu.canOfferSplit(meeting: done, partCount: 1))
        XCTAssertFalse(MeetingOptionsMenu.canOfferSplit(meeting: CoreMeeting.previewFixture(status: .recording), partCount: 2))
    }

    func testSplitDisabledWhileAnyMeetingProcesses() {
        XCTAssertTrue(MeetingOptionsMenu.isSplitDisabled(processingMeetingId: 7))
        XCTAssertFalse(MeetingOptionsMenu.isSplitDisabled(processingMeetingId: nil))
    }
}
