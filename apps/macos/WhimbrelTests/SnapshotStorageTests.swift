import XCTest
import CoreGraphics
import UniformTypeIdentifiers
@testable import Whimbrel

final class SnapshotStorageTests: XCTestCase {
    private var tmp: URL!

    override func setUpWithError() throws {
        tmp = FileManager.default.temporaryDirectory.appendingPathComponent("SnapshotStorageTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        // Restore permissions so read-only fixtures can be removed.
        try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: tmp.path)
        try? FileManager.default.removeItem(at: tmp)
    }

    private var utc: TimeZone { TimeZone(identifier: "UTC")! }
    private var start: Date { Date(timeIntervalSince1970: 1_792_664_040) } // 2026-10-22 10:14:00 UTC

    private func storage() -> SnapshotStorage {
        SnapshotStorage(root: tmp.appendingPathComponent("root"), fallbackRoot: tmp.appendingPathComponent("fallback"))
    }

    func testFolderNameFormat() {
        XCTAssertEqual(SnapshotStorage.folderName(start: start, title: "Acme sync", timeZone: utc), "2026-10-22 10.14 Acme sync")
    }

    func testFolderNameSanitisesHostileTitles() {
        func title(_ t: String) -> String {
            String(SnapshotStorage.folderName(start: start, title: t, timeZone: utc).dropFirst("2026-10-22 10.14 ".count))
        }
        XCTAssertEqual(title("a/b:c"), "a-b-c")
        XCTAssertEqual(title("a\\b"), "a-b")
        XCTAssertEqual(title("..hidden"), "hidden")
        XCTAssertEqual(title("a\u{07}b\nc"), "abc")
        XCTAssertEqual(title(String(repeating: "x", count: 300)).count, 80)
        let family = title(String(repeating: "👨‍👩‍👧", count: 100))
        XCTAssertLessThanOrEqual(family.utf8.count, 200)
        XCTAssertEqual(family, String(repeating: "👨‍👩‍👧", count: family.count), "truncates on grapheme boundaries")
        XCTAssertEqual(title("🚀 Launch"), "🚀 Launch")
        XCTAssertEqual(title("   "), "Meeting")
        XCTAssertEqual(title("../.."), "-..", "\"..-..\" with leading dots stripped")
    }

    func testLongMultibyteTitlesStillCreateFolders() throws {
        let s = storage()
        for title in [String(repeating: "漢", count: 80), String(repeating: "👨‍👩‍👧", count: 100)] {
            let folder = try s.makeMeetingFolder(start: start, title: title)
            let again = try s.makeMeetingFolder(start: start, title: title)
            XCTAssertEqual(folder.deletingLastPathComponent().standardizedFileURL.path, s.root.standardizedFileURL.path)
            XCTAssertLessThanOrEqual(folder.lastPathComponent.utf8.count, 255)
            XCTAssertLessThanOrEqual(again.lastPathComponent.utf8.count, 255)
        }
    }

    func testHostileTitlesCreateFoldersDirectlyUnderRoot() throws {
        let s = storage()
        for title in ["a/b:c", "../.."] {
            let folder = try s.makeMeetingFolder(start: start, title: title)
            XCTAssertEqual(folder.deletingLastPathComponent().standardizedFileURL.path, s.root.standardizedFileURL.path)
        }
    }

    func testFolderCollisionGetsSuffix() throws {
        let s = storage()
        let a = try s.makeMeetingFolder(start: start, title: "Acme")
        let b = try s.makeMeetingFolder(start: start, title: "Acme")
        let c = try s.makeMeetingFolder(start: start, title: "Acme")
        XCTAssertEqual(a.lastPathComponent.hasSuffix("Acme"), true)
        XCTAssertEqual(b.lastPathComponent, a.lastPathComponent + " 2")
        XCTAssertEqual(c.lastPathComponent, a.lastPathComponent + " 3")
    }

    func testMakeFolderWritesMarker() throws {
        let folder = try storage().makeMeetingFolder(start: start, title: "Acme")
        let names = try FileManager.default.contentsOfDirectory(atPath: folder.path)
        XCTAssertEqual(names, [".kestrel-snapshots"])
        XCTAssertEqual(try Data(contentsOf: folder.appendingPathComponent(".kestrel-snapshots")).count, 0)
    }

    func testFallsBackWhenRootUnwritable() throws {
        let locked = tmp.appendingPathComponent("locked")
        try FileManager.default.createDirectory(at: locked, withIntermediateDirectories: true)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: locked.path)
        let fallback = tmp.appendingPathComponent("fallback")
        let s = SnapshotStorage(root: locked.appendingPathComponent("sub"), fallbackRoot: fallback)
        let folder = try s.makeMeetingFolder(start: start, title: "Acme")
        XCTAssertEqual(folder.deletingLastPathComponent().standardizedFileURL.path, fallback.standardizedFileURL.path)
        XCTAssertTrue(FileManager.default.fileExists(atPath: folder.appendingPathComponent(".kestrel-snapshots").path))
    }

    func testMediaRootUncreatableFallsBack() throws {
        // The media folder's parent is unwritable, so `<media>/Snapshots` can't be created.
        let locked = tmp.appendingPathComponent("lockedParent")
        let media = locked.appendingPathComponent("Whimbrel")
        try FileManager.default.createDirectory(at: media, withIntermediateDirectories: true)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: media.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: media.path) }
        let fallback = tmp.appendingPathComponent("fallback")
        let s = SnapshotStorage(root: media.appendingPathComponent("Snapshots"), fallbackRoot: fallback)
        let folder = try s.makeMeetingFolder(start: start, title: "Acme")
        XCTAssertEqual(folder.deletingLastPathComponent().standardizedFileURL.path, fallback.standardizedFileURL.path)
    }

    func testMissingMediaFolderFallsBackWithoutCreatingIt() throws {
        // An unmounted drive: the media folder (the root's parent) doesn't exist. Nothing is
        // created at that path; the meeting folder lands under the fallback.
        let mountPoint = tmp.appendingPathComponent("mnt-drive")
        try FileManager.default.createDirectory(at: mountPoint, withIntermediateDirectories: true)
        let media = mountPoint.appendingPathComponent("Whimbrel")
        let fallback = tmp.appendingPathComponent("fallback")
        let s = SnapshotStorage(root: media.appendingPathComponent("Snapshots"), fallbackRoot: fallback)
        let folder = try s.makeMeetingFolder(start: start, title: "Acme")
        XCTAssertEqual(folder.deletingLastPathComponent().standardizedFileURL.path, fallback.standardizedFileURL.path)
        XCTAssertFalse(FileManager.default.fileExists(atPath: media.path))
    }

    func testFileNameCollision() {
        let t = Date(timeIntervalSince1970: 1_792_664_040)
        let base = SnapshotStorage.fileName(captured: t, existing: [])
        XCTAssertTrue(base.hasSuffix(".heic"))
        XCTAssertEqual(base.count, "HH.mm.ss.heic".count)
        let stem = String(base.dropLast(".heic".count))
        XCTAssertEqual(SnapshotStorage.fileName(captured: t, existing: [base]), "\(stem)-2.heic")
        XCTAssertEqual(SnapshotStorage.fileName(captured: t, existing: [base, "\(stem)-2.heic"]), "\(stem)-3.heic")
        // Any file of that stem takes the name: a PNG or thumbnail would share its `.thumb.jpg`.
        XCTAssertEqual(SnapshotStorage.fileName(captured: t, existing: ["\(stem).png"]), "\(stem)-2.heic")
        XCTAssertEqual(SnapshotStorage.fileName(captured: t, existing: ["\(stem).thumb.jpg"]), "\(stem)-2.heic")
        XCTAssertEqual(SnapshotStorage.fileName(captured: t, existing: [], ext: "png"), "\(stem).png")
    }

    private func blankImage(width: Int = 37, height: Int = 21) -> CGImage {
        CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                  space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!.makeImage()!
    }

    func testWriteHEICRoundTripsDimensions() throws {
        let folder = try storage().makeMeetingFolder(start: start, title: "Acme")
        let t = Date()
        let first = try storage().writeHEIC(blankImage(), to: folder, captured: t)
        let second = try storage().writeHEIC(blankImage(), to: folder, captured: t)
        XCTAssertNotEqual(first, second, "same-second captures do not overwrite")
        XCTAssertEqual(first.pathExtension, "heic")
        let src = CGImageSourceCreateWithURL(first as CFURL, nil)!
        XCTAssertEqual(CGImageSourceGetType(src) as String?, UTType.heic.identifier)
        let back = CGImageSourceCreateImageAtIndex(src, 0, nil)!
        XCTAssertEqual(back.width, 37)
        XCTAssertEqual(back.height, 21)
    }

    /// No HEIC encoder: the frame is still saved, as the PNG it always was.
    func testWriteHEICFallsBackToPNG() throws {
        SnapshotStorage.heicType = "com.example.no-such-type"
        defer { SnapshotStorage.heicType = UTType.heic.identifier }
        let folder = try storage().makeMeetingFolder(start: start, title: "Acme")
        let url = try storage().writeHEIC(blankImage(), to: folder, captured: Date())
        XCTAssertEqual(url.pathExtension, "png")
        let src = CGImageSourceCreateWithURL(url as CFURL, nil)!
        XCTAssertEqual(CGImageSourceGetType(src) as String?, UTType.png.identifier)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: folder.path).sorted(),
                       [SnapshotStorage.markerName, url.lastPathComponent].sorted())
    }

    @MainActor
    func testSettingsDefaultEnabled() {
        let suite = "SnapshotSettingsTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = SnapshotSettings(defaults: defaults)
        XCTAssertTrue(settings.enabled)
        settings.enabled = false
        let reloaded = SnapshotSettings(defaults: defaults)
        XCTAssertFalse(reloaded.enabled)
    }
}
