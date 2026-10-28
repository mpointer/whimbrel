import SwiftUI
import Combine

enum NoteSaveState: Equatable {
    case idle
    case saving
    case saved
    case error
}

/// Debounced autosave for the During-state note field, backed by the new
/// `meeting_note_draft.personal_notes` column (Task 2) — NOT `AppState.meetingNotes`,
/// which is in-memory-only and cleared on `endMeeting()`, so it can't survive a crash
/// or app restart mid-meeting. `onSave` is injected so this is testable without a real
/// `CoreMemoryStore`/`accessQueue` round trip.
@MainActor
final class ActiveNoteAutosaveController: ObservableObject {
    @Published var noteText: String = "" {
        didSet { if !seeding { scheduleSave() } }
    }
    @Published private(set) var saveState: NoteSaveState = .idle

    private let debounceSeconds: TimeInterval
    private let onSave: (String, Int64) -> Bool
    private var saveTask: Task<Void, Never>?
    private var seeding = false

    init(debounceSeconds: TimeInterval = 1.5, onSave: @escaping (String, Int64) -> Bool) {
        self.debounceSeconds = debounceSeconds
        self.onSave = onSave
    }

    /// Sets the text from a loaded draft without scheduling a save, so it is never written back.
    func seed(_ text: String) {
        seeding = true
        noteText = text
        seeding = false
    }

    /// Writes a pending edit now (the view is going away); no-op when nothing is pending.
    func flush() {
        guard saveState == .saving else { return }
        saveTask?.cancel()
        saveTask = nil
        saveState = onSave(noteText, Int64(Date().timeIntervalSince1970 * 1000)) ? .saved : .error
    }

    private func scheduleSave() {
        saveTask?.cancel()
        saveState = .saving
        let text = noteText
        let onSave = onSave
        saveTask = Task { [weak self] in
            if debounceSeconds > 0 {
                try? await Task.sleep(nanoseconds: UInt64(debounceSeconds * 1_000_000_000))
                if Task.isCancelled { return }
            }
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            let succeeded = onSave(text, nowMs)
            // No `await MainActor.run` hop here: this Task already runs on the MainActor
            // (inherited from `scheduleSave`, itself `@MainActor`-isolated), and a hop would
            // insert a suspension point between `onSave` (where callers/tests commonly
            // signal completion) and this state write — letting an XCTestExpectation-based
            // wait return before `saveState` actually updates. Direct assignment keeps the
            // signal and the state write in the same uninterrupted execution slice.
            self?.saveState = succeeded ? .saved : .error
        }
    }
}

/// The During-state main canvas: a large, distraction-free note editor plus an
/// unambiguous, non-decorative recording indicator, elapsed time, and Stop control
/// wired to the EXISTING `AppState.endActiveMeetingIfNeeded()` — this view never
/// implements its own recording lifecycle.
struct ActiveMeetingView: View {
    let model: ActiveMeetingModel
    let initialNotes: String
    /// Injected so the view doesn't reach into `CoreMemoryStore.accessQueue` directly —
    /// the caller (a container that owns `appState`) dispatches the real write.
    let onSaveNotes: (String, Int64) -> Bool
    let onStop: () -> Void
    /// The live transcript pane and Ask bar, when live transcription is on (built by the host,
    /// which has `appState` in the environment). Nil leaves the notes editor alone.
    let liveSection: AnyView?

    @StateObject private var autosave: ActiveNoteAutosaveController
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(
        model: ActiveMeetingModel, initialNotes: String, onSaveNotes: @escaping (String, Int64) -> Bool,
        onStop: @escaping () -> Void, liveSection: AnyView? = nil
    ) {
        self.model = model
        self.initialNotes = initialNotes
        self.onSaveNotes = onSaveNotes
        self.onStop = onStop
        self.liveSection = liveSection
        _autosave = StateObject(wrappedValue: ActiveNoteAutosaveController(onSave: onSaveNotes))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(Color.whimbrelBorder)
            if let liveSection { liveSection }
            TextEditor(text: $autosave.noteText)
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .scrollContentBackground(.hidden)
                .padding(Spacing.xl)
                .accessibilityIdentifier("active-meeting-note-editor")
                .accessibilityLabel("Meeting notes")
        }
        .background(Color.whimbrelBackgroundPage)
        .onAppear { autosave.seed(initialNotes) }
        .onDisappear { autosave.flush() }
    }

    private var header: some View {
        HStack(spacing: Spacing.lg) {
            recordingIndicator
            VStack(alignment: .leading, spacing: 2) {
                Text(model.meeting.title)
                    .font(.whimbrelDisplay(.title3))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                Text(elapsedLabel)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .accessibilityIdentifier("active-meeting-elapsed-time")
            }
            Spacer()
            saveStateLabel
            Button("Stop meeting", action: onStop)
                .buttonStyle(.whimbrelSecondary)
                .accessibilityIdentifier("active-meeting-stop-button")
        }
        .padding(Spacing.lg)
    }

    private var recordingIndicator: some View {
        // Non-color status indication (required by the spec's accessibility bar):
        // shape + a text label, not a bare colored dot. The pulse animation is purely
        // decorative and is disabled under Reduce Motion — the label and shape alone
        // still communicate the state.
        HStack(spacing: Spacing.xs) {
            Circle()
                .fill(Color.whimbrelError)
                .frame(width: 8, height: 8)
                .opacity(reduceMotion ? 1 : (model.isProcessing ? 0.5 : 1))
            Text("Recording")
                .font(.whimbrelBody(.caption2).weight(.semibold))
                .foregroundStyle(Color.whimbrelTextPrimary)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Recording in progress")
        .accessibilityIdentifier("active-meeting-recording-indicator")
    }

    private var saveStateLabel: some View {
        Group {
            switch autosave.saveState {
            case .idle: EmptyView()
            case .saving: Text("Saving…")
            case .saved: Text("Saved")
            case .error: Text("Couldn't save")
            }
        }
        .font(.whimbrelBody(.caption2))
        .foregroundStyle(autosave.saveState == .error ? .red : Color.whimbrelTextSecondary)
        .accessibilityIdentifier("active-meeting-save-state")
    }

    private var elapsedLabel: String {
        let minutes = model.elapsedSeconds / 60
        let seconds = model.elapsedSeconds % 60
        return String(format: "%d:%02d elapsed", minutes, seconds)
    }
}
