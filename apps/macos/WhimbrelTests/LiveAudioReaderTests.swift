import AVFoundation
import XCTest
@testable import Whimbrel

final class LiveAudioReaderTests: XCTestCase {
    private func tempCAF() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("live-reader-\(UUID().uuidString).caf")
    }

    private var captureFormat: AVAudioFormat {
        AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 1, interleaved: false)!
    }

    /// A buffer of a 440 Hz sine, `seconds` long, in the capture format (48 kHz mono Float32).
    private func sine(seconds: Double, format: AVAudioFormat, amplitude: Float = 0.5) -> AVAudioPCMBuffer {
        let frames = AVAudioFrameCount(seconds * format.sampleRate)
        let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames)!
        buffer.frameLength = frames
        let channel = buffer.floatChannelData![0]
        for i in 0..<Int(frames) {
            channel[i] = amplitude * sinf(2 * .pi * 440 * Float(i) / Float(format.sampleRate))
        }
        return buffer
    }

    func testReadsSettledFramesWhileTheWriterStillHasTheFileOpen() throws {
        let url = tempCAF()
        defer { try? FileManager.default.removeItem(at: url) }
        let format = captureFormat
        let writer = try AVAudioFile(forWriting: url, settings: format.settings)
        try writer.write(from: sine(seconds: 3, format: format))

        let reader = LiveAudioReader(url: url)
        let first = try reader.readNewSamples(maxSeconds: 60)
        // 3 s at 16 kHz, give or take the converter's edge frames.
        XCTAssertGreaterThan(first.count, 16_000 * 3 - 800)
        XCTAssertLessThanOrEqual(first.count, 16_000 * 3 + 64)
        XCTAssertGreaterThan(first.map { abs($0) }.max() ?? 0, 0.3, "the sine must come through, not silence")

        // The reader's view of the file can trail the writer by a few hundred frames (the last
        // partial packet settles late). Nothing is lost: the next poll catches up, then it is idle.
        let catchUp = try reader.readNewSamples(maxSeconds: 60)
        XCTAssertLessThan(catchUp.count, 1_000)
        XCTAssertEqual(first.count + catchUp.count, 16_000 * 3, accuracy: 64)
        XCTAssertEqual(reader.framesRead, writer.length)
        XCTAssertEqual(try reader.readNewSamples(maxSeconds: 60).count, 0)

        // More audio is appended while the same writer is still open.
        try writer.write(from: sine(seconds: 2, format: format))
        let second = try reader.readNewSamples(maxSeconds: 60)
        XCTAssertGreaterThan(second.count, 16_000 * 2 - 800)
        XCTAssertLessThanOrEqual(second.count, 16_000 * 2 + 64)
    }

    func testRespectsMaxSecondsPerCall() throws {
        let url = tempCAF()
        defer { try? FileManager.default.removeItem(at: url) }
        let format = captureFormat
        let writer = try AVAudioFile(forWriting: url, settings: format.settings)
        try writer.write(from: sine(seconds: 10, format: format))

        let reader = LiveAudioReader(url: url)
        let part = try reader.readNewSamples(maxSeconds: 4)
        XCTAssertLessThanOrEqual(part.count, 16_000 * 4 + 64)
        let rest = try reader.readNewSamples(maxSeconds: 60)
        XCTAssertGreaterThan(part.count + rest.count, 16_000 * 10 - 1_600)
    }

    func testAMissingFileIsAnErrorNotACrash() {
        let reader = LiveAudioReader(url: URL(fileURLWithPath: "/nonexistent/none.caf"))
        XCTAssertThrowsError(try reader.readNewSamples(maxSeconds: 5))
    }

    func testReadingDoesNotChangeTheRecording() throws {
        let url = tempCAF()
        let referenceURL = tempCAF()
        defer {
            try? FileManager.default.removeItem(at: url)
            try? FileManager.default.removeItem(at: referenceURL)
        }
        let format = captureFormat
        do {
            let writer = try AVAudioFile(forWriting: url, settings: format.settings)
            try writer.write(from: sine(seconds: 2, format: format))
            let reader = LiveAudioReader(url: url)
            _ = try reader.readNewSamples(maxSeconds: 60)
            try writer.write(from: sine(seconds: 2, format: format))
            _ = try reader.readNewSamples(maxSeconds: 60)
        }
        // The recording must be exactly what a reader-free run would have written.
        do {
            let reference = try AVAudioFile(forWriting: referenceURL, settings: format.settings)
            try reference.write(from: sine(seconds: 2, format: format))
            try reference.write(from: sine(seconds: 2, format: format))
        }
        // Both writers are closed (out of scope) so the bytes are final.
        XCTAssertEqual(try Data(contentsOf: url), try Data(contentsOf: referenceURL))
    }

    func testConcurrentWriterAndPollingReaderLoseNothing() throws {
        let url = tempCAF()
        defer { try? FileManager.default.removeItem(at: url) }
        let format = captureFormat
        let writer = try AVAudioFile(forWriting: url, settings: format.settings)
        let buffer = sine(seconds: 0.5, format: format)
        let chunks = 16
        let done = DispatchSemaphore(value: 0)
        var writeError: Error?
        let thread = Thread {
            for _ in 0..<chunks {
                do { try writer.write(from: buffer) } catch { writeError = error; break }
                Thread.sleep(forTimeInterval: 0.1)
            }
            done.signal()
        }
        thread.start()

        let reader = LiveAudioReader(url: url)
        var total = 0
        var peak: Float = 0
        func poll() throws {
            let samples = try reader.readNewSamples(maxSeconds: 30)
            total += samples.count
            peak = max(peak, samples.map { abs($0) }.max() ?? 0)
        }
        while done.wait(timeout: .now() + 0.1) == .timedOut {
            try poll()
        }
        XCTAssertNil(writeError)
        try poll()

        let expected = Int(writer.length) * 16_000 / 48_000
        XCTAssertEqual(writer.length, AVAudioFramePosition(chunks) * AVAudioFramePosition(buffer.frameLength))
        XCTAssertLessThanOrEqual(abs(total - expected), 16_000, "reader saw \(total), expected ~\(expected)")
        XCTAssertGreaterThan(peak, 0.3)
    }

    /// Many short polls over a settled file must not lose samples at the poll boundaries
    /// (a fresh resampler per poll can swallow a few frames each time).
    func testManySmallPollsLoseNoSamplesAtThePollBoundaries() throws {
        let url = tempCAF()
        defer { try? FileManager.default.removeItem(at: url) }
        let format = captureFormat
        do {
            let writer = try AVAudioFile(forWriting: url, settings: format.settings)
            try writer.write(from: sine(seconds: 20, format: format))
        }
        let reader = LiveAudioReader(url: url)
        var total = 0
        for _ in 0..<110 {
            total += try reader.readNewSamples(maxSeconds: 0.2).count
        }
        let expected = 20 * 16_000
        XCTAssertLessThanOrEqual(abs(total - expected), expected / 1_000, "total \(total) vs \(expected)")
    }

    /// Review focus 5: the real recording format (16 kHz mono Int16 on disk, via
    /// `RecordingWriter.makeFile`), written in increments while the reader polls it.
    func testReadsARecordingWriterFileWhileItIsWritten() throws {
        let url = tempCAF()
        defer { try? FileManager.default.removeItem(at: url) }
        let writer = RecordingWriter(file: try RecordingWriter.makeFile(at: url))
        let format = writer.file.processingFormat
        XCTAssertEqual(format.sampleRate, 16_000)
        let reader = LiveAudioReader(url: url)
        var total = 0
        var peak: Float = 0
        for _ in 0..<10 {
            XCTAssertEqual(writer.write(sine(seconds: 0.5, format: format)), 8_000)
            let samples = try reader.readNewSamples(maxSeconds: 30)
            total += samples.count
            peak = max(peak, samples.map { abs($0) }.max() ?? 0)
        }
        total += try reader.readNewSamples(maxSeconds: 30).count
        XCTAssertEqual(total, 5 * 16_000, accuracy: 1_000, "reader saw \(total)")
        XCTAssertGreaterThan(peak, 0.3, "the sine must come through, not silence")
    }

    /// The system recording may be stereo: it must come out as mono 16 kHz with real audio.
    func testAStereoRecordingIsDownmixedToMono() throws {
        let url = tempCAF()
        defer { try? FileManager.default.removeItem(at: url) }
        let stereo = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 2, interleaved: false)!
        let frames = AVAudioFrameCount(3 * 48_000)
        let buffer = AVAudioPCMBuffer(pcmFormat: stereo, frameCapacity: frames)!
        buffer.frameLength = frames
        for channel in 0..<2 {
            let data = buffer.floatChannelData![channel]
            for i in 0..<Int(frames) { data[i] = 0.5 * sinf(2 * .pi * 440 * Float(i) / 48_000) }
        }
        do {
            let writer = try AVAudioFile(forWriting: url, settings: stereo.settings)
            try writer.write(from: buffer)
        }
        let samples = try LiveAudioReader(url: url).readNewSamples(maxSeconds: 60)
        // One call over a finished file leaves only the resampler's latency tail (about 13 ms) unemitted.
        XCTAssertLessThanOrEqual(abs(samples.count - 3 * 16_000), 400)
        XCTAssertGreaterThan(samples.map { abs($0) }.max() ?? 0, 0.3, "a stereo source must not downmix to silence")
    }
}
