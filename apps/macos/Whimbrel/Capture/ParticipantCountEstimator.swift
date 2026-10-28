import AppKit
import Foundation

/// Reads a running participant count from a heuristically-detected meeting's own
/// app UI via Accessibility, so it can be used as a diarization hint the same way a
/// calendar meeting's real attendee count already is. See
/// docs/superpowers/specs/2026-08-02-heuristic-speaker-count-hint-design.md.
protocol MeetingParticipantCountReader {
    /// Attempts to read the current participant count from this window's
    /// Accessibility tree. Returns nil if the expected element can't be found (app
    /// UI changed, panel not visible, wrong window, etc.) — never throws, never
    /// crashes; a nil result simply means this poll contributes nothing. A
    /// conformer may need to trigger a side effect before reading (e.g.
    /// `TeamsParticipantCountReader` activating Chromium's lazy accessibility mode)
    /// — that's an implementation detail of the individual reader, not part of this
    /// protocol's contract. Note this can be called more than once per poll (once
    /// per distinct candidate window `ParticipantCountEstimator.poll()` tries, see
    /// `resolveWindowCandidates()`), so a side-effecting conformer must bound its
    /// own cost per *read*, not assume one call per poll interval.
    func readParticipantCount(from window: AXUIElement) -> Int?
}

/// Selects the right per-app reader for a detected meeting. Returns nil for any
/// bundle ID / meeting type with no reader implemented yet.
enum MeetingParticipantCountReaderFactory {
    static func reader(forBundleID bundleID: String?, isGoogleMeet: Bool) -> MeetingParticipantCountReader? {
        // Per-app readers, each investigated against a real live meeting — see
        // ZoomParticipantCountReader, GoogleMeetParticipantCountReader,
        // TeamsParticipantCountReader.
        if isGoogleMeet { return GoogleMeetParticipantCountReader() }
        if bundleID == "us.zoom.xos" { return ZoomParticipantCountReader() }
        if bundleID == "com.microsoft.teams2" || bundleID == "com.microsoft.teams" { return TeamsParticipantCountReader() }
        return nil
    }
}

/// Polls a detected meeting's app UI throughout the meeting's duration, tracking a
/// running histogram of observed participant counts, and persists the current best
/// estimate (the mode, tie-broken by recency) to the meeting's DB row.
@MainActor
final class ParticipantCountEstimator {
    private let meetingId: Int64
    private let appBundleID: String?
    private let isGoogleMeet: Bool
    private let browserWindowProvider: () -> AXUIElement?
    private let reader: MeetingParticipantCountReader?
    private let permissions: PermissionsManager
    private let store: CoreMemoryStore

    private var observations: [Int] = []
    private var pollTimer: Timer?

    private static let pollInterval: TimeInterval = 15

    /// - Parameters:
    ///   - browserWindowProvider: returns the currently-tracked Google Meet browser
    ///     window (e.g. `MeetingHeuristicDetector.browserMeetingWindow(forMeetingId:)`),
    ///     or nil if this isn't a browser-based meeting or the tracked window is gone.
    init(
        meetingId: Int64,
        appBundleID: String?,
        isGoogleMeet: Bool,
        browserWindowProvider: @escaping () -> AXUIElement?,
        permissions: PermissionsManager,
        store: CoreMemoryStore
    ) {
        self.meetingId = meetingId
        self.appBundleID = appBundleID
        self.isGoogleMeet = isGoogleMeet
        self.browserWindowProvider = browserWindowProvider
        self.reader = MeetingParticipantCountReaderFactory.reader(forBundleID: appBundleID, isGoogleMeet: isGoogleMeet)
        self.permissions = permissions
        self.store = store
    }

    func start() {
        guard reader != nil else { return }  // no reader for this app — nothing to poll
        pollTimer?.invalidate()
        pollTimer = Timer.scheduledTimer(withTimeInterval: Self.pollInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
    }

    func stop() {
        pollTimer?.invalidate()
        pollTimer = nil
    }

    private func poll() {
        guard permissions.accessibilityGranted, let reader else { return }
        var foundCount: Int?
        for candidate in resolveWindowCandidates() {
            if let count = reader.readParticipantCount(from: candidate), count > 0 {
                foundCount = count
                break
            }
        }
        guard let count = foundCount else { return }
        observations.append(count)
        guard let best = Self.bestEstimate(from: observations) else { return }
        CoreMemoryStore.accessQueue.async { [store, meetingId] in
            store.setMeetingEstimatedSpeakerCount(id: meetingId, count: best)
        }
    }

    /// Candidate windows to try the reader against, in priority order. For the
    /// native-app case this deliberately returns *every* plausible window
    /// (`kAXFocusedWindowAttribute`, `kAXMainWindowAttribute`, then all of
    /// `kAXWindowsAttribute`) rather than stopping at the first one resolved: a real,
    /// non-self-healing failure mode was confirmed live where Zoom's focused window
    /// is "Zoom Workplace" (its always-on hub window) rather than "Zoom Meeting" —
    /// unlike the "Participants panel not yet opened" case, "Zoom Workplace" can stay
    /// focused indefinitely while the user is elsewhere, so stopping at the first
    /// resolved window would lock onto the wrong one and never self-correct. `poll()`
    /// tries the reader against each candidate in turn until one yields a real count.
    /// The Google Meet case has no such ambiguity — `browserWindowProvider()` already
    /// resolves the one specific browser window this meeting is tracked in (see
    /// `MeetingHeuristicDetector.browserMeetingWindow(forMeetingId:)` and its
    /// self-healing fallback), not a list of candidates — so it's returned as a
    /// single-element list here rather than needing this method's iteration logic.
    private func resolveWindowCandidates() -> [AXUIElement] {
        if isGoogleMeet {
            guard let window = browserWindowProvider() else { return [] }
            return [window]
        }
        guard let appBundleID,
              let app = NSWorkspace.shared.runningApplications.first(where: { $0.bundleIdentifier == appBundleID })
        else { return [] }
        let appElement = AXUIElementCreateApplication(app.processIdentifier)

        // Deduplicated by `CFEqual` (not Swift `==`/`Hashable`, which `AXUIElement`
        // doesn't reliably conform to) — focused/main/first-of-`kAXWindowsAttribute`
        // are frequently the exact same window, and a conformer with a side effect
        // per read (`TeamsParticipantCountReader`'s activation call) should only pay
        // that cost once per genuinely distinct window, not 2-3x for duplicates of
        // the one window that mattered.
        var candidates: [AXUIElement] = []
        func addIfNew(_ window: AXUIElement) {
            guard !candidates.contains(where: { CFEqual($0, window) }) else { return }
            candidates.append(window)
        }
        for attribute in [kAXFocusedWindowAttribute, kAXMainWindowAttribute] {
            var windowRef: CFTypeRef?
            if AXUIElementCopyAttributeValue(appElement, attribute as CFString, &windowRef) == .success,
               let windowRef {
                addIfNew(windowRef as! AXUIElement)
            }
        }
        var windowsRef: CFTypeRef?
        if AXUIElementCopyAttributeValue(appElement, kAXWindowsAttribute as CFString, &windowsRef) == .success,
           let windows = windowsRef as? [AXUIElement] {
            windows.forEach(addIfNew)
        }
        return candidates
    }

    /// Given a sequence of observed participant counts (in observation order),
    /// returns the mode — the most frequently observed value — breaking ties by
    /// preferring whichever tied value was most recently observed (a later reading
    /// is more likely to reflect the meeting's steady state than an earlier one).
    nonisolated static func bestEstimate(from observations: [Int]) -> Int? {
        guard !observations.isEmpty else { return nil }
        var counts: [Int: Int] = [:]
        var lastSeenIndex: [Int: Int] = [:]
        for (index, value) in observations.enumerated() {
            counts[value, default: 0] += 1
            lastSeenIndex[value] = index
        }
        return counts.max { a, b in
            if a.value != b.value { return a.value < b.value }
            return lastSeenIndex[a.key]! < lastSeenIndex[b.key]!
        }?.key
    }
}
