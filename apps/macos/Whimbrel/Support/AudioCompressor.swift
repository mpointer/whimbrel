import AVFoundation
import Foundation
import os

/// Storage retention's audio compression (Task 7): a meeting's tracks become 32 kbps mono
/// AAC `.m4a` files beside the originals. Every track is encoded to a hidden temp, verified
/// and flushed; then all temps are renamed into place, the stored paths are rewritten, and
/// only then are the originals deleted. Any failure before the rewrite removes what this run
/// created and leaves the originals and their stored paths alone.
///
/// The store calls hop to `CoreMemoryStore.accessQueue.sync`, so never call
/// `compressMeeting` from `accessQueue` itself.
enum AudioCompressor {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "AudioCompressor")

    /// The fixed keys; `compress` adds `AVSampleRateKey` = the source's rate.
    static let settings: [String: Any] = [
        AVFormatIDKey: kAudioFormatMPEG4AAC,
        AVNumberOfChannelsKey: 1,
        AVEncoderBitRateKey: 32_000,
    ]
    /// A compressed track may differ from its source by this much and still count.
    static let durationTolerance: Double = 0.5

    /// Test seam: the encoder `compressMeeting` uses for each track.
    nonisolated(unsafe) static var encode: (URL, URL) throws -> Void = compress
    /// Test seam: runs just before the stored paths are rewritten.
    nonisolated(unsafe) static var beforePathUpdate: () -> Void = {}

    struct VerifyFailed: Error { let file: URL }
    struct DestinationTaken: Error { let file: URL }
    struct UnsupportedSampleRate: Error { let rate: Double }

    /// Only Whimbrel's own recordings are compressed: never a user's file.
    static let sourceExtensions: Set<String> = ["caf", "wav"]

    /// Encodes `src` to AAC at `dst` (which must not exist), streamed in 10 s chunks; more
    /// than one channel is downmixed. A 0-frame source writes a valid 0-frame file. On a
    /// throw, nothing is left at `dst`. The file is closed when this returns.
    static func compress(_ src: URL, to dst: URL) throws {
        guard !FileManager.default.fileExists(atPath: dst.path) else { throw DestinationTaken(file: dst) }
        do {
            let input = try AVAudioFile(forReading: src)
            let inFormat = input.processingFormat
            var outSettings = settings
            outSettings[AVSampleRateKey] = inFormat.sampleRate
            let output = try AVAudioFile(forWriting: dst, settings: outSettings, commonFormat: .pcmFormatFloat32, interleaved: false)
            let outFormat = output.processingFormat
            // The encoder must keep the source's rate; it never resamples.
            guard outFormat.sampleRate == inFormat.sampleRate else { throw UnsupportedSampleRate(rate: inFormat.sampleRate) }

            var converter: AVAudioConverter?
            if inFormat.channelCount != outFormat.channelCount {
                guard let c = AVAudioConverter(from: inFormat, to: outFormat) else {
                    throw AudioConversion.ConversionError.cannotCreateConverter
                }
                c.downmix = true
                converter = c
            }
            let chunk = AVAudioFrameCount(inFormat.sampleRate * 10)
            guard let inBuf = AVAudioPCMBuffer(pcmFormat: inFormat, frameCapacity: chunk),
                  let monoBuf = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: chunk) else {
                throw AudioConversion.ConversionError.cannotCreateBuffer
            }
            while input.framePosition < input.length {
                try autoreleasepool {
                    try input.read(into: inBuf, frameCount: chunk)
                    guard inBuf.frameLength > 0 else { return }
                    if let converter {
                        try converter.convert(to: monoBuf, from: inBuf)
                        try output.write(from: monoBuf)
                    } else {
                        try output.write(from: inBuf)
                    }
                }
                if inBuf.frameLength == 0 { break }
            }
        } catch {
            try? FileManager.default.removeItem(at: dst)
            throw error
        }
    }

    /// `out` opens as mono AAC and lasts as long as `src`, within `durationTolerance`.
    static func verify(_ out: URL, against src: URL) throws {
        let o = try AVAudioFile(forReading: out), s = try AVAudioFile(forReading: src)
        let seconds = { (f: AVAudioFile) in Double(f.length) / f.fileFormat.sampleRate }
        guard o.fileFormat.settings[AVFormatIDKey] as? UInt32 == kAudioFormatMPEG4AAC,
              o.fileFormat.channelCount == 1,
              abs(seconds(o) - seconds(s)) <= durationTolerance else { throw VerifyFailed(file: out) }
    }

    /// Compresses every existing `.caf`/`.wav` track of each stored `system-audio-*` or
    /// `mic-audio-*` path's pair as one unit; any other stored path is left alone. Then
    /// rewrites the stored paths to `.m4a` and deletes the originals once that commit is
    /// durable. False, with the originals and stored paths untouched and nothing this run
    /// created left behind, on any failure before the rewrite. If the rewrite commits but
    /// can't be confirmed durable, it returns false and deletes nothing at all.
    static func compressMeeting(meetingId: Int64, audioPaths: [String], store: CoreMemoryStore) -> Bool {
        dispatchPrecondition(condition: .notOnQueue(CoreMemoryStore.accessQueue))
        let fm = FileManager.default
        let m4a = { (u: URL) in u.deletingPathExtension().appendingPathExtension("m4a") }

        // Deduped: the meeting row repeats its last part's path.
        var stored: [String] = []
        for p in audioPaths where !stored.contains(p) { stored.append(p) }

        var files: [(src: URL, dst: URL, temp: URL)] = []
        var moves: [(old: String, new: String)] = []
        for p in stored {
            let url = URL(fileURLWithPath: p)
            guard sourceExtensions.contains(url.pathExtension.lowercased()),
                  let pair = TranscriptionService.trackURLs(forAudioPath: p) else { continue }
            var any = false
            for t in [pair.system, pair.mic] {
                // A gone folder on an unplugged drive: skip the whole meeting. Deleted on a
                // mounted volume: those files are already gone.
                if !fm.fileExists(atPath: t.deletingLastPathComponent().path) {
                    if MediaMover.volumeGone(t) { return false }
                    continue
                }
                guard fm.fileExists(atPath: t.path) else { continue }
                any = true
                guard !files.contains(where: { $0.src == t }) else { continue }
                let dst = m4a(t)
                files.append((t, dst, dst.deletingLastPathComponent().appendingPathComponent("." + dst.lastPathComponent)))
            }
            if any { moves.append((p, m4a(url).path)) }
        }
        guard !files.isEmpty else { return true }
        // Two sources can't become one `.m4a` (`x.caf` and `x.wav`), nor two stored paths one.
        guard Set(files.map(\.dst)).count == files.count, Set(moves.map(\.new)).count == moves.count else { return false }

        // Phase 1: every track to a verified, flushed hidden temp.
        let removeTemps = { for f in files { try? fm.removeItem(at: f.temp) } }
        do {
            // Never replace a file that is already there under the final name; checked
            // before any encoding so a taken name costs no work.
            if let taken = files.first(where: { fm.fileExists(atPath: $0.dst.path) }) { throw DestinationTaken(file: taken.dst) }
            for f in files {
                try? fm.removeItem(at: f.temp)  // ours by name: left by a run that stopped
                try encode(f.src, f.temp)
                try verify(f.temp, against: f.src)
                try MediaMover.flush(f.temp)
            }
        } catch {
            log.error("meeting \(meetingId): compression failed: \(String(describing: error))")
            removeTemps()
            return false
        }

        // Phase 2: rename into place, flush the folders, rewrite the stored paths durably.
        var renamed: [URL] = []
        let undo = {
            for r in renamed { try? fm.removeItem(at: r) }
            removeTemps()
        }
        do {
            for f in files {
                try fm.moveItem(at: f.temp, to: f.dst)  // fails if `dst` appeared meanwhile
                renamed.append(f.dst)
            }
            for dir in Set(files.map { $0.dst.deletingLastPathComponent() }) { try MediaMover.flush(dir) }
        } catch {
            log.error("meeting \(meetingId): compressed rename failed: \(String(describing: error))")
            undo()
            return false
        }
        beforePathUpdate()
        let (rewritten, durable) = CoreMemoryStore.accessQueue.sync {
            let ok = store.setMeetingAudioPaths(meetingId: meetingId, moves: moves)
            return (ok, ok && store.checkpointDurable())
        }
        guard rewritten else {
            log.error("meeting \(meetingId): audio path update failed; originals kept")
            undo()
            return false
        }
        // The rows may point at the `.m4a` files already: keep both until the commit is durable.
        guard durable else {
            log.error("meeting \(meetingId): audio path update not confirmed durable; keeping originals and .m4a files")
            return false
        }

        // The rows point at the `.m4a` files now; an original that won't delete is an orphan.
        for f in files {
            do { try fm.removeItem(at: f.src) } catch {
                log.error("meeting \(meetingId): could not delete \(f.src.path): \(String(describing: error))")
            }
        }
        return true
    }
}
