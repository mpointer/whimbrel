import AVFoundation
import Foundation

/// Finds the recording a crashed session left behind, by the stamp in its file name
/// (`system-audio-<stamp>.caf` / `mic-audio-<stamp>.caf`, `SpikePaths.sessionTimestamp()`),
/// so launch can re-link it to its still-`.recording` meeting (spec §6).
enum StrandedAudioMatch {
    /// Wide because a manual start's file stamp can land minutes after its row when the
    /// store queue is busy (R9); closest-unreferenced keeps neighbouring meetings apart.
    static let window: TimeInterval = 30 * 60

    private static let system = "system-audio-"
    private static let mic = "mic-audio-"
    private static let ext = ".caf"

    /// The file name to use as audioPath (system preferred, else mic) for a meeting that started at `start`,
    /// choosing the stamp closest to `start` within [start, start+window]; nil when none qualifies.
    /// `referenced` holds file names that meetings already use; a stamp is taken if either sibling is referenced.
    static func audioFileName(meetingStart: Date, fileNames: [String], referenced: Set<String>) -> String? {
        let names = Set(fileNames)
        var best: (stamp: String, offset: TimeInterval)?
        for name in names {
            guard let stamp = stamp(of: name), let date = SpikePaths.sessionStartDate(from: stamp) else { continue }
            let offset = date.timeIntervalSince(meetingStart)
            guard offset >= 0, offset <= window,
                  !referenced.contains(system + stamp + ext), !referenced.contains(mic + stamp + ext),
                  offset < (best?.offset ?? .infinity) else { continue }
            best = (stamp, offset)
        }
        guard let stamp = best?.stamp else { return nil }
        return names.contains(system + stamp + ext) ? system + stamp + ext : mic + stamp + ext
    }

    /// The `<stamp>` of a `system-audio-<stamp>.caf` / `mic-audio-<stamp>.caf` name.
    private static func stamp(of name: String) -> String? {
        guard name.hasSuffix(ext), let prefix = [system, mic].first(where: name.hasPrefix) else { return nil }
        return String(name.dropFirst(prefix.count).dropLast(ext.count))
    }

    /// When the recording at `url` stopped: its stamp plus the longer of its system/mic
    /// pair's lengths (one stream can stop early); nil when the stamp or both files are unreadable.
    static func recordingEnd(of url: URL) -> Date? {
        guard let stamp = stamp(of: url.lastPathComponent), let start = SpikePaths.sessionStartDate(from: stamp) else { return nil }
        let dir = url.deletingLastPathComponent()
        let seconds = [system, mic].compactMap { prefix -> TimeInterval? in
            guard let file = try? AVAudioFile(forReading: dir.appendingPathComponent(prefix + stamp + ext)),
                  file.processingFormat.sampleRate > 0 else { return nil }
            return Double(file.length) / file.processingFormat.sampleRate
        }.max()
        return seconds.map { start.addingTimeInterval($0) }
    }

    /// File names of `paths`, each with its system/mic sibling: one recording is a pair.
    static func referencedNames(paths: [String?]) -> Set<String> {
        var out = Set<String>()
        for case let path? in paths {
            let name = (path as NSString).lastPathComponent
            out.insert(name)
            if name.hasPrefix(system) { out.insert(mic + name.dropFirst(system.count)) }
            if name.hasPrefix(mic) { out.insert(system + name.dropFirst(mic.count)) }
        }
        return out
    }

    /// The part a crash interrupted: the latest, since only it can be recording (an
    /// earlier end-NULL part is stale).
    static func openPart(_ parts: [CoreMeetingPart]) -> CoreMeetingPart? {
        parts.max { $0.partIndex < $1.partIndex }
    }
}
