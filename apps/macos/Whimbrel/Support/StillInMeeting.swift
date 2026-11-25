import Foundation
import os
import UserNotifications

/// "Still in a meeting?" for a recording started by hand: 15 minutes with no transcribed
/// speech asks; "Stop" or 2 minutes with no answer stops the recording untrimmed and marks it
/// for an auto-trim: once its full transcription is stored, the transcription queue trims it
/// back to the last stored speech plus 30 s (`autoTrimKeepMs`), so audio live transcription
/// hadn't heard yet is never cut (docs/superpowers/specs/2026-11-05-trim-and-still-in-meeting-design.md).
/// Only a line of speech resets the quiet clock: at least `minSpeechWords` words with letters
/// and `minSpeechCs` (1 s) of audio, so filler like "Thank you." from near-silent audio does not.
/// Words are counted for any script (`speechWordCount`); a run of 2 or more letters in a script
/// written without spaces (Japanese, Chinese, Thai, Lao, Khmer, Myanmar) is enough on its own.
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
        let quietSince = quietSince(recordingStartedAt: recordingStartedAt, lastSpeechAt: lastSpeechAt,
                                    speechArrivedAt: speechArrivedAt, keptAt: keptAt)
        return now.timeIntervalSince(quietSince) >= Double(quietSeconds) ? .prompt : .none
    }

    /// When the quiet clock last started: the latest of the recording's start, the last speech,
    /// the last speech arrival and the last "Keep recording".
    static func quietSince(recordingStartedAt: Date, lastSpeechAt: Date?, speechArrivedAt: Date?, keptAt: Date?) -> Date {
        [recordingStartedAt, lastSpeechAt, speechArrivedAt, keptAt].compactMap { $0 }.max()!
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

    /// The shortest line, in centiseconds, that can count as speech. Near-silent audio makes
    /// brief filler ("Thank you.") that is not someone talking.
    static let minSpeechCs: Int64 = 100
    /// The fewest words with letters a line needs to count as speech.
    static let minSpeechWords = 2

    /// How many words with letters `text` holds, for the filler rule. A word is a run of
    /// characters between spaces and punctuation that has at least one letter (any script),
    /// so digits and symbols alone are not words. Japanese, Chinese, Thai and the other
    /// scripts written without spaces have no word breaks to count, so a run of 2 or more
    /// letters in such a script counts as `minSpeechWords` on its own (one letter counts as one).
    static func speechWordCount(_ text: String) -> Int {
        var count = 0
        var spaced = false      // the current run has a letter in a script that uses spaces
        var unspacedRun = 0     // letters in a row of a no-space script
        var inWord = false
        func endWord() {
            if unspacedRun >= 2 { count += minSpeechWords } else if spaced || unspacedRun == 1 { count += 1 }
            spaced = false; unspacedRun = 0; inWord = false
        }
        for scalar in text.unicodeScalars {
            let isLetter = scalar.properties.isAlphabetic || scalar.properties.generalCategory == .otherLetter
            let isMark = scalar.properties.generalCategory == .nonspacingMark || scalar.properties.generalCategory == .spacingMark
            let isJoiner = scalar == "'" || scalar == "\u{2019}" || scalar == "-"
            if isLetter {
                inWord = true
                if isNoSpaceScript(scalar) { unspacedRun += 1 } else { spaced = true }
            } else if (isMark || isJoiner) && inWord {
                continue
            } else if inWord {
                endWord()
            }
        }
        if inWord { endWord() }
        return count
    }

    /// Han, Hiragana, Katakana, Thai, Lao, Khmer and Myanmar: written without spaces.
    private static func isNoSpaceScript(_ s: Unicode.Scalar) -> Bool {
        switch s.value {
        case 0x3040...0x30FF, 0x31F0...0x31FF, 0x3400...0x4DBF, 0x4E00...0x9FFF, 0xF900...0xFAFF,
             0x20000...0x2FA1F, 0xFF66...0xFF9F, 0x0E00...0x0EFF, 0x1000...0x109F, 0x1780...0x17FF:
            return true
        default: return false
        }
    }

    /// Whether a live line is someone speaking: at least `minSpeechWords` words with letters
    /// and `minSpeechCs` of audio. Filler from near-silent audio is neither.
    static func countsAsSpeech(_ segment: CoreLiveSegment) -> Bool {
        segment.endCs - segment.startCs >= minSpeechCs && speechWordCount(segment.text) >= minSpeechWords
    }

    /// Whether `new` (live rows) holds a line of speech that `old` didn't.
    static func hasNewSpeech(old: [CoreLiveSegment], new: [CoreLiveSegment]) -> Bool {
        struct Key: Hashable { let channel: String, startCs: Int64, text: String }
        let spoken = { (rows: [CoreLiveSegment]) in
            rows.filter(countsAsSpeech)
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
            let endCs = liveSegments.filter(countsAsSpeech).map(\.endCs).max()
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

/// The unified log for the recording check-ins ("Still recording?" hourly and "Still in a
/// meeting?" quiet), so a recording that ran too long can be explained afterwards:
/// `log show --predicate 'subsystem == "com.mpointer.kestrel" AND category == "check-in"'`.
///
/// Privacy: only event names, reasons, times, the live status, the engine's type name and
/// counts or lengths are logged, all marked public because none of it is private content.
/// Transcript text, titles, names and file paths never go in.
enum CheckInLog {
    static let logger = Logger(subsystem: "com.mpointer.kestrel", category: "check-in")

    /// One line: `event=<event> reason=<reason> lastSpeechAt=<time|none> quietSince=<time|none> live=<status> engine=<name>`.
    static func line(event: String, reason: String, lastSpeechAt: Date?, quietSince: Date?,
                     liveStatus: String, engine: String, extra: String = "") -> String {
        let fields = "event=\(event) reason=\(reason) lastSpeechAt=\(time(lastSpeechAt)) quietSince=\(time(quietSince)) live=\(liveStatus) engine=\(engine)"
        return extra.isEmpty ? fields : fields + " " + extra
    }

    static func record(event: String, reason: String, lastSpeechAt: Date?, quietSince: Date?,
                       liveStatus: String, engine: String, extra: String = "") {
        let text = line(event: event, reason: reason, lastSpeechAt: lastSpeechAt, quietSince: quietSince,
                        liveStatus: liveStatus, engine: engine, extra: extra)
        logger.notice("\(text, privacy: .public)")
    }

    /// What a live-lines update brought, as counts only: lines that count as speech, filler
    /// lines the clock ignored, words with letters and characters in the speech lines.
    struct Arrival: Equatable {
        var speechLines = 0, fillerLines = 0, words = 0, characters = 0
        var extra: String { "speechLines=\(speechLines) fillerLines=\(fillerLines) words=\(words) characters=\(characters)" }
    }

    static func arrival(old: [CoreLiveSegment], new: [CoreLiveSegment]) -> Arrival {
        struct Key: Hashable { let channel: String, startCs: Int64, text: String }
        let seen = Set(old.map { Key(channel: $0.channel, startCs: $0.startCs, text: $0.text) })
        var result = Arrival()
        for row in new where !seen.contains(Key(channel: row.channel, startCs: row.startCs, text: row.text)) {
            guard !row.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { continue }
            if StillInMeeting.countsAsSpeech(row) {
                result.speechLines += 1
                result.words += StillInMeeting.speechWordCount(row.text)
                result.characters += row.text.count
            } else {
                result.fillerLines += 1
            }
        }
        return result
    }

    /// Whole seconds into a `HH:mm:ss` local time, or `none`.
    static func time(_ date: Date?) -> String {
        guard let date else { return "none" }
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "HH:mm:ss"
        return f.string(from: date)
    }
}
