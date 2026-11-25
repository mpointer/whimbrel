import Foundation

// Pure decisions behind the Memory & Privacy section of the redesigned Settings window. The views call these
// functions, so the tests cover what the views do. Nothing here knows about SwiftUI, the app or the clock.

// MARK: Retention

/// What is stored for one retention row: a number of days and whether the class is kept forever. The two are separate
/// settings, as in the app, so a stored number survives "Keep forever" being on.
public struct WRetentionStored: Equatable, Sendable {
    public var days: Int
    public var keepForever: Bool
    public init(days: Int, keepForever: Bool) {
        self.days = days
        self.keepForever = keepForever
    }
}

/// The class of data a retention row governs. Used for words and for the counts a confirm may quote.
public enum WRetentionKind: String, CaseIterable, Sendable {
    case screenText, meetings, conversations, deleteAudio
}

/// What a row does to data once its period has passed. Deleting is one thing; re-encoding audio at lower quality and
/// making images smaller both replace the original, so none of the three can be undone.
public enum WRetentionEffect: Sendable, Equatable {
    case deletes
    case recompressesAudio
    case shrinksImages
}

/// When a retention change is saved and when the person is asked first (decision 4 of the Settings screen).
///
/// - Turning "Keep forever" off saves nothing: the row shows the last stored number and waits for a period to be committed.
/// - Committing a period shorter than the stored one, or any period when the stored setting is "Keep forever", asks first:
///   the next retention run would delete data that is kept today.
/// - A longer period keeps more, so it is saved without asking. Turning "Keep forever" on keeps everything, so it is saved at once.
/// - A number outside the allowed range, or no number, saves nothing and says why.
public enum WRetentionDecision {
    public static let range = 1...3650

    public enum Problem: Equatable, Sendable {
        case empty
        case belowMinimum(Int)
        case aboveMaximum(Int)
    }

    /// What the current setting would be replaced by a shorter period.
    public enum Replaces: Equatable, Sendable {
        case forever
        case days(Int)
    }

    public enum Outcome: Equatable, Sendable {
        case nothingToSave
        case save(days: Int)
        case askFirst(days: Int, replacing: Replaces)
        case refuse(Problem)
    }

    public static func validate(_ days: Int?) -> Problem? {
        guard let days else { return .empty }
        if days < range.lowerBound { return .belowMinimum(range.lowerBound) }
        if days > range.upperBound { return .aboveMaximum(range.upperBound) }
        return nil
    }

    /// The decision when the person commits `proposed` days.
    public static func commit(stored: WRetentionStored, proposed: Int?) -> Outcome {
        if let problem = validate(proposed) { return .refuse(problem) }
        guard let days = proposed else { return .refuse(.empty) }
        if stored.keepForever { return .askFirst(days: days, replacing: .forever) }
        if days == stored.days { return .nothingToSave }
        if days < stored.days { return .askFirst(days: days, replacing: .days(stored.days)) }
        return .save(days: days)
    }

    /// What the companion checkbox ("Keep forever", "Keep full quality") does when it is changed.
    public enum CompanionOutcome: Equatable, Sendable {
        /// Keeping more destroys nothing: saved at once.
        case saveKeepForever
        /// Turning it off saves nothing: the row shows this number (the last stored one, kept in range) and waits for
        /// a period to be committed, which asks first.
        case choosePeriod(showing: Int)
        case nothingToSave
    }

    public static func companion(stored: WRetentionStored, keepForever: Bool) -> CompanionOutcome {
        if keepForever { return stored.keepForever ? .nothingToSave : .saveKeepForever }
        return stored.keepForever ? .choosePeriod(showing: numberToShow(stored: stored)) : .nothingToSave
    }

    /// The number a row shows when "Keep forever" is turned off: the last stored number, kept inside the range.
    /// Nothing is written.
    public static func numberToShow(stored: WRetentionStored) -> Int {
        min(max(stored.days, range.lowerBound), range.upperBound)
    }

    /// A stored number outside the range (an old or synced setting) is shown clamped. This says so, so the row can too.
    public static func storedProblem(_ stored: WRetentionStored) -> Problem? { validate(stored.days) }
}

/// The classes that have a limit, for the "Apply Retention Now" question.
public enum WRetentionApplyPlan {
    public struct Limit: Equatable, Sendable {
        public var kind: WRetentionKind
        public var days: Int
        public init(kind: WRetentionKind, days: Int) {
            self.kind = kind
            self.days = days
        }
    }

    /// True when at least one class has a limit. With none, "Apply Retention Now" has nothing to delete and offers no confirm.
    public static func hasLimit(_ rows: [WRetentionStored]) -> Bool { rows.contains { !$0.keepForever } }

    /// Only the classes that are not kept forever, in the order the rows show them.
    public static func limits(_ rows: [(kind: WRetentionKind, stored: WRetentionStored)]) -> [Limit] {
        rows.filter { !$0.stored.keepForever }.map { Limit(kind: $0.kind, days: $0.stored.days) }
    }
}

// MARK: Memory row

/// Which controls the Memory row offers for a state. The same pause choices the menu bar offers; the same On and Off.
public struct WMemoryControls: Equatable, Sendable {
    public enum Switch: Equatable, Sendable { case turnOff, turnOn }
    /// The Pause menu (its choices include Resume now while paused).
    public var showsPause: Bool
    /// The On or Off button; nil while a timed pause is running (the menu's own "Turn off memory" resumes then, so
    /// Settings offers Resume now instead of a button whose name would not match what it does).
    public var toggle: Switch?
    /// Both controls are off while the library is closed (no store): nothing can be saved.
    public var isEnabled: Bool

    public static func controls(for state: WMemoryState, storeOpen: Bool) -> WMemoryControls {
        switch state {
        case .on, .onNotCapturing: return WMemoryControls(showsPause: true, toggle: .turnOff, isEnabled: storeOpen)
        case .pausedUntil: return WMemoryControls(showsPause: true, toggle: nil, isEnabled: storeOpen)
        case .off: return WMemoryControls(showsPause: false, toggle: .turnOn, isEnabled: storeOpen)
        }
    }

    public var isPausedChoices: Bool { toggle == nil && showsPause }
}

// MARK: Lists

public enum WExcludedAppsFilter {
    public enum Result: Equatable, Sendable {
        case apps([WExcludedApp])
        /// Nothing is running that could be listed.
        case noneRunning
        /// The search matched nothing.
        case noMatches
    }

    /// The same filter the classic pane uses: the name contains the search text, ignoring case.
    public static func filter(_ apps: [WExcludedApp], search: String) -> Result {
        let shown = search.isEmpty ? apps : apps.filter { $0.name.localizedCaseInsensitiveContains(search) }
        if shown.isEmpty { return apps.isEmpty ? .noneRunning : .noMatches }
        return .apps(shown)
    }
}

public struct WExcludedApp: Identifiable, Equatable, Sendable {
    /// The bundle id.
    public let id: String
    public var name: String
    public var excluded: Bool
    public init(id: String, name: String, excluded: Bool = false) {
        self.id = id
        self.name = name
        self.excluded = excluded
    }
}
