import XCTest
@testable import Whimbrel

/// Back-to-back meetings: when the next calendar meeting starts during a recording, the
/// recording splits (rules in `MeetingSplit.shouldSplit`), and "Keep together" folds the
/// new meeting back into the earlier one as a part.
@MainActor
final class MeetingSplitTests: XCTestCase {
    /// 11:00 on an arbitrary day, in seconds; times below are minutes from here.
    private let base: TimeInterval = 1_800_000_000
    private func at(_ minutes: Double) -> Date { Date(timeIntervalSince1970: base + minutes * 60) }

    private let zoomA = "https://us02web.zoom.us/j/111"
    private let zoomB = "https://us02web.zoom.us/j/222"
    private let meetB = "https://meet.google.com/bbb-bbbb-bbb"

    private func link(_ url: String?) -> DetectedConferenceLink? {
        url.map { DetectedConferenceLink(provider: $0.contains("zoom") ? .zoom : .googleMeet, url: URL(string: $0)!) }
    }

    private func event(id: String = "B", start: Double, end: Double, url: String? = nil,
                       response: SelfResponse? = .accepted, organizer: Bool = false,
                       attendees: [String] = ["Ana"]) -> QualifyingMeeting {
        QualifyingMeeting(
            id: id, title: "Event \(id)", startDate: at(start), endDate: at(end), attendees: attendees,
            conferenceLink: link(url), source: .calendar, appBundleID: nil, organizerEmail: nil,
            organizerName: nil, selfIsOrganizer: organizer, selfResponse: response
        )
    }

    /// A calendar recording of event "A" (its own link), started at `started`.
    private func calendarRecording(end: Double, url: String? = nil, started: Double = 0) -> MeetingSplit.Current {
        MeetingSplit.Current(
            calendarEvent: .init(id: "A", conferenceLink: link(url), scheduledEnd: at(end)),
            link: link(url), meetingStartedAt: at(started)
        )
    }

    private func split(_ current: MeetingSplit.Current, _ next: QualifyingMeeting, now: Double, live: String? = nil) -> Bool {
        MeetingSplit.shouldSplit(current: current, next: next, liveCallURL: { live }, now: at(now))
    }

    // MARK: Back to back

    /// Oct 28: same Zoom app, nothing tells the calls apart, A's scheduled end is B's start.
    func testBackToBackInTheSameAppSplitsAtTheSecondMeetingsStart() {
        let current = calendarRecording(end: 30, url: zoomA)
        let next = event(start: 30, end: 60, url: zoomB)
        XCTAssertFalse(split(current, next, now: 29), "not before the next meeting starts")
        XCTAssertTrue(split(current, next, now: 30))
        XCTAssertTrue(split(current, next, now: 31))
        XCTAssertFalse(split(current, next, now: 60), "the next meeting is over")
    }

    func testBackToBackInDifferentAppsSplits() {
        let current = calendarRecording(end: 30, url: zoomA)
        let next = event(start: 30, end: 60, url: meetB)
        XCTAssertTrue(split(current, next, now: 30, live: meetB))
        XCTAssertTrue(split(current, next, now: 30), "the schedule alone is enough")
    }

    /// Rule 2: the current event runs until 12:00, but the call he is in is now B's.
    func testCallMovedToTheNextMeetingsLinkSplitsEvenBeforeTheCurrentEventEnds() {
        let current = calendarRecording(end: 60, url: zoomA)
        let next = event(start: 30, end: 60, url: meetB)
        XCTAssertTrue(split(current, next, now: 30, live: meetB + "?authuser=0"))
        XCTAssertTrue(split(current, next, now: 30, live: "https://meet.google.com/ccc-cccc-ccc"), "any change of call")
        XCTAssertFalse(split(current, next, now: 30), "no evidence: an overlap stays whole")
    }

    /// The same link on both events (a personal meeting room) doesn't prove a new call.
    func testTheSameLinkOnBothEventsIsNotACallChange() {
        let current = calendarRecording(end: 60, url: zoomA)
        let next = event(start: 30, end: 90, url: zoomA)
        XCTAssertFalse(split(current, next, now: 30, live: zoomA))
        XCTAssertTrue(split(current, next, now: 60, live: zoomA), "rule 1 still splits at the current event's end")
    }

    // MARK: Overrun

    func testAnOverrunningMeetingSplitsAtTheAcceptedNextStart() {
        let current = calendarRecording(end: 0 + 30, url: zoomA)  // 11:00-11:30, still going
        let next = event(start: 60, end: 90, url: zoomB)          // 12:00
        XCTAssertFalse(split(current, next, now: 45), "overrunning alone never splits")
        XCTAssertTrue(split(current, next, now: 60))
    }

    // MARK: Never split

    func testADeclinedNextMeetingNeverSplits() {
        let current = calendarRecording(end: 60, url: zoomA)
        let declined = event(start: 30, end: 90, url: meetB, response: .declined)
        XCTAssertFalse(split(current, declined, now: 30))
        XCTAssertFalse(split(current, declined, now: 61), "not even after the current event ends")
        XCTAssertFalse(split(current, declined, now: 30, live: meetB))
    }

    func testATentativeOrUnansweredNextMeetingSplitsOnlyWhenItIsTheCallHesIn() {
        let current = calendarRecording(end: 30, url: zoomA)
        for response in [SelfResponse.tentative, .needsAction] {
            let next = event(start: 30, end: 60, url: meetB, response: response)
            XCTAssertFalse(split(current, next, now: 30), "\(response)")
            XCTAssertFalse(split(current, next, now: 30, live: zoomA), "\(response): still in the current call")
            XCTAssertTrue(split(current, next, now: 30, live: meetB), "\(response): in its call")
        }
    }

    func testAnAcceptedOverlapWithoutEvidenceDoesNotSplit() {
        let current = calendarRecording(end: 60, url: zoomA)
        XCTAssertFalse(split(current, event(start: 30, end: 45, url: zoomB), now: 30))
    }

    /// Meeting continue rejoins the same event; it is never "the next meeting".
    func testTheSameEventNeverSplits() {
        let current = calendarRecording(end: 30, url: zoomA)
        XCTAssertFalse(split(current, event(id: "A", start: 0, end: 30, url: zoomA), now: 31))
        XCTAssertFalse(split(current, event(id: "A", start: 40, end: 70, url: zoomA), now: 40))
    }

    /// An event that started before this recording did isn't the next one.
    func testAnEventThatStartedBeforeTheRecordingIsNotNext() {
        let current = calendarRecording(end: 30, url: zoomA, started: 5)
        XCTAssertFalse(split(current, event(start: 0, end: 60, url: zoomB), now: 31))
    }

    // MARK: Manual and detected

    /// Oct 29: a manual recording from 13:13 runs into the 15:00 meeting.
    func testAManualRecordingSplitsAtAnAcceptedCalendarMeeting() {
        let manual = MeetingSplit.Current(calendarEvent: nil, link: nil, meetingStartedAt: at(13))
        let next = event(start: 120, end: 150, url: zoomB)
        XCTAssertFalse(split(manual, next, now: 119))
        XCTAssertTrue(split(manual, next, now: 120))
        XCTAssertTrue(split(manual, event(start: 120, end: 150, response: nil, organizer: true), now: 120), "organizer")
        XCTAssertTrue(split(manual, event(start: 120, end: 150, response: nil, attendees: []), now: 120), "his own event")
        XCTAssertFalse(split(manual, event(start: 120, end: 150, response: .declined), now: 120))
        XCTAssertFalse(split(manual, event(start: 120, end: 150, response: .tentative), now: 120))
    }

    /// Started a few minutes early for that same meeting: not a split.
    func testAManualRecordingStartedJustBeforeTheEventIsThatEvent() {
        let early = MeetingSplit.Current(calendarEvent: nil, link: nil, meetingStartedAt: at(115))
        XCTAssertFalse(split(early, event(start: 120, end: 150), now: 121))
    }

    func testADetectedCallSplitsWhenItsLinkChanges() {
        let detected = MeetingSplit.Current(calendarEvent: nil, link: link(meetB), meetingStartedAt: at(115))
        let next = event(start: 120, end: 150, url: "https://meet.google.com/ddd-dddd-ddd")
        XCTAssertFalse(split(detected, next, now: 121, live: meetB))
        XCTAssertTrue(split(detected, next, now: 121, live: "https://meet.google.com/ddd-dddd-ddd"))
    }

    func testDetectedMeetingsAreNeverTheNextMeeting() {
        let manual = MeetingSplit.Current(calendarEvent: nil, link: nil, meetingStartedAt: at(0))
        let detected = QualifyingMeeting(
            id: "detected-us.zoom.xos-1", title: "Zoom", startDate: at(60), endDate: at(90), attendees: [],
            conferenceLink: nil, source: .detected, appBundleID: "us.zoom.xos", organizerEmail: nil,
            organizerName: nil, selfIsOrganizer: false, selfResponse: nil
        )
        XCTAssertFalse(split(manual, detected, now: 61))
    }

    /// No RSVP on an event with other attendees isn't acceptance: like needs-action, it
    /// splits only when he is in its call.
    func testNoRSVPWithOtherAttendeesSplitsOnlyInItsCall() {
        let manual = MeetingSplit.Current(calendarEvent: nil, link: nil, meetingStartedAt: at(13))
        let invite = event(start: 120, end: 150, url: meetB, response: nil)
        XCTAssertFalse(split(manual, invite, now: 120))
        XCTAssertTrue(split(manual, invite, now: 120, live: meetB))
        let calendar = calendarRecording(end: 120, url: zoomA)
        XCTAssertFalse(split(calendar, invite, now: 120), "rule 1 alone isn't enough without acceptance")
        XCTAssertTrue(split(calendar, invite, now: 120, live: meetB))
    }

    /// A continued meeting counts from its original start, not from the part that began when
    /// it continued (here 2 minutes before the event, which alone would look like an early start).
    func testAContinuedRecordingCountsFromItsOriginalStart() {
        let continued = MeetingSplit.Current(calendarEvent: nil, link: nil, meetingStartedAt: at(13))
        XCTAssertTrue(split(continued, event(start: 120, end: 150), now: 121))
        // And an event between the original start and the rejoin is still "next".
        XCTAssertTrue(split(continued, event(start: 100, end: 150), now: 121))
    }

    // MARK: Held prompt

    func testHeldPromptWaitsThenSplits() {
        let current = calendarRecording(end: 30, url: zoomA)
        let next = event(start: 30, end: 60)
        XCTAssertEqual(MeetingSplit.holdAction(recording: true, current: current, next: next, liveCallURL: { nil }, now: at(29)), .wait)
        XCTAssertEqual(MeetingSplit.holdAction(recording: true, current: current, next: next, liveCallURL: { nil }, now: at(30)), .split)
    }

    func testHeldPromptGoesBackToTheNormalPromptOnceTheRecordingEnds() {
        XCTAssertEqual(MeetingSplit.holdAction(recording: false, current: nil, next: event(start: 30, end: 60), liveCallURL: { nil }, now: at(29)), .prompt)
    }

    func testHeldPromptWaitsWhileTheRecordingIsStillStarting() {
        XCTAssertEqual(MeetingSplit.holdAction(recording: true, current: nil, next: event(start: 30, end: 60), liveCallURL: { nil }, now: at(31)), .wait)
    }

    func testHeldPromptIsDroppedWhenTheNextMeetingEndsWithoutASplit() {
        let current = calendarRecording(end: 120, url: zoomA)
        XCTAssertEqual(MeetingSplit.holdAction(recording: true, current: current, next: event(start: 30, end: 60), liveCallURL: { nil }, now: at(60)), .drop)
    }

    /// The held prompt is re-read from the calendar each tick: gone or now declined drops it.
    func testHeldPromptIsDroppedWhenTheEventIsGoneOrDeclined() {
        let current = calendarRecording(end: 30, url: zoomA)
        XCTAssertEqual(MeetingSplit.holdAction(recording: true, current: current, next: nil, liveCallURL: { nil }, now: at(30)), .drop)
        XCTAssertEqual(MeetingSplit.holdAction(recording: true, current: current, next: event(start: 30, end: 60, response: .declined),
                                               liveCallURL: { nil }, now: at(20)), .drop)
        XCTAssertEqual(MeetingSplit.holdAction(recording: false, current: nil, next: nil, liveCallURL: { nil }, now: at(20)), .drop)
    }

    /// The live URL is read only once the next meeting has started.
    func testLiveURLIsNotReadBeforeTheNextMeetingStarts() {
        var reads = 0
        _ = MeetingSplit.shouldSplit(current: calendarRecording(end: 60), next: event(start: 30, end: 60),
                                     liveCallURL: { reads += 1; return nil }, now: at(29))
        XCTAssertEqual(reads, 0)
    }

    // MARK: Keep together

    /// Split: A completed, B recording. Keep together: B's recording becomes A's part 2 and
    /// B is deleted, leaving one meeting.
    func testKeepTogetherLeavesOneMeeting() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("t.sqlite"), modelsDir: nil))
        let a = try XCTUnwrap(store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Working Session",
            attendees: [], trigger: .calendar, audioPath: nil, status: .recording
        )))
        XCTAssertEqual(store.beginMeetingPart(meetingId: a, startUnixMs: 1_000, continueKey: "cal:A"), 1)
        MeetingContinuation.completeRecording(store: store, meetingId: a, partIndex: 1, continued: false,
                                              endUnixMs: 60_000, audioPath: "/rec/a.m4a")
        let b = try XCTUnwrap(store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 60_000, endUnixMs: nil, title: "AVA Check In",
            attendees: [], trigger: .calendar, audioPath: nil, status: .recording
        )))
        XCTAssertEqual(store.beginMeetingPart(meetingId: b, startUnixMs: 60_500, continueKey: "cal:B"), 1)

        let joined = try XCTUnwrap(MeetingSplit.rejoin(store: store, meetingId: a, partStartUnixMs: 60_500))
        XCTAssertEqual(joined.partIndex, 2)
        XCTAssertEqual(joined.meeting.title, "Working Session")
        XCTAssertEqual(store.getMeeting(id: a)?.status, .recording)
        XCTAssertEqual(MeetingSplit.absorb(store: store, into: a, from: b, makeFolder: { _, _ in
            XCTFail("A has a folder"); return dir
        }), true)
        XCTAssertTrue(MeetingSplit.discard(store: store, meetingId: b, recordingsDir: dir))
        XCTAssertNil(store.getMeeting(id: b))

        // The recording then ends as A's part 2.
        MeetingContinuation.completeRecording(store: store, meetingId: a, partIndex: 2, continued: true,
                                              endUnixMs: 120_000, audioPath: "/rec/b.m4a")
        let parts = try XCTUnwrap(store.meetingParts(meetingId: a))
        XCTAssertEqual(parts.map(\.partIndex), [1, 2])
        XCTAssertEqual(parts.map(\.audioPath), ["/rec/a.m4a", "/rec/b.m4a"])
        XCTAssertEqual(parts.last?.startUnixMs, 60_500)
        XCTAssertEqual(store.getMeeting(id: a)?.status, .completed)
        // A rejoin of B's call (key cal:B) continues the joined meeting.
        XCTAssertEqual(store.findContinuableMeeting(continueKey: "cal:B", nowUnixMs: 121_000), a)
    }

    /// Keep together never deletes B's slides: their rows and files move into A's folder.
    func testKeepTogetherMovesTheSplitOffSlidesIntoTheEarlierMeeting() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        let folderA = dir.appendingPathComponent("A", isDirectory: true)
        let folderB = dir.appendingPathComponent("B", isDirectory: true)
        for f in [folderA, folderB] { try FileManager.default.createDirectory(at: f, withIntermediateDirectories: true) }
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("t.sqlite"), modelsDir: nil))
        func meeting(_ title: String) throws -> Int64 {
            try XCTUnwrap(store.insertMeeting(CoreNewMeeting(
                deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: title,
                attendees: [], trigger: .calendar, audioPath: nil, status: .recording
            )))
        }
        let a = try meeting("A")
        let b = try meeting("B")
        store.setMeetingSnapshotsDir(meetingId: a, dir: folderA.path)
        store.setMeetingSnapshotsDir(meetingId: b, dir: folderB.path)
        let slide = folderB.appendingPathComponent("slide-1.png")
        try Data("png".utf8).write(to: slide)
        XCTAssertNotNil(store.insertMeetingSnapshot(meetingId: b, capturedUnixMs: 70_000, filePath: slide.path, width: 1, height: 1, ocrText: "Q3"))

        XCTAssertEqual(MeetingSplit.absorb(store: store, into: a, from: b, makeFolder: { _, _ in folderA }), true)
        XCTAssertTrue(MeetingSplit.discard(store: store, meetingId: b, recordingsDir: dir))

        let moved = folderA.appendingPathComponent("slide-1.png")
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: a).map(\.filePath), [moved.path])
        XCTAssertTrue(FileManager.default.fileExists(atPath: moved.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: slide.path))
    }

    /// A failed file move leaves the slides where they are, still owned (by A), and B's folder
    /// survives B's delete.
    func testKeepTogetherKeepsTheSlidesInPlaceWhenTheMoveFails() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        let folderB = dir.appendingPathComponent("B", isDirectory: true)
        try FileManager.default.createDirectory(at: folderB, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("t.sqlite"), modelsDir: nil))
        let a = try XCTUnwrap(store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "A",
            attendees: [], trigger: .calendar, audioPath: nil, status: .recording
        )))
        let b = try XCTUnwrap(store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "B",
            attendees: [], trigger: .calendar, audioPath: nil, status: .recording
        )))
        store.setMeetingSnapshotsDir(meetingId: b, dir: folderB.path)
        let slide = folderB.appendingPathComponent("slide-1.png")
        try Data("png".utf8).write(to: slide)
        _ = store.insertMeetingSnapshot(meetingId: b, capturedUnixMs: 70_000, filePath: slide.path, width: 1, height: 1, ocrText: "")

        XCTAssertEqual(MeetingSplit.absorb(store: store, into: a, from: b, makeFolder: { _, _ in throw CocoaError(.fileNoSuchFile) }), false)
        XCTAssertTrue(MeetingSplit.discard(store: store, meetingId: b, recordingsDir: dir))

        XCTAssertEqual(store.listMeetingSnapshots(meetingId: a).map(\.filePath), [slide.path])
        XCTAssertTrue(FileManager.default.fileExists(atPath: slide.path))
    }

    /// Rejoin is undone when its part can't be written, so the earlier meeting stays completed.
    func testRejoinOfAMissingMeetingChangesNothing() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("t.sqlite"), modelsDir: nil))
        XCTAssertNil(MeetingSplit.rejoin(store: store, meetingId: 999, partStartUnixMs: 1))
    }

    func testSplitIsNotACallGoingAway() {
        XCTAssertFalse(MeetingEndDetector.EndReason.split.callWentAway)
    }
}
