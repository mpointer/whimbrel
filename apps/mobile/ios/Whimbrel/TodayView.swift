import SwiftUI

/// The phone's first screen, as one line of time: the next meeting with its prep above the now
/// marker; the day's brief and recent meetings behind it; then when the replica last synced.
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
    @State private var showPrep = TodayView.demoShowsPrep

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WTokens.Space.md) {
                header
                if pendingDevices > 0 { approvalBanner }
                upNextStop
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    WNowMarker(time: ReadLogic.timeText(context.date))
                }
                briefStop
                recentStops
                syncLine
            }
            .padding(.horizontal, WTokens.Space.lg)
            .padding(.bottom, WTokens.Space.xl)
            .frame(maxWidth: WTokens.Size.contentMax)
            .frame(maxWidth: .infinity)
        }
        .environment(\.wStrings, .app)
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

    private func time(_ unixMs: Int64) -> String {
        ReadLogic.timeText(Date(timeIntervalSince1970: Double(unixMs) / 1000))
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: WTokens.Space.xxs) {
            Text("Today")
                .wType(WTokens.TypeScale.pageTitle)
                .foregroundStyle(Color.theme(.textPrimary))
                .accessibilityAddTraits(.isHeader)
            Text(Date().formatted(.dateTime.weekday(.wide).month(.wide).day()))
                .wType(WTokens.TypeScale.secondary)
                .foregroundStyle(Color.theme(.textSecondary))
        }
    }

    private var approvalBanner: some View {
        Button { openSettings() } label: {
            Label(pendingDevices == 1 ? "A new device is waiting for your approval" : "\(pendingDevices) new devices are waiting for your approval", systemImage: "iphone.badge.plus")
                .wType(WTokens.TypeScale.body)
                .foregroundStyle(Color.theme(.textPrimary))
                .padding(WTokens.Space.md)
                .frame(maxWidth: .infinity, minHeight: WTokens.Size.target, alignment: .leading)
                .wCard(tint: Color.theme(.highlight))
        }
        .buttonStyle(.plain)
    }

    /// What is coming: the next meeting, above the now marker.
    @ViewBuilder private var upNextStop: some View {
        if let event = model.upNext {
            WSpineStop(time: time(event.startUnixMs), node: .upcoming) {
                WMeetingCard(variant: .next, title: event.title, status: String(localized: "Up next"),
                             detail: event.attendees.isEmpty ? nil : ReadLogic.attendeeSummary(event.attendees)) {
                    if model.prep?.brief != nil {
                        Button { showPrep = true } label: { Label("Prep ready", systemImage: "doc.text") }
                            .buttonStyle(.w(.secondary))
                    }
                }
            }
        } else {
            WSpineStop(time: "", node: .upcoming, showsNode: false) {
                Text(model.loaded ? "Nothing else today" : "Loading…")
                    .wType(WTokens.TypeScale.body)
                    .foregroundStyle(Color.theme(.textSecondary))
                    .padding(WTokens.Space.md)
                    .frame(maxWidth: .infinity, minHeight: WTokens.Size.target, alignment: .leading)
            }
        }
    }

    /// The day's brief, on the thread at the time it ran.
    private var briefStop: some View {
        WSpineStop(time: model.brief.map { time($0.ranAtUnixMs) } ?? "", node: .past, showsNode: model.brief != nil) {
            VStack(alignment: .leading, spacing: WTokens.Space.sm) {
                Text("Daily brief")
                    .wType(WTokens.TypeScale.subtitle)
                    .foregroundStyle(Color.theme(.textPrimary))
                    .accessibilityAddTraits(.isHeader)
                if let brief = model.brief {
                    Text(brief.outputSummary)
                        .wType(WTokens.TypeScale.body)
                        .foregroundStyle(Color.theme(.textPrimary))
                        .lineLimit(briefExpanded ? nil : 4)
                    if brief.outputSummary.count > 60 {
                        Button(briefExpanded ? "Show less" : "Show more") { briefExpanded.toggle() }
                            .buttonStyle(.w(.quiet))
                    }
                } else {
                    Text(model.loaded ? "No brief yet today" : "Loading…")
                        .wType(WTokens.TypeScale.body)
                        .foregroundStyle(Color.theme(.textSecondary))
                }
            }
            .padding(WTokens.Space.md)
            .frame(maxWidth: .infinity, alignment: .leading)
            .wCard()
        }
    }

    /// Meetings that have happened, newest first, each a stop behind the now marker.
    @ViewBuilder private var recentStops: some View {
        if model.recent.isEmpty {
            WSpineStop(time: "", node: .past, showsNode: false) {
                Text(model.loaded ? "No meetings yet" : "Loading…")
                    .wType(WTokens.TypeScale.body)
                    .foregroundStyle(Color.theme(.textSecondary))
                    .padding(WTokens.Space.md)
                    .frame(maxWidth: .infinity, minHeight: WTokens.Size.target, alignment: .leading)
            }
        }
        ForEach(model.recent) { meeting in
            WSpineStop(time: ThreadTime.pastLabel(meeting.startUnixMs), node: .past) {
                // The chip sits below the card's link, not inside it, so a tap reaches one of them.
                VStack(alignment: .leading, spacing: WTokens.Space.xs) {
                    NavigationLink(value: meeting.id) {
                        WMeetingCard(variant: .past, title: meeting.title, detail: recentDetail(meeting))
                    }
                    .buttonStyle(.plain)
                    if let topic = TopicsModel.chip(for: meeting, in: model.topicChips) {
                        TopicChipButton(topic: topic) { openTopic(TopicRoute(key: topic.key)) }
                    }
                }
            }
        }
    }

    /// Where a meeting was recorded and how many copies exist.
    private func recentDetail(_ meeting: PhoneMeeting) -> String {
        var parts = [labels.label(for: meeting.deviceId)]
        if let copies = meeting.alsoRecorded, !copies.isEmpty { parts.append("+\(copies.count) \(copies.count == 1 ? "copy" : "copies")") }
        return parts.joined(separator: " · ")
    }

    private var syncLine: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            VStack(spacing: WTokens.Space.xs) {
                Text(ReadLogic.syncLine(lastSyncedAt: sync.lastSyncedAt, now: context.date, isSyncing: sync.isSyncing, error: sync.lastError))
                if sync.olderHistoryLoading { Text("Older history is still loading.") }
                if sync.updateNeeded { Text("Update needed on another of your devices before it applies deletes.") }
            }
            .wType(WTokens.TypeScale.secondary)
            .foregroundStyle(Color.theme(.textSecondary))
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
        }
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

extension TodayView {
    /// Demo screenshots only: `-WhimbrelDemoPrep` opens the prep sheet at launch.
    static var demoShowsPrep: Bool {
        #if DEBUG
        return ProcessInfo.processInfo.arguments.contains("-WhimbrelDemoPrep")
        #else
        return false
        #endif
    }
}

/// The synced prep for the next meeting: its summary, attendee notes and open items. Whimbrel
/// wrote all of it, so the page says so once and every line carries Whimbrel's ink and rule.
struct PrepSheet: View {
    let prep: PhoneMeetingPrep?
    @Environment(\.dismiss) private var dismiss

    /// The legend says who wrote the lines, so a screen reader is not told before each one.
    private var noteStrings: WStrings {
        var s = WStrings.app
        s.whimbrelWrote = { $0 }
        return s
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: WTokens.Space.lg) {
                    if let prep, let brief = prep.brief {
                        VStack(alignment: .leading, spacing: WTokens.Space.xs) {
                            Text(prep.title)
                                .wReading(WTokens.TypeScale.readingTitle)
                                .foregroundStyle(Color.theme(.textPrimary))
                            Text(Date(timeIntervalSince1970: Double(prep.startUnixMs) / 1000).formatted(.dateTime.weekday(.wide).month(.abbreviated).day().hour().minute()))
                                .wType(WTokens.TypeScale.secondary)
                                .foregroundStyle(Color.theme(.textSecondary))
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityAddTraits(.isHeader)
                        HStack(spacing: WTokens.Space.sm) {
                            Rectangle().fill(WFixed.aiInk.color).frame(width: 2).accessibilityHidden(true)
                            Text("Written by Whimbrel").wType(WTokens.TypeScale.caption).fontWeight(.semibold).foregroundStyle(WFixed.aiInk.color)
                        }
                        .fixedSize(horizontal: false, vertical: true)
                        WNoteBlock(kind: .paragraph, author: .whimbrel, text: brief.summary.text)
                        if !brief.attendees.isEmpty {
                            group(String(localized: "Attendees"), brief.attendees.map { "\($0.who): \($0.note)" })
                        }
                        if !brief.openItems.isEmpty { group(String(localized: "Open items"), brief.openItems.map(\.text)) }
                        if !brief.openQuestions.isEmpty { group(String(localized: "Open questions"), brief.openQuestions.map(\.text)) }
                    }
                }
                .padding(.horizontal, WTokens.Space.lg)
                .padding(.vertical, WTokens.Space.md)
                .frame(maxWidth: WTokens.Size.contentMax, alignment: .leading)
                .frame(maxWidth: .infinity)
            }
            .environment(\.wStrings, noteStrings)
            .themed()
            .background(Color.theme(.backgroundPage).ignoresSafeArea())
            .navigationTitle(String(localized: "Prep"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
    }

    private func group(_ title: String, _ lines: [String]) -> some View {
        VStack(alignment: .leading, spacing: WTokens.Space.xxs) {
            WNoteBlock(kind: .heading, author: .you, text: title)
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in WNoteBlock(kind: .bullet, author: .whimbrel, text: line) }
        }
    }
}

enum ThreadTime {
    /// A past stop's label: the time, with the day above it when it was not today, so the times
    /// down the thread never read as out of order.
    static func pastLabel(_ unixMs: Int64) -> String {
        let date = Date(timeIntervalSince1970: Double(unixMs) / 1000)
        if Calendar.current.isDateInToday(date) { return ReadLogic.timeText(date) }
        return date.formatted(.dateTime.month(.abbreviated).day()) + "\n" + ReadLogic.timeText(date)
    }
}
