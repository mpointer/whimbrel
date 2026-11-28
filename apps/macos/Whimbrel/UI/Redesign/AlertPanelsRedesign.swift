import AppKit
import SwiftUI
import WhimbrelUI

/// A floating panel for the redesigned small windows. It never takes focus when it appears (it is shown with
/// `orderFrontRegardless`, as today), but it can take the keyboard once the person clicks it, so Return then activates the
/// main action and Escape runs `onCancel`.
final class RedesignFloatingPanel: NSPanel {
    var onCancel: (() -> Void)?

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    /// Escape.
    override func cancelOperation(_ sender: Any?) { onCancel?() }

    /// Keeps the top right corner fixed as the content grows (a button appears) or shrinks.
    override func setContentSize(_ size: NSSize) {
        let top = frame.maxY
        let right = frame.maxX
        super.setContentSize(size)
        setFrameTopLeftPoint(NSPoint(x: right - frame.width, y: top))
    }

    /// The panel the meeting alert and the check-in share: transparent, floating, on every Space and over full screen apps.
    static func make(content: NSViewController) -> RedesignFloatingPanel {
        let panel = RedesignFloatingPanel(
            contentRect: NSRect(x: 0, y: 0, width: 440, height: 160),
            styleMask: [.nonactivatingPanel, .borderless], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false   // the card draws its own
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.contentViewController = content
        return panel
    }

    /// Sizes the panel to its content and puts its top right corner `margin` points in from the corner of the main screen.
    func placeTopRight(margin: CGFloat) {
        contentViewController?.view.layoutSubtreeIfNeeded()
        if let size = contentViewController?.view.fittingSize, size.width > 0, size.height > 0 { setContentSize(size) }
        guard let area = NSScreen.main?.visibleFrame else { return }
        setFrameTopLeftPoint(NSPoint(x: area.maxX - frame.width - margin, y: area.maxY - margin))
    }
}

/// The words and buttons of the meeting alert and the check-in window, as `WAlertPanelModel`s. Pure.
enum AlertPanelAdapter {
    // MARK: Meeting detected, meeting starting soon, recording as a new meeting

    struct MeetingInput: Equatable {
        var title: String
        /// A meeting found by the call detector, not one on the calendar.
        var isDetected: Bool
        /// Set when the panel reports a split: the earlier meeting's title.
        var earlierTitle: String?
        /// Nil means no countdown (auto-start is off).
        var secondsRemaining: Int?
        var hasBrief: Bool
        /// The provider's name when the meeting has a link that can be joined.
        var joinProvider: String?
    }

    struct MeetingCallbacks {
        var accept: () -> Void
        var acceptAndOpen: () -> Void
        var dismiss: () -> Void
        var openBrief: () -> Void
        var join: () -> Void
    }

    static func meetingTitle(_ input: MeetingInput) -> String {
        if input.earlierTitle != nil { return String(localized: "Recording as a New Meeting") }
        return input.isDetected ? String(localized: "Meeting Detected") : String(localized: "Meeting Starting Soon")
    }

    static func meeting(_ input: MeetingInput, _ cb: MeetingCallbacks) -> WAlertPanelModel {
        var lines: [String] = []
        if let earlier = input.earlierTitle {
            lines.append(String(localized: "Ended “\(earlier)” and saved it separately."))
        }
        // While the countdown runs the way out is one control, in the place of Dismiss. It does what Dismiss does (see
        // `MeetingTriggerService.dismissPrompt`): the alert closes and nothing starts, and Whimbrel does not offer this meeting
        // again, so the label says that and is not a bare "Cancel".
        let countdown = input.secondsRemaining.map {
            WAlertCountdown(text: String(localized: "Recording starts in \($0) s", comment: "Countdown on the meeting alert when auto-start is on. The placeholder is the seconds left."))
        }
        var others: [WAlertAction] = []
        if input.hasBrief { others.append(WAlertAction(id: "brief", title: String(localized: "Open brief"), run: cb.openBrief)) }
        if let provider = input.joinProvider {
            others.append(WAlertAction(id: "join", title: String(localized: "Join"), spokenName: String(localized: "Join on \(provider)"), run: cb.join))
        }
        if input.secondsRemaining != nil {
            others.append(WAlertAction(
                id: "dismiss", title: String(localized: "Don\u{2019}t record", comment: "Button on the meeting alert while the auto-start countdown runs. It stops the countdown and closes the alert, and Whimbrel does not offer to record this meeting again."),
                spokenName: String(localized: "Cancel the countdown and don\u{2019}t record this meeting", comment: "Spoken name of the Don\u{2019}t record button on the meeting alert."),
                help: String(localized: "Stops the countdown. Whimbrel will not ask about this meeting again.", comment: "Tooltip of the Don\u{2019}t record button on the meeting alert."),
                run: cb.dismiss))
        } else {
            others.append(WAlertAction(id: "dismiss", title: String(localized: "Dismiss"), run: cb.dismiss))
        }
        let main: WAlertAction
        let alternate: WAlertAction?
        if input.earlierTitle != nil {
            // Not a Return default: it joins two recordings and deletes the split-off meeting (`AppState.keepTogether`).
            main = WAlertAction(
                id: "keep-together", title: String(localized: "Keep together"), help: String(localized: "Undo the split: record this as part of the earlier meeting"), run: cb.accept)
            alternate = nil
        } else {
            // Not a Return default: it starts a recording, so it runs on Return only while it has focus.
            main = WAlertAction(id: "start", title: String(localized: "Start Recording"), run: cb.accept)
            alternate = WAlertAction(id: "start-and-open", title: String(localized: "Start Recording & Open Whimbrel"), run: cb.acceptAndOpen)
        }
        return WAlertPanelModel(
            title: meetingTitle(input), subject: input.title, lines: lines, countdown: countdown, main: main, alternate: alternate, others: others)
    }

    // MARK: Still recording, still in a meeting, still paused

    enum CheckInKind: Equatable { case recording, stillInMeeting, paused }

    struct CheckInCallbacks {
        var keep: () -> Void
        var stop: () -> Void
        var resume: () -> Void
    }

    /// The hourly check-in asks because the recording has run a long time, and says only that. "Still in a meeting?" asks
    /// because no speech arrived for `StillInMeeting.quietSeconds`. A paused recording is never stopped by the app, so its
    /// line says what is true while it is paused.
    static func checkIn(kind: CheckInKind, secondsRemaining: Int?, quietMinutes: Int = StillInMeeting.quietSeconds / 60, _ cb: CheckInCallbacks) -> WAlertPanelModel? {
        switch kind {
        case .paused:
            return WAlertPanelModel(
                title: String(localized: "This recording is still paused.", comment: "Reminder shown when a recording has stayed paused for a while."),
                // Only audio stops: memory keeps running while a recording is paused, so the line says audio.
                lines: [String(localized: "No audio is recorded while it is paused.", comment: "Line on the paused-recording reminder window. Only the recording of audio stops while it is paused; work memory is not affected.")],
                // Not a Return default: Resume restarts the recording, so it runs on Return only while it has focus.
                main: WAlertAction(id: "resume", title: String(localized: "Resume", comment: "Button on the recording bar that resumes a paused recording."), run: cb.resume),
                others: [WAlertAction(id: "stop", title: String(localized: "Stop"), run: cb.stop)])
        case .recording, .stillInMeeting:
            guard let seconds = secondsRemaining else { return nil }
            let asksMeeting = kind == .stillInMeeting
            let reason = asksMeeting
                ? String(localized: "No one has spoken for \(quietMinutes) minutes.", comment: "Line on the Still in a meeting window: why Whimbrel asks. The placeholder is the number of minutes without speech, currently 15.")
                : String(localized: "This recording has been running for a while.", comment: "Line on the hourly check-in window: why Whimbrel asks. It asks because of how long the recording has run, not because of silence.")
            return WAlertPanelModel(
                title: asksMeeting ? String(localized: "Still in a meeting?") : String(localized: "Still recording?"),
                lines: [reason],
                countdown: WAlertCountdown(text: String(localized: "Stopping in \(WRecordingText.clock(seconds: seconds))")),
                // The one Return default: keeping the recording is safe to run by accident.
                main: WAlertAction(id: "keep", title: String(localized: "Keep recording"), isSafeDefault: true, run: cb.keep),
                others: [WAlertAction(id: "stop", title: asksMeeting ? String(localized: "Stop") : String(localized: "Stop now"), run: cb.stop)])
        }
    }
}

/// The meeting alert: the panel state and the actions of `MeetingAlertPanelController`.
struct MeetingAlertRedesignView: View {
    @ObservedObject var controller: MeetingAlertPanelController

    var body: some View {
        if let meeting = controller.meeting {
            let input = AlertPanelAdapter.MeetingInput(
                title: meeting.title, isDetected: meeting.source == .detected, earlierTitle: controller.earlierTitle,
                secondsRemaining: controller.secondsRemaining, hasBrief: controller.prepKey != nil,
                joinProvider: controller.joinTarget?.provider.displayName)
            WAlertPanel(
                model: AlertPanelAdapter.meeting(input, AlertPanelAdapter.MeetingCallbacks(
                    accept: { controller.acceptTapped() }, acceptAndOpen: { controller.acceptAndOpenTimelineTapped() },
                    dismiss: { controller.dismissTapped() }, openBrief: { controller.openPrepTapped() }, join: { controller.joinTapped() })),
                width: 440, mark: Image(nsImage: NSApp.applicationIconImage), subjectIcon: Self.appIcon(for: meeting))
                // Room for the card's shadow inside the panel.
                .padding(14)
                .redesignThemed()
        }
    }

    /// The running app's real icon for a detected meeting. A calendar meeting has no running app, so it shows none here;
    /// the header already carries Whimbrel's own mark. The lookup asks Launch Services and reads the icon from disk, so it is
    /// done once per app, not on every countdown tick.
    private static func appIcon(for meeting: QualifyingMeeting) -> Image? {
        guard let bundleID = meeting.appBundleID else { return nil }
        return AppIconCache.shared.icon(forBundleID: bundleID)
    }
}

/// Icons by bundle id, kept for the run. A missing app is remembered too.
@MainActor
final class AppIconCache {
    static let shared = AppIconCache()
    private var icons: [String: Image?] = [:]
    /// How many times the system was asked, for the test.
    private(set) var lookups = 0

    func icon(forBundleID bundleID: String, lookup: (String) -> NSImage? = AppIconCache.systemIcon) -> Image? {
        if let known = icons[bundleID] { return known }
        lookups += 1
        let image = lookup(bundleID).map { Image(nsImage: $0) }
        icons[bundleID] = .some(image)
        return image
    }

    nonisolated static func systemIcon(_ bundleID: String) -> NSImage? {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) else { return nil }
        return NSWorkspace.shared.icon(forFile: url.path)
    }
}

/// The recording check-in.
struct RecordingCheckInRedesignView: View {
    @ObservedObject var controller: RecordingCheckInPanelController

    var body: some View {
        let kind: AlertPanelAdapter.CheckInKind = controller.isPausedPrompt ? .paused : (controller.asksStillInMeeting ? .stillInMeeting : .recording)
        if let model = AlertPanelAdapter.checkIn(
            kind: kind, secondsRemaining: controller.secondsRemaining,
            AlertPanelAdapter.CheckInCallbacks(
                keep: { controller.keepTapped() }, stop: { controller.stopTapped() }, resume: { controller.resumeTapped() })) {
            WAlertPanel(model: model, width: 380, mark: Image(nsImage: NSApp.applicationIconImage))
                .padding(14)
                .redesignThemed()
        }
    }
}
