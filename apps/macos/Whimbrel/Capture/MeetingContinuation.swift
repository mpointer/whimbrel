import Foundation
import os

/// Meeting continue: a recording that restarts within the continue window (core/memory
/// `CONTINUE_WINDOW_MS`, 10 minutes; 3 for a native-app key without a URL) for the same calendar event, or the same meeting app
/// and URL, becomes a new part of the previous meeting instead of a new meeting.
/// `AppState.startCapture(for:)` wires this up; the window rule itself lives in core.
enum MeetingContinuation {
    /// The previous meeting a start continues, read before anything is written.
    struct Previous {
        let meeting: CoreMeeting
        let snapshotsDir: String?
    }

    /// `cal:<event id>` for calendar meetings; `app:<bundle id>|<URL without query or fragment>`
    /// for detected ones (their `id` has a timestamp in it, so it can't be used). nil means the
    /// start never continues.
    static func key(for meeting: QualifyingMeeting) -> String? {
        switch meeting.source {
        case .calendar, .googleCalendar:
            return meeting.id.isEmpty ? nil : "cal:\(meeting.id)"
        case .detected:
            guard let bundle = meeting.appBundleID, !bundle.isEmpty else { return nil }
            return "app:\(bundle)|\(meeting.conferenceLink.map { normalizedURL($0.url) } ?? "")"
        }
    }

    /// Drops the query and fragment, which carry per-join noise (`authuser`, `pwd`, tracking),
    /// lowercases the scheme and host, and trims a trailing `/`.
    static func normalizedURL(_ url: URL) -> String {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return url.absoluteString }
        parts.query = nil
        parts.fragment = nil
        parts.scheme = parts.scheme?.lowercased()
        parts.host = parts.host?.lowercased()
        if parts.path.hasSuffix("/") { parts.path.removeLast() }
        return parts.string ?? url.absoluteString
    }

    // MARK: Calendar fallback

    /// The calendar event a meeting records.
    struct CalendarEvent {
        let id: String
        let conferenceLink: DetectedConferenceLink?
        let scheduledEnd: Date
    }

    /// The calendar meeting that ended last, kept in memory by `AppState`.
    struct EndedCalendarMeeting {
        let meetingId: Int64
        let event: CalendarEvent
        let endedAt: Date
    }

    /// The window from core's `CONTINUE_WINDOW_MS`, for the in-memory check below.
    static let continueWindow: TimeInterval = 600

    /// A calendar event is prompted once, so a rejoin of its call comes in as a detected
    /// meeting with an `app:` key that can't match the stored `cal:` key. This says whether
    /// that detected start is the same call: within the window, before the event's scheduled
    /// end, the same provider, and the same URL when both sides have one. Any end reason.
    static func calendarFallbackMatches(lastEnded: EndedCalendarMeeting, detected: QualifyingMeeting, now: Date) -> Bool {
        guard detected.source == .detected,
              now.timeIntervalSince(lastEnded.endedAt) <= continueWindow,
              now < lastEnded.event.scheduledEnd,
              let eventLink = lastEnded.event.conferenceLink,
              let provider = provider(for: detected), provider == eventLink.provider else { return false }
        if let detectedURL = detected.conferenceLink?.url {
            return normalizedURL(detectedURL) == normalizedURL(eventLink.url)
        }
        return true
    }

    /// The detected meeting's provider: its browser link's, else its native app's.
    static func provider(for detected: QualifyingMeeting) -> MeetingProvider? {
        if let link = detected.conferenceLink { return link.provider }
        switch detected.appBundleID {
        case "us.zoom.xos": return .zoom
        case let id? where id.hasPrefix("com.microsoft.teams"): return .teams
        case "com.webex.meetingmanager", "Cisco-Systems.Spark": return .webex
        default: return nil
        }
    }

    /// Continue window in ms. A native-app key without a URL (`app:<bundle>|`) can't tell two
    /// back-to-back calls in the same app apart, so it gets 3 minutes; every other key (and
    /// the calendar fallback, which looks up a `cal:` key) keeps core's 10.
    static func window(forKey key: String) -> Int64 {
        key.hasPrefix("app:") && key.hasSuffix("|") ? 180_000 : 600_000
    }

    /// Reads only, so a start that loses the race has changed nothing. Call on
    /// `CoreMemoryStore.accessQueue`.
    static func findPrevious(key: String?, nowUnixMs: Int64, store: CoreMemoryStore) -> Previous? {
        guard let key,
              let id = store.findContinuableMeeting(continueKey: key, nowUnixMs: nowUnixMs, windowMs: window(forKey: key)),
              let meeting = store.getMeeting(id: id) else { return nil }
        return Previous(meeting: meeting, snapshotsDir: store.meetingSnapshotsInfo(meetingId: id)?.dir)
    }

    /// The notes to carry into the next part. Notes are stored only once a part's transcript
    /// lands, so what was typed in this run (`lastEnded`) is fresher than the stored copy.
    static func notes(stored: String?, lastEnded: (id: Int64, notes: String)?, meetingId: Int64) -> String {
        if let lastEnded, lastEnded.id == meetingId { return lastEnded.notes }
        return stored ?? ""
    }

    /// What an ending recording hands the transcription queue.
    enum PartEnding: Equatable {
        case transcribe(partIndex: Int64)
        case nothing
        /// A continued meeting whose part index is unknown: guessing part 1 would overwrite
        /// part 1's segments, so nothing is transcribed (the caller logs it).
        case unknownContinuedPart
    }

    /// A part without audio still goes through the queue when it isn't part 1: its ending can
    /// be what lets the earlier parts' summary run. A nil index means part 1 only for a new
    /// meeting (its part row failed to write; the legacy single-part path covers it).
    static func partEnding(partIndex: Int64?, continued: Bool, hasAudio: Bool) -> PartEnding {
        guard let partIndex else {
            if continued { return .unknownContinuedPart }
            return hasAudio ? .transcribe(partIndex: 1) : .nothing
        }
        return hasAudio || partIndex > 1 ? .transcribe(partIndex: partIndex) : .nothing
    }

    /// Completes the recording's part and the meeting. Call on `CoreMemoryStore.accessQueue`.
    /// A continued meeting with no part index gets its part row now: `completeMeeting` keeps
    /// part 1's audio path, so without the row this recording's audio would be owned by
    /// nothing and the orphan scan would delete it. Its start is the audio file's creation
    /// date (else the end), so a later "Transcribe again" places it roughly right.
    static func completeRecording(store: CoreMemoryStore, meetingId: Int64, partIndex: Int64?, continued: Bool,
                                  endUnixMs: Int64, audioPath: String?) {
        var partIndex = partIndex
        if partIndex == nil, continued, let audioPath {
            let created = (try? FileManager.default.attributesOfItem(atPath: audioPath))?[.creationDate] as? Date
            let startMs = created.map { Int64($0.timeIntervalSince1970 * 1000) } ?? endUnixMs
            partIndex = store.beginMeetingPart(meetingId: meetingId, startUnixMs: min(startMs, endUnixMs), continueKey: nil)
            if partIndex == nil {
                log.error("meeting \(meetingId): no part row for continued recording; its audio is unowned: \(audioPath, privacy: .public)")
            }
        }
        if let partIndex {
            store.completeMeetingPart(meetingId: meetingId, partIndex: partIndex, endUnixMs: endUnixMs, audioPath: audioPath)
        }
        store.completeMeeting(id: meetingId, endUnixMs: endUnixMs, audioPath: audioPath)
    }

    static let log = Logger(subsystem: "com.mpointer.kestrel", category: "meeting-continue")
}
