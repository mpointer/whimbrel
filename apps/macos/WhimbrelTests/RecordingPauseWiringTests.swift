import AVFoundation
import WhimbrelUI
import XCTest
@testable import Whimbrel

/// The app side of the recording pause (docs/ui-redesign/screens/live-meeting.md): the checks that are held, the
/// 6 hour cap that counts a pause, the stored paused spans and the level reading. No real audio runs here.
final class RecordingPauseWiringTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_800_000_000)

    func testTheSilenceRuleCannotEndAHeldRecordingHoweverLongItIsQuiet() {
        var state = MeetingEndDetector.PollState(silenceStartedAt: nil, browserTabGoneStartedAt: nil, captureReleasedStartedAt: nil)
        for minute in 1...30 {
            let reason = MeetingEndDetector.evaluate(
                now: start.addingTimeInterval(Double(minute) * 60), recordingStartedAt: start, appRunning: true, isOnMeetTab: nil,
                scheduledEndUnixMs: nil, audioActive: MeetingEndDetector.effectiveAudioActive(held: true, active: false), state: &state)
            XCTAssertNil(reason, "minute \(minute)")
        }
    }

    func testTheSilenceRuleStillFiresWhenNotHeld() {
        var state = MeetingEndDetector.PollState(silenceStartedAt: nil, browserTabGoneStartedAt: nil, captureReleasedStartedAt: nil)
        var ended = false
        for minute in 1...30 {
            let reason = MeetingEndDetector.evaluate(
                now: start.addingTimeInterval(Double(minute) * 60), recordingStartedAt: start, appRunning: true, isOnMeetTab: nil,
                scheduledEndUnixMs: nil, audioActive: MeetingEndDetector.effectiveAudioActive(held: false, active: false), state: &state)
            if reason == .silence { ended = true }
        }
        XCTAssertTrue(ended)
    }

    func testThePauseCountsTowardTheSixHourCap() {
        let cap = RecordingLimits.capSeconds
        XCTAssertFalse(AudioCaptureService.pausedPastCap(paused: true, startedAt: start, now: start.addingTimeInterval(cap - 60)))
        XCTAssertTrue(AudioCaptureService.pausedPastCap(paused: true, startedAt: start, now: start.addingTimeInterval(cap)))
        XCTAssertFalse(AudioCaptureService.pausedPastCap(paused: false, startedAt: start, now: start.addingTimeInterval(cap + 60)),
                       "a recording that is not paused is capped by its writer, as before")
    }

    @MainActor func testPauseAndResumeDoNothingBeforeARecordingStarts() async {
        let audio = AudioCaptureService()
        audio.pauseRecording()
        XCTAssertFalse(audio.isPaused)
        let outcome = await audio.resumeRecording()
        XCTAssertEqual(outcome, .notPaused)
        XCTAssertFalse(audio.micActive)
        XCTAssertNil(audio.degradedReason)
        XCTAssertNil(audio.micDegradedReason)
        XCTAssertEqual(audio.inputLevel, 0)
    }

    func testTheLevelReadingRecordsNothingUntilItIsOnAndLeavesTheSilenceMeterAlone() {
        let level = LevelPeak()
        let silence = PeakMeter()
        level.record(0.5); silence.record(0.5)
        XCTAssertEqual(level.take(), 0, "off: nothing is sampled")
        level.isEnabled = true
        level.record(0.25); level.record(0.5); silence.record(0.25)
        XCTAssertEqual(level.take(), 0.5)
        XCTAssertEqual(level.take(), 0, "taken once")
        XCTAssertEqual(silence.take(), 0.5, "the silence meter keeps its own reading")
        level.isEnabled = false
        level.record(0.9)
        XCTAssertEqual(level.take(), 0)
    }

    func testPausedSpansLiveBesideTheAudioAndDropTextInsideThem() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("pauses-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let mic = folder.appendingPathComponent("mic-audio-2026-11-18T10-00-00.000Z.caf").path
        let system = folder.appendingPathComponent("system-audio-2026-11-18T10-00-00.000Z.m4a").path
        let sidecar = try XCTUnwrap(RecordingPausedSpans.url(forAudioPath: mic, root: folder))
        XCTAssertEqual(sidecar, RecordingPausedSpans.url(forAudioPath: system, root: folder), "both tracks share one file")
        XCTAssertTrue(sidecar.lastPathComponent.hasPrefix(RecordingPausedSpans.filePrefix))
        XCTAssertNil(RecordingPausedSpans.url(forAudioPath: folder.appendingPathComponent("other.caf").path, root: folder))

        RecordingPausedSpans.write([RecordingPausedSpan(start: 100, end: 160), RecordingPausedSpan(start: 300)], forAudioPath: mic, root: folder)
        let back = RecordingPausedSpans.read(forAudioPath: system, root: folder)
        XCTAssertEqual(back, [RecordingPausedSpan(start: 100, end: 160), RecordingPausedSpan(start: 300, end: nil)])

        let lines = [
            TranscriptionService.Line(speakerLabel: "Me", startCs: 5_000, endCs: 5_400, text: "before"),
            TranscriptionService.Line(speakerLabel: "Others", startCs: 11_000, endCs: 11_300, text: "Thank you."),
            TranscriptionService.Line(speakerLabel: "Me", startCs: 20_000, endCs: 20_400, text: "after the pause"),
            TranscriptionService.Line(speakerLabel: "Others", startCs: 31_000, endCs: 31_300, text: "inside an open pause"),
        ]
        XCTAssertEqual(RecordingPausedSpans.dropping(lines, spans: back).map(\.text), ["before", "after the pause"])
        XCTAssertEqual(RecordingPausedSpans.dropping(lines, spans: []).count, 4)

        // Deleting the meeting's files takes the paused-stretches file with them; a missing file is no error.
        RecordingPausedSpans.remove(forAudioPath: mic, root: folder)
        XCTAssertFalse(FileManager.default.fileExists(atPath: sidecar.path))
        RecordingPausedSpans.remove(forAudioPath: mic, root: folder)
        XCTAssertTrue(RecordingPausedSpans.read(forAudioPath: mic, root: folder).isEmpty)
    }

    func testAPausedMeetingIsStillARecordingForTheAppsOwnReaders() {
        XCTAssertTrue(RecordingPauseRules.holdsRecording(hasActiveMeeting: true, isStarting: false, micActive: false, systemAudioActive: false, paused: true))
        XCTAssertFalse(RecordingPauseRules.mayStartAnotherMeeting(recordingHeld: true))
    }
}
