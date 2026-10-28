import SwiftUI

/// The Before-state main canvas: title/time/attendees, a short "What matters now"
/// paragraph (only from real data — see `MeetingBriefModel`, which carries no
/// synthesized text of its own), related threads as non-interactive-until-supported
/// source chips, and a single dominant Start action wired to the EXISTING capture
/// entry points (`AppState.startCapture(for:)`/`startManualMeeting()`) — this view
/// never starts a recording itself.
struct MeetingBriefView: View {
    let model: MeetingBriefModel
    let relatedThreads: [CoreThread]
    let suggestedQuestions: [String]
    let onStart: () -> Void

    private var meeting: CoreMeeting { model.meeting }

    var body: some View {
        ScrollView {
            WhimbrelSurface {
                VStack(alignment: .leading, spacing: Spacing.lg) {
                    header
                    Divider().overlay(Color.whimbrelBorder)
                    attendeesRow
                    if !relatedThreads.isEmpty {
                        Divider().overlay(Color.whimbrelBorder)
                        relatedThreadsSection
                    }
                    if !suggestedQuestions.isEmpty {
                        Divider().overlay(Color.whimbrelBorder)
                        suggestedQuestionsSection
                    }
                    Divider().overlay(Color.whimbrelBorder)
                    startButton
                }
            }
            .padding(Spacing.xl)
        }
        .background(Color.whimbrelBackgroundPage)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text(meeting.title)
                .font(.whimbrelDisplay(.title))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .accessibilityIdentifier("meeting-brief-title")
            HStack(spacing: Spacing.sm) {
                WhimbrelMetadataChip(systemImage: "clock", text: formattedStartTime)
                WhimbrelMetadataChip(systemImage: meeting.trigger == .calendar ? "calendar" : "person.crop.circle", text: meeting.trigger.rawValue.capitalized)
            }
        }
    }

    private var attendeesRow: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: "Attendees")
            if meeting.attendees.isEmpty {
                Text("No attendees on record.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                Text(meeting.attendees.joined(separator: ", "))
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextPrimary)
            }
        }
    }

    private var relatedThreadsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: "Open loops")
            FlowChips {
                ForEach(relatedThreads) { thread in
                    WhimbrelSourceChip(systemImage: "envelope", text: thread.subject ?? "Untitled thread", onTap: nil)
                }
            }
        }
    }

    private var suggestedQuestionsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: "Suggested questions")
            ForEach(suggestedQuestions, id: \.self) { question in
                Text("· \(question)")
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextPrimary)
            }
        }
    }

    private var startButton: some View {
        Button("Start note", action: onStart)
            .buttonStyle(.whimbrelPrimary)
            .accessibilityIdentifier("meeting-brief-start-button")
    }

    private var formattedStartTime: String {
        let date = Date(timeIntervalSince1970: Double(meeting.startUnixMs) / 1000)
        let formatter = DateFormatter()
        formatter.dateStyle = .none
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }
}

/// A simple left-aligned wrapping row for chip collections — SwiftUI on macOS has no
/// built-in flow layout prior to a `Layout` protocol conformance; kept minimal (no
/// wrapping math beyond what `HStack` inside a `ScrollView(.horizontal)` needs) since
/// the chip counts here are always small (a handful of threads/sources per meeting).
private struct FlowChips<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: Spacing.sm) { content }
        }
    }
}
