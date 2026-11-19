import Foundation

// The pause of a recording (docs/ui-redesign/screens/live-meeting.md, "Pause on a recording"), as one pure
// value. The app owns one for the meeting that records, feeds it the events, and does what the answer says.
// Nothing here reads a clock, a device or a file: `now` comes in.
//
// "Recording pause" is not "capture pause". The capture pause (CapturePause.swift) is work memory's timed
// pause. The two never share a type, a name or a stored value.

/// Where a recording is. `problem` is a recording whose capture could not start again after a pause.
public enum RecordingPauseState: Equatable, Sendable { case recording, paused, problem, stopped }

/// Everything that can happen to the state.
public enum RecordingPauseEvent: Equatable, Sendable {
    case pause
    case resume
    /// Stop, from a recording, a pause or a problem. Quit, sleep, update and delete everything use this too.
    case stop
    /// Capture could not start again after a resume.
    case resumeFailed
}

/// What the app does for an event. `.none` means the event changes nothing (a second Pause, a Resume with no pause).
public enum RecordingPauseAction: Equatable, Sendable {
    case none
    /// Stop the microphone and system audio, stop snapshots, hold the silence checks. Keep the files open.
    case stopCapture
    /// Pad both files to now with silence, start capture and snapshots again, restart the silence checks.
    case startCapture
    /// End the meeting as it does today. A paused meeting stops without resuming first.
    case finish
    /// Say what is wrong, with the existing degraded reason. The meeting keeps what it has.
    case showProblem
}

/// One stretch with nothing captured. Offsets are seconds from the start of the recording on the clock
/// the stored audio and the transcript use, so they include earlier pauses.
public struct RecordingPausedSpan: Equatable, Codable, Sendable {
    public var start: Double
    /// Nil while the pause is still open: it runs to the end of the recording.
    public var end: Double?
    public init(start: Double, end: Double? = nil) {
        self.start = start
        self.end = end
    }
}

public struct RecordingPauseModel: Equatable, Sendable {
    public private(set) var state: RecordingPauseState
    public let startedAt: Date
    public private(set) var pausedSince: Date?
    /// Time spent paused in pauses that have ended.
    public private(set) var pausedTotal: TimeInterval
    /// Every pause so far, oldest first. The last one is open while the state is `paused`.
    public private(set) var spans: [RecordingPausedSpan]

    public init(startedAt: Date) {
        self.state = .recording
        self.startedAt = startedAt
        self.pausedSince = nil
        self.pausedTotal = 0
        self.spans = []
    }

    /// The state table. A step that does not apply returns `.none` and changes nothing.
    @discardableResult
    public mutating func apply(_ event: RecordingPauseEvent, now: Date) -> RecordingPauseAction {
        switch (state, event) {
        case (.recording, .pause):
            state = .paused
            pausedSince = now
            spans.append(RecordingPausedSpan(start: offset(of: now)))
            return .stopCapture
        case (.paused, .resume):
            closePause(at: now)
            state = .recording
            return .startCapture
        case (.paused, .resumeFailed):
            // The pause is over, even though capture did not come back: the gap stays silence.
            closePause(at: now)
            state = .problem
            return .showProblem
        case (.recording, .stop), (.paused, .stop), (.problem, .stop):
            closePause(at: now)
            state = .stopped
            return .finish
        default:
            return .none
        }
    }

    private mutating func closePause(at now: Date) {
        guard let since = pausedSince else { return }
        pausedTotal += max(0, now.timeIntervalSince(since))
        pausedSince = nil
        if var last = spans.popLast(), last.end == nil {
            last.end = offset(of: now)
            spans.append(last)
        }
    }

    private func offset(of date: Date) -> Double { max(0, date.timeIntervalSince(startedAt)) }

    public var isPaused: Bool { state == .paused }

    /// Time spent paused up to `now`, an open pause included.
    public func pausedSeconds(now: Date) -> TimeInterval {
        pausedTotal + (pausedSince.map { max(0, now.timeIntervalSince($0)) } ?? 0)
    }

    /// Seconds on the recording's own clock, pauses included: what the transcript and alerts use.
    public func clockSeconds(now: Date) -> Double { offset(of: now) }

    /// Recorded time: clock time minus paused time. Never negative. It stops counting while paused.
    public func recordedSeconds(now: Date) -> Int {
        let openPause = pausedSince.map { max(0, now.timeIntervalSince($0)) } ?? 0
        let total = now.timeIntervalSince(startedAt) - pausedTotal - openPause
        return max(0, Int(total.rounded(.down)))
    }

    /// The paused spans closed at `now` if one is open, for storage.
    public func spans(closingAt now: Date) -> [RecordingPausedSpan] {
        spans.map { span in
            var copy = span
            if copy.end == nil { copy.end = offset(of: now) }
            return copy
        }
    }

    /// The recording state the bar shows.
    public func barState(problem: String?) -> WRecordingState {
        switch state {
        case .recording: return .recording
        case .paused: return .paused
        case .problem: return .problem(problem ?? "")
        case .stopped: return .processing
        }
    }
}

/// Decisions about whether a paused recording is still a recording for the code that asks.
public enum RecordingPauseRules {
    /// Whether anything is recording or about to. A pause never makes this false: a second meeting cannot
    /// start, a detected call does not start one, background work stays held, and quit, update and delete
    /// everything treat the meeting as recording.
    public static func holdsRecording(hasActiveMeeting: Bool, isStarting: Bool, micActive: Bool, systemAudioActive: Bool, paused: Bool) -> Bool {
        hasActiveMeeting || isStarting || micActive || systemAudioActive || paused
    }

    /// Whether the silence checks, the stream health restarts and the "Still in a meeting?" check run.
    /// They are held while paused, and their clocks start again on resume.
    public static func checksRun(paused: Bool) -> Bool { !paused }

    /// Whether a stream that is not delivering buffers is a fault. While paused none should arrive.
    public static func missingAudioIsAFault(paused: Bool) -> Bool { !paused }

    /// Pausing never marks the audio as degraded. Only a failure to start again after the pause does.
    public static func pauseMarksDegraded(captureRestartFailed: Bool) -> Bool { captureRestartFailed }

    /// Whether another meeting may start, by hand or because a call was detected.
    public static func mayStartAnotherMeeting(recordingHeld: Bool) -> Bool { !recordingHeld }

    /// Whether a snapshot of the screen may be taken.
    public static func snapshotsRun(paused: Bool) -> Bool { !paused }

    /// The hourly check-in's words while paused ("This recording is still paused.") or recording.
    public static func checkInIsAboutPause(paused: Bool) -> Bool { paused }

    /// Whether the hourly check-in may end the recording itself. It never does while paused.
    public static func checkInMayEnd(paused: Bool) -> Bool { !paused }
}

/// "No text from a pause": the silence written for a pause is still read by the transcriber after the
/// meeting stops. A segment that lies in a paused span is dropped.
public enum RecordingPausedText {
    /// Whether a segment that runs from `start` to `end` seconds lies in a paused span: its midpoint is inside.
    /// Windows uses the same rule, so both OSes drop the same lines. An open span runs to the end of the recording.
    public static func isInsideSpan(start: Double, end: Double, spans: [RecordingPausedSpan]) -> Bool {
        let midpoint = (start + max(start, end)) / 2
        return spans.contains { midpoint >= $0.start && midpoint < ($0.end ?? .greatestFiniteMagnitude) }
    }

    /// Spans merged and ordered, so overlapping or touching ones count once.
    public static func normalized(_ spans: [RecordingPausedSpan]) -> [RecordingPausedSpan] {
        let sorted = spans.sorted { $0.start < $1.start }
        var out: [RecordingPausedSpan] = []
        for span in sorted {
            if var last = out.last, span.start <= (last.end ?? .greatestFiniteMagnitude) {
                if let lastEnd = last.end {
                    last.end = span.end.map { max(lastEnd, $0) }
                }
                out[out.count - 1] = last
            } else {
                out.append(span)
            }
        }
        return out
    }
}
