import Foundation

/// A calendar meeting that has not started yet, as Today needs it.
struct TodayUpcoming: Equatable {
    let id: String
    let title: String
    let start: Date
}

/// What fills Today's top slot. See docs/superpowers/specs/2026-10-06-today-rework-design.md.
enum TodayHero: Equatable {
    case live(meetingId: Int64)
    case upcoming(eventId: String)
    case noCalendar
    case noMeetings
    case none
}

/// Today's meeting list: ids only, so the view maps them back to its own models.
struct TodayAgenda: Equatable {
    var upcomingToday: [String]
    var pastToday: [Int64]
    var earlier: [Int64]
}

enum TodayLogic {
    static func greeting(name: String, now: Date, calendar: Calendar) -> String? {
        ReadLogic.greeting(name: name, now: now, calendar: calendar)
    }

    /// The skin's greeting for the hour, with the first word of the profile name; nil with no usable name.
    static func greeting(name: String, now: Date, calendar: Calendar, voice: SkinVoice) -> String? {
        guard greeting(name: name, now: now, calendar: calendar) != nil,
              let first = name.split(whereSeparator: \.isWhitespace).first else { return nil }
        return voice.text(greetingKey(now: now, calendar: calendar), name: String(first))
    }

    static func greetingKey(now: Date, calendar: Calendar) -> SkinVoiceKey {
        switch ReadLogic.daypart(now: now, calendar: calendar) {
        case "morning": return .greetingMorning
        case "afternoon": return .greetingAfternoon
        default: return .greetingEvening
        }
    }

    static func relativeStart(_ start: Date, end: Date, now: Date) -> String? {
        ReadLogic.relativeStart(start, end: end, now: now)
    }

    /// The Daily Brief Today shows as its recap: the newest successful run dated today in this
    /// calendar's day, from any device. Matched by routine name because the persisted-run shape
    /// carries no template; nil means "no brief yet today", and Today falls back to the on-demand digest.
    static func recapRun(from runs: [CoreRoutineRunWithName], now: Date, calendar: Calendar) -> CoreRoutineRunWithName? {
        let start = calendar.startOfDay(for: now)
        guard let end = calendar.date(byAdding: .day, value: 1, to: start) else { return nil }
        let startMs = Int64(start.timeIntervalSince1970 * 1000)
        let endMs = Int64(end.timeIntervalSince1970 * 1000)
        return runs
            .filter { $0.routineName == "Daily Brief" && $0.status == "success" && $0.ranAtUnixMs >= startMs && $0.ranAtUnixMs < endMs }
            .max { $0.ranAtUnixMs < $1.ranAtUnixMs }
    }

    /// Drops upcoming events that were already recorded: `AppState.startCapture(for:)` stores
    /// the meeting with the event's own start (unix ms) and title, so an exact match means the
    /// call is live or done and must not also show as upcoming.
    static func upcomingExcludingRecorded(
        upcoming: [TodayUpcoming], recent: [(id: Int64, startUnixMs: Int64, title: String)]
    ) -> [TodayUpcoming] {
        upcoming.filter { event in
            let startMs = Int64(event.start.timeIntervalSince1970 * 1000)
            return !recent.contains { $0.startUnixMs == startMs && $0.title == event.title }
        }
    }

    /// An active meeting that is in the recent list (or any active meeting while that list has not
    /// loaded yet) wins; then the first upcoming meeting if it is today; with no meetings at all, a
    /// calendar or start-a-note empty state; else nothing.
    static func hero(
        activeMeetingId: Int64?, recentMeetingIds: [Int64], recentLoaded: Bool = true, upcoming: [TodayUpcoming],
        calendarGranted: Bool, now: Date, calendar: Calendar
    ) -> TodayHero {
        if let activeMeetingId, !recentLoaded || recentMeetingIds.contains(activeMeetingId) {
            return .live(meetingId: activeMeetingId)
        }
        if let next = upcoming.min(by: { $0.start < $1.start }), calendar.isDate(next.start, inSameDayAs: now) {
            return .upcoming(eventId: next.id)
        }
        if recentMeetingIds.isEmpty, upcoming.isEmpty {
            return calendarGranted ? .noMeetings : .noCalendar
        }
        return .none
    }

    /// Today: upcoming meetings (earliest first) then meetings already held today (newest first).
    /// Earlier: the other recent meetings, newest first, capped. The active meeting is left out
    /// because the hero shows it.
    static func agenda(
        upcoming: [TodayUpcoming], recent: [(id: Int64, startUnixMs: Int64)], excludingMeetingId: Int64?,
        now: Date, calendar: Calendar, earlierLimit: Int
    ) -> TodayAgenda {
        let upcomingToday = upcoming
            .filter { calendar.isDate($0.start, inSameDayAs: now) }
            .sorted { $0.start < $1.start }
            .map(\.id)
        let kept = recent
            .filter { $0.id != excludingMeetingId }
            .sorted { $0.startUnixMs > $1.startUnixMs }
        func isToday(_ item: (id: Int64, startUnixMs: Int64)) -> Bool {
            calendar.isDate(Date(timeIntervalSince1970: Double(item.startUnixMs) / 1000), inSameDayAs: now)
        }
        return TodayAgenda(
            upcomingToday: upcomingToday,
            pastToday: kept.filter(isToday).map(\.id),
            earlier: Array(kept.filter { !isToday($0) }.prefix(earlierLimit).map(\.id))
        )
    }
}
