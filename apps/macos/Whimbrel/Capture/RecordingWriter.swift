import AVFoundation
import Foundation
import OSLog

enum RecordingStop: Equatable { case durationCap, lowDisk }

enum RecordingLimits {
    static let capSeconds: TimeInterval = 6 * 60 * 60
    static let minFreeBytesToStart: Int64 = 5_000_000_000
    static let minFreeBytesWhileRecording: Int64 = 2_000_000_000

    /// nil (unknown free space) never blocks a recording.
    static func canStart(freeBytes: Int64?) -> Bool {
        guard let freeBytes else { return true }
        return freeBytes >= minFreeBytesToStart
    }

    static func checkDisk(freeBytes: Int64?) -> RecordingStop? {
        guard let freeBytes, freeBytes < minFreeBytesWhileRecording else { return nil }
        return .lowDisk
    }

    static func freeBytes(at url: URL = SpikePaths.root) -> Int64? {
        #if DEBUG
        let override = UserDefaults.standard.integer(forKey: "com.mpointer.kestrel.debugFreeBytes")
        if override > 0 { return Int64(override) }
        #endif
        let values = try? url.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
        return values?.volumeAvailableCapacityForImportantUsage
    }
}

/// One per stream file. Not thread-safe; the caller serializes.
final class RecordingWriter {
    static let fileSettings: [String: Any] = [
        AVFormatIDKey: kAudioFormatLinearPCM,
        AVSampleRateKey: 16_000.0,
        AVNumberOfChannelsKey: 1,
        AVLinearPCMBitDepthKey: 16,
        AVLinearPCMIsFloatKey: false,
        AVLinearPCMIsBigEndianKey: false,
    ]

    static func makeFile(at url: URL) throws -> AVAudioFile {
        try AVAudioFile(forWriting: url, settings: fileSettings, commonFormat: .pcmFormatFloat32, interleaved: false)
    }

    let file: AVAudioFile
    let capFrames: AVAudioFramePosition
    private(set) var framesWritten: AVAudioFramePosition = 0
    private(set) var capReached = false
    private var loggedWriteFailure = false
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "audio-capture")

    init(file: AVAudioFile, capFrames: AVAudioFramePosition) {
        self.file = file
        self.capFrames = capFrames
    }

    convenience init(file: AVAudioFile) {
        self.init(file: file, capFrames: AVAudioFramePosition(RecordingLimits.capSeconds * file.processingFormat.sampleRate))
    }

    @discardableResult
    func write(_ buffer: AVAudioPCMBuffer) -> AVAudioFramePosition {
        let room = capFrames - framesWritten
        guard room > 0 else { capReached = true; return 0 }
        let n = min(AVAudioFramePosition(buffer.frameLength), room)
        let original = buffer.frameLength
        buffer.frameLength = AVAudioFrameCount(n)   // restored below; the caller's buffer is left as found
        defer { buffer.frameLength = original }
        guard writeToFile(buffer) else { return 0 }
        framesWritten += n
        if framesWritten >= capFrames { capReached = true }
        return n
    }

    @discardableResult
    func writeSilence(frames: AVAudioFramePosition) -> AVAudioFramePosition {
        let chunk: AVAudioFrameCount = 48_000
        let target = min(frames, capFrames - framesWritten)
        guard target > 0 else { if frames > 0 { capReached = true }; return 0 }
        guard let silence = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: chunk) else { return 0 }
        silence.frameLength = chunk
        for b in UnsafeMutableAudioBufferListPointer(silence.mutableAudioBufferList) {
            if let data = b.mData { memset(data, 0, Int(b.mDataByteSize)) }
        }
        var written: AVAudioFramePosition = 0
        while written < target {
            silence.frameLength = AVAudioFrameCount(min(target - written, AVAudioFramePosition(chunk)))
            guard writeToFile(silence) else { break }
            written += AVAudioFramePosition(silence.frameLength)
        }
        framesWritten += written
        if framesWritten >= capFrames { capReached = true }
        return written
    }

    /// Logs the first failure only: a broken file fails every buffer, ~50 times a second.
    private func writeToFile(_ buffer: AVAudioPCMBuffer) -> Bool {
        do {
            try file.write(from: buffer)
            return true
        } catch {
            if !loggedWriteFailure {
                loggedWriteFailure = true
                Self.log.error("Recording write failed (\(self.file.url.lastPathComponent, privacy: .public)): \(error.localizedDescription, privacy: .public)")
            }
            return false
        }
    }
}
