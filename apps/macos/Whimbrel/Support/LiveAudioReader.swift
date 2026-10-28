import AVFoundation
import Foundation

/// Reads the settled frames of a recording that is still being written, as 16 kHz mono
/// Float32 for whisper. Opens its own read-only `AVAudioFile` per call so it never holds
/// anything the capture code uses and never writes to the file. See
/// docs/superpowers/specs/2026-10-06-live-transcription-design.md.
///
/// Two things callers must know. (1) The reader's view of a file still being written trails
/// the writer by about one packet (roughly 13 ms): that tail arrives on the next poll, nothing
/// is lost or duplicated, so "nothing new" right after a write is not guaranteed and a final
/// drain at stop should read from the finished file. (2) One converter is kept for the life of
/// the reader so the resampler's state carries across polls; only the last few milliseconds of
/// resampler latency are never emitted, and the final post-call pass covers them.
final class LiveAudioReader {
    enum ReadError: Error {
        case cannotOpen
        case cannotConvert
    }

    private let url: URL
    private var converter: AVAudioConverter?
    private var converterSourceFormat: AVAudioFormat?
    private(set) var framesRead: AVAudioFramePosition = 0
    private(set) var sourceSampleRate: Double = 0

    init(url: URL) {
        self.url = url
    }

    /// New audio since the last call, at most `maxSeconds` of source audio, converted to
    /// 16 kHz mono Float32. Empty when nothing new has settled. Never throws for "no new
    /// data"; throws `.cannotOpen` when the file cannot be opened for reading.
    func readNewSamples(maxSeconds: Double) throws -> [Float] {
        let file: AVAudioFile
        do {
            file = try AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false)
        } catch {
            throw ReadError.cannotOpen
        }
        let sourceFormat = file.processingFormat
        sourceSampleRate = sourceFormat.sampleRate
        let available = file.length - framesRead
        guard available > 0, sourceFormat.sampleRate > 0 else { return [] }

        let maxFrames = AVAudioFramePosition(maxSeconds * sourceFormat.sampleRate)
        let toRead = AVAudioFrameCount(min(available, maxFrames))
        guard toRead > 0,
              let input = AVAudioPCMBuffer(pcmFormat: sourceFormat, frameCapacity: toRead) else { return [] }
        file.framePosition = framesRead
        try? file.read(into: input, frameCount: toRead)
        guard input.frameLength > 0 else { return [] }
        framesRead += AVAudioFramePosition(input.frameLength)

        guard let target = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false) else {
            throw ReadError.cannotConvert
        }
        if converter == nil || converterSourceFormat != sourceFormat {
            guard let made = AVAudioConverter(from: sourceFormat, to: target) else { throw ReadError.cannotConvert }
            converter = made
            converterSourceFormat = sourceFormat
        }
        guard let converter else { throw ReadError.cannotConvert }
        let ratio = target.sampleRate / sourceFormat.sampleRate
        let capacity = AVAudioFrameCount(Double(input.frameLength) * ratio) + 1_024
        guard let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { throw ReadError.cannotConvert }
        var supplied = false
        var conversionError: NSError?
        let status = converter.convert(to: output, error: &conversionError) { _, outStatus in
            if supplied {
                outStatus.pointee = .noDataNow
                return nil
            }
            supplied = true
            outStatus.pointee = .haveData
            return input
        }
        guard status != .error, let data = output.floatChannelData else { throw ReadError.cannotConvert }
        return Array(UnsafeBufferPointer(start: data[0], count: Int(output.frameLength)))
    }
}
