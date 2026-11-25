import Foundation

// The live meeting screen and the in-meeting window as plain values (docs/ui-redesign/screens/live-meeting.md).
// A model says what to show; text the app already owns arrives in it, already in the person's language.
// `WLiveActions` says what each control does. The views are in LiveViews.swift.

/// Where the live meeting is, for the screen and the window.
public enum WLiveState: Equatable, Sendable {
    /// Controls wait.
    case starting
    case recording
    case paused
    /// Capture is starting again; the bar says "Resuming" until audio arrives.
    case resuming
    /// Stopping and processing, with the stage (for example "Transcribing") under the bar.
    case processing(stage: String?)
    /// What is wrong, in a few words. Stop stays available.
    case problem(String)
    /// Another device's meeting: no controls, no notice.
    case readOnly

    /// The state the recording bar draws. A read-only meeting has no bar.
    public var barState: WRecordingState? {
        switch self {
        case .starting: return .starting
        case .recording: return .recording
        case .paused: return .paused
        case .resuming: return .resuming
        case .processing: return .processing
        case .problem(let reason): return .problem(reason)
        case .readOnly: return nil
        }
    }

    /// Whether a recording is still going, so the audio line and the meter mean something.
    public var isLive: Bool {
        switch self {
        case .recording, .paused, .resuming, .problem, .starting: return true
        case .processing, .readOnly: return false
        }
    }

    /// Whether Mark this moment is offered: only while the recording runs or is paused.
    public var allowsMarking: Bool {
        switch self {
        case .recording, .paused: return true
        default: return false
        }
    }
}

/// One moment the person marked. `timeOfDay` is the clock time ("10:12"). The stored time is the app's.
public struct WLiveMark: Identifiable, Equatable, Sendable {
    public let id: String
    public var timeOfDay: String
    public var label: String
    public init(id: String, timeOfDay: String, label: String) {
        self.id = id
        self.timeOfDay = timeOfDay
        self.label = label
    }
}

public enum WLiveStopKind: Sendable { case next, now, started }

/// One stop on the short time line at the left: the next meeting above, now, and the time recording started.
public struct WLiveStop: Identifiable, Equatable, Sendable {
    public let id: String
    public var time: String
    public var caption: String?
    public var kind: WLiveStopKind
    public init(id: String, time: String, caption: String? = nil, kind: WLiveStopKind) {
        self.id = id
        self.time = time
        self.caption = caption
        self.kind = kind
    }
}

public struct WLiveMeetingModel: Sendable {
    public var title: String
    /// "Today, 10:00 AM. Sam Okafor, Priya Shah, you."
    public var detail: String
    public var state: WLiveState
    /// Clock time minus paused time.
    public var recordedSeconds: Int
    /// 0 to 1, from the capture code. Zero while paused or stopped.
    public var level: Float
    /// The recording notice is drawn once, when it is on and not yet dismissed for this meeting.
    public var showsNotice: Bool
    public var noticeCopied: Bool
    public var marks: [WLiveMark]
    public var stops: [WLiveStop]
    /// The right column: only when live transcript is turned on.
    public var showsTranscript: Bool
    public var transcript: [WTranscriptEntry]
    /// "Auto-scroll on" or similar, on the right of the transcript card's title.
    public var transcriptAccessory: String?
    /// The id of a transcript entry to show as cited and scroll to: a live-alert jump or a line the ask bar cited.
    public var highlightedEntryId: String?

    public init(
        title: String, detail: String = "", state: WLiveState = .recording, recordedSeconds: Int = 0, level: Float = 0,
        showsNotice: Bool = false, noticeCopied: Bool = false, marks: [WLiveMark] = [], stops: [WLiveStop] = [],
        showsTranscript: Bool = false, transcript: [WTranscriptEntry] = [], transcriptAccessory: String? = nil,
        highlightedEntryId: String? = nil
    ) {
        self.title = title
        self.detail = detail
        self.state = state
        self.recordedSeconds = recordedSeconds
        self.level = level
        self.showsNotice = showsNotice
        self.noticeCopied = noticeCopied
        self.marks = marks
        self.stops = stops
        self.showsTranscript = showsTranscript
        self.transcript = transcript
        self.transcriptAccessory = transcriptAccessory
        self.highlightedEntryId = highlightedEntryId
    }

    /// Whether the notice is drawn: never on another device's meeting, and not once processing starts.
    public var noticeVisible: Bool {
        guard showsNotice else { return false }
        switch state {
        case .readOnly, .processing: return false
        default: return true
        }
    }

    /// The name of the level meter for a screen reader.
    public func meterName(using strings: WStrings) -> String {
        WLevelMath.isAudio(level) ? strings.live.audioArriving : strings.live.noAudioNow
    }
}

public struct WLiveActions {
    public var pause: () -> Void
    public var resume: () -> Void
    public var stop: () -> Void
    public var mark: () -> Void
    public var removeMark: (String) -> Void
    public var copyNotice: () -> Void
    public var dismissNotice: () -> Void

    public init(
        pause: @escaping () -> Void = {}, resume: @escaping () -> Void = {}, stop: @escaping () -> Void = {},
        mark: @escaping () -> Void = {}, removeMark: @escaping (String) -> Void = { _ in },
        copyNotice: @escaping () -> Void = {}, dismissNotice: @escaping () -> Void = {}
    ) {
        self.pause = pause
        self.resume = resume
        self.stop = stop
        self.mark = mark
        self.removeMark = removeMark
        self.copyNotice = copyNotice
        self.dismissNotice = dismissNotice
    }
}

/// The in-meeting window: a small always-on-top window with the title, the state, the time, the level, Pause
/// or Resume, Stop, the ask box and the audio line.
public struct WMeetingBarModel: Sendable {
    public var title: String
    public var state: WLiveState
    public var recordedSeconds: Int
    public var level: Float

    public init(title: String, state: WLiveState = .recording, recordedSeconds: Int = 0, level: Float = 0) {
        self.title = title
        self.state = state
        self.recordedSeconds = recordedSeconds
        self.level = level
    }
}
