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
