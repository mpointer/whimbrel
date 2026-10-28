import SwiftUI

enum MeetingPageTab: String, CaseIterable, Identifiable {
    case notes = "Notes"
    case transcript = "Transcript"
    var id: String { rawValue }
}

/// Next `order_index` for a newly-inserted action item. `list_meeting_action_items`
/// orders by `order_index ASC` with no secondary tiebreaker, and `delete_meeting_action_item`
/// never resequences the rows left behind -- so reusing `existing.count` after a
/// mid-list delete can collide with a surviving row's `order_index` (e.g. rows at
/// [0,1,2], delete the one at 1, survivors are [0,2] but `.count` is 2 -- colliding with
/// the surviving row already at 2, an undefined SQLite tie-break on next reload). Pure
/// and testable, same reasoning as `regenerationNeedsConflictPrompt`.
func nextActionItemOrderIndex(existing: [CoreMeetingActionItem]) -> Int64 {
    (existing.map(\.orderIndex).max() ?? -1) + 1
}

/// Merges a persisted `meeting_note_draft` with local editable-field state that hasn't
/// autosaved yet. Local state (`summaryDraftText`/etc.) updates synchronously on every
/// keystroke; only the STORE WRITE that mirrors it into `persisted` is debounced
/// (~1.5s). Without this, a fast "type an edit, immediately click Regenerate" sequence
/// could land inside that debounce window: `regenerationNeedsConflictPrompt` would see
/// a stale `persisted` (still nil, or without the newest keystrokes) and silently skip
/// the conflict prompt, letting the regeneration overwrite the just-typed edit with no
/// prompt and no way to recover it. Comparing local values against the pre-regeneration
/// generated content (rather than trusting `persisted` alone) closes that race
/// regardless of timing. Pure and testable.
func mergeUnsavedEditsIntoDraft(
    persisted: CoreMeetingNoteDraft?,
    meetingId: Int64,
    localSummary: String, generatedSummary: String?,
    localDecisions: [String], generatedDecisions: [String],
    localOpenQuestions: [String], generatedOpenQuestions: [String]
) -> CoreMeetingNoteDraft? {
    let summaryOverride = localSummary != (generatedSummary ?? "") ? localSummary : persisted?.summaryEdited
    let decisionsOverride = localDecisions != generatedDecisions ? localDecisions : persisted?.decisionsEdited
    let openQuestionsOverride = localOpenQuestions != generatedOpenQuestions ? localOpenQuestions : persisted?.openQuestionsEdited
    guard summaryOverride != nil || decisionsOverride != nil || openQuestionsOverride != nil else { return persisted }
    return CoreMeetingNoteDraft(
        meetingId: meetingId, summaryEdited: summaryOverride, decisionsEdited: decisionsOverride,
        openQuestionsEdited: openQuestionsOverride, personalNotes: persisted?.personalNotes,
        lastEditedUnixMs: persisted?.lastEditedUnixMs ?? 0, lastRegeneratedSeenUnixMs: persisted?.lastRegeneratedSeenUnixMs
    )
}

/// True when at least one action item shows evidence of real user interaction -- a
/// user-added item, one marked complete, or one whose text/order was touched after
/// creation (a freshly-seeded `source == "generated"` row has `updatedUnixMs ==
/// createdUnixMs`; any text edit, completion toggle, or reorder bumps `updatedUnixMs`).
/// Mirrors `regenerationNeedsConflictPrompt`'s "is there anything a silent regeneration
/// could actually clobber" reasoning, but for action items rather than the summary/
/// decisions/open-questions draft -- see `regenerate()`'s use of this alongside that
/// function to decide whether a regeneration needs the conflict prompt. Deliberately
/// over-triggers on a pure reorder with no other change (nothing in the row model
/// distinguishes "reordered only" from "text edited") -- conservative by design, since
/// the alternative risks silently dropping real user work, which must never happen
/// silently. Pure and testable.
func actionItemsHaveUserInteraction(existing: [CoreMeetingActionItem]) -> Bool {
    existing.contains { $0.source == "user" || $0.completed || $0.updatedUnixMs != $0.createdUnixMs }
}

/// FR-MTG-5: the window shown when opening a meeting from Timeline. Fetches the
/// meeting fresh by id (see `CoreMemoryStore.getMeeting(id:)`) rather than relying on
/// a value passed in at window-open time — this is what lets returning to this window
/// later show a summary regenerated in the meantime, and what lets an old meeting well
/// outside any "recent" window still be opened directly. Degrades to a plain "not
/// available" message if the meeting can't be found (deleted via retention, or a bad
/// id) rather than a blank or crashing view.
struct MeetingDetailView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss
    let meetingId: Int64

    @State private var loadedMeeting: CoreMeeting?
    @State private var loadedDraft: CoreMeetingNoteDraft?
    @State private var loadedActionItems: [CoreMeetingActionItem] = []
    @State private var loadFailed = false

    var body: some View {
        Group {
            if let loadedMeeting {
                MeetingDetailContentView(meeting: loadedMeeting, initialDraft: loadedDraft, initialActionItems: loadedActionItems)
            } else if loadFailed {
                Text("This meeting is no longer available.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 480, idealWidth: 560, minHeight: 400, idealHeight: 560)
        .navigationTitle(loadedMeeting?.title ?? "Meeting")
        .background(Color.whimbrelBackgroundPage)
        .overlay(alignment: .topTrailing) {
            HStack(spacing: Spacing.sm) {
                if let loadedMeeting {
                    MeetingOptionsMenu(meeting: loadedMeeting, appState: appState, onForgotten: { dismiss() })
                }
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 20))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
                .buttonStyle(.plain)
                .help("Close")
            }
            .padding(Spacing.md)
        }
        .onAppear { loadMeetingIfNeeded() }
        .onChange(of: appState.processingMeetingId) { old, new in
            guard old == meetingId, new == nil else { return }
            loadedMeeting = nil
            loadMeetingIfNeeded()
        }
    }

    private func loadMeetingIfNeeded() {
        guard loadedMeeting == nil, !loadFailed else { return }
        guard let store = appState.coreStore else {
            loadFailed = true
            return
        }
        let id = meetingId
        CoreMemoryStore.accessQueue.async {
            let result = store.getMeeting(id: id)
            let draft = store.getMeetingNoteDraft(meetingId: id)
            var items = store.listMeetingActionItems(meetingId: id)
            // Idempotent seed: only inserts rows when none exist yet, so it's safe to
            // run on every load rather than gating on "first time we've seen this meeting".
            if let result, items.isEmpty, !result.summaryActionItems.isEmpty {
                let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
                _ = store.seedMeetingActionItems(meetingId: id, texts: result.summaryActionItems, nowUnixMs: nowMs)
                items = store.listMeetingActionItems(meetingId: id)
            }
            DispatchQueue.main.async {
                if let result {
                    loadedMeeting = result
                    loadedDraft = draft
                    loadedActionItems = items
                } else {
                    loadFailed = true
                }
            }
        }
    }
}

/// The actual meeting-detail content, once `meeting` has been loaded — see
/// `MeetingDetailView` above (the window's real, id-based view) for the fetch/loading/
/// not-found wrapper around this. Task 7 (v0.4.0) turned the read-only summary/
/// decisions/open-questions/action-items section into an editable note backed by
/// Task 2's `meeting_note_draft`/`meeting_action_item` tables, with a conflict-resolution
/// prompt when a Regenerate would otherwise clobber structured edits -- everything else
/// here (transcript rendering/reconstruction, copy controls, the existing
/// `regenerate()`/`appState.regenerateMeetingSummary` call itself) is unchanged.
struct MeetingDetailContentView: View {
    @EnvironmentObject var appState: AppState
    let meeting: CoreMeeting
    /// This page visit's Ask, torn down when the bar leaves.
    @StateObject private var askSession = MeetingAskSession()
    @StateObject private var snapshotsModel: MeetingSnapshotsModel

    @State private var transcriptText: String?
    @State private var transcriptLoadFailed = false

    @State private var regeneratedSummary: CoreMeetingSummary?
    @State private var selectedProvider: RoutingSettings.Provider = .ollama
    @State private var ollamaModel: String = "gemma4:12b"
    @State private var selectedHostedModel: String = "claude-haiku-4-5"
    @State private var isRegenerating = false
    @State private var regenerateError: String?

    // Task 2's editable-overlay draft + action items (FR-MTG editable-note work), loaded
    // by `MeetingDetailView` alongside the meeting fetch and handed in here as the
    // starting point for this view's own mutable copies below.
    @State private var draft: CoreMeetingNoteDraft?
    @State private var actionItems: [CoreMeetingActionItem]
    @State private var summaryDraftText: String
    @State private var decisionsDraftItems: [String]
    @State private var openQuestionsDraftItems: [String]

    // Debounced-autosave bookkeeping. Not `ActiveNoteAutosaveController` (Task 6) --
    // that class's `onSave: (String, Int64) -> Bool` closure has to return synchronously
    // from inside a MainActor `Task`, which forces either a main-thread store call or a
    // semaphore block, either way violating the accessQueue.async + main.async
    // requirement below. This reuses its actual pattern instead (cancel-then-debounce via
    // Task, direct state assignment with no extra `MainActor.run` hop per Task 6's fix)
    // adapted to fire real async store writes. Kept as three independent tasks, one per
    // field, so an edit to one section never sends the others' current values back to
    // `saveNoteOverrides` as if they'd also just been edited (see that function's "nil
    // means untouched" contract) -- passing all three every time would wrongly flip
    // sections the user never touched into "edited" state.
    @State private var summarySaveTask: Task<Void, Never>?
    @State private var decisionsSaveTask: Task<Void, Never>?
    @State private var openQuestionsSaveTask: Task<Void, Never>?
    @State private var actionItemTextSaveTasks: [Int64: Task<Void, Never>] = [:]

    // Guards `summaryDraftText`'s `.onChange` from re-scheduling a save when THIS view
    // (not the user's keyboard) is the one assigning the value -- e.g. applying a
    // regenerated summary, or "Replace" resetting the draft back to generated content.
    // Without this, that programmatic assignment still fires `.onChange`, which schedules
    // a debounced save that ~1.5s later writes the just-applied generated text back into
    // `summary_edited` as if the user had typed it -- silently undoing "Replace" and
    // minting a spurious "Edited by you" on every regeneration. Decisions/open-questions
    // don't need the same guard: unlike `summaryDraftText`, they have no `.onChange`
    // modifier on the array itself -- their save only fires from inside `editableList`'s
    // per-row `Binding.set`, which a bulk `decisionsDraftItems = ...` reassignment never
    // goes through. See `setSummaryDraftTextProgrammatically(_:)` below, the only
    // sanctioned way to assign `summaryDraftText` from code rather than the TextEditor.
    @State private var suppressSummaryAutosave = false

    @State private var pendingConflict: PendingRegenerationConflict?

    @State private var tab: MeetingPageTab = .notes
    @State private var isEditing = false
    @State private var segments: [CoreMeetingTranscriptSegment] = []
    @State private var parts: [CoreMeetingPart] = []
    @State private var highlightedSegmentId: Int64?

    /// Key of the Decisions/Open-questions/Action-item row currently under the mouse --
    /// its reorder/delete buttons fade in only for that row (macOS Mail/Notes-style
    /// row-hover), rather than every row permanently showing three bare icon buttons.
    @State private var hoveredRowKey: String?

    /// Setting `hoveredRowKey` directly from `.onHover`'s closure can re-enter AppKit's
    /// active layout pass (this view sits in a `NavigationSplitView` detail pane) and
    /// crash with `_postWindowNeedsUpdateConstraints`. Deferring to the next runloop
    /// tick breaks the reentrancy.
    private func setHoveredRow(_ hovering: Bool, key: String) {
        DispatchQueue.main.async {
            hoveredRowKey = hovering ? key : (hoveredRowKey == key ? nil : hoveredRowKey)
        }
    }

    private struct PendingRegenerationConflict: Identifiable {
        let id = UUID()
        let summary: CoreMeetingSummary
        let newGenerationUnixMs: Int64
    }

    init(meeting: CoreMeeting, initialDraft: CoreMeetingNoteDraft?, initialActionItems: [CoreMeetingActionItem]) {
        self.meeting = meeting
        _snapshotsModel = StateObject(wrappedValue: MeetingSnapshotsModel(meetingId: meeting.id, store: nil))
        _draft = State(initialValue: initialDraft)
        _actionItems = State(initialValue: initialActionItems)
        _summaryDraftText = State(initialValue: initialDraft?.summaryEdited ?? meeting.summaryTldr ?? "")
        _decisionsDraftItems = State(initialValue: initialDraft?.decisionsEdited ?? meeting.summaryDecisions)
        _openQuestionsDraftItems = State(initialValue: initialDraft?.openQuestionsEdited ?? meeting.summaryOpenQuestions)
    }

    private var currentModelLabel: String {
        selectedProvider == .ollama ? ollamaModel : selectedHostedModel
    }

    /// `meeting` overlaid with a freshly-regenerated summary, if any -- same overlay
    /// pattern `transcriptText` already uses for the transcript. Exists so `noteModel`
    /// (which needs a `CoreMeeting`) sees the latest generated content even though
    /// `meeting` itself is a `let` snapshot from whenever this view was constructed.
    private var effectiveMeeting: CoreMeeting {
        guard let regeneratedSummary else { return meeting }
        return CoreMeeting(
            id: meeting.id, deviceId: meeting.deviceId, startUnixMs: meeting.startUnixMs, endUnixMs: meeting.endUnixMs,
            title: meeting.title, attendees: meeting.attendees, trigger: meeting.trigger, audioPath: meeting.audioPath,
            status: meeting.status, transcriptPath: meeting.transcriptPath, notes: meeting.notes,
            summaryTldr: regeneratedSummary.tldr, summaryDecisions: regeneratedSummary.decisions,
            summaryActionItems: regeneratedSummary.actionItems, summaryOpenQuestions: regeneratedSummary.openQuestions,
            summaryProvider: regeneratedSummary.provider, summaryModel: regeneratedSummary.model,
            estimatedSpeakerCount: meeting.estimatedSpeakerCount
        )
    }

    /// Task 4's convenience model -- the one place "does the user's edit win over the
    /// generated content" is decided (`displaySummary`/`summaryIsUserEdited`/etc.).
    private var noteModel: EditableMeetingNoteModel {
        EditableMeetingNoteModel(meeting: effectiveMeeting, draft: draft, actionItems: actionItems, isProcessing: false, processingStage: nil)
    }

    /// `draft` folded together with any local edit that hasn't autosaved yet -- what
    /// `regenerate()`'s conflict check must evaluate against, not `draft` alone (see
    /// `mergeUnsavedEditsIntoDraft`'s doc comment for the race this closes).
    private var draftReflectingUnsavedEdits: CoreMeetingNoteDraft? {
        let baseline = effectiveMeeting
        return mergeUnsavedEditsIntoDraft(
            persisted: draft, meetingId: meeting.id,
            localSummary: summaryDraftText, generatedSummary: baseline.summaryTldr,
            localDecisions: decisionsDraftItems, generatedDecisions: baseline.summaryDecisions,
            localOpenQuestions: openQuestionsDraftItems, generatedOpenQuestions: baseline.summaryOpenQuestions
        )
    }

    private var currentProvider: String? { regeneratedSummary?.provider ?? meeting.summaryProvider }
    private var currentModel: String? { regeneratedSummary?.model ?? meeting.summaryModel }

    /// Plain-text rendering of the summary for the "Copy" button — mirrors the editable
    /// section's own structure (TL;DR, then each non-empty bullet list with its heading)
    /// so what gets copied matches what's on screen, including any user edits.
    private var summaryPlainText: String {
        var parts: [String] = []
        if let summary = noteModel.displaySummary, !summary.isEmpty {
            parts.append(summary)
        }
        func bulletBlock(title: String, items: [String]) -> String? {
            guard !items.isEmpty else { return nil }
            return "\(title):\n" + items.map { "• \($0)" }.joined(separator: "\n")
        }
        if let block = bulletBlock(title: "Decisions", items: noteModel.displayDecisions) { parts.append(block) }
        if let block = bulletBlock(title: "Action items", items: actionItems.map(\.text)) { parts.append(block) }
        if let block = bulletBlock(title: "Open questions", items: noteModel.displayOpenQuestions) { parts.append(block) }
        return parts.joined(separator: "\n\n")
    }

    /// Replaces the clipboard's entire contents with `text` — matches the existing
    /// copy-to-clipboard pattern already used for MCP tokens (SettingsView.swift).
    private func copyToClipboard(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    /// Assigns `summaryDraftText` from code (applying a regeneration, resolving a
    /// conflict) without re-arming the debounced autosave -- see `suppressSummaryAutosave`'s
    /// doc comment for the bug this closes. The flag is cleared on the next main-queue
    /// turn rather than synchronously, since SwiftUI's `.onChange` fires during the view
    /// update pass triggered by this assignment, not necessarily before the next line of
    /// this function runs.
    private func setSummaryDraftTextProgrammatically(_ text: String) {
        suppressSummaryAutosave = true
        summaryDraftText = text
        DispatchQueue.main.async { suppressSummaryAutosave = false }
    }

    var body: some View {
        VStack(spacing: 0) {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.lg) {
                    Text(meeting.title)
                        .font(.whimbrelDisplay(.largeTitle))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let outcome = meeting.effectiveAwayOutcome {
                        Text(AwayOutcomeLabel.text(outcome, at: meeting.awayOutcomeAt))
                            .font(.whimbrelBody(.callout))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                    }
                    metadataChips
                    MeetingSnapshotsStrip(model: snapshotsModel)
                        .onReceive(NotificationCenter.default.publisher(for: MeetingSnapshotService.didChange)) { note in
                            guard note.userInfo?["meetingId"] as? Int64 == meeting.id else { return }
                            Task { await snapshotsModel.reload() }
                        }
                    Picker("", selection: $tab) {
                        ForEach(MeetingPageTab.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .frame(maxWidth: 260)

                    switch tab {
                    case .notes:
                        notesTab
                    case .transcript:
                        transcriptTab(proxy: proxy)
                    }
                }
                .frame(maxWidth: 760, alignment: .leading)
                .padding(Spacing.xl)
                .frame(maxWidth: .infinity)
            }
        }
        MeetingAskBar(session: askSession, meeting: meeting, segmentCount: segments.count) { segmentId in
            highlightedSegmentId = nil
            tab = .transcript
            DispatchQueue.main.async { highlightedSegmentId = segmentId }
        }
        .onDisappear { askSession.teardown() }
        }
        .onAppear {
            snapshotsModel.store = appState.coreStore
            Task { await snapshotsModel.reload() }
            loadParts()
            loadSegments()
            let defaults = appState.routing.defaultSelection()
            selectedProvider = defaults.provider
            ollamaModel = defaults.ollamaModel
            if defaults.provider != .ollama {
                selectedHostedModel = defaults.hostedModel
            }
        }
        // Only `selectedProvider` gets an onChange here, not the model bindings — those
        // are also written programmatically by `ProviderModelPicker.clampSelectedModelIfNeeded()`
        // (on appear and after every hosted-model-list fetch), so clearing on their
        // change would sometimes wipe a still-relevant error the user hasn't acted on.
        .onChange(of: selectedProvider) { _, _ in
            regenerateError = nil
        }
        // Transcription finished while this page was open: the segments (and the Ask bar's
        // transcript gate) were loaded before they existed.
        .onChange(of: appState.processingMeetingId) { old, new in
            guard old == meeting.id, new != meeting.id else { return }
            loadSegments()
            loadParts(reloadTranscript: true)
        }
        // A split (or a summary after it) changed this meeting's parts, segments or snapshots.
        .onReceive(NotificationCenter.default.publisher(for: MeetingLifecycle.meetingsDidChange)) { note in
            guard (note.userInfo?["meetingIds"] as? [Int64])?.contains(meeting.id) == true else { return }
            loadSegments()
            loadParts(reloadTranscript: true)
            Task { await snapshotsModel.reload() }
        }
        .sheet(item: $pendingConflict) { conflict in
            RegenerationConflictSheet(
                previousSummary: draftReflectingUnsavedEdits?.summaryEdited,
                newSummary: conflict.summary.tldr,
                onChoose: { choice in resolveRegenerationConflict(choice, conflict: conflict) }
            )
        }
    }

    private func loadSegments() {
        guard let store = appState.coreStore else { return }
        let meetingId = meeting.id
        CoreMemoryStore.accessQueue.async {
            let page = store.getTranscriptSegmentsPage(meetingId: meetingId, offset: 0, limit: 20_000)
            DispatchQueue.main.async { segments = page?.segments ?? [] }
        }
    }

    /// Loads the parts off main, then (for a continued meeting) rebuilds the file-text
    /// fallback from every part's transcript file, since `meeting.transcriptPath` is part 1's.
    private func loadParts(reloadTranscript: Bool = false) {
        guard let store = appState.coreStore else { return }
        let meetingId = meeting.id
        CoreMemoryStore.accessQueue.async {
            let loaded = store.meetingParts(meetingId: meetingId) ?? []
            DispatchQueue.main.async {
                parts = loaded
                if reloadTranscript { transcriptText = nil }
                loadTranscriptIfNeeded()
            }
        }
    }

    private var metadataChips: some View {
        HStack(spacing: Spacing.sm) {
            Text(Self.formattedMeetingTime(startUnixMs: meeting.startUnixMs, endUnixMs: meeting.endUnixMs))
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
            DeviceChip(deviceId: meeting.deviceId)
            if !meeting.attendees.isEmpty {
                WhimbrelMetadataChip(systemImage: "person.2", text: meeting.attendees.count == 1 ? meeting.attendees[0] : "\(meeting.attendees.count) people")
            }
        }
    }

    @ViewBuilder
    private var notesTab: some View {
        HStack {
            Spacer()
            Button(isEditing ? "Done" : "Edit") { isEditing.toggle() }
                .buttonStyle(.whimbrelSecondary)
                .controlSize(.small)
                .disabled(meeting.status != .completed)
        }
        if isEditing {
            summarySection
            regenerateSection
        } else {
            readingNotes
        }
        if let notes = meeting.notes, !notes.isEmpty {
            Divider().overlay(Color.whimbrelBorder)
            readingBlock("Your notes", [notes], bullet: false)
        }
    }

    private func bulletRows(_ items: [String]) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                    Text("\u{2022}").foregroundStyle(Color.whimbrelTextSecondary)
                    Text(item).fixedSize(horizontal: false, vertical: true)
                }
                .font(.system(size: appState.readingFontSize, design: .default))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .textSelection(.enabled)
            }
        }
    }

    @ViewBuilder
    private func readingBlock(_ title: String, _ items: [String], bullet: Bool = true) -> some View {
        let items = items.filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: Spacing.sm) {
                Text(title).font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                if bullet {
                    bulletRows(items)
                } else {
                    Text(items.joined(separator: "\n\n"))
                        .font(.system(size: appState.readingFontSize, design: .default))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .textSelection(.enabled)
                }
            }
        }
    }

    /// What the reader sees: the edited text when the user has edited a section, otherwise
    /// the generated one; the same precedence the editors start from.
    @ViewBuilder
    private var readingNotes: some View {
        if meeting.status != .completed {
            Text("Summary will be generated once the meeting ends.")
                .font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextSecondary)
        } else if summaryDraftText.isEmpty && decisionsDraftItems.isEmpty && actionItems.isEmpty && openQuestionsDraftItems.isEmpty {
            Text(summaryPlaceholderText).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            VStack(alignment: .leading, spacing: Spacing.lg) {
                if !summaryDraftText.isEmpty {
                    HStack(alignment: .firstTextBaseline) {
                        Text("Summary").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                        Spacer()
                        Button("Copy") { copyToClipboard(summaryPlainText) }.buttonStyle(.whimbrelSecondary).controlSize(.small)
                    }
                    Text(summaryDraftText)
                        .font(.system(size: appState.readingFontSize + 1, design: .default))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                }
                readingBlock("Decisions", decisionsDraftItems)
                readingBlock("Action items", actionItems.map(\.text))
                readingBlock("Open questions", openQuestionsDraftItems)
                provenanceChips
            }
        }
    }

    @ViewBuilder
    private func transcriptTab(proxy: ScrollViewProxy) -> some View {
        if !fullTranscriptText.isEmpty {
            HStack {
                Spacer()
                Button("Copy") { copyToClipboard(fullTranscriptText) }.buttonStyle(.whimbrelSecondary).controlSize(.small)
            }
        }
        if segments.isEmpty {
            // No per-segment rows (older meetings, or synced without them): the existing view.
            transcriptSection
        } else {
            let dividerAt = MeetingDetailLogic.dividerLabels(
                segmentStartCs: segments.map(\.startCs), dividers: MeetingDetailLogic.dividers(parts: parts))
            LazyVStack(alignment: .leading, spacing: Spacing.md) {
                ForEach(Array(segments.enumerated()), id: \.element.id) { index, segment in
                    if let label = dividerAt[index] { rejoinedDivider(label) }
                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        HStack(spacing: Spacing.xs) {
                            Text(segment.speakerLabel).font(.whimbrelBody(.caption).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                            Text(Self.clock(segment.startCs)).font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
                        }
                        Text(segment.text)
                            .font(.system(size: appState.readingFontSize, design: .default))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(Spacing.sm)
                    .background(highlightedSegmentId == segment.id ? Color.whimbrelHighlight : Color.clear)
                    .clipShape(RoundedRectangle(cornerRadius: Radius.control))
                    .id(segment.id)
                }
            }
            .textSelection(.enabled)
            .onChange(of: highlightedSegmentId) { _, id in
                guard let id else { return }
                withAnimation { proxy.scrollTo(id, anchor: .center) }
            }
            .onAppear {
                if let id = highlightedSegmentId { proxy.scrollTo(id, anchor: .center) }
            }
        }
    }

    private func rejoinedDivider(_ label: String) -> some View {
        Text(label)
            .font(.whimbrelBody(.caption))
            .foregroundStyle(Color.whimbrelTextSecondary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, Spacing.xs)
    }

    private var fullTranscriptText: String {
        if let t = transcriptText, !t.isEmpty { return t }
        return segments.map { "\($0.speakerLabel) (\(Self.clock($0.startCs))): \($0.text)" }.joined(separator: "\n")
    }

    private static func clock(_ startCs: Int64) -> String {
        let seconds = Int(startCs / 100)
        return String(format: "%d:%02d", seconds / 60, seconds % 60)
    }

    /// The empty-state placeholder shown INSIDE the summary editor when there's no
    /// TL;DR text yet -- distinct from the "no card at all" states below, which only
    /// apply before the meeting is even `.completed`. Kept as text overlaid on the
    /// `TextEditor` (SwiftUI's `TextEditor` has no native placeholder support) rather
    /// than hiding the editor, so an empty summary never takes decisions/action items/
    /// open questions down with it -- see Important #1 in the final review.
    private var summaryPlaceholderText: String {
        appState.processingMeetingId == meeting.id ? "Transcribing meeting audio — summary will follow." : "No summary yet."
    }

    private var completedWithoutTranscriptText: String {
        if appState.processingMeetingId == meeting.id {
            return "Transcribing in the background — check back shortly."
        }
        return appState.canTranscribeAgain(meeting)
            ? "No transcript. The recording is still here: choose Meeting options › Transcribe again."
            : "No transcript for this meeting."
    }

    @ViewBuilder
    private var summarySection: some View {
        HStack {
            Text("Summary").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            if !summaryPlainText.isEmpty {
                Spacer()
                Button("Copy") { copyToClipboard(summaryPlainText) }
                    .buttonStyle(.whimbrelSecondary)
                    .controlSize(.small)
            }
        }
        // Gated on the meeting having *reached* the After state, never on the summary
        // text being non-empty -- a completed meeting whose summarization failed/never
        // ran, or whose summary the user select-all-deleted (persisting `summary_edited
        // = ""`), must still offer this editable surface: decisions, open questions, and
        // action items live here too, and were previously unreachable once the summary
        // string was empty (Important #1).
        if meeting.status == .completed {
            VStack(alignment: .leading, spacing: Spacing.lg) {
                ZStack(alignment: .topLeading) {
                    if summaryDraftText.isEmpty {
                        Text(summaryPlaceholderText)
                            .font(.system(size: appState.readingFontSize + 2, design: .default))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                            .padding(.top, 8)
                            .padding(.leading, 5)
                            .allowsHitTesting(false)
                    }
                    TextEditor(text: $summaryDraftText)
                        .font(.system(size: appState.readingFontSize + 2, design: .default))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .scrollContentBackground(.hidden)
                        .frame(minHeight: 80)
                        .accessibilityIdentifier("meeting-note-summary-editor")
                        .accessibilityLabel("Summary")
                        .onChange(of: summaryDraftText) { _, _ in
                            guard !suppressSummaryAutosave else { return }
                            scheduleSummarySave()
                        }
                }
                editableList(title: "Decisions", items: $decisionsDraftItems, onChange: scheduleDecisionsSave)
                actionItemsSection
                editableList(title: "Open questions", items: $openQuestionsDraftItems, onChange: scheduleOpenQuestionsSave)
                provenanceChips
            }
            .whimbrelCard()
        } else {
            Text("Summary will be generated once the meeting ends.")
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    @ViewBuilder
    private var provenanceChips: some View {
        HStack(spacing: Spacing.xs) {
            if let currentProvider, let currentModel {
                WhimbrelMetadataChip(text: "Generated by \(currentModel) (\(currentProvider))")
                    .accessibilityIdentifier("meeting-note-provenance-chip")
            }
            if noteModel.summaryIsUserEdited {
                WhimbrelMetadataChip(text: "Edited by you")
            }
        }
    }

    /// An editable, reorderable list of short text rows -- used for both Decisions and
    /// Open questions. Bounds-checked bindings (rather than a bare `items[index]`)
    /// because index-identity `ForEach` plus a same-frame delete can otherwise hand a
    /// stale index to a still-in-flight `TextField` binding.
    @ViewBuilder
    private func editableList(title: String, items: Binding<[String]>, onChange: @escaping () -> Void) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text(title).font(.whimbrelBody(.caption).bold()).foregroundStyle(Color.whimbrelTextSecondary)
            ForEach(items.wrappedValue.indices, id: \.self) { index in
                let rowKey = "\(title)-\(index)"
                let isHovered = hoveredRowKey == rowKey
                HStack(spacing: Spacing.sm) {
                    TextField("", text: Binding(
                        get: { items.wrappedValue.indices.contains(index) ? items.wrappedValue[index] : "" },
                        set: { newValue in
                            guard items.wrappedValue.indices.contains(index) else { return }
                            items.wrappedValue[index] = newValue
                            onChange()
                        }
                    ), axis: .vertical)
                    .textFieldStyle(.plain)
                    .lineLimit(1...20)
                    .fixedSize(horizontal: false, vertical: true)
                    .font(.system(size: appState.readingFontSize, design: .default))
                    HStack(spacing: Spacing.sm) {
                        Button {
                            guard index > 0, items.wrappedValue.indices.contains(index) else { return }
                            items.wrappedValue.swapAt(index, index - 1)
                            onChange()
                        } label: {
                            Image(systemName: "chevron.up")
                        }
                        .buttonStyle(.plain)
                        .disabled(index == 0)
                        .help("Move up")
                        Button {
                            guard items.wrappedValue.indices.contains(index + 1) else { return }
                            items.wrappedValue.swapAt(index, index + 1)
                            onChange()
                        } label: {
                            Image(systemName: "chevron.down")
                        }
                        .buttonStyle(.plain)
                        .disabled(index == items.wrappedValue.count - 1)
                        .help("Move down")
                        Button {
                            guard items.wrappedValue.indices.contains(index) else { return }
                            items.wrappedValue.remove(at: index)
                            onChange()
                        } label: {
                            Image(systemName: "minus.circle")
                        }
                        .buttonStyle(.plain)
                        .help("Remove")
                    }
                    .opacity(isHovered ? 1 : 0)
                    .allowsHitTesting(isHovered)
                }
                .onHover { hovering in setHoveredRow(hovering, key: rowKey) }
            }
            Button("Add") { items.wrappedValue.append(""); onChange() }
                .buttonStyle(.whimbrelQuiet)
                .controlSize(.small)
        }
    }

    @ViewBuilder
    private var actionItemsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text("Action items").font(.whimbrelBody(.caption).bold()).foregroundStyle(Color.whimbrelTextSecondary)
            ForEach(actionItems) { item in
                let rowKey = "actionitem-\(item.id)"
                let isHovered = hoveredRowKey == rowKey
                HStack(spacing: Spacing.sm) {
                    Toggle("Completed", isOn: actionItemCompletedBinding(id: item.id))
                        .labelsHidden()
                    TextField("", text: actionItemTextBinding(id: item.id), axis: .vertical)
                        .lineLimit(1...20)
                        .fixedSize(horizontal: false, vertical: true)
                        .textFieldStyle(.plain)
                        .font(.system(size: appState.readingFontSize, design: .default))
                        .strikethrough(item.completed)
                    HStack(spacing: Spacing.sm) {
                        Button { moveActionItem(id: item.id, by: -1) } label: { Image(systemName: "chevron.up") }
                            .buttonStyle(.plain)
                            .help("Move up")
                        Button { moveActionItem(id: item.id, by: 1) } label: { Image(systemName: "chevron.down") }
                            .buttonStyle(.plain)
                            .help("Move down")
                        Button { deleteActionItem(id: item.id) } label: { Image(systemName: "minus.circle") }
                            .buttonStyle(.plain)
                            .help("Remove")
                    }
                    .opacity(isHovered ? 1 : 0)
                    .allowsHitTesting(isHovered)
                }
                .onHover { hovering in setHoveredRow(hovering, key: rowKey) }
                .accessibilityIdentifier("meeting-note-action-item-row-\(item.id)")
            }
            Button("Add action item") { addActionItem() }
                .buttonStyle(.whimbrelQuiet)
                .controlSize(.small)
                .accessibilityIdentifier("meeting-note-add-action-item-button")
        }
    }

    /// `CoreMeetingActionItem` is an immutable value type (Task 2's wire-format struct --
    /// every field a `let`), so a row's Toggle/TextField can't bind into it directly the
    /// way `ForEach($array) { $item in ... }` normally allows. These construct a fresh
    /// `Binding` that reads the current value out of `actionItems` by id and, on write,
    /// replaces that array element with a copy carrying the new value.
    private func actionItemCompletedBinding(id: Int64) -> Binding<Bool> {
        Binding(
            get: { actionItems.first(where: { $0.id == id })?.completed ?? false },
            set: { newValue in
                guard let index = actionItems.firstIndex(where: { $0.id == id }) else { return }
                let item = actionItems[index]
                // `updatedUnixMs` bumped locally too, not just on the eventual DB write --
                // `actionItemsHaveUserInteraction` reads THIS array at regeneration time,
                // not the DB, so a stale local timestamp would make a completed-then-
                // unchecked item look untouched again. See the other two bindings/
                // `moveActionItem` below for the same reasoning.
                actionItems[index] = CoreMeetingActionItem(
                    id: item.id, meetingId: item.meetingId, text: item.text, completed: newValue,
                    orderIndex: item.orderIndex, source: item.source, createdUnixMs: item.createdUnixMs,
                    updatedUnixMs: Int64(Date().timeIntervalSince1970 * 1000)
                )
                persistActionItemCompleted(id: id, completed: newValue)
            }
        )
    }

    private func actionItemTextBinding(id: Int64) -> Binding<String> {
        Binding(
            get: { actionItems.first(where: { $0.id == id })?.text ?? "" },
            set: { newValue in
                guard let index = actionItems.firstIndex(where: { $0.id == id }) else { return }
                let item = actionItems[index]
                // See `actionItemCompletedBinding`'s comment: `updatedUnixMs` must move
                // locally, in the same edit that mutates `text`, or a same-session
                // "edit a generated item's text, then Regenerate" sequence would evaluate
                // `actionItemsHaveUserInteraction` against a stale (pre-edit) timestamp and
                // silently let `replaceMeetingActionItemsFromGenerated` delete the row the
                // user just edited -- confirmed via final-review follow-up (2026-08-19):
                // the debounced `scheduleActionItemTextSave` write below lands ~1.5s later
                // and is far too slow to be the only place this timestamp updates.
                actionItems[index] = CoreMeetingActionItem(
                    id: item.id, meetingId: item.meetingId, text: newValue, completed: item.completed,
                    orderIndex: item.orderIndex, source: item.source, createdUnixMs: item.createdUnixMs,
                    updatedUnixMs: Int64(Date().timeIntervalSince1970 * 1000)
                )
                scheduleActionItemTextSave(id: id, text: newValue)
            }
        )
    }

    @ViewBuilder
    private var regenerateSection: some View {
        if transcriptText != nil {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                ProviderModelPicker(
                    selectedProvider: $selectedProvider,
                    ollamaModel: $ollamaModel,
                    selectedHostedModel: $selectedHostedModel
                )
                HStack {
                    Button(isRegenerating ? "Regenerating…" : "Regenerate") {
                        regenerate()
                    }
                    .disabled(isRegenerating)
                    if isRegenerating {
                        ProgressView().controlSize(.small)
                    }
                }
                if let regenerateError {
                    Text(regenerateError)
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelError)
                }
            }
        }
    }

    @ViewBuilder
    private var transcriptSection: some View {
        if let transcriptText {
            let chunks = MeetingDetailLogic.splitAtDividers(transcriptText)
            let parsed = chunks.map { CoreMemoryStore.parseTranscriptTurns(from: $0.body) }
            if parsed.allSatisfy(\.isEmpty) {
                // Unparseable/unexpected format -- fall back to the old flat rendering
                // rather than showing nothing.
                Text(transcriptText)
                    .font(.system(size: appState.readingFontSize, design: .default))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .textSelection(.enabled)
            } else {
                VStack(alignment: .leading, spacing: Spacing.md) {
                    ForEach(Array(chunks.enumerated()), id: \.offset) { index, chunk in
                        if let label = chunk.label { rejoinedDivider(label) }
                        ForEach(Array(parsed[index].enumerated()), id: \.offset) { _, turn in
                            VStack(alignment: .leading, spacing: Spacing.xs) {
                                HStack(spacing: Spacing.xs) {
                                    Text(turn.speaker).font(.whimbrelBody(.caption).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                                    Text(turn.timestamp).font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
                                }
                                Text(turn.text)
                                    .font(.system(size: appState.readingFontSize, design: .default))
                                    .foregroundStyle(Color.whimbrelTextPrimary)
                            }
                        }
                    }
                }
                .textSelection(.enabled)
            }
        } else if transcriptLoadFailed {
            Text("Transcript unavailable.").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
        } else if meeting.transcriptPath != nil, !transcriptLoadFailed {
            Text("Loading transcript…").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
        } else if meeting.status == .completed {
            Text(completedWithoutTranscriptText)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            Text("No transcript yet — the meeting hasn't ended.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    /// `transcriptPath` is a local file path, written on whichever device actually ran
    /// transcription — it never transfers between devices, so a meeting pulled in via
    /// sync has a real (non-nil) `transcriptPath` pointing at a file that simply doesn't
    /// exist on this Mac. `transcript_segment` rows sync independently and do carry the
    /// real content, so a missing local file falls back to reconstructing the transcript
    /// from those — this is what makes a meeting captured on one Mac actually readable
    /// on another (confirmed missing via live dogfooding 2026-08-05: meeting entry
    /// synced, transcript did not).
    private func loadTranscriptIfNeeded() {
        guard transcriptText == nil, !transcriptLoadFailed, let path = meeting.transcriptPath else { return }
        let parts = parts
        DispatchQueue.global(qos: .userInitiated).async {
            if parts.count > 1,
               let text = MeetingDetailLogic.joinedTranscript(parts: parts, read: { try? String(contentsOfFile: $0, encoding: .utf8) }) {
                DispatchQueue.main.async { transcriptText = text }
                return
            }
            if let text = try? String(contentsOfFile: path, encoding: .utf8) {
                DispatchQueue.main.async { transcriptText = text }
                return
            }
            loadTranscriptFromSyncedSegments()
        }
    }

    /// The local-file read failed — either genuinely missing, or (the sync case this
    /// exists for) a meeting pulled in via sync whose `transcriptPath` is a file path
    /// from a different Mac that never transferred here. `transcript_segment` rows sync
    /// independently and do carry real content, so this reconstructs the transcript
    /// from those instead of giving up (confirmed missing via live dogfooding
    /// 2026-08-05: meeting entry synced, transcript did not). FFI calls must serialize
    /// through `CoreMemoryStore.accessQueue`, not the generic global queue the caller's
    /// file read ran on (see `CoreMemoryStore`'s own not-thread-safe contract).
    private func loadTranscriptFromSyncedSegments() {
        guard let store = appState.coreStore else {
            DispatchQueue.main.async { transcriptLoadFailed = true }
            return
        }
        CoreMemoryStore.accessQueue.async {
            var allSegments: [CoreMeetingTranscriptSegment] = []
            var offset = 0
            let pageSize = 500
            var sawDbError = false
            while true {
                guard let page = store.getTranscriptSegmentsPage(meetingId: meeting.id, offset: offset, limit: pageSize) else {
                    sawDbError = true // a DB error mid-page can't be trusted as complete
                    break
                }
                allSegments.append(contentsOf: page.segments)
                offset += page.segments.count
                if page.segments.isEmpty || allSegments.count >= page.total {
                    break
                }
            }

            guard !sawDbError, !allSegments.isEmpty else {
                DispatchQueue.main.async { transcriptLoadFailed = true }
                return
            }

            // Reassembles the same "[HH:mm:ss] Speaker: text" shape TranscriptionService
            // writes to the local file, using the meeting's real start time as the
            // session anchor, so regenerate() (which sends this text straight to the
            // LLM) sees materially the same input either way.
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.timeZone = .current
            formatter.dateFormat = "HH:mm:ss"
            let sessionStart = Date(timeIntervalSince1970: Double(meeting.startUnixMs) / 1000)

            let reconstructed = allSegments
                .sorted { $0.startCs < $1.startCs }
                .map { segment -> String in
                    let lineDate = sessionStart.addingTimeInterval(Double(segment.startCs) / 100.0)
                    let label = formatter.string(from: lineDate)
                    return "[\(label)] \(segment.speakerLabel): \(segment.text)"
                }
                .joined(separator: "\n")

            DispatchQueue.main.async { transcriptText = reconstructed }
        }
    }

    private func regenerate() {
        guard let transcriptText, !isRegenerating else { return }
        isRegenerating = true
        regenerateError = nil
        let routingConfig = appState.routing.routingConfig(preferredProvider: selectedProvider, preferredModel: currentModelLabel)
        appState.regenerateMeetingSummary(
            meetingId: meeting.id,
            notes: meeting.notes ?? "",
            transcript: transcriptText,
            ollamaModel: ollamaModel,
            routing: routingConfig,
            customInstructions: appState.promptLibrary.instructions[.meetingSummary] ?? ""
        ) { outcome in
            isRegenerating = false
            switch outcome {
            case .some(.success(let summary)):
                // The FFI call above (`Store::update_meeting_summary`, unmodified) has
                // already persisted the new summary_* columns -- this only decides what
                // the editable-overlay layer does with the draft/action items sitting on
                // top of them. No timestamp for "when was this generation produced"
                // exists anywhere in the summary payload or the `meeting` row, so one is
                // minted here and used consistently as this generation's identity for
                // both the conflict check and every `markMeetingRegenerationSeen` call
                // below.
                let newGenerationUnixMs = Int64(Date().timeIntervalSince1970 * 1000)
                // Two independent things a silent regeneration could clobber: structured
                // draft overrides (summary/decisions/open-questions) and action items the
                // user has actually interacted with. Either one alone is enough to require
                // the explicit prompt -- see Important #3 in the final review: action items
                // used to be silently dropped whenever rows already existed (idempotent
                // seeding, correct for first load, wrong here) even though nothing had
                // touched them, or worse, even when the user HAD interacted with them.
                let needsConflictPrompt = regenerationNeedsConflictPrompt(existingDraft: draftReflectingUnsavedEdits, newGenerationUnixMs: newGenerationUnixMs)
                    || actionItemsHaveUserInteraction(existing: actionItems)
                if needsConflictPrompt {
                    pendingConflict = PendingRegenerationConflict(summary: summary, newGenerationUnixMs: newGenerationUnixMs)
                } else {
                    applyRegeneratedSummary(summary, newGenerationUnixMs: newGenerationUnixMs)
                }
            case .some(.failure(let error)):
                regenerateError = error.userFacingMessage
            case .none:
                // Genuinely invalid usage (null handle, malformed args) -- shouldn't
                // happen in practice; falls back to a generic message rather than
                // showing nothing, since this path (unlike Ask-chat) already has a
                // visible error slot in the UI.
                regenerateError = "Couldn't generate a summary. Try again."
            }
        }
    }

    // MARK: - Note-overrides autosave

    private static let autosaveDebounceNanoseconds: UInt64 = 1_500_000_000

    private func scheduleSummarySave() {
        summarySaveTask?.cancel()
        let meetingId = meeting.id
        let text = summaryDraftText
        summarySaveTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: Self.autosaveDebounceNanoseconds)
            if Task.isCancelled { return }
            guard let store = appState.coreStore else { return }
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            CoreMemoryStore.accessQueue.async {
                _ = store.saveNoteOverrides(meetingId: meetingId, summaryEdited: text, decisionsEdited: nil, openQuestionsEdited: nil, nowUnixMs: nowMs)
                DispatchQueue.main.async {
                    draft = CoreMeetingNoteDraft(
                        meetingId: meetingId, summaryEdited: text, decisionsEdited: draft?.decisionsEdited,
                        openQuestionsEdited: draft?.openQuestionsEdited, personalNotes: draft?.personalNotes,
                        lastEditedUnixMs: nowMs, lastRegeneratedSeenUnixMs: draft?.lastRegeneratedSeenUnixMs
                    )
                }
            }
        }
    }

    private func scheduleDecisionsSave() {
        decisionsSaveTask?.cancel()
        let meetingId = meeting.id
        let items = decisionsDraftItems
        decisionsSaveTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: Self.autosaveDebounceNanoseconds)
            if Task.isCancelled { return }
            guard let store = appState.coreStore else { return }
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            CoreMemoryStore.accessQueue.async {
                _ = store.saveNoteOverrides(meetingId: meetingId, summaryEdited: nil, decisionsEdited: items, openQuestionsEdited: nil, nowUnixMs: nowMs)
                DispatchQueue.main.async {
                    draft = CoreMeetingNoteDraft(
                        meetingId: meetingId, summaryEdited: draft?.summaryEdited, decisionsEdited: items,
                        openQuestionsEdited: draft?.openQuestionsEdited, personalNotes: draft?.personalNotes,
                        lastEditedUnixMs: nowMs, lastRegeneratedSeenUnixMs: draft?.lastRegeneratedSeenUnixMs
                    )
                }
            }
        }
    }

    private func scheduleOpenQuestionsSave() {
        openQuestionsSaveTask?.cancel()
        let meetingId = meeting.id
        let items = openQuestionsDraftItems
        openQuestionsSaveTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: Self.autosaveDebounceNanoseconds)
            if Task.isCancelled { return }
            guard let store = appState.coreStore else { return }
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            CoreMemoryStore.accessQueue.async {
                _ = store.saveNoteOverrides(meetingId: meetingId, summaryEdited: nil, decisionsEdited: nil, openQuestionsEdited: items, nowUnixMs: nowMs)
                DispatchQueue.main.async {
                    draft = CoreMeetingNoteDraft(
                        meetingId: meetingId, summaryEdited: draft?.summaryEdited, decisionsEdited: draft?.decisionsEdited,
                        openQuestionsEdited: items, personalNotes: draft?.personalNotes,
                        lastEditedUnixMs: nowMs, lastRegeneratedSeenUnixMs: draft?.lastRegeneratedSeenUnixMs
                    )
                }
            }
        }
    }

    // MARK: - Action items

    private func persistActionItemCompleted(id: Int64, completed: Bool) {
        guard let store = appState.coreStore else { return }
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        CoreMemoryStore.accessQueue.async {
            _ = store.setMeetingActionItemCompleted(id: id, completed: completed, nowUnixMs: nowMs)
        }
    }

    private func scheduleActionItemTextSave(id: Int64, text: String) {
        actionItemTextSaveTasks[id]?.cancel()
        guard let store = appState.coreStore else { return }
        actionItemTextSaveTasks[id] = Task { @MainActor in
            try? await Task.sleep(nanoseconds: Self.autosaveDebounceNanoseconds)
            if Task.isCancelled { return }
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            CoreMemoryStore.accessQueue.async {
                _ = store.updateMeetingActionItemText(id: id, text: text, nowUnixMs: nowMs)
            }
        }
    }

    private func deleteActionItem(id: Int64) {
        guard let store = appState.coreStore else { return }
        actionItemTextSaveTasks[id]?.cancel()
        actionItemTextSaveTasks[id] = nil
        actionItems.removeAll { $0.id == id }
        CoreMemoryStore.accessQueue.async {
            _ = store.deleteMeetingActionItem(id: id)
        }
    }

    private func addActionItem() {
        guard let store = appState.coreStore else { return }
        let meetingId = meeting.id
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let orderIndex = nextActionItemOrderIndex(existing: actionItems)
        CoreMemoryStore.accessQueue.async {
            guard let newId = store.insertMeetingActionItem(meetingId: meetingId, text: "", orderIndex: orderIndex, nowUnixMs: nowMs) else { return }
            DispatchQueue.main.async {
                actionItems.append(CoreMeetingActionItem(
                    id: newId, meetingId: meetingId, text: "", completed: false, orderIndex: orderIndex,
                    source: "user", createdUnixMs: nowMs, updatedUnixMs: nowMs
                ))
            }
        }
    }

    /// `by` is `-1` for "move up" / `+1` for "move down" -- explicit buttons, not
    /// drag-and-drop-only, per the spec's accessibility requirement.
    private func moveActionItem(id: Int64, by offset: Int) {
        guard let index = actionItems.firstIndex(where: { $0.id == id }) else { return }
        let newIndex = index + offset
        guard actionItems.indices.contains(newIndex) else { return }
        actionItems.swapAt(index, newIndex)
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        // Same reasoning as the two bindings above: both moved rows' `updatedUnixMs` must
        // bump locally, in this same reorder, or `actionItemsHaveUserInteraction` reads a
        // stale (pre-reorder) timestamp for the rest of this view's lifetime.
        for i in [index, newIndex] {
            let item = actionItems[i]
            actionItems[i] = CoreMeetingActionItem(
                id: item.id, meetingId: item.meetingId, text: item.text, completed: item.completed,
                orderIndex: item.orderIndex, source: item.source, createdUnixMs: item.createdUnixMs, updatedUnixMs: nowMs
            )
        }
        guard let store = appState.coreStore else { return }
        let meetingId = meeting.id
        let orderedIds = actionItems.map(\.id)
        CoreMemoryStore.accessQueue.async {
            _ = store.reorderMeetingActionItems(meetingId: meetingId, orderedIds: orderedIds, nowUnixMs: nowMs)
        }
    }

    // MARK: - Regeneration + conflict resolution

    /// The no-conflict path: `regenerate()` only reaches this when NEITHER the draft nor
    /// the action items have anything a silent overwrite could clobber (see
    /// `regenerationNeedsConflictPrompt` and `actionItemsHaveUserInteraction`). Applies
    /// the fresh summary and REPLACES action items from the new generation -- not
    /// `seedMeetingActionItems`, which no-ops whenever any rows already exist (correct
    /// for the very first load, wrong here: rows always exist after that first load, so
    /// every later regeneration on an untouched meeting used to silently drop the new
    /// generated items -- Important #3 in the final review). `replaceMeetingActionItemsFromGenerated`
    /// is safe unconditionally in this branch precisely because we've already confirmed
    /// there's no `source == "user"` row and nothing completed/edited to lose.
    private func applyRegeneratedSummary(_ summary: CoreMeetingSummary, newGenerationUnixMs: Int64) {
        summarySaveTask?.cancel()
        decisionsSaveTask?.cancel()
        openQuestionsSaveTask?.cancel()
        regeneratedSummary = summary
        setSummaryDraftTextProgrammatically(summary.tldr)
        decisionsDraftItems = summary.decisions
        openQuestionsDraftItems = summary.openQuestions
        guard let store = appState.coreStore else { return }
        let meetingId = meeting.id
        let texts = summary.actionItems
        CoreMemoryStore.accessQueue.async {
            _ = store.replaceMeetingActionItemsFromGenerated(meetingId: meetingId, texts: texts, nowUnixMs: newGenerationUnixMs)
            let refreshed = store.listMeetingActionItems(meetingId: meetingId)
            DispatchQueue.main.async {
                actionItems = refreshed
            }
        }
    }

    private func resolveRegenerationConflict(_ choice: RegenerationConflictChoice, conflict: PendingRegenerationConflict) {
        guard let store = appState.coreStore else {
            pendingConflict = nil
            return
        }
        let meetingId = meeting.id
        let summary = conflict.summary
        let newGenerationUnixMs = conflict.newGenerationUnixMs
        switch choice {
        case .replace:
            summarySaveTask?.cancel()
            decisionsSaveTask?.cancel()
            openQuestionsSaveTask?.cancel()
            CoreMemoryStore.accessQueue.async {
                _ = store.resetMeetingNoteDraft(meetingId: meetingId, nowUnixMs: newGenerationUnixMs)
                _ = store.replaceMeetingActionItemsFromGenerated(meetingId: meetingId, texts: summary.actionItems, nowUnixMs: newGenerationUnixMs)
                _ = store.markMeetingRegenerationSeen(meetingId: meetingId, regeneratedUnixMs: newGenerationUnixMs)
                let refreshedDraft = store.getMeetingNoteDraft(meetingId: meetingId)
                let refreshedItems = store.listMeetingActionItems(meetingId: meetingId)
                DispatchQueue.main.async {
                    draft = refreshedDraft
                    actionItems = refreshedItems
                    regeneratedSummary = summary
                    setSummaryDraftTextProgrammatically(summary.tldr)
                    decisionsDraftItems = summary.decisions
                    openQuestionsDraftItems = summary.openQuestions
                    pendingConflict = nil
                }
            }
        case .keepMine:
            CoreMemoryStore.accessQueue.async {
                _ = store.markMeetingRegenerationSeen(meetingId: meetingId, regeneratedUnixMs: newGenerationUnixMs)
                DispatchQueue.main.async {
                    // The generated columns are already updated (existing regenerate()
                    // call, unmodified) -- refresh the provider/model attribution and the
                    // "no override yet" fallback path, but leave the user's summary/
                    // decisions/open-questions overrides (and the local fields showing
                    // them) untouched.
                    regeneratedSummary = summary
                    pendingConflict = nil
                }
            }
        case .compare:
            break // RegenerationConflictSheet handles this entirely with its own @State.
        }
    }

    private static func formattedMeetingTime(startUnixMs: Int64, endUnixMs: Int64?) -> String {
        let start = Date(timeIntervalSince1970: TimeInterval(startUnixMs) / 1000)
        let startText = start.formatted(date: .abbreviated, time: .shortened)
        guard let endUnixMs else { return startText }
        let end = Date(timeIntervalSince1970: TimeInterval(endUnixMs) / 1000)
        let minutes = Int((end.timeIntervalSince1970 - start.timeIntervalSince1970) / 60)
        return "\(startText) · \(minutes) min"
    }
}

/// Pure helpers for the "— rejoined HH:MM —" dividers of a continued meeting's transcript.
/// The label matches core's `meeting_transcript_text` (local time, from the part's start).
enum MeetingDetailLogic {
    private static let dividerPrefix = "\u{2014} rejoined "
    private static let dividerSuffix = " \u{2014}"

    static func dividerLabel(startUnixMs: Int64) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "HH:mm"
        return dividerPrefix + f.string(from: Date(timeIntervalSince1970: Double(startUnixMs) / 1000)) + dividerSuffix
    }

    /// One divider per part after the first, at the part's offset (centiseconds) from part 1.
    static func dividers(parts: [CoreMeetingPart]) -> [(beforeStartCs: Int64, label: String)] {
        guard let first = parts.first else { return [] }
        return parts.dropFirst().map { ((($0.startUnixMs - first.startUnixMs) / 10), dividerLabel(startUnixMs: $0.startUnixMs)) }
    }

    /// Segment index -> the divider shown before it: the first segment at or past a divider's
    /// offset gets it; a divider no segment reaches is dropped, and when two reach the same
    /// segment the later one wins (as in core).
    static func dividerLabels(segmentStartCs: [Int64], dividers: [(beforeStartCs: Int64, label: String)]) -> [Int: String] {
        var out: [Int: String] = [:]
        for d in dividers {
            if let i = segmentStartCs.firstIndex(where: { $0 >= d.beforeStartCs }) { out[i] = d.label }
        }
        return out
    }

    /// The parts' transcript files in order, a divider before each later part that has one.
    static func joinedTranscript(parts: [CoreMeetingPart], read: (String) -> String?) -> String? {
        var pieces: [String] = []
        for (i, p) in parts.enumerated() {
            guard let path = p.transcriptPath, let text = read(path),
                  !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { continue }
            if i > 0 { pieces.append(dividerLabel(startUnixMs: p.startUnixMs)) }
            pieces.append(text.trimmingCharacters(in: .newlines))
        }
        return pieces.isEmpty ? nil : pieces.joined(separator: "\n")
    }

    /// Splits transcript text at its divider lines: each chunk's label is the divider above it.
    static func splitAtDividers(_ text: String) -> [(label: String?, body: String)] {
        var out: [(label: String?, body: String)] = [(nil, "")]
        for line in text.components(separatedBy: "\n") {
            if line.hasPrefix(dividerPrefix), line.hasSuffix(dividerSuffix) {
                out.append((line, ""))
            } else {
                out[out.count - 1].body += out[out.count - 1].body.isEmpty ? line : "\n" + line
            }
        }
        return out.filter { $0.label != nil || !$0.body.isEmpty }
    }
}
