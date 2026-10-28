import AVFoundation

/// FR-MTG-4: core/transcribe's `Transcriber::transcribe_wav` requires 16kHz i16 PCM WAV
/// input and deliberately does no resampling — captured audio (`.caf`, whatever rate/
/// format AudioCaptureService wrote it in, e.g. 48kHz Float32) has to be converted
/// first. Uses AVAudioConverter (no new dependency) rather than a Rust-side resampler.
enum AudioConversion {
    enum ConversionError: Error, CustomStringConvertible {
        case emptyRecording
        case cannotCreateConverter
        case cannotCreateBuffer
        case converterFailed(String)

        var description: String {
            switch self {
            case .emptyRecording: return "recording contains no audio (0 frames) — capture likely degraded or was stopped immediately after starting"
            case .cannotCreateConverter: return "could not create an AVAudioConverter for this format"
            case .cannotCreateBuffer: return "could not allocate an audio buffer"
            case .converterFailed(let msg): return "AVAudioConverter failed: \(msg)"
            }
        }
    }

    /// FR-MTG-10 degraded sessions still write a header-only .caf for the channel that
    /// never actually captured (system audio has been observed doing this consistently
    /// when the tap degrades) — callers juggling multiple channels should check this
    /// before treating such a file as real content to transcribe.
    static func hasAudioContent(_ url: URL) -> Bool {
        guard let file = try? AVAudioFile(forReading: url) else { return false }
        return file.length > 0
    }

    /// Reads the whole source file into memory to convert it in one shot — simple and
    /// fine for the short capture-spike sessions this is exercised against, but scales
    /// linearly with recording length (e.g. ~230MB as Float32 for a 1-hour recording in
    /// today's 16 kHz mono format, ~1.4GB for an older 48 kHz stereo one); a real pipeline
    /// would stream/chunk this instead.
    static func convertTo16kHzMonoWAV(source: URL, destination: URL) throws {
        let sourceFile = try AVAudioFile(forReading: source)
        guard sourceFile.length > 0 else { throw ConversionError.emptyRecording }
        let sourceFormat = sourceFile.processingFormat

        guard let targetFormat = AVAudioFormat(
            commonFormat: .pcmFormatInt16, sampleRate: 16_000, channels: 1, interleaved: true
        ) else {
            throw ConversionError.cannotCreateConverter
        }
        guard let converter = AVAudioConverter(from: sourceFormat, to: targetFormat) else {
            throw ConversionError.cannotCreateConverter
        }

        guard let inputBuffer = AVAudioPCMBuffer(pcmFormat: sourceFormat, frameCapacity: AVAudioFrameCount(sourceFile.length)) else {
            throw ConversionError.cannotCreateBuffer
        }
        try sourceFile.read(into: inputBuffer)

        let ratio = targetFormat.sampleRate / max(sourceFormat.sampleRate, 1)
        let outputCapacity = AVAudioFrameCount(Double(inputBuffer.frameLength) * ratio) + 4096
        guard let outputBuffer = AVAudioPCMBuffer(pcmFormat: targetFormat, frameCapacity: outputCapacity) else {
            throw ConversionError.cannotCreateBuffer
        }

        var suppliedInput = false
        var conversionError: NSError?
        let status = converter.convert(to: outputBuffer, error: &conversionError) { _, outStatus in
            if suppliedInput {
                outStatus.pointee = .noDataNow
                return nil
            }
            suppliedInput = true
            outStatus.pointee = .haveData
            return inputBuffer
        }
        guard status != .error else {
            throw ConversionError.converterFailed(conversionError?.localizedDescription ?? "unknown error")
        }

        let outputSettings: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1,
            AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false,
        ]
        let outputFile = try AVAudioFile(
            forWriting: destination, settings: outputSettings, commonFormat: .pcmFormatInt16, interleaved: true
        )
        try outputFile.write(from: outputBuffer)
    }
}
