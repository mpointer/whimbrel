import SwiftUI

/// A topic's name in a capsule; a tap shows its page on the Topics tab. Pass `onTap` to handle the
/// tap yourself (a meeting card routes every chip through one callback); nil keeps the default.
struct TopicChip: View {
    let topic: TopicRef
    var onTap: (() -> Void)? = nil

    var body: some View {
        if let onTap {
            TopicChipButton(topic: topic, action: onTap)
        } else {
            AppStateTopicChip(topic: topic)
        }
    }
}

private struct AppStateTopicChip: View {
    @EnvironmentObject var appState: AppState
    let topic: TopicRef

    var body: some View {
        TopicChipButton(topic: topic) { appState.openTopic(topic.topicId) }
    }
}

private struct TopicChipButton: View {
    let topic: TopicRef
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(topic.name, systemImage: "tag")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
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
/// Pass `onTap` to handle the tap yourself; nil keeps the default.
struct PrepReadyChip: View {
    /// The stored brief's key (`CoreMeetingPrep.eventId`).
    let prepKey: String
    var onTap: (() -> Void)? = nil

    var body: some View {
        if let onTap {
            PrepReadyChipButton(action: onTap)
        } else {
            AppStatePrepReadyChip(prepKey: prepKey)
        }
    }
}

private struct AppStatePrepReadyChip: View {
    @EnvironmentObject var appState: AppState
    let prepKey: String

    var body: some View {
        PrepReadyChipButton { appState.showPrepBrief(key: prepKey) }
    }
}

private struct PrepReadyChipButton: View {
    let action: () -> Void

    var body: some View {
        WhimbrelSourceChip(systemImage: "doc.text", text: String(localized: "Prep ready"), onTap: action)
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
