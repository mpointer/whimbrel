import AppKit
import SwiftUI
import WhimbrelUI

/// What the redesign Home reads that the old Today does not: today's memory blocks. Loaded by `TodayView` only
/// when the redesign flag is on.
struct HomeRedesignData: Equatable {
    var blocks: [CoreDayBlock] = []

    static func == (a: HomeRedesignData, b: HomeRedesignData) -> Bool {
        a.blocks.map(\.startUnixMs) == b.blocks.map(\.startUnixMs) && a.blocks.map(\.endUnixMs) == b.blocks.map(\.endUnixMs)
    }
}

/// The shell talks to the Home view through this: Home says which parts of the line exist (the sidebar's Thread list), and
/// the sidebar says which one to scroll to.
@MainActor
final class HomeRedesignBridge: ObservableObject {
    @Published private(set) var thread: [WHomeThreadEntry] = []
    @Published private(set) var scroll: WHomeScroll?
    /// Names from the attendees of recent meetings, for the People chip.
    @Published private(set) var people: [String] = []

    func setPeople(_ names: [String]) { if names != people { people = names } }

    func setThread(_ entries: [WHomeThreadEntry]) { if entries != thread { thread = entries } }
    func scrollTo(_ id: String) { scroll = WHomeScroll(id: id, count: (scroll?.count ?? 0) + 1) }
}

private struct HomeBridgeKey: EnvironmentKey { static let defaultValue: HomeRedesignBridge? = nil }
private struct HomeScrollKey: EnvironmentKey { static let defaultValue: WHomeScroll? = nil }

extension EnvironmentValues {
    var homeBridge: HomeRedesignBridge? {
        get { self[HomeBridgeKey.self] }
        set { self[HomeBridgeKey.self] = newValue }
    }
    var homeScroll: WHomeScroll? {
        get { self[HomeScrollKey.self] }
        set { self[HomeScrollKey.self] = newValue }
    }
}

/// `TodayContent` (and the new reads) to the `WHomeModel` the WhimbrelUI Home draws. Pure apart from app
/// names, which come from the system. Home draws only what `TodayContent` already holds; nothing is fetched here.
enum HomeRedesignAdapter {
    // MARK: Ids

    static func eventId(_ id: String) -> String { "e:" + id }
    static func heldId(_ id: Int64) -> String { "h:" + String(id) }

    static func parseHeld(_ id: String) -> Int64? { id.hasPrefix("h:") ? Int64(id.dropFirst(2)) : nil }
    static func parseEvent(_ id: String) -> String? { id.hasPrefix("e:") ? String(id.dropFirst(2)) : nil }

    // MARK: Apps

    private static var names: [String: String] = [:]

    /// The name a person knows an app by: the system's, else the last part of the bundle id.
    static func appName(_ bundleId: String) -> String {
        if let cached = names[bundleId] { return cached }
        var name = bundleId.split(separator: ".").last.map(String.init) ?? bundleId
        if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId) {
            name = FileManager.default.displayName(atPath: url.path)
            if name.hasSuffix(".app") { name = String(name.dropLast(4)) }
        }
        names[bundleId] = name
        return name
    }

    static func glyph(_ bundleId: String) -> String {
        let id = bundleId.lowercased()
        let table: [(String, String)] = [
            ("mail", "envelope"), ("outlook", "envelope"), ("slack", "number"), ("excel", "tablecells"), ("numbers", "tablecells"),
            ("safari", "globe"), ("chrome", "globe"), ("firefox", "globe"), ("arc", "globe"), ("edge", "globe"),
            ("xcode", "chevron.left.forwardslash.chevron.right"), ("vscode", "chevron.left.forwardslash.chevron.right"),
            ("terminal", "terminal"), ("word", "doc.text"), ("pages", "doc.text"), ("notes", "note.text"), ("textedit", "doc.text"),
            ("zoom", "video"), ("teams", "video"), ("calendar", "calendar"), ("finder", "folder"), ("preview", "doc.richtext"),
        ]
        return table.first { id.contains($0.0) }?.1 ?? "app"
    }

    /// A steady dot color for an app, so one app always looks the same.
    static func kind(_ bundleId: String) -> WAppKind {
        let sum = bundleId.unicodeScalars.reduce(0) { ($0 &* 31 &+ Int($1.value)) & 0xFFFF }
        return WAppKind.allCases[sum % WAppKind.allCases.count]
    }

    // MARK: Blocks

    static func block(_ b: CoreDayBlock) -> WHomeBlock {
        var items: [WMemoryItem] = []
        for app in b.apps {
            let name = appName(app.appBundleId)
            if app.items.isEmpty {
                items.append(WMemoryItem(app: kind(app.appBundleId), glyph: glyph(app.appBundleId), name: name))
            }
            for title in app.items {
                items.append(WMemoryItem(
                    app: kind(app.appBundleId), glyph: glyph(app.appBundleId),
                    name: String(localized: "\(title) in \(name)", comment: "One thing Whimbrel saw on screen: a window title, then the app. For example: Pricing v3.xlsx in Excel.")))
            }
        }
        let minutes = max(1, Int(((Double(b.endUnixMs - b.startUnixMs)) / 60000).rounded()))
        let start = Date(timeIntervalSince1970: Double(b.startUnixMs) / 1000), end = Date(timeIntervalSince1970: Double(b.endUnixMs) / 1000)
        return WHomeBlock(
            id: String(b.startUnixMs), timeRange: MeetingCardModel.timeRange(start: start, end: end),
            duration: String(localized: "\(minutes) min"), items: items,
            more: b.moreApps > 0 ? String(localized: "More apps: \(b.moreApps)", comment: "Under a memory block that lists only some of the apps seen. The number is how many more.") : nil,
            openTitle: String(localized: "Open"))
    }

    /// Today's meeting intervals in ms, merged where they overlap, for cutting memory blocks around meetings.
    static func meetingIntervals(content: TodayContent, startOfDay: Date, endOfDay: Date) -> [(start: Int64, end: Int64)] {
        func ms(_ d: Date) -> Int64 { Int64(d.timeIntervalSince1970 * 1000) }
        var spans: [(Int64, Int64)] = content.events.filter { $0.startDate < endOfDay && $0.endDate > startOfDay }.map { (ms($0.startDate), ms($0.endDate)) }
        for m in content.recentMeetings where m.startUnixMs >= ms(startOfDay) && m.startUnixMs < ms(endOfDay) {
            spans.append((m.startUnixMs, max(m.startUnixMs, m.endUnixMs ?? m.startUnixMs)))
        }
        var merged: [(start: Int64, end: Int64)] = []
        for s in spans.sorted(by: { $0.0 < $1.0 }) where s.1 > s.0 {
            if let last = merged.last, s.0 <= last.end { merged[merged.count - 1].end = max(last.end, s.1) } else { merged.append((s.0, s.1)) }
        }
        return merged
    }

    // MARK: Capture state

    static func memoryState(enabled: Bool, resumeAt: Date?, now: Date) -> WMemoryState {
        switch CapturePauseModel.state(captureOn: enabled, resumeAt: resumeAt) {
        case .on: return .on
        case .paused: return .pausedUntil(CapturePauseModel.untilText(resumeAt ?? now, now: now))
        case .off: return .off
        }
    }

    static func nowLabel(_ memory: WMemoryState) -> String {
        switch memory {
        case .on: return String(localized: "Memory on", comment: "Work memory state in the sidebar footer: Whimbrel is saving context.")
        case .off: return String(localized: "Memory off", comment: "Work memory state in the sidebar footer: Whimbrel is not saving context.")
        case .pausedUntil(let time): return String(localized: "Paused until \(time)", comment: "Work memory state in the sidebar footer. The placeholder is a clock time, for example 10:52 AM.")
        }
    }

    /// The one line where blocks would be when memory is not saving, so an empty stretch is not read as nothing happening.
    static func notice(_ memory: WMemoryState) -> String? {
        switch memory {
        case .on: return nil
        case .off: return String(localized: "Memory is off.", comment: "One line on the time line of Today when Whimbrel is not saving context.")
        case .pausedUntil(let time): return String(localized: "Memory is paused until \(time).", comment: "One line on the time line of Today during a timed pause. The placeholder is a clock time, for example 3:10 PM.")
        }
    }

    // MARK: Model

    /// The header search: a field when Ask can run, else a button into setup (the same rule as `TodayComposer`).
    @MainActor
    static func ask(_ appState: AppState) -> WHomeAsk {
        let routing = appState.routing
        let provider = routing.defaultSelection().provider
        let hasKey = routing.hasKey(for: provider) && (provider != .openrouter || !routing.openrouterDefaultModel.isEmpty)
        if TodayComposer.askAvailable(provider: provider, localOnly: routing.localOnlyMode, hasKey: hasKey) {
            return .available(placeholder: SkinStore.shared.selected.voice.text(.askPlaceholder, name: ""))
        }
        return .needsSetup(title: String(localized: "Set up an AI provider"))
    }

    /// People from the attendees of the meetings Home already loaded (recent and today's calendar), most often met first.
    static func people(content: TodayContent, limit: Int = 30) -> [String] {
        var counts: [String: Int] = [:]
        for name in content.recentMeetings.flatMap(\.attendees) + content.events.flatMap(\.attendees) + content.upcomingWeek.flatMap(\.attendees) {
            let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty, !trimmed.contains("@") { counts[trimmed, default: 0] += 1 }
        }
        return counts.sorted { ($1.value, $0.key) < ($0.value, $1.key) }.prefix(limit).map(\.key)
    }

    /// The date under "Today" in the header row.
    static func dateLine(_ now: Date) -> String { now.formatted(.dateTime.weekday(.wide).month(.wide).day()) }

    /// One row of "Yesterday" or "Last week": the meetings of that stretch, newest first; nil when it has none.
    static func history(
        id: String, title: String, from: Date, to: Date, content: TodayContent
    ) -> WHomeHistory? {
        func ms(_ d: Date) -> Int64 { Int64(d.timeIntervalSince1970 * 1000) }
        let meetings = content.recentMeetings.filter { $0.startUnixMs >= ms(from) && $0.startUnixMs < ms(to) }.sorted { $0.startUnixMs > $1.startUnixMs }
        guard let newest = meetings.first else { return nil }
        let day = Date(timeIntervalSince1970: Double(newest.startUnixMs) / 1000)
        return WHomeHistory(
            id: id, title: title, dateText: day.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day()),
            meetings: meetings.map {
                WHomeMeeting(
                    id: heldId($0.id), kind: .past, time: MeetingRowText.time($0.startUnixMs), title: $0.title, attendees: $0.attendees,
                    gist: $0.summaryTldr?.split(separator: "\n").first.map(String.init), openTitle: String(localized: "Open"))
            })
    }

    static func model(
        content: TodayContent, data: HomeRedesignData, memory: WMemoryState, elapsedSeconds: Int?,
        now: Date, voice: SkinVoice, elements: WHomeElements
    ) -> WHomeModel {
        var m = WHomeModel()
        m.elements = elements
        m.load = content.recentLoaded ? .ready : .loading
        m.emptyTitle = String(localized: "Nothing on today yet.", comment: "Title of Today when there are no meetings and nothing saved yet.")

        // Hero: the next meeting (drawn in the line at its time), or the live call, or an empty state.
        var heroEventId: String?
        switch content.hero {
        case .live(let id):
            m.hero = .live(title: content.recentMeetings.first { $0.id == id }?.title ?? "", elapsedSeconds: elapsedSeconds)
        case .upcoming:
            if let row = TodayRows.upNext(content) {
                heroEventId = row.event.id
                let source = content.events.first { $0.id == row.event.id }
                var facts = [WHomeFact(label: String(localized: "Time", comment: "Row label in the next meeting box on Today."), value: source.map { MeetingCardModel.timeRange(start: $0.startDate, end: $0.endDate) } ?? row.timeText)]
                if let names = source?.attendees, !names.isEmpty {
                    facts.append(WHomeFact(label: String(localized: "With", comment: "Row label in the next meeting box on Today. The people in the meeting follow."), value: names.joined(separator: ", ")))
                }
                var hasBrief = false
                if case .ready = row.prep { hasBrief = true }
                m.hero = .next(WHomeMeeting(
                    id: eventId(row.event.id), kind: .upcoming, time: row.timeText,
                    timeRange: source.map { MeetingCardModel.timeRange(start: $0.startDate, end: $0.endDate) },
                    title: row.title, attendees: source?.attendees ?? [], eyebrow: row.relative,
                    startTitle: row.showsStart ? String(localized: "Start") : nil,
                    briefTitle: hasBrief ? String(localized: "Open brief") : nil, facts: facts))
            }
        case .noCalendar:
            m.hero = .noCalendar(
                title: String(localized: "Nothing on today yet."), help: String(localized: "Connect a calendar to see upcoming meetings here."),
                actionTitle: String(localized: "Connect Calendar"))
        case .noMeetings:
            m.hero = .noMeetings(
                title: String(localized: "Nothing on today yet."), help: String(localized: "Start a manual note to begin."),
                actionTitle: String(localized: "Start note"))
        case .none:
            m.hero = .none
        }

        // The line, newest at the top. Later meetings above the hero, furthest ahead first; memory blocks and past meetings
        // below the now marker, newest first.
        struct Item { let start: Date; let stop: WHomeStop }
        var later: [Item] = [], earlier: [Item] = []
        for entry in TodayRows.timeline(content, now: now) {
            switch entry.kind {
            case .event(let event):
                if event.id == heroEventId { continue }
                let source = content.events.first { $0.id == event.id }
                later.append(Item(start: entry.start, stop: .meeting(WHomeMeeting(
                    id: eventId(event.id), kind: entry.model.isRunning ? .running : .upcoming, time: entry.model.startText,
                    timeRange: entry.model.timeRange, title: entry.model.title, attendees: source?.attendees ?? [],
                    eyebrow: entry.model.hint ?? (entry.model.isRunning ? String(localized: "In progress") : nil),
                    startTitle: content.activeMeetingId == nil ? String(localized: "Start") : nil))))
            case .held(let id):
                let meeting = content.recentMeetings.first { $0.id == id }
                let gist = content.processingMeetingId == id ? String(localized: "Transcribing…") : meeting?.summaryTldr?.split(separator: "\n").first.map(String.init)
                earlier.append(Item(start: entry.start, stop: .meeting(WHomeMeeting(
                    id: heldId(id), kind: .past, time: entry.model.startText, timeRange: entry.model.timeRange, title: entry.model.title,
                    attendees: meeting?.attendees ?? [], gist: gist, openTitle: String(localized: "Open")))))
            }
        }
        for b in data.blocks {
            earlier.append(Item(start: Date(timeIntervalSince1970: Double(b.startUnixMs) / 1000), stop: .memory(block(b))))
        }
        later.sort { $0.start > $1.start }
        earlier.sort { $0.start > $1.start }
        var stops = later.map(\.stop)
        stops.append(.now(time: MeetingRowText.time(now), label: nowLabel(memory)))
        if let notice = notice(memory) { stops.append(.notice(notice)) }
        stops += earlier.map(\.stop)
        let calendar = Calendar.current
        let today = calendar.startOfDay(for: now)
        let yesterday = calendar.date(byAdding: .day, value: -1, to: today) ?? today
        let weekAgo = calendar.date(byAdding: .day, value: -7, to: today) ?? yesterday
        if let h = history(id: "yesterday", title: String(localized: "Yesterday"), from: yesterday, to: today, content: content) { stops.append(.history(h)) }
        if let h = history(id: "lastWeek", title: String(localized: "Earlier"), from: weekAgo, to: yesterday, content: content) { stops.append(.history(h)) }
        m.timeline = stops

        // Below the line
        if content.pendingApprovalCount > 0 {
            m.sections.append(WHomeSection(kind: .approvals, title: String(localized: "Needs your decision"), rows: [
                WHomeRow(id: "approvals", title: String(localized: "\(content.pendingApprovalCount) pending approvals"), actionTitle: String(localized: "Review"))]))
        }
        if !content.followups.isEmpty {
            m.sections.append(WHomeSection(kind: .followups, title: String(localized: "Follow-ups"), rows: content.followups.map {
                WHomeRow(id: String($0.id), title: $0.text, actionTitle: String(localized: "Done"),
                         actionName: String(localized: "Done, \($0.text)", comment: "Spoken name of the Done button on a follow-up. The placeholder is the follow-up."))
            }))
        }
        if !content.attentionThreads.isEmpty {
            m.sections.append(WHomeSection(kind: .threads, title: String(localized: "Open loops"), rows: content.attentionThreads.map {
                WHomeRow(id: String($0.id), title: $0.subject ?? $0.lastMessageFrom, actionTitle: String(localized: "Open"))
            }))
        }
        switch TodayRows.recap(content) {
        case .brief(let text, _), .digest(let text):
            m.sections.append(WHomeSection(kind: .recap, title: voice.text(.recapHeader, name: ""), rows: [WHomeRow(id: "recap", title: text)]))
        case .failed(let message, let retry):
            m.sections.append(WHomeSection(kind: .recap, title: voice.text(.recapHeader, name: ""), rows: [
                WHomeRow(id: "recap", title: message, actionTitle: retry ? String(localized: "Try again") : nil)]))
        case .none: break
        }
        var week: [WHomeRow] = []
        for day in TodayRows.upcomingDays(content, expanded: true) {
            for (i, row) in day.rows.enumerated() {
                week.append(WHomeRow(id: row.id, time: row.timeText, title: row.title, detail: row.invited.isEmpty ? nil : row.invited, heading: i == 0 ? day.heading : nil))
            }
        }
        if !week.isEmpty { m.sections.append(WHomeSection(kind: .week, title: String(localized: "Upcoming"), rows: week)) }
        return m
    }

    // MARK: Filter

    static func libraryFilter(_ f: WHomeFilter, apps: [WHomeAppChoice], strings: WHomeStrings, now: Date = Date()) -> LibraryCaptureFilter {
        var out = LibraryCaptureFilter(appId: f.appId, person: f.person)
        out.appName = apps.first { $0.id == f.appId }?.name ?? f.appId.map(appName)
        if let dates = f.dates {
            let b = dates.bounds(now: now)
            out.startUnixMs = Int64(b.start.timeIntervalSince1970 * 1000)
            out.endUnixMs = Int64(b.end.timeIntervalSince1970 * 1000)
            switch dates {
            case .today: out.dateLabel = strings.today
            case .yesterday: out.dateLabel = strings.yesterday
            case .lastSevenDays: out.dateLabel = strings.lastSevenDays
            case .custom(let from, let to):
                let style = Date.FormatStyle().month(.abbreviated).day()
                out.dateLabel = "\(from.formatted(style)) - \(to.formatted(style))"
            }
        }
        return out
    }
}

/// Home in the redesign: `TodayView` still owns all loading; this draws what it loaded. The header row (title, search,
/// capture state) and the filter chips are the shell's.
struct TodayHomeRedesignView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.homeBridge) private var bridge
    @Environment(\.homeScroll) private var scroll
    let content: TodayContent
    let actions: TodayActions
    let live: AnyView?
    let data: HomeRedesignData
    /// A block's Open (the Library range).
    let openBlock: (CoreDayBlock) -> Void

    var body: some View {
        // The clock ticks so the now marker moves and a finished meeting goes behind it.
        SwiftUI.TimelineView(.periodic(from: .now, by: 30)) { context in
            let skin = SkinStore.shared.selected
            let model = HomeRedesignAdapter.model(
                content: content, data: data,
                memory: HomeRedesignAdapter.memoryState(enabled: appState.activityCaptureEnabled, resumeAt: appState.captureResumeAt, now: context.date),
                elapsedSeconds: appState.activeMeetingStartedAt.map { max(0, Int(context.date.timeIntervalSince($0))) },
                now: context.date, voice: skin.voice, elements: WHomeElements.resolve(options: skin.archetypeOptions, archetype: skin.archetype.rawValue))
            let thread = model.thread(todayTitle: String(localized: "Today"), nowTime: MeetingRowText.time(context.date))
            // While a meeting records, the live meeting screen takes Home's place (redesign flag only).
            Group {
                if let live { live } else { WHomeView(model: model, actions: wiring(), scroll: scroll) { EmptyView() } }
            }
                .redesignThemed()
                .onAppear {
                    bridge?.setThread(thread)
                    bridge?.setPeople(HomeRedesignAdapter.people(content: content))
                }
                .onChange(of: thread) { _, new in bridge?.setThread(new) }
        }
    }

    private func wiring() -> WHomeActions {
        var a = WHomeActions()
        a.startMeeting = { id in
            if let eventId = HomeRedesignAdapter.parseEvent(id), let event = content.upcoming.first(where: { $0.id == eventId }) { actions.startMeeting(event) }
        }
        a.openMeeting = { id in if let held = HomeRedesignAdapter.parseHeld(id) { actions.openMeeting(held) } }
        a.openBrief = { _ in
            if let row = TodayRows.upNext(content), case .ready(let prep) = row.prep { actions.showPrepBrief(prep.eventId) }
        }
        a.openLive = { if let id = content.liveMeetingId { actions.openMeeting(id) } }
        a.startNote = actions.startNote
        a.connectCalendar = actions.requestCalendar
        a.openBlock = { id in if let block = data.blocks.first(where: { String($0.startUnixMs) == id }) { openBlock(block) } }
        a.rowAction = { kind, id in
            switch kind {
            case .approvals: actions.reviewApprovals()
            case .followups: if let n = Int64(id) { actions.completeFollowup(n) }
            case .threads: if let n = Int64(id) { actions.openThread(n) }
            case .recap: actions.generateDigest()
            case .week: break
            }
        }
        a.retry = {}
        return a
    }
}
