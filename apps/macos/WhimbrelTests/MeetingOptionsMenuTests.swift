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

    func testKeepMediaTargetsThisDevicesOwnCopy() {
        var grouped = CoreMeeting.previewFixture(status: .completed) // id 1 on "d1"
        XCTAssertEqual(grouped.ownCopyId(deviceId: "d1"), 1, "the meeting itself is ours")
        XCTAssertNil(grouped.ownCopyId(deviceId: "mac"), "no copy here: hide the switch")
        grouped.alsoRecorded = [CoreMeetingCopy(id: 2, deviceId: "mac", title: "Standup")]
        XCTAssertEqual(grouped.ownCopyId(deviceId: "mac"), 2, "lead is a peer's copy; ours is in alsoRecorded")
    }

    func testMissingKeepMediaKeyDecodesAsFalse() throws {
        let json = #"{"id":1,"device_id":"d","start_unix_ms":1,"end_unix_ms":null,"title":"t","attendees":[],"trigger":"manual","audio_path":null,"status":"completed","transcript_path":null,"notes":null,"summary_tldr":null,"summary_decisions":[],"summary_action_items":[],"summary_open_questions":[],"summary_provider":null,"summary_model":null,"estimated_speaker_count":null}"#
        let m = try JSONDecoder().decode(CoreMeeting.self, from: Data(json.utf8))
        XCTAssertFalse(m.keepMedia)
    }

    func testSplitDisabledWhileAnyMeetingProcesses() {
        XCTAssertTrue(MeetingOptionsMenu.isSplitDisabled(processingMeetingId: 7))
        XCTAssertFalse(MeetingOptionsMenu.isSplitDisabled(processingMeetingId: nil))
    }
}
