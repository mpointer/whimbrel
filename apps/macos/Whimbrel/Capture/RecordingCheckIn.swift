import Foundation

/// Pure decision logic for the "Still recording?" check-in. The caller polls with a clock
/// and acts on the returned `Action`; nothing here touches UI or audio.
struct RecordingCheckIn: Equatable {
    static let defaultCountdown: TimeInterval = 5 * 60
    static let defaultReaskAfter: TimeInterval = 60 * 60
    static let maxDurationSuppression: TimeInterval = 65 * 60

    let startedAt: Date
    /// nil = Off.
    let firstPromptAfter: TimeInterval?
    let countdown: TimeInterval
    let reaskAfter: TimeInterval
    private(set) var nextPromptAt: Date?
    private(set) var promptShownAt: Date?
    private(set) var lastKeepRecordingAt: Date?

    init(startedAt: Date, firstPromptAfter: TimeInterval?,
         countdown: TimeInterval = Self.defaultCountdown, reaskAfter: TimeInterval = Self.defaultReaskAfter) {
        self.startedAt = startedAt
        self.firstPromptAfter = firstPromptAfter
        self.countdown = countdown
        self.reaskAfter = reaskAfter
        nextPromptAt = firstPromptAfter.map { startedAt.addingTimeInterval($0) }
    }

    enum Action: Equatable { case none, showPrompt(deadline: Date), stop }

    mutating func poll(now: Date) -> Action {
        if let shown = promptShownAt {
            guard now >= shown.addingTimeInterval(countdown) else { return .none }
            promptShownAt = nil
            nextPromptAt = nil   // stop fires once; nothing re-arms
            return .stop
        }
        guard let next = nextPromptAt, now >= next else { return .none }
        promptShownAt = now
        nextPromptAt = nil
        return .showPrompt(deadline: now.addingTimeInterval(countdown))
    }

    mutating func keepRecording(now: Date) {
        promptShownAt = nil
        lastKeepRecordingAt = now
        nextPromptAt = now.addingTimeInterval(reaskAfter)
    }

    /// True while a prompt is due or showing (its own countdown decides) and for 65 min
    /// after "Keep recording" (the hour to the next prompt plus that prompt's countdown).
    func suppressesMaxDuration(now: Date) -> Bool {
        if promptShownAt != nil { return true }
        if let next = nextPromptAt, now >= next { return true }
        guard let last = lastKeepRecordingAt else { return false }
        return now.timeIntervalSince(last) < Self.maxDurationSuppression
    }
}
