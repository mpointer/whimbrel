import XCTest
@testable import Whimbrel

@MainActor
final class MeetingSnapshotsStripTests: XCTestCase {
    private var tempDir: URL!
    private var store: CoreMemoryStore!
    private var meetingId: Int64!

    override func setUp() {
        super.setUp()
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try! FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
        meetingId = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .manual, audioPath: nil, status: .completed
        ))!
    }

    override func tearDown() {
        store = nil
        try? FileManager.default.removeItem(at: tempDir)
        super.tearDown()
    }

    private func addSnapshot(_ name: String, at ms: Int64) -> URL {
        let url = tempDir.appendingPathComponent(name)
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        try! rep.representation(using: .png, properties: [:])!.write(to: url)
        _ = store.insertMeetingSnapshot(meetingId: meetingId, capturedUnixMs: ms, filePath: url.path, width: 1, height: 1, ocrText: "")
        return url
    }

    private func makeModel() -> MeetingSnapshotsModel {
        MeetingSnapshotsModel(meetingId: meetingId, store: store)
    }

    /// I6: Timeline rows and the strip share one downsampled, cached loader.
    func testThumbnailLoaderDownsamplesAndCaches() async throws {
        let storage = SnapshotStorage(root: tempDir, fallbackRoot: tempDir)
        let url = try storage.writeHEIC(SnapshotFixtures.slide(["Roadmap", "Ship it"]), to: tempDir, captured: Date())
        let first = await SnapshotThumbnails.image(for: url.path)
        let image = try XCTUnwrap(first)
        XCTAssertLessThanOrEqual(max(image.size.width, image.size.height), 480)
        let second = await SnapshotThumbnails.image(for: url.path)
        XCTAssertTrue(second === image, "second load must come from the cache")
        let missing = await SnapshotThumbnails.image(for: tempDir.appendingPathComponent("gone.png").path)
        XCTAssertNil(missing)
    }

    /// A build replaces a slide's file; the strip shows the new image after a reload.
    func testReloadRefreshesThumbnailWhenFileChanges() async throws {
        let storage = SnapshotStorage(root: tempDir, fallbackRoot: tempDir)
        let first = try storage.writeHEIC(SnapshotFixtures.slide(["Roadmap"]), to: tempDir, captured: Date())
        let id = try XCTUnwrap(store.insertMeetingSnapshot(meetingId: meetingId, capturedUnixMs: 1, filePath: first.path,
                                                           width: 1, height: 1, ocrText: ""))
        let model = makeModel()
        await model.reload()
        let before = try XCTUnwrap(model.thumbnails[id]?.image)

        let second = try storage.writeHEIC(SnapshotFixtures.slide(["Roadmap", "Ship it"]), to: tempDir, captured: Date())
        XCTAssertTrue(store.replaceMeetingSnapshot(id: id, filePath: second.path, width: 1, height: 1, ocrText: ""))
        await model.reload()
        XCTAssertEqual(model.thumbnails[id]?.path, second.path)
        XCTAssertFalse(model.thumbnails[id]?.image === before)
    }

    func testReloadListsSnapshotsInCaptureOrder() async {
        let b = addSnapshot("b.png", at: 20)
        let a = addSnapshot("a.png", at: 10)
        let model = makeModel()
        await model.reload()
        XCTAssertEqual(model.snapshots.map(\.filePath), [a.path, b.path])
    }

    func testDeleteRemovesFileAndRow() async {
        let a = addSnapshot("a.png", at: 10)
        _ = addSnapshot("b.png", at: 20)
        let model = makeModel()
        await model.reload()
        await model.delete(model.snapshots[0])
        XCTAssertEqual(model.snapshots.count, 1)
        XCTAssertFalse(FileManager.default.fileExists(atPath: a.path))
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: meetingId).count, 1)
    }

    func testSetKeepPersists() async {
        _ = addSnapshot("a.png", at: 10)
        let model = makeModel()
        await model.setKeep(true)
        XCTAssertTrue(model.keep)
        XCTAssertEqual(store.meetingSnapshotsInfo(meetingId: meetingId)?.keep, true)
        let fresh = makeModel()
        await fresh.reload()
        XCTAssertTrue(fresh.keep)
    }

    func testFolderURLFromInfo() async {
        _ = addSnapshot("a.png", at: 10)
        store.setMeetingSnapshotsDir(meetingId: meetingId, dir: tempDir.path)
        let model = makeModel()
        await model.reload()
        XCTAssertEqual(model.folderURL?.path, tempDir.path)
        XCTAssertNil(MeetingSnapshotsModel.folderURL(fromInfoDir: nil))
    }
}
