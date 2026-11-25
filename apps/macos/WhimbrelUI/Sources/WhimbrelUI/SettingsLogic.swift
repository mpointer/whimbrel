import Foundation

// Pure logic behind the settings parts. The views call these functions, so the tests cover what the
// views do. Nothing here knows about SwiftUI or about the app.

// MARK: List keys

/// A key that moves the selection in a list.
public enum WListKey: Sendable, Equatable {
    case next, previous, first, last
}

/// Selection movement for `WSettingsNav`, `WSettingsSearchResults` and the segmented control.
///
/// Decision: movement does not wrap. Home and End exist, and a list whose ends are clear is easier to
/// learn by ear than one that jumps from the last item to the first.
public enum WListNavigation {
    /// The index after `key`, or nil when the list is empty. With nothing selected, Down and Home pick
    /// the first item and Up and End pick the last. At an end the index stays where it is.
    public static func index(after current: Int?, count: Int, key: WListKey) -> Int? {
        guard count > 0 else { return nil }
        let last = count - 1
        switch key {
        case .first: return 0
        case .last: return last
        case .next:
            guard let current else { return 0 }
            return min(max(current, 0) + 1, last)
        case .previous:
            guard let current else { return last }
            return max(min(current, last) - 1, 0)
        }
    }

    /// Type-ahead: the first title, searching forward from the item after `current` and wrapping once,
    /// whose start matches `typed` (ignoring case and accents). Repeating one letter cycles through the
    /// titles that start with it.
    public static func typeAhead(_ typed: String, titles: [String], from current: Int?) -> Int? {
        let needle = typed.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: .current)
        guard !needle.isEmpty, !titles.isEmpty else { return nil }
        let repeated = needle.count > 1 && Set(needle).count == 1
        let probe = repeated ? String(needle.prefix(1)) : needle
        let start = (current ?? -1) + 1
        for offset in 0..<titles.count {
            let index = (start + offset) % titles.count
            let title = titles[index].folding(options: [.caseInsensitive, .diacriticInsensitive], locale: .current)
            if title.hasPrefix(probe) { return index }
        }
        return nil
    }
}

// MARK: Picker style

public enum WPickerStyle: Sendable, Equatable { case menu, segmented }

extension WPickerStyle {
    /// The segmented style holds two to four choices. Anything else falls back to a menu, so a list that
    /// grows (the default provider) never overflows the row.
    public static let segmentedLimit = 4

    public static func effective(_ requested: WPickerStyle, optionCount: Int) -> WPickerStyle {
        requested == .segmented && optionCount >= 2 && optionCount <= segmentedLimit ? .segmented : .menu
    }
}

// MARK: Numbers

/// What a number field accepts.
public struct WNumberRule: Sendable, Equatable {
    public var minimum: Double
    public var maximum: Double
    /// Blank is a real answer ("None", "use the default"). When false, blank is an error and nothing is written.
    public var allowsEmpty: Bool
    /// 0 for whole numbers, 2 for dollars.
    public var fractionDigits: Int

    public init(minimum: Double = 0, maximum: Double = .greatestFiniteMagnitude, allowsEmpty: Bool = false, fractionDigits: Int = 0) {
        self.minimum = minimum
        self.maximum = maximum
        self.allowsEmpty = allowsEmpty
        self.fractionDigits = fractionDigits
    }

    /// A stepper's days or minutes: whole, at least `minimum`.
    public static func whole(_ range: ClosedRange<Int>) -> WNumberRule {
        WNumberRule(minimum: Double(range.lowerBound), maximum: Double(range.upperBound))
    }
}

public enum WNumberProblem: Sendable, Equatable {
    case notANumber
    case emptyNotAllowed
    case belowMinimum(Double)
    case aboveMaximum(Double)
    case tooManyDecimals(Int)
    /// A grouping separator, or a separator that could be read two ways. Never guessed at.
    case separator
}

public enum WNumberParse: Sendable, Equatable {
    case value(Double)
    case empty
    case invalid(WNumberProblem)
}

/// What a number field tells the app after an edit. Only valid input becomes one of these.
public enum WNumberCommit: Sendable, Equatable {
    case set(Double)
    /// Blank, where the rule allows it.
    case clear
}

public enum WNumberInput {
    /// Reads typed text in `locale`.
    ///
    /// Rules, so a figure is never read wrong without a message:
    /// - A leading currency sign ("$", or the locale's own) is ignored.
    /// - The locale's decimal separator is accepted. A "." is accepted as a decimal point too, but only when it
    ///   cannot be a grouping separator in that locale: in a locale that groups with ".", three digits after it
    ///   ("1.000") is refused as ambiguous.
    /// - Any other grouping separator (a "," in en, a space in fr), and any input that holds both a "." and a ","
    ///   ("1,000.50", "1.000,50"), is refused with `.separator`. The text is never guessed at.
    public static func parse(_ text: String, rule: WNumberRule, locale givenLocale: Locale) -> WNumberParse {
        let locale = latinDigits(givenLocale)
        var cleaned = latinized(text).trimmingCharacters(in: .whitespacesAndNewlines)
        for symbol in Set(["$", locale.currencySymbol].compactMap { $0 }.filter { !$0.isEmpty }) where cleaned.hasPrefix(symbol) {
            cleaned.removeFirst(symbol.count)
            break
        }
        cleaned = cleaned.trimmingCharacters(in: .whitespacesAndNewlines)
        if cleaned.isEmpty { return rule.allowsEmpty ? .empty : .invalid(.emptyNotAllowed) }

        var sign = ""
        if let first = cleaned.first, first == "-" || first == "+" {
            sign = first == "-" ? "-" : ""
            cleaned.removeFirst()
        }
        let decimal = locale.decimalSeparator ?? "."
        let grouping = locale.groupingSeparator ?? ","
        let allowed = Set("0123456789.,")
        // Anything that is not a digit, a separator or a space is not a number at all.
        for ch in cleaned where !allowed.contains(ch) && !String(ch).unicodeScalars.allSatisfy({ $0.properties.isWhitespace }) && String(ch) != grouping && String(ch) != decimal {
            return .invalid(.notANumber)
        }
        // A space inside the number or the locale's grouping mark ("1 000", "1'000") is a grouping separator.
        let inner = cleaned.unicodeScalars.contains(where: { $0.properties.isWhitespace }) || (grouping != "." && grouping != "," && cleaned.contains(grouping))
        if inner { return .invalid(.separator) }

        let dots = cleaned.filter { $0 == "." }.count
        let commas = cleaned.filter { $0 == "," }.count
        var normalized = cleaned
        if dots + commas > 0 {
            // Both kinds, or the same kind twice, is grouping.
            if (dots > 0 && commas > 0) || dots > 1 || commas > 1 { return .invalid(.separator) }
            let mark: Character = dots == 1 ? "." : ","
            let after = cleaned.split(separator: mark, omittingEmptySubsequences: false).last?.count ?? 0
            if mark == "." {
                // A dot is a decimal point unless it is this locale's grouping mark followed by exactly three digits.
                if grouping == "." && decimal != "." && after == 3 { return .invalid(.separator) }
            } else if decimal != "," {
                // A comma the locale does not use as a decimal separator is its grouping mark.
                return .invalid(.separator)
            }
            normalized = cleaned.replacingOccurrences(of: String(mark), with: ".")
        }
        // Digits, then optionally one point and digits.
        let parts = normalized.split(separator: ".", omittingEmptySubsequences: false)
        let digits = Set("0123456789")
        guard parts.count <= 2, parts.allSatisfy({ $0.allSatisfy(digits.contains) }),
              !(parts.count == 2 && parts[1].isEmpty), !(parts[0].isEmpty && parts.count == 1) else { return .invalid(.notANumber) }
        let whole = parts[0].isEmpty ? "0" : String(parts[0])
        guard let number = Double(sign + whole + (parts.count == 2 ? "." + parts[1] : "")), number.isFinite else {
            return .invalid(.notANumber)
        }
        if parts.count == 2 && parts[1].count > rule.fractionDigits { return .invalid(.tooManyDecimals(rule.fractionDigits)) }
        if number < rule.minimum { return .invalid(.belowMinimum(rule.minimum)) }
        if number > rule.maximum { return .invalid(.aboveMaximum(rule.maximum)) }
        return .value(number)
    }

    /// `locale` with Latin digits, whatever its own numbering system: figures are shown and read with 0 to 9 in every language,
    /// so what `text(for:)` writes is what `parse` reads.
    static func latinDigits(_ locale: Locale) -> Locale {
        var components = Locale.Components(locale: locale)
        components.numberingSystem = Locale.NumberingSystem("latn")
        return Locale(components: components)
    }

    /// Typed digits of any script (Arabic-Indic, Persian, Devanagari...) as 0 to 9, and the Arabic decimal separator as ".".
    static func latinized(_ text: String) -> String {
        var out = String.UnicodeScalarView()
        for scalar in text.unicodeScalars {
            if scalar.properties.numericType == .decimal, let n = scalar.properties.numericValue, n >= 0, n <= 9 {
                out.append(Unicode.Scalar(UInt8(48 + Int(n))))
            } else if scalar == "\u{066B}" {
                out.append(".")
            } else {
                out.append(scalar)
            }
        }
        return String(out)
    }

    /// The write the app should do, or nil. Invalid input and a disallowed blank return nil, so a typo or an
    /// emptied field never stores 0 or clears a setting.
    public static func commit(_ text: String, rule: WNumberRule, locale: Locale) -> WNumberCommit? {
        switch parse(text, rule: rule, locale: locale) {
        case .value(let number): return .set(number)
        case .empty: return .clear
        case .invalid: return nil
        }
    }

    /// The text a stored value shows, in `locale`: whole numbers without a decimal point, money with its digits,
    /// never with a grouping separator (the parser would refuse its own output otherwise).
    public static func text(for value: Double?, rule: WNumberRule, locale: Locale) -> String {
        guard let value else { return "" }
        if rule.fractionDigits == 0 { return String(Int(value.rounded())) }
        return value.formatted(.number.locale(latinDigits(locale)).precision(.fractionLength(rule.fractionDigits)).grouping(.never))
    }
}

/// A stepper with an optional companion checkbox ("Keep forever").
public struct WNumberStepper: Sendable, Equatable {
    public var value: Int
    public var range: ClosedRange<Int>
    public var step: Int
    /// The companion checkbox is on. The number is kept and shown, and the stepper is off.
    public var companionOn: Bool

    public init(value: Int, range: ClosedRange<Int>, step: Int = 1, companionOn: Bool = false) {
        self.value = value
        self.range = range
        self.step = max(1, step)
        self.companionOn = companionOn
    }

    /// The value on screen. It never changes because the companion is on: "Keep forever" leaves "30 days" showing.
    public var displayedValue: Int { clamp(value) }
    public var isStepperEnabled: Bool { !companionOn }
    public var canDecrease: Bool { isStepperEnabled && displayedValue > range.lowerBound }
    public var canIncrease: Bool { isStepperEnabled && displayedValue < range.upperBound }

    /// The value after one step up or down, or nil when the stepper is off or at its end (nothing to write).
    public func stepped(_ direction: Int) -> Int? {
        guard isStepperEnabled, direction != 0 else { return nil }
        let next = clamp(displayedValue + (direction > 0 ? step : -step))
        return next == displayedValue ? nil : next
    }

    /// A stored value outside the range (an old setting) is shown clamped. Nothing is written until the person steps.
    public func clamp(_ v: Int) -> Int { min(max(v, range.lowerBound), range.upperBound) }
}

// MARK: Secrets

/// Where a secret row is. The row never receives the secret, only whether one is saved.
public enum WSecretPhase: Sendable, Equatable {
    /// Nothing saved, nothing typed.
    case empty
    /// A value is saved. The row shows a mask and Replace and Remove.
    case saved
    /// A field is open. `replacing` is true when it will overwrite a saved value.
    case editing(replacing: Bool)
}

public enum WSecretMachine {
    /// The mask. A fixed string: its length says nothing about the saved value.
    public static let mask = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}"

    public static func phase(isSaved: Bool, replacing: Bool) -> WSecretPhase {
        if isSaved && !replacing { return .saved }
        return replacing && isSaved ? .editing(replacing: true) : (isSaved ? .saved : .empty)
    }

    /// The phase with a draft: typing into an empty row is editing a new value.
    public static func phase(isSaved: Bool, replacing: Bool, draft: String) -> WSecretPhase {
        let base = phase(isSaved: isSaved, replacing: replacing)
        if base == .empty && !draft.isEmpty { return .editing(replacing: false) }
        return base
    }

    /// Save needs something other than spaces.
    public static func canSave(draft: String) -> Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// What Save hands the app: the draft without surrounding spaces and newlines (a pasted key often has one).
    public static func valueToSave(draft: String) -> String? {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// Remove is offered when something is saved, or when a draft could be thrown away.
    public static func canRemove(isSaved: Bool) -> Bool { isSaved }

    /// What the row shows as its field text once a value is saved: the draft is emptied, the value is not echoed.
    public static func draftAfterSave() -> String { "" }
}

/// The typed value and the replace flag of one secret row, with what happens to them on each button. The value is kept
/// only while it is being typed, and only while a save has not worked: success, Cancel, Remove and the row going away all
/// empty it, and a failed save keeps it so the person can fix the key without pasting it again.
public struct WSecretDraft: Equatable, Sendable {
    public var text = ""
    public var replacing = false

    public init(text: String = "", replacing: Bool = false) {
        self.text = text
        self.replacing = replacing
    }

    /// What Save hands the app, or nil when there is nothing to save.
    public var valueToSave: String? { WSecretMachine.valueToSave(draft: text) }

    /// The app answered Save. Only a saved key empties the field and leaves "replace".
    public mutating func finishSave(succeeded: Bool) {
        guard succeeded else { return }
        text = WSecretMachine.draftAfterSave()
        replacing = false
    }

    public mutating func cancel() { text = ""; replacing = false }
    public mutating func remove() { text = ""; replacing = false }
    /// The row left the screen: nothing typed is kept.
    public mutating func disappear() { text = ""; replacing = false }
}

// MARK: Typed confirmation

public enum WTypedConfirmation {
    /// The confirm button is on only when the typed word matches exactly, character for character, with no trimming:
    /// "delete" and " DELETE " do not confirm "DELETE", so the person has to mean it.
    public static func matches(typed: String, word: String) -> Bool {
        !word.isEmpty && typed == word
    }
}

// MARK: List add

public enum WListAddProblem: Sendable, Equatable {
    case blank
    case duplicate
    case tooLong(Int)
    case full(Int)
}

public enum WListAddResult: Sendable, Equatable {
    case accepted(String)
    case rejected(WListAddProblem)

    public var value: String? { if case .accepted(let v) = self { return v } else { return nil } }
    public var problem: WListAddProblem? { if case .rejected(let p) = self { return p } else { return nil } }
}

public struct WListAddRule: Sendable, Equatable {
    public var rejectsDuplicates: Bool
    public var maxLength: Int?
    public var maxCount: Int?

    public init(rejectsDuplicates: Bool = true, maxLength: Int? = nil, maxCount: Int? = nil) {
        self.rejectsDuplicates = rejectsDuplicates
        self.maxLength = maxLength
        self.maxCount = maxCount
    }

    /// Trims, then checks. A duplicate ignores case and accents ("Acme" and "acme" are one entry).
    public func validate(_ text: String, existing: [String]) -> WListAddResult {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return .rejected(.blank) }
        if let maxCount, existing.count >= maxCount { return .rejected(.full(maxCount)) }
        if let maxLength, trimmed.count > maxLength { return .rejected(.tooLong(maxLength)) }
        if rejectsDuplicates {
            func key(_ s: String) -> String { s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil) }
            let k = key(trimmed)
            if existing.contains(where: { key($0.trimmingCharacters(in: .whitespacesAndNewlines)) == k }) { return .rejected(.duplicate) }
        }
        return .accepted(trimmed)
    }
}

// MARK: Middle truncation

public enum WMiddleTruncation {
    /// Shortens a long value by dropping the middle: "/Users/alex/Library/…/Whimbrel/Audio". The start and the end
    /// are what a person recognises. A value within `limit` is returned as it is. The full value still goes to VoiceOver
    /// and the hover help.
    public static func truncate(_ value: String, limit: Int) -> String {
        let ellipsis = "\u{2026}"
        guard limit >= 3, value.count > limit else { return value }
        let budget = limit - 1
        let tail = budget / 2
        let head = budget - tail
        return String(value.prefix(head)) + ellipsis + String(value.suffix(tail))
    }
}

// MARK: Shortcut keys

public enum WShortcutDisplay {
    /// The key names for the keycaps, in order. Empty means no shortcut is set.
    public static func keys(from combination: String?) -> [String] {
        guard let combination, !combination.trimmingCharacters(in: .whitespaces).isEmpty else { return [] }
        return combination.split(separator: " ").map(String.init)
    }
}

// MARK: Search hit hand-off

public enum WSettingsHandoff {
    /// The row to scroll to now. The target waits until the detail column is on screen: with the section picker in use, the
    /// column is not there while the search results are, so a target set at the moment of the choice has to be picked up
    /// when the column appears, not only when the target changes.
    public static func ready(target: String?, detailMounted: Bool) -> String? { detailMounted ? target : nil }

    /// The target once a scroll to `done` has finished: cleared, so the same result chosen again is a new hand-off, unless
    /// a newer choice replaced it in the meantime.
    public static func afterScroll(target: String?, done: String) -> String? { target == done ? nil : target }

    /// True when a search hit is a row inside a collapsed group (`group` holds that group's anchors), so the group has to
    /// open before the window scrolls to the row.
    public static func opensGroup(_ group: [String], for target: String?) -> Bool {
        guard let target else { return false }
        return group.contains(target)
    }
}

// MARK: Fact edit

public enum WFactEdit {
    /// The text to save when an edit is left open and the person moves on (starts another edit, leaves the section or closes the
    /// window). The classic pane saved on every keystroke, so nothing typed was ever lost; here nothing is lost either. Nil when
    /// there is nothing to save: no edit open, an unchanged text or a blank one (a blank fact is never saved, as in the classic list).
    public static func textToSaveOnLeaving(original: String?, draft: String) -> String? {
        guard let original else { return nil }
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty || draft == original ? nil : draft
    }
}
