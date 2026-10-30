import XCTest
@testable import Whimbrel

/// Swift-side plumbing for meeting snapshots (core/memory Task 1 store, Task 2 folder
/// deletion). The rules themselves are tested in Rust; these prove the bridge reaches them.
final class MeetingSnapshotBridgeTests: XCTestCase {
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
            deviceId: "dev", startUnixMs: 1000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .manual, audioPath: nil, status: .completed
        ))!
    }

    private func touch(_ url: URL) {
        try! Data("x".utf8).write(to: url)
    }

    func testSetKeepMediaRoundTrips() {
        let mid = makeMeeting()
        XCTAssertEqual(store.getMeeting(id: mid)?.keepMedia, false)
        XCTAssertTrue(store.setKeepMedia(meetingId: mid, keep: true))
        XCTAssertEqual(store.getMeeting(id: mid)?.keepMedia, true)
        XCTAssertFalse(store.setKeepMedia(meetingId: 999, keep: true))
    }

    func testInsertListReplaceDeleteRoundTrip() {
        let mid = makeMeeting()
        let a = tempDir.appendingPathComponent("a.png"), b = tempDir.appendingPathComponent("b.png")
        touch(a); touch(b)

        let sid = store.insertMeetingSnapshot(meetingId: mid, capturedUnixMs: 5, filePath: a.path, width: 10, height: 20, ocrText: "hello")
        XCTAssertNotNil(sid)
        XCTAssertNil(store.insertMeetingSnapshot(meetingId: 999, capturedUnixMs: 5, filePath: a.path, width: 1, height: 1, ocrText: ""))

        XCTAssertEqual(store.listMeetingSnapshots(meetingId: mid), [
            CoreMeetingSnapshot(id: sid!, meetingId: mid, capturedUnixMs: 5, filePath: a.path, width: 10, height: 20, ocrText: "hello")
        ])

        XCTAssertTrue(store.replaceMeetingSnapshot(id: sid!, filePath: b.path, width: 1, height: 2, ocrText: "yo"))
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: mid).first?.filePath, b.path)
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: mid).first?.ocrText, "yo")

        XCTAssertEqual(store.deleteMeetingSnapshot(id: sid!), b.path)
        XCTAssertNil(store.deleteMeetingSnapshot(id: sid!))
        XCTAssertTrue(store.listMeetingSnapshots(meetingId: mid).isEmpty)
    }

    func testListDropsRowsWhoseFileIsMissing() {
        let mid = makeMeeting()
        let present = tempDir.appendingPathComponent("present.png")
        touch(present)
        let gone = tempDir.appendingPathComponent("gone.png")
        let keptId = store.insertMeetingSnapshot(meetingId: mid, capturedUnixMs: 1, filePath: present.path, width: 1, height: 1, ocrText: "")!
        let goneId = store.insertMeetingSnapshot(meetingId: mid, capturedUnixMs: 2, filePath: gone.path, width: 1, height: 1, ocrText: "")!

        XCTAssertEqual(store.listMeetingSnapshots(meetingId: mid).map(\.id), [keptId])
        // The stale row was deleted, not just hidden.
        XCTAssertNil(store.deleteMeetingSnapshot(id: goneId))
    }

    /// M1: an unmounted volume or unreachable root makes every file look missing; that must
    /// not wipe the index (and with it the slide text in summaries and search).
    func testListKeepsRowsWhenTheSnapshotFolderItselfIsMissing() {
        let mid = makeMeeting()
        let unmounted = URL(fileURLWithPath: "/Volumes/whimbrel-test-\(UUID().uuidString)/2026-10-22 10.00 Standup/10.14.03.png")
        let id = store.insertMeetingSnapshot(meetingId: mid, capturedUnixMs: 1, filePath: unmounted.path, width: 1, height: 1, ocrText: "Q3 roadmap")!

        XCTAssertEqual(store.listMeetingSnapshots(meetingId: mid).map(\.id), [id])
        XCTAssertEqual(store.meetingSlidesText(meetingId: mid), "Q3 roadmap")
        XCTAssertEqual(store.deleteMeetingSnapshot(id: id), unmounted.path, "the row must not have been pruned")
    }

    func testKeepFlagAndDirRoundTrip() {
        let mid = makeMeeting()
        XCTAssertNil(store.meetingSnapshotsInfo(meetingId: 999))
        var info = store.meetingSnapshotsInfo(meetingId: mid)
        XCTAssertNil(info?.dir)
        XCTAssertEqual(info?.keep, false)

        store.setMeetingSnapshotsDir(meetingId: mid, dir: "/some/dir")
        store.setKeepSnapshots(meetingId: mid, keep: true)
        info = store.meetingSnapshotsInfo(meetingId: mid)
        XCTAssertEqual(info?.dir, "/some/dir")
        XCTAssertEqual(info?.keep, true)
    }

    private func makeMarkedFolder(_ name: String) -> URL {
        let dir = tempDir.appendingPathComponent(name, isDirectory: true)
        try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        touch(dir.appendingPathComponent(".kestrel-snapshots"))
        touch(dir.appendingPathComponent("10.14.03.png"))
        return dir
    }

    private func forget(_ mid: Int64) {
        let done = expectation(description: "forget")
        MeetingLifecycle.forgetMeeting(store: store, meetingId: mid) { result in
            XCTAssertEqual(result, .deleted)
            done.fulfill()
        }
        wait(for: [done], timeout: 10)
    }

    func testForgetMeetingDeletesUnkeptSnapshotFolderAndSparesKept() {
        let unkeptDir = makeMarkedFolder("unkept"), keptDir = makeMarkedFolder("kept")
        let unkept = makeMeeting(), kept = makeMeeting()
        store.setMeetingSnapshotsDir(meetingId: unkept, dir: unkeptDir.path)
        store.setMeetingSnapshotsDir(meetingId: kept, dir: keptDir.path)
        store.setKeepSnapshots(meetingId: kept, keep: true)

        forget(unkept)
        forget(kept)

        XCTAssertFalse(FileManager.default.fileExists(atPath: unkeptDir.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: keptDir.appendingPathComponent("10.14.03.png").path))
    }

    func testKeywordSearchReturnsSnapshotHit() {
        let mid = makeMeeting()
        let a = tempDir.appendingPathComponent("a.png")
        touch(a)
        _ = store.insertMeetingSnapshot(meetingId: mid, capturedUnixMs: 5, filePath: a.path, width: 10, height: 20, ocrText: "Org chart Alice Bob")

        let hits = store.searchSnapshotKeyword("Alice")
        XCTAssertEqual(hits.count, 1)
        XCTAssertEqual(hits.first?.meetingId, mid)
        XCTAssertEqual(hits.first?.meetingTitle, "Standup")
        XCTAssertEqual(hits.first?.filePath, a.path)
        XCTAssertTrue(store.searchSnapshotKeyword("zebra").isEmpty)
    }

    func testMeetingSlidesTextJoinsOcrWithBlankLines() {
        let mid = makeMeeting()
        let a = tempDir.appendingPathComponent("a.png"), b = tempDir.appendingPathComponent("b.png")
        touch(a); touch(b)
        _ = store.insertMeetingSnapshot(meetingId: mid, capturedUnixMs: 5, filePath: a.path, width: 1, height: 1, ocrText: "Slide one")
        _ = store.insertMeetingSnapshot(meetingId: mid, capturedUnixMs: 6, filePath: b.path, width: 1, height: 1, ocrText: "Slide two")
        XCTAssertEqual(store.meetingSlidesText(meetingId: mid), "Slide one\n\nSlide two")
    }
}
