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
    /// `attendees` with their emails, for naming the recording's speakers.
    var attendeeDetails: [CoreAttendee] = []
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
        case attendeeDetails = "attendee_details"
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
            selfResponse: try c.decodeIfPresent(SelfResponse.self, forKey: .selfResponse),
            attendeeDetails: try c.decodeIfPresent([CoreAttendee].self, forKey: .attendeeDetails) ?? []
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
        try c.encode(attendeeDetails, forKey: .attendeeDetails)
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
    /// Today's "Upcoming" section: the 7 days after today, all-day events left out. Kept apart
    /// from `upcomingMeetings` so the meeting trigger and today's schedule don't see it.
    @Published private(set) var upcomingWeek: [QualifyingMeeting] = []
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

    /// The Upcoming window: the 7 calendar days after today, by calendar arithmetic (DST-safe).
    nonisolated static func weekWindow(now: Date) -> Range<Date> {
        let cal = Calendar.current, today = cal.startOfDay(for: now)
        let start = cal.date(byAdding: .day, value: 1, to: today) ?? today
        return start..<(cal.date(byAdding: .day, value: 7, to: start) ?? start)
    }

    func refresh() {
        let authStatus = EKEventStore.authorizationStatus(for: .event)
        guard authStatus == .fullAccess else {
            log.log("Calendar refresh: skipped, authorizationStatus=\(String(describing: authStatus), privacy: .public) (need .fullAccess)")
            upcomingMeetings = []
            upcomingWeek = []
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
        let week = Self.weekWindow(now: now)
        let weekEvents = store.events(matching: store.predicateForEvents(withStart: week.lowerBound, end: week.upperBound, calendars: nil))
        upcomingWeek = weekEvents.filter { !$0.isAllDay && week.contains($0.startDate) }
            .compactMap { Self.qualify($0, alwaysQualifyCalendarIDs: alwaysQualifyIDs, logDrops: false) }
            .sorted { $0.startDate < $1.startDate }
        log.log("Calendar refresh: window \(windowStart, privacy: .public)..\(windowEnd, privacy: .public), \(events.count, privacy: .public) events in window, \(self.upcomingMeetings.count, privacy: .public) qualify as meetings, \(self.upcomingWeek.count, privacy: .public) in the next 7 days")
    }

    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "calendar")

    /// FR-MTG-1: "events containing a conferencing link or ≥1 other attendee" — or,
    /// per #64 above, any non-all-day event on a calendar the user opted into
    /// `alwaysQualifyCalendarIDs`.
    /// `logDrops: false` for the Upcoming pass, so the #64 diagnostic isn't repeated for a week of events every minute.
    private static func qualify(_ event: EKEvent, alwaysQualifyCalendarIDs: Set<String>, logDrops: Bool = true) -> QualifyingMeeting? {
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
            if logDrops { log.notice("qualify: dropped \(identifier, privacy: .private) — no link, no other attendees, calendar not opted in. hasLocation=\(event.location != nil, privacy: .public) hasNotes=\(event.notes != nil, privacy: .public) hasURL=\(event.url != nil, privacy: .public) rawAttendeeCount=\(event.attendees?.count ?? 0, privacy: .public) hasOrganizer=\(event.organizer != nil, privacy: .public) isAllDay=\(event.isAllDay, privacy: .public)") }
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
            selfResponse: (event.attendees ?? []).first(where: \.isCurrentUser).flatMap { selfResponse(from: $0.participantStatus) },
            attendeeDetails: otherAttendees.map(attendee(from:))
        )
    }

    nonisolated static func attendee(from p: EKParticipant) -> CoreAttendee { attendee(name: p.name, url: p.url) }

    /// The name, else the address; the email is the `mailto:` address (none for another URL).
    nonisolated static func attendee(name: String?, url: URL) -> CoreAttendee {
        let raw = url.absoluteString
        let email = raw.range(of: "mailto:", options: [.anchored, .caseInsensitive]).map { String(raw[$0.upperBound...]) }
        return CoreAttendee(name: name ?? email ?? raw, email: email.flatMap { $0.isEmpty ? nil : $0 })
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
