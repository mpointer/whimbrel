import AVFoundation
import XCTest
@testable import Whimbrel

/// "Trim recording…" (docs/superpowers/specs/2026-11-05-trim-and-still-in-meeting-design.md).
final class MeetingTrimTests: XCTestCase {
    private func meeting(deviceId: String, status: CoreMeetingStatus) -> CoreMeeting {
        CoreMeeting(
            id: 1, deviceId: deviceId, startUnixMs: 0, endUnixMs: status == .completed ? 7_200_000 : nil,
            title: "Demo", attendees: [], trigger: .manual, audioPath: nil, status: status,
            transcriptPath: nil, notes: nil, summaryTldr: nil, summaryDecisions: [],
            summaryActionItems: [], summaryOpenQuestions: [], summaryProvider: nil, summaryModel: nil,
            estimatedSpeakerCount: nil
        )
    }

    func testRemovedLineFormats() {
        XCTAssertEqual(TrimRecordingSheet.removedLine(currentMs: 7_200_000, keepMs: 3_900_000), "Removes 55 minutes after 1:05")
        XCTAssertEqual(TrimRecordingSheet.removedLine(currentMs: 137_000_000, keepMs: 1_800_000), "Removes 37 hours, 33 minutes after 0:30")
        XCTAssertEqual(TrimRecordingSheet.removedLine(currentMs: 3_930_000, keepMs: 3_900_000), "Removes 30 seconds after 1:05")
    }

    func testKeepParsesHoursMinutes() {
        XCTAssertEqual(TrimRecordingSheet.keepMs(from: "1:05"), 3_900_000)
        XCTAssertEqual(TrimRecordingSheet.keepMs(from: "0:30"), 1_800_000)
        XCTAssertEqual(TrimRecordingSheet.keepMs(from: "12:00"), 43_200_000)
        XCTAssertEqual(TrimRecordingSheet.keepMs(from: "999:59"), 3_599_940_000)
        for bad in ["1:75", "1:5", "105", "1:05:00", "", ":30", "-1:05", "+1:05", "a:bc", "1000:00", "99999999999999999:00"] {
            XCTAssertNil(TrimRecordingSheet.keepMs(from: bad), bad)
        }
        XCTAssertEqual(TrimRecordingSheet.hoursMinutes(3_900_000), "1:05")
    }

    func testTrimOfferedOnlyWhenOwnedNotRecordingAndShorter() {
        let ours = meeting(deviceId: "mac", status: .completed)
        XCTAssertTrue(MeetingOptionsMenu.canOfferTrim(meeting: ours, deviceId: "mac", lengthMs: 7_200_000))
        XCTAssertFalse(MeetingOptionsMenu.canOfferTrim(meeting: ours, deviceId: "other", lengthMs: 7_200_000), "another device's")
        XCTAssertFalse(MeetingOptionsMenu.canOfferTrim(meeting: meeting(deviceId: "mac", status: .recording), deviceId: "mac", lengthMs: 7_200_000))
        XCTAssertFalse(MeetingOptionsMenu.canOfferTrim(meeting: ours, deviceId: "mac", lengthMs: 60_000), "no whole minute shorter")
        XCTAssertFalse(MeetingOptionsMenu.canOfferTrim(meeting: ours, deviceId: "mac", lengthMs: nil), "length not loaded")
        // The sheet's Trim needs a valid keep above zero and shorter than the meeting.
        XCTAssertEqual(TrimRecordingSheet.keep("1:05", lengthMs: 7_200_000), 3_900_000)
        XCTAssertNil(TrimRecordingSheet.keep("2:00", lengthMs: 7_200_000), "not shorter")
        XCTAssertNil(TrimRecordingSheet.keep("0:00", lengthMs: 7_200_000), "nothing kept")
        XCTAssertNil(TrimRecordingSheet.keep("1:75", lengthMs: 7_200_000))
    }

    func testKeptTranscriptCutsMatchingFileOnly() {
        let text = "[10:00:01] Me: a\n[10:00:05] Speaker 1: b\n[10:00:09] Me: c"
        XCTAssertEqual(AudioTrimmer.keptTranscript(text, lines: 3, keep: 2), "[10:00:01] Me: a\n[10:00:05] Speaker 1: b")
        XCTAssertNil(AudioTrimmer.keptTranscript(text, lines: 4, keep: 2), "doesn't match the stored lines")
    }

    /// Three parts cut inside part 2: part 3's lines (read before the trim) don't count
    /// against part 2's file, which is cut rather than deleted.
    func testSpanningPartTranscriptIsCutWhenLaterPartsAreRemoved() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = { (n: Int) in dir.appendingPathComponent("transcript-\(n).txt") }
        try "[10:00:01] Me: p1".write(to: file(1), atomically: true, encoding: .utf8)
        try "[10:01:05] Me: a\n[10:01:15] Me: b\n[10:01:35] Me: c".write(to: file(2), atomically: true, encoding: .utf8)
        try "[10:02:05] Me: p3".write(to: file(3), atomically: true, encoding: .utf8)
        // Parts 0-50 s, 60-100 s, 120-190 s; cut at 80 s (20 s into part 2).
        let parts = [(1, 0, 50_000), (2, 60_000, 100_000), (3, 120_000, 190_000)].map { n, start, end in
            CoreMeetingPart(partIndex: Int64(n), startUnixMs: Int64(start), endUnixMs: Int64(end), audioPath: nil,
                            transcriptPath: file(n).path, transcribed: true)
        }
        let json = #"{"removed_parts":[3],"spanning_part":[2,20000],"removed_snapshot_paths":[]}"#
        let result = try JSONDecoder().decode(CoreTrimResult.self, from: Data(json.utf8))
        // Stored lines (meeting clock, cs): part 1 at 1 s; part 2 at 65, 75, 95 s; part 3 at 125 s.
        await AudioTrimmer.apply(result, parts: parts, segments: [100, 6_500, 7_500, 9_500, 12_500].map(line))

        XCTAssertEqual(try String(contentsOf: file(2), encoding: .utf8), "[10:01:05] Me: a\n[10:01:15] Me: b")
        XCTAssertEqual(try String(contentsOf: file(1), encoding: .utf8), "[10:00:01] Me: p1", "earlier part untouched")
        XCTAssertFalse(FileManager.default.fileExists(atPath: file(3).path), "removed part's file deleted")
    }

    private func line(_ startCs: Int64) -> TranscriptionService.Line {
        TranscriptionService.Line(speakerLabel: "Me", startCs: startCs, text: "at \(startCs)")
    }

    /// A part file that doesn't match the stored lines is rewritten from the kept ones
    /// (part clock, timed from the file's stamp), not deleted.
    func testMismatchedTranscriptIsRewrittenFromStoredLines() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let stamp = "2026-11-05T10-01-00.000Z"
        let file = dir.appendingPathComponent("transcript-\(stamp).txt")
        try "only one line".write(to: file, atomically: true, encoding: .utf8)
        // Part 2 starts 60 s in; its stored lines are at 65, 75 and 95 s; cut 20 s into it.
        let parts = [CoreMeetingPart(partIndex: 1, startUnixMs: 0, endUnixMs: 50_000, audioPath: nil, transcriptPath: nil, transcribed: true),
                     CoreMeetingPart(partIndex: 2, startUnixMs: 60_000, endUnixMs: 100_000, audioPath: nil,
                                     transcriptPath: file.path, transcribed: true)]
        let json = #"{"removed_parts":[],"spanning_part":[2,20000],"removed_snapshot_paths":[]}"#
        let result = try JSONDecoder().decode(CoreTrimResult.self, from: Data(json.utf8))
        await AudioTrimmer.apply(result, parts: parts, segments: [100, 6_500, 7_500, 9_500].map(line))

        let expected = TranscriptionService.transcriptText(
            [TranscriptionService.Line(speakerLabel: "Me", startCs: 500, text: "at 6500"),
             TranscriptionService.Line(speakerLabel: "Me", startCs: 1_500, text: "at 7500")], stamp: stamp)
        XCTAssertEqual(try String(contentsOf: file, encoding: .utf8), expected)
    }

    func testTruncateCafAndM4aDurations() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let aac: [String: Any] = [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 16_000.0, AVNumberOfChannelsKey: 1]
        for (name, settings) in [("mic-audio-x.caf", RecordingWriter.fileSettings), ("mic-audio-x.m4a", aac)] {
            let url = dir.appendingPathComponent(name)
            try writeTone(url, seconds: 10, settings: settings)
            try await AudioTrimmer.truncate(url: url, keepSeconds: 4)
            let file = try AVAudioFile(forReading: url)
            XCTAssertEqual(Double(file.length) / file.fileFormat.sampleRate, 4, accuracy: 0.05, name)
        }
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted(),
                       ["mic-audio-x.caf", "mic-audio-x.m4a"], "no temp file left")
    }

    private func writeTone(_ url: URL, seconds: Double, settings: [String: Any]) throws {
        let file = try AVAudioFile(forWriting: url, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        let rate = file.processingFormat.sampleRate
        let frames = AVAudioFrameCount(seconds * rate)
        let buf = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: frames)!
        buf.frameLength = frames
        for i in 0..<Int(frames) { buf.floatChannelData![0][i] = 0.3 * sin(Float(i) * 2 * .pi * 440 / Float(rate)) }
        try file.write(from: buf)
    }
}
