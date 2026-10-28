import XCTest
@testable import Whimbrel

/// Meeting continue: a calendar meeting that ends before the event does can be prompted
/// again, so a rejoin starts with the calendar key and continues.
@MainActor
final class MeetingTriggerServiceTests: XCTestCase {
    private var defaults: UserDefaults!
    private var suite: String!

    override func setUp() {
        super.setUp()
        suite = "MeetingTriggerServiceTests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
        defaults = nil
        super.tearDown()
    }

    private let start = Date(timeIntervalSince1970: 1_000_000)
    private var event: QualifyingMeeting {
        QualifyingMeeting(id: "EVT-1", title: "Planning", startDate: start, endDate: start.addingTimeInterval(3600),
                          attendees: ["Ana"], conferenceLink: nil, source: .calendar, appBundleID: nil, organizerEmail: nil, organizerName: nil, selfIsOrganizer: false, selfResponse: nil)
    }

    func testEndedBeforeEventEndIsPromptableAgain() {
        let service = MeetingTriggerService(defaults: defaults)
        service.evaluate(meetings: [event], now: start)
        XCTAssertEqual(service.pendingPrompt?.id, "EVT-1")
        service.dismissPrompt()
        service.evaluate(meetings: [event], now: start.addingTimeInterval(60))
        XCTAssertNil(service.pendingPrompt, "acted on once")

        service.meetingEnded(eventId: "EVT-1", reason: .appQuit, now: start.addingTimeInterval(600))
        service.evaluate(meetings: [event], now: start.addingTimeInterval(660))
        XCTAssertEqual(service.pendingPrompt?.id, "EVT-1")
    }

    /// Only an end because the call went away re-arms: a manual End (no reason), silence, the
    /// duration cap or the scheduled-end ceiling mean the user is done, so no re-prompt.
    func testOnlyCallGoneEndsRearm() {
        let rearming: [MeetingEndDetector.EndReason?] = [.appQuit, .captureReleased, .browserTabGone]
        let staying: [MeetingEndDetector.EndReason?] = [nil, .silence, .maxDuration, .scheduledEndHardCeiling]
        for reason in rearming + staying {
            let service = MeetingTriggerService(defaults: defaults)
            service.evaluate(meetings: [event], now: start)
            service.dismissPrompt()
            service.meetingEnded(eventId: "EVT-1", reason: reason, now: start.addingTimeInterval(600))
            service.evaluate(meetings: [event], now: start.addingTimeInterval(660))
            XCTAssertEqual(service.pendingPrompt != nil, rearming.contains(reason), "\(String(describing: reason))")
            defaults.removePersistentDomain(forName: suite)
        }
    }

    func testEndedAfterEventEndStaysActedOn() {
        let service = MeetingTriggerService(defaults: defaults)
        service.evaluate(meetings: [event], now: start)
        service.dismissPrompt()

        service.meetingEnded(eventId: "EVT-1", reason: .appQuit, now: start.addingTimeInterval(3601))
        service.evaluate(meetings: [event], now: start.addingTimeInterval(120))
        XCTAssertNil(service.pendingPrompt)
    }

    /// The un-acting is persisted too, so a relaunch sees it.
    func testEndedIsPersisted() {
        let service = MeetingTriggerService(defaults: defaults)
        service.evaluate(meetings: [event], now: start)
        service.meetingEnded(eventId: "EVT-1", reason: .appQuit, now: start.addingTimeInterval(600))
        let stored = defaults.dictionary(forKey: "com.mpointer.kestrel.meetingTriggerActedOn") ?? [:]
        XCTAssertNil(stored["EVT-1"])
    }
}
