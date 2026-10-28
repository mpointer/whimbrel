import Foundation

/// What Today shows, read from the replica off the main thread.
@MainActor
final class TodayModel: ObservableObject {
    @Published private(set) var brief: PhoneRoutineRun?
    @Published private(set) var upNext: PhoneCalendarEvent?
    @Published private(set) var prep: PhoneMeetingPrep?
    @Published private(set) var recent: [PhoneMeeting] = []
    /// False until the first read finished, so the view can tell "loading" from "nothing".
    @Published private(set) var loaded = false

    private struct Snapshot {
        let runs: [PhoneRoutineRun]
        let event: PhoneCalendarEvent?
        let prep: PhoneMeetingPrep?
        let meetings: [PhoneMeeting]
    }

    /// The newest successful Daily Brief run dated today, from any device.
    static func briefRun(from runs: [PhoneRoutineRun], now: Date, calendar: Calendar) -> PhoneRoutineRun? {
        let start = calendar.startOfDay(for: now)
        guard let end = calendar.date(byAdding: .day, value: 1, to: start) else { return nil }
        let startMs = Int64(start.timeIntervalSince1970 * 1000)
        let endMs = Int64(end.timeIntervalSince1970 * 1000)
        return runs
            .filter { $0.routineName == "Daily Brief" && $0.status == "success" && $0.ranAtUnixMs >= startMs && $0.ranAtUnixMs < endMs }
            .max { $0.ranAtUnixMs < $1.ranAtUnixMs }
    }

    func reload(sync: SyncModel, now: Date, calendar: Calendar) async {
        func ms(_ date: Date) -> Int64 { Int64(date.timeIntervalSince1970 * 1000) }
        // Like the Mac: an event that started up to 15 minutes ago still counts as next.
        let from = ms(now.addingTimeInterval(-15 * 60))
        let endOfToday = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: now)) ?? now.addingTimeInterval(86_400)
        let snapshot = await sync.read { store -> Snapshot in
            let event = store.upcomingEvents(fromUnixMs: from, untilUnixMs: ms(endOfToday), limit: 1).first
            return Snapshot(
                runs: store.routineRuns(limit: 50),
                event: event,
                prep: event.flatMap { store.prep(eventId: $0.externalId) },
                meetings: store.recentMeetings(limit: 10)
            )
        }
        if let snapshot {
            brief = Self.briefRun(from: snapshot.runs, now: now, calendar: calendar)
            upNext = snapshot.event
            prep = snapshot.prep
            recent = snapshot.meetings
        } else {
            brief = nil; upNext = nil; prep = nil; recent = []
        }
        loaded = true
    }
}
