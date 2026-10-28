import AppKit
import CoreAudio
import Foundation
import OSLog

/// FR-MTG-2: "detect an active meeting with no matching calendar entry via heuristics —
/// meeting app running ... AND sustained bidirectional audio (mic in use + system audio
/// out) — and surface a one-click prompt within 30s of meeting start."
///
/// "Meet in browser" detection is implemented via `BrowserMeetingCheck` — a lightweight
/// AX check (not the full `ScreenTextCaptureService`/`AXTextWalker.extract()` text-walk
/// machinery, which only runs once capture is already on) for whether the frontmost
/// app is a known browser (Safari/Chrome — though Chrome is present in
/// `BrowserMeetingCheck.knownBrowserBundleIDs` but non-functional as shipped; see that
/// property's doc comment and `docs/05-roadmap.md`'s FR-MTG-2 paragraph for the
/// root-caused gap) currently showing a Google Meet, Teams or Zoom web-client URL.
/// Checked FIRST on every poll, ahead of the native-app bundle-ID check, because it's
/// keyed off the frontmost app — a specific, real-time signal — whereas the native
/// check matches any known meeting app merely *running* anywhere; checking the native
/// path first would starve the browser path any time an unrelated meeting app (e.g.
/// Slack) happens to be open in the background. The native "running anywhere" check
/// only runs as a fallback when the frontmost app isn't a recognized browser on a Meet
/// URL. Either path feeds the same debounce/dedup/`QualifyingMeeting` construction.
/// Once a browser meeting is detected, its AX window is tracked
/// (`browserMeetingWindow`) and re-checked directly on every later poll — via
/// `BrowserMeetingCheck.currentURL(of:)`, which doesn't require the window to still be
/// frontmost — so switching away from the browser mid-meeting doesn't wrongly clear
/// it; it's only cleared once that direct re-check confirms the tab is genuinely gone
/// or navigated away. Other browsers (Arc/Edge/Firefox) and other in-browser meeting
/// platforms (Zoom/Teams-in-browser) are not covered — same "most common cases, not
/// exhaustive" tolerance this class already applies to its native-app bundle-ID list.
@MainActor
final class MeetingHeuristicDetector: ObservableObject {
    @Published private(set) var detectedMeeting: QualifyingMeeting?

    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "meeting-heuristic")
    private var pollTimer: Timer?
    private var consecutiveMatches = 0
    private var lastMatchedBundleID: String?
    private var alreadyPromptedBundleIDs = Set<String>()
    /// Bundles whose recorded meeting ended (`meetingEnded(bundleID:)`). They are re-armed
    /// (removed from `alreadyPromptedBundleIDs`) only once mic + output audio drops, not at
    /// the end itself: a rejoin is then detected and continued, but pressing End while still
    /// in the call doesn't prompt or auto-start again within one poll. Trade-off: if the
    /// meeting ended on silence while the app still holds both devices, the bundle stays
    /// blocked until the app quits or the audio drops.
    private var endedBundleIDs = Set<String>()
    /// Test seam for the mic + output check.
    var isBidirectionalAudioActive: () -> Bool = {
        AudioDeviceActivity.isDefaultInputInUse() && AudioDeviceActivity.isDefaultOutputInUse()
    }

    /// Set only when `poll()`'s browser-based detection path matches —
    /// nil for every native-app-detected or calendar-triggered meeting. Correlated by
    /// `QualifyingMeeting.id` rather than added as a field on `QualifyingMeeting`
    /// itself, since that struct is a simple, `Equatable` value type and this is a
    /// live `AXUIElement` reference with no meaningful equality of its own.
    private var browserMeetingWindow: (meetingId: String, window: AXUIElement)?

    /// Set the moment the tracked browser window's Meet URL first fails to re-match
    /// (line ~113), cleared the moment it re-matches (either via the browser-frontmost
    /// path re-confirming it, or the direct tracked-window re-check succeeding again).
    /// Debounces the clearing of `browserMeetingWindow` so an ordinary brief tab switch
    /// inside the tracked window doesn't instantly drop it — without this, a re-detection
    /// shortly after would mint a fresh `meetingId` (line ~150) that a still-pending
    /// prompt's id no longer matches, silently losing the browser end-signal handoff
    /// (see `browserMeetingWindow(forMeetingId:)`'s callers).
    private var trackedBrowserWindowGoneStartedAt: Date?
    /// Mirrors `MeetingEndDetector.browserTabGoneGraceSeconds` — the same tolerance, on
    /// the same underlying signal, applied at both ends of the browser-meeting handoff.
    private static let trackedBrowserWindowGoneGraceSeconds: TimeInterval = 15

    /// Bundle IDs FR-MTG-2 names explicitly (Zoom, Teams, Slack huddle, Webex,
    /// FaceTime). Not verified against real installs this pass — Teams and Webex in
    /// particular have varied bundle IDs across versions/installers; treat these as a
    /// starting point, not a verified-correct list. Slack's huddle state can't be
    /// distinguished from "Slack is just open" from outside the app — this fires on
    /// Slack running + audio activity, which is a coarser signal than "a huddle is
    /// specifically active."
    nonisolated static let knownMeetingAppBundleIDs: Set<String> = [
        "us.zoom.xos",
        "com.microsoft.teams2",
        "com.microsoft.teams",
        "com.tinyspeck.slackmacgap",
        "com.webex.meetingmanager",
        "Cisco-Systems.Spark",
        "com.apple.FaceTime",
    ]

    /// Consecutive positive polls required before firing — damps single-poll blips
    /// while keeping total detection latency well under FR-MTG-2's 30s budget.
    private static let requiredConsecutiveMatches = 2
    private static let pollInterval: TimeInterval = 5

    func start() {
        pollTimer?.invalidate()
        pollTimer = Timer.scheduledTimer(withTimeInterval: Self.pollInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
    }

    func stop() {
        pollTimer?.invalidate()
        pollTimer = nil
        consecutiveMatches = 0
        lastMatchedBundleID = nil
        browserMeetingWindow = nil
        trackedBrowserWindowGoneStartedAt = nil
    }

    private func poll() {
        // Checked first, based on the *frontmost* app: a much stronger, more specific
        // real-time signal than "some known meeting app happens to be running
        // somewhere" below. Checking the native "running anywhere" path first would
        // starve this branch entirely whenever any of knownMeetingAppBundleIDs (e.g.
        // Slack) is merely open in the background — extremely common — even during a
        // genuinely audio-active Meet call in the actual frontmost browser tab.
        if let frontmost = NSWorkspace.shared.frontmostApplication,
           let bundleID = frontmost.bundleIdentifier,
           BrowserMeetingCheck.knownBrowserBundleIDs.contains(bundleID),
           let (window, urlString) = BrowserMeetingCheck.focusedWebAreaURL(for: frontmost),
           let provider = Self.meetingURLProvider(urlString) {
            // A known browser is frontmost and on Meet, so a browser meeting is still
            // live (not necessarily a check against this exact tracked window — this
            // doesn't compare `window` against `browserMeetingWindow.window`, same
            // per-bundle-ID coarseness the rest of this class already has). Reset any
            // in-progress grace countdown so a later brief tab-switch gets the full
            // 15s, rather than inheriting a stale start time from an earlier switch.
            trackedBrowserWindowGoneStartedAt = nil
            let link = URL(string: urlString).map { DetectedConferenceLink(provider: provider, url: $0) }
            handleMatch(bundleID: bundleID, title: Self.title(for: provider), browserWindow: window, conferenceLink: link)
            return
        }

        if let runningApp = Self.runningMeetingApp(), let bundleID = runningApp.bundleIdentifier {
            handleMatch(bundleID: bundleID, title: runningApp.localizedName ?? "Meeting", browserWindow: nil)
            return
        }

        // Neither path matched via this poll's frontmost/running-app checks. If we're
        // tracking a previously-detected browser meeting, that alone doesn't mean it's
        // over — the user may have simply switched focus elsewhere (checking Slack,
        // taking notes, or just deciding whether to accept the detection prompt, which
        // can reasonably take longer than one 5s poll interval).
        // BrowserMeetingCheck.currentURL(of:) doesn't require the window to be
        // frontmost, so re-check the actual tracked window directly before concluding
        // the meeting ended, rather than clearing it just because it isn't frontmost
        // this particular poll. That re-check itself is debounced
        // (`trackedBrowserWindowGoneStartedAt`/`trackedBrowserWindowGoneGraceSeconds`):
        // without the grace period, a tab switch inside the tracked window that lasts
        // longer than one poll would clear `browserMeetingWindow` immediately, and the
        // re-detection that follows would mint a fresh `meetingId` that a still-pending
        // prompt's id no longer matches — silently dropping the browser end-signal
        // handoff for that meeting. Note also that any re-detection published here
        // while a capture is already active is dropped downstream by `AppState`'s
        // `!captureEnabled` gate (`AppState.swift`), so this debounce only matters for
        // the pre-accept prompt window, not for an already-recording meeting.
        if let tracked = browserMeetingWindow {
            if Self.meetingURLProvider(BrowserMeetingCheck.currentURL(of: tracked.window)) != nil {
                trackedBrowserWindowGoneStartedAt = nil
                return
            }
            if trackedBrowserWindowGoneStartedAt == nil {
                trackedBrowserWindowGoneStartedAt = Date()
            }
            if let startedAt = trackedBrowserWindowGoneStartedAt,
               Date().timeIntervalSince(startedAt) < Self.trackedBrowserWindowGoneGraceSeconds {
                return  // within grace — keep the tracked window and its meetingId
            }
        }
        trackedBrowserWindowGoneStartedAt = nil

        consecutiveMatches = 0
        lastMatchedBundleID = nil
        // No known meeting app is running and no tracked browser Meet tab is still
        // live — treat this as "that meeting is over" and forget which bundle IDs were
        // already prompted, so the next meeting (same app/browser or different) can
        // trigger a fresh detection. Without this, a bundle ID that fired once could
        // never fire again for the rest of the app's run, even across separate,
        // unrelated meetings.
        alreadyPromptedBundleIDs.removeAll()
        endedBundleIDs.removeAll()
        browserMeetingWindow = nil
    }

    /// Shared debounce/dedup/fire sequence for both detection paths above — identical
    /// consecutive-match damping and per-bundle-ID dedup either way; only the
    /// `QualifyingMeeting` payload and whether `browserMeetingWindow` gets set differ.
    func handleMatch(bundleID: String, title: String, browserWindow: AXUIElement?, conferenceLink: DetectedConferenceLink? = nil) {
        guard isBidirectionalAudioActive() else {
            consecutiveMatches = 0
            alreadyPromptedBundleIDs.subtract(endedBundleIDs)
            endedBundleIDs.removeAll()
            return
        }

        if lastMatchedBundleID != bundleID {
            consecutiveMatches = 0
            lastMatchedBundleID = bundleID
        }
        consecutiveMatches += 1
        guard consecutiveMatches >= Self.requiredConsecutiveMatches else { return }
        guard !alreadyPromptedBundleIDs.contains(bundleID) else { return }

        alreadyPromptedBundleIDs.insert(bundleID)
        let now = Date()
        let meetingId = "detected-\(bundleID)-\(Int(now.timeIntervalSince1970))"
        if let browserWindow {
            browserMeetingWindow = (meetingId: meetingId, window: browserWindow)
        }
        detectedMeeting = QualifyingMeeting(
            id: meetingId,
            title: title,
            startDate: now,
            endDate: now.addingTimeInterval(60 * 60),
            attendees: [],
            conferenceLink: conferenceLink,
            source: .detected,
            appBundleID: bundleID,
            organizerEmail: nil, organizerName: nil, selfIsOrganizer: false, selfResponse: nil
        )
        log.info("Detected likely meeting via \(bundleID, privacy: .public) + active mic/output")
    }

    /// The recorded meeting from this app ended: detect it again after its audio drops.
    func meetingEnded(bundleID: String) {
        endedBundleIDs.insert(bundleID)
    }

    /// Consumed once handled (prompted or auto-started) so the same poll cycle doesn't
    /// re-fire while the caller is still acting on the previous detection.
    func clearDetection() {
        detectedMeeting = nil
    }

    private static func runningMeetingApp() -> NSRunningApplication? {
        NSWorkspace.shared.runningApplications.first { app in
            guard let bundleID = app.bundleIdentifier else { return false }
            return knownMeetingAppBundleIDs.contains(bundleID)
        }
    }

    /// Whether some process has the default input device open: a live call (Zoom, Teams,
    /// Meet) always does, and Whimbrel itself only opens it once recording starts. The
    /// signal that this Mac is actually in the call rather than near one.
    static func isMicrophoneInUse() -> Bool {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDefaultInputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var device = AudioObjectID(0)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device) == noErr,
              device != 0 else { return false }
        address.mSelector = kAudioDevicePropertyDeviceIsRunningSomewhere
        var running: UInt32 = 0
        size = UInt32(MemoryLayout<UInt32>.size)
        guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &running) == noErr else { return false }
        return running != 0
    }

    /// FR-MTG-5: is a specific app (by bundle id) still running? Used by
    /// `MeetingEndDetector` to end a *detected* meeting the moment its app quits,
    /// independent of `knownMeetingAppBundleIDs` (the caller already knows exactly
    /// which app it's watching, from `QualifyingMeeting.appBundleID`).
    static func isAppRunning(bundleID: String) -> Bool {
        NSWorkspace.shared.runningApplications.contains { $0.bundleIdentifier == bundleID }
    }

    /// The AX window a browser-based detection matched, if `meetingId` is the exact
    /// meeting that match produced — `nil` for a calendar or native-app-detected
    /// meeting (which never set this), or if a *different*, later detection has since
    /// overwritten it.
    func browserMeetingWindow(forMeetingId meetingId: String) -> AXUIElement? {
        guard let browserMeetingWindow, browserMeetingWindow.meetingId == meetingId else { return nil }
        return browserMeetingWindow.window
    }

    /// Self-healing fallback for `browserMeetingWindow(forMeetingId:)`. That lookup
    /// goes permanently stale once a >15s tab switch lets `poll()`'s grace period
    /// expire and clear the tracked window: a later re-detection mints a *new*
    /// `meetingId` internally, which a caller's already-captured id (e.g.
    /// `ParticipantCountEstimator`'s `browserWindowProvider`, built once at
    /// estimator-construction time in `AppState.finishStartingCapture`) can never
    /// match again — silently killing the Google Meet hint for the rest of the
    /// meeting even though the tab is still open. This resolves a Meet browser
    /// window fresh instead, the same way `poll()`'s own browser-Meet detection path
    /// does (frontmost app, known browser, focused web area on a `meet.google.com`
    /// URL), so a caller that tries the id-keyed lookup first and falls back to this
    /// gets the same self-correcting behavior `resolveWindow()` already has for
    /// native-app windows. Returns nil if the frontmost app isn't a matching browser
    /// tab at this exact instant — harmless, since the next poll tries again.
    static func resolveFrontmostGoogleMeetWindow() -> AXUIElement? {
        guard let frontmost = NSWorkspace.shared.frontmostApplication,
              let bundleID = frontmost.bundleIdentifier,
              BrowserMeetingCheck.knownBrowserBundleIDs.contains(bundleID),
              let (window, urlString) = BrowserMeetingCheck.focusedWebAreaURL(for: frontmost),
              meetingURLProvider(urlString) == .googleMeet
        else { return nil }
        return window
    }

    /// Which meeting service a browser URL is on: Google Meet, Teams or Zoom's web
    /// client. Core's exact-host match (shared with Windows), not a substring check, so
    /// a URL that merely mentions a meeting host elsewhere (a query parameter, a
    /// look-alike subdomain) never matches. `nil` input is never a match.
    nonisolated static func meetingURLProvider(_ urlString: String?) -> MeetingProvider? {
        urlString.flatMap(CoreMemoryStore.meetingURLProvider)
    }

    private static func title(for provider: MeetingProvider) -> String {
        switch provider {
        case .googleMeet: "Google Meet"
        case .teams: "Microsoft Teams"
        case .zoom: "Zoom"
        case .webex: "Webex"
        }
    }
}
