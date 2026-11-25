import AppKit
import Combine
import SwiftUI
import WhimbrelUI

/// Meetings in the redesigned Settings window. This file feeds the package's `WMeetingsSettingsView` from the app's state
/// and calls the same functions the classic panes call (`MeetingsSettingsPane`, `TranscriptionSettingsPane`). The classic
/// panes are not changed. The recording notice and the consent reminder are the first card, at full size.

// MARK: Adapter (pure, tested)

enum MeetingsAdapter {
    /// The "Ask if I'm still recording after" choices, in the classic order and words.
    static func checkInOptions(hours allowed: [Int] = RecordingCheckInSettings.allowedHours) -> [WChoice] {
        allowed.map { hours in
            WChoice(id: String(hours), title: hours == 0 ? String(localized: "Off") : hours == 1 ? String(localized: "1 hour") : String(localized: "\(hours) hours"))
        }
    }

    static func qualityOptions() -> [WChoice] {
        [
            WChoice(id: TranscriptionQuality.automatic.rawValue, title: String(localized: "Automatic")),
            WChoice(id: TranscriptionQuality.better.rawValue, title: String(localized: "Better")),
            WChoice(id: TranscriptionQuality.fast.rawValue, title: String(localized: "Fast")),
        ]
    }

    /// The line under the Quality picker; the classic pane's three sentences.
    static func qualityDetail(_ quality: TranscriptionQuality) -> String {
        switch quality {
        case .automatic: return String(localized: "The best engine this computer runs well.")
        case .better: return String(localized: "The most accurate model, a download of about 574 MB. Slower on older computers.")
        case .fast: return String(localized: "The small model. Quickest, least accurate.")
        }
    }

    /// The classic pane shows a progress line while downloading and a retry button after a failure.
    static func download(_ phase: ModelBootstrap.Phase) -> (state: WModelDownloadState, text: String) {
        switch phase {
        case .downloading(let fraction, _):
            return (.downloading(fraction: fraction), String(localized: "Downloading the better transcription model…"))
        case .failed: return (.failed, "")
        case .idle, .ready: return (.none, "")
        }
    }

    /// The standard reminder wording, read from `RecordingNoticeSettings.text` (its one source) with nothing custom set. The
    /// settings object is built on a private defaults suite that is only read, never written, and holds no custom text.
    @MainActor
    static var defaultNoticeText: String {
        RecordingNoticeSettings(defaults: UserDefaults(suiteName: "whimbrel.settings.default-notice-text")!).text
    }

    /// Whether the "Alert me" switch shows on: the alert is on only while there is a name to listen for.
    static func liveQuestionOn(hasNames: Bool, enabled: Bool) -> Bool { hasNames && enabled }

    /// The next quality choice that starts the download: choosing Better, from anything else.
    static func startsDownload(from old: TranscriptionQuality, to new: TranscriptionQuality) -> Bool { new == .better && old != .better }

    /// Whether turning "other languages" on asks for the models first.
    static func asksForModels(turnedOn: Bool, allModelsPresent: Bool) -> Bool { turnedOn && !allModelsPresent }

    /// Every sentence the redesigned section writes of its own, for the copy tests.
    static var ownStrings: [String] {
        let l = labels()
        return [
            l.consentNote, l.noticeTextHelp("x"), l.qualifyingNote, l.heuristicNote, l.checkInNote, l.awayNote, l.awayZoomSetup, l.awayConsent,
            l.awayComingSoonNote, l.liveTranscriptNote, l.snapshotsNote, l.screenRecordingOff, l.questionsNote, l.addYourNameFirst,
            l.rememberVoicesNote, l.deleteVoiceQuestion("x"), l.calendarsNote, l.noCalendars, l.downloadFailed, l.modelFileNote, l.multilingualNote,
            l.diarizationNote, l.fileMissing("x"), l.calendarsHeading, l.noticeHeading,
        ]
    }

    static func labels() -> WMeetingsLabels {
        var l = WMeetingsLabels()
        l.noticeHeading = String(localized: "Recording notice", comment: "Heading of the Settings card with the reminder to tell people you are recording.")
        l.pathLocation = String(localized: "Location", comment: "Label of the line that shows where a model file is, under the row that names the file.")
        l.consentNote = String(localized: "Some places require everyone's consent before a meeting is recorded. Check the rules where you and the other people are.")
        l.noticeToggle = String(localized: "Remind me to tell people I'm recording")
        l.noticeTextLabel = String(localized: "Your own wording", comment: "Label of the field where you write your own recording reminder in Settings.")
        l.noticeTextHelp = { standard in String(localized: "Leave this blank to use the standard reminder: \(standard)", comment: "Help under the recording reminder field in Settings. The placeholder is the standard sentence.") }
        l.meetingsHeading = String(localized: "Meetings")
        l.autoStart = String(localized: "Auto-start recording for calendar meetings")
        l.qualifyingNote = String(localized: "A qualifying calendar meeting has a conferencing link (Zoom/Meet/Teams/Webex) or at least one other attendee. Upcoming meetings and the recording prompt itself are shown in the main window, not here.")
        l.leadTime = String(localized: "Prompt lead time", comment: "Label of the Settings row that sets how many minutes before a calendar meeting Whimbrel offers to record.")
        l.minutes = { n in String(localized: "\(n) min") }
        l.heuristic = String(localized: "Detect meetings with no calendar entry")
        l.heuristicNote = String(localized: "Prompts when a known meeting app (Zoom, Teams, Slack, Webex, FaceTime) is running with the mic and system audio both active, for ad-hoc calls with nothing on your calendar. Uses the Auto-start setting above too. Doesn't cover a meeting joined from a browser tab (such as Meet) yet.", comment: "Help under the switch that detects meetings with no calendar entry.")
        l.checkIn = String(localized: "Ask if I'm still recording after")
        l.checkInNote = String(localized: "Shows a \"Still recording?\" prompt. With no answer in 5 minutes, the recording stops and is saved. Recordings always stop at 6 hours or when less than 2 GB is free. Applies from the next recording.")
        l.awayMode = String(localized: "Away Mode")
        l.awayNote = String(localized: "While this is on, Whimbrel joins your Zoom meetings for you, muted with the camera off, records them, and posts a short note in the meeting chat. Keep this Mac unlocked and signed in to Zoom.")
        l.awayZoomSetup = String(localized: "In Zoom > Settings > Audio, turn on \"Automatically join audio by computer\" and \"Always mute my microphone when joining\". In Zoom > Settings > Video, turn on \"Turn off my video when joining\" and turn off \"Always show video preview dialog\".")
        l.awayConsent = String(localized: "Some places require everyone's consent before a meeting is recorded. The chat note tells people, but it isn't consent. Check the rules where you and the people you meet are.")
        l.awayComingSoonNote = String(localized: "Whimbrel will join the Zoom meetings you can't make, record them and post a short note in the chat.")
        l.betaComingSoon = String(localized: "Beta coming soon")
        l.liveTranscript = String(localized: "Live transcript during meetings")
        l.liveTranscriptNote = String(localized: "Transcribes while you record so you can read along and ask questions during the call. Uses extra CPU while recording and pauses in Low Power Mode. The full transcript is still made after the meeting and replaces the live one. Question alerts also use it.")
        l.snapshots = String(localized: "Save slides and shared screens")
        l.snapshotsNote = String(localized: "Saves one image per slide shown in recorded meetings to the media folder (set in Retention). Needs Screen Recording permission.", comment: "Help under the switch that saves slides and shared screens.")
        l.screenRecordingOff = String(localized: "Screen Recording permission is off.", comment: "Added to the slides help when the Screen Recording permission is not granted.")
        l.readingSize = String(localized: "Transcript and summary reading size", comment: "Label of the Settings row that sets the text size of transcripts and summaries.")
        l.points = { n in String(localized: "\(n) pt") }
        l.questionsHeading = String(localized: "Question alerts", comment: "Heading of the Settings card about alerts when someone asks you something in a meeting.")
        l.questionsNote = String(localized: "Lines that mention you, and the minutes before them, go to your Ask model. Alert emails go only to your own address. While alerts are on, the live transcript keeps running in meetings even when \"Live transcript during meetings\" is off.")
        l.liveQuestion = String(localized: "Alert me when someone asks me something")
        l.addYourNameFirst = String(localized: "Add your name in Settings, Profile first.", comment: "Shown on the question alerts switch when the Profile has no name. Settings, Profile is the path to the Profile section.")
        l.alsoEmail = String(localized: "Also email me")
        l.otherNames = String(localized: "Other names people call you")
        l.voicesHeading = String(localized: "Voices", comment: "Heading of the Settings card about remembering people's voices.")
        l.rememberVoices = String(localized: "Remember people's voices")
        l.rememberVoicesNote = String(localized: "Whimbrel learns a voice when you confirm or fix a name. Voices stay on your devices and in your encrypted sync.")
        l.rememberedVoices = String(localized: "Remembered voices", comment: "Title of the sheet that lists the voices Whimbrel remembers.")
        l.rememberedVoicesButton = String(localized: "Remembered voices…")
        l.noVoices = String(localized: "No remembered voices yet.")
        l.deleteVoice = String(localized: "Delete")
        l.deleteVoiceQuestion = { name in String(localized: "Delete the remembered voice of \(name)?", comment: "Question before a remembered voice is deleted. The placeholder is the person's name.") }
        l.done = String(localized: "Done")
        l.calendarsHeading = String(localized: "Always treat as meetings", comment: "Heading of the Settings card that lists calendars whose events always count as meetings.")
        l.calendarsNote = String(localized: "Some work Exchange/CalDAV accounts don't sync a conferencing link or attendee list at all, so those events never qualify above. Opt a calendar in here and every non-all-day event on it counts as a meeting.")
        l.noCalendars = String(localized: "No calendars found. Grant Calendar access first.", comment: "Shown in the Always treat as meetings card when Whimbrel can see no calendars.")
        l.transcriptionHeading = String(localized: "Transcription")
        l.quality = String(localized: "Quality")
        l.downloadFailed = String(localized: "The better transcription model did not download.", comment: "Shown in Settings when the larger transcription model failed to download, above a Try again button.")
        l.tryAgain = String(localized: "Try again")
        l.modelFileNote = String(localized: "Whisper.cpp GGML model used to transcribe captured meeting audio. A file you choose here is used in place of the Quality setting.")
        l.chooseModelFile = String(localized: "Choose Model File…")
        l.multilingual = String(localized: "I have meetings in other languages")
        l.multilingualNote = String(localized: "Whimbrel detects each meeting's language and transcribes it as spoken, using a larger multilingual model. Off, every meeting is transcribed as English.")
        l.diarizationHeading = String(localized: "Speaker Diarization")
        l.diarizationNote = String(localized: "Real per-speaker labels for the system-audio channel. Whimbrel can fetch these two models automatically via the first-run \"Set Up Models\" prompt, or see core/diarize/README.md to fetch them manually. If either is missing, transcripts fall back to a single \"Others\" label, exactly as before this feature existed.")
        l.chooseSegmentation = String(localized: "Choose Segmentation Model…")
        l.chooseEmbedding = String(localized: "Choose Speaker-Embedding Model…")
        l.fileMissing = { label in String(localized: "\(label): the file is not there.", comment: "Shown in Settings under a model file path when the file does not exist. The placeholder is the name of the model.") }
        return l
    }
}

// MARK: Runtime state

/// What the classic panes keep in `@State`, with the same calls.
@MainActor
final class MeetingsRuntime: ObservableObject {
    unowned let appState: AppState
    init(appState: AppState) { self.appState = appState }

    @Published var voices: [CoreVoicePerson] = []
    @Published var modelPath: URL = TranscriptionService.modelPath
    @Published var statusMessage: String?
    @Published var segmentationModelPath: URL = DiarizationService.segmentationModelPath
    @Published var embeddingModelPath: URL = DiarizationService.embeddingModelPath
    @Published var diarizationStatusMessage: String?
    @Published var multilingual = AppLanguage.multilingual()
    @Published var showModelSetup = false

    func loadVoices() { voices = appState.coreStore?.voicePeople() ?? [] }

    func forgetVoice(_ key: String) {
        voices = appState.coreStore?.forgetVoice(personKey: key) ?? voices
    }

    func setMultilingual(_ on: Bool) {
        multilingual = on
        UserDefaults.standard.set(on, forKey: AppLanguage.multilingualKey)
        if MeetingsAdapter.asksForModels(turnedOn: on, allModelsPresent: ModelDownloadService.allModelsPresent) { showModelSetup = true }
    }

    private func chooseFile(message: String) -> URL? {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = String(localized: "Choose")
        panel.message = message
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url
    }

    func chooseModel() {
        guard let url = chooseFile(message: String(localized: "Choose a GGML whisper model (.bin)")) else { return }
        TranscriptionService.modelPath = url
        modelPath = url
        statusMessage = String(localized: "Model set to \(url.lastPathComponent).")
    }

    func chooseSegmentationModel() {
        guard let url = chooseFile(message: String(localized: "Choose the pyannote segmentation ONNX model")) else { return }
        DiarizationService.segmentationModelPath = url
        segmentationModelPath = url
        diarizationStatusMessage = String(localized: "Segmentation model set to \(url.lastPathComponent).")
    }

    func chooseEmbeddingModel() {
        guard let url = chooseFile(message: String(localized: "Choose the speaker-embedding ONNX model")) else { return }
        DiarizationService.embeddingModelPath = url
        embeddingModelPath = url
        diarizationStatusMessage = String(localized: "Speaker-embedding model set to \(url.lastPathComponent).")
    }
}

// MARK: The view

struct MeetingsSettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let highlighted: String?

    var body: some View {
        MeetingsSettingsRedesignBody(appState: appState, highlighted: highlighted)
    }
}

private struct MeetingsSettingsRedesignBody: View {
    let appState: AppState
    let highlighted: String?
    @StateObject private var runtime: MeetingsRuntime
    // Not forwarded by AppState's own change publisher, so observed here: the switches are drawn from values.
    @ObservedObject private var notice: RecordingNoticeSettings
    @ObservedObject private var speakerNames: SpeakerNamesSettings
    @ObservedObject private var turbo: ModelBootstrap
    @AppStorage(TranscriptionQuality.key) private var quality = TranscriptionQuality.automatic

    init(appState: AppState, highlighted: String?) {
        self.appState = appState
        self.highlighted = highlighted
        _runtime = StateObject(wrappedValue: MeetingsRuntime(appState: appState))
        _notice = ObservedObject(wrappedValue: appState.recordingNoticeSettings)
        _speakerNames = ObservedObject(wrappedValue: appState.speakerNamesSettings)
        _turbo = ObservedObject(wrappedValue: appState.turboDownload)
    }

    var body: some View {
        WMeetingsSettingsView(model: model(), actions: actions(), highlighted: highlighted)
            .sheet(isPresented: $runtime.showModelSetup) {
                RedesignSwitch(enabled: appState.redesignEnabled) { ModelSetupRedesignView() } old: { ModelSetupView() }
            }
    }

    private func path(_ label: String, _ url: URL) -> WPathModel {
        WPathModel(label: label, path: url.path, exists: FileManager.default.fileExists(atPath: url.path))
    }

    private func model() -> WMeetingsModel {
        let hasNames = !appState.profile.liveQuestionNames.isEmpty
        let download = MeetingsAdapter.download(turbo.phase)
        return WMeetingsModel(
            labels: MeetingsAdapter.labels(),
            noticeEnabled: notice.enabled, noticeCustomText: notice.customText,
            noticeDefaultText: MeetingsAdapter.defaultNoticeText,
            autoStart: appState.meetingTrigger.autoStartEnabled, leadMinutes: Int(appState.meetingTrigger.leadMinutes),
            heuristic: appState.meetingTrigger.heuristicDetectionEnabled, checkInHours: appState.recordingCheckInSettings.hours,
            checkInOptions: MeetingsAdapter.checkInOptions(),
            away: AppState.awayZoomJoinEnabled ? .available(enabled: appState.awayMode.enabled) : .comingSoon,
            liveTranscript: appState.liveTranscriptSettings.enabled, snapshots: appState.snapshotSettings.enabled,
            screenRecordingGranted: appState.permissions.screenRecordingGranted, readingSize: Int(appState.readingFontSize),
            hasNames: hasNames, liveQuestion: MeetingsAdapter.liveQuestionOn(hasNames: hasNames, enabled: appState.liveQuestionSettings.enabled),
            liveQuestionEmail: appState.liveQuestionSettings.emailEnabled, otherNames: appState.profile.otherNames,
            rememberVoices: speakerNames.rememberVoices,
            voices: runtime.voices.map { WVoicePerson(id: $0.personKey, name: $0.name) },
            calendars: appState.calendar.availableCalendars.map {
                WCalendarChoice(id: $0.id, title: $0.title, isOn: appState.calendar.alwaysQualifyCalendarIDs.contains($0.id))
            },
            transcription: WTranscriptionModel(
                qualityOptions: MeetingsAdapter.qualityOptions(), quality: quality.rawValue, qualityDetail: MeetingsAdapter.qualityDetail(quality),
                download: download.state, downloadText: download.text, modelFile: path(String(localized: "Model file"), runtime.modelPath),
                multilingual: runtime.multilingual, statusMessage: runtime.statusMessage,
                segmentation: path(String(localized: "Segmentation model"), runtime.segmentationModelPath),
                embedding: path(String(localized: "Speaker-embedding model"), runtime.embeddingModelPath),
                diarizationStatus: runtime.diarizationStatusMessage),
            anchors: SettingsAnchorID.meetings)
    }

    private func actions() -> WMeetingsActions {
        WMeetingsActions(
            setNoticeEnabled: { notice.enabled = $0 }, setNoticeText: { notice.customText = $0 },
            setAutoStart: { appState.meetingTrigger.autoStartEnabled = $0 }, setLeadMinutes: { appState.meetingTrigger.leadMinutes = Double($0) },
            setHeuristic: { appState.meetingTrigger.heuristicDetectionEnabled = $0 },
            setCheckInHours: { appState.recordingCheckInSettings.hours = $0 }, setAway: { appState.awayMode.enabled = $0 },
            setLiveTranscript: { appState.liveTranscriptSettings.enabled = $0 }, setSnapshots: { appState.snapshotSettings.enabled = $0 },
            setReadingSize: { appState.readingFontSize = Double($0) },
            setLiveQuestion: { appState.liveQuestionSettings.enabled = $0 }, setLiveQuestionEmail: { appState.liveQuestionSettings.emailEnabled = $0 },
            setOtherNames: { appState.profile.otherNames = $0 }, setRememberVoices: { speakerNames.rememberVoices = $0 },
            voicesOpened: { runtime.loadVoices() }, forgetVoice: { runtime.forgetVoice($0) },
            setCalendar: { id, on in
                if on { appState.calendar.alwaysQualifyCalendarIDs.insert(id) } else { appState.calendar.alwaysQualifyCalendarIDs.remove(id) }
            },
            setQuality: { id in
                let chosen = TranscriptionQuality(rawValue: id) ?? .automatic
                let old = quality
                quality = chosen
                if MeetingsAdapter.startsDownload(from: old, to: chosen) { Task { await appState.turboDownload.start() } }
            },
            retryDownload: { Task { await appState.turboDownload.start() } },
            chooseModelFile: { runtime.chooseModel() }, setMultilingual: { runtime.setMultilingual($0) },
            chooseSegmentation: { runtime.chooseSegmentationModel() }, chooseEmbedding: { runtime.chooseEmbeddingModel() })
    }
}
