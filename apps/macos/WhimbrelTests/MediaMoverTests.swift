import SQLite3
import XCTest
@testable import Whimbrel

/// Copy, verify, rewrite paths, then delete: the media move. Everything runs in a temp
/// folder with a temp store.
final class MediaMoverTests: XCTestCase {
    private var tempDir: URL!
    private var recordings: URL!
    private var media: URL!
    private var store: CoreMemoryStore!
    private let fm = FileManager.default

    override func setUp() {
        super.setUp()
        tempDir = fm.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        recordings = tempDir.appendingPathComponent("recordings", isDirectory: true)
        media = tempDir.appendingPathComponent("media", isDirectory: true)
        try! fm.createDirectory(at: recordings, withIntermediateDirectories: true)
        try! fm.createDirectory(at: media, withIntermediateDirectories: true)
        store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    override func tearDown() {
        MediaMover.copyItem = { try FileManager.default.copyItem(at: $0, to: $1) }
        MediaMover.beforePathUpdate = {}
        store = nil
        try? fm.removeItem(at: tempDir)
        tempDir = nil
        super.tearDown()
    }

    @discardableResult
    private func write(_ url: URL, _ text: String) -> URL {
        try! Data(text.utf8).write(to: url)
        return url
    }

    private func contents(_ url: URL) -> [String] {
        ((try? fm.contentsOfDirectory(atPath: url.path)) ?? []).sorted()
    }

    /// A continued meeting: two parts, each with a system/mic pair in `recordings`.
    private func makeTwoPartMeeting() -> (id: Int64, paths: [String], files: [URL]) {
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .manual, audioPath: nil, status: .recording
        ))!
        var paths: [String] = [], files: [URL] = []
        for (i, stamp) in ["A", "B"].enumerated() {
            let sys = write(recordings.appendingPathComponent("system-audio-\(stamp).caf"), "sys \(stamp)")
            let mic = write(recordings.appendingPathComponent("mic-audio-\(stamp).caf"), "mic \(stamp) longer")
            files += [sys, mic]
            paths.append(sys.path)
            if i == 1 { XCTAssertTrue(store.reopenMeeting(id: id)) }
            let part = store.beginMeetingPart(meetingId: id, startUnixMs: Int64(1_000 + i * 100_000), continueKey: nil)!
            XCTAssertTrue(store.completeMeetingPart(meetingId: id, partIndex: part, endUnixMs: Int64(60_000 + i * 100_000), audioPath: sys.path))
            XCTAssertTrue(store.completeMeeting(id: id, endUnixMs: Int64(60_000 + i * 100_000), audioPath: sys.path))
        }
        return (id, paths, files)
    }

    private func storedAudioPaths(_ id: Int64) -> [String?] {
        [store.getMeeting(id: id)?.audioPath] + (store.meetingParts(meetingId: id) ?? []).map(\.audioPath)
    }

    // MARK: copyVerified

    func testCopyVerifiedRefusesADifferentFileAtTheDestination() throws {
        let src = write(recordings.appendingPathComponent("system-audio-A.caf"), "the real recording")
        let dst = media.appendingPathComponent("system-audio-A.caf")
        write(dst, "short")
        XCTAssertThrowsError(try MediaMover.copyVerified(src, to: dst))
        XCTAssertEqual(try String(contentsOf: dst), "short", "someone else's file is never replaced")
        XCTAssertEqual(contents(media), ["system-audio-A.caf"])
        XCTAssertTrue(fm.fileExists(atPath: src.path))
    }

    func testCopyVerifiedWritesThroughAHiddenPartialThenRenames() throws {
        let src = write(recordings.appendingPathComponent("system-audio-A.caf"), "the real recording")
        let dst = media.appendingPathComponent("system-audio-A.caf")
        var copiedTo: URL?
        MediaMover.copyItem = { from, to in copiedTo = to; try FileManager.default.copyItem(at: from, to: to) }
        XCTAssertTrue(try MediaMover.copyVerified(src, to: dst), "created by this call")
        XCTAssertEqual(copiedTo?.lastPathComponent, ".system-audio-A.caf.partial")
        XCTAssertEqual(contents(media), ["system-audio-A.caf"])
        XCTAssertEqual(try Data(contentsOf: dst), try Data(contentsOf: src))
        XCTAssertFalse(try MediaMover.copyVerified(src, to: dst), "an identical file was already there")
    }

    func testCopyVerifiedRemovesACopyThatFailsVerify() {
        let src = write(recordings.appendingPathComponent("system-audio-A.caf"), "the real recording")
        let dst = media.appendingPathComponent("system-audio-A.caf")
        MediaMover.copyItem = { _, to in try Data("the real recordinG".utf8).write(to: to) }  // same size, other bytes
        XCTAssertThrowsError(try MediaMover.copyVerified(src, to: dst))
        XCTAssertEqual(contents(media), [])
        XCTAssertEqual(try Data(contentsOf: src), Data("the real recording".utf8))
    }

    func testCopyVerifiedIntoAMissingFolderThrows() {
        let src = write(recordings.appendingPathComponent("system-audio-A.caf"), "x")
        XCTAssertThrowsError(try MediaMover.copyVerified(src, to: tempDir.appendingPathComponent("gone/system-audio-A.caf")))
        XCTAssertTrue(fm.fileExists(atPath: src.path))
    }

    // MARK: moveAudio

    func testMoveAudioMovesBothTracksOfEveryPartAndRewritesPaths() {
        let m = makeTwoPartMeeting()
        let audioDir = media.appendingPathComponent("Audio/dev-1", isDirectory: true)
        let before = storedAudioPaths(m.id)
        XCTAssertTrue(MediaMover.moveAudio(meetingId: m.id, audioPaths: m.paths, to: audioDir, store: store))

        XCTAssertEqual(storedAudioPaths(m.id), before.map { audioDir.appendingPathComponent(URL(fileURLWithPath: $0!).lastPathComponent).path })
        XCTAssertEqual(store.meetingParts(meetingId: m.id)?.map(\.audioPath), [
            audioDir.appendingPathComponent("system-audio-A.caf").path,
            audioDir.appendingPathComponent("system-audio-B.caf").path,
        ])
        XCTAssertEqual(contents(audioDir), ["mic-audio-A.caf", "mic-audio-B.caf", "system-audio-A.caf", "system-audio-B.caf"])
        XCTAssertEqual(try String(contentsOf: audioDir.appendingPathComponent("mic-audio-B.caf")), "mic B longer")
        for f in m.files { XCTAssertFalse(fm.fileExists(atPath: f.path), f.path) }
    }

    func testMoveAudioIntoAMissingDriveLeavesOriginals() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        let drive = tempDir.appendingPathComponent("Drive", isDirectory: true)
        try! fm.createDirectory(at: drive, withIntermediateDirectories: true)
        try! fm.removeItem(at: drive)
        let audioDir = drive.appendingPathComponent("Whimbrel/Audio/dev-1", isDirectory: true)

        XCTAssertFalse(MediaMover.moveAudio(meetingId: m.id, audioPaths: m.paths, to: audioDir, store: store))
        XCTAssertFalse(fm.fileExists(atPath: drive.path), "nothing may be written where the drive was")
        XCTAssertEqual(storedAudioPaths(m.id), before)
        for f in m.files { XCTAssertTrue(fm.fileExists(atPath: f.path), f.path) }
    }

    func testMoveAudioFailedVerifyRemovesCopiesAndKeepsOriginals() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        let audioDir = media.appendingPathComponent("Audio/dev-1", isDirectory: true)
        var calls = 0
        MediaMover.copyItem = { from, to in
            calls += 1
            if calls == 3 { try Data("corrupt".utf8).write(to: to) } else { try FileManager.default.copyItem(at: from, to: to) }
        }

        XCTAssertFalse(MediaMover.moveAudio(meetingId: m.id, audioPaths: m.paths, to: audioDir, store: store))
        XCTAssertEqual(contents(audioDir), [], "every copy made so far is removed")
        XCTAssertEqual(storedAudioPaths(m.id), before)
        for f in m.files { XCTAssertTrue(fm.fileExists(atPath: f.path), f.path) }
    }

    func testMoveAudioFailedPathUpdateKeepsOriginals() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        // A path the store doesn't hold for this meeting: the update rolls back and fails.
        let stray = write(recordings.appendingPathComponent("system-audio-Z.caf"), "stray")
        let audioDir = media.appendingPathComponent("Audio/dev-1", isDirectory: true)

        XCTAssertFalse(MediaMover.moveAudio(meetingId: m.id, audioPaths: m.paths + [stray.path], to: audioDir, store: store))
        XCTAssertEqual(storedAudioPaths(m.id), before)
        XCTAssertEqual(contents(audioDir), [])
        for f in m.files + [stray] { XCTAssertTrue(fm.fileExists(atPath: f.path), f.path) }
    }

    func testMoveAudioNotConfirmedDurableDeletesNothing() {
        let m = makeTwoPartMeeting()
        let audioDir = media.appendingPathComponent("Audio/dev-1", isDirectory: true)
        var reader: OpaquePointer?
        MediaMover.beforePathUpdate = { reader = holdReadSnapshot(self.tempDir.appendingPathComponent("test.sqlite")) }

        XCTAssertFalse(MediaMover.moveAudio(meetingId: m.id, audioPaths: m.paths, to: audioDir, store: store))
        XCTAssertNotNil(reader)
        sqlite3_close_v2(reader)
        // Committed but unconfirmed: keep the originals and the copies.
        for f in m.files {
            XCTAssertTrue(fm.fileExists(atPath: f.path), f.path)
            XCTAssertTrue(fm.fileExists(atPath: audioDir.appendingPathComponent(f.lastPathComponent).path))
        }
    }

    func testMoveAudioOverAnIdenticalExistingCopySucceeds() throws {
        let m = makeTwoPartMeeting()
        let audioDir = media.appendingPathComponent("Audio/dev-1", isDirectory: true)
        try fm.createDirectory(at: audioDir, withIntermediateDirectories: true)
        // Left behind by a run that stopped after copying.
        try fm.copyItem(at: m.files[0], to: audioDir.appendingPathComponent(m.files[0].lastPathComponent))

        XCTAssertTrue(MediaMover.moveAudio(meetingId: m.id, audioPaths: m.paths, to: audioDir, store: store))
        XCTAssertEqual(contents(audioDir).count, 4)
        XCTAssertEqual(try String(contentsOf: audioDir.appendingPathComponent("system-audio-A.caf")), "sys A")
        for f in m.files { XCTAssertFalse(fm.fileExists(atPath: f.path), f.path) }
    }

    func testMoveAudioRefusesWhenADifferentFileHoldsTheName() throws {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        let audioDir = media.appendingPathComponent("Audio/dev-1", isDirectory: true)
        try fm.createDirectory(at: audioDir, withIntermediateDirectories: true)
        let squatter = write(audioDir.appendingPathComponent("mic-audio-B.caf"), "not ours")

        XCTAssertFalse(MediaMover.moveAudio(meetingId: m.id, audioPaths: m.paths, to: audioDir, store: store))
        XCTAssertEqual(contents(audioDir), ["mic-audio-B.caf"], "copies removed, the other file kept")
        XCTAssertEqual(try String(contentsOf: squatter), "not ours")
        XCTAssertEqual(storedAudioPaths(m.id), before)
        for f in m.files { XCTAssertTrue(fm.fileExists(atPath: f.path), f.path) }
    }

    func testMoveAudioRefusesTwoSourcesWithOneName() throws {
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .manual, audioPath: nil, status: .recording
        ))!
        var files: [URL] = []
        for (i, sub) in ["x", "y"].enumerated() {
            let dir = recordings.appendingPathComponent(sub, isDirectory: true)
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            // Identical bytes, so only the name guard (not the destination check) can refuse.
            let f = write(dir.appendingPathComponent("system-audio-S.caf"), "take")
            files.append(f)
            if i == 1 { XCTAssertTrue(store.reopenMeeting(id: id)) }
            let part = store.beginMeetingPart(meetingId: id, startUnixMs: Int64(1_000 + i * 100_000), continueKey: nil)!
            XCTAssertTrue(store.completeMeetingPart(meetingId: id, partIndex: part, endUnixMs: Int64(60_000 + i * 100_000), audioPath: f.path))
            XCTAssertTrue(store.completeMeeting(id: id, endUnixMs: Int64(60_000 + i * 100_000), audioPath: f.path))
        }
        let audioDir = media.appendingPathComponent("Audio/dev-1", isDirectory: true)
        XCTAssertFalse(MediaMover.moveAudio(meetingId: id, audioPaths: files.map(\.path), to: audioDir, store: store))
        XCTAssertEqual(contents(audioDir), [])
        for f in files { XCTAssertTrue(fm.fileExists(atPath: f.path), f.path) }
    }

    func testMoveAudioAlreadyInTheAudioFolderDeletesNothing() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        XCTAssertTrue(MediaMover.moveAudio(meetingId: m.id, audioPaths: m.paths, to: recordings, store: store))
        XCTAssertEqual(storedAudioPaths(m.id), before)
        for f in m.files { XCTAssertTrue(fm.fileExists(atPath: f.path), f.path) }
    }

    // MARK: moveSnapshots

    private func makeCompletedMeeting(_ title: String) -> Int64 {
        store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: 2_000, title: title,
            attendees: [], trigger: .manual, audioPath: nil, status: .completed
        ))!
    }

    /// A marked snapshot folder in the old Snapshots root.
    private func makeOldFolder() -> URL {
        try! SnapshotStorage.create("2026-10-01 10.00 Standup", under: recordings.appendingPathComponent("Snapshots"))
    }

    private func addSnapshot(_ meeting: Int64, _ folder: URL, _ name: String) -> URL {
        let file = write(folder.appendingPathComponent(name), "png \(name)")
        XCTAssertNotNil(store.insertMeetingSnapshot(meetingId: meeting, capturedUnixMs: 5, filePath: file.path, width: 1, height: 1, ocrText: ""))
        store.setMeetingSnapshotsDir(meetingId: meeting, dir: folder.path)
        return file
    }

    func testMoveSnapshotsOfOneMeetingInASharedFolderLeavesTheOther() {
        let a = makeCompletedMeeting("Standup"), b = makeCompletedMeeting("Standup part 2")
        let old = makeOldFolder()
        let aFile = addSnapshot(a, old, "10.00.01.png")
        let bFile = addSnapshot(b, old, "10.30.01.png")
        let root = media.appendingPathComponent("Snapshots", isDirectory: true)

        XCTAssertTrue(MediaMover.moveSnapshots(meetingId: a, to: root, store: store))

        let newFolder = root.appendingPathComponent(old.lastPathComponent)
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: a).map(\.filePath), [newFolder.appendingPathComponent("10.00.01.png").path])
        XCTAssertEqual(store.meetingSnapshotsInfo(meetingId: a)?.dir, newFolder.path)
        XCTAssertEqual(contents(newFolder), [SnapshotStorage.markerName, "10.00.01.png"].sorted())
        XCTAssertFalse(fm.fileExists(atPath: aFile.path))
        XCTAssertTrue(fm.fileExists(atPath: bFile.path))
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: b).map(\.filePath), [bFile.path])
        XCTAssertEqual(store.meetingSnapshotsInfo(meetingId: b)?.dir, old.path)
    }

    func testMoveSnapshotsRemovesTheEmptiedOldFolder() throws {
        let a = makeCompletedMeeting("Standup")
        let old = makeOldFolder()
        _ = addSnapshot(a, old, "10.00.01.png")
        _ = addSnapshot(a, old, "10.00.02.png")
        write(old.appendingPathComponent(".DS_Store"), "")
        let root = media.appendingPathComponent("Snapshots", isDirectory: true)

        XCTAssertTrue(MediaMover.moveSnapshots(meetingId: a, to: root, store: store))
        XCTAssertFalse(fm.fileExists(atPath: old.path))
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: a).count, 2)
        for s in store.listMeetingSnapshots(meetingId: a) { XCTAssertTrue(fm.fileExists(atPath: s.filePath)) }
    }

    func testMoveSnapshotsKeepsAnOldFolderWithAUserFile() {
        let a = makeCompletedMeeting("Standup")
        let old = makeOldFolder()
        _ = addSnapshot(a, old, "10.00.01.png")
        let mine = write(old.appendingPathComponent("my notes.txt"), "mine")
        XCTAssertTrue(MediaMover.moveSnapshots(meetingId: a, to: media.appendingPathComponent("Snapshots"), store: store))
        XCTAssertTrue(fm.fileExists(atPath: mine.path))
    }

    func testMoveSnapshotsIntoAMissingDriveLeavesEverything() {
        let a = makeCompletedMeeting("Standup")
        let old = makeOldFolder()
        let file = addSnapshot(a, old, "10.00.01.png")
        let drive = tempDir.appendingPathComponent("Drive", isDirectory: true)

        XCTAssertFalse(MediaMover.moveSnapshots(meetingId: a, to: drive.appendingPathComponent("Whimbrel/Snapshots"), store: store))
        XCTAssertFalse(fm.fileExists(atPath: drive.path))
        XCTAssertTrue(fm.fileExists(atPath: file.path))
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: a).map(\.filePath), [file.path])
    }

    func testMoveSnapshotsFailedVerifyRemovesTheNewFolder() {
        let a = makeCompletedMeeting("Standup")
        let old = makeOldFolder()
        let file = addSnapshot(a, old, "10.00.01.png")
        let root = media.appendingPathComponent("Snapshots", isDirectory: true)
        MediaMover.copyItem = { _, to in try Data("bad".utf8).write(to: to) }

        XCTAssertFalse(MediaMover.moveSnapshots(meetingId: a, to: root, store: store))
        XCTAssertEqual(contents(root), [], "no marker-only folder is left behind")
        XCTAssertTrue(fm.fileExists(atPath: file.path))
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: a).map(\.filePath), [file.path])
    }

    func testMoveSnapshotsFailedPathUpdateKeepsTheSources() {
        let a = makeCompletedMeeting("Standup")
        let old = makeOldFolder()
        let file = addSnapshot(a, old, "10.00.01.png")
        let root = media.appendingPathComponent("Snapshots", isDirectory: true)
        // The row goes away between the copy and the rewrite: the update matches nothing.
        MediaMover.beforePathUpdate = { [store] in
            for s in store!.listMeetingSnapshots(meetingId: a) { store!.deleteMeetingSnapshot(id: s.id) }
        }

        XCTAssertFalse(MediaMover.moveSnapshots(meetingId: a, to: root, store: store))
        XCTAssertTrue(fm.fileExists(atPath: file.path))
        XCTAssertEqual(contents(root), [])
        XCTAssertEqual(store.meetingSnapshotsInfo(meetingId: a)?.dir, old.path)
    }

    func testMoveSnapshotsNotConfirmedDurableDeletesNothing() {
        let a = makeCompletedMeeting("Standup")
        let old = makeOldFolder()
        let file = addSnapshot(a, old, "10.00.01.png")
        let root = media.appendingPathComponent("Snapshots", isDirectory: true)
        var reader: OpaquePointer?
        MediaMover.beforePathUpdate = { reader = holdReadSnapshot(self.tempDir.appendingPathComponent("test.sqlite")) }

        XCTAssertFalse(MediaMover.moveSnapshots(meetingId: a, to: root, store: store))
        XCTAssertNotNil(reader)
        sqlite3_close_v2(reader)
        // Committed but unconfirmed: the original and the new folder's copy both stay.
        XCTAssertTrue(fm.fileExists(atPath: file.path))
        let newPath = store.listMeetingSnapshots(meetingId: a).first!.filePath
        XCTAssertNotEqual(newPath, file.path)
        XCTAssertTrue(fm.fileExists(atPath: newPath))
    }

    // MARK: Sources gone on a mounted volume, or on an unplugged drive

    /// Part B's stored path points into `folder`, and B's files are gone.
    private func makeMeetingWithPartBAt(_ folder: String) -> (id: Int64, a: String, b: String) {
        let m = makeTwoPartMeeting()
        let b = URL(fileURLWithPath: folder).appendingPathComponent("system-audio-B.caf").path
        XCTAssertTrue(store.setMeetingAudioPaths(meetingId: m.id, moves: [(old: m.paths[1], new: b)]))
        for f in m.files where f.lastPathComponent.hasSuffix("-B.caf") { try! fm.removeItem(at: f) }
        return (m.id, m.paths[0], b)
    }

    func testMoveAudioMovesTheRestWhenAPartsFolderWasDeletedOnAMountedVolume() {
        let m = makeMeetingWithPartBAt(tempDir.appendingPathComponent("deleted-in-finder").path)
        let audioDir = media.appendingPathComponent("Audio/dev-1", isDirectory: true)
        XCTAssertTrue(MediaMover.moveAudio(meetingId: m.id, audioPaths: [m.a, m.b], to: audioDir, store: store))
        XCTAssertEqual(contents(audioDir), ["mic-audio-A.caf", "system-audio-A.caf"])
        XCTAssertEqual(store.meetingParts(meetingId: m.id)?.map(\.audioPath),
                       [audioDir.appendingPathComponent("system-audio-A.caf").path, m.b], "the gone part's path is left as it was")
    }

    func testMoveAudioFromAnUnpluggedDriveFails() {
        let m = makeMeetingWithPartBAt("/Volumes/Whimbrel-Test-Gone/Recordings")
        let audioDir = media.appendingPathComponent("Audio/dev-1", isDirectory: true)
        XCTAssertFalse(MediaMover.moveAudio(meetingId: m.id, audioPaths: [m.a, m.b], to: audioDir, store: store))
        XCTAssertTrue(fm.fileExists(atPath: m.a))
        XCTAssertEqual(store.meetingParts(meetingId: m.id)?.map(\.audioPath), [m.a, m.b])
    }

    func testMoveSnapshotsWithOneFileDeletedInFinderMovesTheRest() {
        // The listing drops a row whose file is gone from an existing folder, so the move
        // never sees it.
        let a = makeCompletedMeeting("Standup")
        let old = makeOldFolder()
        let kept = addSnapshot(a, old, "10.00.01.png")
        let deleted = addSnapshot(a, old, "10.00.02.png")
        try! fm.removeItem(at: deleted)
        let root = media.appendingPathComponent("Snapshots", isDirectory: true)

        XCTAssertTrue(MediaMover.moveSnapshots(meetingId: a, to: root, store: store))
        let newFolder = root.appendingPathComponent(old.lastPathComponent)
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: a).map(\.filePath), [newFolder.appendingPathComponent("10.00.01.png").path])
        XCTAssertEqual(contents(newFolder), [SnapshotStorage.markerName, "10.00.01.png"].sorted())
        XCTAssertFalse(fm.fileExists(atPath: kept.path))
        XCTAssertFalse(fm.fileExists(atPath: old.path), "the emptied old folder goes")
    }

    func testMoveSnapshotsLeavesARowWhoseFolderWasDeletedOnAMountedVolume() {
        let a = makeCompletedMeeting("Standup")
        let old = makeOldFolder()
        _ = addSnapshot(a, old, "10.00.01.png")
        let gone = tempDir.appendingPathComponent("deleted-in-finder/10.00.09.png").path
        XCTAssertNotNil(store.insertMeetingSnapshot(meetingId: a, capturedUnixMs: 9, filePath: gone, width: 1, height: 1, ocrText: ""))
        let root = media.appendingPathComponent("Snapshots", isDirectory: true)

        XCTAssertTrue(MediaMover.moveSnapshots(meetingId: a, to: root, store: store))
        let paths = store.listMeetingSnapshots(meetingId: a).map(\.filePath)
        XCTAssertTrue(paths.contains(gone))
        XCTAssertTrue(paths.contains(root.appendingPathComponent(old.lastPathComponent).appendingPathComponent("10.00.01.png").path))
    }

    func testMoveSnapshotsFromAnUnpluggedDriveFails() {
        let a = makeCompletedMeeting("Standup")
        let old = makeOldFolder()
        let file = addSnapshot(a, old, "10.00.01.png")
        let gone = "/Volumes/Whimbrel-Test-Gone/Snapshots/m/10.00.09.png"
        XCTAssertNotNil(store.insertMeetingSnapshot(meetingId: a, capturedUnixMs: 9, filePath: gone, width: 1, height: 1, ocrText: ""))

        XCTAssertFalse(MediaMover.moveSnapshots(meetingId: a, to: media.appendingPathComponent("Snapshots"), store: store))
        XCTAssertTrue(fm.fileExists(atPath: file.path))
        XCTAssertEqual(Set(store.listMeetingSnapshots(meetingId: a).map(\.filePath)), [file.path, gone])
    }

    // MARK: Delete audio is off until turned on (real planner)

    @MainActor
    func testDefaultSettingsPlanNoDeleteUntilTurnedOn() {
        let m = makeTwoPartMeeting()
        XCTAssertTrue(store.setMeetingPartTranscript(meetingId: m.id, partIndex: 1, transcriptPath: "/t1.txt"))
        XCTAssertTrue(store.setMeetingPartTranscript(meetingId: m.id, partIndex: 2, transcriptPath: "/t2.txt"))
        let suite = "MediaMoverTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = MediaSettings(defaults: defaults)
        let now: Int64 = 200 * 86_400_000

        let off = store.mediaPlan(policy: settings.policy, nowUnixMs: now, mediaRoot: media, busy: [])
        XCTAssertFalse(off?.work.first { $0.meetingId == m.id }?.steps.contains(.deleteAudio) ?? true)
        XCTAssertGreaterThan(off?.usage.audioDeletableBytes ?? 0, 0, "Settings can show what turning it on frees")

        settings.neverDeleteAudio = false
        let on = store.mediaPlan(policy: settings.policy, nowUnixMs: now, mediaRoot: media, busy: [])
        XCTAssertTrue(on?.work.first { $0.meetingId == m.id }?.steps.contains(.deleteAudio) ?? false)
    }

    // MARK: Bridge

    func testSetMeetingAudioPathsRejectsAnUnknownPath() {
        let m = makeTwoPartMeeting()
        XCTAssertFalse(store.setMeetingAudioPaths(meetingId: m.id, moves: [(old: "/nope/system-audio-Q.caf", new: "/x/system-audio-Q.caf")]))
        XCTAssertTrue(store.setMeetingAudioPaths(meetingId: m.id, moves: [(old: m.paths[0], new: "/x/system-audio-A.caf")]))
        XCTAssertEqual(store.meetingParts(meetingId: m.id)?.first?.audioPath, "/x/system-audio-A.caf")
    }

    func testMediaPlanDecodesAMoveForAudioOutsideTheMediaFolder() {
        let m = makeTwoPartMeeting()
        let plan = store.mediaPlan(policy: CoreMediaPolicy(), nowUnixMs: 10_000_000, mediaRoot: media, busy: [])
        XCTAssertNotNil(plan)
        let work = plan?.work.first { $0.meetingId == m.id }
        XCTAssertEqual(work?.steps, [.moveAudio])
        XCTAssertEqual(Set(work?.audioPaths ?? []), Set(m.paths))
        XCTAssertGreaterThan(plan?.usage.audioFullBytes ?? 0, 0)
        XCTAssertNil(store.mediaPlan(policy: CoreMediaPolicy(), nowUnixMs: 10_000_000, mediaRoot: media, busy: [m.id])?.work.first { $0.meetingId == m.id })
    }
}
