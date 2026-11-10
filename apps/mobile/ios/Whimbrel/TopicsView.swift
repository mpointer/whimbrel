import SwiftUI

/// The topic pages the computer built: name, the page's lead and the next meeting's time.
struct TopicsView: View {
    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    @StateObject private var model = TopicsModel()

    var body: some View {
        List {
            ForEach(model.topics) { topic in
                NavigationLink(value: TopicRoute(key: topic.topicKey)) { TopicRow(topic: topic) }
                    .listRowBackground(Color.theme(.backgroundPage))
            }
            if model.topics.isEmpty {
                Text(model.loaded ? String(localized: "Topics appear here once Whimbrel on your computer has built them.") : String(localized: "Loading…"))
                    .foregroundStyle(Color.theme(.textSecondary))
                    .listRowBackground(Color.theme(.backgroundPage))
            }
        }
        .themed()
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle("Topics")
        .navigationDestination(for: TopicRoute.self) { TopicPageView(key: $0.key, sync: sync) }
        .navigationDestination(for: Int64.self) { MeetingDetailView(meetingId: $0, sync: sync, labels: labels) }
        .task(id: sync.readRevision) { await model.reload(sync: sync) }
        .refreshable { await sync.syncNow(manual: true) }
    }
}

/// One topic in the list: its name, then its lead and when the next meeting is.
private struct TopicRow: View {
    let topic: PhoneTopicSnapshot

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(topic.name)
                .font(.body)
                .foregroundStyle(Color.theme(.textPrimary))
                .multilineTextAlignment(.leading)
            if let lead = topic.lead {
                Text(TopicText.lead(lead))
                    .font(.subheadline)
                    .foregroundStyle(Color.theme(.textSecondary))
                    .lineLimit(2)
            }
            if let next = topic.upcomingMeeting() {
                Label(TopicText.dayTime(ms: next.startUnixMs), systemImage: "calendar")
                    .font(.subheadline)
                    .foregroundStyle(Color.theme(.accentPrimary))
            }
        }
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .contentShape(Rectangle())
    }
}
