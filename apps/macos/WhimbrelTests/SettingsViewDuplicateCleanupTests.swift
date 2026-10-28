import XCTest
@testable import Whimbrel

/// `SettingsView.cleanUpDuplicateMeetings()` is a `private func` on a SwiftUI View
/// (file-scoped, and it touches `@EnvironmentObject var appState`), so it can't be
/// invoked directly from a test even with `@testable import`. Instead this exercises,
/// with a real `CoreMemoryStore` and real files on disk, the exact per-duplicate-id
/// operation the function performs: a plain `store.deleteMeeting(id:)`, with NO file
/// cleanup.
///
/// That absence is the point. Duplicate groups are peer-sourced only
/// (`find_duplicate_meeting_groups` filters `device_id != own_device_id`), and
/// `apply_synced_meeting` always inserts `audio_path` as NULL, so no local audio file
/// can be orphaned on this path. `transcript_path` can be non-null, but every member
/// of a group is the same sync payload applied repeatedly — they share one path
/// string, so cleaning up files for a dropped duplicate would delete the file the kept
/// row still references. These tests document that hazard against a real store. They
/// exercise `deleteMeeting` directly, not the private production loop, so they cannot
/// by themselves fail if file cleanup is re-added there -- the comment in
/// `cleanUpDuplicateMeetings` is the actual guard.
final class SettingsViewDuplicateCleanupTests: XCTestCase {
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

    private func makeStore() -> CoreMemoryStore {
        CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    /// Mirrors what a peer-synced meeting actually looks like locally: no audio path
    /// (`apply_synced_meeting` writes NULL), and a `transcript_path` that every copy in
    /// the duplicate group shares because they are the same payload applied twice.
    private func makeSyncedDuplicate(store: CoreMemoryStore, sharedTranscriptPath: String) -> Int64 {
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "peer-device", startUnixMs: 1000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .manual, audioPath: nil, status: .completed
        ))!
        XCTAssertTrue(store.setMeetingTranscript(id: id, transcriptPath: sharedTranscriptPath, notes: nil))
        return id
    }

    func testDroppedDuplicateRowIsDeletedAndSharedTranscriptFileSurvives() {
        let store = makeStore()
        let transcriptURL = tempDir.appendingPathComponent("shared-transcript.txt")
        try! Data("transcript".utf8).write(to: transcriptURL)

        let droppedId = makeSyncedDuplicate(store: store, sharedTranscriptPath: transcriptURL.path)
        let keptId = makeSyncedDuplicate(store: store, sharedTranscriptPath: transcriptURL.path)

        // Exactly the body of the loop in cleanUpDuplicateMeetings, for the one id a
        // real DuplicateGroup's `meetingIds.dropLast()` would yield.
        XCTAssertTrue(store.deleteMeeting(id: droppedId))

        XCTAssertNil(store.getMeeting(id: droppedId), "dropped duplicate's DB row must be gone")
        XCTAssertNotNil(store.getMeeting(id: keptId), "kept duplicate must survive")
        XCTAssertTrue(FileManager.default.fileExists(atPath: transcriptURL.path),
                      "the transcript file is shared with the kept row — cleaning it up here would destroy the kept row's transcript")
        XCTAssertEqual(store.getMeeting(id: keptId)?.transcriptPath, transcriptURL.path)
    }

    /// The "orphaned audio file" premise does not apply to this path: peer-synced
    /// meetings never carry a local audio path, so there is nothing to orphan.
    func testSyncedDuplicateHasNoLocalAudioPathToOrphan() {
        let store = makeStore()
        let id = makeSyncedDuplicate(store: store, sharedTranscriptPath: tempDir.appendingPathComponent("t.txt").path)
        XCTAssertNil(store.getMeeting(id: id)?.audioPath)
    }
}
