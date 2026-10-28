import XCTest
@testable import Whimbrel

private let t0 = Date(timeIntervalSince1970: 1_800_000_000)

private final class FakeChat: ZoomChatPosting {
    var result = true
    var posts: [String] = []
    func post(_ text: String) async -> Bool {
        posts.append(text)
        return result
    }
}

private final class FakeLeaver: ZoomLeaving {
    var leaves = 0
    func leave() async -> Bool {
        leaves += 1
        return true
    }
}

/// A scripted Zoom window (the last state repeats) on a fake clock that each sleep advances.
@MainActor
private final class FakeAwayEnvironment: AwayEnvironment {
    struct Outcome: Equatable {
        let meetingId: Int64
        let outcome: AwayOutcome
    }

    var now = t0
    var isScreenLocked = false
    var isMicInUse = false
    var isRecording = false
    var profileName = "Mike"
    var openResult = true
    var states: [ZoomWindowState]
    let captureId: Int64 = 100

    private(set) var opened: [URL] = []
    private(set) var lastState: ZoomWindowState?
    /// The Zoom state last read when each capture started.
    private(set) var stateAtCapture: [ZoomWindowState?] = []
    private(set) var endCaptures = 0
    private(set) var missedRows: [String] = []
    private(set) var outcomes: [Outcome] = []
    private(set) var followups: [AwayFollowupRequest] = []
    let fakeChat = FakeChat()
    let fakeLeaver = FakeLeaver()
    var chat: ZoomChatPosting { fakeChat }
    var leaver: ZoomLeaving { fakeLeaver }

    init(states: [ZoomWindowState]) { self.states = states }

    func open(_ url: URL) -> Bool {
        opened.append(url)
        return openResult
    }

    func zoomState() -> ZoomWindowState {
        let state = states.count > 1 ? states.removeFirst() : states[0]
        lastState = state
        return state
    }

    func startCapture(_ m: QualifyingMeeting) async -> Int64? {
        stateAtCapture.append(lastState)
        isRecording = true
        return captureId
    }

    func endCapture() {
        endCaptures += 1
        isRecording = false
    }

    func insertMissedMeeting(_ m: QualifyingMeeting) async -> Int64? {
        missedRows.append(m.id)
        return 200 + Int64(missedRows.count)
    }

    func setOutcome(meetingId: Int64, outcome: AwayOutcome, at: Date) async {
        outcomes.append(Outcome(meetingId: meetingId, outcome: outcome))
    }

    func recordFollowup(_ req: AwayFollowupRequest) async {
        followups.append(req)
    }

    func sleep(seconds: Double) async {
        now = now.addingTimeInterval(seconds)
        await Task.yield()
    }
}

@MainActor
final class AwayModeCoordinatorTests: XCTestCase {
    private func meeting(id: String = "m1", start: Date = t0, minutes: Double = 30,
                         organizer: Bool = false, response: SelfResponse? = .accepted,
                         source: MeetingSource = .calendar) -> QualifyingMeeting {
        QualifyingMeeting(
            id: id, title: "Weekly sync", startDate: start, endDate: start.addingTimeInterval(minutes * 60),
            attendees: ["Ana"],
            conferenceLink: DetectedConferenceLink(provider: .zoom, url: URL(string: "https://acme.zoom.us/j/123456789?pwd=abc")!),
            source: source, appBundleID: nil, organizerEmail: "ana@acme.com", organizerName: "Ana",
            selfIsOrganizer: organizer, selfResponse: response
        )
    }

    private func coordinator(_ env: FakeAwayEnvironment, enabled: Bool = true) -> AwayModeCoordinator {
        AwayModeCoordinator(env: env, isEnabled: { enabled })
    }

    /// Nothing was captured or posted, Zoom was left, and the meeting got a missed row and a follow-up.
    private func assertMissed(_ env: FakeAwayEnvironment, _ outcome: AwayOutcome, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(env.stateAtCapture.isEmpty, "capture started", file: file, line: line)
        XCTAssertTrue(env.fakeChat.posts.isEmpty, "chat posted", file: file, line: line)
        XCTAssertEqual(env.missedRows, ["m1"], file: file, line: line)
        XCTAssertEqual(env.outcomes, [.init(meetingId: 201, outcome: outcome)], file: file, line: line)
        XCTAssertEqual(env.followups.map(\.meetingId), [201], file: file, line: line)
    }

    func test_leaves_when_not_muted() async {
        let env = FakeAwayEnvironment(states: [.notOpen, .inMeeting(micMuted: false, videoOff: true)])
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.notMuted))
        XCTAssertEqual(env.fakeLeaver.leaves, 1)
        assertMissed(env, .notMuted)
    }

    func test_camera_on_is_not_muted() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: false)])
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.notMuted))
        XCTAssertEqual(env.fakeLeaver.leaves, 1)
        assertMissed(env, .notMuted)
    }

    func test_passcode_prompt_is_failure() async {
        let env = FakeAwayEnvironment(states: [.notOpen, .passcodePrompt])
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.passcodeRequired))
        XCTAssertEqual(env.fakeLeaver.leaves, 1)
        assertMissed(env, .passcodeRequired)
    }

    func test_stands_aside_when_mic_in_use() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true)])
        env.isMicInUse = true
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .stoodAside)
        XCTAssertTrue(env.opened.isEmpty)
        XCTAssertTrue(env.fakeChat.posts.isEmpty)
        XCTAssertTrue(env.missedRows.isEmpty)
    }

    func test_stands_aside_when_recording() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true)])
        env.isRecording = true
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .stoodAside)
        XCTAssertTrue(env.opened.isEmpty)
    }

    func test_overlap_marks_second_missed() async {
        let env = FakeAwayEnvironment(states: Array(repeating: .inMeeting(micMuted: true, videoOff: true), count: 5) + [.ended])
        let c = coordinator(env)
        let first = Task { await c.run(for: meeting(id: "m1")) }
        // Wait until the first run is in the meeting and recording.
        for _ in 0..<1000 where env.stateAtCapture.isEmpty { await Task.yield() }
        XCTAssertTrue(c.isRunning)
        XCTAssertTrue(env.isRecording)

        let second = await c.run(for: meeting(id: "m2", start: t0.addingTimeInterval(5 * 60)))
        XCTAssertEqual(second, .finished(.missedOverlap))
        XCTAssertEqual(env.missedRows, ["m2"])
        XCTAssertEqual(env.followups.map(\.meetingId), [201])
        XCTAssertEqual(env.opened.count, 1)

        let firstOutcome = await first.value
        XCTAssertEqual(firstOutcome, .finished(.attended))
        XCTAssertFalse(c.isRunning)
    }

    func test_app_never_opens_is_failure() async {
        let env = FakeAwayEnvironment(states: [.notOpen])
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.appFailed))
        XCTAssertGreaterThanOrEqual(env.now.timeIntervalSince(t0), 60)
        XCTAssertLessThan(env.now.timeIntervalSince(t0), 63)
        XCTAssertEqual(env.fakeLeaver.leaves, 0)
        assertMissed(env, .appFailed)
    }

    func test_seen_then_lost_window_still_leaves() async {
        // In the meeting (unmuted) at 50 s, then the window reads as gone past the 60 s mark.
        let env = FakeAwayEnvironment(states: Array(repeating: .notOpen, count: 25)
            + [.inMeeting(micMuted: false, videoOff: true), .notOpen])
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.appFailed))
        XCTAssertEqual(env.fakeLeaver.leaves, 1)
        assertMissed(env, .appFailed)
    }

    func test_open_refused_is_failure_at_once() async {
        let env = FakeAwayEnvironment(states: [.notOpen])
        env.openResult = false
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.appFailed))
        XCTAssertEqual(env.now, t0)
        XCTAssertEqual(env.fakeLeaver.leaves, 0)
        assertMissed(env, .appFailed)
    }

    func test_happy_path_attended() async {
        let env = FakeAwayEnvironment(states: [
            .notOpen, .inMeeting(micMuted: true, videoOff: true), .inMeeting(micMuted: true, videoOff: true), .ended,
        ])
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.attended))
        XCTAssertEqual(env.opened, [URL(string: "zoommtg://zoom.us/join?action=join&confno=123456789&pwd=abc")!])
        XCTAssertEqual(env.stateAtCapture, [.inMeeting(micMuted: true, videoOff: true)])
        XCTAssertEqual(env.fakeChat.posts, [
            "Mike can't make it live. Their Whimbrel notes app is capturing this meeting so they can catch up.",
        ])
        XCTAssertEqual(env.endCaptures, 1)
        XCTAssertEqual(env.outcomes, [.init(meetingId: 100, outcome: .attended)])
        XCTAssertTrue(env.followups.isEmpty)
        XCTAssertTrue(env.missedRows.isEmpty)
        XCTAssertEqual(env.fakeLeaver.leaves, 0)
    }

    func test_mute_confirmed_within_window_joins() async {
        let env = FakeAwayEnvironment(states: [
            .inMeeting(micMuted: false, videoOff: false), .inMeeting(micMuted: true, videoOff: false),
            .inMeeting(micMuted: true, videoOff: true), .ended,
        ])
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.attended))
        XCTAssertEqual(env.stateAtCapture, [.inMeeting(micMuted: true, videoOff: true)])
    }

    func test_scheduled_end_leaves_while_still_muted() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true)])
        let outcome = await coordinator(env).run(for: meeting(minutes: 10))
        XCTAssertEqual(outcome, .finished(.attended))
        XCTAssertGreaterThanOrEqual(env.now, t0.addingTimeInterval(15 * 60))
        XCTAssertEqual(env.endCaptures, 1)
        XCTAssertEqual(env.fakeLeaver.leaves, 1)
    }

    func test_scheduled_end_stays_when_user_took_over() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true), .inMeeting(micMuted: false, videoOff: true)])
        let outcome = await coordinator(env).run(for: meeting(minutes: 10))
        XCTAssertEqual(outcome, .finished(.attended))
        XCTAssertEqual(env.endCaptures, 1)
        XCTAssertEqual(env.fakeLeaver.leaves, 0)
    }

    func test_chat_copy_without_a_name() {
        XCTAssertEqual(AwayModeCoordinator.chatMessage(profileName: "  "),
                       "I can't make it live. My Whimbrel notes app is capturing this meeting so I can catch up.")
    }

    func test_chat_failure_stays_and_records() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true), .ended])
        env.fakeChat.result = false
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.chatNotPosted))
        XCTAssertEqual(env.stateAtCapture.count, 1)
        XCTAssertEqual(env.fakeChat.posts.count, 1)
        XCTAssertEqual(env.outcomes, [.init(meetingId: 100, outcome: .chatNotPosted)])
        XCTAssertTrue(env.followups.isEmpty)
        XCTAssertEqual(env.endCaptures, 1)
    }

    func test_removed_keeps_partial() async {
        let env = FakeAwayEnvironment(states: [
            .inMeeting(micMuted: true, videoOff: true), .inMeeting(micMuted: true, videoOff: true), .removed,
        ])
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.removed))
        XCTAssertEqual(env.endCaptures, 1)
        XCTAssertEqual(env.outcomes, [.init(meetingId: 100, outcome: .attended), .init(meetingId: 100, outcome: .removed)])
        XCTAssertEqual(env.followups.count, 1)
        let f = env.followups[0]
        XCTAssertEqual(f.meetingId, 100)
        XCTAssertEqual(f.organizerEmail, "ana@acme.com")
        XCTAssertEqual(f.organizerName, "Ana")
        XCTAssertEqual(f.profileName, "Mike")
        XCTAssertEqual(f.meetingStartUnixMs, Int64(t0.timeIntervalSince1970 * 1000))
        XCTAssertTrue(env.missedRows.isEmpty)
    }

    func test_locked_screen() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true)])
        env.isScreenLocked = true
        let outcome = await coordinator(env).run(for: meeting())
        XCTAssertEqual(outcome, .finished(.screenLocked))
        XCTAssertTrue(env.opened.isEmpty)
        assertMissed(env, .screenLocked)
    }

    func test_waiting_room_timeout() async {
        let env = FakeAwayEnvironment(states: [.notOpen, .waitingRoom])
        let outcome = await coordinator(env).run(for: meeting(minutes: 60))
        XCTAssertEqual(outcome, .finished(.waitingRoom))
        XCTAssertGreaterThanOrEqual(env.now, t0.addingTimeInterval(10 * 60))
        XCTAssertLessThan(env.now, t0.addingTimeInterval(10 * 60 + 3))
        XCTAssertEqual(env.fakeLeaver.leaves, 1)
        assertMissed(env, .waitingRoom)
    }

    func test_waiting_room_gives_up_at_a_short_meetings_end() async {
        let env = FakeAwayEnvironment(states: [.waitingRoom])
        let outcome = await coordinator(env).run(for: meeting(minutes: 5))
        XCTAssertEqual(outcome, .finished(.waitingRoom))
        XCTAssertLessThan(env.now, t0.addingTimeInterval(5 * 60 + 3))
    }

    func test_ineligible_declined_or_organizer() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true)])
        let c = coordinator(env)
        let declined = await c.run(for: meeting(response: .declined))
        XCTAssertEqual(declined, .notHandled)
        let organized = await c.run(for: meeting(organizer: true))
        XCTAssertEqual(organized, .notHandled)
        XCTAssertTrue(env.opened.isEmpty)
        XCTAssertTrue(env.missedRows.isEmpty)

        XCTAssertTrue(AwayModeCoordinator.eligible(meeting(response: nil)))
        XCTAssertTrue(AwayModeCoordinator.eligible(meeting(response: .tentative)))
        XCTAssertTrue(AwayModeCoordinator.eligible(meeting(response: .needsAction)))
    }

    func test_disabled_before_start_hands_back() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true)])
        let outcome = await coordinator(env, enabled: false).run(for: meeting())
        XCTAssertEqual(outcome, .stoodAside)
        XCTAssertTrue(env.opened.isEmpty)
        XCTAssertTrue(env.missedRows.isEmpty)
    }

    func test_present_hands_back_to_alert() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true)])
        let c = coordinator(env)
        XCTAssertFalse(c.userIsPresent)
        env.isMicInUse = true
        XCTAssertTrue(c.userIsPresent)
        let outcome = await c.run(for: meeting())
        XCTAssertEqual(outcome, .stoodAside)
        XCTAssertTrue(env.opened.isEmpty)
        XCTAssertTrue(env.missedRows.isEmpty)
        XCTAssertTrue(env.followups.isEmpty)
    }

    func test_own_join_is_not_user_presence() async {
        let env = FakeAwayEnvironment(states: Array(repeating: .inMeeting(micMuted: true, videoOff: true), count: 3) + [.ended])
        let c = coordinator(env)
        let run = Task { await c.run(for: meeting()) }
        for _ in 0..<1000 where env.stateAtCapture.isEmpty { await Task.yield() }
        env.isMicInUse = true
        XCTAssertFalse(c.userIsPresent)
        _ = await run.value
    }

    func test_google_calendar_meeting_eligible() {
        XCTAssertTrue(AwayModeCoordinator.eligible(meeting(source: .googleCalendar)))
    }

    func test_detected_meeting_ineligible() {
        XCTAssertFalse(AwayModeCoordinator.eligible(meeting(source: .detected)))
    }

    func test_takes_prompt_only_when_join_available_and_enabled() {
        XCTAssertTrue(AwayModeCoordinator.takesPrompt(meeting(), enabled: true, joinAvailable: true))
        XCTAssertFalse(AwayModeCoordinator.takesPrompt(meeting(), enabled: true, joinAvailable: false))
        XCTAssertFalse(AwayModeCoordinator.takesPrompt(meeting(), enabled: false, joinAvailable: true))
        XCTAssertFalse(AwayModeCoordinator.takesPrompt(meeting(response: .declined), enabled: true, joinAvailable: true))
        // Until Task 7b the live join is off, so the prompt flow is unchanged.
        XCTAssertFalse(AppState.awayZoomJoinEnabled)
    }

    func test_same_meeting_never_rejoins() async {
        let env = FakeAwayEnvironment(states: [.inMeeting(micMuted: true, videoOff: true), .removed])
        let c = coordinator(env)
        let first = await c.run(for: meeting())
        XCTAssertEqual(first, .finished(.removed))
        let again = await c.run(for: meeting())
        XCTAssertEqual(again, .notHandled)
        XCTAssertEqual(env.opened.count, 1)
    }
}
