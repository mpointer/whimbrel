import AppKit
import Combine
import SwiftUI
import WhimbrelUI

/// What the shell and Library's Find page share: the header field's text and the filter chips, which the shell draws and
/// the page reads, and the requests that move between them (Enter, Command Return, Command K, Search Library).
@MainActor
final class FindBridge: ObservableObject {
    /// The header field's text and the chips, as the shell last reported them. Plain values, not published: a keystroke redraws nobody.
    private(set) var query = ""
    private(set) var filter = WHomeFilter()
    let queryChanges = PassthroughSubject<String, Never>()
    let filterChanges = PassthroughSubject<WHomeFilter, Never>()
    /// The page setting the field and the chips (a starter, an opened conversation, Home's block Open).
    @Published private(set) var push = WShellPush(token: 0, query: "", filter: WHomeFilter())
    /// A change moves the keyboard to the header field.
    @Published var focusRequest = 0
    /// Enter in the header: search with `query` and `filter`.
    @Published var searchRequest = 0
    /// Command Return in the header: ask with `query` and `filter`.
    @Published var askRequest = 0
    /// "Results: 3", at the right of the chips.
    @Published var countText: String?
    /// The chips show on Find only; Recent, Episodes and Digest have nothing to filter.
    @Published var showsFilters = true
    /// An answer is being written or saved: the shell's Ask button on the other sections shows off, and Command Return there waits.
    @Published var askBusy = false

    /// Find's controller lives here, with the shell, and not with the Library screen: choosing another section destroys the screen, and an
    /// answer still arriving would be lost with it (and a prepared ask left behind). Made when Library first shows, so a session that never
    /// opens Find creates nothing.
    lazy var controller = FindController()

    func shellQueryChanged(_ text: String) { query = text; queryChanges.send(text) }
    func shellFilterChanged(_ new: WHomeFilter) { filter = new; filterChanges.send(new) }

    func setQuery(_ text: String) {
        query = text
        push = WShellPush(token: push.token + 1, query: text, filter: filter)
    }

    func setFilter(_ new: WHomeFilter) {
        filter = new
        push = WShellPush(token: push.token + 1, query: query, filter: new)
    }
}

private struct FindBridgeKey: EnvironmentKey { static let defaultValue: FindBridge? = nil }

extension EnvironmentValues {
    var findBridge: FindBridge? {
        get { self[FindBridgeKey.self] }
        set { self[FindBridgeKey.self] = newValue }
    }
}

/// What a card needs from the app to be drawn: names, icons, the device and the time. Injected so the adapter is pure.
struct FindCardEnv {
    var appName: (String) -> String
    var glyph: (String) -> String
    var dot: (String) -> WAppKind
    var device: (String) -> String
    var strings: WFindStrings
    var locale: Locale = .current
    var calendar: Calendar = .current

    @MainActor
    static func live(appState: AppState, strings: WFindStrings) -> FindCardEnv {
        FindCardEnv(
            appName: { HomeRedesignAdapter.appName($0) }, glyph: { HomeRedesignAdapter.glyph($0) }, dot: { HomeRedesignAdapter.kind($0) },
            device: { appState.deviceLabel(for: $0) }, strings: strings)
    }
}

/// The core's cards, filters, routes and saved answers as the plain values the Find page draws. Pure: no store, no `AppState`.
enum FindAdapter {
    // MARK: Cards

    /// "Thu, Nov 12, 4:10 PM".
    static func timeText(_ ms: Int64, env: FindCardEnv) -> String {
        let date = Date(timeIntervalSince1970: Double(ms) / 1000)
        var style = Date.FormatStyle(date: .omitted, time: .omitted, locale: env.locale, calendar: env.calendar, timeZone: env.calendar.timeZone)
        style = style.weekday(.abbreviated).month(.abbreviated).day().hour().minute()
        return date.formatted(style)
    }

    static func kind(_ coreKind: String) -> WSourceKind {
        switch coreKind {
        case "transcript": return .transcriptMoment
        case "snapshot": return .slide
        case "thread": return .email
        case "episode": return .episode
        case "routine_run": return .routineRun
        case "meeting": return .meeting
        default: return .seenOnScreen
        }
    }

    /// What Open does, per the spec's table: a capture opens the saved capture, a transcript line the meeting at that line, a slide or a
    /// meeting the meeting, an episode the episode, an email thread its summary, and an automation run nothing.
    static func open(_ c: CoreSourceCard) -> WFindOpen? {
        guard c.found else { return nil }
        switch c.kind {
        case "capture": return .capture(c.id)
        case "transcript": return c.meetingId.map { .meetingAt(id: $0, startCs: c.startCs ?? 0) }
        case "snapshot": return c.meetingId.map { .meeting($0) }
        case "meeting": return .meeting(c.id)
        case "episode": return .episode(c.id)
        case "thread": return .thread(c.id)
        default: return nil
        }
    }

    static func card(_ c: CoreSourceCard, env: FindCardEnv) -> WFindCard {
        let s = env.strings
        let id = c.kind + "-" + String(c.id)
        guard c.found else { return WFindCard(id: id, kind: kind(c.kind), title: "", isGone: true) }
        var card = WFindCard(id: id, kind: kind(c.kind), title: c.title.isEmpty ? (c.meetingTitle ?? c.appBundleId.map(env.appName) ?? "") : c.title)
        card.timeMs = c.tsUnixMs
        card.time = c.tsUnixMs.map { timeText($0, env: env) } ?? ""
        card.device = c.deviceId.map(env.device)
        card.text = c.text
        card.open = open(c)
        switch c.kind {
        case "capture", "episode":
            if let bundle = c.appBundleId {
                card.appName = env.appName(bundle)
                card.appGlyph = env.glyph(bundle)
                card.appDot = env.dot(bundle)
            }
            card.openHint = c.kind == "capture" ? s.opensSavedCapture : s.opensEpisode
        case "transcript":
            card.appName = s.meetingAudio
            card.title = c.meetingTitle ?? c.title
            if let cs = c.startCs { card.lineTime = WLaneMath.clock(cs: cs) }
            card.speaker = c.speakerLabel.map { c.speakerIsGuess == true ? $0 + "?" : $0 }
            if let cs = c.startCs { card.openHint = s.opensNoteAt(WLaneMath.clock(cs: cs)) }
            switch c.via {
            case "speaker": card.via = c.speakerLabel.map(s.spokenBy)
            case "attendee": card.via = s.inMeetingTheyAttended
            default: break
            }
        case "snapshot":
            card.appName = c.meetingTitle ?? c.title
            card.openHint = s.opensMeeting
        case "thread":
            card.speaker = c.sender
            card.openHint = s.opensThread
        case "meeting":
            card.openHint = s.opensMeeting
        default:
            card.appName = c.title
        }
        return card
    }

    /// A numbered source as a card carrying its number. A row that could not be read just now (`unreadable`, with `found == false`) is not a
    /// row that is gone: its card says so and never claims it is no longer on this device.
    static func numberedCard(_ source: CoreNumberedSource, env: FindCardEnv) -> WFindCard {
        var base = card(source.card, env: env)
        if source.unreadable, !source.card.found {
            base.isGone = false
            base.isUnreadable = true
        }
        return base.numbered(source.n)
    }

    /// The numbered sources an answer cited, as cards carrying their numbers, in number order. `cited` is the core's list.
    static func sourceCards(_ sources: [CoreNumberedSource], cited: [Int], env: FindCardEnv) -> [WFindCard] {
        cited.sorted().compactMap { n in sources.first { $0.n == n }.map { numberedCard($0, env: env) } }
    }

    // MARK: Filters

    static func coreFilter(_ f: WHomeFilter, now: Date = Date(), calendar: Calendar = .current) -> CoreFindFilter {
        var out = CoreFindFilter()
        if let dates = f.dates {
            let b = dates.bounds(now: now, calendar: calendar)
            out.startUnixMs = Int64(b.start.timeIntervalSince1970 * 1000)
            out.endUnixMs = Int64(b.end.timeIntervalSince1970 * 1000)
        }
        out.appBundleId = f.appId
        if let person = f.person { out.person = CoreFindFilter.Person(name: person) }
        return out
    }

    /// Home's block Open and the old chips carry a `LibraryCaptureFilter`: the same filter as the shell's chips draw it. A time range
    /// that is not whole days becomes an exact range under its own label.
    static func homeFilter(_ f: LibraryCaptureFilter) -> WHomeFilter {
        var out = WHomeFilter(appId: f.appId, person: f.person)
        if f.startUnixMs > 0, f.endUnixMs > f.startUnixMs {
            let start = Date(timeIntervalSince1970: Double(f.startUnixMs) / 1000), end = Date(timeIntervalSince1970: Double(f.endUnixMs) / 1000)
            out.dates = .exact(start: start, end: end, title: f.dateLabel ?? MeetingCardModel.timeRange(start: start, end: end))
        }
        return out
    }

    /// "Nov 9 to Nov 15": the days a date filter covers (the last day shown is the one before the end).
    static func rangeText(_ f: WHomeFilter, now: Date = Date(), calendar: Calendar = .current, locale: Locale = .current) -> String? {
        guard let dates = f.dates else { return nil }
        let b = dates.bounds(now: now, calendar: calendar)
        return rangeText(start: b.start, end: b.end, calendar: calendar, locale: locale)
    }

    /// The range an answer's retrieval actually used (the core's `date_range`), as the days it covers. Nil unless both sides are known: a
    /// range left open on one side has no honest "Nov 9 to Nov 15", and the line then keeps what the filter said.
    static func rangeText(_ range: CoreDateRange?, calendar: Calendar = .current, locale: Locale = .current) -> String? {
        guard let start = range?.startUnixMs, let end = range?.endUnixMs, end > start else { return nil }
        return rangeText(start: Date(timeIntervalSince1970: Double(start) / 1000), end: Date(timeIntervalSince1970: Double(end) / 1000), calendar: calendar, locale: locale)
    }

    static func rangeText(start: Date, end: Date, calendar: Calendar = .current, locale: Locale = .current) -> String? {
        let b = (start: start, end: end)
        let last = calendar.date(byAdding: .second, value: -1, to: b.end) ?? b.end
        let style = Date.FormatStyle(date: .omitted, time: .omitted, locale: locale, calendar: calendar, timeZone: calendar.timeZone).month(.abbreviated).day()
        let from = b.start.formatted(style), to = last.formatted(style)
        if calendar.isDate(b.start, inSameDayAs: last) { return from }
        return String(localized: "\(from) to \(to)")
    }

    /// The filters in words under the field, one line each, for the new rules (docs/ui-redesign/screens/ask.md, "Filters").
    static func filterLines(_ f: WHomeFilter, appName: (String) -> String, range: String?) -> [String] {
        var lines: [String] = []
        if let person = f.person {
            if let range {
                lines.append(String(localized: "Meetings with \(person), and screen captures that mention \(person), from \(range).", comment: "Line under the Library search field while the People and Dates filters are on. The placeholders are a person's name, the same name, then a range of days."))
            } else {
                lines.append(String(localized: "Meetings with \(person), and screen captures that mention \(person).", comment: "Line under the Library search field while the People filter is on. Both placeholders are the same person's name."))
            }
        }
        if let appId = f.appId {
            lines.append(String(localized: "Screen captures from \(appName(appId)) only. Meetings, slides, email and episodes are left out.", comment: "Line under the Library search field while the Apps filter is on. The placeholder is an app's name."))
        }
        if f.person == nil, let range {
            lines.append(String(localized: "Only what was saved from \(range).", comment: "Line under the Library search field while the Dates filter is on. The placeholder is a range of days, for example Nov 9 to Nov 15."))
        }
        return lines
    }

    /// The filters as a short list for the no-results line: "People: Sam Okafor, Dates: Last week".
    static func filterSummary(_ f: WHomeFilter, appName: (String) -> String, datesTitle: String?, strings: WHomeStrings) -> String? {
        var parts: [String] = []
        if let person = f.person { parts.append("\(strings.peopleFilter): \(person)") }
        if let appId = f.appId { parts.append("\(strings.appsFilter): \(appName(appId))") }
        if let datesTitle { parts.append("\(strings.datesFilter): \(datesTitle)") }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }

    // MARK: The filters a Find conversation keeps

    /// What a conversation keeps of the filters a question was sent with: the bounds and the app it was sent with, and only whether a
    /// person was in it.
    static func savedFindFilter(_ sent: CoreFindFilter?) -> CoreSavedFindFilter {
        CoreSavedFindFilter(
            startUnixMs: sent?.startUnixMs, endUnixMs: sent?.endUnixMs, appBundleId: sent?.appBundleId, peopleLeftOut: sent?.person != nil)
    }

    static func scopeKind(_ scope: CoreConversationScope) -> WConversationScopeKind {
        switch scope {
        case .everything: return .everything
        case .topic: return .topic
        case .meeting: return .meeting
        }
    }

    static func ui(_ saved: CoreSavedFindFilter) -> WSavedFindFilter {
        WSavedFindFilter(startUnixMs: saved.startUnixMs, endUnixMs: saved.endUnixMs, appBundleId: saved.appBundleId, peopleLeftOut: saved.peopleLeftOut)
    }

    /// The chips a reopened Find conversation shows and asks under: its app and the days it asked about, as "Nov 9 to Nov 15". No person.
    static func reopenedFilter(_ saved: CoreSavedFindFilter, calendar: Calendar = .current, locale: Locale = .current) -> WHomeFilter {
        let kept = ui(saved)
        let title = kept.range.flatMap { rangeText(start: $0.start, end: $0.end, calendar: calendar, locale: locale) } ?? ""
        return kept.homeFilter(title: title)
    }

    /// The filter a follow-up of a kept Find conversation is asked under; nil when only a left-out person was kept.
    static func coreFilter(_ saved: CoreSavedFindFilter) -> CoreFindFilter? {
        let f = CoreFindFilter(startUnixMs: saved.startUnixMs, endUnixMs: saved.endUnixMs, appBundleId: saved.appBundleId, person: nil)
        return f.isEmpty ? nil : f
    }

    /// Said when a reopened conversation was asked under a People filter, which conversations do not keep.
    static var peopleFilterNotKept: String {
        String(localized: "The People filter was not kept with this conversation, so follow-ups are not limited to a person.",
               comment: "Note on a reopened conversation whose questions were asked under a People filter. Conversations keep the Dates and Apps filters only, never a person.")
    }

    /// A filter change after an answer starts a new conversation: an answer made under one set of filters is never fed into a question under another.
    /// Compared as they bound Ask (`WFindConversationFilter`): a reopened conversation's kept days, shown as an exact range, are not a
    /// change from the preset they were asked under.
    static func startsNewConversation(old: WHomeFilter, new: WHomeFilter, hasExchanges: Bool) -> Bool {
        WFindConversationFilter.startsNewConversation(old: old, new: new, hasExchanges: hasExchanges)
    }

    // MARK: Route

    static func route(_ preview: CoreRoutePreview?) -> WFindRoute? {
        guard let preview else { return nil }
        let destination: WFindRoute.Destination
        switch preview.destination {
        case "local": destination = .local
        case "hosted": destination = .hosted
        case "refused": destination = .refused
        default: destination = .unknown
        }
        // "local" is true only for a local destination: anything else never reads "on this Mac".
        if destination == .local, !preview.local { return WFindRoute(destination: .unknown) }
        let name = preview.provider.map { RoutingSettings.Provider(rawValue: $0)?.displayName ?? $0 }
        return WFindRoute(destination: destination, provider: preview.provider, providerName: name, mayFallBack: preview.mayFallBack)
    }

    // MARK: Saved answers

    /// The chips of an answer saved before numbered sources, from its `citations_json` (today's `SavedCitation`).
    static func legacyChips(_ json: String?) -> [WFindLegacyChip] {
        guard let json, let saved = try? JSONDecoder().decode([SavedCitation].self, from: Data(json.utf8)) else { return [] }
        return saved.map { citation in
            let kind = citation.item.kind
            let glyph: String
            switch kind {
            case .capture: glyph = "macwindow"
            case .transcript: glyph = "waveform"
            case .routineRun: glyph = "bolt"
            case .episode: glyph = "rectangle.stack"
            case .thread: glyph = "envelope"
            case .snapshot: glyph = "photo"
            case .mcpItem: glyph = "tray.and.arrow.down"
            }
            var open: WFindOpen?
            if let target = citation.open {
                switch target.kind {
                case "capture": open = .capture(target.id)
                case "meeting": open = .meeting(target.id)
                case "episode": open = .episode(target.id)
                case "thread": open = .thread(target.id)
                default: open = nil
                }
            }
            return WFindLegacyChip(id: kind.rawValue + "-" + String(citation.item.id), glyph: glyph, text: citation.label, tooltip: citation.label + ": " + citation.text, open: open)
        }
    }

    // MARK: Recent and Episodes

    /// "Completed", "Recording" and so on for a meeting row.
    static func statusWord(_ status: CoreMeetingStatus) -> String {
        switch status {
        case .recording: return String(localized: "Recording")
        case .completed: return String(localized: "Completed")
        case .scheduled: return String(localized: "Scheduled")
        case .cancelled: return String(localized: "Cancelled")
        }
    }

    /// Recent: screen captures (when shown) and meetings, newest first. A capture opens its saved capture; a meeting opens the meeting, and keeps
    /// Open in new window and, unless it is recording, Forget meeting.
    static func recentRows(captures: [CoreCaptureItem], meetings: [CoreMeeting], env: FindCardEnv) -> [WLibraryRow] {
        var rows: [WLibraryRow] = []
        for item in captures {
            var card = WFindCard(
                id: "capture-" + String(item.id), kind: .seenOnScreen, appGlyph: env.glyph(item.appBundleId), appName: env.appName(item.appBundleId),
                appDot: env.dot(item.appBundleId), title: item.windowTitle.isEmpty ? env.appName(item.appBundleId) : item.windowTitle,
                timeMs: item.tsUnixMs, time: timeText(item.tsUnixMs, env: env), device: env.device(item.deviceId), text: item.text,
                open: .capture(item.id), openHint: env.strings.opensSavedCapture)
            card.entityKey = card.id
            rows.append(WLibraryRow(id: card.id, card: card))
        }
        for m in meetings {
            let attendees = String(localized: "\(m.attendees.count) attendees")
            let card = WFindCard(
                id: "meeting-" + String(m.id), kind: .meeting, appName: env.strings.kindMeeting, title: m.title, timeMs: m.startUnixMs,
                time: LibraryTimeText.formattedMeetingTime(startUnixMs: m.startUnixMs, endUnixMs: m.endUnixMs), device: env.device(m.deviceId),
                open: .meeting(m.id), openHint: env.strings.opensMeeting)
            rows.append(WLibraryRow(
                id: card.id, card: card, statusLine: statusWord(m.status) + ", " + attendees, canOpenInNewWindow: true, canForget: m.status != .recording))
        }
        return rows.sorted { ($0.card.timeMs ?? 0) > ($1.card.timeMs ?? 0) }
    }

    static func episodeSummary(_ e: CoreEpisode) -> String {
        switch e.status {
        case "summarized": return e.summary ?? String(localized: "Summary unavailable")
        case "summarize_failed": return String(localized: "Summary unavailable")
        default: return String(localized: "Summarizing…")
        }
    }

    /// Episodes, newest first as the core returns them; hidden ones only when asked for.
    static func episodeRows(_ episodes: [CoreEpisode], hidden: Set<Int64>, showHidden: Bool, env: FindCardEnv) -> [WLibraryRow] {
        episodes.filter { showHidden || !hidden.contains($0.id) }.map { e in
            let card = WFindCard(
                id: "episode-" + String(e.id), kind: .episode, appGlyph: env.glyph(e.appBundleId), appName: env.appName(e.appBundleId),
                appDot: env.dot(e.appBundleId), title: e.windowTitle ?? env.appName(e.appBundleId), timeMs: e.startTsUnixMs,
                time: LibraryTimeText.formattedEpisodeTime(startUnixMs: e.startTsUnixMs, endUnixMs: e.endTsUnixMs), device: env.device(e.deviceId),
                text: episodeSummary(e), open: .episode(e.id), openHint: env.strings.opensEpisode)
            return WLibraryRow(id: card.id, card: card, isHidden: hidden.contains(e.id))
        }
    }

    // MARK: Words

    /// The page draws no dash, and never changes a word of a person's, a meeting's or a model's: so the app messages it reuses that held a
    /// dash have a dash-free message of their own, in the catalog with their translations, and the rest are shown as the old screens show them.

    /// What a failed ask says. The one reused message with a dash in it has its own wording; every other message is the old screen's own.
    static func failureText(_ error: CoreLLMError) -> String {
        if case .spendCap(.global, let spentCents, let capCents) = error {
            return String(
                localized: "The app-wide daily spend limit was reached (\(String(spentCents))¢/\(String(capCents))¢). This isn't specific to you. Try again later.",
                comment: "Shown on Library's Find page when the app-wide daily spend limit stops an answer. The placeholders are cents spent and the limit in cents.")
        }
        return error.userFacingMessage
    }

    /// The capture state line of Library's status header, as `AppState.statusSummary` words it but with no dash: the meeting's title is joined
    /// with a middle dot and is never altered.
    static func statusLine(
        recordingPaused: Bool, meetingTitle: String?, systemAudio: Bool, mic: Bool, screenText: Bool, captureEnabled: Bool
    ) -> String {
        let joiner = " \u{00B7} "
        if recordingPaused, let meetingTitle { return String(localized: "Recording paused") + joiner + meetingTitle }
        var parts: [String] = []
        if systemAudio { parts.append(String(localized: "system audio")) }
        if mic { parts.append(String(localized: "mic")) }
        if screenText { parts.append(String(localized: "screen text")) }
        let suffix: String
        if parts.isEmpty {
            suffix = captureEnabled
                ? String(localized: "Activity capture on, waiting for permissions", comment: "Library's status line while capture is on but nothing is capturing yet, for lack of permissions.")
                : String(localized: "Activity capture paused")
        } else {
            suffix = String(localized: "Capturing: \(parts.joined(separator: ", "))")
        }
        guard let meetingTitle else { return suffix }
        return suffix + joiner + meetingTitle
    }

    // MARK: Counts

    static func countText(total: Int, mentions: Int, answerSources: Int?, strings: WFindStrings) -> String? {
        if let answerSources, answerSources > 0 { return strings.sourceCount(answerSources) }
        let n = total + mentions
        return n > 0 ? strings.resultCount(n) : nil
    }
}

extension WHomeAsk {
    /// Ask can run: a provider is set up (the header field shows) rather than a button into setup.
    var isAvailable: Bool { if case .available = self { return true } else { return false } }
}
