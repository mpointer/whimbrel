import AppKit
import SwiftUI
import WhimbrelUI

/// Turns what the finished meeting page already holds into the plain values the redesigned note page draws.
/// Everything here is pure, so tests can check it without an `AppState` or a store.
enum MeetingNoteAdapter {
    /// A speaker as shown on the page: the resolved name (a guess reads "Sam?"), else the stored label, translated.
    static func speakerName(_ rawLabel: String, names: [String: CoreDisplayLabel]) -> String {
        let shown = names[rawLabel] ?? CoreDisplayLabel(label: rawLabel, guess: false)
        return MeetingDetailLogic.displaySpeaker(shown.label, guess: shown.guess)
    }

    /// The transcript lines, with the part each is in and the "rejoined" divider above the first line of a later part.
    static func segments(
        _ lines: [CoreMeetingTranscriptSegment], names: [String: CoreDisplayLabel], dividers: [(beforeStartCs: Int64, label: String)]
    ) -> [WNoteSegment] {
        let dividerAt = MeetingDetailLogic.dividerLabels(segmentStartCs: lines.map(\.startCs), dividers: dividers)
        var part = 0
        return lines.enumerated().map { index, line in
            let divider = dividerAt[index]
            if divider != nil { part += 1 }
            return WNoteSegment(
                id: String(line.id), startCs: line.startCs, endCs: line.endCs, time: WLaneMath.clock(cs: line.startCs),
                speakerKey: line.speakerLabel, speaker: speakerName(line.speakerLabel, names: names), text: line.text,
                partIndex: part, dividerBefore: divider.map(MeetingDetailLogic.renderedNoteDivider))
        }
    }

    static func marks(_ markers: [CoreMeetingMarker]) -> [WNoteMark] {
        markers.sorted { $0.atCs < $1.atCs }.map {
            WNoteMark(id: String($0.id), atCs: $0.atCs, clock: WLaneMath.clock(cs: $0.atCs), label: $0.label ?? "")
        }
    }

    /// The segment id of the line nearest to a moment, or nil when there are no lines.
    static func nearestSegmentId(to atCs: Int64, in lines: [CoreMeetingTranscriptSegment]) -> Int64? {
        LiveAlertsSection.nearest(lines.map { ($0.id, $0.startCs) }, to: atCs)
    }

    private static func source(
        _ raw: CoreLineSource, number: Int, labels: [Int64: String], names: [String: CoreDisplayLabel], lineText: String
    ) -> WNoteSource {
        let label = labels[raw.segmentId] ?? raw.speakerLabel
        return WNoteSource(
            number: number, segmentId: String(raw.segmentId), speaker: speakerName(label, names: names),
            time: WLaneMath.clock(cs: raw.startCs), quote: raw.text, lineText: lineText)
    }

    /// The speaker label of each transcript line by id, for naming a source's speaker without a scan per source.
    static func labelsById(_ lines: [CoreMeetingTranscriptSegment]) -> [Int64: String] {
        Dictionary(lines.map { ($0.id, $0.speakerLabel) }, uniquingKeysWith: { first, _ in first })
    }

    /// What the page shows of the summary, decisions and open questions right now: the editors' live text, not the
    /// saved draft, which trails an edit by the autosave delay. A section is the person's when a saved override
    /// exists or its live text differs from what Whimbrel wrote.
    struct LiveNote: Equatable {
        var summary: String
        var summaryEdited: Bool
        var decisions: [String]
        var decisionsEdited: Bool
        var openQuestions: [String]
        var openQuestionsEdited: Bool
    }

    static func liveNote(
        summary: String, decisions: [String], openQuestions: [String],
        generatedSummary: String?, generatedDecisions: [String], generatedOpenQuestions: [String], persisted: CoreMeetingNoteDraft?
    ) -> LiveNote {
        LiveNote(
            summary: summary, summaryEdited: persisted?.summaryEdited != nil || summary != (generatedSummary ?? ""),
            decisions: decisions, decisionsEdited: persisted?.decisionsEdited != nil || decisions != generatedDecisions,
            openQuestions: openQuestions, openQuestionsEdited: persisted?.openQuestionsEdited != nil || openQuestions != generatedOpenQuestions)
    }

    /// Lines of a list (decisions or open questions) with their sources, numbered from `numberFrom` down the page.
    /// A line the person edited is theirs: not Whimbrel's, so no source. `edited` says the stored list is the person's;
    /// a line still equal to a generated one stays Whimbrel's. A blank line is never drawn.
    static func lines(
        texts: [String], generated: [String], edited: Bool, sourced: [CoreSourcedText], labels: [Int64: String],
        names: [String: CoreDisplayLabel], numberFrom: Int, idPrefix: String
    ) -> (lines: [WNoteLine], next: Int) {
        var pool = sourced
        var number = numberFrom
        var out: [WNoteLine] = []
        for (index, text) in texts.enumerated() where !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            let isAI = !edited || generated.contains(text)
            var attached: WNoteSource?
            if isAI, let at = pool.firstIndex(where: { $0.text == text }) {
                let entry = pool.remove(at: at)
                if let raw = entry.source {
                    attached = source(raw, number: number, labels: labels, names: names, lineText: text)
                    number += 1
                }
            }
            out.append(WNoteLine(id: "\(idPrefix)-\(index)", text: text, isAI: isAI, source: attached))
        }
        return (out, number)
    }

    /// Whether an action item row is Whimbrel's. Never one the person added. A row whose text is one Whimbrel wrote
    /// (`generated` holds every generation this page has seen, so a Regenerate that kept the person's edits does not
    /// turn the earlier rows into the person's) or whose text was never touched (`updatedUnixMs` still equals
    /// `createdUnixMs`) is Whimbrel's. A row the person changed to other words is theirs.
    static func isWhimbrelsActionItem(_ item: CoreMeetingActionItem, generated: [String]) -> Bool {
        guard item.source != "user" else { return false }
        return generated.contains(item.text) || item.updatedUnixMs == item.createdUnixMs
    }

    /// Action items from the stored rows, in their order. Owner and due come from core, only when it stored them. A
    /// source is attached only when the stored one was written for these same words. A row with no words is not drawn,
    /// as on the page today.
    static func actionItems(
        _ items: [CoreMeetingActionItem], generated: [String], sourced: [CoreSourcedActionItem], labels: [Int64: String],
        names: [String: CoreDisplayLabel], numberFrom: Int
    ) -> [WNoteActionItem] {
        var pool = sourced
        var number = numberFrom
        return items.compactMap { item in
            if item.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return nil }
            let isAI = isWhimbrelsActionItem(item, generated: generated)
            var entry: CoreSourcedActionItem?
            if let at = pool.firstIndex(where: { $0.itemId == item.id && $0.text == item.text })
                ?? pool.firstIndex(where: { $0.itemId == nil && $0.text == item.text }) {
                entry = pool.remove(at: at)
            }
            var attached: WNoteSource?
            if isAI, let raw = entry?.source {
                attached = source(raw, number: number, labels: labels, names: names, lineText: item.text)
                number += 1
            }
            func present(_ value: String?) -> String? {
                guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty else { return nil }
                return value
            }
            return WNoteActionItem(
                id: String(item.id), line: WNoteLine(id: "action-\(item.id)", text: item.text, isAI: isAI, source: attached),
                isDone: item.completed, owner: present(entry?.owner), due: present(entry?.due))
        }
    }

    /// What the note area is: processing, a failure, a reason there is no summary, or the sections. While a summary is
    /// being made and there is none to show, the page says so, whatever it said before.
    static func state(
        isCompleted: Bool, isProcessing: Bool, isRegenerating: Bool = false, regenerating: String = "", stage: String?, hasContent: Bool,
        failure: String?, placeholder: String, notYet: String
    ) -> WNoteState {
        if !isCompleted { return .noSummary(notYet) }
        if hasContent { return .ready }
        if isProcessing { return .processing(stage: stage ?? placeholder) }
        if isRegenerating { return .processing(stage: regenerating) }
        if let failure { return .failed(failure) }
        return .noSummary(placeholder)
    }

    /// Changes when what the sources depend on changes: the words Whimbrel wrote (all four lists), the action item
    /// rows, and the transcript. Not the person's own edits, which cannot change a source, so typing never reloads.
    static func sourcesKey(
        summary: String?, decisions: [String], openQuestions: [String], actionItemTexts: [String], itemIds: [Int64], segmentCount: Int
    ) -> Int {
        var hasher = Hasher()
        hasher.combine(summary)
        hasher.combine(decisions)
        hasher.combine(openQuestions)
        hasher.combine(actionItemTexts)
        hasher.combine(itemIds)
        hasher.combine(segmentCount)
        return hasher.finalize()
    }

    /// Whether the transcript has anything to copy, without building the text: the same answer as
    /// `MeetingDetailLogic.transcriptCopyText` being non-empty.
    static func hasTranscriptToCopy(fileText: String?, segmentCount: Int) -> Bool {
        segmentCount > 0 || !(fileText ?? "").isEmpty
    }

    /// Lines from a transcript file, for an older meeting with no stored segments.
    static func turns(chunks: [(label: String?, turns: [(speaker: String, timestamp: String, text: String)])]) -> [WNoteTurn] {
        var out: [WNoteTurn] = []
        for chunk in chunks {
            for (position, turn) in chunk.turns.enumerated() {
                out.append(WNoteTurn(
                    id: out.count, speaker: MeetingDetailLogic.displaySpeaker(turn.speaker), time: turn.timestamp, text: turn.text,
                    dividerBefore: position == 0 ? chunk.label.map(MeetingDetailLogic.renderedNoteDivider) : nil))
            }
        }
        return out
    }
}

/// What the page works out from the transcript, kept between passes of the view body: the redesigned page rebuilds its
/// model on every keystroke in an editor and every live alert tick, and a long meeting has thousands of lines. Each
/// value is rebuilt only when what it came from changed. A reference type held in `@State`, so filling it is not a
/// state change.
final class NoteTranscriptCache {
    struct LineKey: Equatable {
        var count: Int
        var first: Int64?
        var last: Int64?
    }

    struct TranscriptKey: Equatable {
        var lines: LineKey
        var names: [String: CoreDisplayLabel]
        var partCount: Int
        var marks: [Int64]
    }

    private var labelKey: LineKey?
    private var labels: [Int64: String] = [:]
    private var transcriptKey: TranscriptKey?
    private var transcript: (segments: [WNoteSegment], groups: [WLaneGroup]) = ([], [])
    private var turnsKey: [Int]?
    private var turns: [WNoteTurn] = []

    static func lineKey(_ lines: [CoreMeetingTranscriptSegment]) -> LineKey {
        LineKey(count: lines.count, first: lines.first?.id, last: lines.last?.id)
    }

    func labelsById(_ lines: [CoreMeetingTranscriptSegment]) -> [Int64: String] {
        let key = Self.lineKey(lines)
        if labelKey != key {
            labels = MeetingNoteAdapter.labelsById(lines)
            labelKey = key
        }
        return labels
    }

    func transcript(
        lines: [CoreMeetingTranscriptSegment], names: [String: CoreDisplayLabel], partCount: Int, markers: [CoreMeetingMarker],
        build: () -> [WNoteSegment], marks: [WNoteMark]
    ) -> (segments: [WNoteSegment], groups: [WLaneGroup]) {
        let key = TranscriptKey(lines: Self.lineKey(lines), names: names, partCount: partCount, marks: markers.flatMap { [$0.id, $0.atCs] })
        if transcriptKey != key {
            let built = build()
            transcript = (built, WLaneMath.groups(segments: built, marks: marks))
            transcriptKey = key
        }
        return transcript
    }

    func turns(key: [Int], build: () -> [WNoteTurn]) -> [WNoteTurn] {
        if turnsKey != key {
            turns = build()
            turnsKey = key
        }
        return turns
    }
}

/// The finished note page in the redesign. It is `MeetingDetailContentView`'s own state and logic (loading, autosave, the
/// editors, regenerate and its conflict sheet, the recap sheet, the options menu, the ask bar, speaker naming, the notices)
/// drawn through `WNoteView`; nothing is re-implemented here.
extension MeetingDetailLogic {
    /// The "rejoined" divider as the redesigned page shows it: a plain label with the time, no dashes around it.
    /// Reads the stored English form (`HH:mm` inside the label) and falls back to the time-less text if it cannot.
    static func renderedNoteDivider(_ label: String) -> String {
        let stamp = label.split(separator: " ").first { $0.contains(":") }
        guard let stamp else { return String(localized: "Rejoined", comment: "Divider above the part of a transcript recorded after the person rejoined the meeting.") }
        let hhmm = stamp.split(separator: ":")
        guard hhmm.count == 2, let h = Int(hhmm[0]), let m = Int(hhmm[1]),
              let date = Calendar.current.date(bySettingHour: h, minute: m, second: 0, of: Date()) else {
            return String(localized: "Rejoined", comment: "Divider above the part of a transcript recorded after the person rejoined the meeting.")
        }
        let time = date.formatted(date: .omitted, time: .shortened)
        return String(localized: "Rejoined at \(time)", comment: "Divider above the part of a transcript recorded after the person rejoined the meeting. %@ is the time of day.")
    }
}

extension MeetingDetailContentView {
    /// The empty summary box's text on the redesigned page: the same words as the old page, without the dash.
    var noteSummaryPlaceholderText: String {
        switch MeetingDetailLogic.summaryPlaceholder(processing: appState.processingMeetingId == meeting.id, refusedForExclusion: summaryRefusedForExclusion) {
        case .transcribing:
            return String(localized: "Transcribing meeting audio. The summary will follow.", comment: "Placeholder in the empty summary while the meeting audio is still being transcribed.")
        default:
            return summaryPlaceholderText
        }
    }

    /// The transcript tab's text for a finished meeting with no transcript yet, on the redesigned page.
    var noteCompletedWithoutTranscriptText: String {
        if appState.processingMeetingId == meeting.id {
            return String(localized: "Transcribing in the background. Check back shortly.", comment: "Shown in place of the transcript while it is still being made in the background.")
        }
        return completedWithoutTranscriptText
    }

    /// Changes when something the sources depend on changes, so they are read again.
    var noteSourcesKey: Int {
        let generated = effectiveMeeting
        return MeetingNoteAdapter.sourcesKey(
            summary: generated.summaryTldr, decisions: generated.summaryDecisions, openQuestions: generated.summaryOpenQuestions,
            actionItemTexts: generated.summaryActionItems, itemIds: actionItems.map(\.id), segmentCount: segments.count)
    }

    func loadNoteSources() {
        guard let store = appState.coreStore else { return }
        let meetingId = meeting.id
        CoreMemoryStore.accessQueue.async {
            let loaded = store.meetingNoteSources(meetingId: meetingId)
            DispatchQueue.main.async { if noteSources != loaded { noteSources = loaded } }
        }
    }

    func loadMarkers() {
        guard let store = appState.coreStore else { return }
        let meetingId = meeting.id
        CoreMemoryStore.accessQueue.async {
            let loaded = store.meetingMarkers(meetingId: meetingId) ?? []
            DispatchQueue.main.async { if markers != loaded { markers = loaded } }
        }
    }

    func removeMarker(_ id: String) {
        guard let markerId = Int64(id), let store = appState.coreStore else { return }
        markers.removeAll { $0.id == markerId }
        CoreMemoryStore.accessQueue.async { store.deleteMeetingMarker(id: markerId) }
    }

    /// Opens the transcript at the line nearest a moment (centiseconds); the transcript view opens even with no lines.
    func openTranscript(atCs: Int64) {
        if let id = MeetingNoteAdapter.nearestSegmentId(to: atCs, in: segments) { highlightSegment(id) } else { tab = .transcript }
    }

    private var noteFailure: String? {
        guard !isRegenerating else { return nil }
        return regenerateError ?? appState.summaryFailures[meeting.id]?.userFacingMessage
    }

    private var noteTranscriptMessage: String? {
        if appState.noSpeechDetected(meeting) {
            return String(localized: "No speech detected in this recording. The recording is still here: choose Meeting options › Transcribe again.")
        }
        if transcriptText != nil { return nil }
        if transcriptLoadFailed { return String(localized: "Transcript unavailable.") }
        if meeting.transcriptPath != nil { return String(localized: "Loading transcript…") }
        if meeting.status == .completed { return noteCompletedWithoutTranscriptText }
        return String(localized: "No transcript yet. The meeting hasn't ended.")
    }

    private var noteTurns: [WNoteTurn] {
        guard segments.isEmpty, !appState.noSpeechDetected(meeting), let transcriptText else { return [] }
        return noteCache.turns(key: [transcriptText.utf8.count, parts.count]) {
            let chunks = MeetingDetailLogic.splitAtDividers(transcriptText)
            let parsed = chunks.map { CoreMemoryStore.parseTranscriptTurns(from: $0.body) }
            if parsed.allSatisfy(\.isEmpty) {
                return [WNoteTurn(id: 0, speaker: "", time: "", text: transcriptText)]
            }
            return MeetingNoteAdapter.turns(chunks: zip(chunks, parsed).map { chunk, turns in
                (chunk.label, turns.map { ($0.speaker, $0.timestamp, $0.text) })
            })
        }
    }

    var noteViewModel: WNoteModel {
        let sources = noteSources ?? CoreNoteSources()
        let names = speakerNames
        let generated = effectiveMeeting
        let live = MeetingNoteAdapter.liveNote(
            summary: summaryDraftText, decisions: decisionsDraftItems, openQuestions: openQuestionsDraftItems,
            generatedSummary: generated.summaryTldr, generatedDecisions: generated.summaryDecisions,
            generatedOpenQuestions: generated.summaryOpenQuestions, persisted: draft)
        // Every generation this page has seen: a Regenerate that kept the person's edits leaves the earlier lines in place.
        let wroteDecisions = generated.summaryDecisions + meeting.summaryDecisions
        let wroteQuestions = generated.summaryOpenQuestions + meeting.summaryOpenQuestions
        let wroteActions = generated.summaryActionItems + meeting.summaryActionItems
        let hasSources = !sources.decisions.isEmpty || !sources.openQuestions.isEmpty || !sources.actionItems.isEmpty
        let labels = hasSources ? noteCache.labelsById(segments) : [:]
        let summaryLine = live.summary.isEmpty ? nil : WNoteLine(id: "summary", text: live.summary, isAI: !live.summaryEdited)
        let decisions = MeetingNoteAdapter.lines(
            texts: live.decisions, generated: wroteDecisions, edited: live.decisionsEdited, sourced: sources.decisions,
            labels: labels, names: names, numberFrom: 1, idPrefix: "decision")
        let questions = MeetingNoteAdapter.lines(
            texts: live.openQuestions, generated: wroteQuestions, edited: live.openQuestionsEdited,
            sourced: sources.openQuestions, labels: labels, names: names, numberFrom: decisions.next, idPrefix: "question")
        let items = MeetingNoteAdapter.actionItems(
            actionItems, generated: wroteActions, sourced: sources.actionItems, labels: labels, names: names, numberFrom: questions.next)
        let hasContent = summaryLine != nil || !decisions.lines.isEmpty || !questions.lines.isEmpty || !items.isEmpty
        let completed = meeting.status == .completed
        var provenance: [String] = []
        if let currentProvider, let currentModel { provenance.append(String(localized: "Generated by \(currentModel) (\(currentProvider))")) }
        if live.summaryEdited { provenance.append(String(localized: "Edited by you")) }
        let mode: WNoteMode = tab == .transcript ? .transcript : .note
        let marks = MeetingNoteAdapter.marks(markers)
        // The transcript is only worked out while it is showing.
        var transcriptSegments: [WNoteSegment] = []
        var groups: [WLaneGroup]?
        var turns: [WNoteTurn] = []
        var message: String?
        if mode == .transcript {
            let built = noteCache.transcript(
                lines: segments, names: names, partCount: parts.count, markers: markers,
                build: { MeetingNoteAdapter.segments(segments, names: names, dividers: MeetingDetailLogic.dividers(parts: parts)) },
                marks: marks)
            transcriptSegments = built.segments
            groups = built.groups
            turns = noteTurns
            message = noteTranscriptMessage
        }
        return WNoteModel(
            mode: mode, backTitle: onBack == nil ? nil : String(localized: "Meetings"), title: MeetingDetailLogic.displayTitle(meeting.title),
            eyebrow: Self.formattedMeetingTime(startUnixMs: meeting.startUnixMs, endUnixMs: meeting.endUnixMs),
            attendees: meeting.attendees.joined(separator: ", "), yourNotes: meeting.notes ?? "",
            state: MeetingNoteAdapter.state(
                isCompleted: completed, isProcessing: appState.processingMeetingId == meeting.id,
                isRegenerating: isRegenerating, regenerating: String(localized: "Regenerating…"), stage: appState.processingStage,
                hasContent: hasContent, failure: noteFailure, placeholder: noteSummaryPlaceholderText,
                notYet: String(localized: "Summary will be generated once the meeting ends.")),
            canEdit: completed, canRegenerate: transcriptText != nil, isEditing: isEditing,
            canSendRecap: completed, canCopy: !summaryPlainText.isEmpty, sentLine: recapSent.map(recapSentText),
            summary: summaryLine, decisions: decisions.lines, openQuestions: questions.lines, actionItems: items,
            marks: marks, provenance: provenance, selectedSource: noteSelectedSource,
            readingScale: WReadingScale.scale(fontSize: appState.readingFontSize),
            segments: transcriptSegments, turns: turns, transcriptMessage: message,
            highlightedSegmentId: highlightedSegmentId.map { String($0) },
            canCopyTranscript: MeetingNoteAdapter.hasTranscriptToCopy(fileText: transcriptText, segmentCount: segments.count),
            laneGroups: groups)
    }

    var noteActions: WNoteActions {
        WNoteActions(
            back: { onBack?() },
            setMode: { tab = $0 == .note ? .notes : .transcript },
            copyPlain: { copyToClipboard(summaryPlainText) },
            copyMarkdown: { copyRecap(format: "markdown") },
            copySlack: { copyRecap(format: "slack") },
            copyTranscript: { copyToClipboard(fullTranscriptText) },
            sendRecap: { showRecapSheet = true },
            toggleEdit: { isEditing.toggle() },
            regenerate: { regenerate() },
            selectSource: { noteSelectedSource = $0 },
            openSegment: { if let id = Int64($0) { highlightSegment(id) } },
            toggleActionItem: { id, done in
                if let itemId = Int64(id) { actionItemCompletedBinding(id: itemId).wrappedValue = done }
            },
            openMark: { id in
                if let markerId = Int64(id), let marker = markers.first(where: { $0.id == markerId }) { openTranscript(atCs: marker.atCs) }
            },
            removeMark: { removeMarker($0) })
    }

    @ViewBuilder
    private var noteNotices: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let topic { TopicChip(topic: topic) }
            if let outcome = meeting.effectiveAwayOutcome {
                Text(AwayOutcomeLabel.text(outcome, at: meeting.awayOutcomeAt))
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
            DeviceChip(deviceId: meeting.deviceId)
            if let fallback = shownSummaryFallback {
                SummaryFallbackNoticeView(
                    fallback: fallback, isWorking: isRegenerating, errorMessage: resummarizeError,
                    canResummarize: transcriptText != nil, onResummarize: resummarizeWithWhimbrel)
            }
            if namesChanged {
                NamesChangedBanner(updating: isRegenerating, canUpdate: transcriptText != nil, error: regenerateError) { regenerate() }
            }
            // A failed Regenerate on a note that has a summary: the line is never hidden.
            if let failure = noteFailure, meeting.summaryTldr != nil || regeneratedSummary != nil, !isEditing {
                Text(failure).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
            }
        }
    }

    @ViewBuilder
    private var noteExtras: some View {
        VStack(alignment: .leading, spacing: 12) {
            MeetingSnapshotsStrip(model: snapshotsModel)
                .onReceive(NotificationCenter.default.publisher(for: MeetingSnapshotService.didChange)) { note in
                    guard note.userInfo?["meetingId"] as? Int64 == meeting.id else { return }
                    Task { await snapshotsModel.reload() }
                }
            LiveAlertsSection(
                rows: liveAlerts,
                paused: meeting.id == appState.activeMeetingId
                    ? LiveAlertsSection.pausedLabel(
                        status: liveStatus,
                        alertsOn: LiveQuestionWatcher.shouldRun(settings: appState.liveQuestionSettings, names: appState.profile.liveQuestionNames))
                    : nil,
                onJump: segments.isEmpty ? nil : { seekTranscript(to: $0) }
            )
            MeetingPrepSection(meetingId: meeting.id)
        }
    }

    @ViewBuilder
    var noteRedesignPage: some View {
        WNoteView(
            model: noteViewModel, actions: noteActions,
            options: { MeetingOptionsMenu(meeting: meeting, appState: appState, onForgotten: { onForgotten?() }) },
            notices: { noteNotices },
            editor: {
                VStack(alignment: .leading, spacing: Spacing.lg) {
                    if meeting.status == .completed { summaryEditorsCard }
                    regenerateSection
                }
            },
            extras: { noteExtras },
            ask: {
                MeetingAskBar(session: askSession, meeting: meeting, segmentCount: segments.count) { segmentId in
                    highlightSegment(segmentId)
                }
                .onDisappear { askSession.teardown() }
            },
            speaker: { segment in
                let raw = segment.speakerKey
                SpeakerMenu(
                    shown: speakerNames[raw] ?? CoreDisplayLabel(label: raw, guess: false),
                    invitees: invitees,
                    offered: partsLoaded && MeetingDetailLogic.speakerMenuOffered(rawLabel: raw, partCount: parts.count),
                    onConfirm: { personKey in nameSpeaker(rawLabel: raw) { $0.confirmMeetingSpeaker(meetingId: $1, speakerId: $2, personKey: personKey) } }
                ) { name, email in nameSpeaker(rawLabel: raw) { $0.setMeetingSpeaker(meetingId: $1, speakerId: $2, name: name, email: email) } }
            })
            .redesignThemed()
            .task { loadMarkers() }
            .task(id: noteSourcesKey) { loadNoteSources() }
            .onChange(of: appState.processingMeetingId == meeting.id ? appState.processingStage : nil) { _, stage in
                if let stage { AccessibilityNotification.Announcement(stage).post() }
            }
    }
}
