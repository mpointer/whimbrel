import SwiftUI

enum MeetingRowText {
    /// Up to two attendees ("A, B +3"), then the count of other recorded copies.
    static func subtitle(for meeting: CoreMeeting) -> String {
        var parts: [String] = []
        if let outcome = meeting.effectiveAwayOutcome { parts.append(AwayOutcomeLabel.text(outcome, at: meeting.awayOutcomeAt)) }
        if !meeting.attendees.isEmpty { parts.append(attendeeSummary(meeting.attendees)) }
        if let copies = meeting.alsoRecorded, !copies.isEmpty {
            parts.append(String(localized: "+\(copies.count) copies"))
        }
        return parts.joined(separator: " · ")
    }

    /// The Away Mode outcome, then the count of other recorded copies; nil when there is neither.
    static func statusLine(for meeting: CoreMeeting) -> String? {
        var parts: [String] = []
        if let outcome = meeting.effectiveAwayOutcome { parts.append(AwayOutcomeLabel.text(outcome, at: meeting.awayOutcomeAt)) }
        if let copies = meeting.alsoRecorded, !copies.isEmpty {
            parts.append(String(localized: "+\(copies.count) copies"))
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// "A, B +3": the first two names, then how many more.
    static func attendeeSummary(_ names: [String]) -> String { ReadLogic.attendeeSummary(names) }

    static func time(_ unixMs: Int64) -> String {
        time(Date(timeIntervalSince1970: Double(unixMs) / 1000))
    }

    static func time(_ date: Date) -> String { ReadLogic.timeText(date) }
}
