import AVFoundation
import XCTest
@testable import Whimbrel

/// Re-linking a crash-stranded recording to its meeting by the file name's stamp (spec §6).
final class StrandedAudioMatchTests: XCTestCase {
    private let start = ISO8601DateFormatter().date(from: "2026-10-23T20:42:49Z")!

    private func pair(_ stamp: String) -> [String] {
        ["system-audio-\(stamp).caf", "mic-audio-\(stamp).caf"]
    }

    func testPicksPairStampedShortlyAfterStart() {
        let names = pair("2026-10-23T20-43-02.120Z")
        XCTAssertEqual(StrandedAudioMatch.audioFileName(meetingStart: start, fileNames: names, referenced: []),
                       "system-audio-2026-10-23T20-43-02.120Z.caf")
    }

    func testFallsBackToMicWhenNoSystemFile() {
        let names = ["mic-audio-2026-10-23T20-43-02.120Z.caf"]
        XCTAssertEqual(StrandedAudioMatch.audioFileName(meetingStart: start, fileNames: names, referenced: []),
                       "mic-audio-2026-10-23T20-43-02.120Z.caf")
    }

    func testIgnoresStampsBeforeStartOrPastWindow() {
        // 31 min after the start is past the 30 min window (R9).
        let names = pair("2026-10-23T20-42-00.000Z") + pair("2026-10-23T21-13-49.000Z")
        XCTAssertNil(StrandedAudioMatch.audioFileName(meetingStart: start, fileNames: names, referenced: []))
    }

    /// R9: a manual start's stamp can land minutes after its row when the store queue is busy.
    func testMatchesStampTenMinutesAfterStart() {
        let names = pair("2026-10-23T20-52-49.000Z")
        XCTAssertEqual(StrandedAudioMatch.audioFileName(meetingStart: start, fileNames: names, referenced: []),
                       "system-audio-2026-10-23T20-52-49.000Z.caf")
    }

    func testPicksClosestWithinWideWindow() {
        let names = pair("2026-10-23T21-02-49.000Z") + pair("2026-10-23T20-52-49.000Z")
        XCTAssertEqual(StrandedAudioMatch.audioFileName(meetingStart: start, fileNames: names, referenced: []),
                       "system-audio-2026-10-23T20-52-49.000Z.caf")
    }

    func testSkipsReferencedStamps() {
        let names = pair("2026-10-23T20-43-02.120Z")
        XCTAssertNil(StrandedAudioMatch.audioFileName(meetingStart: start, fileNames: names,
                                                      referenced: ["mic-audio-2026-10-23T20-43-02.120Z.caf"]))
    }

    func testPicksClosestOfTwo() {
        let names = pair("2026-10-23T20-43-30.000Z") + pair("2026-10-23T20-42-55.000Z")
        XCTAssertEqual(StrandedAudioMatch.audioFileName(meetingStart: start, fileNames: names, referenced: []),
                       "system-audio-2026-10-23T20-42-55.000Z.caf")
    }

    func testIgnoresOtherExtensionsAndNames() {
        let names = ["system-audio-2026-10-23T20-43-02.120Z.wav", "notes-2026-10-23T20-43-02.120Z.caf", "core-memory.sqlite"]
        XCTAssertNil(StrandedAudioMatch.audioFileName(meetingStart: start, fileNames: names, referenced: []))
    }

    func testReferencedNamesIncludeSiblings() {
        let names = StrandedAudioMatch.referencedNames(paths: ["/x/system-audio-A.caf", "/x/mic-audio-B.caf", nil])
        XCTAssertEqual(names, ["system-audio-A.caf", "mic-audio-A.caf", "mic-audio-B.caf", "system-audio-B.caf"])
    }

    private func part(_ index: Int64, end: Int64?) -> CoreMeetingPart {
        CoreMeetingPart(partIndex: index, startUnixMs: 0, endUnixMs: end, audioPath: nil, transcriptPath: nil, transcribed: false)
    }

    /// Review focus 4: a crash during part 2 re-links part 2, not part 1. Only the latest
    /// part can be recording, so a stale end-NULL earlier part never wins.
    func testOpenPartIsTheLatest() {
        XCTAssertEqual(StrandedAudioMatch.openPart([part(1, end: 5), part(2, end: nil)])?.partIndex, 2)
        XCTAssertEqual(StrandedAudioMatch.openPart([part(2, end: 9), part(1, end: 5)])?.partIndex, 2)
        XCTAssertEqual(StrandedAudioMatch.openPart([part(1, end: nil), part(2, end: nil)])?.partIndex, 2)
        XCTAssertEqual(StrandedAudioMatch.openPart([part(1, end: nil), part(2, end: 9)])?.partIndex, 2)
        XCTAssertNil(StrandedAudioMatch.openPart([]))
    }

    private func writeCaf(_ url: URL, frames: AVAudioFrameCount) throws {
        let format = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)!
        let file = try AVAudioFile(forWriting: url, settings: [
            AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 16_000, AVNumberOfChannelsKey: 1,
            AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false,
        ])
        let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames)!
        buffer.frameLength = frames
        try file.write(from: buffer)
    }

    func testRecordingEndIsStampPlusDuration() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("mic-audio-2026-10-23T20-43-02.000Z.caf")
        try writeCaf(url, frames: 32_000)
        let end = try XCTUnwrap(StrandedAudioMatch.recordingEnd(of: url))
        XCTAssertEqual(end.timeIntervalSince(ISO8601DateFormatter().date(from: "2026-10-23T20:43:02Z")!), 2, accuracy: 0.01)
    }

    /// One stream can stop early (a device drop); the recording ends when the longer one does.
    func testRecordingEndUsesLongerOfPair() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let system = dir.appendingPathComponent("system-audio-2026-10-23T20-43-02.000Z.caf")
        try writeCaf(system, frames: 32_000)
        try writeCaf(dir.appendingPathComponent("mic-audio-2026-10-23T20-43-02.000Z.caf"), frames: 48_000)
        let end = try XCTUnwrap(StrandedAudioMatch.recordingEnd(of: system))
        XCTAssertEqual(end.timeIntervalSince(ISO8601DateFormatter().date(from: "2026-10-23T20:43:02Z")!), 3, accuracy: 0.01)
    }
}
