import AppKit
import Combine
import SwiftUI

/// Owns the floating "meeting detected" panel (`NSPanel`, not a SwiftUI `Window` scene —
/// this needs to appear automatically above all other windows/Spaces, which SwiftUI's
/// declarative window scenes don't support) plus the auto-start countdown timer and the
/// panel's three actions (Start Recording, Start Recording & Open Timeline, Dismiss).
/// Created once by `AppState` and reused across trigger events — never reallocated per
/// meeting.
@MainActor
final class MeetingAlertPanelController: ObservableObject {
    /// `nil` when no alert is showing. Read by `MeetingAlertView`.
    @Published fileprivate(set) var meeting: QualifyingMeeting?
    /// `nil` means "no countdown — wait indefinitely" (auto-start off). Read by
    /// `MeetingAlertView` to show (or hide) the "Auto-starting in Ns…" caption.
    @Published fileprivate(set) var secondsRemaining: Int?

    private static let countdownSeconds = 10
    private static let panelSize = NSSize(width: 380, height: 132)
    private static let screenMargin: CGFloat = 16

    private var panel: NSPanel?
    private var countdownTimer: Timer?
    private var onAccept: (() -> Void)?
    private var onAcceptAndOpenTimeline: (() -> Void)?
    private var onDismiss: (() -> Void)?

    /// Shows the panel for `meeting`. `autoStart` (the caller's current
    /// `meetingTrigger.autoStartEnabled` value at trigger time) decides whether a
    /// countdown runs — if it completes, `onAccept` fires automatically, exactly as if
    /// the user had clicked Start Recording.
    func show(
        for meeting: QualifyingMeeting,
        autoStart: Bool,
        onAccept: @escaping () -> Void,
        onAcceptAndOpenTimeline: @escaping () -> Void,
        onDismiss: @escaping () -> Void
    ) {
        self.meeting = meeting
        self.onAccept = onAccept
        self.onAcceptAndOpenTimeline = onAcceptAndOpenTimeline
        self.onDismiss = onDismiss

        countdownTimer?.invalidate()
        countdownTimer = nil
        secondsRemaining = autoStart ? Self.countdownSeconds : nil
        if autoStart {
            // Built explicitly (rather than via `Timer.scheduledTimer`) and added to the
            // RunLoop in `.common` mode so the countdown keeps ticking while a menu is
            // open or a window is being dragged — `.scheduledTimer` only runs in
            // `.default` mode, which stalls during menu tracking / drag loops.
            let timer = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
                Task { @MainActor in self?.tick() }
            }
            RunLoop.main.add(timer, forMode: .common)
            countdownTimer = timer
        }

        let panel = ensurePanel()
        positionPanel(panel)
        panel.orderFrontRegardless()
    }

    /// Clears the panel's own local state (timer, published properties, window
    /// visibility) without invoking any of the action closures — used both when an
    /// action closure itself dismisses (see `acceptTapped()` etc.) and when `AppState`
    /// observes `pendingPrompt` clearing for any other reason.
    func hide() {
        countdownTimer?.invalidate()
        countdownTimer = nil
        meeting = nil
        secondsRemaining = nil
        onAccept = nil
        onAcceptAndOpenTimeline = nil
        onDismiss = nil
        panel?.orderOut(nil)
    }

    /// Called from `MeetingAlertView.swift` (a separate file — deliberately not
    /// `fileprivate`, since Swift's `fileprivate` is file-scoped, not class-scoped, and
    /// would otherwise block the view's button actions from calling these).
    func acceptTapped() {
        let action = onAccept
        hide()
        action?()
    }

    func acceptAndOpenTimelineTapped() {
        let action = onAcceptAndOpenTimeline
        hide()
        action?()
    }

    func dismissTapped() {
        let action = onDismiss
        hide()
        action?()
    }

    private func tick() {
        guard let remaining = secondsRemaining else { return }
        let next = remaining - 1
        secondsRemaining = next
        if next <= 0 {
            acceptTapped()
        }
    }

    private func ensurePanel() -> NSPanel {
        if let panel { return panel }
        let hostingView = NSHostingView(rootView: MeetingAlertView(controller: self))
        hostingView.frame = NSRect(origin: .zero, size: Self.panelSize)

        let panel = NSPanel(
            contentRect: NSRect(origin: .zero, size: Self.panelSize),
            styleMask: [.nonactivatingPanel, .borderless],
            backing: .buffered,
            defer: false
        )
        panel.isOpaque = false
        panel.backgroundColor = .clear
        // `whimbrelCard()` on the SwiftUI content already draws its own shadow; disabling
        // the panel's native window shadow avoids the two visibly doubling up.
        panel.hasShadow = false
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.contentView = hostingView

        self.panel = panel
        return panel
    }

    private func positionPanel(_ panel: NSPanel) {
        guard let screenFrame = NSScreen.main?.visibleFrame else { return }
        let origin = NSPoint(
            x: screenFrame.maxX - Self.panelSize.width - Self.screenMargin,
            y: screenFrame.maxY - Self.panelSize.height - Self.screenMargin
        )
        panel.setFrameOrigin(origin)
    }
}
