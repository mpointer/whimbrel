import SwiftUI

/// The phone's first screen: a greeting, the day's brief, the next meeting with its prep,
/// recent meetings and when the replica last synced.
struct TodayView: View {
    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    /// Devices waiting for approval: a local, in-app banner (no push yet).
    let pendingDevices: Int
    let openSettings: () -> Void
    /// Pushes a topic page onto this tab's path (a meeting's topic chip).
    let openTopic: (TopicRoute) -> Void
    @StateObject private var model = TodayModel()
    @State private var briefExpanded = false
    @State private var showPrep = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header
                if pendingDevices > 0 { approvalBanner }
                briefCard
                upNextCard
                recentSection
                syncLine
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 24)
        }
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button { openSettings() } label: { Image(systemName: "person.crop.circle") }
                    .accessibilityLabel("Settings")
            }
        }
        .navigationDestination(for: Int64.self) { MeetingDetailView(meetingId: $0, sync: sync, labels: labels) }
        .navigationDestination(for: TopicRoute.self) { TopicPageView(key: $0.key, sync: sync) }
        .task(id: sync.readRevision) { await model.reload(sync: sync, now: Date(), calendar: .current) }
        .refreshable { await sync.syncNow(manual: true) }
        .sheet(isPresented: $showPrep) { PrepSheet(prep: model.prep) }
    }

    private var header: some View {
        let now = Date()
        return VStack(alignment: .leading, spacing: 4) {
            Text(now.formatted(.dateTime.weekday(.wide).month(.wide).day()))
                .font(.subheadline)
                .foregroundStyle(Color.theme(.textSecondary))
            Text("Good \(ReadLogic.daypart(now: now, calendar: .current))")
                .font(.system(.largeTitle, design: .serif))
                .foregroundStyle(Color.theme(.textPrimary))
        }
        .accessibilityElement(children: .combine)
    }

    private var approvalBanner: some View {
        Button { openSettings() } label: {
            card {
                Label(pendingDevices == 1 ? "A new device is waiting for your approval" : "\(pendingDevices) new devices are waiting for your approval", systemImage: "iphone.badge.plus")
                    .foregroundStyle(Color.theme(.textPrimary))
            }
        }
        .buttonStyle(.plain)
    }

    private var briefCard: some View {
        card {
            VStack(alignment: .leading, spacing: 8) {
                sectionLabel("Daily brief")
                if let brief = model.brief {
                    Text(brief.outputSummary)
                        .foregroundStyle(Color.theme(.textPrimary))
                        .lineLimit(briefExpanded ? nil : 4)
                    if brief.outputSummary.count > 60 {
                        Button(briefExpanded ? "Show less" : "Show more") { briefExpanded.toggle() }
                            .font(.subheadline)
                            .frame(minHeight: 44, alignment: .leading)
                    }
                } else {
                    Text(model.loaded ? "No brief yet today" : "Loading…")
                        .foregroundStyle(Color.theme(.textSecondary))
                }
            }
        }
    }

    private var upNextCard: some View {
        card {
            VStack(alignment: .leading, spacing: 8) {
                sectionLabel("Up next")
                if let event = model.upNext {
                    Text(ReadLogic.timeText(Date(timeIntervalSince1970: Double(event.startUnixMs) / 1000)))
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(Color.theme(.accentPrimary))
                    Text(event.title)
                        .font(.headline)
                        .foregroundStyle(Color.theme(.textPrimary))
                    if !event.attendees.isEmpty {
                        Text(ReadLogic.attendeeSummary(event.attendees))
                            .font(.subheadline)
                            .foregroundStyle(Color.theme(.textSecondary))
                    }
                    if model.prep?.brief != nil {
                        Button { showPrep = true } label: {
                            Label("Prep ready", systemImage: "doc.text")
                                .font(.subheadline.weight(.medium))
                                .padding(.horizontal, 12)
                                .frame(minHeight: 44)
                                .background(Color.theme(.accentSecondary).opacity(0.12), in: Capsule())
                        }
                        .foregroundStyle(Color.theme(.accentSecondary))
                    }
                } else {
                    Text(model.loaded ? "Nothing else today" : "Loading…")
                        .foregroundStyle(Color.theme(.textSecondary))
                }
            }
        }
    }

    private var recentSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            sectionLabel("Recent meetings")
            if model.recent.isEmpty {
                Text(model.loaded ? "No meetings yet" : "Loading…").foregroundStyle(Color.theme(.textSecondary))
            }
            ForEach(model.recent) { meeting in
                // The chip sits below the row's link, not inside it, so a tap reaches one of them.
                VStack(alignment: .leading, spacing: 0) {
                    NavigationLink(value: meeting.id) {
                        MeetingRow(meeting: meeting, labels: labels)
                    }
                    .buttonStyle(.plain)
                    if let topic = TopicsModel.chip(for: meeting, in: model.topicChips) {
                        TopicChipButton(topic: topic) { openTopic(TopicRoute(key: topic.key)) }
                    }
                }
            }
        }
    }

    private var syncLine: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            VStack(spacing: 4) {
                Text(ReadLogic.syncLine(lastSyncedAt: sync.lastSyncedAt, now: context.date, isSyncing: sync.isSyncing, error: sync.lastError))
                if sync.olderHistoryLoading { Text("Older history is still loading.") }
            }
            .font(.footnote)
            .foregroundStyle(Color.theme(.textSecondary))
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
        }
    }

    private func sectionLabel(_ text: String) -> some View {
        Text(text.uppercased())
            .font(.caption.weight(.semibold))
            .tracking(0.6)
            .foregroundStyle(Color.theme(.textSecondary))
    }

    private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        content()
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(16)
            .background(Color.theme(.backgroundCard), in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.theme(.border), lineWidth: 1))
    }
}

/// One meeting in a list: title, then time, where it was recorded and how many copies exist,
/// and the topic it belongs to as a chip that opens the topic page.
struct MeetingRow: View {
    let meeting: PhoneMeeting
    let labels: DeviceLabels
    var topic: PhoneTopicChip? = nil
    var openTopic: (TopicRoute) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(meeting.title)
                .font(.body)
                .foregroundStyle(Color.theme(.textPrimary))
                .multilineTextAlignment(.leading)
            Text(subtitle)
                .font(.subheadline)
                .foregroundStyle(Color.theme(.textSecondary))
            if let topic {
                TopicChipButton(topic: topic) { openTopic(TopicRoute(key: topic.key)) }
            }
        }
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .contentShape(Rectangle())
    }

    private var subtitle: String {
        var parts = [
            ReadLogic.timeText(Date(timeIntervalSince1970: Double(meeting.startUnixMs) / 1000)),
            labels.label(for: meeting.deviceId),
        ]
        if let copies = meeting.alsoRecorded, !copies.isEmpty { parts.append("+\(copies.count) \(copies.count == 1 ? "copy" : "copies")") }
        return parts.joined(separator: " · ")
    }
}

/// A meeting's topic as a capsule that opens the topic page. Borderless: inside a List row
/// (Meetings) a borderless button takes its own tap instead of selecting the row.
struct TopicChipButton: View {
    let topic: PhoneTopicChip
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(topic.name, systemImage: "tag")
                .font(.subheadline.weight(.medium))
                .lineLimit(1)
                .padding(.horizontal, 12)
                .frame(minHeight: 44)
                .background(Color.theme(.accentSecondary).opacity(0.12), in: Capsule())
        }
        .buttonStyle(.borderless)
        .foregroundStyle(Color.theme(.accentSecondary))
        .accessibilityLabel(String(localized: "Topic: \(topic.name)"))
    }
}

/// The synced prep for the next meeting: its summary, attendee notes and open items.
struct PrepSheet: View {
    let prep: PhoneMeetingPrep?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let brief = prep?.brief {
                        Text(brief.summary.text).foregroundStyle(Color.theme(.textPrimary))
                        if !brief.attendees.isEmpty {
                            group(String(localized: "Attendees"), brief.attendees.map { "\($0.who): \($0.note)" })
                        }
                        if !brief.openItems.isEmpty { group(String(localized: "Open items"), brief.openItems.map(\.text)) }
                        if !brief.openQuestions.isEmpty { group(String(localized: "Open questions"), brief.openQuestions.map(\.text)) }
                    }
                }
                .padding(16)
            }
            .background(Color.theme(.backgroundPage).ignoresSafeArea())
            .navigationTitle(prep?.title ?? String(localized: "Prep"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
    }

    private func group(_ title: String, _ lines: [String]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title.uppercased()).font(.caption.weight(.semibold)).tracking(0.6).foregroundStyle(Color.theme(.textSecondary))
            ForEach(lines, id: \.self) { Text("• \($0)").foregroundStyle(Color.theme(.textPrimary)) }
        }
    }
}
