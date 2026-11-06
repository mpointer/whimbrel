import AVFoundation
import os

/// "Trim recording": the audio side of a core trim (`CoreMemoryStore.trimMeeting`).
enum AudioTrimmer {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "trim")

    /// A core trim with what its files need: `parts` and `segments` as they were before it.
    struct CoreTrim {
        let result: CoreTrimResult
        let parts: [CoreMeetingPart]
        let segments: [TranscriptionService.Line]

        func applyToFiles() async { await AudioTrimmer.apply(result, parts: parts, segments: segments) }
    }

    /// Trims the meeting's rows in core (on `CoreMemoryStore.accessQueue`), reading first
    /// what the files' cut needs: core deletes the removed parts' rows and lines. Nil when
    /// core had nothing to trim.
    static func trimStored(store: CoreMemoryStore, meetingId: Int64, keepMs: Int64) throws -> CoreTrim? {
        let parts = store.meetingParts(meetingId: meetingId) ?? []
        var lines: [TranscriptionService.Line] = []
        while let page = store.getTranscriptSegmentsPage(meetingId: meetingId, offset: lines.count, limit: 500),
              !page.segments.isEmpty {
            lines += page.segments.map { TranscriptionService.Line(speakerLabel: $0.speakerLabel, startCs: $0.startCs, text: $0.text) }
        }
        return try store.trimMeeting(meetingId: meetingId, keepMs: keepMs).map { CoreTrim(result: $0, parts: parts, segments: lines) }
    }

    /// Keeps the first `keepSeconds` of the `.caf` (LPCM, passthrough) or `.m4a` (AAC) file at
    /// `url`, replacing it atomically. The original stays as it was on any failure.
    static func truncate(url: URL, keepSeconds: Double) async throws {
        let m4a = url.pathExtension.lowercased() == "m4a"
        guard let session = AVAssetExportSession(asset: AVURLAsset(url: url),
                                                 presetName: m4a ? AVAssetExportPresetAppleM4A : AVAssetExportPresetPassthrough) else {
            throw CocoaError(.fileWriteUnknown, userInfo: [NSFilePathErrorKey: url.path])
        }
        let temp = url.deletingLastPathComponent().appendingPathComponent(".trim-" + url.lastPathComponent)
        try? FileManager.default.removeItem(at: temp)
        defer { try? FileManager.default.removeItem(at: temp) }
        session.timeRange = CMTimeRange(start: .zero, duration: CMTime(seconds: keepSeconds, preferredTimescale: 1000))
        let type: AVFileType = m4a ? .m4a : .caf
        if #available(macOS 15, *) {
            try await session.export(to: temp, as: type)
        } else {
            session.outputURL = temp
            session.outputFileType = type
            await session.export()
            if session.status != .completed { throw session.error ?? CocoaError(.fileWriteUnknown) }
        }
        _ = try FileManager.default.replaceItemAt(url, withItemAt: temp)
    }

    /// A part's transcript file (one line per stored segment, in time order) cut to its first
    /// `keep` lines; nil when its line count isn't `lines`, so it can't be matched safely.
    static func keptTranscript(_ text: String, lines: Int, keep: Int) -> String? {
        let all = text.split(separator: "\n", omittingEmptySubsequences: false)
        guard all.count == lines else { return nil }
        return all.prefix(keep).joined(separator: "\n")
    }

    /// After a core trim: cuts both tracks of the spanning part at its kept length and its
    /// transcript file to the kept lines, and deletes the removed parts' tracks and transcript
    /// files and the removed snapshots. `parts` and `segments` (the meeting's stored lines,
    /// meeting clock) are as they were before the trim: core has deleted the removed rows.
    /// Failures are logged and skipped: the trim itself has already happened.
    static func apply(_ result: CoreTrimResult, parts: [CoreMeetingPart], segments: [TranscriptionService.Line]) async {
        let fm = FileManager.default
        let tracks = { (part: CoreMeetingPart) -> [URL] in
            guard let path = part.audioPath else { return [] }
            guard let pair = TranscriptionService.trackURLs(forAudioPath: path) else { return [URL(fileURLWithPath: path)] }
            return [pair.system, pair.mic]
        }
        if let span = result.spanningPart, let part = parts.first(where: { $0.partIndex == span.partIndex }) {
            for url in tracks(part) where fm.fileExists(atPath: url.path) {
                do { try await truncate(url: url, keepSeconds: Double(span.keepMs) / 1000) } catch {
                    log.error("trim: couldn't cut \(url.lastPathComponent, privacy: .public): \(String(describing: error), privacy: .public)")
                }
            }
            if let path = part.transcriptPath, let first = parts.first {
                // The part's lines run to the next part's start (as core's replace bounds them).
                let next = parts.first { $0.partIndex > part.partIndex }.map { ($0.startUnixMs - first.startUnixMs) / 10 } ?? .max
                cutTranscript(path, segments: segments.filter { $0.startCs < next },
                              partOffsetMs: part.startUnixMs - first.startUnixMs, keepMs: span.keepMs)
            }
        }
        let removed = parts.filter { result.removedParts.contains($0.partIndex) }
        var files = result.removedSnapshotPaths.map { URL(fileURLWithPath: $0) }
        for part in removed {
            files += tracks(part)
            if let transcript = part.transcriptPath { files.append(URL(fileURLWithPath: transcript)) }
        }
        for url in files where fm.fileExists(atPath: url.path) {
            do { try fm.removeItem(at: url) } catch {
                log.error("trim: couldn't delete \(url.lastPathComponent, privacy: .public): \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// `segments` holds no later part's lines, so the part's lines are those from its offset
    /// on. A file that doesn't match them line for line is rewritten from the kept lines
    /// (part clock, labeled from the file's session stamp, as transcription writes it).
    private static func cutTranscript(_ path: String, segments: [TranscriptionService.Line], partOffsetMs: Int64, keepMs: Int64) {
        guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { return }
        let inPart = segments.filter { $0.startCs >= partOffsetMs / 10 }
        let kept = inPart.filter { $0.startCs * 10 < partOffsetMs + keepMs }
        do {
            if let cut = keptTranscript(text, lines: inPart.count, keep: kept.count) {
                try cut.write(toFile: path, atomically: true, encoding: .utf8)
            } else {
                log.notice("trim: \((path as NSString).lastPathComponent, privacy: .public) doesn't match the stored lines; rewritten")
                let name = ((path as NSString).lastPathComponent as NSString).deletingPathExtension
                let stamp = name.hasPrefix("transcript-") ? String(name.dropFirst("transcript-".count)) : name
                let lines = kept.map { TranscriptionService.Line(speakerLabel: $0.speakerLabel, startCs: $0.startCs - partOffsetMs / 10, text: $0.text) }
                try TranscriptionService.transcriptText(lines, stamp: stamp).write(toFile: path, atomically: true, encoding: .utf8)
            }
        } catch {
            log.error("trim: couldn't cut \((path as NSString).lastPathComponent, privacy: .public): \(String(describing: error), privacy: .public)")
        }
    }
}
