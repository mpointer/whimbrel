import XCTest
@testable import Whimbrel

final class MeetingLifecycleTests: XCTestCase {
    func testForgetMeetingDeletesRowBeforeFiles() {
        var callOrder: [String] = []
        let result = MeetingLifecycle.forgetMeetingSync(
            deleteRow: { callOrder.append("row"); return true },
            deleteFiles: { callOrder.append("files") }
        )
        XCTAssertEqual(callOrder, ["row", "files"])
        XCTAssertEqual(result, .deleted)
    }

    func testForgetMeetingSkipsFileCleanupWhenRowDeleteFails() {
        var filesCalled = false
        let result = MeetingLifecycle.forgetMeetingSync(
            deleteRow: { false },
            deleteFiles: { filesCalled = true }
        )
        XCTAssertFalse(filesCalled)
        XCTAssertEqual(result, .failed)
    }

    // MARK: - split

    private func makeSplittable() throws -> (dir: URL, store: CoreMemoryStore, id: Int64, snap: URL) {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let store = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("t.sqlite"), modelsDir: nil))
        let id = try XCTUnwrap(store.insertMeeting(CoreNewMeeting(
            deviceId: "d", startUnixMs: 1_000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .manual, audioPath: nil, status: .recording)))
        _ = store.beginMeetingPart(meetingId: id, startUnixMs: 1_000, continueKey: nil)
        _ = store.completeMeetingPart(meetingId: id, partIndex: 1, endUnixMs: 30_000, audioPath: nil)
        _ = store.completeMeeting(id: id, endUnixMs: 30_000, audioPath: nil)
        _ = store.reopenMeeting(id: id)
        _ = store.beginMeetingPart(meetingId: id, startUnixMs: 61_000, continueKey: nil)
        _ = store.completeMeetingPart(meetingId: id, partIndex: 2, endUnixMs: 90_000, audioPath: nil)
        _ = store.completeMeeting(id: id, endUnixMs: 90_000, audioPath: nil)
        let old = dir.appendingPathComponent("old", isDirectory: true)
        try FileManager.default.createDirectory(at: old, withIntermediateDirectories: true)
        let snap = old.appendingPathComponent("snap-2.png")
        try Data("png".utf8).write(to: snap)
        _ = store.insertMeetingSnapshot(meetingId: id, capturedUnixMs: 70_000, filePath: snap.path, width: 1, height: 1, ocrText: "")
        return (dir, store, id, snap)
    }

    func testSplitCopiesSnapshotsThenRepointsThenDeletesOriginals() throws {
        let m = try makeSplittable()
        defer { try? FileManager.default.removeItem(at: m.dir) }
        let target = m.dir.appendingPathComponent("new-folder", isDirectory: true)
        let newId = try XCTUnwrap(MeetingLifecycle.splitLastPartSync(store: m.store, meetingId: m.id) { _, _ in
            try FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
            return target
        }).newMeetingId
        let moved = target.appendingPathComponent("snap-2.png")
        XCTAssertTrue(FileManager.default.fileExists(atPath: moved.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: m.snap.path))
        XCTAssertEqual(m.store.listMeetingSnapshots(meetingId: newId).map(\.filePath), [moved.path])
        XCTAssertEqual(m.store.meetingParts(meetingId: m.id)?.count, 1)
        XCTAssertEqual(m.store.meetingParts(meetingId: newId)?.count, 1)
    }

    func testSplitLeavesOriginalsWhenTheFolderCannotBeMade() throws {
        let m = try makeSplittable()
        defer { try? FileManager.default.removeItem(at: m.dir) }
        let outcome = MeetingLifecycle.splitLastPartSync(store: m.store, meetingId: m.id) { _, _ in throw CocoaError(.fileWriteUnknown) }
        XCTAssertEqual(outcome?.snapshotMoveFailed, true)
        let newId = outcome?.newMeetingId
        XCTAssertTrue(FileManager.default.fileExists(atPath: m.snap.path))
        XCTAssertEqual(m.store.listMeetingSnapshots(meetingId: try XCTUnwrap(newId)).map(\.filePath), [m.snap.path])
    }

    func testSplitRefusesASinglePartMeeting() throws {
        let m = try makeSplittable()
        defer { try? FileManager.default.removeItem(at: m.dir) }
        let newId = try XCTUnwrap(MeetingLifecycle.splitLastPartSync(store: m.store, meetingId: m.id) { _, _ in m.dir }).newMeetingId
        XCTAssertNil(MeetingLifecycle.splitLastPartSync(store: m.store, meetingId: newId) { _, _ in m.dir })
    }

    func testSplitWrapperPostsDidChangeForBothMeetings() throws {
        let m = try makeSplittable()
        defer { try? FileManager.default.removeItem(at: m.dir) }
        let posted = expectation(description: "posted")
        var ids: [Int64] = []
        let token = NotificationCenter.default.addObserver(forName: MeetingLifecycle.meetingsDidChange, object: nil, queue: nil) {
            ids = $0.userInfo?["meetingIds"] as? [Int64] ?? []
            posted.fulfill()
        }
        defer { NotificationCenter.default.removeObserver(token) }
        var outcome: MeetingLifecycle.SplitOutcome?
        MeetingLifecycle.splitLastPart(store: m.store, meetingId: m.id, makeFolder: { _, _ in m.dir }) { outcome = $0 }
        wait(for: [posted], timeout: 5)
        XCTAssertEqual(ids, [m.id, try XCTUnwrap(outcome?.newMeetingId)])
    }

    func testDeletingEitherSplitMeetingKeepsTheOthersSnapshotFiles() throws {
        for deleteOriginal in [true, false] {
            let m = try makeSplittable()
            defer { try? FileManager.default.removeItem(at: m.dir) }
            let keptOriginalSnap = m.dir.appendingPathComponent("old/snap-1.png")
            try Data("png".utf8).write(to: keptOriginalSnap)
            _ = m.store.insertMeetingSnapshot(meetingId: m.id, capturedUnixMs: 5_000, filePath: keptOriginalSnap.path, width: 1, height: 1, ocrText: "")
            let target = m.dir.appendingPathComponent("new-folder", isDirectory: true)
            let out = try XCTUnwrap(MeetingLifecycle.splitLastPartSync(store: m.store, meetingId: m.id) { _, _ in
                try FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
                return target
            })
            let (gone, kept) = deleteOriginal ? (m.id, out.newMeetingId) : (out.newMeetingId, m.id)
            let files = try XCTUnwrap(m.store.deleteMeetingReturningFiles(id: gone))
            if let dir = files.snapshotsDir { try? FileManager.default.removeItem(atPath: dir) } // what the cleanup does to a returned folder
            for snap in m.store.listMeetingSnapshots(meetingId: kept) {
                XCTAssertTrue(FileManager.default.fileExists(atPath: snap.filePath), "\(snap.filePath) (deleteOriginal=\(deleteOriginal))")
            }
        }
    }

    func testFailedCopyLeavesOriginalDeleteReportingNoSnapshotFolder() throws {
        let m = try makeSplittable()
        defer { try? FileManager.default.removeItem(at: m.dir) }
        let out = try XCTUnwrap(MeetingLifecycle.splitLastPartSync(store: m.store, meetingId: m.id) { _, _ in throw CocoaError(.fileWriteUnknown) })
        XCTAssertTrue(out.snapshotMoveFailed)
        // The new meeting's rows still point into the old folder, so deleting the original must not offer it.
        let files = try XCTUnwrap(m.store.deleteMeetingReturningFiles(id: m.id))
        XCTAssertNil(files.snapshotsDir)
        XCTAssertTrue(FileManager.default.fileExists(atPath: m.snap.path))
    }
}
