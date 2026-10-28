import ApplicationServices
import Foundation
import OSLog

/// FR-MTG-5: watches the currently-recording meeting and decides when it's ended, per
/// five heuristics, in priority order: the meeting app quitting entirely (checked
/// first, instantly — a quit process is a hard, unambiguous fact); for a
/// browser-detected meeting, its Meet tab being sustainedly gone — closed, or
/// navigated away from and not back — for a short grace period (checked second, fast
/// but debounced, NOT instant, since a single poll seeing a different active tab in
/// the same window is not itself proof the call ended — see
/// `browserTabGoneGraceSeconds` below); sustained audio silence
/// (60s), or a shorter silence window (15s) once a calendar-triggered meeting's own
/// scheduled end time has passed; a hard ceiling well past a calendar meeting's
/// scheduled end time, regardless of audio activity (see `scheduledEndHardCeilingSeconds`
/// below — the audio-silence heuristics alone were found live to sometimes never fire,
/// since they depend on a peak-amplitude threshold that isn't correctly calibrated for
/// every microphone/environment; this ceiling guarantees a calendar meeting can't stay
/// stuck recording indefinitely even when that happens); and an absolute maximum
/// recording duration, regardless of trigger type or audio activity, as a last-resort
/// safety net for a meeting with no calendar end time at all (detected/heuristic).
/// A manual "End Meeting" action (in `AppState`) always overrides all of this regardless
/// of what it observes.
///
/// Every non-`appQuit` heuristic additionally waits for a minimum recording age (a
/// just-started recording must not be killed before it captures anything).
///
/// The rules and their thresholds live in core (`api::meetings::end_rule`, where each
/// threshold's reasoning is documented) so the Mac and Windows share them; `evaluate`
/// below adapts this detector's inputs to that call.
@MainActor
final class MeetingEndDetector {
    private static let pollInterval: TimeInterval = 5

    private nonisolated static let log = Logger(subsystem: "com.mpointer.kestrel", category: "meeting-end-detector")

    private var pollTimer: Timer?
    private var recordingStartedAt: Date?
    private var silenceStartedAt: Date?
    private var browserTabGoneStartedAt: Date?
    private var captureReleasedStartedAt: Date?
    private var scheduledEndUnixMs: Int64?
    private var appBundleID: String?
    private var browserMeetingWindow: AXUIElement?
    private var isAudioActive: (() -> Bool)?
    private var onEnded: ((EndReason) -> Void)?
    private var suppressMaxDuration: () -> Bool = { false }

    /// Starts watching a just-begun recording session. `scheduledEndUnixMs` is the
    /// calendar event's own end time in unix milliseconds (nil for a detected/
    /// heuristic meeting, which has no calendar entry). `appBundleID` is the detected
    /// meeting app's bundle id (nil for a calendar-triggered meeting — there's no
    /// specific app to watch for quitting). `isAudioActive` reports whether real,
    /// audible content has been captured recently — deliberately NOT
    /// `AudioDeviceActivity`'s device-running check, which reports true for as long as
    /// our own capture engine is installed, even during real silence (that check exists
    /// for `MeetingHeuristicDetector`'s different use case: detecting a meeting
    /// *before* Whimbrel starts capturing anything of its own).
    func start(
        scheduledEndUnixMs: Int64?,
        appBundleID: String?,
        browserMeetingWindow: AXUIElement? = nil,
        isAudioActive: @escaping () -> Bool,
        suppressMaxDuration: @escaping () -> Bool = { false },
        onEnded: @escaping (EndReason) -> Void
    ) {
        stop()
        recordingStartedAt = Date()
        self.suppressMaxDuration = suppressMaxDuration
        self.scheduledEndUnixMs = scheduledEndUnixMs
        self.appBundleID = appBundleID
        self.browserMeetingWindow = browserMeetingWindow
        self.isAudioActive = isAudioActive
        self.onEnded = onEnded
        pollTimer = Timer.scheduledTimer(withTimeInterval: Self.pollInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
    }

    /// Between `start` and `stop`.
    var isRunning: Bool { pollTimer != nil }

    func stop() {
        pollTimer?.invalidate()
        pollTimer = nil
        recordingStartedAt = nil
        silenceStartedAt = nil
        browserTabGoneStartedAt = nil
        captureReleasedStartedAt = nil
        scheduledEndUnixMs = nil
        appBundleID = nil
        browserMeetingWindow = nil
        isAudioActive = nil
        onEnded = nil
        suppressMaxDuration = { false }
    }

    private func poll() {
        guard let recordingStartedAt else { return }
        let isOnMeetTab = browserMeetingWindow.map {
            MeetingHeuristicDetector.meetingURLProvider(BrowserMeetingCheck.currentURL(of: $0)) != nil
        }
        // #84: only for a detected native meeting app. A browser meeting has the
        // tab-gone rule, and a browser's audio processes serve every tab.
        let captureSessionActive = appBundleID.flatMap { bundleID in
            BrowserMeetingCheck.knownBrowserBundleIDs.contains(bundleID) ? nil : AudioDeviceActivity.isAppCapturingAudio(bundleID: bundleID)
        }
        var state = PollState(
            silenceStartedAt: silenceStartedAt,
            browserTabGoneStartedAt: browserTabGoneStartedAt,
            captureReleasedStartedAt: captureReleasedStartedAt
        )
        let reason = Self.evaluate(
            now: Date(),
            recordingStartedAt: recordingStartedAt,
            appRunning: appBundleID.map { MeetingHeuristicDetector.isAppRunning(bundleID: $0) } ?? true,
            isOnMeetTab: isOnMeetTab,
            scheduledEndUnixMs: scheduledEndUnixMs,
            audioActive: isAudioActive?() ?? false,
            captureSessionActive: captureSessionActive,
            state: &state
        )
        silenceStartedAt = state.silenceStartedAt
        browserTabGoneStartedAt = state.browserTabGoneStartedAt
        captureReleasedStartedAt = state.captureReleasedStartedAt
        // A showing check-in prompt, or a recent "Keep recording", holds off the 4 h cap.
        // The detector keeps polling, so the cap applies again once that lapses.
        if let reason, !(reason == .maxDuration && suppressMaxDuration()) {
            fireEnded(reason: reason, recordingStartedAt: recordingStartedAt)
        }
    }

    private func fireEnded(reason: EndReason, recordingStartedAt: Date) {
        let elapsed = Date().timeIntervalSince(recordingStartedAt)
        Self.log.notice("[meeting-end] reason=\(reason.rawValue, privacy: .public) recordingAgeSeconds=\(elapsed, privacy: .public)")
        let callback = onEnded
        stop()
        callback?(reason)
    }

    /// Which heuristic decided a recording should end, for diagnostics — the
    /// "doesn't detect a meeting ending" reports have no signal today for *why* a
    /// heuristic did or didn't fire; this makes the next report carry real evidence.
    enum EndReason: String {
        case appQuit = "app_quit"
        case captureReleased = "capture_released"
        case browserTabGone = "browser_tab_gone"
        case scheduledEndHardCeiling = "scheduled_end_hard_ceiling"
        case maxDuration = "max_duration"
        case silence = "silence"
        // Swift-only (never returned by core): the recording failsafes.
        case checkInTimeout = "check_in_timeout"
        case checkInStop = "check_in_stop"
        case durationCap = "duration_cap"
        case lowDisk = "low_disk"

        /// The call itself went away (left, closed or quit), as opposed to the user being done
        /// (silence, the caps). Only these re-arm a calendar event for a rejoin.
        var callWentAway: Bool {
            switch self {
            case .appQuit, .captureReleased, .browserTabGone: return true
            case .scheduledEndHardCeiling, .maxDuration, .silence,
                 .checkInTimeout, .checkInStop, .durationCap, .lowDisk: return false
            }
        }
    }

    /// Mutable state `evaluate` threads through polls — pulled out of the instance so
    /// the decision logic below can be a pure, directly-unit-testable function.
    struct PollState: Equatable {
        var silenceStartedAt: Date?
        var browserTabGoneStartedAt: Date?
        var captureReleasedStartedAt: Date?
    }

    /// The actual end-detection decision, isolated from the `Timer`/AX/instance-state
    /// plumbing above so it can be driven by a test with an arbitrary `now` and
    /// synthetic inputs instead of real wall-clock time and a live meeting app.
    /// `isOnMeetTab` is nil when no browser window is being tracked for this meeting.
    nonisolated static func evaluate(
        now: Date,
        recordingStartedAt: Date,
        appRunning: Bool,
        isOnMeetTab: Bool?,
        scheduledEndUnixMs: Int64?,
        audioActive: Bool,
        captureSessionActive: Bool? = nil,
        state: inout PollState
    ) -> EndReason? {
        let signals = CoreMeetingEndSignals(
            nowUnixMs: unixMs(now),
            recordingStartedUnixMs: unixMs(recordingStartedAt),
            appRunning: appRunning,
            onMeetingTab: isOnMeetTab,
            scheduledEndUnixMs: scheduledEndUnixMs,
            audioActive: audioActive,
            captureSessionActive: captureSessionActive
        )
        let coreState = CoreMeetingEndState(
            silenceStartedUnixMs: state.silenceStartedAt.map(unixMs),
            browserTabGoneStartedUnixMs: state.browserTabGoneStartedAt.map(unixMs),
            captureReleasedStartedUnixMs: state.captureReleasedStartedAt.map(unixMs)
        )
        guard let result = CoreMemoryStore.evaluateMeetingEnd(signals, state: coreState) else {
            log.error("[meeting-end] core end-rule call failed; not ending")
            return nil
        }
        state.silenceStartedAt = result.state.silenceStartedUnixMs.map(date)
        state.browserTabGoneStartedAt = result.state.browserTabGoneStartedUnixMs.map(date)
        state.captureReleasedStartedAt = result.state.captureReleasedStartedUnixMs.map(date)
        return result.reason.flatMap(EndReason.init(rawValue:))
    }

    private nonisolated static func unixMs(_ date: Date) -> Int64 {
        Int64((date.timeIntervalSince1970 * 1000).rounded())
    }

    private nonisolated static func date(_ unixMs: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(unixMs) / 1000)
    }
}
