import EventKit
import Foundation
import OSLog

/// How a `QualifyingMeeting` was found — calendar (FR-MTG-1) or heuristically detected
/// with no matching calendar entry (FR-MTG-2). Maps directly to `CoreMeetingTrigger`
/// when the meeting is recorded into `core/memory`.
/// Raw values match core/api::connectors::calendar's `MeetingSource` JSON.
enum MeetingSource: String, Codable, Equatable {
    case calendar
    case googleCalendar
    case detected
}

/// FR-MTG-1/2: a meeting that qualifies for a recording prompt — either a calendar
/// event (conferencing link or ≥1 other attendee) or a heuristically detected one (a
/// known meeting app running with sustained bidirectional audio). Just the fields the
/// trigger/UI layers need, not the raw `EKEvent` (which isn't safely `Sendable` across
/// contexts).
struct QualifyingMeeting: Identifiable, Equatable {
    /// For calendar meetings, `EKEvent.eventIdentifier`; for detected meetings, a
    /// synthesized id — either way, stable enough to dedupe repeated prompts.
    let id: String
    let title: String
    let startDate: Date
    let endDate: Date
    /// Attendee display names (falling back to their `mailto:` address), excluding
    /// the current user. Always empty for detected meetings — there's no attendee list
    /// to read from an app/audio heuristic.
    let attendees: [String]
    let conferenceLink: DetectedConferenceLink?
    let source: MeetingSource
    /// The detected meeting app's bundle id (e.g. "us.zoom.xos") — used by the
    /// meeting-alert panel to show that app's real icon. Always nil for calendar-
    /// sourced meetings (no running app to attribute a calendar event to).
    let appBundleID: String?
    /// The organizer's address (`mailto:` stripped) and display name, when the source
    /// knows them. Always nil for detected meetings.
    let organizerEmail: String?
    let organizerName: String?
    /// True when the current user organized the meeting.
    let selfIsOrganizer: Bool
    /// The current user's own RSVP; nil when they aren't in the attendee list.
    let selfResponse: SelfResponse?
}

/// Mirrors core's `SelfResponse` (snake_case on the wire).
enum SelfResponse: String, Codable, Equatable {
    case accepted, tentative, declined
    case needsAction = "needs_action"
}

/// JSON mirror of core/api::connectors::calendar's `QualifyingMeeting` (M3.0 Task 8):
/// dates travel as Unix milliseconds.
extension QualifyingMeeting: Codable {
    enum CodingKeys: String, CodingKey {
        case id, title, attendees, source
        case startUnixMs = "start_unix_ms"
        case endUnixMs = "end_unix_ms"
        case conferenceLink = "conference_link"
        case appBundleID = "app_bundle_id"
        case organizerEmail = "organizer_email"
        case organizerName = "organizer_name"
        case selfIsOrganizer = "self_is_organizer"
        case selfResponse = "self_response"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            id: try c.decode(String.self, forKey: .id),
            title: try c.decode(String.self, forKey: .title),
            startDate: Date(timeIntervalSince1970: Double(try c.decode(Int64.self, forKey: .startUnixMs)) / 1000),
            endDate: Date(timeIntervalSince1970: Double(try c.decode(Int64.self, forKey: .endUnixMs)) / 1000),
            attendees: try c.decode([String].self, forKey: .attendees),
            conferenceLink: try c.decodeIfPresent(DetectedConferenceLink.self, forKey: .conferenceLink),
            source: try c.decode(MeetingSource.self, forKey: .source),
            appBundleID: try c.decodeIfPresent(String.self, forKey: .appBundleID),
            organizerEmail: try c.decodeIfPresent(String.self, forKey: .organizerEmail),
            organizerName: try c.decodeIfPresent(String.self, forKey: .organizerName),
            selfIsOrganizer: try c.decodeIfPresent(Bool.self, forKey: .selfIsOrganizer) ?? false,
            selfResponse: try c.decodeIfPresent(SelfResponse.self, forKey: .selfResponse)
        )
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(title, forKey: .title)
        try c.encode(Int64((startDate.timeIntervalSince1970 * 1000).rounded()), forKey: .startUnixMs)
        try c.encode(Int64((endDate.timeIntervalSince1970 * 1000).rounded()), forKey: .endUnixMs)
        try c.encode(attendees, forKey: .attendees)
        try c.encode(conferenceLink, forKey: .conferenceLink)
        try c.encode(source, forKey: .source)
        try c.encode(appBundleID, forKey: .appBundleID)
        try c.encode(organizerEmail, forKey: .organizerEmail)
        try c.encode(organizerName, forKey: .organizerName)
        try c.encode(selfIsOrganizer, forKey: .selfIsOrganizer)
        try c.encode(selfResponse, forKey: .selfResponse)
    }
}

/// FR-MTG-1: reads calendar events via EventKit (local/iCloud/Exchange accounts
/// configured in macOS) and filters to ones that qualify as meetings per
/// `QualifyingMeeting`. Google Calendar meeting detection is handled separately, by
/// `GoogleCalendarPollingService` polling Whimbrel's own OAuth connector -- the two
/// sources are merged in `AppState.allUpcomingMeetings`. Outlook via an MCP connector
/// remains unimplemented.
/// A calendar EventKit exposes, for the "always treat as meetings" allowlist below —
/// just id/title, not the full `EKCalendar` (not `Sendable`, and the UI only needs this).
struct CalendarSummary: Identifiable, Equatable {
    let id: String
    let title: String
}

@MainActor
final class CalendarService: ObservableObject {
    @Published private(set) var upcomingMeetings: [QualifyingMeeting] = []
    @Published private(set) var availableCalendars: [CalendarSummary] = []
    /// #64: real diagnostic data (see `qualify`'s `.notice` log below) showed a work
    /// CalDAV/Exchange-bridged event with EVERY optional field EventKit exposes empty --
    /// no location/notes/url, no attendees, no organizer, no availability. There's no
    /// per-event signal left to distinguish a real meeting from a personal calendar
    /// block on an account synced this thin, so instead of guessing with a heuristic
    /// (which risks flooding Whimbrel with non-meeting noise on calendars that behave
    /// normally), the user opts specific calendars in explicitly via Settings. A
    /// non-all-day event on an opted-in calendar always qualifies.
    @Published var alwaysQualifyCalendarIDs: Set<String> {
        didSet { UserDefaults.standard.set(Array(alwaysQualifyCalendarIDs), forKey: Self.alwaysQualifyDefaultsKey) }
    }

    private static let alwaysQualifyDefaultsKey = "com.mpointer.kestrel.calendarAlwaysQualifyIDs"

    private let store = EKEventStore()
    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "calendar")
    private var refreshTimer: Timer?

    /// How far back a meeting can have started and still count as "current" (covers a
    /// meeting already in progress when the app launches or refreshes).
    private let lookbehindInterval: TimeInterval = 15 * 60

    init() {
        let saved = UserDefaults.standard.array(forKey: Self.alwaysQualifyDefaultsKey) as? [String] ?? []
        alwaysQualifyCalendarIDs = Set(saved)
    }

    func start() {
        refresh()
        refreshTimer?.invalidate()
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    func stop() {
        refreshTimer?.invalidate()
        refreshTimer = nil
    }

    /// The end of the meeting window: the start of tomorrow.
    nonisolated static func todayWindowEnd(now: Date) -> Date {
        Calendar.current.startOfDay(for: now.addingTimeInterval(24 * 60 * 60))
    }

    func refresh() {
        let authStatus = EKEventStore.authorizationStatus(for: .event)
        guard authStatus == .fullAccess else {
            log.log("Calendar refresh: skipped, authorizationStatus=\(String(describing: authStatus), privacy: .public) (need .fullAccess)")
            upcomingMeetings = []
            availableCalendars = []
            return
        }
        availableCalendars = store.calendars(for: .event)
            .map { CalendarSummary(id: $0.calendarIdentifier, title: $0.title) }
            .sorted { $0.title < $1.title }
        let now = Date()
        let windowStart = now.addingTimeInterval(-lookbehindInterval)
        // Rest of the calendar day, not a fixed lookahead — so the panel reads as
        // "today's remaining meetings" and resets naturally at midnight. The trigger
        // service (MeetingTriggerService) only fires within its own lead-time window
        // before each meeting's start, so a longer list here doesn't change when it fires.
        let windowEnd = Self.todayWindowEnd(now: now)
        let predicate = store.predicateForEvents(withStart: windowStart, end: windowEnd, calendars: nil)
        let events = store.events(matching: predicate) ?? []
        let alwaysQualifyIDs = alwaysQualifyCalendarIDs
        upcomingMeetings = events.compactMap { Self.qualify($0, alwaysQualifyCalendarIDs: alwaysQualifyIDs) }
            .sorted { $0.startDate < $1.startDate }
        log.log("Calendar refresh: window \(windowStart, privacy: .public)..\(windowEnd, privacy: .public), \(events.count, privacy: .public) events in window, \(self.upcomingMeetings.count, privacy: .public) qualify as meetings")
    }

    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "calendar")

    /// FR-MTG-1: "events containing a conferencing link or ≥1 other attendee" — or,
    /// per #64 above, any non-all-day event on a calendar the user opted into
    /// `alwaysQualifyCalendarIDs`.
    private static func qualify(_ event: EKEvent, alwaysQualifyCalendarIDs: Set<String>) -> QualifyingMeeting? {
        guard let identifier = event.eventIdentifier, let startDate = event.startDate, let endDate = event.endDate else {
            return nil
        }
        let link = MeetingLinkDetector.detectLink(location: event.location, notes: event.notes, url: event.url)
        let otherAttendees = (event.attendees ?? []).filter { !$0.isCurrentUser }
        let calendarAlwaysQualifies = event.calendar.map { alwaysQualifyCalendarIDs.contains($0.calendarIdentifier) } ?? false
        guard link != nil || !otherAttendees.isEmpty || (calendarAlwaysQualifies && !event.isAllDay) else {
            // #64 diagnostic: field *presence* only, never content — calendar text can
            // be sensitive. .notice, not .debug -- .debug messages aren't persisted to
            // the unified log's data store, only visible while actively streaming (same
            // issue AudioCaptureService.logAmplitudeIfNeeded's doc comment covers).
            log.notice("qualify: dropped \(identifier, privacy: .private) — no link, no other attendees, calendar not opted in. hasLocation=\(event.location != nil, privacy: .public) hasNotes=\(event.notes != nil, privacy: .public) hasURL=\(event.url != nil, privacy: .public) rawAttendeeCount=\(event.attendees?.count ?? 0, privacy: .public) hasOrganizer=\(event.organizer != nil, privacy: .public) isAllDay=\(event.isAllDay, privacy: .public)")
            return nil
        }

        return QualifyingMeeting(
            id: identifier,
            title: event.title?.isEmpty == false ? event.title : "Untitled meeting",
            startDate: startDate,
            endDate: endDate,
            attendees: otherAttendees.map { $0.name ?? $0.url.absoluteString },
            conferenceLink: link,
            source: .calendar,
            appBundleID: nil,
            organizerEmail: event.organizer?.url.absoluteString.replacingOccurrences(of: "mailto:", with: "", options: [.anchored, .caseInsensitive]),
            organizerName: event.organizer?.name,
            selfIsOrganizer: event.organizer?.isCurrentUser ?? false,
            selfResponse: (event.attendees ?? []).first(where: \.isCurrentUser).flatMap { selfResponse(from: $0.participantStatus) }
        )
    }

    /// EventKit's RSVP for the current user, in core's vocabulary. Statuses with no
    /// RSVP meaning (delegated, completed, in process) map to nil.
    nonisolated static func selfResponse(from status: EKParticipantStatus) -> SelfResponse? {
        switch status {
        case .accepted: return .accepted
        case .tentative: return .tentative
        case .pending, .unknown: return .needsAction
        case .declined: return .declined
        default: return nil
        }
    }
}
