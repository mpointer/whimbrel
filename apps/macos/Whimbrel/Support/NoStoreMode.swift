import SwiftUI

/// Running without the memory database (X4 Task 11b-2, Mike's decision 1B): after a failed
/// plain open the user may "Continue without the memory database". Then no recording starts
/// by any path (its rows and files need the store), background screen capture, meeting
/// detection and sync stay off, and a notice shows in the main window and the menu bar.
/// With a store every answer is yes and nothing changes.
@MainActor
final class RecordingStartGate {
    /// Every way a meeting recording starts (`AppState.startManualMeeting` and
    /// `AppState.startCapture(for:trigger:)`; there is no phone- or MCP-triggered start).
    enum Source: CaseIterable {
        /// "Start Meeting" in the menu bar, the timeline header or Today.
        case manual
        /// A calendar meeting: the prompt's Record, its auto-start, Today's Record, a split.
        case calendarOrPrompt
        /// A detected call the user accepted.
        case detectedMeeting
        /// Away Mode joining a meeting unattended.
        case awayMode

        init(trigger: CoreMeetingTrigger) {
            switch trigger {
            case .manual: self = .manual
            case .calendar: self = .calendarOrPrompt
            case .detected: self = .detectedMeeting
            case .away: self = .awayMode
            }
        }
    }

    let storeOpen: Bool
    /// Tells the user why a start did not happen: in the window (`RecordingRefusal`), and as a
    /// notification (secondary: none shows while Whimbrel is in front).
    var notify: (String) -> Void

    init(storeOpen: Bool, notify: @escaping (String) -> Void = { _ in }) {
        self.storeOpen = storeOpen
        self.notify = notify
    }

    /// Whether `source` may start a recording now; when not, says why. The on-screen Start
    /// controls are disabled without a store (`RecordingControls`), so this mostly catches
    /// prompts, Away Mode and links.
    func allow(_ source: Source) -> Bool {
        if storeOpen { return true }
        notify(LibraryCopyRedesign.map(EncryptionCopy.recordingBlockedNoStore))
        return false
    }

    /// Screen text capture writes files the store ingests later: off without a store.
    var allowsBackgroundCapture: Bool { storeOpen }
    /// Detection and calendar prompts only lead to recordings: off without a store.
    var allowsMeetingDetection: Bool { storeOpen }
    /// Content and settings sync pull store-backed items: not started without a store, so
    /// nothing is pulled, applied or acknowledged.
    var allowsSync: Bool { storeOpen }
    /// The persistent notice outside Settings.
    var showsNotice: Bool { !storeOpen }
}

/// What a start does with the answer of the meeting-row insert. A recording needs its row: audio,
/// snapshots and the end-of-meeting steps all hang off the meeting id, so a start whose insert
/// failed must refuse, never carry on recording with no row.
enum RecordingStartInsert {
    enum Outcome: Equatable {
        case proceed(meetingId: Int64)
        /// Show this in the window (`RecordingRefusal`) and as a notification; start nothing.
        case refuse(message: String)
    }

    static func outcome(insertedMeetingId: Int64?) -> Outcome {
        guard let insertedMeetingId else { return .refuse(message: LibraryCopyRedesign.map(EncryptionCopy.recordingNotSaved)) }
        return .proceed(meetingId: insertedMeetingId)
    }
}

/// Starts whose meeting-row insert is queued or running. Quitting waits for them (a short,
/// bounded time), so an insert already queued behind other store work is not thrown away with
/// the process: the row is written, and the next launch completes it if no audio ever started.
/// The wait never blocks the main thread: the continue path (`AppState.continueCapture`) runs
/// its first step on main, so a blocking wait there would hang quit for the whole cap and the
/// reopen would never be written (#850 review, I3).
final class PendingRecordingStarts: @unchecked Sendable {
    static let shared = PendingRecordingStarts()
    /// How long quitting waits for pending starts. The meeting-row insert gives up within about
    /// 15 s (`whimbrel_memory_insert_meeting`: 5 s per attempt, 10 s of attempts), so the row is
    /// covered. The writes after it (calendar source, attendee details, the part row, and on the
    /// continue path the reopen) each wait up to the connection's 30 s busy timeout, so under heavy
    /// contention quitting can end before they land; the next launch then finds the row without them.
    static let quitWaitSeconds: TimeInterval = 20

    private let group = DispatchGroup()
    private let lock = NSLock()
    private var count = 0
    /// A quit is already waiting (main only): a second Cmd-Q does not start a second wait.
    private var quitWaiting = false

    /// A start queued its insert. Pair every call with one `end()`.
    func begin() {
        lock.lock(); count += 1; lock.unlock()
        group.enter()
    }

    func end() {
        lock.lock(); count -= 1; lock.unlock()
        group.leave()
    }

    var pending: Int { lock.lock(); defer { lock.unlock() }; return count }

    /// Calls `done` on the main queue exactly once: with `true` as soon as no start is pending
    /// (at once if none is), or with `false` after `timeout`. Never blocks the caller, so a
    /// pending start whose completion needs the main thread still finishes.
    func whenSettled(timeout: TimeInterval, done: @escaping (Bool) -> Void) {
        let fired = NSLock()
        var didFire = false
        let fire = { (settled: Bool) in
            fired.lock()
            let first = !didFire
            didFire = true
            fired.unlock()
            if first { done(settled) }
        }
        group.notify(queue: .main) { fire(true) }
        DispatchQueue.main.asyncAfter(deadline: .now() + timeout) { fire(false) }
    }

    /// The quit answer (`AppTerminationGuard`): `.terminateNow` when no start is pending.
    /// Otherwise `.terminateLater`, and `reply` runs on main once the starts land or `timeout`
    /// passes, whichever is first; the caller then lets the app quit
    /// (`NSApp.reply(toApplicationShouldTerminate: true)`). A quit asked for while one is
    /// already waiting is cancelled: the waiting one still quits.
    @MainActor
    func quitReply(timeout: TimeInterval, reply: @escaping () -> Void) -> NSApplication.TerminateReply {
        guard pending > 0 else { return .terminateNow }
        guard !quitWaiting else { return .terminateCancel }
        quitWaiting = true
        whenSettled(timeout: timeout) { [self] _ in
            quitWaiting = false
            reply()
        }
        return .terminateLater
    }
}

/// The Start and capture controls: disabled without a store, with a caption (menu bar,
/// timeline header) or a tooltip (Today's Start buttons in every skin).
struct RecordingControls: Equatable {
    let enabled: Bool
    let caption: String?

    init(storeOpen: Bool) {
        enabled = storeOpen
        caption = storeOpen ? nil : EncryptionCopy.recordingOffCaption
    }
}

private struct RecordingControlsKey: EnvironmentKey {
    static let defaultValue = RecordingControls(storeOpen: true)
}

extension EnvironmentValues {
    var recordingControls: RecordingControls {
        get { self[RecordingControlsKey.self] }
        set { self[RecordingControlsKey.self] = newValue }
    }
}

private struct RecordingControlModifier: ViewModifier {
    @Environment(\.recordingControls) private var controls

    func body(content: Content) -> some View {
        content.disabled(!controls.enabled).help(controls.caption ?? "")
    }
}

extension View {
    /// A control that starts recording or capture: off without a store.
    func recordingControl() -> some View { modifier(RecordingControlModifier()) }
}

/// The caption shown under the menu bar's and the timeline's Start controls without a store.
struct RecordingOffCaption: View {
    @Environment(\.recordingControls) private var controls

    var body: some View {
        if let caption = controls.caption {
            Text(caption).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelWarning)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// A refused start, shown in the window (an alert) until dismissed.
@MainActor
final class RecordingRefusal: ObservableObject {
    @Published private(set) var message: String?
    func show(_ message: String) { self.message = message }
    func dismiss() { message = nil }
}

struct RecordingRefusalAlert: ViewModifier {
    @ObservedObject var refusal: RecordingRefusal

    func body(content: Content) -> some View {
        content.alert(refusal.message ?? "", isPresented: Binding(
            get: { refusal.message != nil }, set: { if !$0 { refusal.dismiss() } })) {
            Button(EncryptionCopy.dismiss) { refusal.dismiss() }
        }
    }
}

/// "The memory database could not be opened. Recording is off. Restart Whimbrel to try
/// again." with Open Settings, at
/// the top of the main window and the menu bar while running without a store.
struct NoStoreNotice: View {
    let openSettings: () -> Void

    var body: some View {
        // One row in the main window; text above the button in the narrow menu bar window.
        ViewThatFits(in: .horizontal) {
            HStack(spacing: Spacing.md) {
                icon
                message.fixedSize()
                Spacer(minLength: Spacing.sm)
                button
            }
            VStack(alignment: .leading, spacing: Spacing.sm) {
                HStack(alignment: .top, spacing: Spacing.sm) { icon; message }
                button
            }
        }
        .padding(.horizontal, Spacing.lg)
        .padding(.vertical, Spacing.sm)
        .background(Color.whimbrelWarning.opacity(0.12))
    }
}

private extension NoStoreNotice {
    var icon: some View { Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Color.whimbrelWarning) }
    var message: some View {
        Text(LibraryCopyRedesign.map(EncryptionCopy.noStoreNotice)).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextPrimary)
            .fixedSize(horizontal: false, vertical: true)
    }
    var button: some View { Button(EncryptionCopy.openSettingsAction, action: openSettings).buttonStyle(.whimbrelSecondary) }
}

/// "Your library couldn't be found..." (X4 Task 13c item 4): once, at the top of the main
/// window and the menu bar (one note, dismissed from either), after a launch that found neither the database nor a copy where a store had been
/// and opened a new empty one. Non-blocking; Dismiss removes it for this launch, and the
/// next launch finds the new database, so it never shows again.
struct LibraryMissingNoticeHost: View {
    @ObservedObject var coordinator: LaunchCoordinator<CoreMemoryStore>

    var body: some View {
        if coordinator.notes.contains(.libraryMissing) {
            HStack(alignment: .top, spacing: Spacing.md) {
                Image(systemName: "info.circle.fill").foregroundStyle(Color.whimbrelWarning)
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    Text(EncryptionCopy.libraryMissingNotice).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                    // The help page the notice names (the app's help convention: a website link).
                    Link(EncryptionCopy.libraryBackupHelpLink, destination: EncryptionCopy.libraryBackupHelpURL)
                        .font(.whimbrelBody(.callout))
                }
                Spacer(minLength: Spacing.sm)
                Button(EncryptionCopy.dismiss) { coordinator.dismissLibraryMissingNotice() }.buttonStyle(.whimbrelSecondary)
            }
            .padding(.horizontal, Spacing.lg)
            .padding(.vertical, Spacing.sm)
            .background(Color.whimbrelWarning.opacity(0.12))
        }
    }
}

/// Opens Settings on the section that shows why (Capture & Privacy, the encryption card).
struct NoStoreNoticeHost: View {
    @EnvironmentObject private var appState: AppState
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        if appState.recordingGate.showsNotice {
            NoStoreNotice {
                appState.pendingSettingsSection = .capture
                NSApp.activate(ignoringOtherApps: true)
                openSettings()
            }
        }
    }
}
