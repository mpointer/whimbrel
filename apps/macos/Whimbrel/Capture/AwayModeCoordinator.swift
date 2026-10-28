import Foundation

/// What the Away Mode coordinator needs from the app. A port, so the join logic is
/// tested against a scripted Zoom window and a fake clock.
@MainActor
protocol AwayEnvironment {
    var now: Date { get }
    var isScreenLocked: Bool { get }
    var isMicInUse: Bool { get }
    var isRecording: Bool { get }
    func open(_ url: URL) -> Bool
    func zoomState() -> ZoomWindowState
    /// Starts capture tagged `away`. The meeting's id, or nil when capture didn't start.
    func startCapture(_ m: QualifyingMeeting) async -> Int64?
    /// Ends the capture `startCapture` began (never another recording).
    func endCapture()
    /// A completed meeting row with trigger `away` and no audio.
    func insertMissedMeeting(_ m: QualifyingMeeting) async -> Int64?
    func setOutcome(meetingId: Int64, outcome: AwayOutcome, at: Date) async
    func recordFollowup(_ req: AwayFollowupRequest) async
    var chat: ZoomChatPosting { get }
    var leaver: ZoomLeaving { get }
    var profileName: String { get }
    func sleep(seconds: Double) async
}

/// What an Away run did with a meeting.
enum AwayRunResult: Equatable {
    /// Not Away Mode's meeting (ineligible, or already handled this app run).
    case notHandled
    /// Away Mode is off or the user is present: the meeting goes back to the normal prompt.
    case stoodAside
    case finished(AwayOutcome)
}

/// Away Mode: joins a Zoom meeting for the user, muted with the camera off, and records it.
/// Capture starts only once Zoom shows the mic muted and the camera off, so nothing is
/// recorded (and no chat is posted) while the mic might be live. Every failure leaves
/// Zoom and records a missed meeting with a follow-up.
@MainActor
final class AwayModeCoordinator {
    static let pollSeconds = 2.0
    static let appOpenTimeout: TimeInterval = 60
    static let muteConfirmTimeout: TimeInterval = 15
    /// R3: give up on the waiting room this long after the start (or at the end, if sooner).
    static let waitingRoomLimit: TimeInterval = 10 * 60
    static let overrun: TimeInterval = 5 * 60

    private let env: AwayEnvironment
    private let isEnabled: () -> Bool
    /// One join at a time.
    private(set) var isRunning = false
    /// Meetings already handled this app run. The end detector forgets a calendar event
    /// whose call went away, so the same meeting can be prompted again; it never rejoins.
    private var handled: Set<String> = []

    init(env: AwayEnvironment, isEnabled: @escaping () -> Bool) {
        self.env = env
        self.isEnabled = isEnabled
    }

    /// A calendar (EventKit or Google) Zoom meeting the user didn't organize or decline.
    static func eligible(_ m: QualifyingMeeting) -> Bool {
        m.source != .detected && m.conferenceLink?.provider == .zoom && !m.selfIsOrganizer && m.selfResponse != .declined
    }

    /// Whether Away Mode takes a meeting prompt instead of the alert panel. `joinAvailable`
    /// is false until Zoom can be opened and watched; then the prompt flow is unchanged.
    static func takesPrompt(_ m: QualifyingMeeting, enabled: Bool, joinAvailable: Bool) -> Bool {
        joinAvailable && enabled && eligible(m)
    }

    /// The user is already in a call or recording (not this coordinator's own join), so
    /// Whimbrel records as it does without Away Mode.
    var userIsPresent: Bool { !isRunning && (env.isMicInUse || env.isRecording) }

    static func chatMessage(profileName: String) -> String {
        let name = profileName.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty
            ? "I can't make it live. My Whimbrel notes app is capturing this meeting so I can catch up."
            : "\(name) can't make it live. Their Whimbrel notes app is capturing this meeting so they can catch up."
    }

    func run(for m: QualifyingMeeting) async -> AwayRunResult {
        guard Self.eligible(m), !handled.contains(m.id) else { return .notHandled }
        guard isEnabled() else { return .stoodAside }
        // Checked before standing aside: the active run's own capture holds the mic and a recording.
        if isRunning {
            handled.insert(m.id)
            return .finished(await missed(m, .missedOverlap))
        }
        if userIsPresent { return .stoodAside }
        handled.insert(m.id)
        return .finished(await join(m))
    }

    private func join(_ m: QualifyingMeeting) async -> AwayOutcome {
        isRunning = true
        defer { isRunning = false }

        if env.isScreenLocked { return await missed(m, .screenLocked) }
        // Nothing opened: there is no meeting to leave.
        guard let link = m.conferenceLink?.url, let url = ZoomJoinURL.make(from: link), env.open(url) else {
            return await missed(m, .appFailed)
        }

        // Join: wait for Zoom to show the meeting with the mic muted and the camera off.
        let openedAt = env.now
        let waitingRoomDeadline = min(m.startDate.addingTimeInterval(Self.waitingRoomLimit), m.endDate)
        var joinedAt: Date?
        while true {
            let state = env.zoomState()
            if state == .inMeeting(micMuted: true, videoOff: true) { break }
            if case .inMeeting = state {
                joinedAt = joinedAt ?? env.now
            } else if let failure = joinFailure(state, openedAt: openedAt, waitingRoomDeadline: waitingRoomDeadline) {
                // Zoom never showed the meeting: never press Leave on it. Once it has, always leave.
                return state == .notOpen && joinedAt == nil ? await missed(m, failure) : await leaveAndMiss(m, failure)
            }
            if let joinedAt, env.now.timeIntervalSince(joinedAt) >= Self.muteConfirmTimeout {
                return await leaveAndMiss(m, .notMuted)
            }
            await env.sleep(seconds: Self.pollSeconds)
        }

        // Muted and camera off confirmed: only now record, then disclose in the chat.
        guard let meetingId = await env.startCapture(m) else { return await leaveAndMiss(m, .appFailed) }
        let posted = await env.chat.post(Self.chatMessage(profileName: env.profileName))
        let outcome: AwayOutcome = posted ? .attended : .chatNotPosted
        await env.setOutcome(meetingId: meetingId, outcome: outcome, at: env.now)

        // Stay until the meeting ends, the user is removed, or the scheduled end has passed.
        let leaveBy = m.endDate.addingTimeInterval(Self.overrun)
        while true {
            let state = env.zoomState()
            if state == .removed {
                env.endCapture()
                await env.setOutcome(meetingId: meetingId, outcome: .removed, at: env.now)
                await followup(m, meetingId: meetingId)
                return .removed
            }
            if state == .ended { break }
            if env.now >= leaveBy {
                // Still muted means nobody took over; an unmuted user stays in their meeting.
                if case .inMeeting(micMuted: true, _) = state { _ = await env.leaver.leave() }
                break
            }
            await env.sleep(seconds: Self.pollSeconds)
        }
        env.endCapture()
        return outcome
    }

    private func joinFailure(_ state: ZoomWindowState, openedAt: Date, waitingRoomDeadline: Date) -> AwayOutcome? {
        switch state {
        case .notOpen: return env.now.timeIntervalSince(openedAt) >= Self.appOpenTimeout ? .appFailed : nil
        case .waitingRoom: return env.now >= waitingRoomDeadline ? .waitingRoom : nil
        case .passcodePrompt: return .passcodeRequired
        case .signInRequired: return .signInRequired
        case .removed: return .removed
        // Ended before Whimbrel got in: no outcome fits better.
        case .ended: return .appFailed
        case .inMeeting: return nil
        }
    }

    private func leaveAndMiss(_ m: QualifyingMeeting, _ outcome: AwayOutcome) async -> AwayOutcome {
        _ = await env.leaver.leave()
        return await missed(m, outcome)
    }

    private func missed(_ m: QualifyingMeeting, _ outcome: AwayOutcome) async -> AwayOutcome {
        guard let id = await env.insertMissedMeeting(m) else { return outcome }
        await env.setOutcome(meetingId: id, outcome: outcome, at: env.now)
        await followup(m, meetingId: id)
        return outcome
    }

    private func followup(_ m: QualifyingMeeting, meetingId: Int64) async {
        await env.recordFollowup(AwayFollowupRequest(
            meetingId: meetingId,
            meetingTitle: m.title,
            meetingStartUnixMs: Int64(m.startDate.timeIntervalSince1970 * 1000),
            organizerEmail: m.organizerEmail,
            organizerName: m.organizerName,
            selfIsOrganizer: m.selfIsOrganizer,
            profileName: env.profileName,
            nowUnixMs: Int64(env.now.timeIntervalSince1970 * 1000)
        ))
    }
}
