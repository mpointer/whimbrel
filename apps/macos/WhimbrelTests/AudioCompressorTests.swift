import AVFoundation
import SQLite3
import XCTest
@testable import Whimbrel

/// Opens a second, read-only connection to the store at `db` and starts a read
/// transaction, which pins the WAL frames it can see: a FULL checkpoint after a later
/// commit then can't finish, as if another reader were busy. Close the handle to release.
func holdReadSnapshot(_ db: URL) -> OpaquePointer? {
    var h: OpaquePointer?
    guard sqlite3_open_v2(db.path, &h, SQLITE_OPEN_READONLY, nil) == SQLITE_OK,
          sqlite3_exec(h, "BEGIN; SELECT count(*) FROM meeting;", nil, nil, nil) == SQLITE_OK else { return nil }
    return h
}

/// Compress a meeting's tracks to 32 kbps mono AAC: temps first, verify, rename, rewrite
/// the stored paths, then delete the originals. Everything runs in a temp folder with a
/// temp store.
final class AudioCompressorTests: XCTestCase {
    private var tempDir: URL!
    private var recordings: URL!
    private var store: CoreMemoryStore!
    private let fm = FileManager.default

    override func setUp() {
        super.setUp()
        tempDir = fm.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        recordings = tempDir.appendingPathComponent("recordings", isDirectory: true)
        try! fm.createDirectory(at: recordings, withIntermediateDirectories: true)
        store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    override func tearDown() {
        AudioCompressor.encode = AudioCompressor.compress
        AudioCompressor.beforePathUpdate = {}
        try? fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: recordings.path)
        store = nil
        try? fm.removeItem(at: tempDir)
        tempDir = nil
        super.tearDown()
    }

    /// A track as the recorder writes it (16 kHz unless `rate` says otherwise): `seconds`
    /// of a 440 Hz tone (0 = header only).
    @discardableResult
    private func writeTrack(_ url: URL, seconds: Double, rate: Double = 16_000) -> URL {
        var settings = RecordingWriter.fileSettings
        settings[AVSampleRateKey] = rate
        let file = try! AVAudioFile(forWriting: url, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        let frames = AVAudioFrameCount(seconds * rate)
        if frames > 0 {
            let buf = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: frames)!
            buf.frameLength = frames
            for i in 0..<Int(frames) { buf.floatChannelData![0][i] = 0.3 * sin(Float(i) * 2 * .pi * 440 / Float(rate)) }
            try! file.write(from: buf)
        }
        return url
    }

    private func m4a(_ url: URL) -> URL { url.deletingPathExtension().appendingPathExtension("m4a") }

    private func contents(_ url: URL) -> [String] {
        ((try? fm.contentsOfDirectory(atPath: url.path)) ?? []).sorted()
    }

    private func seconds(_ url: URL) -> Double {
        guard let f = try? AVAudioFile(forReading: url) else { return -1 }
        return Double(f.length) / f.fileFormat.sampleRate
    }

    /// A continued meeting: two parts, each with a system/mic pair. `silentSystem` makes
    /// part B's system track header-only (an FR-MTG-10 degraded session).
    private func makeTwoPartMeeting(silentSystem: Bool = false) -> (id: Int64, paths: [String], files: [URL]) {
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .manual, audioPath: nil, status: .recording
        ))!
        var paths: [String] = [], files: [URL] = []
        for (i, stamp) in ["A", "B"].enumerated() {
            let sys = writeTrack(recordings.appendingPathComponent("system-audio-\(stamp).caf"), seconds: silentSystem && i == 1 ? 0 : 2)
            let mic = writeTrack(recordings.appendingPathComponent("mic-audio-\(stamp).caf"), seconds: 3)
            files += [sys, mic]
            paths.append(sys.path)
            if i == 1 { XCTAssertTrue(store.reopenMeeting(id: id)) }
            let part = store.beginMeetingPart(meetingId: id, startUnixMs: Int64(1_000 + i * 100_000), continueKey: nil)!
            XCTAssertTrue(store.completeMeetingPart(meetingId: id, partIndex: part, endUnixMs: Int64(60_000 + i * 100_000), audioPath: sys.path))
            XCTAssertTrue(store.completeMeeting(id: id, endUnixMs: Int64(60_000 + i * 100_000), audioPath: sys.path))
        }
        // The meeting row repeats a part's path, as the app stores it.
        return (id, [store.getMeeting(id: id)!.audioPath!] + paths, files)
    }

    private func storedAudioPaths(_ id: Int64) -> [String?] {
        [store.getMeeting(id: id)?.audioPath] + (store.meetingParts(meetingId: id) ?? []).map(\.audioPath)
    }

    /// Nothing changed: same stored paths, every original present, no `.m4a` or temp left.
    private func assertUntouched(_ m: (id: Int64, paths: [String], files: [URL]), before: [String?],
                                 file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(storedAudioPaths(m.id), before, file: file, line: line)
        for f in m.files { XCTAssertTrue(fm.fileExists(atPath: f.path), f.path, file: file, line: line) }
        XCTAssertEqual(contents(recordings), m.files.map(\.lastPathComponent).sorted(), "no .m4a and no temp left", file: file, line: line)
    }

    // MARK: compress

    func testCompressWritesMono32kAAC() throws {
        // 30 s, not the brief's 3 s: AVAudioFile writes ~22 KB of fixed container overhead,
        // more than a quarter of a 3 s source (100 KB) on its own. The payload is ~32 kbps.
        let src = writeTrack(recordings.appendingPathComponent("mic-audio-A.caf"), seconds: 30)
        let dst = recordings.appendingPathComponent("mic-audio-A.m4a")
        try AudioCompressor.compress(src, to: dst)

        let out = try AVAudioFile(forReading: dst)
        XCTAssertEqual(out.fileFormat.settings[AVFormatIDKey] as? UInt32, kAudioFormatMPEG4AAC)
        XCTAssertEqual(out.fileFormat.channelCount, 1)
        XCTAssertEqual(out.fileFormat.sampleRate, 16_000)
        XCTAssertEqual(seconds(dst), 30, accuracy: 0.1)
        let size = { (u: URL) in try self.fm.attributesOfItem(atPath: u.path)[.size] as! Int }
        XCTAssertLessThan(try size(dst), try size(src) / 4)
    }

    func testCompressDownmixesStereoToMono() throws {
        let src = recordings.appendingPathComponent("mic-audio-S.caf")
        let fmt = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2)!
        do {
            let f = try AVAudioFile(forWriting: src, settings: fmt.settings)
            let buf = AVAudioPCMBuffer(pcmFormat: fmt, frameCapacity: 48_000 * 2)!
            buf.frameLength = buf.frameCapacity
            for c in 0..<2 { for i in 0..<Int(buf.frameLength) { buf.floatChannelData![c][i] = 0.3 * sin(Float(i) / 20) } }
            try f.write(from: buf)
        }
        let dst = m4a(src)
        try AudioCompressor.compress(src, to: dst)
        let out = try AVAudioFile(forReading: dst)
        XCTAssertEqual(out.fileFormat.channelCount, 1)
        XCTAssertEqual(out.fileFormat.sampleRate, 48_000)
        XCTAssertEqual(seconds(dst), 2, accuracy: 0.1)
    }

    func testCompressHeaderOnlyTrackYieldsEmptyM4A() throws {
        let src = writeTrack(recordings.appendingPathComponent("system-audio-A.caf"), seconds: 0)
        let dst = m4a(src)
        try AudioCompressor.compress(src, to: dst)
        // Opens as a real AAC file with no frames: `hasAudioContent` alone would also be
        // false for a file that doesn't open at all.
        let out = try AVAudioFile(forReading: dst)
        XCTAssertEqual(out.fileFormat.settings[AVFormatIDKey] as? UInt32, kAudioFormatMPEG4AAC)
        XCTAssertEqual(out.length, 0)
        XCTAssertFalse(AudioConversion.hasAudioContent(dst))
    }

    // MARK: compressMeeting

    func testCompressMeetingRewritesPathsAndRemovesOriginals() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        XCTAssertTrue(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))

        XCTAssertEqual(storedAudioPaths(m.id), before.map { $0.map { m4a(URL(fileURLWithPath: $0)).path } })
        XCTAssertEqual(Set(storedAudioPaths(m.id).compactMap { $0 }).count, 2, "both parts rewritten")
        XCTAssertEqual(contents(recordings), m.files.map { m4a($0).lastPathComponent }.sorted(), "originals gone, no temps")
        for f in m.files { XCTAssertEqual(seconds(m4a(f)), f.lastPathComponent.hasPrefix("mic") ? 3 : 2, accuracy: 0.1) }
    }

    func testCompressMeetingWithAHeaderOnlySystemTrackKeepsThePair() {
        let m = makeTwoPartMeeting(silentSystem: true)
        XCTAssertTrue(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        let sysB = m4a(m.files[2])
        XCTAssertTrue(fm.fileExists(atPath: sysB.path), "both files of the pair keep one extension")
        XCTAssertFalse(AudioConversion.hasAudioContent(sysB))
        XCTAssertEqual(contents(recordings), m.files.map { m4a($0).lastPathComponent }.sorted())
    }

    func testCompressMeetingFailureLeavesOriginalsAndNoTemps() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        try! fm.setAttributes([.posixPermissions: 0o555], ofItemAtPath: recordings.path)

        XCTAssertFalse(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        assertUntouched(m, before: before)
    }

    func testCompressMeetingConversionFailureLeavesOriginals() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        var calls = 0
        AudioCompressor.encode = { src, dst in
            calls += 1
            if calls == 3 { throw CocoaError(.fileWriteUnknown) }
            try AudioCompressor.compress(src, to: dst)
        }
        XCTAssertFalse(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        assertUntouched(m, before: before)
    }

    func testCompressMeetingOneTrackOfAPairFailingFailsTheMeeting() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        AudioCompressor.encode = { src, dst in
            if src.lastPathComponent == "mic-audio-B.caf" { throw CocoaError(.fileWriteUnknown) }
            try AudioCompressor.compress(src, to: dst)
        }
        XCTAssertFalse(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        assertUntouched(m, before: before)
    }

    func testCompressMeetingShortOutputFailsTheDurationCheck() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        let short = writeTrack(tempDir.appendingPathComponent("short.caf"), seconds: 1)
        AudioCompressor.encode = { src, dst in
            try AudioCompressor.compress(src.lastPathComponent == "mic-audio-A.caf" ? short : src, to: dst)
        }
        XCTAssertFalse(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        assertUntouched(m, before: before)
    }

    func testCompressMeetingFailedPathUpdateKeepsOriginals() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        // A path the store doesn't hold for this meeting: the update rolls back and fails.
        let stray = writeTrack(recordings.appendingPathComponent("system-audio-Z.caf"), seconds: 1)

        XCTAssertFalse(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths + [stray.path], store: store))
        assertUntouched((m.id, m.paths, m.files + [stray]), before: before)
    }

    func testCompressMeetingRefusesADifferentFileAtTheDestination() throws {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        let theirs = m4a(m.files[1])
        try Data("someone else's".utf8).write(to: theirs)

        XCTAssertFalse(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        XCTAssertEqual(try String(contentsOf: theirs), "someone else's")
        try fm.removeItem(at: theirs)
        assertUntouched(m, before: before)
    }

    func testCompressMeetingWithAnUnreachableFolderChangesNothing() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        let gone = "/Volumes/Whimbrel-Test-Gone/Recordings/system-audio-Q.caf"

        XCTAssertFalse(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths + [gone], store: store))
        assertUntouched(m, before: before)
    }

    func testCompressMeetingWithAFolderDeletedOnAMountedVolumeCompressesTheRest() {
        // A part's folder deleted in Finder: its files are already gone, not on an unplugged drive.
        let m = makeTwoPartMeeting()
        let gone = tempDir.appendingPathComponent("deleted-in-finder/system-audio-Q.caf").path

        XCTAssertTrue(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths + [gone], store: store))
        XCTAssertEqual(contents(recordings), m.files.map { m4a($0).lastPathComponent }.sorted())
    }

    func testCompressMeetingAlreadyCompressedIsANoOp() {
        let m = makeTwoPartMeeting()
        XCTAssertTrue(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        let after = storedAudioPaths(m.id)
        XCTAssertTrue(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: after.compactMap { $0 }, store: store))
        XCTAssertEqual(storedAudioPaths(m.id), after)
    }

    func testCompressRefusesAnUnsupportedSampleRate() {
        let src = writeTrack(recordings.appendingPathComponent("mic-audio-H.caf"), seconds: 1, rate: 96_000)
        let dst = m4a(src)
        XCTAssertThrowsError(try AudioCompressor.compress(src, to: dst))
        XCTAssertFalse(fm.fileExists(atPath: dst.path), "nothing left at the destination")
    }

    func testCompressMeetingWithAnOddSampleRateTrackChangesNothing() {
        let m = makeTwoPartMeeting()
        let before = storedAudioPaths(m.id)
        try! fm.removeItem(at: m.files[3])
        writeTrack(m.files[3], seconds: 1, rate: 12_345)

        XCTAssertFalse(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        assertUntouched(m, before: before)
    }

    func testCompressMeetingSkipsFilesThatAreNotWhimbrelRecordings() throws {
        let interview = writeTrack(recordings.appendingPathComponent("interview.caf"), seconds: 1)
        let notes = writeTrack(recordings.appendingPathComponent("notes.wav"), seconds: 1)
        let mp3 = recordings.appendingPathComponent("system-audio-X.mp3")
        try Data("not ours".utf8).write(to: mp3)
        let id = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1_000, endUnixMs: 2_000, title: "Interview",
            attendees: [], trigger: .manual, audioPath: interview.path, status: .completed
        ))!
        let before = contents(recordings)

        XCTAssertTrue(AudioCompressor.compressMeeting(meetingId: id, audioPaths: [interview.path, notes.path, mp3.path], store: store))
        XCTAssertEqual(contents(recordings), before)
        XCTAssertEqual(store.getMeeting(id: id)?.audioPath, interview.path)
    }

    func testCompressMeetingWithAMissingSystemFileCompressesTheMic() {
        let m = makeTwoPartMeeting()
        try! fm.removeItem(at: m.files[0])  // system-audio-A.caf

        XCTAssertTrue(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        XCTAssertEqual(contents(recordings), ["mic-audio-A.m4a", "mic-audio-B.m4a", "system-audio-B.m4a"])
        // The stored path follows the pair's extension, so the mic `.m4a` is still found.
        XCTAssertEqual(store.meetingParts(meetingId: m.id)?.first?.audioPath, m4a(m.files[0]).path)
        XCTAssertEqual(TranscriptionService.trackURLs(forAudioPath: m4a(m.files[0]).path)?.mic, m4a(m.files[1]))
    }

    func testCompressMeetingNotConfirmedDurableDeletesNothing() throws {
        let m = makeTwoPartMeeting()
        var reader: OpaquePointer?
        AudioCompressor.beforePathUpdate = { reader = holdReadSnapshot(self.tempDir.appendingPathComponent("test.sqlite")) }

        XCTAssertFalse(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))
        XCTAssertNotNil(reader)
        sqlite3_close_v2(reader)
        // Committed but unconfirmed: the rows may point either way after a power cut, so
        // both the originals and the `.m4a` files stay.
        for f in m.files {
            XCTAssertTrue(fm.fileExists(atPath: f.path), f.path)
            XCTAssertTrue(fm.fileExists(atPath: m4a(f).path), m4a(f).path)
        }
        XCTAssertEqual(Set(storedAudioPaths(m.id).compactMap { $0.map { URL(fileURLWithPath: $0).pathExtension } }), ["m4a"])
    }

    func testCompressedMeetingTranscribesAgain() throws {
        let m = makeTwoPartMeeting(silentSystem: true)
        XCTAssertTrue(AudioCompressor.compressMeeting(meetingId: m.id, audioPaths: m.paths, store: store))

        let lock = NSLock()
        var calls: [String] = []
        let queue = MeetingTranscriptionQueue(
            store: store,
            transcribe: { path, _ in
                lock.withLock { calls.append(path) }
                return (self.tempDir.appendingPathComponent("t.txt"), [])
            },
            summarize: { _, _, _ in }, onStage: { _, _ in })
        queue.transcribeAgain(meetingId: m.id, notes: "")
        CoreMemoryStore.accessQueue.sync {}
        queue.waitUntilIdle()
        XCTAssertEqual(lock.withLock { calls }, [m.files[0], m.files[2]].map { m4a($0).path })

        for path in calls {
            let tracks = try XCTUnwrap(TranscriptionService.trackURLs(forAudioPath: path))
            XCTAssertTrue(fm.fileExists(atPath: tracks.system.path))
            XCTAssertTrue(fm.fileExists(atPath: tracks.mic.path))
            XCTAssertEqual(tracks.mic.pathExtension, "m4a")
            let wav = tempDir.appendingPathComponent(UUID().uuidString + ".wav")
            try AudioConversion.convertTo16kHzMonoWAV(source: tracks.mic, destination: wav)
            XCTAssertEqual(seconds(wav), 3, accuracy: 0.1)
        }
        // Part B's header-only system track is skipped, as before compression.
        let sysB = try XCTUnwrap(TranscriptionService.trackURLs(forAudioPath: calls[1])).system
        XCTAssertFalse(AudioConversion.hasAudioContent(sysB))
    }
}
