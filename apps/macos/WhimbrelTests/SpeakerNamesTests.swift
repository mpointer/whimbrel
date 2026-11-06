import XCTest
@testable import Whimbrel

/// Speaker names on the Mac: invitee emails from the calendar, and naming a recording's
/// speakers after its transcript is stored and before it is summarized.
final class SpeakerNamesTests: XCTestCase {
    private var tempDir: URL!
    private var store: CoreMemoryStore!

    override func setUp() {
        super.setUp()
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try! FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    override func tearDown() {
        store = nil
        try? FileManager.default.removeItem(at: tempDir)
        tempDir = nil
        super.tearDown()
    }

    func testAttendeeStripsMailto() {
        let a = CalendarService.attendee(name: "Ana Diaz", url: URL(string: "MAILTO:ana@x.com")!)
        XCTAssertEqual(a, CoreAttendee(name: "Ana Diaz", email: "ana@x.com"))
    }

    func testAttendeeFallsBackToEmailForName() {
        XCTAssertEqual(CalendarService.attendee(name: nil, url: URL(string: "mailto:bo@x.com")!),
                       CoreAttendee(name: "bo@x.com", email: "bo@x.com"))
        // Not an address: no email, and the URL stands in for the name.
        XCTAssertEqual(CalendarService.attendee(name: nil, url: URL(string: "https://x.com/cy")!),
                       CoreAttendee(name: "https://x.com/cy", email: nil))
    }

    private func meeting(attendees: [String]) -> Int64 {
        store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Sync",
            attendees: attendees, trigger: .calendar, audioPath: nil, status: .recording
        ))!
    }

    private let routing = CoreRoutingConfig(
        anthropicKey: nil, openaiKey: nil, googleKey: nil, typesafeKey: nil,
        userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: false,
        preferredProvider: "ollama", preferredModel: "no-such-model",
        customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: ""
    )

    /// One invitee and one voice: named by elimination, no model call. Exercises the JSON both ways.
    func testAssignNamesTheOnlyInvitee() throws {
        let id = meeting(attendees: ["Ana"])
        XCTAssertTrue(store.setMeetingAttendeeDetails(meetingId: id, attendees: [CoreAttendee(name: "Ana", email: "ana@x.com")]))
        let voices = [CoreVoice(speakerId: 0, embedding: [1, 0], speechCs: 500)]
        let named = try XCTUnwrap(store.assignMeetingSpeakers(meetingId: id, voices: voices, model: "no-such-model", routing: routing, remember: true))
        XCTAssertEqual(named, [CoreMeetingSpeaker(speakerId: 0, personKey: "ana@x.com", name: "Ana", status: "guess")])
        XCTAssertEqual(store.meetingSpeakers(meetingId: id), named)

        let confirmed = try XCTUnwrap(store.setMeetingSpeaker(meetingId: id, speakerId: 0, name: "Ana", email: nil))
        XCTAssertEqual(confirmed.first?.status, "confirmed")
        XCTAssertEqual(store.voicePeople()?.map(\.personKey), ["ana@x.com"])
        XCTAssertEqual(store.forgetVoice(personKey: "ana@x.com"), [])
    }

    private func touch(_ name: String) -> String {
        let url = tempDir.appendingPathComponent(name)
        try! Data(name.utf8).write(to: url)
        return url.path
    }

    /// Records "name <id> <speaker ids> stored=<segments in>" and "summarize" in order.
    private func queue(_ events: @escaping (String) -> Void) -> MeetingTranscriptionQueue {
        MeetingTranscriptionQueue(
            store: store,
            transcribe: { [tempDir] path, _, language in
                let name = URL(fileURLWithPath: path).deletingPathExtension().lastPathComponent
                let url = tempDir!.appendingPathComponent("transcript-\(name).txt")
                let voices = [CoreVoice(speakerId: 0, embedding: [1, 0], speechCs: 100)]
                return (url, [TranscriptionService.Line(speakerLabel: "Speaker 1", startCs: 100, text: "said in \(name)")], language, voices)
            },
            summarize: { _, _, transcript in events("summarize \(transcript.contains("said in"))") },
            nameSpeakers: { [store] meetingId, voices in
                let text = CoreMemoryStore.accessQueue.sync { store!.meetingTranscriptText(meetingId: meetingId) } ?? ""
                events("name \(meetingId) \(voices.map(\.speakerId)) stored=\(text.contains("said in"))")
            },
            onStage: { _, _ in },
            multilingual: { false },
            modelReady: { true }
        )
    }

    /// A single-part meeting is named once, after its segments are stored and before the summary.
    func testNamesSpeakersAfterTheSegmentsBeforeTheSummary() throws {
        let id = meeting(attendees: ["Ana", "Bo"])
        let a = touch("system-audio-A.caf")
        XCTAssertEqual(try XCTUnwrap(store.beginMeetingPart(meetingId: id, startUnixMs: 1_000, continueKey: "cal:1")), 1)
        _ = store.completeMeetingPart(meetingId: id, partIndex: 1, endUnixMs: 30_000, audioPath: a)
        _ = store.completeMeeting(id: id, endUnixMs: 30_000, audioPath: a)

        let lock = NSLock()
        var events: [String] = []
        let q = queue { e in lock.withLock { events.append(e) } }
        q.enqueue(.init(meetingId: id, partIndex: 1, audioPath: a, notes: ""))
        q.waitUntilIdle()
        XCTAssertEqual(lock.withLock { events }, ["name \(id) [0] stored=true", "summarize true"])
    }

    /// Diarization restarts per part, so a continued meeting is never named, and is still summarized.
    func testTwoPartMeetingIsNotNamedButIsSummarized() throws {
        // Built as MeetingPartTranscriptionTests builds one.
        let id = meeting(attendees: ["Ana", "Bo"])
        let a = touch("system-audio-A.caf"), b = touch("system-audio-B.caf")
        XCTAssertEqual(try XCTUnwrap(store.beginMeetingPart(meetingId: id, startUnixMs: 1_000, continueKey: "cal:1")), 1)
        _ = store.completeMeetingPart(meetingId: id, partIndex: 1, endUnixMs: 30_000, audioPath: a)
        _ = store.completeMeeting(id: id, endUnixMs: 30_000, audioPath: a)
        _ = store.reopenMeeting(id: id)
        XCTAssertEqual(try XCTUnwrap(store.beginMeetingPart(meetingId: id, startUnixMs: 61_000, continueKey: nil)), 2)
        _ = store.completeMeetingPart(meetingId: id, partIndex: 2, endUnixMs: 90_000, audioPath: b)
        _ = store.completeMeeting(id: id, endUnixMs: 90_000, audioPath: b)

        let lock = NSLock()
        var events: [String] = []
        let q = queue { e in lock.withLock { events.append(e) } }
        q.enqueue(.init(meetingId: id, partIndex: 1, audioPath: a, notes: ""))
        q.enqueue(.init(meetingId: id, partIndex: 2, audioPath: b, notes: ""))
        q.waitUntilIdle()
        XCTAssertEqual(lock.withLock { events }, ["summarize true"])
    }

    /// "Still in a meeting?" stopped it: once the full transcript is stored, the meeting is
    /// trimmed to the last line with text plus 30 s, before naming and the summary. A job
    /// that wasn't marked never trims.
    func testAutoTrimRunsAfterTheTranscriptBeforeNamingAndSummary() throws {
        let id = meeting(attendees: ["Ana"])
        let a = touch("system-audio-A.caf")
        XCTAssertEqual(try XCTUnwrap(store.beginMeetingPart(meetingId: id, startUnixMs: 1_000, continueKey: nil)), 1)
        _ = store.completeMeetingPart(meetingId: id, partIndex: 1, endUnixMs: 300_000, audioPath: a)
        _ = store.completeMeeting(id: id, endUnixMs: 300_000, audioPath: a)

        let lock = NSLock()
        var events: [String] = []
        let record = { (e: String) in lock.withLock { events.append(e) } }
        let q = MeetingTranscriptionQueue(
            store: store,
            transcribe: { [tempDir] _, _, language in
                // Speech 1-5 s, then a blank line at 20 s: the keep is 5 s + 30 s.
                let lines = [TranscriptionService.Line(speakerLabel: "Speaker 1", startCs: 100, endCs: 500, text: "hello"),
                             TranscriptionService.Line(speakerLabel: "Speaker 1", startCs: 2_000, endCs: 2_100, text: " ")]
                return (tempDir!.appendingPathComponent("transcript-A.txt"), lines, language,
                        [CoreVoice(speakerId: 0, embedding: [1, 0], speechCs: 100)])
            },
            summarize: { _, _, _ in record("summarize") },
            nameSpeakers: { _, _ in record("name") },
            trim: { [store] meetingId, keepMs in
                let stored = CoreMemoryStore.accessQueue.sync { store!.meetingTranscriptText(meetingId: meetingId) } ?? ""
                record("trim \(keepMs) stored=\(stored.contains("hello"))")
            },
            onStage: { _, _ in },
            multilingual: { false },
            modelReady: { true }
        )
        q.enqueue(.init(meetingId: id, partIndex: 1, audioPath: a, notes: "", autoTrim: true))
        q.waitUntilIdle()
        XCTAssertEqual(lock.withLock { events }, ["trim 35000 stored=true", "name", "summarize"])

        lock.withLock { events = [] }
        q.enqueue(.init(meetingId: id, partIndex: 1, audioPath: a, notes: ""))
        q.waitUntilIdle()
        XCTAssertEqual(lock.withLock { events }, ["name", "summarize"])
    }

    func testRememberVoicesDefaultsOn() {
        let defaults = UserDefaults(suiteName: UUID().uuidString)!
        XCTAssertTrue(SpeakerNamesSettings(defaults: defaults).rememberVoices)
        SpeakerNamesSettings(defaults: defaults).rememberVoices = false
        XCTAssertFalse(SpeakerNamesSettings(defaults: defaults).rememberVoices)
    }
}
