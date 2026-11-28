import WhimbrelUI
import XCTest
@testable import Whimbrel

/// The meeting alert, the check-in window and the hotkey panel as the redesign draws them (`UI/Redesign/AlertPanelsRedesign.swift`,
/// `HotkeyPanelRedesign.swift`). Pure: no panel is built, no timer runs, nothing is sent.
final class AlertPanelRedesignTests: XCTestCase {
    private func meeting(
        detected: Bool = true, earlier: String? = nil, seconds: Int? = nil, brief: Bool = false, join: String? = nil
    ) -> AlertPanelAdapter.MeetingInput {
        AlertPanelAdapter.MeetingInput(title: "Pricing review", isDetected: detected, earlierTitle: earlier, secondsRemaining: seconds, hasBrief: brief, joinProvider: join)
    }

    private final class Calls {
        var log: [String] = []
        var meeting: AlertPanelAdapter.MeetingCallbacks {
            AlertPanelAdapter.MeetingCallbacks(
                accept: { self.log.append("accept") }, acceptAndOpen: { self.log.append("acceptAndOpen") }, dismiss: { self.log.append("dismiss") },
                openBrief: { self.log.append("brief") }, join: { self.log.append("join") })
        }
        var checkIn: AlertPanelAdapter.CheckInCallbacks {
            AlertPanelAdapter.CheckInCallbacks(keep: { self.log.append("keep") }, stop: { self.log.append("stop") }, resume: { self.log.append("resume") })
        }
    }

    // MARK: Meeting alert

    func testAPlainAlertStartsRecordingFirstAndKeepsEveryButton() {
        let calls = Calls()
        let model = AlertPanelAdapter.meeting(meeting(), calls.meeting)
        XCTAssertEqual(model.main.id, "start")
        XCTAssertEqual(model.alternate?.id, "start-and-open")
        XCTAssertEqual(model.others.map(\.id), ["dismiss"])
        XCTAssertNil(model.countdown)
        XCTAssertEqual(model.subject, "Pricing review")
        // What the old alert drew and no more: the title and the meeting. No sentence about the app, no time of day.
        XCTAssertTrue(model.lines.isEmpty)
        XCTAssertFalse(model.main.isSafeDefault)
        model.main.run(); model.alternate?.run(); model.others[0].run()
        XCTAssertEqual(calls.log, ["accept", "acceptAndOpen", "dismiss"])
    }

    func testBriefAndJoinAppearOnlyWhenTheMeetingHasThemAndJoinIsNamedForItsProvider() {
        let calls = Calls()
        let model = AlertPanelAdapter.meeting(meeting(detected: false, brief: true, join: "Zoom"), calls.meeting)
        XCTAssertEqual(model.others.map(\.id), ["brief", "join", "dismiss"])
        XCTAssertEqual(model.others[1].spokenName, String(localized: "Join on \("Zoom")"))
        XCTAssertEqual(AlertPanelAdapter.meeting(meeting(), calls.meeting).others.map(\.id), ["dismiss"])
    }

    func testTheCountdownHasOneWayOutAndItIsDismissNamedForWhatItDoes() throws {
        let calls = Calls()
        let model = AlertPanelAdapter.meeting(meeting(seconds: 8), calls.meeting)
        let countdown = try XCTUnwrap(model.countdown)
        // Compared with the same localized call the adapter makes: another language or numeral system writes the 8 differently.
        let eight = 8
        XCTAssertEqual(countdown.text, String(localized: "Recording starts in \(eight) s", comment: "Countdown on the meeting alert when auto-start is on. The placeholder is the seconds left."))
        // One control, not two: no Cancel beside the timer and a Dismiss below it.
        XCTAssertNil(countdown.cancel)
        XCTAssertEqual(model.others.map(\.id), ["dismiss"])
        let way = model.others[0]
        // `dismissPrompt` closes the alert and the meeting is not offered again, so the label is not a bare "Cancel".
        XCTAssertEqual(way.title, String(localized: "Don\u{2019}t record"))
        XCTAssertNotEqual(way.title, String(localized: "Cancel"))
        XCTAssertNotNil(way.help)
        way.run()
        XCTAssertEqual(calls.log, ["dismiss"])
        // No countdown: the button keeps its old name.
        let plain = AlertPanelAdapter.meeting(meeting(seconds: nil), calls.meeting)
        XCTAssertNil(plain.countdown)
        XCTAssertEqual(plain.others.map(\.title), [String(localized: "Dismiss")])
    }

    func testOnlyKeepRecordingIsReturnsDefaultAndNothingThatStartsResumesStopsOrJoinsRecordingsIs() throws {
        let calls = Calls()
        var main: [WAlertAction] = []
        main.append(AlertPanelAdapter.meeting(meeting(), calls.meeting).main)                       // Start Recording
        main.append(AlertPanelAdapter.meeting(meeting(earlier: "Roadmap"), calls.meeting).main)    // Keep together
        main.append(try XCTUnwrap(AlertPanelAdapter.checkIn(kind: .paused, secondsRemaining: nil, calls.checkIn)).main)  // Resume
        XCTAssertTrue(main.allSatisfy { !$0.isSafeDefault })
        let keep = try XCTUnwrap(AlertPanelAdapter.checkIn(kind: .recording, secondsRemaining: 60, calls.checkIn))
        XCTAssertTrue(keep.main.isSafeDefault)
        XCTAssertTrue(try XCTUnwrap(AlertPanelAdapter.checkIn(kind: .stillInMeeting, secondsRemaining: 60, calls.checkIn)).main.isSafeDefault)
        // No other button is a default.
        let others = [keep.others, AlertPanelAdapter.meeting(meeting(brief: true, join: "Zoom"), calls.meeting).others].flatMap { $0 }
        XCTAssertTrue(others.allSatisfy { !$0.isSafeDefault })
    }

    func testTheAppIconIsLookedUpOncePerAppNotOnEveryTick() async {
        await MainActor.run {
            let cache = AppIconCache()
            var asked = 0
            let lookup: (String) -> NSImage? = { _ in asked += 1; return NSImage(size: NSSize(width: 1, height: 1)) }
            for _ in 0..<10 { XCTAssertNotNil(cache.icon(forBundleID: "us.zoom.xos", lookup: lookup)) }
            XCTAssertEqual(asked, 1)
            // An app that is not there is remembered too.
            let none: (String) -> NSImage? = { _ in asked += 1; return nil }
            for _ in 0..<5 { XCTAssertNil(cache.icon(forBundleID: "com.example.gone", lookup: none)) }
            XCTAssertEqual(asked, 2)
            XCTAssertEqual(cache.lookups, 2)
        }
    }

    func testTheTitleFollowsTheSource() {
        XCTAssertEqual(AlertPanelAdapter.meetingTitle(meeting(detected: true)), String(localized: "Meeting Detected"))
        XCTAssertEqual(AlertPanelAdapter.meetingTitle(meeting(detected: false)), String(localized: "Meeting Starting Soon"))
        XCTAssertEqual(AlertPanelAdapter.meetingTitle(meeting(earlier: "Roadmap")), String(localized: "Recording as a New Meeting"))
    }

    func testASplitOffersKeepTogetherAsTheMainActionAndSaysWhatEnded() {
        let calls = Calls()
        let model = AlertPanelAdapter.meeting(meeting(earlier: "Roadmap check-in"), calls.meeting)
        XCTAssertEqual(model.main.id, "keep-together")
        XCTAssertNil(model.alternate)
        XCTAssertNotNil(model.main.help)
        XCTAssertTrue(model.lines.first?.contains("Roadmap check-in") == true)
        model.main.run()
        XCTAssertEqual(calls.log, ["accept"])
        XCTAssertEqual(model.actionIDs.first, "keep-together")
    }

    // MARK: Check-in

    func testTheHourlyCheckInSaysHowLongItHasRunNotThatNoOneSpoke() throws {
        let calls = Calls()
        let model = try XCTUnwrap(AlertPanelAdapter.checkIn(kind: .recording, secondsRemaining: 272, calls.checkIn))
        XCTAssertEqual(model.title, String(localized: "Still recording?"))
        XCTAssertEqual(model.lines.count, 1)
        XCTAssertFalse(model.lines[0].contains("spoken"))
        XCTAssertTrue(model.countdown?.text.contains("4:32") == true)
        XCTAssertNil(model.countdown?.cancel)
        XCTAssertEqual(model.main.id, "keep")
        XCTAssertTrue(model.main.isSafeDefault)
        XCTAssertEqual(model.others.map(\.title), [String(localized: "Stop now")])
        model.main.run(); model.others[0].run()
        XCTAssertEqual(calls.log, ["keep", "stop"])
    }

    func testStillInAMeetingSaysNoOneSpokeAndThatMatchesTheRule() throws {
        let model = try XCTUnwrap(AlertPanelAdapter.checkIn(kind: .stillInMeeting, secondsRemaining: 108, Calls().checkIn))
        XCTAssertEqual(model.title, String(localized: "Still in a meeting?"))
        // The number comes from the rule, so the line cannot drift from it.
        let minutes = StillInMeeting.quietSeconds / 60
        XCTAssertEqual(model.lines, [String(localized: "No one has spoken for \(minutes) minutes.")])
        XCTAssertEqual(model.lines[0], String(localized: "No one has spoken for \(minutes) minutes."))
        let other = try XCTUnwrap(AlertPanelAdapter.checkIn(kind: .stillInMeeting, secondsRemaining: 108, quietMinutes: 20, Calls().checkIn))
        let twenty = 20
        XCTAssertEqual(other.lines[0], String(localized: "No one has spoken for \(twenty) minutes."))
        XCTAssertEqual(StillInMeeting.quietSeconds, 900)
        XCTAssertEqual(model.others.map(\.title), [String(localized: "Stop")])
    }

    func testNothingShowsWhileACountdownHasNoSeconds() {
        XCTAssertNil(AlertPanelAdapter.checkIn(kind: .recording, secondsRemaining: nil, Calls().checkIn))
        XCTAssertNil(AlertPanelAdapter.checkIn(kind: .stillInMeeting, secondsRemaining: nil, Calls().checkIn))
    }

    func testThePausedReminderHasNoCountdownAndOffersResumeAndStop() throws {
        let calls = Calls()
        let model = try XCTUnwrap(AlertPanelAdapter.checkIn(kind: .paused, secondsRemaining: nil, calls.checkIn))
        XCTAssertNil(model.countdown)
        // Only audio stops while paused; memory keeps running, so the line must not say "nothing".
        XCTAssertEqual(model.lines, [String(localized: "No audio is recorded while it is paused.")])
        XCTAssertFalse(model.lines[0].contains("Nothing"))
        XCTAssertFalse(model.main.isSafeDefault)
        XCTAssertEqual(model.main.id, "resume")
        XCTAssertEqual(model.others.map(\.id), ["stop"])
        model.main.run(); model.others[0].run()
        XCTAssertEqual(calls.log, ["resume", "stop"])
    }

    func testNoTextHasADash() throws {
        let calls = Calls()
        var texts: [String] = []
        for input in [meeting(), meeting(detected: false, seconds: 5, brief: true, join: "Zoom"), meeting(earlier: "Roadmap")] {
            let m = AlertPanelAdapter.meeting(input, calls.meeting)
            texts += [m.title] + m.lines + (m.countdown.map { [$0.text] } ?? []) + [m.main.title] + m.others.map(\.title)
        }
        for kind in [AlertPanelAdapter.CheckInKind.recording, .stillInMeeting, .paused] {
            if let m = AlertPanelAdapter.checkIn(kind: kind, secondsRemaining: 60, calls.checkIn) {
                texts += [m.title] + m.lines + (m.countdown.map { [$0.text] } ?? []) + [m.main.title] + m.others.map(\.title)
            }
        }
        for text in texts {
            XCTAssertFalse(text.contains("\u{2014}") || text.contains("\u{2013}"), text)
        }
    }

    // MARK: Hotkey panel

    private func context(_ reading: String = "Reading: Safari", _ destination: String = "Sent to your chosen model.") -> HotkeyPanelModel.ContextLine {
        HotkeyPanelModel.ContextLine(reading: reading, destination: destination, truncated: false)
    }

    func testTheReadingAndDestinationLinesPassThroughUnchanged() {
        let model = HotkeyPanelAdapter.model(state: .ready(context()), context: context(), partialAnswer: "")
        XCTAssertEqual(model.phase, .ready)
        XCTAssertEqual(model.readingLine, "Reading: Safari")
        XCTAssertEqual(model.destinationLine, "Sent to your chosen model.")
        let none = HotkeyPanelAdapter.model(state: .reading, context: nil, partialAnswer: "")
        XCTAssertEqual(none.phase, .reading)
        XCTAssertNil(none.readingLine)
        XCTAssertNil(none.destinationLine)
    }

    func testEveryStateHasAPhase() {
        func phase(_ state: HotkeyPanelModel.State, partial: String = "") -> WHotkeyPhase {
            HotkeyPanelAdapter.model(state: state, context: context(), partialAnswer: partial).phase
        }
        XCTAssertEqual(phase(.answering, partial: "The sheet"), .answering(partial: "The sheet"))
        XCTAssertEqual(phase(.answered("Done")), .answered("Done"))
        XCTAssertEqual(phase(.failed("No answer")), .failed("No answer"))
    }

    func testEachBlockedStateKeepsItsMessageAndItsButton() {
        let cases: [(HotkeyPanelModel.Reason, String?)] = [
            (.notPermitted, HotkeyPanelAdapter.accessibilityActionId),
            (.localModelMissing, HotkeyPanelAdapter.localModelActionId),
            (.clientNeedsLocalModel, HotkeyPanelAdapter.localModelActionId),
            (.excluded("Mail"), nil),
            (.unreadable("Mail"), nil),
        ]
        for (reason, id) in cases {
            let phase = HotkeyPanelAdapter.model(state: .blocked(reason), context: nil, partialAnswer: "").phase
            guard case let .blocked(message, action) = phase else { return XCTFail("expected blocked") }
            XCTAssertEqual(message, reason.message)
            XCTAssertEqual(action?.id, id)
        }
        XCTAssertEqual(HotkeyPanelAdapter.action(for: .notPermitted)?.title, String(localized: "Open System Settings"))
        XCTAssertEqual(HotkeyPanelAdapter.action(for: .localModelMissing)?.title, String(localized: "Install a local model"))
    }

    func testTheCopyButtonHasItsOwnCopiedWord() {
        let model = HotkeyPanelAdapter.model(state: .reading, context: nil, partialAnswer: "")
        // Compared with the app's own lookup, so a Mac set to another language passes too.
        XCTAssertEqual(model.copiedTitle, String(localized: "hotkey.copied", defaultValue: "Copied"))
        XCTAssertFalse(model.copiedTitle.isEmpty)
    }

    func testThePanelStaysInsideTheScreenAndIsCentredWhenItFits() {
        let screen = NSRect(x: 0, y: 40, width: 1440, height: 860)
        let size = NSSize(width: 548, height: 160)
        let p = HotkeyPanelController.topLeft(forPanelOf: size, in: screen)
        XCTAssertEqual(p.x, 1440 / 2 - 548 / 2, accuracy: 0.5)
        XCTAssertEqual(p.y, screen.maxY - screen.height * 0.2, accuracy: 0.5)
        // A narrow screen: still on it, with a margin.
        let narrow = NSRect(x: 100, y: 0, width: 600, height: 700)
        let q = HotkeyPanelController.topLeft(forPanelOf: NSSize(width: 580, height: 200), in: narrow)
        XCTAssertGreaterThanOrEqual(q.x, narrow.minX + 8)
        XCTAssertLessThanOrEqual(q.x + 580, narrow.maxX - 8 + 0.5)
        // Wider than the screen: left aligned, never off to the left.
        let wide = HotkeyPanelController.topLeft(forPanelOf: NSSize(width: 900, height: 200), in: narrow)
        XCTAssertEqual(wide.x, narrow.minX + 8)
        // Taller than the room below the usual top: moved up so its bottom stays on screen.
        let tall = HotkeyPanelController.topLeft(forPanelOf: NSSize(width: 548, height: 650), in: narrow)
        XCTAssertGreaterThanOrEqual(tall.y - 650, narrow.minY + 8 - 0.5)
        XCTAssertLessThanOrEqual(tall.y, narrow.maxY - 8)
    }

    @MainActor
    func testTheHotkeyPanelAsksToBeLeftOutOfScreenSharing() throws {
        XCTAssertTrue(HotkeyPanelController.hidesFromScreenSharing)
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/Support/HotkeyPanelController.swift")
        let text = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(text.contains("panel.sharingType = Self.hidesFromScreenSharing ? .none : .readOnly"))
        // Nothing promises the window is hidden: it is a request, and some sharing apps ignore it.
        XCTAssertTrue(text.contains("Some sharing apps ignore the request"))
    }
}
