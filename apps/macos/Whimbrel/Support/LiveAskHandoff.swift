import Foundation

/// A question typed in the in-meeting window, held until the live screen's ask bar takes it. Take-once: the bar that
/// takes it is the only one that gets it. If nothing takes it within `deadline`, the sender gets it back and keeps the text.
struct LiveAskHandoff: Equatable {
    static let deadline: TimeInterval = 5

    private(set) var held: String?
    private(set) var heldAt: Date?

    var isHolding: Bool { held != nil }

    /// A new question replaces one that is still waiting.
    mutating func hold(_ text: String, now: Date) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        held = trimmed
        heldAt = now
    }

    /// The ask bar takes the question. A second take gets nothing.
    mutating func take() -> String? {
        guard held != nil else { return nil }   // no write, so nothing observing it is woken
        defer { held = nil; heldAt = nil }
        return held
    }

    func isExpired(now: Date) -> Bool {
        guard let heldAt, held != nil else { return false }
        return now.timeIntervalSince(heldAt) >= Self.deadline
    }

    /// Gives up: the question was not delivered. Returns whether one was waiting.
    @discardableResult
    mutating func giveUp() -> Bool {
        guard held != nil else { return false }
        held = nil
        heldAt = nil
        return true
    }
}
