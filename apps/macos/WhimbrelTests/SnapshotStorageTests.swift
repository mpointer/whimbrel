import XCTest
import CoreGraphics
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

    func testFileNameCollision() {
        let t = Date(timeIntervalSince1970: 1_792_664_040)
        let base = SnapshotStorage.fileName(captured: t, existing: [])
        XCTAssertTrue(base.hasSuffix(".png"))
        XCTAssertEqual(base.count, "HH.mm.ss.png".count)
        XCTAssertEqual(SnapshotStorage.fileName(captured: t, existing: [base]), base.replacingOccurrences(of: ".png", with: "-2.png"))
        XCTAssertEqual(SnapshotStorage.fileName(captured: t, existing: [base, base.replacingOccurrences(of: ".png", with: "-2.png")]),
                       base.replacingOccurrences(of: ".png", with: "-3.png"))
    }

    func testWritePNGRoundTripsDimensions() throws {
        let ctx = CGContext(data: nil, width: 37, height: 21, bitsPerComponent: 8, bytesPerRow: 0,
                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        let image = ctx.makeImage()!
        let folder = try storage().makeMeetingFolder(start: start, title: "Acme")
        let t = Date()
        let first = try storage().writePNG(image, to: folder, captured: t)
        let second = try storage().writePNG(image, to: folder, captured: t)
        XCTAssertNotEqual(first, second, "same-second captures do not overwrite")
        let src = CGImageSourceCreateWithURL(first as CFURL, nil)!
        let back = CGImageSourceCreateImageAtIndex(src, 0, nil)!
        XCTAssertEqual(back.width, 37)
        XCTAssertEqual(back.height, 21)
    }

    @MainActor
    func testSettingsDefaultEnabledAndPicturesRoot() {
        let suite = "SnapshotSettingsTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = SnapshotSettings(defaults: defaults)
        XCTAssertTrue(settings.enabled)
        XCTAssertNil(settings.customRootPath)
        XCTAssertEqual(settings.rootURL.path, NSHomeDirectory() + "/Pictures/Whimbrel Snapshots")

        settings.customRootPath = "/tmp/elsewhere"
        settings.enabled = false
        let reloaded = SnapshotSettings(defaults: defaults)
        XCTAssertFalse(reloaded.enabled)
        XCTAssertEqual(reloaded.rootURL.path, "/tmp/elsewhere")
    }
}
