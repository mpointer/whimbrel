import Foundation

// Library's Find page as plain values (docs/ui-redesign/screens/ask.md). A model says what to show; text that belongs
// to the screen arrives in it, already in the person's language. `WFindActions` says what each control does. Nothing
// here knows the app. The pure rules (where a source mark goes, how results group into days, what the Ask button is
// called, what a finished stream does) live here too, so `swift test` covers them. The views are in FindViews.swift.

/// An id built from a kind and a value ("source.2"): the parts joined by a dot.
func wid(_ kind: String, _ value: Any) -> String { kind + "." + String(describing: value) }

// MARK: Kinds and opening

/// What opening a card does. The app turns each into its own window or tab.
public enum WFindOpen: Equatable, Sendable {
    case capture(Int64)
    /// The meeting's transcript at a line, `startCs` centiseconds into the meeting.
    case meetingAt(id: Int64, startCs: Int64)
    case meeting(Int64)
    case episode(Int64)
    case thread(Int64)
}

extension WSourceKind {
    /// The icon for the kind, beside its words.
    public var glyph: String {
        switch self {
        case .seenOnScreen: return "macwindow"
        case .transcriptMoment: return "waveform"
        case .slide: return "photo"
        case .email: return "envelope"
        case .episode: return "rectangle.stack"
        case .routineRun: return "bolt"
        case .meeting: return "video"
        }
    }

    /// The kind in words on a Find card, per the Find spec ("Email thread", "Automation run"). The
    /// source card elsewhere says "Email" and "Routine run" (`label(using:)`).
    public func findLabel(using strings: WStrings) -> String {
        switch self {
        case .email: return strings.find.kindEmailThread
        case .routineRun: return strings.find.kindAutomationRun
        default: return label(using: strings)
        }
    }
}

// MARK: Cards

/// One result, or one numbered source. A card whose row is gone says so and holds no text.
public struct WFindCard: Identifiable, Equatable, Sendable {
    public var id: String
    /// What the card is, "capture-12": the same for a result and for that row as a numbered source, so a source is not listed twice.
    public var entityKey: String
    /// The number the model saw, on a source of an answer.
    public var number: Int?
    public var kind: WSourceKind
    public var appGlyph: String
    /// The app's name for a capture or an episode; for a transcript moment, a slide and a meeting, what they come from.
    public var appName: String?
    public var appDot: WAppKind?
    public var title: String
    public var timeMs: Int64?
    /// "Thu, Nov 12, 4:10 PM".
    public var time: String
    public var device: String?
    /// The stored text, cut to length by the core. Nil when the row is gone.
    public var text: String?
    public var speaker: String?
    /// A transcript line's place in its meeting, "14:32".
    public var lineTime: String?
    /// Why a line matched a person ("Spoken by Sam Okafor"), already in words.
    public var via: String?
    public var isGone: Bool
    /// The database could not be read just now, so what the row holds is not known. Not the same as `isGone`: nothing says the row was
    /// deleted, and the card must not say "no longer on this device".
    public var isUnreadable: Bool
    public var open: WFindOpen?
    /// Under the Open button: "Opens the saved capture".
    public var openHint: String?

    public init(
        id: String, number: Int? = nil, kind: WSourceKind, appGlyph: String? = nil, appName: String? = nil, appDot: WAppKind? = nil,
        title: String, timeMs: Int64? = nil, time: String = "", device: String? = nil, text: String? = nil, speaker: String? = nil,
        lineTime: String? = nil, via: String? = nil, isGone: Bool = false, isUnreadable: Bool = false, open: WFindOpen? = nil, openHint: String? = nil
    ) {
        self.id = id
        self.entityKey = id
        self.number = number
        self.kind = kind
        self.appGlyph = appGlyph ?? kind.glyph
        self.appName = appName
        self.appDot = appDot
        self.title = title
        self.timeMs = timeMs
        self.time = time
        self.device = device
        self.text = text
        self.speaker = speaker
        self.lineTime = lineTime
        self.via = via
        self.isGone = isGone
        self.isUnreadable = isUnreadable
        self.open = open
        self.openHint = openHint
    }

    /// The same card as the numbered source `n`.
    public func numbered(_ n: Int) -> WFindCard {
        var copy = self
        copy.number = n
        copy.id = wid("source", n)
        return copy
    }
}

/// A day on the time line: its heading and the cards in it.
public struct WFindDay: Identifiable, Equatable, Sendable {
    public var id: String
    public var heading: String
    public var cards: [WFindCard]
    public init(id: String, heading: String, cards: [WFindCard]) {
        self.id = id
        self.heading = heading
        self.cards = cards
    }
}

public enum WFindOrder: String, CaseIterable, Sendable {
    case byDay, bestFirst
    /// The order name the core takes.
    public var coreName: String { self == .byDay ? "time" : "rank" }

    /// The order a new search opens on: a question ("Where is the kite festival?", "who owns the budget") opens on best matches
    /// first, since the newest row holding one of its words is rarely the answer; plain words open by day. The picker still changes it.
    public static func `default`(for words: String) -> WFindOrder {
        let w = words.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if w.isEmpty { return .byDay }
        if w.hasSuffix("?") || w.hasSuffix("\u{FF1F}") { return .bestFirst }
        let trimmed = w.drop(while: { "\u{00BF}\u{00A1}\"'\u{201C}\u{2018}(".contains($0) })
        let first = trimmed.split(whereSeparator: { $0.isWhitespace || $0 == "," }).first.map(String.init) ?? ""
        return WFindQuestionWords.all.contains(first) ? .bestFirst : .byDay
    }
}

// MARK: Days and source order (pure)

public enum WFindDays {
    /// Groups cards under days, in the order they come, so a newest-first list stays newest-first. A card with no time
    /// (its row is gone) goes last under `undated`. A card whose row could not be read just now has no time either, but is not gone: it
    /// goes under `unreadable` after those, so no heading says "no longer on this device" over a row nobody knows is deleted. With no
    /// `unreadable` heading it falls under `undated` as before.
    public static func group(
        _ cards: [WFindCard], today: String, yesterday: String, undated: String, unreadable: String? = nil, now: Date = Date(), calendar: Calendar = .current
    ) -> [WFindDay] {
        var order: [String] = []
        var heading: [String: String] = [:]
        var buckets: [String: [WFindCard]] = [:]
        var undatedCards: [WFindCard] = []
        var unreadableCards: [WFindCard] = []
        for card in cards {
            if card.isUnreadable, unreadable != nil { unreadableCards.append(card); continue }
            guard let ms = card.timeMs else { undatedCards.append(card); continue }
            let date = Date(timeIntervalSince1970: Double(ms) / 1000)
            let start = calendar.startOfDay(for: date)
            let key = String(Int64(start.timeIntervalSince1970))
            if buckets[key] == nil {
                order.append(key)
                heading[key] = dayHeading(date, today: today, yesterday: yesterday, now: now, calendar: calendar)
            }
            buckets[key, default: []].append(card)
        }
        var days = order.map { WFindDay(id: wid("day", $0), heading: heading[$0] ?? "", cards: buckets[$0] ?? []) }
        if !undatedCards.isEmpty { days.append(WFindDay(id: wid("day", "undated"), heading: undated, cards: undatedCards)) }
        if let unreadable, !unreadableCards.isEmpty { days.append(WFindDay(id: wid("day", "unreadable"), heading: unreadable, cards: unreadableCards)) }
        return days
    }

    /// "Today", "Yesterday", else "Tue, Nov 10" (with the year when it is not this one).
    public static func dayHeading(_ date: Date, today: String, yesterday: String, now: Date, calendar: Calendar) -> String {
        let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: now)).day ?? 0
        if days == 0 { return today }
        if days == 1 { return yesterday }
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = calendar.locale ?? Locale.current
        let sameYear = calendar.isDate(date, equalTo: now, toGranularity: .year)
        formatter.setLocalizedDateFormatFromTemplate(sameYear ? WFindStrings.dayTemplate : WFindStrings.dayTemplateWithYear)
        return formatter.string(from: date)
    }

    /// An answer's sources as evidence: oldest first, so the time line reads forward; a source with no time (gone) last. Ties
    /// keep their numbers' order.
    public static func sourcesInTimeOrder(_ sources: [WFindCard]) -> [WFindCard] {
        let indexed = sources.enumerated().map { (index: $0.offset, card: $0.element) }
        return indexed.sorted { a, b in
            switch (a.card.timeMs, b.card.timeMs) {
            case let (x?, y?): return x != y ? x < y : a.index < b.index
            case (nil, nil): return a.index < b.index
            case (nil, _): return false
            case (_, nil): return true
            }
        }.map(\.card)
    }
}

// MARK: Source marks in an answer (pure)

/// One run of an answer: words, or the mark of a source.
public enum WAnswerPart: Equatable, Sendable {
    case text(String)
    case mark(Int)
}

public enum WAnswerMarks {
    /// Finds the `[n]` markers of an answer's words: a `[`, ASCII digits only, then `]`. `[1, 2]`, `[x]` and `[3 and` are not markers and
    /// stay in the words. Which markers are drawn as source marks is `cited`'s to say, and nothing else's: the numbers the core returned
    /// for this very answer (`cited`, or the saved sources of a reopened one). A marker whose number is not in it stays in the words as
    /// plain text, so the page never marks something the core did not count as a citation. The core reads citations with a stricter
    /// grammar than this scan (it skips fenced code with backticks or tildes, inline code and a bracket followed by "("). The answer's
    /// blocks and Markdown cover the same cases, so the two agree; the one disagreement left is a number the core cited once that the
    /// answer also writes inside code this page does not recognise as code, which then also gets a mark there.
    public static func parts(_ text: String, cited: Set<Int>) -> [WAnswerPart] {
        var parts: [WAnswerPart] = []
        var pending = ""
        let chars = Array(text)
        var i = 0
        func flush() { if !pending.isEmpty { parts.append(.text(pending)); pending = "" } }
        while i < chars.count {
            if chars[i] == "[" {
                var j = i + 1
                var digits = ""
                while j < chars.count, chars[j].isASCII, chars[j].isNumber { digits.append(chars[j]); j += 1 }
                if !digits.isEmpty, j < chars.count, chars[j] == "]", let n = Int(digits), cited.contains(n) {
                    flush()
                    parts.append(.mark(n))
                    i = j + 1
                    continue
                }
            }
            pending.append(chars[i])
            i += 1
        }
        flush()
        return parts
    }

    /// The numbers the answer marks among `cited`, in first-marked order without repeats.
    public static func numbers(in text: String, cited: Set<Int>) -> [Int] {
        var out: [Int] = []
        for case .mark(let n) in parts(text, cited: cited) where !out.contains(n) { out.append(n) }
        return out
    }

    /// The answer as words only, for Copy answer and for a screen reader: markers of a cited source are dropped, with the space
    /// before them, so "Fridays [1]." reads "Fridays." Other bracketed text stays.
    public static func plain(_ text: String, cited: Set<Int>) -> String {
        var out = ""
        for part in parts(text, cited: cited) {
            switch part {
            case .text(let words): out += words
            case .mark:
                while out.last == " " { out.removeLast() }
            }
        }
        return out
    }
}

// MARK: The Ask button's promise (pure)

/// Where the core's preview says an answer will come from, as raw facts. The words on the button come from `WFindAskButton.make`.
public struct WFindRoute: Equatable, Sendable {
    public enum Destination: String, Equatable, Sendable { case local, hosted, refused, unknown }
    public var destination: Destination
    /// The provider's id as the core names it ("anthropic", "ollama"), for a hosted or a local answer.
    public var provider: String?
    /// For a hosted answer, the provider's name as a person knows it ("Anthropic").
    public var providerName: String?
    /// A failure may move the answer to another provider (the core says so for every streamed hosted answer). Unknown counts as true.
    public var mayFallBack: Bool
    public init(destination: Destination, provider: String? = nil, providerName: String? = nil, mayFallBack: Bool = true) {
        self.destination = destination
        self.provider = provider
        self.providerName = providerName
        self.mayFallBack = mayFallBack
    }
}

public struct WFindAskButton: Equatable, Sendable {
    public var title: String
    public var isEnabled: Bool
    /// The button promised this Mac. The ask must then pass `require_local` and never reach a hosted model.
    public var promisesLocal: Bool
    /// The button named the provider that will answer ("Anthropic"). Only when the preview says one hosted provider and no fallback.
    public var namesProvider: String?

    public init(title: String, isEnabled: Bool = true, promisesLocal: Bool = false, namesProvider: String? = nil) {
        self.title = title
        self.isEnabled = isEnabled
        self.promisesLocal = promisesLocal
        self.namesProvider = namesProvider
    }

    /// The same button with a different enabled state (the words are a promise and do not change with it).
    public func enabled(_ canAsk: Bool) -> WFindAskButton {
        var copy = self
        copy.isEnabled = canAsk
        return copy
    }

    /// "Ask, on this Mac" only when the core's preview said local. "Ask Anthropic" only when the preview says a hosted answer from one
    /// named provider that cannot move to another: a streamed hosted answer can (a failed stream, or a spent budget, answers through
    /// the whole provider chain), so it says plain "Ask", and the "Asking ..." line and the model line name who answered. An Ollama
    /// model that leaves this Mac (a cloud model) is hosted too: it never reads as local, and is not named. Plain "Ask" when the route
    /// cannot be known before sending (no preview yet, unknown, refused: the ask then fails with its own message).
    public static func make(route: WFindRoute?, canAsk: Bool, strings: WFindStrings) -> WFindAskButton {
        guard let route else { return WFindAskButton(title: strings.askButton, isEnabled: canAsk) }
        switch route.destination {
        case .local:
            return WFindAskButton(title: strings.askOnThisMac, isEnabled: canAsk, promisesLocal: true)
        case .hosted:
            guard !route.mayFallBack, route.provider != "ollama", let name = route.providerName, !name.isEmpty else {
                return WFindAskButton(title: strings.askButton, isEnabled: canAsk)
            }
            return WFindAskButton(title: strings.askProvider(name), isEnabled: canAsk, namesProvider: name)
        case .refused, .unknown:
            return WFindAskButton(title: strings.askButton, isEnabled: canAsk)
        }
    }
}

/// The decision at the moment of sending: the label the person saw, against a preview made for the exact question, history, filters
/// and mode that are about to be sent. Pure, so `swift test` covers it.
public enum WFindSendDecision: Equatable, Sendable {
    /// Send it. `requireLocal` is passed to every call of the ask, the streamed one's blocking fallback included.
    case send(requireLocal: Bool)
    /// The label made a promise the fresh preview no longer keeps: send nothing, say so, and refresh the label.
    case changed

    /// - label: what the Ask button said when it was pressed; nil when no button was on screen (Command Return from another tab, a hand-off
    ///   from Today, a follow-up, a New Conversation prompt): such an ask promised nothing.
    /// - fresh: the preview of what is about to be sent; nil when the core could not say.
    public static func decide(label: WFindAskButton?, fresh: WFindRoute?) -> WFindSendDecision {
        if label?.promisesLocal == true {
            return fresh?.destination == .local ? .send(requireLocal: true) : .changed
        }
        if let named = label?.namesProvider {
            if let fresh, fresh.destination == .hosted, !fresh.mayFallBack, fresh.provider != "ollama", fresh.providerName == named { return .send(requireLocal: false) }
            return .changed
        }
        // No promise: a question the core would answer on this Mac stays on it, whatever the stream route says.
        return .send(requireLocal: fresh?.destination == .local)
    }
}

// MARK: What a finished stream does (pure)

/// What the app does when a streamed answer ends. The same rules as today's Library, as a table the tests can read.
public enum WAskStreamEnd: Equatable, Sendable {
    /// Finalize the prepared ask with the streamed text and show the answer.
    case finalize
    /// Throw the prepared ask away and put the question back in the field. Nothing is shown.
    case discardAndRestoreQuestion
    /// Throw the prepared ask away, restore the question and show this failure.
    case failWith(WAskFailure)
    /// Nothing was ever emitted: throw the prepared ask away and answer through the blocking call. `requireLocal` is the promise
    /// the button made, and the blocking call keeps it.
    case fallBackToBlocking(requireLocal: Bool)

    public enum WAskFailure: Equatable, Sendable {
        case creditExhausted, exclusionUnreadable, providerFailure
    }

    public enum Phase: Equatable, Sendable {
        case completed, cancelled, failed
    }

    /// - hadOutput: the stream had already emitted text.
    /// - isWhimbrelAI402: the Whimbrel AI stream failed with an HTTP 402 (its credit is used up).
    /// - exclusionUnreadable: the exclusion setting could not be read, so nothing hosted went.
    public static func decide(
        phase: Phase, hadOutput: Bool, isWhimbrelAI402: Bool, exclusionUnreadable: Bool, requireLocal: Bool
    ) -> WAskStreamEnd {
        switch phase {
        case .completed: return .finalize
        case .cancelled: return .discardAndRestoreQuestion
        case .failed:
            if isWhimbrelAI402 { return .failWith(.creditExhausted) }
            if exclusionUnreadable { return .failWith(.exclusionUnreadable) }
            if hadOutput { return .failWith(.providerFailure) }
            return .fallBackToBlocking(requireLocal: requireLocal)
        }
    }

    /// Whether the stream may start at all. A button that said "on this Mac" streams only to the on-device model: a stream route to a
    /// hosted provider is then not used, and the ask goes to the blocking call with `require_local`.
    public static func mayStream(promisesLocal: Bool, streamIsOnDevice: Bool, hasStreamRoute: Bool) -> Bool {
        guard hasStreamRoute else { return false }
        return !promisesLocal || streamIsOnDevice
    }
}

// MARK: The page

/// The legacy chip of an answer saved before numbered sources.
public struct WFindLegacyChip: Identifiable, Equatable, Sendable {
    public var id: String
    public var glyph: String
    public var text: String
    public var tooltip: String
    public var open: WFindOpen?
    public init(id: String, glyph: String, text: String, tooltip: String, open: WFindOpen? = nil) {
        self.id = id
        self.glyph = glyph
        self.text = text
        self.tooltip = tooltip
        self.open = open
    }
}

/// One question and its answer.
public struct WFindExchange: Identifiable, Equatable, Sendable {
    public enum Body: Equatable, Sendable {
        case answered(text: String)
        /// A failed ask: its message, and the name of the button that goes with it (open AI settings) when there is one.
        case failed(message: String, buttonTitle: String?)
        /// Saved before numbered sources: today's plain chips.
        case legacy(text: String, chips: [WFindLegacyChip])
    }

    public var id: String
    public var question: String
    /// "From your work memory, Nov 9 to Nov 15".
    public var scope: String
    public var body: Body
    /// "Answered by claude-sonnet" and so on, below the answer.
    public var modelLine: String?
    /// Answered locally, left out N items, fallback: each in its existing words.
    public var notes: [String]
    /// The numbered sources the model saw and the answer cited; each card carries its number.
    public var sources: [WFindCard]
    /// The numbers the core returned as cited for this answer (a reopened answer: its saved sources). The only numbers drawn as marks.
    public var cited: [Int]

    /// - cited: nil takes the numbers of `sources`, which are the cited ones (the sources of an answer are only the cited ones).
    public init(
        id: String, question: String, scope: String, body: Body, modelLine: String? = nil, notes: [String] = [], sources: [WFindCard] = [], cited: [Int]? = nil
    ) {
        self.cited = cited ?? sources.compactMap(\.number)
        self.id = id
        self.question = question
        self.scope = scope
        self.body = body
        self.modelLine = modelLine
        self.notes = notes
        self.sources = sources
    }

    public var citedNumbers: Set<Int> { Set(cited) }
}

public struct WFindConversation: Identifiable, Equatable, Sendable {
    public var id: Int64
    public var title: String
    public var starred: Bool
    public var isActive: Bool
    /// False while an answer is being made in this conversation: deleting it then would drop the answer. Rename stays.
    public var canDelete: Bool
    public init(id: Int64, title: String, starred: Bool, isActive: Bool, canDelete: Bool = true) {
        self.id = id
        self.title = title
        self.starred = starred
        self.isActive = isActive
        self.canDelete = canDelete
    }
}

public struct WFindStarter: Identifiable, Equatable, Sendable {
    public var id: String { title }
    public var title: String
    public var query: String
    public init(title: String, query: String) {
        self.title = title
        self.query = query
    }
}

/// What the ask is doing now.
public enum WFindAsking: Equatable, Sendable {
    case idle
    /// Before any text: the stage words that exist today ("Asking ..."). `canStop` is true while something can still be cancelled (the
    /// sign-in refresh, the preview, the preparing of the prompt); the blocking model call cannot be, as today.
    case waiting(line: String, canStop: Bool)
    /// The answer is arriving. Its text goes to `WFindStream`, not through the model.
    case streaming(question: String, line: String)
}

/// Which field a question was typed in.
public enum WFindField: Equatable, Sendable { case header, followUp }

/// A question going back to the field it came from. The counter makes a repeat of the same text a new request.
public struct WFindRestore: Equatable, Sendable {
    public var token: Int
    public var field: WFindField
    public var text: String
    public init(token: Int, field: WFindField, text: String) {
        self.token = token
        self.field = field
        self.text = text
    }
}

/// The follow-up field's rule: the text leaves the field only once the ask has really started.
public enum WFollowUpField {
    public static func textAfterSending(_ text: String, started: Bool) -> String { started ? "" : text }
}

public enum WFindStage: Equatable, Sendable {
    /// Nothing typed, nothing asked.
    case empty
    case searching
    case results
    case noResults
    case failed(title: String, help: String)
}

public struct WFindModel: Equatable, Sendable {
    public var stage: WFindStage = .empty
    public var query: String = ""
    /// What the filters say in words ("People: Sam Okafor, Dates: Last week"); nil when none is on.
    public var filterSummary: String?
    /// The filters in words under the field, one line each ("Meetings with Sam Okafor, and captures that mention Sam Okafor.").
    public var filterLines: [String] = []
    /// "3 results" or "12 sources found"; nil when there is nothing to count.
    public var countText: String?
    public var order: WFindOrder = .byDay
    public var capped: Bool = false
    public var noMeaningSearch: Bool = false
    public var starters: [WFindStarter] = []
    public var askButton: WFindAskButton = WFindAskButton(title: "")
    /// The ask can run at all (a provider is set up). False shows the app's set-up slot where the Ask row is.
    public var canAsk: Bool = true
    public var asking: WFindAsking = .idle
    public var exchanges: [WFindExchange] = []
    /// The exchange whose sources the time line shows; the latest unless a mark of an earlier one was chosen.
    public var focusedExchangeId: String?
    /// Set to an exchange's id when an ask finishes (answer or failure), never when a saved conversation is opened: the page says the
    /// answer aloud once per new value.
    public var announceToken: String?
    /// A question that did not go (stopped, failed, held back) and the field it belongs in; a new `token` puts the text back there.
    public var restore: WFindRestore?
    /// A new value moves the keyboard and the screen reader to the latest answer (a saved conversation was opened).
    public var focusToken: Int = 0
    /// Any filter is on: the page then keeps a Clear filters button.
    public var hasFilters: Bool = false
    /// "Stopped. ..." or "You changed the filters ..." for one moment after it happens.
    public var notice: String?
    /// Results of the current search, in the chosen order (by time, newest first, for by day).
    public var results: [WFindCard] = []
    public var mentions: [WFindCard] = []
    public var mentionsName: String?
    public var mentionsTotal: Int = 0
    public var more: Bool = false
    public var loadingMore: Bool = false
    public var conversations: [WFindConversation] = []
    public var showsConversations: Bool = true
    public var followUpAvailable: Bool = true
    /// A new value sends the follow-up field's text, as Return does (Command Return while that field has the keyboard).
    public var submitFollowUpToken: Int = 0

    public init() {}

    public var focusedExchange: WFindExchange? {
        exchanges.first { $0.id == focusedExchangeId } ?? exchanges.last
    }
}

/// What each control does. Every closure has an empty default, so a gallery needs none.
public struct WFindActions {
    public var open: (WFindOpen) -> Void = { _ in }
    public var ask: () -> Void = {}
    /// Returns whether the ask started: the follow-up field keeps its text when it did not (busy, empty).
    public var askFollowUp: (String) -> Bool = { _ in false }
    public var stop: () -> Void = {}
    public var copyAnswer: (String) -> Void = { _ in }
    public var focusExchange: (String) -> Void = { _ in }
    public var setOrder: (WFindOrder) -> Void = { _ in }
    public var loadMore: () -> Void = {}
    public var starter: (WFindStarter) -> Void = { _ in }
    public var clearFilters: () -> Void = {}
    public var retry: () -> Void = {}
    public var failureButton: () -> Void = {}
    public var newConversation: () -> Void = {}
    public var openConversation: (Int64) -> Void = { _ in }
    public var toggleStar: (Int64) -> Void = { _ in }
    public var rename: (Int64) -> Void = { _ in }
    public var deleteConversation: (Int64) -> Void = { _ in }
    public var toggleConversations: () -> Void = {}
    /// The follow-up field gained or lost the keyboard: the app sends Command Return to it while it has it.
    public var followUpFocus: (Bool) -> Void = { _ in }

    public init() {}
}

/// The answer as it streams in. Only the answer card reads it, so a streamed token redraws that card and nothing else.
@MainActor
public final class WFindStream: ObservableObject {
    @Published public var text: String = ""
    public init(text: String = "") { self.text = text }
}

// MARK: Library views

public enum WLibraryView: String, CaseIterable, Sendable { case find, recent, episodes, digest }

/// A row of Recent or Episodes: the same card, plus the actions a meeting or an episode row keeps.
public struct WLibraryRow: Identifiable, Equatable, Sendable {
    public var id: String
    public var card: WFindCard
    /// A meeting row: its status word ("Completed") and attendee count.
    public var statusLine: String?
    public var isHidden: Bool
    public var canOpenInNewWindow: Bool
    public var canForget: Bool

    public init(id: String, card: WFindCard, statusLine: String? = nil, isHidden: Bool = false, canOpenInNewWindow: Bool = false, canForget: Bool = false) {
        self.id = id
        self.card = card
        self.statusLine = statusLine
        self.isHidden = isHidden
        self.canOpenInNewWindow = canOpenInNewWindow
        self.canForget = canForget
    }
}

public struct WLibraryListModel: Equatable, Sendable {
    public var rows: [WLibraryRow] = []
    public var isLoading: Bool = false
    public var emptyTitle: String = ""
    public var emptyHelp: String = ""
    public init() {}
}

public struct WLibraryDigestModel: Equatable, Sendable {
    public enum State: Equatable, Sendable {
        case idle(help: String)
        case generating(line: String)
        case failed(message: String)
        case ready(summary: String, openLoops: [String])
    }
    public var state: State = .idle(help: "")
    public var canGenerate: Bool = true
    public init() {}
}

public struct WLibraryActions {
    public var open: (WFindOpen) -> Void = { _ in }
    public var openInNewWindow: (String) -> Void = { _ in }
    public var forget: (String) -> Void = { _ in }
    public var generateDigest: () -> Void = {}
    public init() {}
}
