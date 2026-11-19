import AppKit
import SwiftUI
import WhimbrelUI

/// Turns the app's recording state into the plain values the live meeting screen and the in-meeting window draw.
/// Everything here is pure, so tests can check it without an `AppState`.
enum LiveMeetingAdapter {
    static func state(
        isProcessing: Bool, stage: String?, isStarting: Bool, pause: RecordingPauseModel?, resuming: Bool, problem: String?
    ) -> WLiveState {
        if isProcessing { return .processing(stage: stage) }
        if isStarting { return .starting }
        guard let pause else { return .recording }
        switch pause.state {
        case .recording: return .recording
        case .paused: return resuming ? .resuming : .paused
        case .problem: return .problem(problem ?? "")
        case .stopped: return .processing(stage: stage)
        }
    }

    /// Clock time since the meeting started minus time paused.
    static func recordedSeconds(now: Date, startedAt: Date?, pause: RecordingPauseModel?) -> Int {
        guard let startedAt else { return pause?.recordedSeconds(now: now) ?? 0 }
        let paused = pause?.pausedSeconds(now: now) ?? 0
        return max(0, Int((now.timeIntervalSince(startedAt) - paused).rounded(.down)))
    }

    static func timeOfDay(_ date: Date, locale: Locale = .current) -> String {
        date.formatted(Date.FormatStyle(date: .omitted, time: .shortened, locale: locale))
    }

    static func marks(_ markers: [CoreMeetingMarker], locale: Locale = .current) -> [WLiveMark] {
        markers.map { marker in
            WLiveMark(
                id: String(marker.id),
                timeOfDay: timeOfDay(Date(timeIntervalSince1970: Double(marker.createdUnixMs) / 1000), locale: locale),
                label: marker.label ?? "")
        }
    }

    /// The live lines without any that fall in a paused stretch, as transcript entries timed by the clock.
    static func entries(
        segments: [CoreLiveSegment], spans: [RecordingPausedSpan], recordingStartedAt: Date?, lastLine: WTranscriptLineState, you: String,
        locale: Locale = .current
    ) -> [WTranscriptEntry] {
        let kept = RecordingPausedSpans.dropping(segments, spans: spans).suffix(20)
        let start = recordingStartedAt ?? Date()
        let rows = Array(kept)
        return rows.enumerated().map { index, row in
            let at = start.addingTimeInterval(Double(row.startCs) / 100)
            let name = row.speakerLabel == TranscriptionService.Speaker.me.rawValue ? you : row.speakerLabel
            return WTranscriptEntry(
                time: timeOfDay(at, locale: locale), speaker: WPerson(name: name), text: row.text,
                state: index == rows.count - 1 ? lastLine : .normal)
        }
    }

    /// What the newest line says under it: Listening while recording, Paused or Resuming otherwise, nothing when not live.
    static func lastLineState(_ state: WLiveState, listening: Bool) -> WTranscriptLineState {
        switch state {
        case .paused: return .paused
        case .resuming: return .resuming
        case .recording: return listening ? .live : .normal
        default: return .normal
        }
    }

    /// The short time line: the next meeting above, now, and the time recording started.
    static func stops(next: (time: Date, title: String)?, now: Date, startedAt: Date?, started: String, nowWord: String) -> [WLiveStop] {
        var out: [WLiveStop] = []
        if let next { out.append(WLiveStop(id: "next", time: timeOfDay(next.time), caption: next.title, kind: .next)) }
        out.append(WLiveStop(id: "now", time: timeOfDay(now), caption: nowWord, kind: .now))
        if let startedAt { out.append(WLiveStop(id: "started", time: timeOfDay(startedAt), caption: started, kind: .started)) }
        return out
    }

    /// "Today, 10:00 AM. Sam Okafor, Priya Shah." The date and the people the meeting has.
    static func detail(start: Date, attendees: [String], now: Date) -> String {
        let when = Calendar.current.isDate(start, inSameDayAs: now)
            ? String(localized: "Today, \(timeOfDay(start))", comment: "Under the title of the live meeting: today and the start time of the meeting.")
            : start.formatted(date: .abbreviated, time: .shortened)
        let people = attendees.joined(separator: ", ")
        return people.isEmpty ? when + "." : "\(when). \(people)."
    }

    /// The last non-empty line of the notes, for a marked moment's label.
    static func lastLine(of notes: String) -> String {
        notes.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespaces) }.last { !$0.isEmpty } ?? ""
    }
}

/// The live meeting in the redesign. The existing notes editor and autosave, the live alerts and the meeting ask go in as
/// slots (restyled through the theme; their logic is untouched). Takes Home's place while a meeting records.
struct LiveMeetingRedesignScreen: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject var audio: AudioCaptureService
    @ObservedObject var live: LiveTranscriptionService
    let model: ActiveMeetingModel
    let meeting: CoreMeeting
    let initialNotes: String
    let nextMeeting: (time: Date, title: String)?
    @StateObject private var autosave: ActiveNoteAutosaveController
    @State private var noticeCopied = false

    init(
        appState: AppState, model: ActiveMeetingModel, meeting: CoreMeeting, initialNotes: String,
        nextMeeting: (time: Date, title: String)?, onSaveNotes: @escaping (String, Int64) -> Bool
    ) {
        self.audio = appState.audio
        self.live = appState.liveTranscript
        self.model = model
        self.meeting = meeting
        self.initialNotes = initialNotes
        self.nextMeeting = nextMeeting
        _autosave = StateObject(wrappedValue: ActiveNoteAutosaveController(onSave: onSaveNotes))
    }

    var body: some View {
        SwiftUI.TimelineView(.periodic(from: .now, by: 1)) { context in
            let strings = WhimbrelUIStrings.make()
            let liveModel = liveModel(now: context.date, strings: strings)
            WLiveMeetingView(
                model: liveModel, actions: actions,
                warnings: { warnings },
                editor: { editor },
                alerts: {
                    // The existing alerts and ask, without a second transcript.
                    if appState.liveTranscriptSettings.enabled { LiveMeetingSection(live: live, meeting: meeting, showsTranscript: false) }
                },
                ask: { EmptyView() }
            )
            .redesignThemed()
        }
        .onAppear {
            autosave.seed(initialNotes)
            appState.liveScreenShowing = true
        }
        .onDisappear {
            autosave.flush()
            appState.liveScreenShowing = false
        }
        .onChange(of: autosave.noteText) { _, text in appState.liveNotesLastLine = LiveMeetingAdapter.lastLine(of: text) }
        .onChange(of: appState.liveMarks.count) { old, new in
            guard new > old, let last = appState.liveMarks.last else { return }
            let time = LiveMeetingAdapter.timeOfDay(Date(timeIntervalSince1970: Double(last.createdUnixMs) / 1000))
            AccessibilityNotification.Announcement(WhimbrelUIStrings.make().live.momentMarked(time)).post()
        }
    }

    private func liveModel(now: Date, strings: WStrings) -> WLiveMeetingModel {
        let pause = appState.recordingPause
        let state = LiveMeetingAdapter.state(
            isProcessing: model.isProcessing, stage: model.processingStage, isStarting: false, pause: pause,
            resuming: appState.isResumingRecording, problem: appState.recordingProblem)
        let transcriptOn = appState.liveTranscriptSettings.enabled
        return WLiveMeetingModel(
            title: meeting.title,
            detail: LiveMeetingAdapter.detail(start: Date(timeIntervalSince1970: Double(meeting.startUnixMs) / 1000), attendees: meeting.attendees, now: now),
            state: state,
            recordedSeconds: LiveMeetingAdapter.recordedSeconds(now: now, startedAt: appState.activeMeetingStartedAt, pause: pause),
            level: audio.inputLevel,
            showsNotice: appState.recordingNoticeSettings.enabled && appState.recordingNoticeSettings.dismissedMeetingId != meeting.id,
            noticeCopied: noticeCopied,
            marks: LiveMeetingAdapter.marks(appState.liveMarks),
            stops: LiveMeetingAdapter.stops(
                next: nextMeeting, now: now, startedAt: appState.activeMeetingStartedAt, started: strings.live.started, nowWord: strings.now),
            showsTranscript: transcriptOn,
            transcript: LiveMeetingAdapter.entries(
                segments: live.segments, spans: pause?.spans(closingAt: now) ?? [], recordingStartedAt: audio.recordingStartedAt,
                lastLine: LiveMeetingAdapter.lastLineState(state, listening: live.status == .listening), you: String(localized: "You", comment: "Speaker name for the person using this Mac in the live transcript.")),
            transcriptAccessory: state == .paused ? strings.paused : (state == .resuming ? strings.resuming : (live.status == .listening ? strings.listening : nil)))
    }

    private var actions: WLiveActions {
        WLiveActions(
            pause: { appState.pauseRecording() }, resume: { appState.resumeRecording() },
            stop: { appState.endActiveMeetingIfNeeded() },
            mark: { appState.markMoment() },
            removeMark: { if let id = Int64($0) { appState.removeMarker(id) } },
            copyNotice: {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(appState.recordingNoticeSettings.text, forType: .string)
                noticeCopied = true
            },
            dismissNotice: { appState.recordingNoticeSettings.dismissedMeetingId = meeting.id })
    }

    /// The existing warnings keep their place and are never hidden: the silence warning and a degraded audio reason.
    @ViewBuilder
    private var warnings: some View {
        VStack(spacing: 8) {
            SilenceWarningBanner()
            if let reason = audio.degradedReason ?? audio.micDegradedReason {
                Text(reason)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("live-degraded-reason")
            }
        }
    }

    private var editor: some View {
        TextEditor(text: $autosave.noteText)
            .font(.whimbrelBody(.body))
            .foregroundStyle(Color.whimbrelTextPrimary)
            .scrollContentBackground(.hidden)
            .accessibilityIdentifier("active-meeting-note-editor")
    }
}
