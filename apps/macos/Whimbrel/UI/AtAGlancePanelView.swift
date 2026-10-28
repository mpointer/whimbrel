import AppKit
import SwiftUI

/// FR-MTG-5/FR-MEM-3: a persistent right-side panel in the Timeline window — upcoming
/// meetings (real data, `CalendarService`) and today's digest (real data, the existing
/// daily-digest feature), cached per calendar day via `DailyDigestCache` so reopening
/// the panel doesn't re-trigger the underlying LLM call. No Routines/Projects section
/// — both are deferred, separate sub-projects (see the design spec).
struct AtAGlancePanelView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openWindow) private var openWindow

    @State private var digest: CoreDailyDigest?
    @State private var isGeneratingDigest = false
    @State private var threads: [CoreThread] = []

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.lg) {
                Text("At a glance").font(.whimbrelDisplay(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
                Text(Date().formatted(date: .complete, time: .omitted))
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)

                scheduleSection
                Divider().overlay(Color.whimbrelBorder)
                digestSection
                Divider().overlay(Color.whimbrelBorder)
                threadsSection
            }
            .padding(Spacing.lg)
        }
        .frame(width: 280)
        .background(Color.whimbrelBackgroundPage)
        .onAppear {
            loadDigestIfNeeded()
            loadThreadsIfNeeded()
        }
    }

    @ViewBuilder
    private var scheduleSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Schedule").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            if !appState.permissions.calendarGranted && appState.calendarConnectorState.accounts.isEmpty {
                Text("Grant Calendar access in Settings, or connect a Google Calendar account, to see upcoming meetings.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else if appState.allUpcomingMeetings.isEmpty {
                Text("No upcoming meetings left today.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                ForEach(appState.allUpcomingMeetings) { meeting in
                    scheduleRow(for: meeting)
                }
            }
        }
    }

    @ViewBuilder
    private func scheduleRow(for meeting: QualifyingMeeting) -> some View {
        AccentBarRow(color: Color.whimbrelAccentPrimary) {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                HStack {
                    Text(meeting.startDate.formatted(date: .omitted, time: .shortened))
                        .font(.whimbrelBody(.caption).bold())
                        .foregroundStyle(Color.whimbrelTextPrimary)
                    Spacer()
                    if let url = meeting.conferenceLink?.url {
                        Button("Join") { NSWorkspace.shared.open(url) }
                            .buttonStyle(.whimbrelSecondary)
                    }
                }
                Text(meeting.title)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, Spacing.xs)
    }

    @ViewBuilder
    private var digestSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            HStack {
                Text("Today's summary").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                Spacer()
                Button("Refresh") { generateDigest() }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(isGeneratingDigest)
            }
            if isGeneratingDigest {
                ProgressView().controlSize(.small)
            } else if let digest {
                Text(digest.summary)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                if !digest.openLoops.isEmpty {
                    Text("Open loops").font(.whimbrelBody(.caption2).bold()).foregroundStyle(Color.whimbrelTextSecondary)
                    ForEach(digest.openLoops, id: \.self) { loop in
                        Text("• \(loop)").font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
                    }
                }
            } else {
                Text("No summary yet today.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    private func loadDigestIfNeeded() {
        if let cached = DailyDigestCache.loadIfFresh() {
            digest = cached
            return
        }
        generateDigest()
    }

    private func generateDigest() {
        guard let store = appState.slowStore, !isGeneratingDigest else { return }
        isGeneratingDigest = true
        let calendar = Calendar.current
        let startOfToday = calendar.startOfDay(for: Date())
        let startOfTomorrow = calendar.date(byAdding: .day, value: 1, to: startOfToday) ?? startOfToday
        let startMs = Int64(startOfToday.timeIntervalSince1970 * 1000)
        let endMs = Int64(startOfTomorrow.timeIntervalSince1970 * 1000)
        let ollamaModel = appState.routing.ollamaDefaultModel
        let routing = appState.routing.routingConfig(
            preferredProvider: appState.routing.preferredProvider, preferredModel: appState.routing.preferredModel,
            dailyDigestInstructions: appState.promptLibrary.instructions[.dailyDigest] ?? ""
        )
        CoreMemoryStore.slowLaneQueue.async {
            let outcome = store.dailyDigest(model: ollamaModel, startUnixMs: startMs, endUnixMs: endMs, routing: routing)
            DispatchQueue.main.async {
                isGeneratingDigest = false
                if case .some(.success(let result)) = outcome {
                    digest = result
                    DailyDigestCache.store(result)
                }
            }
        }
    }

    @ViewBuilder
    private var threadsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Needs your attention").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            if threads.isEmpty {
                Text("No threads waiting on you right now.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                ForEach(threads) { thread in
                    threadRow(for: thread)
                }
            }
        }
    }

    @ViewBuilder
    private func threadRow(for thread: CoreThread) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Button {
                openWindow(id: "thread-detail", value: thread.id)
            } label: {
                HStack(spacing: Spacing.xs) {
                    Image(systemName: Self.threadStatusSymbol(thread.status))
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Self.threadStatusColor(thread.status))
                        .accessibilityLabel(Self.threadStatusLabel(thread.status))
                    Text(thread.subject ?? thread.lastMessageFrom)
                        .font(.whimbrelBody(.caption).bold())
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .lineLimit(1)
                    Spacer()
                }
            }
            .buttonStyle(.plain)
            Text(Self.relativeThreadTime(thread.lastActivityTsUnixMs))
                .font(.whimbrelBody(.caption2))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
        Divider().overlay(Color.whimbrelBorder)
    }

    /// A thread's direction at a glance. `threads_needing_attention` (this section's
    /// backing query) only ever returns `waiting_on_me`/`waiting_on_them`, so the
    /// `default` arm exists purely so an unrecognized status can't render as nothing.
    private static func threadStatusSymbol(_ status: String) -> String {
        switch status {
        case "waiting_on_me": "arrow.down.circle.fill"
        case "waiting_on_them": "arrow.up.circle.fill"
        default: "circle"
        }
    }

    private static func threadStatusColor(_ status: String) -> Color {
        switch status {
        case "waiting_on_me": Color.whimbrelAccentPrimary
        case "waiting_on_them": Color.whimbrelAccentSecondary
        default: Color.whimbrelTextSecondary
        }
    }

    private static func threadStatusLabel(_ status: String) -> String {
        switch status {
        case "waiting_on_me": "Waiting on you"
        case "waiting_on_them": "Waiting on them"
        default: status
        }
    }

    private static func relativeThreadTime(_ unixMs: Int64) -> String {
        Date(timeIntervalSince1970: Double(unixMs) / 1000).formatted(.relative(presentation: .named))
    }

    private func loadThreadsIfNeeded() {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let result = store.threadsNeedingAttention(limit: 20) ?? []
            DispatchQueue.main.async {
                threads = result
            }
        }
    }
}
