import Foundation

// Home as plain values (docs/ui-redesign/screens/home.md). A model says what to show; text the app
// already owns arrives in it, already in the person's language. `WHomeActions` says what each
// control does. Nothing here knows the app: the app builds a model from its own state and calls its
// own functions from the actions. The views are in HomeViews.swift.

// MARK: Element choices

public enum WNextMeetingStyle: String, CaseIterable, Sendable { case card, scoreboard, infobox }
public enum WTimelineStyle: String, CaseIterable, Sendable { case line, ruled }
public enum WLabelStyle: String, CaseIterable, Sendable { case plain, bracketed, smallCaps }
public enum WAskStyle: String, CaseIterable, Sendable { case field, prompt }
public enum WActionStyle: String, CaseIterable, Sendable { case buttons, links }

/// The five choices a skin makes for Home (`today.options`). Mirrors core/skins/src/elements.rs:
/// a missing key takes the old layout's choice, an unknown value takes the default for that part.
public struct WHomeElements: Equatable, Sendable {
    public var nextMeeting: WNextMeetingStyle = .card
    public var timeline: WTimelineStyle = .line
    public var labels: WLabelStyle = .plain
    public var ask: WAskStyle = .field
    public var actions: WActionStyle = .buttons

    public init(
        nextMeeting: WNextMeetingStyle = .card, timeline: WTimelineStyle = .line, labels: WLabelStyle = .plain,
        ask: WAskStyle = .field, actions: WActionStyle = .buttons
    ) {
        self.nextMeeting = nextMeeting
        self.timeline = timeline
        self.labels = labels
        self.ask = ask
        self.actions = actions
    }

    /// Every key and its allowed values, in the order the spec lists them.
    public static let keys: [(key: String, values: [String])] = [
        ("nextMeeting", WNextMeetingStyle.allCases.map(\.rawValue)),
        ("timeline", WTimelineStyle.allCases.map(\.rawValue)),
        ("labels", WLabelStyle.allCases.map(\.rawValue)),
        ("ask", WAskStyle.allCases.map(\.rawValue)),
        ("actions", WActionStyle.allCases.map(\.rawValue)),
    ]

    /// The default and each non-default value once, so a repair covers any skin (core/skins/src/elements.rs `every_variant`).
    public static var everyVariant: [WHomeElements] {
        let d = WHomeElements()
        return [
            d, WHomeElements(nextMeeting: .scoreboard), WHomeElements(nextMeeting: .infobox), WHomeElements(timeline: .ruled),
            WHomeElements(labels: .bracketed), WHomeElements(labels: .smallCaps), WHomeElements(ask: .prompt), WHomeElements(actions: .links),
        ]
    }

    /// What an old skin with no element keys draws, by its `archetype` raw value
    /// (standard, field, terminal, almanac, reference, glass). An unknown archetype draws the Standard row.
    public static func fromArchetype(_ archetype: String) -> WHomeElements {
        switch archetype {
        case "field": return WHomeElements(nextMeeting: .scoreboard)
        case "terminal": return WHomeElements(labels: .bracketed, ask: .prompt)
        case "almanac": return WHomeElements(timeline: .ruled, labels: .smallCaps)
        case "reference": return WHomeElements(nextMeeting: .infobox, timeline: .ruled, actions: .links)
        default: return WHomeElements()
        }
    }

    public static func resolve(options: [String: String], archetype: String) -> WHomeElements {
        let base = fromArchetype(archetype)
        func pick<T: RawRepresentable>(_ key: String, _ fallback: T, _ standard: T) -> T where T.RawValue == String {
            guard let raw = options[key] else { return fallback }
            return T(rawValue: raw) ?? standard
        }
        let d = WHomeElements()
        return WHomeElements(
            nextMeeting: pick("nextMeeting", base.nextMeeting, d.nextMeeting),
            timeline: pick("timeline", base.timeline, d.timeline),
            labels: pick("labels", base.labels, d.labels),
            ask: pick("ask", base.ask, d.ask),
            actions: pick("actions", base.actions, d.actions)
        )
    }
}

// MARK: Filters

/// A date choice on the Dates chip.
public enum WHomeDateRange: Equatable, Sendable {
    case today, yesterday, lastSevenDays
    /// First and last day, both included.
    case custom(from: Date, to: Date)

    /// The range as [start, end): midnight to midnight on the calendar's days.
    public func bounds(now: Date, calendar: Calendar = .current) -> (start: Date, end: Date) {
        let today = calendar.startOfDay(for: now)
        func day(_ offset: Int, from date: Date) -> Date { calendar.date(byAdding: .day, value: offset, to: date) ?? date }
        switch self {
        case .today: return (today, day(1, from: today))
        case .yesterday: return (day(-1, from: today), today)
        case .lastSevenDays: return (day(-6, from: today), day(1, from: today))
        case .custom(let from, let to):
            let a = calendar.startOfDay(for: min(from, to)), b = calendar.startOfDay(for: max(from, to))
            return (a, day(1, from: b))
        }
    }
}

public struct WHomeAppChoice: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }
}

/// What the chips carry into a search.
public struct WHomeFilter: Equatable, Sendable {
    public var appId: String?
    public var dates: WHomeDateRange?
    /// A person from the attendees of recent meetings.
    public var person: String?
    public init(appId: String? = nil, dates: WHomeDateRange? = nil, person: String? = nil) {
        self.appId = appId
        self.dates = dates
        self.person = person
    }
    public var isEmpty: Bool { appId == nil && dates == nil && person == nil }
}

// MARK: Pieces of the day

public struct WHomeFact: Equatable, Sendable, Identifiable {
    public var id: String { label }
    public let label: String
    public let value: String
    public init(label: String, value: String) {
        self.label = label
        self.value = value
    }
}

public struct WHomeMeeting: Identifiable, Equatable, Sendable {
    public enum Kind: Equatable, Sendable { case past, running, upcoming }
    public let id: String
    public var kind: Kind
    /// "10:31 AM", the start.
    public var time: String
    /// "10:31 AM to 11:15 AM" for the hero and the spoken name; falls back to `time`.
    public var timeRange: String
    public var title: String
    public var attendees: [String]
    /// "Next, in 9 minutes", "In progress".
    public var eyebrow: String?
    /// One line about a past meeting.
    public var gist: String?
    public var startTitle: String?
    public var openTitle: String?
    public var briefTitle: String?
    /// The infobox rows, built by the app in its language.
    public var facts: [WHomeFact]

    public init(
        id: String, kind: Kind, time: String, timeRange: String? = nil, title: String, attendees: [String] = [],
        eyebrow: String? = nil, gist: String? = nil, startTitle: String? = nil, openTitle: String? = nil,
        briefTitle: String? = nil, facts: [WHomeFact] = []
    ) {
        self.id = id
        self.kind = kind
        self.time = time
        self.timeRange = timeRange ?? time
        self.title = title
        self.attendees = attendees
        self.eyebrow = eyebrow
        self.gist = gist
        self.startTitle = startTitle
        self.openTitle = openTitle
        self.briefTitle = briefTitle
        self.facts = facts
    }
}

/// One stretch of work memory between meetings.
public struct WHomeBlock: Identifiable, Equatable, Sendable {
    public let id: String
    /// "9:30 to 9:52 AM".
    public var timeRange: String
    /// "22 min", on the left of the time line.
    public var duration: String
    public var items: [WMemoryItem]
    /// "and 2 more apps", when the block was cut.
    public var more: String?
    public var openTitle: String

    public init(id: String, timeRange: String, duration: String, items: [WMemoryItem], more: String? = nil, openTitle: String) {
        self.id = id
        self.timeRange = timeRange
        self.duration = duration
        self.items = items
        self.more = more
        self.openTitle = openTitle
    }
}

public enum WHomeStop: Identifiable, Equatable, Sendable {
    case meeting(WHomeMeeting)
    case memory(WHomeBlock)
    /// The now marker: its time and the capture state beside it.
    case now(time: String, label: String)
    /// One line where blocks would be, so an empty stretch is never read as nothing happening.
    case notice(String)
    /// "Yesterday" or "Last week": collapsed to chips, expands to past meeting rows.
    case history(WHomeHistory)

    public var id: String {
        switch self {
        case .meeting(let m): return "m" + m.id
        case .memory(let b): return "b" + b.id
        case .now: return "now"
        case .notice: return "notice"
        case .history(let h): return "h" + h.id
        }
    }
}

/// A day (or the week) before today, shown as one collapsed row.
public struct WHomeHistory: Identifiable, Equatable, Sendable {
    /// "yesterday" or "lastWeek"; also the scroll target.
    public let id: String
    public var title: String
    /// "Mon, Nov 16".
    public var dateText: String
    public var meetings: [WHomeMeeting]
    public init(id: String, title: String, dateText: String, meetings: [WHomeMeeting]) {
        self.id = id
        self.title = title
        self.dateText = dateText
        self.meetings = meetings
    }
}

/// An item of the sidebar's Thread list: a part of Home to scroll to.
public struct WHomeThreadEntry: Identifiable, Equatable, Sendable {
    public let id: String
    public var title: String
    /// A time or a date, on the right.
    public var detail: String
    public init(id: String, title: String, detail: String) {
        self.id = id
        self.title = title
        self.detail = detail
    }
}

/// A request to scroll Home to a part of the line. The counter makes a repeat of the same target a new request.
public struct WHomeScroll: Equatable, Sendable {
    public var id: String
    public var count: Int
    public init(id: String, count: Int) {
        self.id = id
        self.count = count
    }
}

public struct WHomeRow: Identifiable, Equatable, Sendable {
    public let id: String
    public var time: String?
    public var title: String
    public var detail: String?
    /// The row's one action ("Review", "Done", "Open", "Try again"); nil when the row has none.
    public var actionTitle: String?
    /// The spoken name of the action, when the title alone is not enough ("Done, Send the deck").
    public var actionName: String?
    /// A heading drawn before this row (a day in this week's list).
    public var heading: String?

    public init(id: String, time: String? = nil, title: String, detail: String? = nil, actionTitle: String? = nil, actionName: String? = nil, heading: String? = nil) {
        self.heading = heading
        self.id = id
        self.time = time
        self.title = title
        self.detail = detail
        self.actionTitle = actionTitle
        self.actionName = actionName
    }
}

public struct WHomeSection: Identifiable, Equatable, Sendable {
    public enum Kind: String, Equatable, Sendable { case approvals, followups, threads, recap, week }
    public var id: String { kind.rawValue }
    public var kind: Kind
    public var title: String
    public var rows: [WHomeRow]

    public init(kind: Kind, title: String, rows: [WHomeRow]) {
        self.kind = kind
        self.title = title
        self.rows = rows
    }
}

public enum WHomeHero: Equatable, Sendable {
    /// A call is live: the indicator, its title, Open. The app's live pane goes in the live slot.
    case live(title: String, elapsedSeconds: Int?)
    case next(WHomeMeeting)
    /// No calendar: the page shows the connect action.
    case noCalendar(title: String, help: String, actionTitle: String)
    case noMeetings(title: String, help: String, actionTitle: String)
    case none
}

/// The header search: a field, or (no usable AI provider) a button into setup.
public enum WHomeAsk: Equatable, Sendable {
    case available(placeholder: String)
    /// No usable AI provider: a button into setup, not a dead field.
    case needsSetup(title: String)
}

public enum WHomeLoad: Equatable, Sendable {
    case ready
    /// The quiet bar under the greeting. Cached content stays.
    case loading
    /// Names what failed.
    case failed(title: String, help: String)
}

public struct WHomeModel: Equatable, Sendable {
    public var load: WHomeLoad = .ready
    public var hero: WHomeHero = .none
    /// Newest at the top: later meetings, then the now marker, then memory blocks and past meetings newest first, then
    /// "Yesterday" and "Last week". The hero (the next meeting, or the live call or empty state) is drawn in the line just
    /// above the now marker, and its meeting is not in this list.
    public var timeline: [WHomeStop] = []
    /// "Nothing on today yet." when the day has no meetings and no blocks.
    public var emptyTitle: String?
    public var emptyHelp: String?
    public var sections: [WHomeSection] = []
    public var elements = WHomeElements()

    public init() {}

    /// The sidebar's Thread list: Today first, then each of Yesterday and Last week that has meetings.
    public func thread(todayTitle: String, nowTime: String) -> [WHomeThreadEntry] {
        [WHomeThreadEntry(id: "today", title: todayTitle, detail: nowTime)] + timeline.compactMap {
            if case .history(let h) = $0 { return WHomeThreadEntry(id: h.id, title: h.title, detail: h.dateText) } else { return nil }
        }
    }

    /// The meetings of the time line, in order, never including the hero's meeting.
    public var timelineMeetings: [WHomeMeeting] {
        timeline.compactMap { if case .meeting(let m) = $0 { return m } else { return nil } }
    }

    /// True when the day has nothing to draw in the time line or below it.
    public var isEmptyDay: Bool {
        let drawsStop = timeline.contains { stop in
            switch stop {
            case .meeting, .memory, .history: return true
            default: return false
            }
        }
        if case .next = hero { return false }
        if case .live = hero { return false }
        return !drawsStop && sections.isEmpty
    }
}

/// What each control does. The app wires these to its state.
public struct WHomeActions {
    public var startMeeting: (String) -> Void = { _ in }
    public var openMeeting: (String) -> Void = { _ in }
    public var openBrief: (String) -> Void = { _ in }
    public var openLive: () -> Void = {}
    public var startNote: () -> Void = {}
    public var connectCalendar: () -> Void = {}
    /// A block's Open: Library for its time range.
    public var openBlock: (String) -> Void = { _ in }
    /// A row's action in a section below the line.
    public var rowAction: (WHomeSection.Kind, String) -> Void = { _, _ in }
    public var retry: () -> Void = {}

    public init() {}
}
