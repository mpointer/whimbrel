import SwiftUI

/// Meetings grouped by day, with a device filter and a title search.
struct MeetingsView: View {
    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    /// Pushes a topic page onto this tab's path (a meeting's topic chip).
    let openTopic: (TopicRoute) -> Void
    @StateObject private var model = MeetingsModel()

    var body: some View {
        let sections = MeetingsModel.sections(model.meetings, deviceFilter: model.deviceFilter, query: model.query, calendar: .current)
        ScrollView {
            LazyVStack(alignment: .leading, spacing: WTokens.Space.sm) {
                if model.devices.count > 1 { deviceChips }
                ForEach(sections) { section in
                    Text(dayTitle(section.day))
                        .wType(WTokens.TypeScale.subtitle)
                        .foregroundStyle(Color.theme(.textSecondary))
                        .padding(.top, WTokens.Space.sm)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(section.meetings) { meeting in
                        WSpineStop(time: ReadLogic.timeText(Date(timeIntervalSince1970: Double(meeting.startUnixMs) / 1000)), node: .past) {
                            // The chip sits below the card's link, not inside it, so a tap reaches one of them.
                            VStack(alignment: .leading, spacing: WTokens.Space.xs) {
                                NavigationLink(value: meeting.id) {
                                    WMeetingCard(variant: .past, title: meeting.title, detail: detail(meeting))
                                }
                                .buttonStyle(.plain)
                                if let topic = TopicsModel.chip(for: meeting, in: model.topicChips) {
                                    TopicChipButton(topic: topic) { openTopic(TopicRoute(key: topic.key)) }
                                }
                            }
                        }
                    }
                }
                if sections.isEmpty {
                    WStateView(kind: model.loaded ? .empty : .loading, title: emptyText)
                }
                if model.isCapped {
                    Text("Showing the newest \(MeetingsModel.limit) meetings.")
                        .wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
                        .padding(.top, WTokens.Space.sm)
                }
            }
            .padding(.horizontal, WTokens.Space.lg)
            .padding(.bottom, WTokens.Space.xl)
            .frame(maxWidth: WTokens.Size.contentMax, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .environment(\.wStrings, .app)
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle("Meetings")
        .searchable(text: $model.query, prompt: String(localized: "Search titles"))
        .navigationDestination(for: Int64.self) { MeetingDetailView(meetingId: $0, sync: sync, labels: labels) }
        .navigationDestination(for: TopicRoute.self) { TopicPageView(key: $0.key, sync: sync) }
        .task(id: sync.readRevision) { await model.reload(sync: sync) }
        .refreshable { await sync.syncNow(manual: true) }
    }

    private var emptyText: String {
        if !model.loaded { return String(localized: "Loading…") }
        return model.meetings.isEmpty ? String(localized: "No meetings yet") : String(localized: "No meetings match")
    }

    /// Where a meeting was recorded, how many copies exist, and who was there.
    private func detail(_ meeting: PhoneMeeting) -> String {
        var parts = [labels.label(for: meeting.deviceId)]
        if let copies = meeting.alsoRecorded, !copies.isEmpty { parts.append("+\(copies.count) \(copies.count == 1 ? "copy" : "copies")") }
        if !meeting.attendees.isEmpty { parts.append(ReadLogic.attendeeSummary(meeting.attendees)) }
        return parts.joined(separator: " · ")
    }

    private var deviceChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: WTokens.Space.sm) {
                WFilterChip(label: String(localized: "All"), selected: model.deviceFilter == nil) { model.deviceFilter = nil }
                ForEach(model.devices, id: \.self) { device in
                    WFilterChip(label: labels.label(for: device), selected: model.deviceFilter == device) { model.deviceFilter = device }
                }
            }
        }
    }

    private func dayTitle(_ day: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(day) { return String(localized: "Today") }
        if calendar.isDateInYesterday(day) { return String(localized: "Yesterday") }
        if calendar.isDate(day, equalTo: Date(), toGranularity: .year) {
            return day.formatted(.dateTime.weekday(.wide).month(.abbreviated).day())
        }
        return day.formatted(.dateTime.weekday(.wide).month(.abbreviated).day().year())
    }
}
