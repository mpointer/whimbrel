import Foundation
import WhimbrelUI

/// Where the paused stretches of a recording are kept, so the transcript can leave out whatever the
/// transcriber hears in the silence written for them. One small JSON file in the data folder,
/// named from the recording's own stamp like the transcript file (`transcript-<stamp>.txt`) is, in the
/// data folder the audio already lives in. It outlives a quit and is read again whenever the recording is
/// transcribed (after the meeting, at the next launch, or "Transcribe again"). Nothing in the database or
/// the sync changes. Only recordings that were paused have one.
enum RecordingPausedSpans {
    static let filePrefix = "recording-pauses-"

    /// The sidecar for `audioPath` (either track, `.caf` or compressed), always in the data folder, so it is found
    /// after the audio moved to the media folder. Nil for a path that carries neither track prefix.
    static func url(forAudioPath audioPath: String, root: URL = SpikePaths.root) -> URL? {
        let audio = URL(fileURLWithPath: audioPath)
        let name = audio.deletingPathExtension().lastPathComponent
        for prefix in ["system-audio-", "mic-audio-"] where name.hasPrefix(prefix) {
            return root.appendingPathComponent("\(filePrefix)\(name.dropFirst(prefix.count)).json")
        }
        return nil
    }

    /// Removes the file with the meeting's audio. A missing file is not an error.
    static func remove(forAudioPath audioPath: String, root: URL = SpikePaths.root) {
        guard let url = url(forAudioPath: audioPath, root: root) else { return }
        try? FileManager.default.removeItem(at: url)
    }

    static func write(_ spans: [RecordingPausedSpan], forAudioPath audioPath: String, root: URL = SpikePaths.root) {
        guard let url = url(forAudioPath: audioPath, root: root) else { return }
        if spans.isEmpty { try? FileManager.default.removeItem(at: url); return }
        guard let data = try? JSONEncoder().encode(spans) else { return }
        try? data.write(to: url, options: .atomic)
    }

    static func read(forAudioPath audioPath: String, root: URL = SpikePaths.root) -> [RecordingPausedSpan] {
        guard let url = url(forAudioPath: audioPath, root: root), let data = try? Data(contentsOf: url),
              let spans = try? JSONDecoder().decode([RecordingPausedSpan].self, from: data) else { return [] }
        return RecordingPausedText.normalized(spans)
    }

    /// The lines of a transcript without those that lie in a paused stretch. `startCs` and `endCs` are on
    /// the recording's own clock, which is the one the spans use.
    static func dropping(_ lines: [TranscriptionService.Line], spans: [RecordingPausedSpan]) -> [TranscriptionService.Line] {
        guard !spans.isEmpty else { return lines }
        return lines.filter { line in
            !RecordingPausedText.isInsideSpan(start: Double(line.startCs) / 100, end: Double(line.endCs) / 100, spans: spans)
        }
    }

    /// Live segments, for what the live transcript shows while the meeting records.
    static func dropping(_ segments: [CoreLiveSegment], spans: [RecordingPausedSpan]) -> [CoreLiveSegment] {
        guard !spans.isEmpty else { return segments }
        return segments.filter { !RecordingPausedText.isInsideSpan(start: Double($0.startCs) / 100, end: Double($0.endCs) / 100, spans: spans) }
    }
}
