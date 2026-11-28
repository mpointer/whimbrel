import AppKit
import Combine
import SwiftUI

/// The floating "Still recording?" panel, a sibling of `MeetingAlertPanelController`.
/// Display only: it counts down to the deadline `RecordingCheckIn` gave, but AppState's
/// 5 s check-in poll decides when the meeting stops. Created once by `AppState`.
@MainActor
final class RecordingCheckInPanelController: ObservableObject {
    /// `nil` when the panel isn't showing. Read by `RecordingCheckInRedesignView`.
    @Published fileprivate(set) var secondsRemaining: Int?
    /// True for "Still in a meeting?" (`StillInMeeting`), false for the hourly "Still recording?".
    @Published fileprivate(set) var asksStillInMeeting = false
    /// True for the paused wording: "This recording is still paused." with Resume and Stop, no countdown.
    @Published fileprivate(set) var isPausedPrompt = false
    private var onResume: (() -> Void)?

    private static let screenMargin: CGFloat = 16

    private var panel: RedesignFloatingPanel?
    private var countdownTimer: Timer?
    private var deadline: Date?
    private var onKeep: (() -> Void)?
    private var onStop: (() -> Void)?

    func show(deadline: Date, stillInMeeting: Bool = false, onKeep: @escaping () -> Void, onStop: @escaping () -> Void) {
        asksStillInMeeting = stillInMeeting
        isPausedPrompt = false
        self.deadline = deadline
        self.onKeep = onKeep
        self.onStop = onStop
        countdownTimer?.invalidate()
        // `.common` mode keeps the countdown ticking during menu tracking and drags.
        let timer = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
        RunLoop.main.add(timer, forMode: .common)
        countdownTimer = timer
        tick()

        let panel = ensurePanel()
        positionPanel(panel)
        panel.orderFrontRegardless()
    }

    /// The hourly check-in while the recording is paused. It does not count down and nothing here ends the recording.
    func showPaused(onResume: @escaping () -> Void, onStop: @escaping () -> Void) {
        countdownTimer?.invalidate()
        countdownTimer = nil
        deadline = nil
        asksStillInMeeting = false
        isPausedPrompt = true
        self.onResume = onResume
        self.onKeep = nil
        self.onStop = onStop
        secondsRemaining = 0
        let panel = ensurePanel()
        positionPanel(panel)
        panel.orderFrontRegardless()
    }

    func resumeTapped() {
        let action = onResume
        hide()
        action?()
    }

    func hide() {
        isPausedPrompt = false
        onResume = nil
        countdownTimer?.invalidate()
        countdownTimer = nil
        deadline = nil
        secondsRemaining = nil
        onKeep = nil
        onStop = nil
        panel?.orderOut(nil)
    }

    /// Called from `AlertPanelsRedesign.swift`, so not `fileprivate`.
    func keepTapped() {
        let action = onKeep
        hide()
        action?()
    }

    func stopTapped() {
        let action = onStop
        hide()
        action?()
    }

    private func tick() {
        guard let deadline else { return }
        secondsRemaining = max(0, Int(deadline.timeIntervalSinceNow.rounded(.up)))
    }

    private func ensurePanel() -> RedesignFloatingPanel {
        if let panel { return panel }
        let built = makePanel()
        self.panel = built
        return built
    }

    /// The panel. Escape answers the question the safe way: while a stop is counting down it means Keep
    /// recording (hiding the panel alone would leave the countdown running with nothing on screen), and the paused
    /// reminder just closes, since nothing here ends a paused recording.
    private func makePanel() -> RedesignFloatingPanel {
        let host = NSHostingController(rootView: ThemedRoot { RecordingCheckInRedesignView(controller: self) })
        host.sizingOptions = [.preferredContentSize]
        let panel = RedesignFloatingPanel.make(content: host)
        panel.onCancel = { [weak self] in
            guard let self else { return }
            if self.isPausedPrompt { self.hide() } else { self.keepTapped() }
        }
        panel.setAccessibilityLabel(String(localized: "Whimbrel alert", comment: "Spoken name of the small window that asks about a meeting or a long recording."))
        return panel
    }

    private func positionPanel(_ panel: RedesignFloatingPanel) {
        panel.placeTopRight(margin: Self.screenMargin)
    }
}
