import SwiftUI

/// A topic's name in a capsule; a tap shows its page on the Topics tab.
struct TopicChip: View {
    @EnvironmentObject var appState: AppState
    let topic: TopicRef

    var body: some View {
        Button { appState.openTopic(topic.topicId) } label: {
            Label(topic.name, systemImage: "tag")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .lineLimit(1)
                .padding(.horizontal, Spacing.sm)
                .padding(.vertical, 2)
                .background(Capsule().fill(Color.whimbrelAccentPrimary.opacity(0.12)))
                .overlay(Capsule().stroke(Color.whimbrelBorder))
        }
        .buttonStyle(.plain)
        .help("Open \(topic.name)")
    }
}

/// "Prep ready" on a meeting row that has a brief; a tap opens the brief, as Routines' "Open brief" does.
struct PrepReadyChip: View {
    @EnvironmentObject var appState: AppState
    /// The stored brief's key (`CoreMeetingPrep.eventId`).
    let prepKey: String

    var body: some View {
        WhimbrelSourceChip(systemImage: "doc.text", text: String(localized: "Prep ready"), onTap: { appState.showPrepBrief(key: prepKey) })
            .help(String(localized: "Open brief"))
    }
}

/// "Open topic page: {name}", shown above search results or an answer whose text names a topic.
struct TopicPageLink: View {
    @EnvironmentObject var appState: AppState
    let topic: TopicRef

    var body: some View {
        Button { appState.openTopic(topic.topicId) } label: {
            Label("Open topic page: \(topic.name)", systemImage: "tag")
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelLink)
        }
        .buttonStyle(.plain)
    }
}
