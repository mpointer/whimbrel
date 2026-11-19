import Foundation
import Testing
@testable import WhimbrelUI

/// One test per row of the table in docs/ui-redesign/screens/live-meeting.md, "Pause on a recording".
@Suite struct RecordingPauseTests {
    private let start = Date(timeIntervalSince1970: 1_800_000_000)
    private func at(_ seconds: Double) -> Date { start.addingTimeInterval(seconds) }
    private func model() -> RecordingPauseModel { RecordingPauseModel(startedAt: start) }

    @Test func pauseFromRecordingStopsCaptureAndNotesTheStart() {
        var m = model()
        #expect(m.apply(.pause, now: at(60)) == .stopCapture)
        #expect(m.state == .paused)
        #expect(m.pausedSince == at(60))
        #expect(m.spans == [RecordingPausedSpan(start: 60, end: nil)])
    }

    @Test func resumeFromPausedStartsCaptureAndAddsTheGap() {
        var m = model()
        m.apply(.pause, now: at(60))
        #expect(m.apply(.resume, now: at(180)) == .startCapture)
        #expect(m.state == .recording)
        #expect(m.pausedSince == nil)
        #expect(m.pausedTotal == 120)
        #expect(m.spans == [RecordingPausedSpan(start: 60, end: 180)])
    }

    @Test func stopFromRecordingFinishes() {
        var m = model()
        #expect(m.apply(.stop, now: at(300)) == .finish)
        #expect(m.state == .stopped)
        #expect(m.barState(problem: nil) == .processing)
    }

    @Test func stopFromPausedFinishesWithoutResumingFirst() {
        var m = model()
        m.apply(.pause, now: at(100))
        #expect(m.apply(.stop, now: at(400)) == .finish)
        #expect(m.state == .stopped)
        // The pause is closed at the stop, so the stored span ends there.
        #expect(m.spans == [RecordingPausedSpan(start: 100, end: 400)])
        #expect(m.pausedTotal == 300)
    }

    @Test func quitSleepUpdateAndDeleteEverythingUseTheStopPathFromPaused() {
        // They are the same event as Stop: a paused meeting ends the way a recording that is not paused ends.
        for _ in 0..<4 {
            var m = model()
            m.apply(.pause, now: at(10))
            #expect(m.apply(.stop, now: at(20)) == .finish)
        }
    }

    @Test func captureFailingToRestartMovesToProblemAndKeepsWhatItHas() {
        var m = model()
        m.apply(.pause, now: at(100))
        #expect(m.apply(.resumeFailed, now: at(130)) == .showProblem)
        #expect(m.state == .problem)
        #expect(m.barState(problem: "Microphone not found") == .problem("Microphone not found"))
        #expect(m.spans == [RecordingPausedSpan(start: 100, end: 130)])
        // Stop stays available from a problem, and nothing else changes it.
        var stopped = m
        #expect(stopped.apply(.stop, now: at(200)) == .finish)
        #expect(m.apply(.pause, now: at(140)) == .none)
        #expect(m.apply(.resume, now: at(140)) == .none)
    }

    @Test func crashWhilePausedLeavesTheOpenSpanToTheEndOfTheRecording() {
        var m = model()
        m.apply(.pause, now: at(100))
        // A crash writes nothing more. What was stored has an open span; the transcript drops text in it.
        #expect(m.spans.last?.end == nil)
        #expect(RecordingPausedText.isInsideSpan(start: 500, end: 510, spans: m.spans))
        #expect(m.spans(closingAt: at(150)) == [RecordingPausedSpan(start: 100, end: 150)])
    }

    @Test func eventsThatDoNotApplyChangeNothing() {
        var m = model()
        #expect(m.apply(.resume, now: at(5)) == .none)
        #expect(m.apply(.resumeFailed, now: at(5)) == .none)
        m.apply(.pause, now: at(10))
        #expect(m.apply(.pause, now: at(20)) == .none)
        #expect(m.pausedSince == at(10))
        m.apply(.stop, now: at(30))
        #expect(m.apply(.pause, now: at(40)) == .none)
        #expect(m.apply(.stop, now: at(40)) == .none)
    }

    // MARK: Recorded time

    @Test func recordedTimeIsClockTimeMinusPausedTime() {
        var m = model()
        #expect(m.recordedSeconds(now: at(60)) == 60)
        m.apply(.pause, now: at(60))
        // While paused it holds.
        #expect(m.recordedSeconds(now: at(60)) == 60)
        #expect(m.recordedSeconds(now: at(200)) == 60)
        m.apply(.resume, now: at(200))
        #expect(m.recordedSeconds(now: at(260)) == 120)
        // The clock the transcript uses keeps running through the pause.
        #expect(m.clockSeconds(now: at(260)) == 260)
    }

    @Test func recordedTimeAddsUpSeveralPauses() {
        var m = model()
        m.apply(.pause, now: at(30)); m.apply(.resume, now: at(90))
        m.apply(.pause, now: at(150)); m.apply(.resume, now: at(210))
        #expect(m.pausedTotal == 120)
        #expect(m.recordedSeconds(now: at(300)) == 180)
        #expect(m.spans == [RecordingPausedSpan(start: 30, end: 90), RecordingPausedSpan(start: 150, end: 210)])
    }

    @Test func recordedTimeNeverGoesNegative() {
        let m = model()
        #expect(m.recordedSeconds(now: at(-5)) == 0)
    }

    // MARK: A paused meeting is still a recording

    @Test func aPausedMeetingStillHoldsRecording() {
        // The engine is off while paused, so mic and system audio can read false; the pause itself holds it.
        #expect(RecordingPauseRules.holdsRecording(hasActiveMeeting: false, isStarting: false, micActive: false, systemAudioActive: false, paused: true))
        #expect(RecordingPauseRules.holdsRecording(hasActiveMeeting: true, isStarting: false, micActive: false, systemAudioActive: false, paused: true))
        #expect(!RecordingPauseRules.holdsRecording(hasActiveMeeting: false, isStarting: false, micActive: false, systemAudioActive: false, paused: false))
    }

    @Test func aPauseCannotLetASecondMeetingStartOrADetectedCallStartOne() {
        let held = RecordingPauseRules.holdsRecording(hasActiveMeeting: true, isStarting: false, micActive: false, systemAudioActive: false, paused: true)
        #expect(!RecordingPauseRules.mayStartAnotherMeeting(recordingHeld: held))
    }

    @Test func aPauseNeverMarksTheAudioDegradedButAFailedRestartDoes() {
        #expect(!RecordingPauseRules.pauseMarksDegraded(captureRestartFailed: false))
        #expect(RecordingPauseRules.pauseMarksDegraded(captureRestartFailed: true))
        #expect(!RecordingPauseRules.missingAudioIsAFault(paused: true))
        #expect(RecordingPauseRules.missingAudioIsAFault(paused: false))
    }

    @Test func theChecksAreHeldWhilePausedAndRunWhenNot() {
        #expect(!RecordingPauseRules.checksRun(paused: true))
        #expect(RecordingPauseRules.checksRun(paused: false))
        #expect(!RecordingPauseRules.snapshotsRun(paused: true))
        #expect(RecordingPauseRules.snapshotsRun(paused: false))
    }

    @Test func theHourlyCheckInIsAboutThePauseAndNeverEndsIt() {
        #expect(RecordingPauseRules.checkInIsAboutPause(paused: true))
        #expect(!RecordingPauseRules.checkInMayEnd(paused: true))
        #expect(RecordingPauseRules.checkInMayEnd(paused: false))
    }

    // MARK: No text from a pause

    private let spans = [RecordingPausedSpan(start: 100, end: 160), RecordingPausedSpan(start: 300, end: 330)]

    @Test func aSegmentInsideASpanIsDropped() {
        #expect(RecordingPausedText.isInsideSpan(start: 110, end: 118, spans: spans))
        #expect(RecordingPausedText.isInsideSpan(start: 305, end: 306, spans: spans))
    }

    @Test func aSegmentOutsideEverySpanStays() {
        #expect(!RecordingPausedText.isInsideSpan(start: 20, end: 28, spans: spans))
        #expect(!RecordingPausedText.isInsideSpan(start: 200, end: 210, spans: spans))
        #expect(!RecordingPausedText.isInsideSpan(start: 20, end: 28, spans: []))
    }

    @Test func theMidpointDecidesLikeOnWindows() {
        // Midpoint 161 is outside [100, 160): the segment stays. Midpoint 158 is inside: it goes.
        #expect(!RecordingPausedText.isInsideSpan(start: 156, end: 166, spans: spans))
        #expect(RecordingPausedText.isInsideSpan(start: 153, end: 163, spans: spans))
        // Even a segment mostly outside goes when its midpoint is inside.
        #expect(RecordingPausedText.isInsideSpan(start: 140, end: 170, spans: spans) == (155 < 160))
        #expect(!RecordingPausedText.isInsideSpan(start: 160, end: 160, spans: spans), "the end of a span is outside it")
        #expect(RecordingPausedText.isInsideSpan(start: 100, end: 100, spans: spans), "the start is inside")
    }

    @Test func theSpansAreStoredAtEachStepAndAnOpenSpanRunsToTheEnd() throws {
        var m = model()
        m.apply(.pause, now: at(100))
        // What is written when the pause begins: one open span, which survives a round trip and covers the rest.
        let open = try JSONDecoder().decode([RecordingPausedSpan].self, from: JSONEncoder().encode(m.spans))
        #expect(open == [RecordingPausedSpan(start: 100, end: nil)])
        #expect(RecordingPausedText.isInsideSpan(start: 9000, end: 9010, spans: open))
        m.apply(.resume, now: at(160))
        // And when it resumes: the span is closed, so later text stays.
        let closed = try JSONDecoder().decode([RecordingPausedSpan].self, from: JSONEncoder().encode(m.spans))
        #expect(closed == [RecordingPausedSpan(start: 100, end: 160)])
        #expect(!RecordingPausedText.isInsideSpan(start: 200, end: 210, spans: closed))
    }

    @Test func aSegmentWithNoLengthIsJudgedByItsStart() {
        #expect(RecordingPausedText.isInsideSpan(start: 120, end: 120, spans: spans))
        #expect(!RecordingPausedText.isInsideSpan(start: 90, end: 90, spans: spans))
    }

    @Test func spansMergeAndSurviveARoundTripThroughJSON() throws {
        let merged = RecordingPausedText.normalized([
            RecordingPausedSpan(start: 300, end: 330), RecordingPausedSpan(start: 100, end: 160), RecordingPausedSpan(start: 150, end: 200),
        ])
        #expect(merged == [RecordingPausedSpan(start: 100, end: 200), RecordingPausedSpan(start: 300, end: 330)])
        let data = try JSONEncoder().encode(merged + [RecordingPausedSpan(start: 400)])
        let back = try JSONDecoder().decode([RecordingPausedSpan].self, from: data)
        #expect(back.last == RecordingPausedSpan(start: 400, end: nil))
        #expect(back.count == 3)
    }
}

@Suite struct LevelMathTests {
    @Test func silenceIsZeroAndALoudPeakIsOne() {
        #expect(WLevelMath.level(fromPeak: 0) == 0)
        #expect(WLevelMath.level(fromPeak: 1) == 1)
        #expect(WLevelMath.level(fromPeak: 0.0005) == 0)
    }

    @Test func theLevelRisesWithThePeak() {
        let quiet = WLevelMath.level(fromPeak: 0.02)
        let speech = WLevelMath.level(fromPeak: 0.2)
        #expect(quiet < speech)
        #expect(speech > 0.5 && speech < 1)
    }

    @Test func theMeterNamesAudioAtTheSilenceWarningsFloor() {
        // -50 dB is the silence warning's threshold, a peak of about 0.00316.
        #expect(WLevelMath.isAudio(WLevelMath.level(fromPeak: 0.01)))
        #expect(!WLevelMath.isAudio(WLevelMath.level(fromPeak: 0.0015)))
        #expect(!WLevelMath.isAudio(0))
    }
}

@Suite struct LiveModelTests {
    @Test func theNoticeIsNeverDrawnOnAnotherDevicesMeetingOrWhileProcessing() {
        var m = WLiveMeetingModel(title: "t", state: .recording, showsNotice: true)
        #expect(m.noticeVisible)
        m.state = .readOnly
        #expect(!m.noticeVisible)
        m.state = .processing(stage: nil)
        #expect(!m.noticeVisible)
        m.state = .paused
        #expect(m.noticeVisible)
    }

    @Test func theBarStateFollowsTheLiveState() {
        #expect(WLiveState.starting.barState == .starting)
        #expect(WLiveState.resuming.barState == .resuming)
        #expect(WLiveState.readOnly.barState == nil)
        #expect(WLiveState.problem("x").barState == .problem("x"))
        #expect(WLiveState.recording.allowsMarking && WLiveState.paused.allowsMarking)
        #expect(!WLiveState.starting.allowsMarking && !WLiveState.processing(stage: nil).allowsMarking)
    }

    @Test func theMeterIsNamedForWhetherAudioArrives() {
        let s = WStrings.english
        #expect(WLiveMeetingModel(title: "t", level: 0.6).meterName(using: s) == "Audio is arriving")
        #expect(WLiveMeetingModel(title: "t", level: 0).meterName(using: s) == "No audio right now")
    }

    @Test func stateChangesAreAnnouncedWithTheSpecWords() {
        let s = WStrings.english
        #expect(WRecordingState.paused.announcement(using: s) == "Recording paused")
        #expect(WRecordingState.recording.announcement(using: s) == "Recording")
        #expect(WRecordingState.resuming.announcement(using: s) == "Resuming")
    }
}
