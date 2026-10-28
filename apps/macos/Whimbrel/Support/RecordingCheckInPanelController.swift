import AppKit
import Combine
import SwiftUI

/// The floating "Still recording?" panel, a sibling of `MeetingAlertPanelController`.
/// Display only: it counts down to the deadline `RecordingCheckIn` gave, but AppState's
/// 5 s check-in poll decides when the meeting stops. Created once by `AppState`.
@MainActor
final class RecordingCheckInPanelController: ObservableObject {
    /// `nil` when the panel isn't showing. Read by `RecordingCheckInView`.
    @Published fileprivate(set) var secondsRemaining: Int?

    private static let panelSize = NSSize(width: 380, height: 120)
    private static let screenMargin: CGFloat = 16

    private var panel: NSPanel?
    private var countdownTimer: Timer?
    private var deadline: Date?
    private var onKeep: (() -> Void)?
    private var onStop: (() -> Void)?

    func show(deadline: Date, onKeep: @escaping () -> Void, onStop: @escaping () -> Void) {
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

    func hide() {
        countdownTimer?.invalidate()
        countdownTimer = nil
        deadline = nil
        secondsRemaining = nil
        onKeep = nil
        onStop = nil
        panel?.orderOut(nil)
    }

    /// Called from `RecordingCheckInView.swift`, so not `fileprivate`.
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

    private func ensurePanel() -> NSPanel {
        if let panel { return panel }
        let hostingView = NSHostingView(rootView: RecordingCheckInView(controller: self))
        hostingView.frame = NSRect(origin: .zero, size: Self.panelSize)

        let panel = NSPanel(
            contentRect: NSRect(origin: .zero, size: Self.panelSize),
            styleMask: [.nonactivatingPanel, .borderless],
            backing: .buffered,
            defer: false
        )
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false   // `whimbrelCard()` draws its own
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
        panel.setFrameOrigin(NSPoint(
            x: screenFrame.maxX - Self.panelSize.width - Self.screenMargin,
            y: screenFrame.maxY - Self.panelSize.height - Self.screenMargin
        ))
    }
}
