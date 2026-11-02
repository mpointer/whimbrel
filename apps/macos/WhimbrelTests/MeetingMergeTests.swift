import XCTest
@testable import Whimbrel

/// "Merge with previous meeting" (docs/superpowers/specs/2026-11-02-meeting-dedupe-design.md §3).
final class MeetingMergeTests: XCTestCase {
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
        super.tearDown()
    }

    private func finished(_ title: String, startMs: Int64, endMs: Int64, audio: String) -> Int64 {
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "d", startUnixMs: startMs, endUnixMs: nil, title: title, attendees: [],
            trigger: .calendar, audioPath: nil, status: .recording))!
        _ = store.beginMeetingPart(meetingId: id, startUnixMs: startMs, continueKey: nil)
        _ = store.completeMeetingPart(meetingId: id, partIndex: 1, endUnixMs: endMs, audioPath: audio)
        _ = store.completeMeeting(id: id, endUnixMs: endMs, audioPath: audio)
        return id
    }

    private func slide(_ name: String, in dir: URL) -> URL {
        try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent(name)
        FileManager.default.createFile(atPath: url.path, contents: Data([1]))
        return url
    }

    func testMergeMovesSlidesThenParts() {
        let a = finished("Planning", startMs: 0, endMs: 1_800_000, audio: "/a/a.m4a")
        let b = finished("Planning", startMs: 2_100_000, endMs: 3_600_000, audio: "/a/b.m4a")
        XCTAssertEqual(store.previousMergeableMeeting(meetingId: b, deviceId: "d"), a)
        let shot = slide("s1.png", in: tempDir.appendingPathComponent("b"))
        XCTAssertNotNil(store.insertMeetingSnapshot(meetingId: b, capturedUnixMs: 2_200_000, filePath: shot.path, width: 1, height: 1, ocrText: "Roadmap"))
        let folderA = tempDir.appendingPathComponent("a", isDirectory: true)

        XCTAssertTrue(MeetingSplit.merge(store: store, into: a, from: b, makeFolder: { _, _ in
            try FileManager.default.createDirectory(at: folderA, withIntermediateDirectories: true)
            return folderA
        }))

        XCTAssertNil(store.getMeeting(id: b))
        XCTAssertEqual(store.meetingParts(meetingId: a)?.map(\.audioPath), ["/a/a.m4a", "/a/b.m4a"])
        XCTAssertTrue(FileManager.default.fileExists(atPath: folderA.appendingPathComponent("s1.png").path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: shot.path))
    }

    func testMergeStillHappensWhenSlideFilesCannotMove() {
        let a = finished("Planning", startMs: 0, endMs: 1_800_000, audio: "/a/a.m4a")
        let b = finished("Planning", startMs: 2_100_000, endMs: 3_600_000, audio: "/a/b.m4a")
        let shot = slide("s1.png", in: tempDir.appendingPathComponent("b"))
        XCTAssertNotNil(store.insertMeetingSnapshot(meetingId: b, capturedUnixMs: 2_200_000, filePath: shot.path, width: 1, height: 1, ocrText: "Roadmap"))

        XCTAssertTrue(MeetingSplit.merge(store: store, into: a, from: b, makeFolder: { _, _ in throw CocoaError(.fileWriteNoPermission) }))

        XCTAssertNil(store.getMeeting(id: b))
        XCTAssertEqual(store.meetingParts(meetingId: a)?.count, 2)
        XCTAssertTrue(FileManager.default.fileExists(atPath: shot.path), "slides stay where they were, still A's rows")
    }

    func testMergeRefusedWhenTargetGone() {
        let a = finished("Planning", startMs: 0, endMs: 1_800_000, audio: "/a/a.m4a")
        let b = finished("Planning", startMs: 2_100_000, endMs: 3_600_000, audio: "/a/b.m4a")
        let shot = slide("s1.png", in: tempDir.appendingPathComponent("b"))
        XCTAssertNotNil(store.insertMeetingSnapshot(meetingId: b, capturedUnixMs: 2_200_000, filePath: shot.path, width: 1, height: 1, ocrText: "Roadmap"))
        XCTAssertTrue(store.deleteMeeting(id: a))

        XCTAssertFalse(MeetingSplit.merge(store: store, into: a, from: b, makeFolder: { _, _ in self.tempDir }))

        XCTAssertNotNil(store.getMeeting(id: b))
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: b).count, 1, "slides stay with b")
    }

    func testTranscribeAgainWithNoAudioOnDiskClearsTheStage() {
        let a = finished("Planning", startMs: 0, endMs: 1_800_000, audio: "/nowhere/a.m4a")
        let cleared = expectation(description: "stage cleared")
        let queue = MeetingTranscriptionQueue(store: store, transcribe: { _, _, _ in throw CocoaError(.fileReadUnknown) },
                                              summarize: { _, _, _ in },
                                              onStage: { id, stage in if id == a, stage == nil { cleared.fulfill() } },
                                              multilingual: { false })
        queue.transcribeAgain(meetingId: a, notes: "")
        wait(for: [cleared], timeout: 5)
    }

    func testMergeOfferedOnlyWhenIdle() {
        XCTAssertFalse(MeetingOptionsMenu.canOfferMerge(previousId: 5, ownId: 6, processingMeetingId: nil, mediaWorkingId: nil, audioOnDisk: false))
        XCTAssertTrue(MeetingOptionsMenu.canOfferMerge(previousId: 5, ownId: 6, processingMeetingId: nil, mediaWorkingId: nil, audioOnDisk: true))
        XCTAssertFalse(MeetingOptionsMenu.canOfferMerge(previousId: nil, ownId: 6, processingMeetingId: nil, mediaWorkingId: nil, audioOnDisk: true))
        XCTAssertFalse(MeetingOptionsMenu.canOfferMerge(previousId: 5, ownId: nil, processingMeetingId: nil, mediaWorkingId: nil, audioOnDisk: true))
        XCTAssertFalse(MeetingOptionsMenu.canOfferMerge(previousId: 5, ownId: 6, processingMeetingId: 9, mediaWorkingId: nil, audioOnDisk: true))
        XCTAssertFalse(MeetingOptionsMenu.canOfferMerge(previousId: 5, ownId: 6, processingMeetingId: nil, mediaWorkingId: 5, audioOnDisk: true))
        XCTAssertFalse(MeetingOptionsMenu.canOfferMerge(previousId: 5, ownId: 6, processingMeetingId: nil, mediaWorkingId: 6, audioOnDisk: true))
        XCTAssertTrue(MeetingOptionsMenu.canOfferMerge(previousId: 5, ownId: 6, processingMeetingId: nil, mediaWorkingId: 7, audioOnDisk: true))
    }
}
