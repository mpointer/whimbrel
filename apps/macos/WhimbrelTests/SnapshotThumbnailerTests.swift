import ImageIO
import SQLite3
import UniformTypeIdentifiers
import XCTest
@testable import Whimbrel

/// Shrinking an old snapshot to `<name>.thumb.jpg`: hidden temp, decode check, flush, rename
/// without overwriting, flush the folder, rewrite the row durably, then delete the full image.
/// Everything runs in a temp folder with a temp store.
final class SnapshotThumbnailerTests: XCTestCase {
    private var tempDir: URL!
    private var folder: URL!
    private var store: CoreMemoryStore!
    private var meetingId: Int64!
    private let fm = FileManager.default

    override func setUpWithError() throws {
        tempDir = fm.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        folder = try SnapshotStorage.create("2026-10-22 10.14 Standup", under: tempDir.appendingPathComponent("Snapshots"))
        store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
        meetingId = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1000, endUnixMs: 2000, title: "Standup",
            attendees: [], trigger: .calendar, audioPath: nil, status: .completed
        ))!
    }

    override func tearDownWithError() throws {
        SnapshotThumbnailer.writeJPEG = SnapshotThumbnailer.encodeJPEG
        SnapshotThumbnailer.beforePathUpdate = {}
        store = nil
        try? fm.removeItem(at: tempDir)
    }

    @discardableResult
    private func writeImage(_ name: String, width: Int, height: Int, type: UTType = .png, in dir: URL? = nil) -> URL {
        let url = (dir ?? folder).appendingPathComponent(name)
        let ctx = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        ctx.setFillColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1)
        ctx.fill(CGRect(x: 0, y: 0, width: width / 2, height: height / 2))
        let dest = CGImageDestinationCreateWithURL(url as CFURL, type.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(dest, ctx.makeImage()!, nil)
        XCTAssertTrue(CGImageDestinationFinalize(dest))
        return url
    }

    private func decoded(_ url: URL) -> (type: String?, width: Int, height: Int)? {
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
              let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else { return nil }
        return (CGImageSourceGetType(src) as String?, img.width, img.height)
    }

    private func contents() -> [String] { ((try? fm.contentsOfDirectory(atPath: folder.path)) ?? []).sorted() }

    private func addSnapshot(_ name: String, width: Int = 1920, height: Int = 1080) -> CoreMeetingSnapshot {
        let url = writeImage(name, width: width, height: height)
        let id = store.insertMeetingSnapshot(meetingId: meetingId, capturedUnixMs: 1500, filePath: url.path,
                                             width: width, height: height, ocrText: "Roadmap calendar sync")!
        return store.listMeetingSnapshots(meetingId: meetingId).first { $0.id == id }!
    }

    // MARK: shrink

    func testThumbnailIs320WideJPEGBesideTheOriginal() throws {
        let full = writeImage("10.14.03.png", width: 1920, height: 1080)
        let thumb = try SnapshotThumbnailer.shrink(full)

        XCTAssertEqual(thumb.url.lastPathComponent, "10.14.03.thumb.jpg")
        XCTAssertEqual(thumb.url.deletingLastPathComponent().standardizedFileURL, folder.standardizedFileURL)
        XCTAssertEqual(thumb.width, 320)
        XCTAssertEqual(thumb.height, 180)
        let back = try XCTUnwrap(decoded(thumb.url))
        XCTAssertEqual(back.type, UTType.jpeg.identifier)
        XCTAssertEqual(back.width, 320)
        XCTAssertEqual(back.height, 180)
        XCTAssertTrue(fm.fileExists(atPath: full.path), "shrink alone never deletes the full image")
        XCTAssertEqual(contents(), [SnapshotStorage.markerName, "10.14.03.png", "10.14.03.thumb.jpg"].sorted())
    }

    func testNarrowImageKeepsItsWidth() throws {
        let thumb = try SnapshotThumbnailer.shrink(writeImage("10.14.03-2.heic", width: 200, height: 400, type: .heic))
        XCTAssertEqual(thumb.url.lastPathComponent, "10.14.03-2.thumb.jpg")
        XCTAssertEqual(thumb.width, 200)
        XCTAssertEqual(thumb.height, 400)
    }

    func testPortraitImageIs320Wide() throws {
        let thumb = try SnapshotThumbnailer.shrink(writeImage("10.14.03.png", width: 1080, height: 1920))
        XCTAssertEqual(thumb.width, 320)
        XCTAssertEqual(thumb.height, 569, accuracy: 1)
    }

    func testShrinkRefusesAFolderWithoutTheMarker() throws {
        let plain = tempDir.appendingPathComponent("Pictures", isDirectory: true)
        try fm.createDirectory(at: plain, withIntermediateDirectories: true)
        let photo = writeImage("10.14.03.png", width: 1920, height: 1080, in: plain)
        XCTAssertThrowsError(try SnapshotThumbnailer.shrink(photo))
        XCTAssertEqual(try fm.contentsOfDirectory(atPath: plain.path), ["10.14.03.png"])
    }

    func testShrinkRefusesNamesWhimbrelDoesNotWrite() {
        for name in ["Screenshot 2026-10-22 at 10.14.03.png", "slide.png", "10.14.03.jpg", "10.14.03.thumb.jpg"] {
            writeImage(name, width: 640, height: 360, type: name.hasSuffix(".jpg") ? .jpeg : .png)
            XCTAssertThrowsError(try SnapshotThumbnailer.shrink(folder.appendingPathComponent(name)), name)
        }
        XCTAssertFalse(contents().contains { $0.hasSuffix(".thumb.jpg") && $0 != "10.14.03.thumb.jpg" })
    }

    func testShrinkNeverOverwritesAThumbnail() throws {
        let full = writeImage("10.14.03.png", width: 1920, height: 1080)
        let existing = folder.appendingPathComponent("10.14.03.thumb.jpg")
        try Data("mine".utf8).write(to: existing)
        XCTAssertThrowsError(try SnapshotThumbnailer.shrink(full))
        XCTAssertEqual(try Data(contentsOf: existing), Data("mine".utf8))
        XCTAssertEqual(contents(), [SnapshotStorage.markerName, "10.14.03.png", "10.14.03.thumb.jpg"].sorted())
    }

    func testShrinkThatWontDecodeLeavesNothing() {
        let full = writeImage("10.14.03.png", width: 1920, height: 1080)
        SnapshotThumbnailer.writeJPEG = { _, url in try Data("not a jpeg".utf8).write(to: url) }
        XCTAssertThrowsError(try SnapshotThumbnailer.shrink(full))
        XCTAssertEqual(contents(), [SnapshotStorage.markerName, "10.14.03.png"].sorted())
    }

    // MARK: shrinkSnapshot

    func testShrinkSnapshotRewritesTheRowThenDeletesTheFullImage() throws {
        let snap = addSnapshot("10.14.03.png")
        XCTAssertTrue(SnapshotThumbnailer.shrinkSnapshot(snap, store: store))

        let row = try XCTUnwrap(store.listMeetingSnapshots(meetingId: meetingId).first)
        XCTAssertEqual(URL(fileURLWithPath: row.filePath).lastPathComponent, "10.14.03.thumb.jpg")
        XCTAssertEqual(row.width, 320)
        XCTAssertEqual(row.height, 180)
        XCTAssertEqual(row.ocrText, "Roadmap calendar sync")
        XCTAssertEqual(row.capturedUnixMs, 1500)
        XCTAssertEqual(contents(), [SnapshotStorage.markerName, "10.14.03.thumb.jpg"].sorted())
    }

    func testShrinkSnapshotWriteFailureKeepsTheFullImage() {
        let snap = addSnapshot("10.14.03.png")
        SnapshotThumbnailer.writeJPEG = { _, _ in throw CocoaError(.fileWriteOutOfSpace) }
        XCTAssertFalse(SnapshotThumbnailer.shrinkSnapshot(snap, store: store))
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: meetingId), [snap])
        XCTAssertEqual(contents(), [SnapshotStorage.markerName, "10.14.03.png"].sorted())
    }

    func testShrinkSnapshotRowUpdateFailureRemovesTheThumbnail() {
        let snap = addSnapshot("10.14.03.png")
        // The row is gone by the time the rewrite runs, so the rewrite changes nothing.
        let gone = CoreMeetingSnapshot(id: snap.id + 999, meetingId: snap.meetingId, capturedUnixMs: snap.capturedUnixMs,
                                       filePath: snap.filePath, width: snap.width, height: snap.height, ocrText: snap.ocrText)
        XCTAssertFalse(SnapshotThumbnailer.shrinkSnapshot(gone, store: store))
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: meetingId), [snap])
        XCTAssertEqual(contents(), [SnapshotStorage.markerName, "10.14.03.png"].sorted())
    }

    func testShrinkSnapshotNotConfirmedDurableDeletesNothing() throws {
        let snap = addSnapshot("10.14.03.png")
        var reader: OpaquePointer?
        SnapshotThumbnailer.beforePathUpdate = { reader = holdReadSnapshot(self.tempDir.appendingPathComponent("test.sqlite")) }

        XCTAssertFalse(SnapshotThumbnailer.shrinkSnapshot(snap, store: store))
        XCTAssertNotNil(reader)
        sqlite3_close_v2(reader)
        // Committed but unconfirmed: after a power cut the row may point either way, so both stay.
        XCTAssertEqual(contents(), [SnapshotStorage.markerName, "10.14.03.png", "10.14.03.thumb.jpg"].sorted())
        let row = try XCTUnwrap(store.listMeetingSnapshots(meetingId: meetingId).first)
        XCTAssertTrue(row.filePath.hasSuffix(".thumb.jpg"))
    }

    func testShrinkSnapshotOutsideAMarkedFolderChangesNothing() throws {
        let plain = tempDir.appendingPathComponent("Pictures", isDirectory: true)
        try fm.createDirectory(at: plain, withIntermediateDirectories: true)
        let photo = writeImage("10.14.03.png", width: 1920, height: 1080, in: plain)
        let id = store.insertMeetingSnapshot(meetingId: meetingId, capturedUnixMs: 1500, filePath: photo.path,
                                             width: 1920, height: 1080, ocrText: "x")!
        let snap = store.listMeetingSnapshots(meetingId: meetingId).first { $0.id == id }!
        XCTAssertFalse(SnapshotThumbnailer.shrinkSnapshot(snap, store: store))
        XCTAssertEqual(try fm.contentsOfDirectory(atPath: plain.path), ["10.14.03.png"])
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: meetingId), [snap])
    }
}
