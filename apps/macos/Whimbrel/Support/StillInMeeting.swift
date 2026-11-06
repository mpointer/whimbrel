import Foundation
import UserNotifications

/// "Still in a meeting?" for a recording started by hand: 15 minutes with no transcribed
/// speech asks; "Stop" or 2 minutes with no answer stops the recording untrimmed and marks it
/// for an auto-trim: once its full transcription is stored, the transcription queue trims it
/// back to the last stored speech plus 30 s (`autoTrimKeepMs`), so audio live transcription
/// hadn't heard yet is never cut (docs/superpowers/specs/2026-11-05-trim-and-still-in-meeting-design.md).
/// Pure decision logic; AppState's 5 s check-in poll acts on it. Wall-clock times, so a
/// Mac that slept through the quiet minutes asks on wake.
struct StillInMeeting {
    static let quietSeconds = 900
    static let answerSeconds = 120
    static let tailSeconds = 30

    static let notificationCategory = "still-in-meeting"
    static let keepAction = "keep-recording"
    static let stopAction = "stop"

    /// `.prompt`: the prompt should be showing. `.none` while it shows: speech came, so it goes.
    enum Action: Equatable { case none, prompt, stop }

    static func applies(to trigger: CoreMeetingTrigger) -> Bool { trigger == .manual }

    /// `speechArrivedAt`: when a live line with text last arrived, whatever its audio time.
    static func state(now: Date, recordingStartedAt: Date, lastSpeechAt: Date?, speechArrivedAt: Date? = nil,
                      promptShownAt: Date?, keptAt: Date?) -> Action {
        if let shown = promptShownAt {
            if let speech = lastSpeechAt, speech > shown { return .none }
            if let arrived = speechArrivedAt, arrived > shown { return .none }
            guard now.timeIntervalSince(shown) >= Double(answerSeconds) else { return .prompt }
            return .stop
        }
        let quietSince = [recordingStartedAt, lastSpeechAt, speechArrivedAt, keptAt].compactMap { $0 }.max()!
        return now.timeIntervalSince(quietSince) >= Double(quietSeconds) ? .prompt : .none
    }

    /// The poll runs every 5 s, so a longer gap means the Mac slept.
    static let sleepGapSeconds = 30

    /// When the prompt was shown, for this poll: a prompt whose answer window ran out while
    /// the Mac slept was never seen, so it shows again now with a fresh window.
    static func promptShownAt(now: Date, lastPollAt: Date?, promptShownAt: Date?) -> Date? {
        guard let shown = promptShownAt, let last = lastPollAt,
              now.timeIntervalSince(last) > Double(sleepGapSeconds),
              now.timeIntervalSince(shown) >= Double(answerSeconds) else { return promptShownAt }
        return now
    }

    /// The auto-trim's keep (meeting clock, ms) after the full transcription: the end of the
    /// last line with text plus the tail. Stored lines carry only their start, so the part
    /// just transcribed gives the ends (`partLines`, part clock, `partOffsetMs` into the
    /// meeting). Nil (no trim) when no line has text or the keep isn't shorter than `lengthMs`.
    static func autoTrimKeepMs(stored: [TranscriptionService.Line], partLines: [TranscriptionService.Line],
                               partOffsetMs: Int64, lengthMs: Int64) -> Int64? {
        let spoken = { (l: TranscriptionService.Line) in !l.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        let ends = stored.filter(spoken).map { $0.startCs * 10 }
            + partLines.filter(spoken).map { partOffsetMs + max($0.startCs, $0.endCs) * 10 }
        guard let last = ends.max() else { return nil }
        let keep = last + Int64(tailSeconds) * 1000
        return keep < lengthMs ? keep : nil
    }

    /// Whether `new` (live rows) holds a line with text that `old` didn't.
    static func hasNewSpeech(old: [CoreLiveSegment], new: [CoreLiveSegment]) -> Bool {
        struct Key: Hashable { let channel: String, startCs: Int64, text: String }
        let spoken = { (rows: [CoreLiveSegment]) in
            rows.filter { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
                .map { Key(channel: $0.channel, startCs: $0.startCs, text: $0.text) }
        }
        return !Set(spoken(new)).isSubset(of: Set(spoken(old)))
    }

    /// The last speech: the end of the latest live line with text while live transcription
    /// listens (its times are from the recording's start; a manual recording never continues,
    /// so that is the meeting's start), else the last audible moment. `start()` resets
    /// `lastNonSilentAt` to its own time, so a moment within 5 s of the start is no speech.
    static func lastSpeechAt(recordingStartedAt: Date, liveListening: Bool, liveSegments: [CoreLiveSegment],
                             lastNonSilentAt: Date) -> Date? {
        if liveListening {
            let endCs = liveSegments.filter { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }.map(\.endCs).max()
            return endCs.map { recordingStartedAt.addingTimeInterval(Double($0) / 100) }
        }
        return lastNonSilentAt.timeIntervalSince(recordingStartedAt) > 5 ? lastNonSilentAt : nil
    }

    static var category: UNNotificationCategory {
        UNNotificationCategory(
            identifier: notificationCategory,
            actions: [UNNotificationAction(identifier: keepAction, title: String(localized: "Keep recording")),
                      UNNotificationAction(identifier: stopAction, title: String(localized: "Stop"), options: .destructive)],
            intentIdentifiers: [])
    }
}
