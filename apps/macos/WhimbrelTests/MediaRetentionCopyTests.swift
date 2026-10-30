import XCTest
@testable import Whimbrel

final class MediaRetentionCopyTests: XCTestCase {
    private let noUsage = CoreMediaUsage(audioFullBytes: 0, audioCompressedBytes: 0, imageBytes: 0)
    private let deleteOff = CoreMediaPolicy(compressAfterDays: 30, deleteAudioAfterDays: nil, shrinkImagesAfterDays: 30)
    private let deleteOn = CoreMediaPolicy(compressAfterDays: 30, deleteAudioAfterDays: 90, shrinkImagesAfterDays: 30)
    private let pace = " Up to 10 meetings per run; runs at launch, daily and after each meeting. Includes existing recordings."

    private func work(_ id: Int64, _ steps: [CoreMediaStep]) -> CoreMeetingWork {
        CoreMeetingWork(meetingId: id, steps: steps, audioPaths: [], shrinkSnapshotIds: [])
    }

    func testUsageLine() {
        let u = CoreMediaUsage(audioFullBytes: 1_200_000_000, audioCompressedBytes: 85_000_000, imageBytes: 340_000_000)
        XCTAssertEqual(MediaRetentionCopy.usage(u), "Audio: 1.2 GB full quality, 85 MB compressed. Images: 340 MB.")
    }

    func testPreviewCountsMeetingsPerStepAndInAll() {
        let plan = CoreMediaPlan(work: [
            work(1, [.moveAudio, .moveSnapshots, .compress]),
            work(2, [.compress, .shrinkImages]),
            work(3, [.compress]),
            work(4, [.moveSnapshots, .deleteAudio, .shrinkImages]),
            work(5, [.moveAudio]),
            work(6, [.moveAudio]),
        ], disconnectedMeetings: 0, usage: noUsage)
        XCTAssertEqual(MediaRetentionCopy.preview(plan, policy: deleteOn),
                       "Due now: compress 3 meetings, delete audio of 1, shrink images of 2, move 4 (6 meetings in all)." + pace
                       + " Audio of transcribed meetings older than 90 days is deleted permanently.")
    }

    func testPreviewDropsZeroClauses() {
        let plan = CoreMediaPlan(work: [work(1, [.deleteAudio]), work(2, [.moveAudio])], disconnectedMeetings: 0, usage: noUsage)
        XCTAssertEqual(MediaRetentionCopy.preview(plan, policy: deleteOn), "Due now: delete audio of 1, move 1 (2 meetings in all)." + pace
                       + " Audio of transcribed meetings older than 90 days is deleted permanently.")
        let one = CoreMediaPlan(work: [work(1, [.compress])], disconnectedMeetings: 0, usage: noUsage)
        XCTAssertEqual(MediaRetentionCopy.preview(one, policy: deleteOff), "Due now: compress 1 meeting (1 meeting in all)." + pace)
    }

    func testPreviewNamesDeletionWheneverItIsOnEvenWithNothingDue() {
        let empty = CoreMediaPlan(work: [], disconnectedMeetings: 0, usage: noUsage)
        XCTAssertEqual(MediaRetentionCopy.preview(empty, policy: deleteOff), "Nothing due now." + pace)
        XCTAssertEqual(MediaRetentionCopy.preview(empty, policy: deleteOn), "Nothing due now." + pace
                       + " Audio of transcribed meetings older than 90 days is deleted permanently.")
    }

    func testPreviewDisconnectedDrive() {
        let plan = CoreMediaPlan(work: [work(1, [.compress]), work(2, [.compress])], disconnectedMeetings: 3, usage: noUsage)
        XCTAssertEqual(MediaRetentionCopy.preview(plan, policy: deleteOff),
                       "Due now: compress 2 meetings (2 meetings in all)." + pace + " 3 meetings on a disconnected drive.")
        let none = CoreMediaPlan(work: [], disconnectedMeetings: 1, usage: noUsage)
        XCTAssertEqual(MediaRetentionCopy.preview(none, policy: deleteOff), "Nothing due now." + pace + " 1 meeting on a disconnected drive.")
    }

    func testDeleteSwitchNoteShowsWhatTurningItOnWouldFree() {
        let u = CoreMediaUsage(audioFullBytes: 0, audioCompressedBytes: 0, imageBytes: 0, audioDeletableBytes: 1_200_000_000)
        XCTAssertEqual(MediaRetentionCopy.deleteSwitchNote(u, deleteOn: false), "Turning this on would free 1.2 GB now.")
        XCTAssertEqual(MediaRetentionCopy.deleteSwitchNote(noUsage, deleteOn: false), "Turning this on would free nothing yet.")
        XCTAssertNil(MediaRetentionCopy.deleteSwitchNote(u, deleteOn: true))
    }

    func testProgressNamesTheCurrentStep() {
        XCTAssertEqual(MediaRetentionCopy.progress(step: .compress, done: 3, total: 10), "Compressing… 3 of 10")
        XCTAssertEqual(MediaRetentionCopy.progress(step: .shrinkImages, done: 0, total: 2), "Shrinking images… 0 of 2")
        XCTAssertEqual(MediaRetentionCopy.progress(step: .deleteAudio, done: 1, total: 2), "Deleting audio… 1 of 2")
        XCTAssertEqual(MediaRetentionCopy.progress(step: .moveAudio, done: 1, total: 2), "Moving files… 1 of 2")
        XCTAssertEqual(MediaRetentionCopy.progress(step: .moveSnapshots, done: 1, total: 2), "Moving files… 1 of 2")
        XCTAssertEqual(MediaRetentionCopy.progress(step: nil, done: 0, total: 4), "Working… 0 of 4")
    }
}
