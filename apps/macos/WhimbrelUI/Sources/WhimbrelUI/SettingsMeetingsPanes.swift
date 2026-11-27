import SwiftUI

// The Meetings section drawn on the settings parts: the recording notice and consent reminder first and at full size,
// then starting and stopping, question alerts, voices, calendars and Transcription. The app builds the model from its
// state and passes the actions; nothing here reads the app.

// MARK: Shared small views

/// A sentence under a row, with an optional button. An error draws as an error line.
struct WNoteRow: View {
    let line: WSettingNote
    let onAction: () -> Void

    var body: some View {
        let button: AnyView = {
            guard let title = line.buttonTitle else { return AnyView(EmptyView()) }
            return AnyView(WButton(title, kind: .secondary, action: onAction))
        }()
        switch line.tone {
        case .problem:
            WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: line.text) { button }
        case .warning:
            WSettingsRowFrame(label: line.text, detail: nil, controlSpeaksLabel: false) { button }
        case .info:
            WSettingsRowFrame(label: line.text, detail: nil, controlSpeaksLabel: false) { button }
        }
    }
}

/// A plain sentence in a card: secondary text, with no label above it (a row-label look read as a title with no control).
func wNote(_ text: String) -> some View {
    WSettingsRowFrame(label: "", detail: text, controlSpeaksLabel: false) { EmptyView() }
}

/// "12 models available" or an error, with Refresh Models.
struct WModelStatusRow: View {
    let status: WModelListStatus
    let refreshTitle: String
    let onRefresh: () -> Void

    var body: some View {
        WSettingsRowFrame(
            label: status.isError ? "" : status.text, detail: nil, controlSpeaksLabel: false, errorText: status.isError ? status.text : nil
        ) {
            WButton(refreshTitle, kind: .secondary, action: onRefresh).disabled(status.isLoading)
        }
    }
}

func wOptions(_ choices: [WChoice]) -> [WSettingsOption<String>] { choices.map { WSettingsOption(id: $0.id, title: $0.title) } }

// MARK: Meetings

public struct WMeetingsSettingsView: View {
    @Environment(\.wTheme) private var theme
    @State private var showVoices = false
    let model: WMeetingsModel
    let actions: WMeetingsActions
    let highlighted: String?

    public init(model: WMeetingsModel, actions: WMeetingsActions, highlighted: String? = nil, initiallyShowingVoices: Bool = false) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
        _showVoices = State(initialValue: initiallyShowingVoices)
    }

    private func on(_ id: String) -> Bool { highlighted == id }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(16)) {
            noticeCard
            meetingsCard
            questionsCard
            voicesCard
            calendarsCard
            transcriptionCard
            diarizationCard
        }
        .sheet(isPresented: $showVoices) {
            WRememberedVoicesSheet(model: model, actions: actions, onDone: { showVoices = false })
        }
    }

    // MARK: Recording notice and consent (full size, never in a disclosure)

    private var noticeCard: some View {
        let l = model.labels
        let a = model.anchors
        return WSettingsCard(heading: l.noticeHeading) {
            ForEach(Array(model.noticeRows.enumerated()), id: \.offset) { _, row in
                switch row {
                case .consent(let text):
                    // A plain row at full size, on or off: the consent sentence is never a muted card description.
                    wNote(text)
                case .toggle:
                    WSettingsRow(
                        label: l.noticeToggle, isOn: model.noticeEnabled, highlighted: on(a.recordingNotice), onChange: actions.setNoticeEnabled
                    )
                    .wAnchor(a.recordingNotice)
                case .customText:
                    WSettingsTextEditorRow(
                        label: l.noticeTextLabel, description: l.noticeTextHelp(model.noticeDefaultText),
                        text: Binding(get: { model.noticeCustomText }, set: actions.setNoticeText), minHeight: 64)
                }
            }
        }
    }

    // MARK: Meetings

    private var meetingsCard: some View {
        let l = model.labels
        let a = model.anchors
        return WSettingsCard(heading: l.meetingsHeading) {
            WSettingsRow(
                label: l.autoStart, description: l.qualifyingNote, isOn: model.autoStart, highlighted: on(a.autoStart), onChange: actions.setAutoStart
            )
            .wAnchor(a.autoStart)
            WSettingsNumberRow(
                label: l.leadTime, value: model.leadMinutes, range: 1...10, unit: l.minutes, onChange: actions.setLeadMinutes)
            WSettingsRow(
                label: l.heuristic, description: l.heuristicNote, isOn: model.heuristic, highlighted: on(a.heuristic), onChange: actions.setHeuristic
            )
            .wAnchor(a.heuristic)
            WSettingsPickerRow<String>(
                label: l.checkIn, description: l.checkInNote, options: wOptions(model.checkInOptions), selection: String(model.checkInHours),
                style: .menu, onSelect: { actions.setCheckInHours(Int($0) ?? model.checkInHours) })
            awayRows
            WSettingsRow(
                label: l.liveTranscript, description: l.liveTranscriptNote, isOn: model.liveTranscript,
                highlighted: on(a.liveTranscript), onChange: actions.setLiveTranscript
            )
            .wAnchor(a.liveTranscript)
            WSettingsRow(
                label: l.snapshots, description: l.snapshotsNote + (model.screenRecordingGranted ? "" : " " + l.screenRecordingOff),
                isOn: model.snapshots, highlighted: on(a.snapshots), onChange: actions.setSnapshots
            )
            .wAnchor(a.snapshots)
            WSettingsNumberRow(label: l.readingSize, value: model.readingSize, range: 11...20, unit: l.points, onChange: actions.setReadingSize)
        }
    }

    @ViewBuilder
    private var awayRows: some View {
        let l = model.labels
        switch model.away {
        case .available(let enabled):
            WSettingsRow(label: l.awayMode, description: l.awayNote, isOn: enabled, onChange: actions.setAway)
            wNote(l.awayZoomSetup)
            wNote(l.awayConsent)
        case .comingSoon:
            WSettingsRow(label: l.awayMode, description: l.awayComingSoonNote) { WStatusChip(l.betaComingSoon, tone: .neutral) }
        }
    }

    // MARK: Question alerts

    private var questionsCard: some View {
        let l = model.labels
        return WSettingsCard(heading: l.questionsHeading, description: l.questionsNote) {
            WSettingsRow(
                label: l.liveQuestion, isOn: model.liveQuestion, isEnabled: model.hasNames,
                disabledReason: model.hasNames ? nil : l.addYourNameFirst, onChange: actions.setLiveQuestion)
            if model.hasNames && model.liveQuestion {
                WSettingsRow(label: l.alsoEmail, isOn: model.liveQuestionEmail, onChange: actions.setLiveQuestionEmail)
            }
            WSettingsTextFieldRow(
                label: l.otherNames, text: Binding(get: { model.otherNames }, set: actions.setOtherNames))
        }
    }

    // MARK: Voices

    private var voicesCard: some View {
        let l = model.labels
        let a = model.anchors
        return WSettingsCard(heading: l.voicesHeading) {
            WSettingsRow(
                label: l.rememberVoices, description: l.rememberVoicesNote, isOn: model.rememberVoices,
                highlighted: on(a.voices), onChange: actions.setRememberVoices
            )
            .wAnchor(a.voices)
            WSettingsRow(label: l.rememberedVoices, buttonTitle: l.rememberedVoicesButton) {
                actions.voicesOpened()
                showVoices = true
            }
        }
    }

    // MARK: Always treat as meetings

    private var calendarsCard: some View {
        let l = model.labels
        return WSettingsCard(heading: l.calendarsHeading, description: l.calendarsNote) {
            if model.calendars.isEmpty {
                wNote(l.noCalendars)
            } else {
                ForEach(model.calendars) { calendar in
                    WSettingsRow(label: calendar.title, isOn: calendar.isOn, style: .checkbox, onChange: { actions.setCalendar(calendar.id, $0) })
                }
            }
        }
    }

    // MARK: Transcription

    private var transcriptionCard: some View {
        let l = model.labels
        let a = model.anchors
        let t = model.transcription
        return WSettingsCard(heading: l.transcriptionHeading) {
            WSettingsPickerRow<String>(
                label: l.quality, description: t.qualityDetail, options: wOptions(t.qualityOptions), selection: t.quality, style: .segmented,
                onSelect: actions.setQuality)
            switch t.download {
            case .none:
                EmptyView()
            case .downloading:
                WSettingsProgressRow(label: t.downloadText, fraction: t.download.fraction)
            case .failed:
                WSettingsRow(label: l.downloadFailed, buttonTitle: l.tryAgain, action: actions.retryDownload)
            }
            WSettingsRow(label: t.modelFile.label, description: l.modelFileNote, buttonTitle: l.chooseModelFile, action: actions.chooseModelFile)
            pathRows(t.modelFile)
            WSettingsRow(
                label: l.multilingual, description: l.multilingualNote, isOn: t.multilingual, highlighted: on(a.multilingual),
                onChange: actions.setMultilingual
            )
            .wAnchor(a.multilingual)
            if let status = t.statusMessage { wNote(status) }
        }
        .wSettingsAnchored(a.transcription, highlighted: highlighted)
    }

    private var diarizationCard: some View {
        let l = model.labels
        let t = model.transcription
        return WSettingsCard(heading: l.diarizationHeading, description: l.diarizationNote) {
            WSettingsRow(label: t.segmentation.label, buttonTitle: l.chooseSegmentation, action: actions.chooseSegmentation)
            pathRows(t.segmentation)
            WSettingsRow(label: t.embedding.label, buttonTitle: l.chooseEmbedding, action: actions.chooseEmbedding)
            pathRows(t.embedding)
            if let status = t.diarizationStatus { wNote(status) }
        }
    }

    /// A file path under the row that names it, with the middle of a long path cut and the whole path spoken. The row says which
    /// file it is, so this line only says "Location". A file that is not there says so in words.
    private func pathRows(_ path: WPathModel) -> some View {
        let l = model.labels
        return VStack(spacing: 0) {
            WHairline()
            WSettingsValueList(items: [
                WSettingsValueItem(label: l.pathLocation, value: path.path, copyable: true, truncation: .middle(limit: 56), isProblem: !path.exists)
            ])
            if !path.exists {
                WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: l.fileMissing(path.label)) { EmptyView() }
            }
        }
    }
}

// MARK: Remembered voices sheet

/// Everyone Whimbrel can name by voice, each with Delete. Delete asks first.
struct WRememberedVoicesSheet: View {
    @Environment(\.wTheme) private var theme
    let model: WMeetingsModel
    let actions: WMeetingsActions
    let onDone: () -> Void

    var body: some View {
        let l = model.labels
        WSheet(title: l.rememberedVoices, width: 420, confirm: WSheetButton(l.done, kind: .primary, run: onDone)) {
            if model.voices.isEmpty {
                Text(l.noVoices).wFont(.body).foregroundStyle(theme.textSecondary)
            } else {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(model.voices.enumerated()), id: \.element.id) { index, person in
                        if index > 0 { WHairline() }
                        WReflow(spacing: theme.space(12), stackSpacing: theme.space(8), minLeading: 160) {
                            Text(person.name).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            WConfirmButton(
                                l.deleteVoice, question: l.deleteVoiceQuestion(person.name), confirmTitle: l.deleteVoice,
                                onConfirm: { actions.forgetVoice(person.id) })
                        }
                        .padding(.vertical, theme.space(8))
                    }
                }
            }
        }
        .padding(theme.space(12))
    }
}
