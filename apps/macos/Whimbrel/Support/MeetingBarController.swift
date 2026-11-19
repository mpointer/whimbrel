import AppKit
import SwiftUI
import WhimbrelUI

/// The in-meeting window's panel: a small always-on-top window with the recording controls. Built like the hotkey
/// panel (`HotkeyPanel`), so it can take typing while the app the person was in stays active. Created only when a
/// recording starts with the redesign flag on; with the flag off it is never built.
final class MeetingBarPanel: NSPanel {
    var onCancel: (() -> Void)?

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    /// Escape: focus goes back to the app the person was in. The window stays.
    override func cancelOperation(_ sender: Any?) { onCancel?() }
}

@MainActor
final class MeetingBarController {
    static let frameName = "WhimbrelMeetingBar"
    private static let margin: CGFloat = 16
    /// True keeps the window out of screen shares and recordings, so other people on a shared screen do not see it.
    /// The owner decided this on 2026-11-18: the ask box holds private text.
    static let hidesFromScreenSharing = true

    private unowned let appState: AppState
    private var panel: MeetingBarPanel?
    /// Closed by the person: it stays closed until "Show in-meeting window" or the next recording.
    private(set) var closedByUser = false

    init(appState: AppState) { self.appState = appState }

    var isVisible: Bool { panel?.isVisible == true }

    /// A recording started: shown, not made key (it must not take focus from the meeting).
    func show() {
        guard appState.redesignEnabled else { return }
        closedByUser = false
        present()
    }

    /// "Show in-meeting window" in the menu bar menu.
    func showFromMenu() {
        guard appState.redesignEnabled, appState.activeMeetingId != nil else { return }
        closedByUser = false
        present()
    }

    /// The recording stopped.
    func hide() { panel?.orderOut(nil) }

    private func close() {
        closedByUser = true
        panel?.orderOut(nil)
    }

    private func present() {
        let panel = ensurePanel()
        panel.orderFrontRegardless()
    }

    private func ensurePanel() -> MeetingBarPanel {
        if let panel { return panel }
        let host = NSHostingController(rootView: MeetingBarHostView(appState: appState, audio: appState.audio, live: appState.liveTranscript, onClose: { [weak self] in self?.close() })
            .environmentObject(appState))
        host.sizingOptions = [.preferredContentSize]
        let panel = MeetingBarPanel(
            contentRect: NSRect(x: 0, y: 0, width: 330, height: 260),
            styleMask: [.nonactivatingPanel, .borderless], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false   // the card draws its own
        panel.level = .floating
        // On every Space and over full screen apps.
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.isMovableByWindowBackground = true
        // Whimbrel's own capture leaves this window out: its ask box holds text. Screen text capture already skips
        // Whimbrel's own windows (`CaptureExclusions.alwaysExcludedBundleIds`); snapshots take only the meeting app's
        // window. This keeps it out of any other screen capture too.
        panel.sharingType = Self.hidesFromScreenSharing ? .none : .readOnly
        panel.setAccessibilityLabel(String(localized: "Whimbrel, in-meeting window"))
        panel.contentViewController = host
        panel.onCancel = { [weak self] in self?.returnFocus() }
        // A remembered place; top right of the main screen the first time.
        panel.setFrameAutosaveName(Self.frameName)
        if !panel.setFrameUsingName(Self.frameName), let area = NSScreen.main?.visibleFrame {
            let size = panel.frame.size
            panel.setFrameTopLeftPoint(NSPoint(x: area.maxX - size.width - Self.margin, y: area.maxY - Self.margin))
        }
        self.panel = panel
        return panel
    }

    /// Escape: hands the keyboard back to the previous app without closing the window.
    private func returnFocus() {
        panel?.makeFirstResponder(nil)
        if let previous = appState.frontmostTracker.previous.flatMap({ NSRunningApplication(processIdentifier: $0.app.pid) }),
           !previous.isTerminated {
            previous.activate()
        } else {
            NSApp.deactivate()
        }
    }
}

/// What the panel shows: the shared package view over the app's state.
struct MeetingBarHostView: View {
    @ObservedObject var appState: AppState
    @ObservedObject var audio: AudioCaptureService
    @ObservedObject var live: LiveTranscriptionService
    let onClose: () -> Void
    @State private var question = ""
    @State private var waiting = false
    @State private var undelivered = false

    var body: some View {
        SwiftUI.TimelineView(.periodic(from: .now, by: 1)) { context in
            WMeetingBar(
                model: WMeetingBarModel(
                    title: appState.activeMeetingTitle ?? String(localized: "Meeting"),
                    state: LiveMeetingAdapter.state(
                        isProcessing: appState.processingMeetingId != nil && appState.activeMeetingId == nil, stage: nil,
                        isStarting: appState.isStartingMeetingNow, pause: appState.recordingPause,
                        resuming: appState.isResumingRecording, problem: appState.recordingProblem),
                    recordedSeconds: LiveMeetingAdapter.recordedSeconds(
                        now: context.date, startedAt: appState.activeMeetingStartedAt, pause: appState.recordingPause),
                    level: audio.inputLevel),
                actions: WLiveActions(
                    pause: { appState.pauseRecording() }, resume: { appState.resumeRecording() },
                    stop: { appState.endActiveMeetingIfNeeded() }),
                onClose: onClose
            ) {
                // The ask works from the transcript, so it is offered only when live transcript is on and has lines.
                if appState.liveTranscriptSettings.enabled, !live.segments.isEmpty { askField }
            }
            .padding(12)
        }
        .redesignThemed()
        // The held question was taken by the main window's ask bar: now the field can clear.
        .onReceive(appState.$liveAskHandoff) { handoff in
            if waiting, !handoff.isHolding { waiting = false; question = "" }
        }
    }

    private var askField: some View {
        VStack(alignment: .leading, spacing: 4) {
            askRow
            if undelivered {
                Text(String(localized: "Open Whimbrel to see the answer.", comment: "Shown under the question field of the in-meeting window when the question could not be handed to the main window. The question stays in the field."))
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    private var askRow: some View {
        HStack(spacing: 8) {
            Image(systemName: "sparkles").foregroundStyle(.secondary)
            TextField(String(localized: "Ask about this meeting"), text: $question)
                .textFieldStyle(.plain)
                .onSubmit(send)
            Button(action: send) { Image(systemName: "paperplane") }
                .buttonStyle(.plain)
                .disabled(question.trimmingCharacters(in: .whitespaces).isEmpty)
                .accessibilityLabel(String(localized: "Ask about this meeting"))
        }
        .padding(.horizontal, 10).frame(minHeight: 34)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color.whimbrelBackgroundCard))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.whimbrelBorder))
    }

    /// The answer opens in the main window, in the meeting's own ask.
    private func send() {
        let text = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        // The text stays in the field until the main window has taken it; if it cannot, it is still here.
        undelivered = false
        waiting = true
        appState.askLiveMeeting(text)
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: UInt64(LiveAskHandoff.deadline * 1_000_000_000))
            if waiting, appState.giveUpLiveAsk() { waiting = false; undelivered = true }
        }
    }
}
