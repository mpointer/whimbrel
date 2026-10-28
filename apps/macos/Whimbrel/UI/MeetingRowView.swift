import SwiftUI

/// One meeting in a list: wrapping title, a quiet second line, the start time on the right.
/// Shared by the Meetings list and Today.
struct MeetingRowView: View {
    let title: String
    let subtitle: String
    let time: String
    let action: (() -> Void)?
    /// A quiet trailing verb ("Open") after the time, for rows that are a button.
    let actionLabel: String?

    init(title: String, subtitle: String, time: String, actionLabel: String? = nil, action: (() -> Void)? = nil) {
        self.title = title
        self.subtitle = subtitle
        self.time = time
        self.actionLabel = actionLabel
        self.action = action
    }

    var body: some View {
        if let action {
            Button(action: action) { content }.buttonStyle(.plain)
        } else {
            content
        }
    }

    private var content: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.md) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                if !subtitle.isEmpty {
                    Text(subtitle).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary).lineLimit(1)
                }
            }
            Spacer(minLength: Spacing.md)
            Text(time)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
            if let actionLabel {
                Text(actionLabel)
                    .font(.whimbrelBody(.caption).weight(.medium))
                    .foregroundStyle(Color.whimbrelAccentPrimary)
            }
        }
        .padding(.horizontal, Spacing.md)
        .padding(.vertical, Spacing.sm)
        .contentShape(Rectangle())
    }
}

enum MeetingRowText {
    /// Up to two attendees ("A, B +3"), then the count of other recorded copies.
    static func subtitle(for meeting: CoreMeeting) -> String {
        var parts: [String] = []
        if let outcome = meeting.effectiveAwayOutcome { parts.append(AwayOutcomeLabel.text(outcome, at: meeting.awayOutcomeAt)) }
        if !meeting.attendees.isEmpty { parts.append(attendeeSummary(meeting.attendees)) }
        if let copies = meeting.alsoRecorded, !copies.isEmpty {
            parts.append("+\(copies.count) \(copies.count == 1 ? "copy" : "copies")")
        }
        return parts.joined(separator: " · ")
    }

    /// "A, B +3": the first two names, then how many more.
    static func attendeeSummary(_ names: [String]) -> String { ReadLogic.attendeeSummary(names) }

    static func time(_ unixMs: Int64) -> String {
        time(Date(timeIntervalSince1970: Double(unixMs) / 1000))
    }

    static func time(_ date: Date) -> String { ReadLogic.timeText(date) }
}
