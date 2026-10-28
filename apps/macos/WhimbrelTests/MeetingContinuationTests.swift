import XCTest
@testable import Whimbrel

/// Meeting continue: the continue key, the lookup that feeds a continued start, and the
/// detector re-arming a bundle after its meeting ends.
@MainActor
final class MeetingContinuationTests: XCTestCase {
    private func meeting(id: String = "x", source: MeetingSource, bundle: String? = nil, url: String? = nil) -> QualifyingMeeting {
        QualifyingMeeting(
            id: id, title: "Standup", startDate: Date(timeIntervalSince1970: 0), endDate: Date(timeIntervalSince1970: 3600),
            attendees: [], conferenceLink: url.map { DetectedConferenceLink(provider: .googleMeet, url: URL(string: $0)!) },
            source: source, appBundleID: bundle, organizerEmail: nil, organizerName: nil, selfIsOrganizer: false, selfResponse: nil
        )
    }

    // MARK: Key

    func testCalendarKeyUsesEventId() {
        XCTAssertEqual(MeetingContinuation.key(for: meeting(id: "EVT-1", source: .calendar)), "cal:EVT-1")
        XCTAssertEqual(MeetingContinuation.key(for: meeting(id: "EVT-2", source: .googleCalendar)), "cal:EVT-2")
    }

    func testDetectedKeyWithURL() {
        let m = meeting(id: "detected-com.apple.Safari-1", source: .detected, bundle: "com.apple.Safari",
                        url: "https://meet.google.com/abc-defg-hij")
        XCTAssertEqual(MeetingContinuation.key(for: m), "app:com.apple.Safari|https://meet.google.com/abc-defg-hij")
    }

    func testDetectedKeyWithoutURL() {
        let m = meeting(id: "detected-us.zoom.xos-1", source: .detected, bundle: "us.zoom.xos")
        XCTAssertEqual(MeetingContinuation.key(for: m), "app:us.zoom.xos|")
    }

    /// The detected id has a timestamp in it, so two detections of one meeting share a key.
    func testDetectedKeyIgnoresTheUnstableId() {
        let a = meeting(id: "detected-us.zoom.xos-1", source: .detected, bundle: "us.zoom.xos")
        let b = meeting(id: "detected-us.zoom.xos-999", source: .detected, bundle: "us.zoom.xos")
        XCTAssertEqual(MeetingContinuation.key(for: a), MeetingContinuation.key(for: b))
    }

    func testURLQueryAndFragmentAreStripped() {
        XCTAssertEqual(MeetingContinuation.normalizedURL(URL(string: "https://meet.google.com/abc-defg-hij?authuser=0&hs=1#frag")!),
                       "https://meet.google.com/abc-defg-hij")
        let m = meeting(source: .detected, bundle: "com.apple.Safari", url: "https://app.zoom.us/wc/123/join?pwd=s3cret#x")
        XCTAssertEqual(MeetingContinuation.key(for: m), "app:com.apple.Safari|https://app.zoom.us/wc/123/join")
    }

    func testURLSchemeAndHostAreLowercasedAndTrailingSlashTrimmed() {
        XCTAssertEqual(MeetingContinuation.normalizedURL(URL(string: "HTTPS://Meet.Google.com/abc-defg-hij/")!),
                       "https://meet.google.com/abc-defg-hij")
        XCTAssertEqual(MeetingContinuation.normalizedURL(URL(string: "https://teams.microsoft.com/")!),
                       "https://teams.microsoft.com")
    }

    // MARK: Calendar fallback

    private let ended = Date(timeIntervalSince1970: 100_000)
    private func lastCalendar(provider: MeetingProvider = .zoom, url: String? = "https://us02web.zoom.us/j/123",
                              eventEnd: TimeInterval = 3600) -> MeetingContinuation.EndedCalendarMeeting {
        MeetingContinuation.EndedCalendarMeeting(
            meetingId: 7,
            event: MeetingContinuation.CalendarEvent(
                id: "EVT", conferenceLink: url.map { DetectedConferenceLink(provider: provider, url: URL(string: $0)!) },
                scheduledEnd: ended.addingTimeInterval(eventEnd)),
            endedAt: ended)
    }
    private func detected(bundle: String, provider: MeetingProvider? = nil, url: String? = nil) -> QualifyingMeeting {
        QualifyingMeeting(
            id: "detected-\(bundle)-1", title: "Zoom", startDate: ended, endDate: ended.addingTimeInterval(3600), attendees: [],
            conferenceLink: url.map { DetectedConferenceLink(provider: provider ?? .googleMeet, url: URL(string: $0)!) },
            source: .detected, appBundleID: bundle, organizerEmail: nil, organizerName: nil, selfIsOrganizer: false, selfResponse: nil)
    }
    private func matches(_ last: MeetingContinuation.EndedCalendarMeeting, _ d: QualifyingMeeting, after: TimeInterval = 120) -> Bool {
        MeetingContinuation.calendarFallbackMatches(lastEnded: last, detected: d, now: ended.addingTimeInterval(after))
    }

    func testFallbackSameProviderNativeAppMatches() {
        XCTAssertTrue(matches(lastCalendar(), detected(bundle: "us.zoom.xos")))
        XCTAssertTrue(matches(lastCalendar(provider: .teams, url: "https://teams.microsoft.com/l/meetup-join/x"),
                              detected(bundle: "com.microsoft.teams2")))
    }

    func testFallbackDifferentProviderDoesNotMatch() {
        XCTAssertFalse(matches(lastCalendar(), detected(bundle: "com.microsoft.teams2")))
        XCTAssertFalse(matches(lastCalendar(), detected(bundle: "com.tinyspeck.slackmacgap")))
        XCTAssertFalse(matches(lastCalendar(url: nil), detected(bundle: "us.zoom.xos")), "event with no link")
    }

    func testFallbackBrowserMeetMatchesOnlyTheSameURL() {
        let last = lastCalendar(provider: .googleMeet, url: "https://meet.google.com/abc-defg-hij")
        XCTAssertTrue(matches(last, detected(bundle: "com.apple.Safari", provider: .googleMeet,
                                             url: "https://meet.google.com/abc-defg-hij?authuser=0")))
        XCTAssertFalse(matches(last, detected(bundle: "com.apple.Safari", provider: .googleMeet,
                                              url: "https://meet.google.com/xyz-xyzx-xyz")))
    }

    func testFallbackAfterEventEndDoesNotMatch() {
        XCTAssertFalse(matches(lastCalendar(eventEnd: 60), detected(bundle: "us.zoom.xos"), after: 120))
    }

    func testFallbackAfterTenMinutesDoesNotMatch() {
        XCTAssertTrue(matches(lastCalendar(), detected(bundle: "us.zoom.xos"), after: 600))
        XCTAssertFalse(matches(lastCalendar(), detected(bundle: "us.zoom.xos"), after: 601))
    }

    func testFallbackNeverAppliesToACalendarStart() {
        let cal = QualifyingMeeting(id: "OTHER", title: "x", startDate: ended, endDate: ended.addingTimeInterval(3600),
                                    attendees: [], conferenceLink: nil, source: .calendar, appBundleID: "us.zoom.xos", organizerEmail: nil, organizerName: nil, selfIsOrganizer: false, selfResponse: nil)
        XCTAssertFalse(matches(lastCalendar(), cal))
    }

    /// A start with no calendar event and no app to tie it to (what a manual start is) has no
    /// key, so it never continues. `startManualMeeting` itself never asks for one.
    func testNoCalendarEventAndNoAppGivesNoKey() {
        XCTAssertNil(MeetingContinuation.key(for: meeting(source: .detected, bundle: nil)))
        XCTAssertNil(MeetingContinuation.key(for: meeting(source: .detected, bundle: "")))
    }

    // MARK: Notes

    func testNotesPreferWhatWasTypedInThisRunForTheSameMeeting() {
        XCTAssertEqual(MeetingContinuation.notes(stored: "old", lastEnded: (id: 7, notes: "typed"), meetingId: 7), "typed")
        XCTAssertEqual(MeetingContinuation.notes(stored: "old", lastEnded: (id: 8, notes: "typed"), meetingId: 7), "old")
        XCTAssertEqual(MeetingContinuation.notes(stored: nil, lastEnded: nil, meetingId: 7), "")
    }

    // MARK: Lookup

    func testFindPreviousReturnsTheStoredMeetingAndItsSnapshotsFolder() throws {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let store = try XCTUnwrap(CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil))
        let id = try XCTUnwrap(store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Planning",
            attendees: ["Ana"], trigger: .calendar, audioPath: nil, status: .recording
        )))
        XCTAssertEqual(store.beginMeetingPart(meetingId: id, startUnixMs: 1_000, continueKey: "cal:E"), 1)
        store.completeMeetingPart(meetingId: id, partIndex: 1, endUnixMs: 60_000, audioPath: nil)
        store.completeMeeting(id: id, endUnixMs: 60_000, audioPath: nil)
        store.setMeetingSnapshotsDir(meetingId: id, dir: "/snaps/planning")

        let previous = try XCTUnwrap(MeetingContinuation.findPrevious(key: "cal:E", nowUnixMs: 60_000 + 9 * 60_000, store: store))
        XCTAssertEqual(previous.meeting.id, id)
        XCTAssertEqual(previous.meeting.title, "Planning")
        XCTAssertEqual(previous.snapshotsDir, "/snaps/planning")

        XCTAssertNil(MeetingContinuation.findPrevious(key: "cal:E", nowUnixMs: 60_000 + 11 * 60_000, store: store))
        XCTAssertNil(MeetingContinuation.findPrevious(key: "cal:other", nowUnixMs: 60_001, store: store))
        XCTAssertNil(MeetingContinuation.findPrevious(key: nil, nowUnixMs: 60_001, store: store))
    }

    // MARK: Window

    func testNativeAppKeyWithoutURLGetsThreeMinutes() {
        XCTAssertEqual(MeetingContinuation.window(forKey: "app:us.zoom.xos|"), 180_000)
        XCTAssertEqual(MeetingContinuation.window(forKey: "app:com.apple.Safari|https://meet.google.com/abc"), 600_000)
        XCTAssertEqual(MeetingContinuation.window(forKey: "cal:EVT-1"), 600_000)
    }

    private func endedMeeting(key: String, endUnixMs: Int64) throws -> (CoreMemoryStore, Int64, URL) {
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        let store = try XCTUnwrap(CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil))
        let id = try XCTUnwrap(store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Call",
            attendees: [], trigger: .detected, audioPath: nil, status: .recording
        )))
        _ = store.beginMeetingPart(meetingId: id, startUnixMs: 1_000, continueKey: key)
        store.completeMeeting(id: id, endUnixMs: endUnixMs, audioPath: nil)
        return (store, id, tempDir)
    }

    func testNativeKeyContinuesWithinThreeMinutesOnly() throws {
        let (store, id, dir) = try endedMeeting(key: "app:us.zoom.xos|", endUnixMs: 60_000)
        defer { try? FileManager.default.removeItem(at: dir) }
        XCTAssertEqual(MeetingContinuation.findPrevious(key: "app:us.zoom.xos|", nowUnixMs: 60_000 + 2 * 60_000, store: store)?.meeting.id, id)
        XCTAssertNil(MeetingContinuation.findPrevious(key: "app:us.zoom.xos|", nowUnixMs: 60_000 + 4 * 60_000, store: store))
    }

    func testURLKeyKeepsTenMinutes() throws {
        let key = "app:com.apple.Safari|https://meet.google.com/abc"
        let (store, id, dir) = try endedMeeting(key: key, endUnixMs: 60_000)
        defer { try? FileManager.default.removeItem(at: dir) }
        XCTAssertEqual(MeetingContinuation.findPrevious(key: key, nowUnixMs: 60_000 + 9 * 60_000, store: store)?.meeting.id, id)
    }

    // MARK: Detector

    /// After a meeting ends its bundle can be detected again (a rejoin), but only once the call's
    /// audio has dropped, so pressing End while still in the call doesn't prompt again at once.
    func testDetectorRedetectsTheSameBundleAfterMeetingEnded() {
        let detector = MeetingHeuristicDetector()
        var audioActive = true
        detector.isBidirectionalAudioActive = { audioActive }
        func match() { detector.handleMatch(bundleID: "us.zoom.xos", title: "Zoom", browserWindow: nil) }

        match(); match()
        XCTAssertNotNil(detector.detectedMeeting)
        detector.clearDetection()
        match(); match()
        XCTAssertNil(detector.detectedMeeting, "deduped while the same meeting goes on")

        detector.meetingEnded(bundleID: "us.zoom.xos")
        match(); match()
        XCTAssertNil(detector.detectedMeeting, "still in the call right after End")

        audioActive = false
        match()
        audioActive = true
        match(); match()
        XCTAssertEqual(detector.detectedMeeting?.appBundleID, "us.zoom.xos")
    }

    // MARK: Part ending

    func testPartEndingTranscribesTheKnownPart() {
        XCTAssertEqual(MeetingContinuation.partEnding(partIndex: 2, continued: true, hasAudio: true), .transcribe(partIndex: 2))
        XCTAssertEqual(MeetingContinuation.partEnding(partIndex: 2, continued: true, hasAudio: false), .transcribe(partIndex: 2),
                       "a later part without audio still releases the summary")
        XCTAssertEqual(MeetingContinuation.partEnding(partIndex: 1, continued: false, hasAudio: false), .nothing)
    }

    func testPartEndingDefaultsToPart1OnlyForANewMeeting() {
        XCTAssertEqual(MeetingContinuation.partEnding(partIndex: nil, continued: false, hasAudio: true), .transcribe(partIndex: 1))
        XCTAssertEqual(MeetingContinuation.partEnding(partIndex: nil, continued: false, hasAudio: false), .nothing)
    }

    /// Transcribing it as part 1 would overwrite part 1's segments.
    func testPartEndingNeverGuessesPart1ForAContinuedMeeting() {
        XCTAssertEqual(MeetingContinuation.partEnding(partIndex: nil, continued: true, hasAudio: true), .unknownContinuedPart)
        XCTAssertEqual(MeetingContinuation.partEnding(partIndex: nil, continued: true, hasAudio: false), .unknownContinuedPart)
    }
}
