import Foundation
import os

/// Back-to-back meetings. A recording ends only when `MeetingEndDetector` sees the call end,
/// and going straight from one call into the next (often in the same app) never drops that
/// signal, while the next meeting's calendar prompt can't start anything during a recording.
/// So when the next calendar meeting starts during a recording, `AppState` asks
/// `shouldSplit`, ends the recording and starts the next meeting. "Keep together" undoes it
/// the way meeting continue joins a restart: the earlier meeting gets the recording as its
/// next part (`rejoin`) and the new meeting is deleted (`discard`).
enum MeetingSplit {
    /// The recording in progress.
    struct Current {
        /// The calendar event it records; nil for a manual or detected meeting.
        let calendarEvent: MeetingContinuation.CalendarEvent?
        /// The call's link when known (the event's, or a browser detection's).
        let link: DetectedConferenceLink?
        /// When the meeting began (its first part's start for a continued or joined one).
        let meetingStartedAt: Date
    }

    /// A manual or detected recording that began this close before an event is taken to be
    /// that event, started early.
    // ponytail: fixed 10 min guess; a link match can't replace it (personal rooms reuse links).
    static let sameEventLead: TimeInterval = 600

    /// Whether `next` starting now should end the current recording and start its own.
    /// `liveCallURL` is the call's current URL where it can be read (a Google Meet tab), nil
    /// otherwise; it is read only once `next` has started.
    static func shouldSplit(current: Current, next: QualifyingMeeting, liveCallURL: () -> String?, now: Date) -> Bool {
        guard next.source != .detected, next.id != current.calendarEvent?.id,
              next.startDate > current.meetingStartedAt, now >= next.startDate, now < next.endDate,
              next.selfResponse != .declined else { return false }
        let live = liveCallURL().flatMap(URL.init(string:)).map(MeetingContinuation.normalizedURL)
        let currentLink = current.link.map { MeetingContinuation.normalizedURL($0.url) }
        let nextLink = next.conferenceLink.map { MeetingContinuation.normalizedURL($0.url) }
        let inNextCall = live != nil && live == nextLink
        // Tentative or unanswered: only when it is the call he's in. No RSVP counts as accepted
        // only on his own event (no other attendees).
        let accepted = next.selfIsOrganizer || next.selfResponse == .accepted
            || (next.selfResponse == nil && next.attendees.isEmpty)
        guard accepted || inNextCall else { return false }
        // Rule 2: the call itself changed.
        let linkChanged = live != nil && currentLink != nil && live != currentLink
        guard let event = current.calendarEvent else {
            // Rule 3: manual or detected, unless it was started early for this same event.
            return current.meetingStartedAt < next.startDate.addingTimeInterval(-sameEventLead) || linkChanged
        }
        // Rule 1: the current event reached its scheduled end.
        return now >= event.scheduledEnd || linkChanged || (inNextCall && currentLink != nextLink)
    }

    /// What a calendar prompt held back during a recording does on each check.
    enum HoldAction: Equatable {
        case wait
        /// The recording ended on its own: show the prompt as usual.
        case prompt
        case split
        /// The next meeting ended, left the calendar or was declined, without a split.
        case drop
    }

    /// `next` is the held event as the calendar lists it now (nil: gone). `recording` covers a
    /// start still in flight; `current` is nil until its row lands.
    static func holdAction(recording: Bool, current: Current?, next: QualifyingMeeting?,
                           liveCallURL: () -> String?, now: Date) -> HoldAction {
        guard let next, next.selfResponse != .declined, now < next.endDate else { return .drop }
        guard recording else { return .prompt }
        guard let current else { return .wait }
        return shouldSplit(current: current, next: next, liveCallURL: liveCallURL, now: now) ? .split : .wait
    }

    struct Rejoined {
        let meeting: CoreMeeting
        let partIndex: Int64
    }

    /// "Keep together", step 1: reopens the earlier meeting with a new part starting where
    /// the split recording did; undone if the part can't be written. Call on
    /// `CoreMemoryStore.accessQueue`. A caller that then loses the race undoes it with
    /// `undoMeetingReopen`.
    static func rejoin(store: CoreMemoryStore, meetingId: Int64, partStartUnixMs: Int64) -> Rejoined? {
        guard let meeting = store.getMeeting(id: meetingId) else { return nil }
        store.reopenMeeting(id: meetingId)
        guard let partIndex = store.beginMeetingPart(meetingId: meetingId, startUnixMs: partStartUnixMs, continueKey: nil) else {
            store.undoMeetingReopen(meetingId: meetingId, partIndex: 0)
            return nil
        }
        return Rejoined(meeting: meeting, partIndex: partIndex)
    }

    /// "Keep together", step 2: moves the split-off meeting's slides (rows and files) and
    /// continue key to the earlier meeting. Files are copied into the earlier meeting's folder
    /// (`makeFolder` when it has none), the rows repointed, and only then the originals
    /// deleted. False when the files couldn't move: the rows (now the earlier meeting's) keep
    /// pointing at the originals, and the split-off folder is marked kept so `discard` leaves
    /// it. nil when the rows couldn't move: the caller must not `discard` then. Call on
    /// `CoreMemoryStore.accessQueue`.
    static func absorb(store: CoreMemoryStore, into: Int64, from: Int64,
                       makeFolder: (_ start: Date, _ title: String) throws -> URL) -> Bool? {
        guard let paths = store.absorbSplitMeeting(into: into, from: from) else { return nil }
        guard !paths.isEmpty else { return true }
        let fm = FileManager.default
        var copies: [(old: String, new: String)] = []
        do {
            let existing = store.meetingSnapshotsInfo(meetingId: into)?.dir
                .map { URL(fileURLWithPath: $0, isDirectory: true) }
                .flatMap { fm.fileExists(atPath: $0.path) ? $0 : nil }
            let folder = try existing ?? {
                guard let m = store.getMeeting(id: into) else { throw CocoaError(.fileNoSuchFile) }
                return try makeFolder(Date(timeIntervalSince1970: Double(m.startUnixMs) / 1000), m.title)
            }()
            for old in paths where fm.fileExists(atPath: old) {
                var dest = folder.appendingPathComponent((old as NSString).lastPathComponent)
                if fm.fileExists(atPath: dest.path) {
                    dest = folder.appendingPathComponent("\(UUID().uuidString.prefix(8))-\(dest.lastPathComponent)")
                }
                try fm.copyItem(atPath: old, toPath: dest.path)
                copies.append((old, dest.path))
            }
            guard store.setMeetingSnapshotPaths(meetingId: into, moves: copies, snapshotsDir: folder.path) else {
                throw CocoaError(.fileWriteUnknown)
            }
            for c in copies { try? fm.removeItem(atPath: c.old) }
            return true
        } catch {
            log.error("keep together: moving \(paths.count) slides from meeting \(from) failed: \(String(describing: error))")
            for c in copies { try? fm.removeItem(atPath: c.new) }
            store.setKeepSnapshots(meetingId: from, keep: true)
            return false
        }
    }

    /// "Keep together", step 3: deletes the split-off meeting and its snapshot files. It is
    /// still recording, so it has no audio path yet (written at completion) and was never
    /// synced. Call on `CoreMemoryStore.accessQueue`.
    @discardableResult
    static func discard(store: CoreMemoryStore, meetingId: Int64, recordingsDir: URL = SpikePaths.root) -> Bool {
        guard let files = store.deleteMeetingReturningFiles(id: meetingId) else { return false }
        CoreMemoryStore.deleteAllMeetingFiles(files, recordingsDir: recordingsDir)
        return true
    }

    static let log = Logger(subsystem: "com.mpointer.kestrel", category: "meeting-split")
}
