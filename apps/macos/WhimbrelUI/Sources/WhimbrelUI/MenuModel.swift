import Foundation

// The menu bar menu, the small alert windows and the hotkey panel as plain values (docs/ui-redesign/screens/menu-bar.md).
// A model says what to show; every text arrives in it already in the person's language. The actions say what each
// control does. The views are in MenuViews.swift, AlertPanels.swift and HotkeyPanel.swift.

// MARK: The menu

/// One thing a row in the menu can do, with its name.
public struct WMenuAction {
    public var title: String
    public var spokenName: String?
    /// The action opens a web page: a screen reader says "link", as it did when the old menu drew it as a link.
    public var isLink: Bool
    public var run: () -> Void
    public init(title: String, spokenName: String? = nil, isLink: Bool = false, run: @escaping () -> Void) {
        self.title = title
        self.spokenName = spokenName
        self.isLink = isLink
        self.run = run
    }
}

/// Which audio streams are live now. The menu says so in words, so a recording that has lost both is not drawn as fine.
public struct WMenuStreams: Equatable, Sendable {
    public var microphone: Bool
    public var systemAudio: Bool
    public init(microphone: Bool, systemAudio: Bool) {
        self.microphone = microphone
        self.systemAudio = systemAudio
    }
    public var isNeither: Bool { !microphone && !systemAudio }
}

/// The recording section, drawn whenever audio is being captured or a meeting is open or starting. It is never left
/// out and never shortened: it always says that a meeting is open, in which state, for how long and which one.
public struct WMenuRecording {
    public var state: WRecordingState
    public var elapsedSeconds: Int?
    /// The meeting's title, in its own line under the state.
    public var title: String?
    /// Pause or Resume is offered and not waiting.
    public var canTogglePause: Bool
    /// The streams that are live. Nil when the app does not know.
    public var streams: WMenuStreams?
    /// Stop can run now. It cannot while a recording is only starting and has no meeting yet.
    public var canStop: Bool
    /// A Stop was asked for during the start and the meeting is still being saved: the audio is off and nothing is left to stop. The
    /// row shows a disabled "Saving..." where Stop was, and no line about Stop.
    public var isSaving: Bool

    public init(
        state: WRecordingState, elapsedSeconds: Int? = nil, title: String? = nil, canTogglePause: Bool = true, streams: WMenuStreams? = nil,
        canStop: Bool = true, isSaving: Bool = false
    ) {
        self.canStop = canStop
        self.isSaving = isSaving
        self.state = state
        self.elapsedSeconds = elapsedSeconds
        self.title = title
        self.canTogglePause = canTogglePause
        self.streams = streams
    }

    /// Whether the buttons offer Resume (a paused or resuming recording) rather than Pause.
    public var offersResume: Bool { state == .paused || state == .resuming }

    /// Whether Stop is offered: any state where a recording is still going.
    public var offersStop: Bool {
        switch state {
        case .recording, .paused, .resuming, .starting, .problem: return true
        case .processing, .off: return false
        }
    }

    /// Pause or Resume is drawn while the recording starts, runs or is paused. A problem has neither: Resume only runs from a
    /// pause (`AppState.resumeRecording` ignores anything else), so offering it would be a button that does nothing. The
    /// live meeting screen and the in-meeting window offer Stop alone in this state too.
    public var showsPauseButton: Bool {
        switch state {
        case .recording, .paused, .resuming, .starting: return true
        case .processing, .off, .problem: return false
        }
    }

    /// The reason to say in words next to the state, for a problem. Nil when the app gave none.
    public var problemReason: String? {
        if case .problem(let reason) = state, !reason.isEmpty { return reason }
        return nil
    }

    /// Whether the streams line is drawn: only while audio should be arriving. A paused recording captures nothing, so it
    /// says nothing about streams. A start with a stream already live says which (the microphone can be on before the meeting
    /// is saved); a start with none says nothing, since "no audio" would be an alarm for what is only a moment.
    public var showsStreams: Bool {
        if state == .recording || state.isProblem { return true }
        return state == .starting && streams.map { !$0.isNeither } == true
    }

    /// Audio is live but Stop cannot run, and no Stop is on its way: a start that is not one of the detected or calendar starts
    /// whose audio runs ahead of its row (`WStopOffer`). The menu says so in a line, so the person is told why the button is off while
    /// the microphone is on. Not shown while Stop works, nor while the Stop already asked for is being saved (`isSaving`).
    public var stopWaitsForMeeting: Bool {
        state == .starting && !canStop && !isSaving && streams.map { !$0.isNeither } == true
    }
}

/// What the recording controls offer where Stop would be.
public enum WStopOffer: Equatable, Sendable {
    /// Stop works now.
    case stop
    /// A Stop was asked for and the meeting is still being saved: a disabled "Saving..." and no Stop.
    case saving
    /// Stop does nothing yet and none is on its way.
    case unavailable
}

/// Pure rules behind the menu's recording section, so they run in `swift test` and the app only passes values in.
public enum WMenuRecordingRules {
    /// Which of Stop, Saving or nothing the recording controls offer. Stop works when a meeting is open, when audio records with
    /// no meeting row after a failed insert, and for a start still waiting on its row whose audio is already on
    /// (`canStopStarting`, the app's `StartStopRules.canOfferStop`): the app then stops the audio at once and finishes the meeting
    /// when the row lands. After that Stop, until the row lands, the controls say Saving (`stopPendingForRow`). Any other start in
    /// flight has nothing to stop yet.
    public static func stopOffer(
        hasActiveMeeting: Bool, isStarting: Bool, audioLive: Bool, canStopStarting: Bool, stopPendingForRow: Bool
    ) -> WStopOffer {
        if hasActiveMeeting { return .stop }
        if isStarting {
            if canStopStarting { return .stop }
            return stopPendingForRow ? .saving : .unavailable
        }
        return audioLive ? .stop : .unavailable
    }

    /// How long after a recording starts the app waits for the first audio before it calls a silent recording a problem.
    /// Capture starts in a task after the meeting is open, and the system stream can take a moment to open.
    public static let startGrace: TimeInterval = 5

    /// When the current start began: the later of the meeting's start and the audio's own start. A continued meeting keeps its
    /// first start time (long ago), and the audio's start time is the last recording's until the new one begins; the later of
    /// the two is the one that belongs to this start. Nil when neither is known.
    public static func startAnchor(meetingStartedAt: Date?, audioStartedAt: Date?) -> Date? {
        switch (meetingStartedAt, audioStartedAt) {
        case let (a?, b?): return max(a, b)
        case let (a?, nil): return a
        case let (nil, b?): return b
        case (nil, nil): return nil
        }
    }

    /// The state to draw. A running recording with neither stream live is a problem, not silence, but only once capture has had
    /// `grace` seconds from `startedAt` to begin: before that the row says it is starting. A paused, resuming, starting or
    /// problem state is never judged by the flags (a pause leaves them set while nothing is captured). With no start time known
    /// the streams are judged at once.
    public static func judged(
        _ state: WRecordingState, microphoneLive: Bool, systemAudioLive: Bool, startedAt: Date?, now: Date, reason: String?,
        grace: TimeInterval = startGrace
    ) -> WRecordingState {
        guard state == .recording, !microphoneLive, !systemAudioLive else { return state }
        if let startedAt, now.timeIntervalSince(startedAt) < grace { return .starting }
        return .problem(reason ?? "")
    }
}

/// How long work memory may be on and not saving before the menu calls it a problem. Memory starts at launch, after a timed
/// pause and when the person turns it on, and capture follows within a moment; a state that holds only that long is not news.
public enum WMemoryHold {
    public static let grace: TimeInterval = 5

    /// `since` is when memory was last switched on or the app launched (not when this view was drawn), so a state that has
    /// held for hours is called what it is at once. With no time known nothing is held back.
    public static func state(_ memory: WMemoryState, since: Date?, now: Date, grace: TimeInterval = grace) -> WMemoryState {
        guard memory == .onNotCapturing, let since, now.timeIntervalSince(since) < grace else { return memory }
        return .on
    }

    /// The moment the hold ends, while it is still holding: memory is switched on, not capturing, and `since + grace` is ahead of
    /// `now`. Nil otherwise (nothing is held back, or it has already elapsed). A view that draws "Memory on" in the hold must draw
    /// again at this moment, or the words wait for the next periodic tick (up to a minute).
    public static func holdEnds(capturing: Bool, since: Date?, now: Date, grace: TimeInterval = grace) -> Date? {
        guard !capturing, let since else { return nil }
        let end = since.addingTimeInterval(grace)
        return end > now ? end : nil
    }
}

/// When a periodic view next redraws: the next tick of `interval` from `anchor` after `t`, or `extra` if that comes first. The
/// menu and Home keep one periodic schedule (one view identity) and add the end of the memory hold to it as `extra`.
public enum WRedrawClock {
    public static func next(after t: Date, anchor: Date, interval: TimeInterval, extra: Date?) -> Date {
        let step = max(interval, 0.001)
        var tick = anchor.addingTimeInterval((floor(t.timeIntervalSince(anchor) / step) + 1) * step)
        if tick <= t { tick = tick.addingTimeInterval(step) }
        if let extra, extra > t, extra < tick { return extra }
        return tick
    }
}

/// Whether the scrolling middle of a capped menu has more above or below it, for the cue at its edges.
public struct WMenuScrollCue: Equatable, Sendable {
    public var above: Bool
    public var below: Bool
    public init(above: Bool, below: Bool) {
        self.above = above
        self.below = below
    }

    /// `offset` is how far the content has moved up (0 at the top). A row cut by less than `tolerance` points is not a cue.
    public static func cue(contentHeight: CGFloat, viewportHeight: CGFloat, offset: CGFloat, tolerance: CGFloat = 2) -> WMenuScrollCue {
        guard viewportHeight > 0, contentHeight - viewportHeight > tolerance else { return WMenuScrollCue(above: false, below: false) }
        let travel = contentHeight - viewportHeight
        return WMenuScrollCue(above: offset > tolerance, below: offset < travel - tolerance)
    }
}

public enum WMenuNeedKind: Sendable { case permission, audio, sync, library, noStore, memory }

/// One thing that needs attention: what is wrong and, where there is one, what to do.
public struct WMenuNeed: Identifiable {
    public let id: String
    public var kind: WMenuNeedKind
    public var title: String
    public var detail: String?
    public var actions: [WMenuAction]

    public init(id: String, kind: WMenuNeedKind, title: String, detail: String? = nil, actions: [WMenuAction] = []) {
        self.id = id
        self.kind = kind
        self.title = title
        self.detail = detail
        self.actions = actions
    }
}

public struct WMenuNext {
    public var title: String
    /// "10:00 AM".
    public var time: String
    public var topic: String?
    public var hasBrief: Bool

    public init(title: String, time: String, topic: String? = nil, hasBrief: Bool = false) {
        self.title = title
        self.time = time
        self.topic = topic
        self.hasBrief = hasBrief
    }
}

public struct WMenuModel {
    public var recording: WMenuRecording?
    public var memory: WMemoryState
    public var needs: [WMenuNeed]
    public var next: WMenuNext?
    /// The caption under Start Meeting while recording is unavailable (no store).
    public var startCaption: String?
    /// The processing stage, for example "Transcribing".
    public var workingOn: String?
    /// Controls that are drawn but switched off, so arrow keys and the first focus skip them.
    public var disabled: Set<String>

    public init(
        recording: WMenuRecording? = nil, memory: WMemoryState = .on, needs: [WMenuNeed] = [], next: WMenuNext? = nil,
        startCaption: String? = nil, workingOn: String? = nil, disabled: Set<String> = []
    ) {
        self.disabled = disabled
        self.recording = recording
        self.memory = memory
        self.needs = needs
        self.next = next
        self.startCaption = startCaption
        self.workingOn = workingOn
    }

    /// Start Meeting is offered only when nothing records.
    public var offersStart: Bool { recording == nil }
    public var showsNeeds: Bool { !needs.isEmpty }

    /// Where keyboard focus can go, top to bottom. Arrow keys move through this list. A control that is switched off is
    /// not in it: focus cannot rest there, so a key would stop on it.
    public var focusOrder: [String] {
        var ids: [String] = []
        if let recording {
            if recording.showsPauseButton && recording.canTogglePause { ids.append(WMenuFocusID.pause) }
            if recording.offersStop && recording.canStop { ids.append(WMenuFocusID.stop) }
            ids.append(WMenuFocusID.meetingWindow)
        }
        ids.append(WMenuFocusID.memory)
        for need in needs {
            for index in need.actions.indices { ids.append(WMenuFocusID.need(need.id, index)) }
            if need.kind == .sync { ids.append(WMenuFocusID.need(need.id, need.actions.count)) }
        }
        if let next {
            if next.topic != nil { ids.append(WMenuFocusID.topic) }
            if next.hasBrief { ids.append(WMenuFocusID.brief) }
        }
        if offersStart { ids.append(WMenuFocusID.start) }
        ids.append(contentsOf: [WMenuFocusID.ask, WMenuFocusID.open, WMenuFocusID.settings, WMenuFocusID.quit])
        return ids.filter { !disabled.contains($0) }
    }

    /// Where focus goes when the menu opens: the first control that is on.
    public var initialFocus: String? { focusOrder.first }
}

public enum WMenuFocusID {
    public static let pause = "pause", stop = "stop", meetingWindow = "meetingWindow", memory = "memory"
    public static let topic = "topic", brief = "brief", start = "start", ask = "ask"
    public static let open = "open", settings = "settings", quit = "quit"
    public static func need(_ id: String, _ index: Int) -> String { ["need", id, String(index)].joined(separator: ".") }
}

public struct WMenuActions {
    public var pauseRecording: () -> Void
    public var resumeRecording: () -> Void
    public var stopRecording: () -> Void
    public var showMeetingWindow: () -> Void
    public var pauseMemory: (CapturePauseChoice) -> Void
    public var resumeMemory: () -> Void
    public var toggleMemory: () -> Void
    public var whatIsCaptured: () -> Void
    public var openBrief: () -> Void
    public var openTopic: () -> Void
    public var startMeeting: () -> Void
    public var askAboutWindow: () -> Void
    public var openWhimbrel: () -> Void
    public var openSettings: () -> Void
    public var quit: () -> Void

    public init(
        pauseRecording: @escaping () -> Void = {}, resumeRecording: @escaping () -> Void = {}, stopRecording: @escaping () -> Void = {},
        showMeetingWindow: @escaping () -> Void = {}, pauseMemory: @escaping (CapturePauseChoice) -> Void = { _ in },
        resumeMemory: @escaping () -> Void = {}, toggleMemory: @escaping () -> Void = {}, whatIsCaptured: @escaping () -> Void = {},
        openBrief: @escaping () -> Void = {}, openTopic: @escaping () -> Void = {}, startMeeting: @escaping () -> Void = {},
        askAboutWindow: @escaping () -> Void = {}, openWhimbrel: @escaping () -> Void = {}, openSettings: @escaping () -> Void = {},
        quit: @escaping () -> Void = {}
    ) {
        self.pauseRecording = pauseRecording
        self.resumeRecording = resumeRecording
        self.stopRecording = stopRecording
        self.showMeetingWindow = showMeetingWindow
        self.pauseMemory = pauseMemory
        self.resumeMemory = resumeMemory
        self.toggleMemory = toggleMemory
        self.whatIsCaptured = whatIsCaptured
        self.openBrief = openBrief
        self.openTopic = openTopic
        self.startMeeting = startMeeting
        self.askAboutWindow = askAboutWindow
        self.openWhimbrel = openWhimbrel
        self.openSettings = openSettings
        self.quit = quit
    }
}

/// How tall the scrolling middle of the menu is. A pure rule so it can be tested: the menu is as tall as its content up to
/// `cap`; the Recording and Memory rows above and the footer below keep their height, and the middle takes what is left.
public enum WMenuLayout {
    /// The room the middle may use. Never negative, and never less than `minimum` so it stays a usable list even when the
    /// pinned parts are tall (the menu then runs past the cap, which only happens on a screen too small to matter).
    public static func middleHeight(ideal: CGFloat, pinnedTop: CGFloat, footer: CGFloat, cap: CGFloat?, minimum: CGFloat = 96) -> CGFloat {
        guard let cap else { return ideal }
        return min(ideal, max(minimum, cap - pinnedTop - footer))
    }
}

/// Arrow keys over the menu's controls. A pure rule so it can be tested: up and down move one step and stop at the
/// ends (a menu does not wrap), and with nothing focused down goes to the first control and up to the last.
public enum WMenuFocus {
    public enum Direction: Sendable { case up, down }

    /// Where focus is after the controls change (a recording starts or stops while the menu is open). A control that is still
    /// there keeps focus, so the menu does not jump; one that went away hands focus to `fallback`. Nothing focused stays so.
    public static func kept(_ current: String?, in order: [String], fallback: String?) -> String? {
        guard let current else { return nil }
        return order.contains(current) ? current : fallback
    }

    public static func move(from current: String?, in order: [String], _ direction: Direction) -> String? {
        guard !order.isEmpty else { return nil }
        guard let current, let index = order.firstIndex(of: current) else {
            return direction == .down ? order.first : order.last
        }
        switch direction {
        case .down: return order[min(index + 1, order.count - 1)]
        case .up: return order[max(index - 1, 0)]
        }
    }
}

/// The words of the recording section and the memory control.
public enum WMenuText {
    /// The state in visible words that say a recording is open: "Recording", "Recording paused", "Recording problem". The
    /// plain "Paused" and "Problem" of the indicator are not enough here, where memory can be paused too.
    public static func stateWord(_ state: WRecordingState, strings: WStrings) -> String {
        switch state {
        case .recording: return strings.recording
        case .paused: return strings.recordingPaused
        case .problem: return strings.menu.recordingProblem
        case .starting: return strings.menu.startingRecording
        case .resuming: return strings.menu.resumingRecording
        case .processing: return strings.processing
        case .off: return strings.off
        }
    }

    /// The meeting's title, in its own line. Nil when there is none or nothing records.
    public static func titleLine(_ recording: WMenuRecording) -> String? {
        guard let title = recording.title, !title.isEmpty else { return nil }
        switch recording.state {
        case .recording, .paused, .resuming, .starting, .problem: return title
        case .processing, .off: return nil
        }
    }

    /// What is wrong, for a problem: the app's reason, or a true line that claims nothing more. Nil for any other state, and
    /// when the app gave no reason but the streams line already says that no audio is being captured.
    public static func problemLine(_ recording: WMenuRecording, strings: WStrings) -> String? {
        guard recording.state.isProblem else { return nil }
        if let reason = recording.problemReason { return reason }
        if recording.streams?.isNeither == true { return nil }
        return strings.menu.problemUnknown
    }

    /// Why Stop is off while audio is being captured: the meeting is not saved yet. Nil in every other state.
    public static func stopWaitsLine(_ recording: WMenuRecording, strings: WStrings) -> String? {
        recording.stopWaitsForMeeting ? strings.menu.stopWaitsForMeeting : nil
    }

    /// Which streams are live, in plain words. Nil while paused or when the app does not know. A problem with neither says so.
    public static func streamsLine(_ recording: WMenuRecording, strings: WStrings) -> String? {
        guard recording.showsStreams, let streams = recording.streams else { return nil }
        switch (streams.microphone, streams.systemAudio) {
        case (true, true): return strings.menu.streamsBoth
        case (true, false): return strings.menu.streamsMicrophone
        case (false, true): return strings.menu.streamsSystemAudio
        case (false, false): return strings.menu.streamsNone
        }
    }

    /// The name a screen reader gives the state: "Recording problem, 14 minutes 8 seconds".
    public static func spokenState(_ recording: WMenuRecording, strings: WStrings) -> String {
        var parts = [stateWord(recording.state, strings: strings)]
        if let seconds = recording.elapsedSeconds { parts.append(WRecordingText.spoken(seconds: seconds, strings: strings)) }
        if let title = titleLine(recording) { parts.append(title) }
        return strings.joinDetails(parts)
    }

    /// The name a screen reader gives the memory control. A paused memory says "Memory", so it is not taken for a paused
    /// recording. The words on screen are the same.
    public static func memoryName(_ state: WMemoryState, strings: WStrings) -> String {
        switch state {
        case .pausedUntil(let time): return strings.menu.memoryPausedUntil(time)
        default: return state.title(using: strings)
        }
    }
}

// MARK: The small alert windows

/// A button on a small window.
public struct WAlertAction: Identifiable {
    public let id: String
    public var title: String
    public var spokenName: String?
    public var glyph: String?
    /// A longer line for the tooltip and the spoken hint, for a button whose name alone is not enough.
    public var help: String?
    /// Return runs this button even when nothing in the window has the keyboard. Only for an action that is safe to run by
    /// accident, one that never starts, resumes, stops or changes a recording. Every other button runs on Return only while
    /// it is the one that has focus.
    public var isSafeDefault: Bool
    public var run: () -> Void

    public init(
        id: String, title: String, spokenName: String? = nil, glyph: String? = nil, help: String? = nil, isSafeDefault: Bool = false,
        run: @escaping () -> Void
    ) {
        self.id = id
        self.title = title
        self.spokenName = spokenName
        self.glyph = glyph
        self.help = help
        self.isSafeDefault = isSafeDefault
        self.run = run
    }
}

/// "Recording starts in 8 s", with the way out beside it.
public struct WAlertCountdown {
    public var text: String
    public var cancel: WAlertAction?
    public init(text: String, cancel: WAlertAction? = nil) {
        self.text = text
        self.cancel = cancel
    }
}

/// The compact window: app mark and name, a title, one or two lines, an action row with the main action first.
public struct WAlertPanelModel {
    public var title: String
    /// The meeting or the subject, in a stronger weight under the title.
    public var subject: String?
    /// One or two plain lines.
    public var lines: [String]
    public var countdown: WAlertCountdown?
    /// The main action, first in the row. Return activates it only when `isSafeDefault` is set.
    public var main: WAlertAction
    /// The one alternate way to do the main action, shown as a small menu beside it.
    public var alternate: WAlertAction?
    public var others: [WAlertAction]

    public init(
        title: String, subject: String? = nil, lines: [String] = [], countdown: WAlertCountdown? = nil,
        main: WAlertAction, alternate: WAlertAction? = nil, others: [WAlertAction] = []
    ) {
        self.title = title
        self.subject = subject
        self.lines = lines
        self.countdown = countdown
        self.main = main
        self.alternate = alternate
        self.others = others
    }

    /// Every button in reading order: the main action first.
    public var actionIDs: [String] {
        var ids = [main.id]
        if let alternate { ids.append(alternate.id) }
        ids.append(contentsOf: others.map(\.id))
        return ids
    }
}

// MARK: The hotkey panel

public enum WHotkeyPhase: Equatable, Sendable {
    /// Reading the window in front.
    case reading
    /// Read; waiting for a question.
    case ready
    /// Cannot ask. The message says why; a button may offer a way out.
    case blocked(message: String, action: WHotkeyAction?)
    case answering(partial: String)
    case answered(String)
    case failed(String)
}

/// Which phase the panel is in, without what it carries. A streamed answer changes its text many times in one phase.
public enum WHotkeyPhaseKind: Equatable, Sendable { case reading, ready, blocked, answering, answered, failed }

extension WHotkeyPhase {
    public var kind: WHotkeyPhaseKind {
        switch self {
        case .reading: return .reading
        case .ready: return .ready
        case .blocked: return .blocked
        case .answering: return .answering
        case .answered: return .answered
        case .failed: return .failed
        }
    }
}

/// A button on a blocked panel. The app decides what it does by `id`.
public struct WHotkeyAction: Equatable, Sendable {
    public var id: String
    public var title: String
    public init(id: String, title: String) {
        self.id = id
        self.title = title
    }
}

public struct WHotkeyModel: Equatable, Sendable {
    public var phase: WHotkeyPhase
    /// "Reading: Safari, Pricing sheet". Shown before anything is sent.
    public var readingLine: String?
    /// Where the text will be sent. Shown before anything is sent.
    public var destinationLine: String?
    /// The text of the "reading" state, for example "Reading this window".
    public var readingProgress: String
    public var copyTitle: String
    public var copiedTitle: String

    public init(
        phase: WHotkeyPhase, readingLine: String? = nil, destinationLine: String? = nil, readingProgress: String,
        copyTitle: String, copiedTitle: String
    ) {
        self.phase = phase
        self.readingLine = readingLine
        self.destinationLine = destinationLine
        self.readingProgress = readingProgress
        self.copyTitle = copyTitle
        self.copiedTitle = copiedTitle
    }
}

public struct WHotkeyActions {
    public var submit: (String) -> Void
    public var close: () -> Void
    public var copy: () -> Void
    public var perform: (String) -> Void
    public var questionChanged: (String) -> Void

    public init(
        submit: @escaping (String) -> Void = { _ in }, close: @escaping () -> Void = {}, copy: @escaping () -> Void = {},
        perform: @escaping (String) -> Void = { _ in }, questionChanged: @escaping (String) -> Void = { _ in }
    ) {
        self.submit = submit
        self.close = close
        self.copy = copy
        self.perform = perform
        self.questionChanged = questionChanged
    }
}
