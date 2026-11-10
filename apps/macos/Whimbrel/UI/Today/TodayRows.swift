import Foundation

/// What a Today row or recap shows and which of its actions are on, decided once. Archetype
/// layouts (Terminal, Glass) style these models and wire the actions; they do not re-derive them.
/// Labels stay Whimbrel's: Start, Open, Open brief, Hide brief, Try again.
struct TodayEventRowModel {
    enum Prep: Equatable {
        /// The next event has no prep with a readable brief: "No prep yet". Only the next event says so.
        case missing
        case ready(CoreMeetingPrep)

        static func == (a: Prep, b: Prep) -> Bool {
            switch (a, b) {
            case (.missing, .missing): return true
            case let (.ready(x), .ready(y)): return x.eventId == y.eventId
            default: return false
            }
        }
    }

    let event: TodayUpcoming
    var id: String { event.id }
    var title: String { event.title }
    var start: Date { event.start }
    let timeText: String
    let relative: String?
    /// Attendee summary; empty when there are none.
    let attendees: String
    let isNext: Bool
    /// Start is offered only while no meeting is active.
    let showsStart: Bool
    /// `.ready` for any event with a brief; `.missing` only for the next event; nil otherwise.
    let prep: Prep?
    /// The topic this event belongs to, for its chip.
    let topic: TopicRef?
}

struct TodayMeetingRowModel {
    let id: Int64
    let startUnixMs: Int64
    let timeText: String
    let title: String
    /// "Transcribing…" for the meeting being processed; otherwise the usual subtitle (may be empty).
    let subtitle: String
    /// The topic this meeting belongs to, for its chip.
    let topic: TopicRef?
}

/// One meeting in Today's Upcoming section. A row opens nothing; its topic chip opens the topic.
struct TodayWeekRowModel: Identifiable {
    let id: String
    let timeText: String
    let title: String
    /// "3 invited" (the other attendees); empty when there are none.
    let invited: String
    let topic: TopicRef?
    /// The meeting's brief, for its "Prep ready" chip; nil when it has none.
    var prep: CoreMeetingPrep? = nil
}

/// One day of the Upcoming section: "Tomorrow", then e.g. "Wed, Nov 11".
struct TodayWeekDay: Identifiable {
    let id: Date
    let heading: String
    let rows: [TodayWeekRowModel]
}

enum TodayRecapModel: Equatable {
    case brief(text: String, origin: String?)
    case digest(text: String)
    case failed(message: String, retryEnabled: Bool)
    case none
}

/// One line of a schedule list: upcoming events, then the "Earlier today" label, then held meetings.
enum TodayScheduleItem: Identifiable {
    case event(TodayEventRowModel)
    case separator
    case held(TodayMeetingRowModel)

    var id: String {
        switch self {
        case let .event(e): return "event-\(e.id)"
        case .separator: return "separator"
        case let .held(m): return "held-\(m.id)"
        }
    }
}

enum TodayRows {
    /// The label between the upcoming and the already-held meetings in a schedule (Standard's wording).
    static var heldSeparator: String { String(localized: "Earlier today") }

    /// `agenda.upcomingToday.first`, with its prep state.
    static func upNext(_ content: TodayContent) -> TodayEventRowModel? {
        content.agenda.upcomingToday.first.flatMap { eventRow($0, content: content, isNext: true) }
    }

    /// Today's schedule, each list in ascending time order. With `upNextPlaced`, the up-next event
    /// is left out of `upcoming` (the caller shows it separately); `held` is the meetings already
    /// held today.
    static func schedule(_ content: TodayContent, upNextPlaced: Bool) -> (upcoming: [TodayEventRowModel], held: [TodayMeetingRowModel]) {
        let agenda = content.agenda
        let ids = upNextPlaced ? Array(agenda.upcomingToday.dropFirst()) : agenda.upcomingToday
        let upcoming = ids.compactMap { eventRow($0, content: content, isNext: false) }.sorted { $0.start < $1.start }
        return (upcoming, meetings(agenda.pastToday, content: content))
    }

    /// `schedule`'s two lists as one sequence, with the separator only when both have rows.
    static func scheduleItems(_ schedule: (upcoming: [TodayEventRowModel], held: [TodayMeetingRowModel])) -> [TodayScheduleItem] {
        schedule.upcoming.map(TodayScheduleItem.event)
            + (schedule.upcoming.isEmpty || schedule.held.isEmpty ? [] : [.separator])
            + schedule.held.map(TodayScheduleItem.held)
    }

    /// The "earlier" meetings, newest first as the agenda lists them.
    static func earlier(_ content: TodayContent) -> [TodayMeetingRowModel] {
        content.agenda.earlier.compactMap { meetingRow($0, content: content) }
    }

    /// The Daily Brief, else the digest, else the digest error with a Try again that is off while generating.
    static func recap(_ content: TodayContent) -> TodayRecapModel {
        if let run = content.briefRecap {
            return .brief(text: run.displaySummary, origin: run.originDeviceId.map(content.deviceLabel(for:)))
        } else if let digest = content.digest {
            return .digest(text: digest.summary)
        } else if let message = content.digestError {
            return .failed(message: message, retryEnabled: !content.isGeneratingDigest)
        }
        return .none
    }

    /// Days shown before "Show the rest of the week".
    static let upcomingCollapsedDays = 2
    /// Per device, like the rest of Today's view state.
    static let upcomingExpandedKey = "com.mpointer.kestrel.todayUpcomingExpanded"

    /// The Upcoming section's days, only those with meetings, earliest first; the first
    /// `upcomingCollapsedDays` unless `expanded`.
    static func upcomingDays(_ content: TodayContent, expanded: Bool, calendar: Calendar = .current) -> [TodayWeekDay] {
        let byDay = Dictionary(grouping: content.upcomingWeek) { calendar.startOfDay(for: $0.startDate) }
        let tomorrow = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: content.now))
        let days = byDay.keys.sorted().map { day in
            TodayWeekDay(
                id: day,
                heading: day == tomorrow ? String(localized: "Tomorrow") : day.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day()),
                rows: byDay[day, default: []].sorted { $0.startDate < $1.startDate }.map { m in
                    TodayWeekRowModel(
                        // A recurring EventKit event repeats its id across occurrences; the start tells them apart.
                        id: "\(m.id)@\(m.startDate.timeIntervalSince1970)", timeText: MeetingRowText.time(m.startDate), title: m.title,
                        invited: m.attendees.isEmpty ? "" : String(localized: "\(m.attendees.count) invited"),
                        topic: content.topicsByEvent[m.id], prep: content.prep(eventId: m.id, start: m.startDate)
                    )
                }
            )
        }
        return expanded ? days : Array(days.prefix(upcomingCollapsedDays))
    }

    /// Whether the section has more days than it shows collapsed (so it needs the toggle).
    static func upcomingHasMore(_ content: TodayContent, calendar: Calendar = .current) -> Bool {
        Set(content.upcomingWeek.map { calendar.startOfDay(for: $0.startDate) }).count > upcomingCollapsedDays
    }

    private static func eventRow(_ id: String, content: TodayContent, isNext: Bool) -> TodayEventRowModel? {
        guard let event = content.upcoming.first(where: { $0.id == id }) else { return nil }
        let source = content.events.first { $0.id == id }
        var prep: TodayEventRowModel.Prep?
        if let p = content.prep(eventId: event.id, start: event.start) {
            prep = .ready(p)
        } else if isNext {
            if let p = content.nextPrep, TodayLogic.prep(p, isFor: event), p.brief != nil { prep = .ready(p) } else { prep = .missing }
        }
        return TodayEventRowModel(
            event: event,
            timeText: MeetingRowText.time(event.start),
            relative: source.flatMap { TodayLogic.relativeStart(event.start, end: $0.endDate, now: content.pinnedNow ?? Date()) },
            attendees: MeetingRowText.attendeeSummary(source?.attendees ?? []),
            isNext: isNext, showsStart: content.activeMeetingId == nil, prep: prep, topic: content.topicsByEvent[id]
        )
    }

    private static func meetings(_ ids: [Int64], content: TodayContent) -> [TodayMeetingRowModel] {
        ids.compactMap { meetingRow($0, content: content) }.sorted { $0.startUnixMs < $1.startUnixMs }
    }

    private static func meetingRow(_ id: Int64, content: TodayContent) -> TodayMeetingRowModel? {
        guard let meeting = content.recentMeetings.first(where: { $0.id == id }) else { return nil }
        return TodayMeetingRowModel(
            id: id, startUnixMs: meeting.startUnixMs, timeText: MeetingRowText.time(meeting.startUnixMs), title: meeting.title,
            subtitle: content.processingMeetingId == id ? String(localized: "Transcribing…") : MeetingRowText.subtitle(for: meeting),
            topic: content.topicsByMeeting[id]
        )
    }
}
