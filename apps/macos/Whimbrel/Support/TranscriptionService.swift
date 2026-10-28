import Foundation
import OSLog

/// FR-MTG-4: turns captured meeting audio into a transcript — the first thing to
/// actually call core/transcribe's whisper.cpp integration from the app. Transcribes
/// the most recent system-audio/mic-audio `.caf` pair (see AudioCaptureService) one
/// channel at a time. The mic channel is always labeled "Me"; the system-audio channel
/// gets real per-speaker labels ("Speaker 1", "Speaker 2", ...) when diarization models
/// are configured, falling back to a blanket "Others" label otherwise (see
/// `transcribeSystemAudioWithDiarization`). Merges the two channels' transcripts in
/// timestamp order, and writes the result to
/// a text file next to the audio. Spike-level output (a flat text file), like the rest
/// of apps/macos's file-based spike storage — production storage goes through
/// core/memory per docs/04-architecture.md.
enum TranscriptionService {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "transcription")

    enum Speaker: String {
        case me = "Me"
        case others = "Others"
    }

    /// JSON shape matches core/transcribe's `postprocess::Line` (the bridge encodes
    /// these for `CoreMemoryStore.postprocessTranscript`).
    struct Line: Equatable, Codable {
        let speakerLabel: String
        let startCs: Int64
        var endCs: Int64 = 0
        let text: String

        enum CodingKeys: String, CodingKey {
            case speakerLabel = "speaker_label"
            case startCs = "start_cs"
            case endCs = "end_cs"
            case text
        }
    }

    enum TranscriptionError: Error, CustomStringConvertible {
        case noRecordingFound
        case modelMissing(URL)
        case conversionFailed(String)
        case transcriptionFailed(String)

        var description: String {
            switch self {
            case .noRecordingFound:
                return "No captured audio found in the spike folder — run a capture session first."
            case .modelMissing(let url):
                return "Whisper model not found at \(url.path) — use the first-run \"Set Up Models\" prompt to fetch it automatically, or see core/transcribe/README.md to fetch one manually."
            case .conversionFailed(let msg):
                return "Audio conversion failed: \(msg)"
            case .transcriptionFailed(let file):
                return "Transcription failed for \(file) (bad model, bad audio, or a whisper.cpp error)."
            }
        }
    }

    private static let modelPathDefaultsKey = "com.mpointer.kestrel.transcriptionModelPath"
    private static let legacyModelPathDefaultsKey = "littlebird.transcriptionModelPath"

    /// Same model file core/transcribe/README.md documents fetching — not bundled with
    /// the app, must be present on disk under the spike folder. Overridable from
    /// Settings > Transcription (e.g. to point at a larger/more accurate GGML model).
    static var modelPath: URL {
        get {
            ModelPathDefaultsMigration.resolve(
                newKey: modelPathDefaultsKey,
                legacyKey: legacyModelPathDefaultsKey,
                defaultPath: SpikePaths.root.appendingPathComponent("models/ggml-tiny.en.bin")
            )
        }
        set {
            UserDefaults.standard.set(newValue.path, forKey: modelPathDefaultsKey)
        }
    }

    /// FR-MTG-5: transcribes a specific meeting's recorded audio, given the path
    /// `AppState` stored on that meeting's row (`meeting.audioPath` — either the
    /// system-audio or mic-audio file, whichever `AppState.stopCapture()` preferred).
    /// Derives the shared session timestamp from that one filename, rather than
    /// scanning the directory for "the newest" recording — by the time a background
    /// transcription runs, a *newer* session may already be in progress, so "latest"
    /// would silently transcribe the wrong meeting.
    static func transcribe(meetingAudioPath: String, expectedSpeakerCount: Int?) throws -> (url: URL, lines: [Line]) {
        let url = URL(fileURLWithPath: meetingAudioPath)
        let filename = url.lastPathComponent
        let prefix = filename.hasPrefix("system-audio-") ? "system-audio-" : "mic-audio-"
        guard filename.hasPrefix(prefix) else {
            throw TranscriptionError.noRecordingFound
        }
        let stamp = timestamp(from: url, prefix: prefix)
        return try transcribeSession(stamp: stamp, expectedSpeakerCount: expectedSpeakerCount)
    }

    private static func transcribeSession(stamp: String, expectedSpeakerCount: Int?) throws -> (url: URL, lines: [Line]) {
        guard FileManager.default.fileExists(atPath: modelPath.path) else {
            throw TranscriptionError.modelMissing(modelPath)
        }

        let systemURL = SpikePaths.audioFile(name: "system-audio-\(stamp).caf")
        let micURL = SpikePaths.audioFile(name: "mic-audio-\(stamp).caf")

        let hasSystemAudio = FileManager.default.fileExists(atPath: systemURL.path) && AudioConversion.hasAudioContent(systemURL)
        let hasMicAudio = FileManager.default.fileExists(atPath: micURL.path)
        guard hasSystemAudio || hasMicAudio else {
            throw TranscriptionError.noRecordingFound
        }

        let systemLines = hasSystemAudio ? try transcribeSystemAudioWithDiarization(systemURL, expectedSpeakerCount: expectedSpeakerCount) : []
        let micLines = hasMicAudio ? try transcribeFile(micURL, speaker: .me) : []
        // Bleed filter + chronological interleave + same-speaker merge live in
        // core/transcribe::postprocess (M3.0 Task 1); with no system audio this is
        // exactly the same-speaker merge of the mic channel.
        guard let (lines, micLinesKept) = CoreMemoryStore.postprocessTranscript(micLines: micLines, systemLines: systemLines) else {
            throw TranscriptionError.transcriptionFailed("postprocess")
        }
        if hasSystemAudio, hasMicAudio {
            log.info("filterBleedThrough: \(micLines.count) mic lines before, \(micLinesKept) after")
        }
        return (url: try writeTranscript(lines, stamp: stamp), lines: lines)
    }

    private static func timestamp(from url: URL, prefix: String) -> String {
        url.deletingPathExtension().lastPathComponent.replacingOccurrences(of: prefix, with: "")
    }

    /// FR-MTG-4 follow-on: like `transcribeFile`, but for the system-audio channel
    /// specifically — runs whisper transcription and speaker diarization concurrently
    /// on the same converted WAV (not sequentially — every second matters against the
    /// existing post-meeting summary deadline), then merges them into per-speaker
    /// labeled lines. Falls back to the plain "Others" label — exactly today's
    /// pre-diarization behavior — if diarization models aren't configured, diarization
    /// fails for any reason, or the merge step fails; never throws over a diarization
    /// problem specifically, only over a transcription failure (matching
    /// `transcribeFile`'s existing contract).
    private static func transcribeSystemAudioWithDiarization(_ source: URL, expectedSpeakerCount: Int?) throws -> [Line] {
        let wavURL = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".wav")
        defer { try? FileManager.default.removeItem(at: wavURL) }
        do {
            try AudioConversion.convertTo16kHzMonoWAV(source: source, destination: wavURL)
        } catch {
            throw TranscriptionError.conversionFailed(String(describing: error))
        }

        var transcriptSegments: [CoreTranscriptSegment]?
        var speakerSegments: [CoreSpeakerSegment]?
        let group = DispatchGroup()

        group.enter()
        DispatchQueue.global(qos: .userInitiated).async {
            transcriptSegments = CoreMemoryStore.transcribeWav(modelPath: modelPath, wavPath: wavURL)
            group.leave()
        }

        group.enter()
        DispatchQueue.global(qos: .userInitiated).async {
            if DiarizationService.modelsAvailable {
                speakerSegments = CoreMemoryStore.diarizeWav(
                    segmentationModelPath: DiarizationService.segmentationModelPath,
                    embeddingModelPath: DiarizationService.embeddingModelPath,
                    wavPath: wavURL,
                    expectedSpeakerCount: expectedSpeakerCount
                )
            }
            group.leave()
        }

        group.wait()

        guard let transcriptSegments else {
            throw TranscriptionError.transcriptionFailed(source.lastPathComponent)
        }

        guard let speakerSegments,
              let transcriptData = try? JSONEncoder().encode(transcriptSegments),
              let transcriptJSON = String(data: transcriptData, encoding: .utf8),
              let speakerData = try? JSONEncoder().encode(speakerSegments),
              let speakerJSON = String(data: speakerData, encoding: .utf8),
              let labeled = CoreMemoryStore.labelSpeakers(transcriptSegmentsJSON: transcriptJSON, speakerSegmentsJSON: speakerJSON)
        else {
            // Diarization unavailable, failed, or the merge failed — degrade to
            // exactly today's pre-diarization behavior. Not an error: this function
            // only throws over a transcription failure (checked above).
            if DiarizationService.modelsAvailable {
                log.info("Diarization failed for this session — system-audio channel labeled 'Others'.")
            } else {
                log.info("Diarization models not configured — system-audio channel labeled 'Others'.")
            }
            return transcriptSegments.map { Line(speakerLabel: Speaker.others.rawValue, startCs: $0.startCs, endCs: $0.endCs, text: $0.text) }
        }

        return labeled.map { segment in
            let label = segment.speakerId.map { "Speaker \($0 + 1)" } ?? Speaker.others.rawValue
            return Line(speakerLabel: label, startCs: segment.startCs, endCs: segment.endCs, text: segment.text)
        }
    }

    private static func transcribeFile(_ source: URL, speaker: Speaker) throws -> [Line] {
        let wavURL = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".wav")
        defer { try? FileManager.default.removeItem(at: wavURL) }
        do {
            try AudioConversion.convertTo16kHzMonoWAV(source: source, destination: wavURL)
        } catch {
            throw TranscriptionError.conversionFailed(String(describing: error))
        }

        guard let segments = CoreMemoryStore.transcribeWav(modelPath: modelPath, wavPath: wavURL) else {
            throw TranscriptionError.transcriptionFailed(source.lastPathComponent)
        }
        return segments.map { Line(speakerLabel: speaker.rawValue, startCs: $0.startCs, endCs: $0.endCs, text: $0.text) }
    }

    /// Fixed 24-hour format regardless of the user's locale — a transcript is a
    /// technical artifact meant to be read unambiguously, not a locale-formatted date.
    /// `.current` timeZone is the point of this change: real local wall-clock time
    /// instead of the elapsed-seconds-since-start the transcript previously showed.
    private static func timeLabelFormatter() -> DateFormatter {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .current
        formatter.dateFormat = "HH:mm:ss"
        return formatter
    }

    private static func writeTranscript(_ lines: [Line], stamp: String) throws -> URL {
        let sessionStart = SpikePaths.sessionStartDate(from: stamp)
        let timeFormatter = timeLabelFormatter()
        let body = lines.map { line -> String in
            let label = timeLabel(for: line.startCs, sessionStart: sessionStart, formatter: timeFormatter)
            return "[\(label)] \(line.speakerLabel): \(line.text)"
        }.joined(separator: "\n")
        let url = SpikePaths.root.appendingPathComponent("transcript-\(stamp).txt")
        try body.write(to: url, atomically: true, encoding: .utf8)
        log.info("Wrote transcript to \(url.path, privacy: .public) (\(lines.count) segments)")
        return url
    }

    /// `sessionStart` is nil only if `stamp` fails to parse (shouldn't happen — it's
    /// generated by `SpikePaths.sessionTimestamp()` in the first place) — falls back to
    /// the previous elapsed-seconds format rather than losing the timestamp entirely.
    private static func timeLabel(for startCs: Int64, sessionStart: Date?, formatter: DateFormatter) -> String {
        guard let sessionStart else {
            let seconds = Double(startCs) / 100.0
            return String(format: "%7.1fs", seconds)
        }
        let lineDate = sessionStart.addingTimeInterval(Double(startCs) / 100.0)
        return formatter.string(from: lineDate)
    }
}
