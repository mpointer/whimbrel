import Foundation
import OSLog

/// FR-MTG-1: watches whatever `meetingsProvider` returns (today, `AppState`'s merged
/// list of EventKit and Google Calendar-connector meetings via
/// `AppState.allUpcomingMeetings`) and, `leadMinutes` before a qualifying meeting
/// starts, unconditionally sets `pendingPrompt`. This service no longer decides between
/// prompting and silently auto-starting — that decision now belongs to the meeting-alert
/// panel (owned by `AppState`, subscribed to `$pendingPrompt`), which reads
/// `autoStartEnabled` itself and either runs a ~10-second visible countdown before
/// auto-starting (`autoStartEnabled == true`) or just waits indefinitely for the user to
/// act. That ~10s delay — where this service previously had zero delay and zero visual
/// feedback before auto-starting — is a deliberate, spec-approved UX change, not a
/// regression; don't "restore" instant auto-start from `git blame` history. Each meeting
/// is only acted on once per app run, tracked in memory by `QualifyingMeeting.id` (a
/// source-provided identifier -- `EKEvent.eventIdentifier` for EventKit meetings, a
/// Google Calendar event id for OAuth-sourced ones); this isn't persisted, so a relaunch
/// during a lead window could re-prompt for the same meeting.
@MainActor
final class MeetingTriggerService: ObservableObject {
    @Published var pendingPrompt: QualifyingMeeting?
    @Published var autoStartEnabled: Bool {
        didSet { defaults.set(autoStartEnabled, forKey: Self.autoStartDefaultsKey) }
    }
    /// FR-MTG-1: "prompt N minutes (configurable, default 1) before start."
    @Published var leadMinutes: Double {
        didSet { defaults.set(leadMinutes, forKey: Self.leadMinutesDefaultsKey) }
    }
    /// FR-MTG-2: whether the audio/app heuristic (`MeetingHeuristicDetector`) is allowed
    /// to prompt at all. Separate from `autoStartEnabled`, which governs both triggers
    /// once either fires.
    @Published var heuristicDetectionEnabled: Bool {
        didSet { defaults.set(heuristicDetectionEnabled, forKey: Self.heuristicDetectionDefaultsKey) }
    }

    private static let autoStartDefaultsKey = "com.mpointer.kestrel.meetingAutoStart"
    private static let leadMinutesDefaultsKey = "com.mpointer.kestrel.meetingLeadMinutes"
    private static let heuristicDetectionDefaultsKey = "com.mpointer.kestrel.meetingHeuristicDetectionEnabled"

    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "meeting-trigger")
    /// Event id -> event end (unix seconds). Persisted until the event ends, so relaunching
    /// the app mid-meeting doesn't prompt, and with auto-start record, the same event again.
    private var actedOnEventIDs: [String: TimeInterval] = [:]
    private static let actedOnDefaultsKey = "com.mpointer.kestrel.meetingTriggerActedOn"
    private var pollTimer: Timer?

    /// Set by the owner (AppState) rather than passed to `init`, to avoid an
    /// initialization-order dependency between the two.
    var onAcceptPrompt: ((QualifyingMeeting) -> Void)?

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        let now = Date().timeIntervalSince1970
        let stored = defaults.dictionary(forKey: Self.actedOnDefaultsKey) as? [String: TimeInterval] ?? [:]
        actedOnEventIDs = stored.filter { $0.value > now }
        autoStartEnabled = defaults.bool(forKey: Self.autoStartDefaultsKey)
        let storedLead = defaults.double(forKey: Self.leadMinutesDefaultsKey)
        leadMinutes = storedLead > 0 ? storedLead : 1
        if defaults.object(forKey: Self.heuristicDetectionDefaultsKey) == nil {
            heuristicDetectionEnabled = true // opt-out default: a low-risk, prompt-only feature
        } else {
            heuristicDetectionEnabled = defaults.bool(forKey: Self.heuristicDetectionDefaultsKey)
        }
    }

    /// `meetingsProvider` is called fresh on every 15s tick rather than captured once,
    /// so it always reflects whichever meetings are currently merged (EventKit +
    /// Calendar-connector) -- a plain `[QualifyingMeeting]` snapshot captured at
    /// `start()` time would go stale the moment either source refreshes.
    func start(meetingsProvider: @escaping () -> [QualifyingMeeting]) {
        pollTimer?.invalidate()
        pollTimer = Timer.scheduledTimer(withTimeInterval: 15, repeats: true) { [weak self] _ in
            guard let self else { return }
            Task { @MainActor in self.evaluate(meetings: meetingsProvider()) }
        }
    }

    func stop() {
        pollTimer?.invalidate()
        pollTimer = nil
    }

    func evaluate(meetings: [QualifyingMeeting], now: Date = Date()) {
        for meeting in meetings {
            guard actedOnEventIDs[meeting.id] == nil else { continue }
            let leadWindowStart = meeting.startDate.addingTimeInterval(-leadMinutes * 60)
            guard now >= leadWindowStart, now < meeting.endDate else { continue }
            actedOnEventIDs[meeting.id] = meeting.endDate.timeIntervalSince1970
            defaults.set(actedOnEventIDs.filter { $0.value > now.timeIntervalSince1970 }, forKey: Self.actedOnDefaultsKey)
            trigger(meeting)
        }
    }

    /// Meeting continue: the recording of calendar event `eventId` ended. If it ended because
    /// the call went away (`reason.callWentAway`) and the event itself hasn't ended, forget
    /// that it was acted on so a rejoin prompts (or auto-starts) as the same calendar meeting,
    /// whose `cal:` key then continues the recording. A manual End (`reason` nil), silence or
    /// a cap means the user is done, so it stays acted on and doesn't re-prompt.
    func meetingEnded(eventId: String, reason: MeetingEndDetector.EndReason?, now: Date = Date()) {
        guard reason?.callWentAway == true,
              let end = actedOnEventIDs[eventId], now.timeIntervalSince1970 < end else { return }
        actedOnEventIDs[eventId] = nil
        defaults.set(actedOnEventIDs.filter { $0.value > now.timeIntervalSince1970 }, forKey: Self.actedOnDefaultsKey)
    }

    /// FR-MTG-2: called when `MeetingHeuristicDetector` fires. Unlike calendar meetings,
    /// a detected meeting is already in progress — there's no lead-time countdown, so
    /// this sets `pendingPrompt` immediately rather than going through `evaluate`'s
    /// window math. As with the calendar trigger, whether that results in a visible
    /// countdown-then-auto-start or an indefinite wait for the user is decided later, by
    /// the meeting-alert panel reading `autoStartEnabled` — not here. Skips if a prompt
    /// is already pending, so a detected meeting never clobbers an unanswered calendar
    /// prompt.
    func handleDetectedMeeting(_ meeting: QualifyingMeeting) {
        guard heuristicDetectionEnabled, pendingPrompt == nil else { return }
        trigger(meeting)
    }

    /// Sets `pendingPrompt` unconditionally — the meeting-alert panel (owned by
    /// `AppState`, subscribed to `$pendingPrompt`) reads `autoStartEnabled` itself to
    /// decide whether to show an auto-start countdown or wait indefinitely for the user
    /// to act. This service no longer decides that here, and no longer posts a system
    /// notification — the panel is the only surface for this trigger.
    private func trigger(_ meeting: QualifyingMeeting) {
        pendingPrompt = meeting
    }

    func acceptPrompt() {
        guard let meeting = pendingPrompt else { return }
        pendingPrompt = nil
        onAcceptPrompt?(meeting)
    }

    func dismissPrompt() {
        pendingPrompt = nil
    }
}
