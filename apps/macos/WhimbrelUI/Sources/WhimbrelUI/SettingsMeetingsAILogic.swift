import Foundation

// Pure decisions behind the Meetings, AI and Topics sections of the redesigned Settings window. The views call these
// functions, so the tests cover what the views do. Nothing here knows about SwiftUI, the app, the clock or the language.

// MARK: Keys

/// How a key row's last result is drawn. The app keeps the text and whether it was an error, as the classic pane does;
/// this puts them in the part's status line. The key itself never passes through here.
public enum WKeyStatusMapping {
    public static func status(text: String?, isError: Bool) -> WSettingsStatus? {
        guard let text, !text.isEmpty else { return nil }
        return isError ? .problem(text) : .info(text)
    }
}

// MARK: Monthly budget

/// The monthly budget field. Blank and 0 both mean "no budget". A figure above the largest allowed is refused with a
/// message (the classic field cut it down without saying so).
public enum WBudgetRule {
    public static func rule(maxCents: Int) -> WNumberRule {
        WNumberRule(minimum: 0, maximum: Double(maxCents) / 100, allowsEmpty: true, fractionDigits: 2)
    }

    /// What the field shows for the stored budget; nil shows the empty field ("None").
    public static func dollars(cents: Int?) -> Double? {
        guard let cents, cents > 0 else { return nil }
        return Double(cents) / 100
    }

    /// The dollars to hand to the app's own `budgetCents(dollars:)`: nil for a blank entry.
    public static func dollars(for commit: WNumberCommit) -> Double? {
        switch commit {
        case .clear: return nil
        case .set(let value): return value
        }
    }
}

// MARK: Daily caps

/// The daily caps, in cents. Blank means "use the built-in default" and is stored as nothing. 0 is a real answer: it
/// turns that cap off. The two are different settings and stay different.
public enum WCapRule {
    public static let maximum = 1_000_000_000.0
    public static let rule = WNumberRule(minimum: 0, maximum: maximum, allowsEmpty: true, fractionDigits: 0)

    public static func value(cents: Int?) -> Double? { cents.map(Double.init) }

    public static func cents(for commit: WNumberCommit) -> Int? {
        switch commit {
        case .clear: return nil
        case .set(let value):
            guard value.isFinite, value >= 0, value <= maximum else { return nil }
            return Int(exactly: value.rounded())
        }
    }

    public enum Meaning: Equatable, Sendable { case builtInDefault, off, limit(Int) }

    public static func meaning(of cents: Int?) -> Meaning {
        guard let cents else { return .builtInDefault }
        return cents == 0 ? .off : .limit(cents)
    }
}

// MARK: Model downloads

/// A model download as the row shows it. The classic Transcription card shows a progress line while downloading and a
/// retry button after a failure, and nothing otherwise. The download has no cancel call, so the row offers none.
public enum WModelDownloadState: Equatable, Sendable {
    case none
    case downloading(fraction: Double?)
    case failed

    public var showsProgress: Bool { if case .downloading = self { return true } else { return false } }
    public var canRetry: Bool { self == .failed }

    /// The bar's fill, kept between 0 and 1; nil is the indeterminate bar.
    public var fraction: Double? {
        guard case .downloading(let value) = self, let value, value.isFinite else { return nil }
        return min(max(value, 0), 1)
    }
}

/// The small local model (a file in the data folder). Present or not; there is nothing to download from Settings.
public enum WLocalModelCard: Equatable, Sendable {
    case notInstalled
    case installed

    public var offersRemove: Bool { self == .installed }
}

// MARK: Estimates (TypeSafe sorting and finding steps)

/// What an estimate-backed switch shows under itself, in the order the classic pane shows it.
public struct WEstimateInput: Equatable, Sendable {
    public struct Estimate: Equatable, Sendable {
        public var indexing: Bool
        public var sources: Int
        public var usd: Double
        public init(indexing: Bool, sources: Int, usd: Double) {
            self.indexing = indexing
            self.sources = sources
            self.usd = usd
        }
    }

    public var awaitingStart: Bool
    public var estimateFailed: Bool
    public var estimate: Estimate?
    public var startFailed: Bool
    public var keyRejected: Bool
    public var overBudget: Bool

    public init(
        awaitingStart: Bool = false, estimateFailed: Bool = false, estimate: Estimate? = nil, startFailed: Bool = false,
        keyRejected: Bool = false, overBudget: Bool = false
    ) {
        self.awaitingStart = awaitingStart
        self.estimateFailed = estimateFailed
        self.estimate = estimate
        self.startFailed = startFailed
        self.keyRejected = keyRejected
        self.overBudget = overBudget
    }
}

public enum WEstimateLine: Equatable, Sendable {
    case estimateFailed
    case stillSorting
    case ready(items: Int, usd: Double)
    case startFailed
    case keyRejected
    case overBudget
}

public enum WEstimateLines {
    public static func lines(_ input: WEstimateInput) -> [WEstimateLine] {
        var out: [WEstimateLine] = []
        if input.awaitingStart {
            if input.estimateFailed {
                out.append(.estimateFailed)
            } else if let e = input.estimate {
                out.append(e.indexing || e.sources == 0 ? .stillSorting : .ready(items: e.sources, usd: e.usd))
            }
        }
        if input.startFailed { out.append(.startFailed) }
        if input.keyRejected { out.append(.keyRejected) }
        if input.overBudget { out.append(.overBudget) }
        return out
    }
}

// MARK: Topics

public enum WTopicEditorRule {
    public enum Problem: Equatable, Sendable { case emptyName }

    public static let kinds = ["customer", "product", "project", "subject"]

    public static func trimmedName(_ text: String) -> String { text.trimmingCharacters(in: .whitespaces) }

    public static func problem(name: String) -> Problem? { trimmedName(name).isEmpty ? .emptyName : nil }

    /// Aliases are typed with commas between them; blanks are dropped.
    public static func aliases(from text: String) -> [String] {
        text.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    public static func aliasesText(_ aliases: [String]) -> String { aliases.joined(separator: ", ") }

    /// One line under a topic's name: its kind, then its aliases.
    public static func detail(kindLabel: String?, aliases: [String]) -> String? {
        let parts = ([kindLabel] .compactMap { $0 } + [aliases.joined(separator: ", ")]).filter { !$0.isEmpty }
        return parts.isEmpty ? nil : parts.joined(separator: " \u{00B7} ")
    }
}

public enum WNamesRule {
    /// The list as it is saved: trimmed, no blanks, no repeats ignoring case.
    public static func cleaned(_ list: [String]) -> [String] {
        var seen = Set<String>()
        return list.map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty && seen.insert($0.lowercased()).inserted }
    }

    /// Suggestions plus the profile name, minus names already listed (ignoring case); at most `limit`.
    public static func chips(existing: [String], suggested: [String], profileName: String, limit: Int = 4) -> [String] {
        var seen = Set(existing.map { $0.lowercased() })
        return (suggested + [profileName])
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty && seen.insert($0.lowercased()).inserted }
            .prefix(limit).map { $0 }
    }
}

// MARK: Usage table

public struct WUsageRowModel: Identifiable, Equatable, Sendable {
    public let id: String
    public var time: String
    public var task: String
    public var model: String
    /// "120 in, 40 out"; empty when the call did not record tokens.
    public var tokens: String
    public var cost: String

    public init(id: String, time: String, task: String, model: String, tokens: String, cost: String) {
        self.id = id
        self.time = time
        self.task = task
        self.model = model
        self.tokens = tokens
        self.cost = cost
    }

    /// What VoiceOver reads for the row, in the column order, without the empty columns.
    public var spoken: String { [time, task, model, tokens, cost].filter { !$0.isEmpty }.joined(separator: ", ") }
}

public struct WUsageDayModel: Identifiable, Equatable, Sendable {
    public var id: String { day }
    public var day: String
    public var rows: [WUsageRowModel]
    public init(day: String, rows: [WUsageRowModel]) {
        self.day = day
        self.rows = rows
    }
}
