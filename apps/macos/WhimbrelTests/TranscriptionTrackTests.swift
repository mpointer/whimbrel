import AVFoundation
import XCTest
@testable import Whimbrel

final class TranscriptionTrackTests: XCTestCase {
    func testTrackURLsUseTheStoredFolderAndExtension() throws {
        let tracks = try XCTUnwrap(TranscriptionService.trackURLs(forAudioPath: "/x/Audio/mic-audio-S.m4a"))
        XCTAssertEqual(tracks.system.path, "/x/Audio/system-audio-S.m4a")
        XCTAssertEqual(tracks.mic.path, "/x/Audio/mic-audio-S.m4a")
    }

    func testTrackURLsNilForUnknownName() {
        XCTAssertNil(TranscriptionService.trackURLs(forAudioPath: "/x/Audio/other-S.caf"))
    }

    func testM4AConvertsTo16kHzWAV() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let m4a = dir.appendingPathComponent("mic-audio-S.m4a")
        let wav = dir.appendingPathComponent("out.wav")

        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 32_000,
        ]
        do {
            let file = try AVAudioFile(forWriting: m4a, settings: settings)
            let buf = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 16_000))
            buf.frameLength = 16_000
            let ch = try XCTUnwrap(buf.floatChannelData?[0])
            for i in 0..<16_000 { ch[i] = 0.5 * sinf(2 * .pi * 440 * Float(i) / 16_000) }
            try file.write(from: buf)
        } // file closes here, finalizing the m4a

        XCTAssertTrue(AudioConversion.hasAudioContent(m4a))
        try AudioConversion.convertTo16kHzMonoWAV(source: m4a, destination: wav)
        XCTAssertGreaterThan(try AVAudioFile(forReading: wav).length, 15_000)
    }
}
