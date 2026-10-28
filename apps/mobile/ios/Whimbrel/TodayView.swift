import SwiftUI

/// The phone's first screen: a greeting, the day's brief, the next meeting with its prep,
/// recent meetings and when the replica last synced.
struct TodayView: View {
    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    let openSettings: () -> Void
    @StateObject private var model = TodayModel()
    @State private var briefExpanded = false
    @State private var showPrep = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header
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
                NavigationLink(value: meeting.id) {
                    MeetingRow(meeting: meeting, labels: labels)
                }
                .buttonStyle(.plain)
            }
        }
    }

    private var syncLine: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            Text(ReadLogic.syncLine(lastSyncedAt: sync.lastSyncedAt, now: context.date, isSyncing: sync.isSyncing, error: sync.lastError))
                .font(.footnote)
                .foregroundStyle(Color.theme(.textSecondary))
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

/// One meeting in a list: title, then time, where it was recorded and how many copies exist.
struct MeetingRow: View {
    let meeting: PhoneMeeting
    let labels: DeviceLabels

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(meeting.title)
                .font(.body)
                .foregroundStyle(Color.theme(.textPrimary))
                .multilineTextAlignment(.leading)
            Text(subtitle)
                .font(.subheadline)
                .foregroundStyle(Color.theme(.textSecondary))
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
                            group("Attendees", brief.attendees.map { "\($0.who): \($0.note)" })
                        }
                        if !brief.openItems.isEmpty { group("Open items", brief.openItems.map(\.text)) }
                        if !brief.openQuestions.isEmpty { group("Open questions", brief.openQuestions.map(\.text)) }
                    }
                }
                .padding(16)
            }
            .background(Color.theme(.backgroundPage).ignoresSafeArea())
            .navigationTitle(prep?.title ?? "Prep")
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
