import Foundation

// Pure rules the app's shared Ask runner and Find page decide by, here so `swift test` covers them.

/// How a background call that sends a question and its answer to a model (a conversation's title) is routed.
public enum WAskTitleRoute: Equatable, Sendable {
    /// The account's own title route.
    case normal
    /// Only a model on this computer.
    case localOnly
    /// No title call: the placeholder title stays.
    case skip

    /// - keepLocal: the answer was promised or made on this computer (asked with `requireLocal`, made locally, or kept local for a client
    ///   excluded from hosted models). Its words then never go to a hosted model for a title either.
    /// - localModelAvailable: a model on this computer can make the title.
    public static func decide(keepLocal: Bool, localModelAvailable: Bool) -> WAskTitleRoute {
        guard keepLocal else { return .normal }
        return localModelAvailable ? .localOnly : .skip
    }

    /// Whether the title call needs the hosted provider keys and the Whimbrel sign-in. Only the account's own route does: a title kept
    /// on this computer is made with none, and a skipped one reads nothing.
    public var readsCredentials: Bool { self == .normal }
}

/// What a failed ask held to this computer (`requireLocal`) says.
public enum WAskLocalFailure {
    /// Core answers such an ask with a plain provider failure when no model on this computer could answer. With no local model set up,
    /// "Couldn't reach the AI provider" would be false and invite a retry, so the ask says no model here could answer it. A local model
    /// that was there and failed (Ollama stopped, the model crashed partway) keeps the failure's own words, as other failures do.
    public static func saysNoLocalModel(requireLocal: Bool, isProviderFailure: Bool, localModelAvailable: Bool) -> Bool {
        requireLocal && isProviderFailure && !localModelAvailable
    }
}

/// What a conversation in Find's list asks about, as far as opening it goes.
public enum WConversationScopeKind: Equatable, Sendable { case everything, topic, meeting }

/// Where a conversation chosen in Find's list opens.
public enum WFindConversationOpen: Equatable, Sendable {
    /// On Find, which asks over everything under the Find filters.
    case find
    /// On the Conversations page, which keeps a topic's or meeting's scope.
    case conversationsPage

    /// Only an everything conversation opens on Find: Find would ask a topic or meeting conversation's follow-ups over everything,
    /// wider than its scope. Find filters never combine with those scopes.
    public static func destination(_ scope: WConversationScopeKind) -> WFindConversationOpen {
        scope == .everything ? .find : .conversationsPage
    }
}

/// The Find filters a conversation keeps (core's `SavedFindFilter`): the Dates as the days that were asked about and the App. Never a
/// person (no new data about people is stored), only whether a People filter was left out.
public struct WSavedFindFilter: Equatable, Sendable {
    public var startUnixMs: Int64?
    public var endUnixMs: Int64?
    public var appBundleId: String?
    public var peopleLeftOut: Bool

    public init(startUnixMs: Int64?, endUnixMs: Int64?, appBundleId: String?, peopleLeftOut: Bool) {
        self.startUnixMs = startUnixMs
        self.endUnixMs = endUnixMs
        self.appBundleId = appBundleId
        self.peopleLeftOut = peopleLeftOut
    }

    /// What `filter` keeps, its dates fixed to the days they cover at `now` ("today" stays the day it was asked).
    public init(_ filter: WHomeFilter, now: Date, calendar: Calendar = .current) {
        var start: Int64?, end: Int64?
        if let dates = filter.dates {
            let b = dates.bounds(now: now, calendar: calendar)
            start = Int64(b.start.timeIntervalSince1970 * 1000)
            end = Int64(b.end.timeIntervalSince1970 * 1000)
        }
        self.init(startUnixMs: start, endUnixMs: end, appBundleId: filter.appId, peopleLeftOut: filter.person != nil)
    }

    public var isEmpty: Bool { startUnixMs == nil && endUnixMs == nil && appBundleId == nil && !peopleLeftOut }

    /// The chips a reopened conversation shows and asks under: the app, and the kept days as an exact range named `title`. No person.
    public func homeFilter(title: String) -> WHomeFilter {
        var out = WHomeFilter(appId: appBundleId)
        if let range { out.dates = .exact(start: range.start, end: range.end, title: title) }
        return out
    }

    /// The days kept, when both sides are.
    public var range: (start: Date, end: Date)? {
        guard let startUnixMs, let endUnixMs, endUnixMs > startUnixMs else { return nil }
        return (Date(timeIntervalSince1970: Double(startUnixMs) / 1000), Date(timeIntervalSince1970: Double(endUnixMs) / 1000))
    }
}

/// When a change of the Find filters starts a new conversation (ask.md: an answer made under one set of filters is never fed into a
/// question under another).
public enum WFindConversationFilter {
    /// Two filters bound Ask the same way at `now`: the same days, app and person. A preset and the exact range of the days it covers are
    /// the same filter (a reopened conversation shows its kept days as an exact range); a chip's label is not part of it.
    public static func same(_ a: WHomeFilter, _ b: WHomeFilter, now: Date = Date(), calendar: Calendar = .current) -> Bool {
        guard a.appId == b.appId, a.person == b.person else { return false }
        switch (a.dates, b.dates) {
        case (nil, nil): return true
        case let (x?, y?):
            // A relative preset is the same chip whatever the day: compared by which preset it is, so the date rolling over is no change.
            if x.isRelative || y.isRelative, x.isRelative == y.isRelative { return x == y }
            let bx = x.bounds(now: now, calendar: calendar), by = y.bounds(now: now, calendar: calendar)
            return bx.start == by.start && bx.end == by.end
        default: return false
        }
    }

    /// A conversation with answers in it starts over when the filters now bound Ask differently.
    public static func startsNewConversation(
        old: WHomeFilter, new: WHomeFilter, hasExchanges: Bool, now: Date = Date(), calendar: Calendar = .current
    ) -> Bool {
        hasExchanges && !same(old, new, now: now, calendar: calendar)
    }
}

extension WHomeDateRange {
    /// A preset whose days move with the clock (Today, Yesterday, Last 7 days), as against a range of fixed days.
    public var isRelative: Bool {
        switch self {
        case .today, .yesterday, .lastSevenDays: return true
        case .custom, .exact: return false
        }
    }
}

/// Where a question in flight is, for whether Stop may end it.
public enum WAskStage: Equatable, Sendable {
    /// Before the model call: the sign-in refresh, the route check, the preparing of the prompt.
    case preparing
    case streaming
    /// The stream is over and the answer is being finished (its citations, its log): it is answered already.
    case finalizing
    /// The blocking model call.
    case blocking
}

public enum WAskStop {
    /// Stop ends a question being prepared or streamed. A finished answer being finalized is never dropped, and the blocking call has
    /// no Stop, as before.
    public static func mayStop(_ stage: WAskStage) -> Bool {
        stage == .preparing || stage == .streaming
    }
}

public enum WAskStart {
    /// The question a runner asks for `text`: its words without the space around them, or nil when there are none or a question is
    /// already in flight (nothing starts, and the caller is told so).
    public static func question(_ text: String, busy: Bool) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty || busy ? nil : trimmed
    }
}
