import XCTest
@testable import Whimbrel

/// Smoke tests for the CoreBridge wrappers over core/api::retention. The rules
/// themselves (sibling naming, orphan detection, sweep-then-delete) are tested in Rust
/// (`core/api/src/retention.rs`); these prove the Swift path/JSON plumbing reaches
/// the same files the Rust side expects.
final class RetentionBridgeTests: XCTestCase {
    private var tempDir: URL!

    override func setUp() {
        super.setUp()
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try! FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: tempDir)
        tempDir = nil
        super.tearDown()
    }

    private func touch(_ name: String) -> URL {
        let url = tempDir.appendingPathComponent(name)
        try! Data(name.utf8).write(to: url)
        return url
    }

    func testDeleteMeetingFilesRemovesBothSiblingsAndTranscriptThroughTheBridge() {
        let system = touch("system-audio-STAMP.caf")
        let mic = touch("mic-audio-STAMP.caf")
        let transcript = touch("transcript-STAMP.txt")

        CoreMemoryStore.deleteMeetingFiles(audioPath: system.path, transcriptPath: transcript.path, snapshotsDir: nil, recordingsDir: tempDir)

        XCTAssertFalse(FileManager.default.fileExists(atPath: system.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: mic.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: transcript.path))
        // nil paths and already-missing files are tolerated.
        CoreMemoryStore.deleteMeetingFiles(audioPath: nil, transcriptPath: transcript.path, snapshotsDir: nil, recordingsDir: tempDir)
    }

    func testScanAndDeleteOrphansThroughTheBridge() {
        let store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
        let orphan = touch("system-audio-ORPHAN.caf")
        let live = touch("mic-audio-LIVE.caf")
        // The core sweep spares files modified in the last 7 days.
        try! FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(-8 * 86_400)], ofItemAtPath: orphan.path)

        guard let scan = store.scanOrphanedRecordings(recordingsDir: tempDir, excluding: [live]) else {
            return XCTFail("scan returned nil")
        }
        XCTAssertEqual(scan.orphaned, [orphan.path])
        XCTAssertEqual(scan.totalBytes, Int64("system-audio-ORPHAN.caf".utf8.count))

        XCTAssertEqual(CoreMemoryStore.deleteOrphanedRecordings(scan.orphaned), 1)
        XCTAssertFalse(FileManager.default.fileExists(atPath: orphan.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: live.path))
    }

    func testApplyRetentionOnAnEmptyStoreReportsZeroDeletions() {
        let store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
        let result = store.applyRetention(captureItemDays: 30, meetingDays: 90, conversationDays: 30, nowUnixMs: 1_000_000, recordingsDir: tempDir)
        XCTAssertEqual(result?.meetingsDeleted, 0)
        XCTAssertEqual(result?.captureItemsDeleted, 0)
        XCTAssertEqual(result?.conversationsDeleted, 0)
        XCTAssertEqual(result?.deletedMeetingFiles.count, 0)
    }
}
