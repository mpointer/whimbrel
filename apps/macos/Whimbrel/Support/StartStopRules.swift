import Foundation

/// What a Stop does when it lands while a recording is starting. A detected or calendar start
/// turns the audio on before its meeting row exists (the row insert can queue behind minutes of
/// summary work), so for that window there is audio but no meeting to end. A person can always
/// stop a recording that has started: the audio goes off at once, and the meeting is finished
/// through the normal end once its row lands, with the audio captured so far attached to it.
///
/// Pure decisions only; `AppState` carries them out.
enum StartStopRules {
    /// What a Stop (the End Meeting button, the menu, or a forced stop) does now.
    enum StopAction: Equatable {
        /// A meeting row exists: end it as usual.
        case endMeeting
        /// A start is in flight with its audio started ahead of its row: stop the audio now and
        /// finish the meeting through the normal end when the row lands.
        case stopAudioAndEndWhenRowLands
        /// No row and no start in flight, but audio is running (its insert failed): stop it.
        case stopRowlessRecording
        /// Nothing to stop, or a stop is already waiting for the row.
        case nothing
    }

    static func stopAction(hasRow: Bool, isStarting: Bool, audioStartedAhead: Bool, stopRequested: Bool, audioLive: Bool) -> StopAction {
        if hasRow { return .endMeeting }
        if isStarting {
            return audioStartedAhead && !stopRequested ? .stopAudioAndEndWhenRowLands : .nothing
        }
        return audioLive ? .stopRowlessRecording : .nothing
    }

    /// Whether the menu offers End Meeting for a start in flight: its audio is live (or about to
    /// be) and no stop is waiting yet.
    static func canOfferStop(isStarting: Bool, audioStartedAhead: Bool, stopRequested: Bool) -> Bool {
        isStarting && audioStartedAhead && !stopRequested
    }

    /// What the start's completion does once its insert answers.
    enum FinishAction: Equatable {
        /// The start was replaced meanwhile: complete its row as abandoned (existing rule).
        case abandon
        /// Go live as usual.
        case proceed
        /// A stop was waiting and the row landed: set the meeting up and end it through the normal end.
        case endThroughNormalEnd
        /// No row: stop the audio and refuse (existing rule, a stop or not).
        case refuse
    }

    static func finishAction(generationCurrent: Bool, stopRequested: Bool, insertLanded: Bool) -> FinishAction {
        guard generationCurrent else { return .abandon }
        guard insertLanded else { return .refuse }
        return stopRequested ? .endThroughNormalEnd : .proceed
    }

    /// The completion waits for the audio start to settle first when a stop is waiting, so the
    /// audio file it attaches is final and a start that opened after the stop is closed again.
    static func mustSettleAudioFirst(stopRequested: Bool, alreadySettled: Bool) -> Bool {
        stopRequested && !alreadySettled
    }
}

extension StartStopRules {
    /// How long a stopped start waits for its audio start to settle before it goes on without it. The
    /// start can hang (a Screen Recording prompt left unanswered); it then finds itself stale when it
    /// resumes (`AudioStartEpochs`) and publishes nothing.
    static let settleTimeoutNanoseconds: UInt64 = 4_000_000_000

    /// What a stopped start does with its row once the row has landed.
    enum StoppedStartOutcome: Equatable {
        /// Audio was captured: end the meeting through the normal end with it attached.
        case endNormally
        /// A new meeting with no audio file: nothing was recorded, so the just-created row is deleted
        /// (as `MeetingSplit.discard` does for a split-off meeting that is still recording).
        case deleteEmptyNewMeeting
        /// A continued meeting with no new audio: the reopen and its new part are undone, so the earlier
        /// meeting is back exactly as it was and is not summarised again.
        case undoReopenOfContinued
    }

    static func stoppedStartOutcome(continued: Bool, audioFileExists: Bool) -> StoppedStartOutcome {
        if audioFileExists { return .endNormally }
        return continued ? .undoReopenOfContinued : .deleteEmptyNewMeeting
    }
}

/// Resumes a continuation once, whichever of two racing tasks gets there first.
@MainActor
final class OneShot {
    private var fired = false
    func fire(_ body: () -> Void) {
        guard !fired else { return }
        fired = true
        body()
    }
}

/// Which `AudioCaptureService.start()` owns the service. Every start and every stop moves the epoch on;
/// a start that resumes after an await may publish state only while the epoch it began with is still
/// current. Pure, so the interleavings are tested without any audio.
struct AudioStartEpochs: Equatable {
    private(set) var current = 0

    /// A start begins: it owns the service until something moves the epoch on.
    mutating func begin() -> Int {
        current += 1
        return current
    }

    /// A stop: every start in flight is stale from now on.
    mutating func stop() {
        current += 1
    }

    func owns(_ epoch: Int) -> Bool { epoch == current }
}
