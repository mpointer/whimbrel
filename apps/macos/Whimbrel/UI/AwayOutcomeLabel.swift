import Foundation

/// The one line shown for how an Away Mode meeting went (Meetings list and meeting page).
enum AwayOutcomeLabel {
    static func text(_ outcome: AwayOutcome, at: Date?, locale: Locale = .current) -> String {
        switch outcome {
        case .attended: String(localized: "Attended by Whimbrel")
        case .chatNotPosted: String(localized: "Attended by Whimbrel · chat message not posted")
        case .removed: at.map { String(localized: "Removed at \(time($0, locale))") } ?? String(localized: "Removed from the meeting")
        case .waitingRoom: String(localized: "Couldn't get in: not admitted from the waiting room")
        case .passcodeRequired: String(localized: "Couldn't get in: needed a passcode")
        case .signInRequired: String(localized: "Couldn't get in: needed a sign-in")
        case .notMuted: String(localized: "Left: couldn't confirm mic and camera were off")
        case .screenLocked: String(localized: "Couldn't get in: this Mac was locked")
        case .appFailed: String(localized: "Couldn't get in: Zoom didn't open")
        case .missedOverlap: String(localized: "Missed: another meeting was on at the same time")
        }
    }

    private static func time(_ date: Date, _ locale: Locale) -> String {
        date.formatted(Date.FormatStyle(date: .omitted, time: .shortened, locale: locale))
    }
}

extension CoreMeeting {
    /// An away meeting with no stored outcome was attended; non-away meetings have none.
    var effectiveAwayOutcome: AwayOutcome? {
        trigger == .away ? (awayOutcome ?? .attended) : nil
    }
}
