import SwiftUI

/// The prep brief for the next upcoming meeting, shown in Today's upcoming-meeting teaser
/// (docs/superpowers/specs/2026-10-06-meeting-prep-design.md). Every claim carries the ids of
/// the sources it cites; each source is a chip that opens the earlier meeting or the email
/// thread. A brief that cannot be decoded shows its title and says so instead of failing.
struct MeetingPrepView: View {
    let prep: CoreMeetingPrep
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        WhimbrelSurface {
            VStack(alignment: .leading, spacing: Spacing.lg) {
                Text(prep.title)
                    .font(.whimbrelDisplay(.title))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .accessibilityIdentifier("meeting-prep-title")
                if let brief = prep.brief {
                    Text(brief.summary.text)
                        .font(.whimbrelBody(.body))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                    chips(for: brief.summary.sources, in: brief)
                    if !brief.attendees.isEmpty {
                        Divider().overlay(Color.whimbrelBorder)
                        VStack(alignment: .leading, spacing: Spacing.md) {
                            WhimbrelSectionLabel(text: "People")
                            ForEach(Array(brief.attendees.enumerated()), id: \.offset) { _, note in
                                attendeeRow(note, in: brief)
                            }
                        }
                    }
                    claimSection("Open items", claims: brief.openItems, in: brief)
                    claimSection("Open questions", claims: brief.openQuestions, in: brief)
                } else {
                    Text("The brief could not be shown.")
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
        }
    }

    private func attendeeRow(_ note: CoreMeetingBrief.AttendeeNote, in brief: CoreMeetingBrief) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text(note.who)
                .font(.whimbrelBody(.headline))
                .foregroundStyle(Color.whimbrelTextPrimary)
            if let last = note.lastInteractionUnixMs {
                Text("Last in touch \(Self.relative(last))")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
            Text(note.note)
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextPrimary)
            chips(for: note.sources, in: brief)
        }
    }

    @ViewBuilder
    private func claimSection(_ label: String, claims: [CoreMeetingBrief.Claim], in brief: CoreMeetingBrief) -> some View {
        if !claims.isEmpty {
            Divider().overlay(Color.whimbrelBorder)
            VStack(alignment: .leading, spacing: Spacing.sm) {
                WhimbrelSectionLabel(text: label)
                ForEach(Array(claims.enumerated()), id: \.offset) { _, claim in
                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        Text("· \(claim.text)")
                            .font(.whimbrelBody(.body))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                        chips(for: claim.sources, in: brief)
                    }
                }
            }
        }
    }

    /// One chip per cited source: a meeting opens its detail window, an email thread opens
    /// its thread window, and a source with neither is a plain label.
    @ViewBuilder
    private func chips(for ids: [String], in brief: CoreMeetingBrief) -> some View {
        let cited = ids.compactMap { id in brief.sources.first(where: { $0.id == id }) }
        if !cited.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: Spacing.sm) {
                    ForEach(cited) { source in
                        WhimbrelSourceChip(
                            systemImage: source.kind == "email" ? "envelope" : "person.2",
                            text: source.title,
                            onTap: tapAction(for: source)
                        )
                    }
                }
            }
        }
    }

    private func tapAction(for source: CoreMeetingBrief.Source) -> (() -> Void)? {
        if let meetingId = source.meetingId { return { openWindow(id: "meeting-detail", value: meetingId) } }
        if let threadId = source.threadId { return { openWindow(id: "thread-detail", value: threadId) } }
        return nil
    }

    private static func relative(_ unixMs: Int64) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        return formatter.localizedString(for: Date(timeIntervalSince1970: Double(unixMs) / 1000), relativeTo: Date())
    }
}
