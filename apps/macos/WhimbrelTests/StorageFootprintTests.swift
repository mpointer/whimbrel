import XCTest
@testable import Whimbrel

final class StorageFootprintTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    private func write(_ path: String, _ bytes: Int) throws {
        let url = dir.appendingPathComponent(path)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(repeating: 7, count: bytes).write(to: url)
    }

    func testMeasureSplitsTheParts() throws {
        try write("data/core-memory.sqlite", 100_000)
        try write("data/core-memory.sqlite-wal", 50_000)
        try write("data/models/ggml.bin", 200_000)
        try write("data/screen-text.jsonl", 30_000)
        try write("data/skins/a.json", 10_000)
        try write("media/Audio/dev/m.m4a", 300_000)
        let f = StorageFootprint.measure(dataRoot: dir.appendingPathComponent("data"), mediaFolder: dir.appendingPathComponent("media"))
        let s = StorageFootprint.size(of:)
        XCTAssertEqual(f.database, s(dir.appendingPathComponent("data/core-memory.sqlite")) + s(dir.appendingPathComponent("data/core-memory.sqlite-wal")))
        XCTAssertEqual(f.models, s(dir.appendingPathComponent("data/models")))
        XCTAssertEqual(f.captureLog, s(dir.appendingPathComponent("data/screen-text.jsonl")))
        XCTAssertEqual(f.media, s(dir.appendingPathComponent("media")))
        XCTAssertEqual(f.other, s(dir.appendingPathComponent("data/skins")))
        XCTAssertEqual(f.total, f.database + f.models + f.captureLog + f.media + f.other)
        XCTAssertGreaterThanOrEqual(f.media, 300_000)
    }

    func testAMediaFolderInsideTheDataFolderCountsOnce() throws {
        try write("data/core-memory.sqlite", 100_000)
        try write("data/Media/Audio/m.m4a", 300_000)
        let f = StorageFootprint.measure(dataRoot: dir.appendingPathComponent("data"), mediaFolder: dir.appendingPathComponent("data/Media"))
        XCTAssertEqual(f.other, 0)
        XCTAssertEqual(f.media, StorageFootprint.size(of: dir.appendingPathComponent("data/Media")))
    }

    func testLowDiskReportsEachLowVolumeOnce() {
        let low = StorageFootprint.lowDiskVolumes(
            dataRoot: URL(fileURLWithPath: "/a"), mediaFolder: URL(fileURLWithPath: "/a/m"),
            free: { _ in 5_000_000_000 }, volumeID: { _ in "same" }, name: { _ in "Macintosh HD" })
        XCTAssertEqual(low.count, 1)
        XCTAssertEqual(low.first?.freeBytes, 5_000_000_000)
    }

    func testLowDiskChecksASeparateMediaVolume() {
        let low = StorageFootprint.lowDiskVolumes(
            dataRoot: URL(fileURLWithPath: "/a"), mediaFolder: URL(fileURLWithPath: "/Volumes/Ext/m"),
            free: { $0.path.hasPrefix("/Volumes") ? 1_000_000_000 : 500_000_000_000 },
            volumeID: { $0.path.hasPrefix("/Volumes") ? "ext" : "boot" }, name: { $0.path.hasPrefix("/Volumes") ? "Ext" : "HD" })
        XCTAssertEqual(low.map(\.name), ["Ext"])
    }

    func testNoWarningAtOrAboveThreshold() {
        let low = StorageFootprint.lowDiskVolumes(
            dataRoot: URL(fileURLWithPath: "/a"), mediaFolder: URL(fileURLWithPath: "/b"),
            free: { _ in StorageFootprint.lowDiskThreshold }, volumeID: { $0.path }, name: { $0.path })
        XCTAssertTrue(low.isEmpty)
    }
}
