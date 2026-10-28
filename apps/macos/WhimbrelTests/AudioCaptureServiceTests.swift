import AVFoundation
import XCTest
@testable import Whimbrel

final class AudioCaptureServiceTests: XCTestCase {
    /// Builds a mono float-PCM buffer whose peak absolute sample value is exactly `peak`,
    /// mirroring the shape `audibilityCheck` scans (some samples at/near the peak, some
    /// lower, matching a real captured buffer rather than a single constant-value tone).
    private func makeBuffer(peak: Float) -> AVAudioPCMBuffer {
        let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1)!
        let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 4)!
        buffer.frameLength = 4
        let samples = buffer.floatChannelData![0]
        samples[0] = 0
        samples[1] = peak
        samples[2] = -peak
        samples[3] = peak / 2
        return buffer
    }

    func testAmbientRoomNoiseIsNotAudibleUnderRecalibratedThreshold() {
        // Real measured ambient/quiet-room peak from this session's live calibration
        // (docs/superpowers/specs/2026-08-06-silence-threshold-recalibration-design.md):
        // 0.044-0.075 observed. 0.05 sits in that range — under the old 0.01 threshold
        // this was misclassified as audible; under the recalibrated 0.1 it must read silent.
        let buffer = makeBuffer(peak: 0.05)
        let result = AudioCaptureService.audibilityCheck(buffer)
        XCTAssertFalse(result.isAudible, "ambient room noise (peak 0.05) must not register as audible under the recalibrated threshold")
        XCTAssertEqual(result.peak, 0.05, accuracy: 0.0001)
    }

    func testRealSpeechLevelAudioIsAudible() {
        // Real speech/loud-moment peaks observed this session ranged 0.13-0.33.
        // 0.2 sits comfortably in that range and must still register as audible.
        let buffer = makeBuffer(peak: 0.2)
        let result = AudioCaptureService.audibilityCheck(buffer)
        XCTAssertTrue(result.isAudible, "real speech-level audio (peak 0.2) must register as audible")
        XCTAssertEqual(result.peak, 0.2, accuracy: 0.0001)
    }
    /// 48 kHz stereo sine, the shape ScreenCaptureKit delivers.
    private func makeStereo48k(frames: AVAudioFrameCount, amplitude: Float) -> AVAudioPCMBuffer {
        let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2)!
        let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames)!
        buffer.frameLength = frames
        for ch in 0..<2 {
            for i in 0..<Int(frames) {
                buffer.floatChannelData![ch][i] = amplitude * sin(2 * .pi * 440 * Float(i) / 48_000)
            }
        }
        return buffer
    }

    func testSystemBufferDownmixesTo16kMono() throws {
        let source = makeStereo48k(frames: 4_800, amplitude: 0.5)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("downmix-\(UUID().uuidString).caf")
        defer { try? FileManager.default.removeItem(at: url) }
        let target = try RecordingWriter.makeFile(at: url).processingFormat
        let converter = try XCTUnwrap(AudioCaptureService.makeConverter(from: AudioCaptureService.systemStreamFormat, to: target))
        // The resampler holds back ~200 frames of latency on the first buffer and keeps it
        // across the next ones (the service reuses one converter per recording), so check the
        // per-buffer average over a run rather than the first buffer alone.
        var total: AVAudioFrameCount = 0
        let buffers: AVAudioFrameCount = 20
        for _ in 0..<buffers {
            let out = try XCTUnwrap(AudioCaptureService.downmixToRecordingFormat(source, converter: converter, target: target))
            XCTAssertEqual(out.format.sampleRate, 16_000)
            XCTAssertEqual(out.format.channelCount, 1)
            total += out.frameLength
        }
        XCTAssertEqual(Double(total) / Double(buffers), 1_600, accuracy: 16)
    }

    /// Without `downmix`, AVAudioConverter keeps only the left channel going stereo → mono.
    func testRightOnlySystemAudioSurvivesDownmix() throws {
        let source = makeStereo48k(frames: 4_800, amplitude: 0.5)
        memset(source.floatChannelData![0], 0, Int(source.frameLength) * MemoryLayout<Float>.size)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("downmix-r-\(UUID().uuidString).caf")
        defer { try? FileManager.default.removeItem(at: url) }
        let target = try RecordingWriter.makeFile(at: url).processingFormat
        let converter = try XCTUnwrap(AudioCaptureService.makeConverter(from: AudioCaptureService.systemStreamFormat, to: target))
        var peak: Float = 0
        for _ in 0..<3 {
            let out = try XCTUnwrap(AudioCaptureService.downmixToRecordingFormat(source, converter: converter, target: target))
            peak = max(peak, AudioCaptureService.audibilityCheck(out).peak)
        }
        XCTAssertGreaterThan(peak, 0.1)
    }

    /// Guards against moving the silence check after conversion (Int16 would read as the fail-safe "audible").
    func testAudibilityRunsOnSourceBuffer() {
        XCTAssertFalse(AudioCaptureService.audibilityCheck(makeStereo48k(frames: 4_800, amplitude: 0)).isAudible)
        XCTAssertTrue(AudioCaptureService.audibilityCheck(makeStereo48k(frames: 4_800, amplitude: 0.5)).isAudible)
    }

    #if DEBUG
    /// Low disk on every tick still reports once per recording.
    @MainActor
    func testForcedStopReportsOnce() {
        UserDefaults.standard.set(1, forKey: "com.mpointer.kestrel.debugFreeBytes")
        defer { UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.debugFreeBytes") }
        let service = AudioCaptureService()
        var stops: [RecordingStop] = []
        service.onForcedStop = { stops.append($0) }
        service.checkRecordingLimits()
        service.checkRecordingLimits()
        XCTAssertEqual(stops, [.lowDisk])
    }
    #endif
}
