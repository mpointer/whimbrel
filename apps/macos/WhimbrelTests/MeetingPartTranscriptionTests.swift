import Combine
import XCTest
@testable import Whimbrel

/// `MeetingTranscriptionQueue`: a continued meeting's parts are transcribed one at a time,
/// in order, and summarized once from the whole-meeting transcript. A fake transcriber
/// stands in for whisper.
final class MeetingPartTranscriptionTests: XCTestCase {
    private var tempDir: URL!
    private var store: CoreMemoryStore!
    private let lock = NSLock()
    private var transcribeCalls: [String] = []
    private var summaries: [(id: Int64, notes: String, transcript: String)] = []
    /// Audio paths whose transcription waits on `gate`.
    private var gated: Set<String> = []
    private let gate = DispatchSemaphore(value: 0)
    private var failing: Set<String> = []
    /// Audio paths whose transcription succeeds with no speech.
    private var silent: Set<String> = []
    /// The fake hears only the mic ("Me").
    private var meOnly = false

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

    private func makeQueue() -> MeetingTranscriptionQueue {
        MeetingTranscriptionQueue(
            store: store,
            transcribe: { [unowned self] audioPath, _ in
                lock.withLock { transcribeCalls.append(audioPath) }
                if lock.withLock({ gated.contains(audioPath) }) { gate.wait() }
                if lock.withLock({ failing.contains(audioPath) }) { throw CocoaError(.fileReadUnknown) }
                let name = URL(fileURLWithPath: audioPath).deletingPathExtension().lastPathComponent
                let url = tempDir.appendingPathComponent("transcript-\(name).txt")
                try "x".write(to: url, atomically: true, encoding: .utf8)
                if lock.withLock({ silent.contains(audioPath) }) { return (url, []) }
                let me = TranscriptionService.Speaker.me.rawValue
                return (url, [
                    TranscriptionService.Line(speakerLabel: lock.withLock { meOnly } ? me : "Them", startCs: 100, text: "said in \(name)"),
                    TranscriptionService.Line(speakerLabel: me, startCs: 200, text: "replied in \(name)")
                ])
            },
            summarize: { [unowned self] id, notes, transcript in
                lock.withLock { summaries.append((id, notes, transcript)) }
            },
            onStage: { _, _ in }
        )
    }

    private func touch(_ name: String) -> URL {
        let url = tempDir.appendingPathComponent(name)
        try! Data(name.utf8).write(to: url)
        return url
    }

    /// A meeting whose part 1 (A) starts at 1_000 and part 2 (B) at 61_000. `completed`
    /// false leaves it recording part 2.
    private func makeTwoPartMeeting(completed: Bool = true, trigger: CoreMeetingTrigger = .manual) -> (id: Int64, a: String, b: String) {
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: trigger, audioPath: nil, status: .recording
        ))!
        let a = touch("system-audio-A.caf").path, b = touch("system-audio-B.caf").path
        _ = store.beginMeetingPart(meetingId: id, startUnixMs: 1_000, continueKey: "cal:1")
        _ = store.completeMeetingPart(meetingId: id, partIndex: 1, endUnixMs: 30_000, audioPath: a)
        _ = store.completeMeeting(id: id, endUnixMs: 30_000, audioPath: a)
        _ = store.reopenMeeting(id: id)
        _ = store.beginMeetingPart(meetingId: id, startUnixMs: 61_000, continueKey: nil)
        if completed {
            _ = store.completeMeetingPart(meetingId: id, partIndex: 2, endUnixMs: 90_000, audioPath: b)
            _ = store.completeMeeting(id: id, endUnixMs: 90_000, audioPath: b)
        }
        return (id, a, b)
    }

    private func count(_ needle: String, in text: String) -> Int {
        text.components(separatedBy: needle).count - 1
    }

    func test_part2_waits_for_part1() {
        let m = makeTwoPartMeeting()
        gated = [m.a]
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        Thread.sleep(forTimeInterval: 0.3)
        XCTAssertEqual(lock.withLock { transcribeCalls }, [m.a], "part 2 must not start while part 1 runs")

        gate.signal()
        queue.waitUntilIdle()
        XCTAssertEqual(lock.withLock { transcribeCalls }, [m.a, m.b])
        let text = store.meetingTranscriptText(meetingId: m.id) ?? ""
        let a = text.range(of: "said in system-audio-A"), divider = text.range(of: "rejoined"), b = text.range(of: "said in system-audio-B")
        XCTAssertNotNil(a, text); XCTAssertNotNil(divider, text); XCTAssertNotNil(b, text)
        if let a, let divider, let b {
            XCTAssertLessThan(a.lowerBound, divider.lowerBound)
            XCTAssertLessThan(divider.lowerBound, b.lowerBound)
        }
        XCTAssertEqual(store.meetingParts(meetingId: m.id)?.map(\.transcribed), [true, true])
    }

    func test_summary_runs_once_after_last_part_with_both_parts_text() {
        let m = makeTwoPartMeeting()
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: "early notes"))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: "early notes, later notes"))
        queue.waitUntilIdle()

        let got = lock.withLock { summaries }
        XCTAssertEqual(got.count, 1)
        XCTAssertEqual(got.first?.id, m.id)
        XCTAssertEqual(got.first?.notes, "early notes, later notes")
        XCTAssertTrue(got.first?.transcript.contains("said in system-audio-A") == true, got.first?.transcript ?? "")
        XCTAssertTrue(got.first?.transcript.contains("said in system-audio-B") == true, got.first?.transcript ?? "")
        // Part 1 keeps the legacy single transcript path; notes are stored.
        let meeting = store.getMeeting(id: m.id)
        XCTAssertEqual(meeting?.notes, "early notes, later notes")
        XCTAssertEqual(meeting?.transcriptPath.map { URL(fileURLWithPath: $0).lastPathComponent }, "transcript-system-audio-A.txt")
    }

    func test_part1_landing_while_part2_records_keeps_live_rows_and_skips_summary() {
        let m = makeTwoPartMeeting(completed: false)
        _ = store.insertLiveSegments(meetingId: m.id, channel: "system", chunkIndex: 0,
                                     segmentsJSON: #"[{"start_cs":0,"end_cs":100,"text":"live now"}]"#, speakerLabel: "Them")
        XCTAssertEqual(store.listLiveSegments(meetingId: m.id).count, 1)
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.waitUntilIdle()

        XCTAssertEqual(store.listLiveSegments(meetingId: m.id).count, 1, "part 2's live rows survive part 1 landing")
        XCTAssertEqual(lock.withLock { summaries.count }, 0, "no summary while a part records")
    }

    func test_transcribe_again_does_not_duplicate_segments() {
        let m = makeTwoPartMeeting()
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()

        queue.transcribeAgain(meetingId: m.id, notes: "")
        CoreMemoryStore.accessQueue.sync {}
        queue.waitUntilIdle()

        XCTAssertEqual(lock.withLock { transcribeCalls }, [m.a, m.b, m.a, m.b])
        let text = store.meetingTranscriptText(meetingId: m.id) ?? ""
        XCTAssertEqual(count("said in system-audio-A", in: text), 1, text)
        XCTAssertEqual(count("said in system-audio-B", in: text), 1, text)
        XCTAssertEqual(lock.withLock { summaries.count }, 2, "one summary per run")
    }

    func test_failed_transcription_keeps_the_parts_segments() {
        let m = makeTwoPartMeeting()
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()

        failing = [m.b]
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()
        let text = store.meetingTranscriptText(meetingId: m.id) ?? ""
        XCTAssertEqual(count("said in system-audio-B", in: text), 1, text)
        XCTAssertEqual(lock.withLock { summaries.count }, 2, "the rerun still summarizes what the meeting has")
    }

    func test_summary_runs_despite_a_failed_part() {
        let m = makeTwoPartMeeting()
        failing = [m.b]
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()

        let got = lock.withLock { summaries }
        XCTAssertEqual(got.count, 1)
        XCTAssertTrue(got.first?.transcript.contains("said in system-audio-A") == true, got.first?.transcript ?? "")
        XCTAssertEqual(store.meetingParts(meetingId: m.id)?.map(\.transcribed), [true, false])
    }

    func test_no_summary_when_no_part_has_segments() {
        let m = makeTwoPartMeeting()
        failing = [m.a, m.b]
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()
        XCTAssertEqual(lock.withLock { summaries.count }, 0)
    }

    func test_silent_meeting_with_notes_is_summarized() {
        let m = makeTwoPartMeeting()
        silent = [m.a, m.b]
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: "my notes"))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: "my notes"))
        queue.waitUntilIdle()
        let got = lock.withLock { summaries }
        XCTAssertEqual(got.count, 1)
        XCTAssertEqual(got.first?.notes, "my notes")
    }

    func test_silent_meeting_with_slides_is_summarized() {
        let m = makeTwoPartMeeting()
        silent = [m.a, m.b]
        let slide = touch("slide-1.png")
        XCTAssertNotNil(store.insertMeetingSnapshot(meetingId: m.id, capturedUnixMs: 2_000, filePath: slide.path, width: 1, height: 1, ocrText: "Q3 plan"))
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()
        XCTAssertEqual(lock.withLock { summaries.count }, 1)
    }

    func test_no_summary_when_every_part_failed_without_notes_or_slides() {
        let m = makeTwoPartMeeting()
        failing = [m.a, m.b]
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: "  "))
        queue.waitUntilIdle()
        XCTAssertEqual(lock.withLock { summaries.count }, 0)
    }

    func test_part2_without_audio_still_releases_the_summary() {
        let m = makeTwoPartMeeting(completed: false)
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.waitUntilIdle()
        XCTAssertEqual(lock.withLock { summaries.count }, 0, "part 2 still recording")

        _ = store.completeMeetingPart(meetingId: m.id, partIndex: 2, endUnixMs: 90_000, audioPath: nil)
        _ = store.completeMeeting(id: m.id, endUnixMs: 90_000, audioPath: nil)
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: nil, notes: "after"))
        queue.waitUntilIdle()
        XCTAssertEqual(lock.withLock { summaries.count }, 1)
        XCTAssertEqual(lock.withLock { transcribeCalls }, [m.a])
    }

    func test_notes_saved_on_every_part_even_when_part1_fails() {
        let m = makeTwoPartMeeting()
        failing = [m.a]
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: "first"))
        queue.waitUntilIdle()
        XCTAssertEqual(store.getMeeting(id: m.id)?.notes, "first")

        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: "first, second"))
        queue.waitUntilIdle()
        XCTAssertEqual(store.getMeeting(id: m.id)?.notes, "first, second")

        queue.enqueue(.init(meetingId: m.id, partIndex: 3, audioPath: nil, notes: "first, second, third"))
        queue.waitUntilIdle()
        XCTAssertEqual(store.getMeeting(id: m.id)?.notes, "first, second, third")
    }

    func test_legacy_meeting_without_part_rows_behaves_as_before() {
        let audio = touch("system-audio-L.caf").path
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Old",
            attendees: [], trigger: .manual, audioPath: nil, status: .recording
        ))!
        _ = store.completeMeeting(id: id, endUnixMs: 60_000, audioPath: audio)
        let queue = makeQueue()
        XCTAssertTrue(MeetingTranscriptionQueue.canTranscribeAgain(status: .completed, parts: store.meetingParts(meetingId: id) ?? []))
        queue.enqueue(.init(meetingId: id, partIndex: 1, audioPath: audio, notes: "n"))
        queue.waitUntilIdle()

        let meeting = store.getMeeting(id: id)
        XCTAssertEqual(meeting?.transcriptPath.map { URL(fileURLWithPath: $0).lastPathComponent }, "transcript-system-audio-L.txt")
        XCTAssertEqual(meeting?.notes, "n")
        XCTAssertEqual(lock.withLock { summaries.count }, 1)
        XCTAssertFalse(MeetingTranscriptionQueue.canTranscribeAgain(status: .completed, parts: store.meetingParts(meetingId: id) ?? []))

        queue.transcribeAgain(meetingId: id, notes: "n")
        CoreMemoryStore.accessQueue.sync {}
        queue.waitUntilIdle()
        XCTAssertEqual(count("said in system-audio-L", in: store.meetingTranscriptText(meetingId: id) ?? ""), 1)
    }

    func test_room_audio_title_from_part1_only_and_not_reprefixed() {
        let m = makeTwoPartMeeting(trigger: .calendar)
        meOnly = true
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()
        XCTAssertEqual(store.getMeeting(id: m.id)?.title, "Standup", "a later part never retitles")

        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.waitUntilIdle()
        XCTAssertEqual(store.getMeeting(id: m.id)?.title, "Room audio only · Standup")

        queue.transcribeAgain(meetingId: m.id, notes: "")
        CoreMemoryStore.accessQueue.sync {}
        queue.waitUntilIdle()
        XCTAssertEqual(store.getMeeting(id: m.id)?.title, "Room audio only · Standup")
    }

    @MainActor
    func test_availability_never_blocks_main_and_refreshes() {
        let m = makeTwoPartMeeting()
        let availability = TranscribeAgainAvailability(store: store)
        let meeting = store.getMeeting(id: m.id)!
        // Hold accessQueue: a synchronous read would hang here.
        let hold = DispatchSemaphore(value: 0)
        CoreMemoryStore.accessQueue.async { hold.wait() }
        XCTAssertFalse(availability.canTranscribeAgain(meeting), "unknown until checked off main")
        hold.signal()
        let became = expectation(description: "checked")
        became.assertForOverFulfill = false
        let sub = availability.$ids.sink { if $0.contains(m.id) { became.fulfill() } }
        wait(for: [became], timeout: 5)
        sub.cancel()
        XCTAssertTrue(availability.canTranscribeAgain(meeting))

        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()
        availability.refresh(m.id)
        let cleared = expectation(description: "rechecked")
        cleared.assertForOverFulfill = false
        let sub2 = availability.$ids.sink { if !$0.contains(m.id) { cleared.fulfill() } }
        wait(for: [cleared], timeout: 5)
        sub2.cancel()
    }

    func test_can_transcribe_again_considers_every_part() {
        let m = makeTwoPartMeeting()
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.waitUntilIdle()
        let parts = store.meetingParts(meetingId: m.id) ?? []
        XCTAssertTrue(MeetingTranscriptionQueue.canTranscribeAgain(status: .completed, parts: parts),
                      "part 2 has audio and no transcript")
        XCTAssertFalse(MeetingTranscriptionQueue.canTranscribeAgain(status: .recording, parts: parts))

        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()
        XCTAssertFalse(MeetingTranscriptionQueue.canTranscribeAgain(status: .completed, parts: store.meetingParts(meetingId: m.id) ?? []))
    }

    func test_forget_deletes_every_part_file() {
        let m = makeTwoPartMeeting()
        let micA = touch("mic-audio-A.caf"), micB = touch("mic-audio-B.caf")
        let queue = makeQueue()
        queue.enqueue(.init(meetingId: m.id, partIndex: 1, audioPath: m.a, notes: ""))
        queue.enqueue(.init(meetingId: m.id, partIndex: 2, audioPath: m.b, notes: ""))
        queue.waitUntilIdle()
        let transcripts = (store.meetingParts(meetingId: m.id) ?? []).compactMap(\.transcriptPath)
        XCTAssertEqual(transcripts.count, 2)

        let done = expectation(description: "forgotten")
        MeetingLifecycle.forgetMeeting(store: store, meetingId: m.id, recordingsDir: tempDir) { result in
            XCTAssertEqual(result, .deleted)
            done.fulfill()
        }
        wait(for: [done], timeout: 5)
        for path in [m.a, m.b, micA.path, micB.path] + transcripts {
            XCTAssertFalse(FileManager.default.fileExists(atPath: path), path)
        }
    }
}
