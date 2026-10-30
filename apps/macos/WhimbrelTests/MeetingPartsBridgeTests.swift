import XCTest
@testable import Whimbrel

/// Swift plumbing for meeting parts (core/memory `meeting_parts`). The rules are tested in
/// Rust; these prove the bridge reaches them and the JSON decodes.
final class MeetingPartsBridgeTests: XCTestCase {
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

    private func makeMeeting() -> Int64 {
        store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .manual, audioPath: nil, status: .recording
        ))!
    }

    private func touch(_ name: String) -> URL {
        let url = tempDir.appendingPathComponent(name)
        try! Data(name.utf8).write(to: url)
        return url
    }

    func testBeginCompleteFindAndReopen() {
        let mid = makeMeeting()
        XCTAssertEqual(store.beginMeetingPart(meetingId: mid, startUnixMs: 1_000, continueKey: "cal:1"), 1)
        XCTAssertTrue(store.completeMeetingPart(meetingId: mid, partIndex: 1, endUnixMs: 60_000, audioPath: "/r/system-audio-A.caf"))
        XCTAssertTrue(store.completeMeeting(id: mid, endUnixMs: 60_000, audioPath: "/r/system-audio-A.caf"))

        XCTAssertEqual(store.findContinuableMeeting(continueKey: "cal:1", nowUnixMs: 120_000), mid)
        XCTAssertNil(store.findContinuableMeeting(continueKey: "cal:other", nowUnixMs: 120_000))
        XCTAssertNil(store.findContinuableMeeting(continueKey: "cal:1", nowUnixMs: 60_000 + 600_001))

        XCTAssertTrue(store.reopenMeeting(id: mid))
        XCTAssertEqual(store.beginMeetingPart(meetingId: mid, startUnixMs: 100_000, continueKey: nil), 2)
        XCTAssertTrue(store.setMeetingPartTranscript(meetingId: mid, partIndex: 2, transcriptPath: "/r/transcript-B.txt"))

        let parts = store.meetingParts(meetingId: mid)
        XCTAssertEqual(parts?.map(\.partIndex), [1, 2])
        XCTAssertEqual(parts?[0].audioPath, "/r/system-audio-A.caf")
        XCTAssertEqual(parts?[0].endUnixMs, 60_000)
        XCTAssertEqual(parts?[1].transcriptPath, "/r/transcript-B.txt")
        XCTAssertEqual(parts?[1].transcribed, true)
        XCTAssertEqual(parts?[0].transcribed, false)
    }

    func testReplacePartSegmentsThenTranscriptTextHasDivider() {
        let mid = makeMeeting()
        _ = store.beginMeetingPart(meetingId: mid, startUnixMs: 1_000, continueKey: nil)
        _ = store.beginMeetingPart(meetingId: mid, startUnixMs: 100_000, continueKey: nil)
        XCTAssertEqual(store.replacePartTranscriptSegments(meetingId: mid, partIndex: 1, segments: [
            NewTranscriptSegmentInput(startCs: 0, speakerLabel: "Me", text: "first part")
        ]), 1)
        XCTAssertEqual(store.replacePartTranscriptSegments(meetingId: mid, partIndex: 2, segments: [
            NewTranscriptSegmentInput(startCs: 5, speakerLabel: "Them", text: "second part")
        ]), 1)

        let text = store.meetingTranscriptText(meetingId: mid) ?? ""
        XCTAssertTrue(text.contains("first part"), text)
        XCTAssertTrue(text.contains("second part"), text)
        XCTAssertTrue(text.contains("rejoined"), text)
        XCTAssertLessThan(text.range(of: "first part")!.lowerBound, text.range(of: "rejoined")!.lowerBound)
        XCTAssertLessThan(text.range(of: "rejoined")!.lowerBound, text.range(of: "second part")!.lowerBound)
    }

    func testSplitMovesLaterPartsToANewMeeting() {
        let mid = makeMeeting()
        _ = store.beginMeetingPart(meetingId: mid, startUnixMs: 1_000, continueKey: nil)
        _ = store.beginMeetingPart(meetingId: mid, startUnixMs: 100_000, continueKey: nil)
        XCTAssertTrue(store.completeMeeting(id: mid, endUnixMs: 200_000, audioPath: nil))

        guard let result = store.splitMeeting(meetingId: mid, fromPartIndex: 2) else { return XCTFail("split returned nil") }
        XCTAssertNotEqual(result.newMeetingId, mid)
        XCTAssertEqual(result.movedSnapshotPaths, [])
        XCTAssertEqual(store.meetingParts(meetingId: mid)?.count, 1)
        XCTAssertEqual(store.meetingParts(meetingId: result.newMeetingId)?.count, 1)
        XCTAssertNil(store.splitMeeting(meetingId: mid, fromPartIndex: 1), "cannot split before the first part")
        XCTAssertTrue(store.setMeetingSnapshotPaths(meetingId: result.newMeetingId, moves: [], snapshotsDir: tempDir.path))
    }

    func testDeleteReturningFilesRemovesEveryPartsFiles() {
        let mid = makeMeeting()
        let s1 = touch("system-audio-A.caf"), m1 = touch("mic-audio-A.caf"), t1 = touch("transcript-A.txt")
        let s2 = touch("system-audio-B.caf"), m2 = touch("mic-audio-B.caf"), t2 = touch("transcript-B.txt")
        _ = store.beginMeetingPart(meetingId: mid, startUnixMs: 1_000, continueKey: nil)
        _ = store.completeMeetingPart(meetingId: mid, partIndex: 1, endUnixMs: 2_000, audioPath: s1.path)
        _ = store.setMeetingPartTranscript(meetingId: mid, partIndex: 1, transcriptPath: t1.path)
        _ = store.beginMeetingPart(meetingId: mid, startUnixMs: 3_000, continueKey: nil)
        _ = store.completeMeetingPart(meetingId: mid, partIndex: 2, endUnixMs: 4_000, audioPath: s2.path)
        _ = store.setMeetingPartTranscript(meetingId: mid, partIndex: 2, transcriptPath: t2.path)

        guard let files = store.deleteMeetingReturningFiles(id: mid) else { return XCTFail("no files returned") }
        XCTAssertEqual(Set(files.audioPaths), [s1.path, s2.path])
        XCTAssertEqual(Set(files.transcriptPaths), [t1.path, t2.path])
        CoreMemoryStore.deleteAllMeetingFiles(files, recordingsDir: tempDir)

        for url in [s1, m1, t1, s2, m2, t2] {
            XCTAssertFalse(FileManager.default.fileExists(atPath: url.path), url.lastPathComponent)
        }
        XCTAssertNil(store.deleteMeetingReturningFiles(id: mid), "already deleted")
    }

    /// Part 1 recorded A; the meeting was reopened for a part 2 whose row never got written.
    private func continuedMeetingMissingPart2() -> (id: Int64, a: URL, b: URL) {
        let mid = makeMeeting()
        let a = touch("system-audio-A.caf"), b = touch("system-audio-B.caf")
        _ = store.beginMeetingPart(meetingId: mid, startUnixMs: 1_000, continueKey: "cal:1")
        _ = store.completeMeetingPart(meetingId: mid, partIndex: 1, endUnixMs: 60_000, audioPath: a.path)
        _ = store.completeMeeting(id: mid, endUnixMs: 60_000, audioPath: a.path)
        _ = store.reopenMeeting(id: mid)
        return (mid, a, b)
    }

    /// An unknown part index on a continued meeting must not leave its audio unowned:
    /// `completeMeeting` keeps part 1's path, so the orphan scan would delete it.
    func testEndingAContinuedMeetingWithUnknownPartKeepsItsAudioOwned() {
        let m = continuedMeetingMissingPart2()
        // Backdate past the core sweep's 7-day age guard so ownership, not age, is what's tested.
        for u in [m.a, m.b] {
            try! FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(-8 * 86_400)], ofItemAtPath: u.path)
        }
        MeetingContinuation.completeRecording(store: store, meetingId: m.id, partIndex: nil, continued: true,
                                              endUnixMs: 120_000, audioPath: m.b.path)
        let orphaned = store.scanOrphanedRecordings(recordingsDir: tempDir, mediaAudioDir: nil, excluding: [])?.orphaned
        XCTAssertEqual(orphaned, [], "both parts' audio stays owned")
        let parts = store.meetingParts(meetingId: m.id)
        XCTAssertEqual(parts?.map(\.partIndex), [1, 2])
        XCTAssertEqual(parts?.map(\.audioPath), [m.a.path, m.b.path], "part 1 untouched")
        XCTAssertEqual(store.getMeeting(id: m.id)?.status, .completed)
        XCTAssertTrue(MeetingTranscriptionQueue.canTranscribeAgain(status: .completed, parts: parts ?? []),
                      "the skipped part can still be transcribed by hand")
    }

    func testEndingWithAKnownPartCompletesThatPart() {
        let m = continuedMeetingMissingPart2()
        let part = store.beginMeetingPart(meetingId: m.id, startUnixMs: 100_000, continueKey: nil)
        MeetingContinuation.completeRecording(store: store, meetingId: m.id, partIndex: part, continued: true,
                                              endUnixMs: 120_000, audioPath: m.b.path)
        let parts = store.meetingParts(meetingId: m.id)
        XCTAssertEqual(parts?.map(\.audioPath), [m.a.path, m.b.path])
        XCTAssertEqual(parts?.last?.endUnixMs, 120_000)
    }
}
