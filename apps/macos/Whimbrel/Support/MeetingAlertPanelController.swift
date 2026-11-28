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
    /// `nil` when no alert is showing. Read by `MeetingAlertRedesignView`.
    @Published fileprivate(set) var meeting: QualifyingMeeting?
    /// `nil` means "no countdown — wait indefinitely" (auto-start off). Read by
    /// `MeetingAlertRedesignView` to show (or hide) the "Auto-starting in Ns…" caption.
    @Published fileprivate(set) var secondsRemaining: Int?
    /// Set when the panel reports a split (`showKeepTogether`): the earlier meeting's title.
    @Published fileprivate(set) var earlierTitle: String?
    /// The showing meeting's brief key, when it has a readable brief. Set by `setPrep` once the
    /// lookup lands; nil hides "Open brief".
    @Published fileprivate(set) var prepKey: String?
    /// Opens a brief by key. Set once by `AppState`.
    var onOpenPrep: ((String) -> Void)?

    private static let countdownSeconds = 10
    private static let screenMargin: CGFloat = 16

    private var panel: RedesignFloatingPanel?
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
        earlierTitle = nil
        prepKey = nil
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

    /// After a back-to-back split (`MeetingSplit`): `meeting` is recording as a new meeting;
    /// "Keep together" (`onAccept`) undoes that. No countdown.
    func showKeepTogether(for meeting: QualifyingMeeting, earlierTitle: String, onKeepTogether: @escaping () -> Void) {
        show(for: meeting, autoStart: false, onAccept: onKeepTogether, onAcceptAndOpenTimeline: onKeepTogether, onDismiss: {})
        self.earlierTitle = earlierTitle
    }

    /// Hides the panel only when it is showing a split (for `meetingId`, when given).
    func hideKeepTogether(for meetingId: String? = nil) {
        guard earlierTitle != nil, meetingId == nil || meeting?.id == meetingId else { return }
        hide()
    }

    /// Hides a prompt, leaving a showing "Keep together" pop-up alone.
    func hidePrompt() {
        if earlierTitle == nil { hide() }
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
        earlierTitle = nil
        prepKey = nil
        onAccept = nil
        onAcceptAndOpenTimeline = nil
        onDismiss = nil
        panel?.orderOut(nil)
    }

    /// Called from `AlertPanelsRedesign.swift` (a separate file — deliberately not
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

    /// The Join button's target — calendar meetings only (a detected meeting is one
    /// the user is already in). Nil hides the button.
    var joinTarget: JoinTarget? {
        guard let meeting, meeting.source != .detected else { return nil }
        return JoinTarget.make(from: meeting.conferenceLink)
    }

    /// Opens the meeting; deliberately leaves the panel and its Record choice alone.
    func joinTapped() {
        guard let target = joinTarget else { return }
        NSWorkspace.shared.open(target.urlToOpen { NSWorkspace.shared.urlForApplication(toOpen: $0) != nil })
    }

    /// Shows "Open brief" for `key` while the panel still shows `meetingId` (a late lookup for an
    /// earlier meeting is ignored).
    func setPrep(key: String?, forMeetingId meetingId: String) {
        guard meeting?.id == meetingId else { return }
        prepKey = key
    }

    /// Opens the brief; like Join, leaves the panel and its Record choice alone.
    func openPrepTapped() {
        guard let prepKey else { return }
        onOpenPrep?(prepKey)
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

    private func ensurePanel() -> RedesignFloatingPanel {
        if let panel { return panel }
        let built = makePanel()
        self.panel = built
        return built
    }

    /// The panel: Escape dismisses once the panel has the keyboard. Return activates the main action only when that
    /// action is marked as the safe default (`WAlertAction.isSafeDefault`), which Start Recording and Keep together are not, so
    /// Return never starts a recording. It is shown with `orderFrontRegardless`, so it never takes focus from the app in front
    /// when it appears.
    private func makePanel() -> RedesignFloatingPanel {
        let host = NSHostingController(rootView: ThemedRoot { MeetingAlertRedesignView(controller: self) })
        host.sizingOptions = [.preferredContentSize]
        let panel = RedesignFloatingPanel.make(content: host)
        // Escape is Dismiss, the same call as the Dismiss button (and Cancel on the countdown).
        panel.onCancel = { [weak self] in self?.dismissTapped() }
        panel.setAccessibilityLabel(String(localized: "Whimbrel alert", comment: "Spoken name of the small window that asks about a meeting or a long recording."))
        return panel
    }

    private func positionPanel(_ panel: RedesignFloatingPanel) {
        panel.placeTopRight(margin: Self.screenMargin)
    }
}
