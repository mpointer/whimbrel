import Foundation

/// The one line shown for how an Away Mode meeting went (Meetings list and meeting page).
enum AwayOutcomeLabel {
    static func text(_ outcome: AwayOutcome, at: Date?) -> String {
        switch outcome {
        case .attended: "Attended by Whimbrel"
        case .chatNotPosted: "Attended by Whimbrel · chat message not posted"
        case .removed: at.map { "Removed at \(time($0))" } ?? "Removed from the meeting"
        case .waitingRoom: "Couldn't get in: not admitted from the waiting room"
        case .passcodeRequired: "Couldn't get in: needed a passcode"
        case .signInRequired: "Couldn't get in: needed a sign-in"
        case .notMuted: "Left: couldn't confirm mic and camera were off"
        case .screenLocked: "Couldn't get in: this Mac was locked"
        case .appFailed: "Couldn't get in: Zoom didn't open"
        case .missedOverlap: "Missed: another meeting was on at the same time"
        }
    }

    private static func time(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "h:mm a"
        return formatter.string(from: date)
    }
}

extension CoreMeeting {
    /// An away meeting with no stored outcome was attended; non-away meetings have none.
    var effectiveAwayOutcome: AwayOutcome? {
        trigger == .away ? (awayOutcome ?? .attended) : nil
    }
}
