import SwiftUI

/// The topic pages the computer built: name, the page's lead and the next meeting's time.
struct TopicsView: View {
    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    @StateObject private var model = TopicsModel()

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: WTokens.Space.md) {
                if model.topics.isEmpty {
                    if model.loaded {
                        WStateView(kind: .empty, title: String(localized: "Topics appear here once Whimbrel on your computer has built them."))
                    } else {
                        WStateView(kind: .loading, title: String(localized: "Loading…"))
                    }
                }
                ForEach(model.topics) { topic in
                    NavigationLink(value: TopicRoute(key: topic.topicKey)) { TopicRow(topic: topic) }
                        .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, WTokens.Space.lg)
            .padding(.vertical, WTokens.Space.md)
            .frame(maxWidth: WTokens.Size.contentMax, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle("Topics")
        .navigationDestination(for: TopicRoute.self) { TopicPageView(key: $0.key, sync: sync) }
        .navigationDestination(for: Int64.self) { MeetingDetailView(meetingId: $0, sync: sync, labels: labels) }
        .task(id: sync.readRevision) { await model.reload(sync: sync) }
        .refreshable { await sync.syncNow(manual: true) }
    }
}

/// One topic in the list: a card with its name, its lead and when the next meeting is.
private struct TopicRow: View {
    let topic: PhoneTopicSnapshot

    var body: some View {
        HStack(alignment: .center, spacing: WTokens.Space.sm) {
            VStack(alignment: .leading, spacing: WTokens.Space.xs) {
                Text(topic.name)
                    .wType(WTokens.TypeScale.subtitle)
                    .foregroundStyle(Color.theme(.textPrimary))
                    .multilineTextAlignment(.leading)
                if let lead = topic.lead {
                    Text(TopicText.lead(lead))
                        .wType(WTokens.TypeScale.secondary)
                        .foregroundStyle(Color.theme(.textSecondary))
                        .multilineTextAlignment(.leading)
                        .lineLimit(3)
                }
                if let next = topic.upcomingMeeting() {
                    Label(TopicText.dayTime(ms: next.startUnixMs), systemImage: "calendar")
                        .wType(WTokens.TypeScale.time)
                        .foregroundStyle(Color.theme(.accentPrimary))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
                .foregroundStyle(Color.theme(.textSecondary)).accessibilityHidden(true)
        }
        .padding(WTokens.Space.lg)
        .frame(maxWidth: .infinity, minHeight: WTokens.Size.target, alignment: .leading)
        .wCard()
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}
