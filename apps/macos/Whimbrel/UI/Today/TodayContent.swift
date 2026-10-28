import Foundation

/// The parts of Today a layout can place. `nonEmptySections()` lists the ones with something to show.
enum TodaySection: CaseIterable {
    case greeting, composer, upNext, schedule, earlier, threads, followups, recap, devices
}

/// Everything Today shows, as plain values: no store, no `AppState`. `TodayView` builds one from
/// its state; layouts and snapshot tests read it. `hero`, `agenda` and `upcoming` are pure
/// functions over `TodayLogic`, recomputed for the `now` the caller passes.
struct TodayContent {
    var name: String
    var now: Date
    /// When set, the schedule card uses this instead of its ticking clock. Fixtures only.
    var pinnedNow: Date?
    /// Calendar events as `AppState.allUpcomingMeetings` reports them (may include ended ones).
    var events: [QualifyingMeeting]
    var recentMeetings: [CoreMeeting]
    /// False until `TodayView` has fetched `recentMeetings` once (a freshly built Today, e.g. after a skin switch).
    var recentLoaded = true
    var calendarGranted: Bool
    var activeMeetingId: Int64?
    var processingMeetingId: Int64?
    var nextPrep: CoreMeetingPrep?
    var pendingApprovalCount: Int
    var attentionThreads: [CoreThread]
    var briefRecap: CoreRoutineRunWithName?
    var digest: CoreDailyDigest?
    var digestError: String?
    var isGeneratingDigest: Bool
    var deviceId: String
    var deviceRows: [CoreDeviceActivity]
    /// Display labels by device id (this Mac, the other devices, a recap's origin).
    var deviceLabels: [String: String]
    /// Open to-dos Away Mode left behind.
    var followups: [CoreMeetingFollowup] = []

    private var clock: Date { pinnedNow ?? Date() }

    func deviceLabel(for id: String) -> String { deviceLabels[id] ?? id }

    /// The Ask composer shows only when no call is live or starting.
    var showComposer: Bool { liveMeetingId == nil && activeMeetingId == nil }

    /// Calendar events that have not ended yet. `CalendarService` looks back 15 minutes, so
    /// the raw list can include events that already finished.
    var upcoming: [TodayUpcoming] { upcoming(now: clock) }

    func upcoming(now: Date) -> [TodayUpcoming] {
        let upcoming = events.filter { $0.endDate > now }
            .sorted { $0.startDate < $1.startDate }
            .map { TodayUpcoming(id: $0.id, title: $0.title, start: $0.startDate) }
        return TodayLogic.upcomingExcludingRecorded(
            upcoming: upcoming, recent: recentMeetings.map { (id: $0.id, startUnixMs: $0.startUnixMs, title: $0.title) }
        )
    }

    var hero: TodayHero {
        TodayLogic.hero(
            activeMeetingId: activeMeetingId, recentMeetingIds: recentMeetings.map(\.id), recentLoaded: recentLoaded, upcoming: upcoming,
            calendarGranted: calendarGranted, now: clock, calendar: .current
        )
    }

    var liveMeetingId: Int64? {
        if case let .live(id) = hero { return id }
        return nil
    }

    var agenda: TodayAgenda { agenda(now: clock) }

    func agenda(now: Date) -> TodayAgenda {
        TodayLogic.agenda(
            upcoming: upcoming(now: now), recent: recentMeetings.map { (id: $0.id, startUnixMs: $0.startUnixMs) },
            excludingMeetingId: liveMeetingId, now: now, calendar: .current, earlierLimit: 5
        )
    }

    /// The sections a layout would have something to put in, in the standard order.
    func nonEmptySections() -> [TodaySection] {
        let agenda = agenda
        return TodaySection.allCases.filter { section in
            switch section {
            case .greeting, .devices: true
            case .composer: showComposer
            case .upNext: !agenda.upcomingToday.isEmpty
            case .schedule: !agenda.upcomingToday.isEmpty || !agenda.pastToday.isEmpty
            case .earlier: !agenda.earlier.isEmpty
            case .threads: !attentionThreads.isEmpty
            case .followups: !followups.isEmpty
            case .recap: briefRecap != nil || digest != nil || digestError != nil
            }
        }
    }
}

/// What Today can do, as closures. `TodayView` wires them to `AppState` and the window
/// environment; layouts only call them.
struct TodayActions {
    var openMeeting: (Int64) -> Void
    var openThread: (Int64) -> Void
    var ask: (String) -> Void
    var openSettings: () -> Void
    var generateDigest: () -> Void
    var reviewApprovals: () -> Void
    var startMeeting: (TodayUpcoming) -> Void
    var startNote: () -> Void
    var completeFollowup: (Int64) -> Void
}
