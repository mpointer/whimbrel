import AppKit
import SwiftUI

struct MenuBarContentView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openSettings) private var openSettings
    @Environment(\.openWindow) private var openWindow
    @State private var nextTopic: TopicRef?

    /// The next calendar event that has not ended (the list looks back 15 minutes).
    private var next: QualifyingMeeting? {
        let now = Date()
        return appState.allUpcomingMeetings.filter { $0.endDate > now }.min { $0.startDate < $1.startDate }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text("Whimbrel")
                .font(.whimbrelDisplay())
                .foregroundStyle(Color.whimbrelTextPrimary)

            Toggle("Activity capture", isOn: Binding(
                get: { appState.activityCaptureEnabled },
                // See TimelineView.swift's identical toggle for why this is
                // deferred rather than called inline — same reentrancy risk, same fix.
                set: { _ in Task { @MainActor in appState.toggleActivityCapture() } }
            ))
            .tint(Color.whimbrelAccentPrimary)

            Text(appState.statusSummary)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

            if let next {
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    Text("Next: \(next.title), \(next.startDate.formatted(.dateTime.hour().minute()))")
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .lineLimit(2)
                    if let nextTopic { TopicChip(topic: nextTopic) }
                }
                .task(id: next.id) {
                    guard let store = appState.coreStore else { nextTopic = nil; return }
                    let (id, title) = (next.id, next.title)
                    nextTopic = await withCheckedContinuation { done in
                        CoreMemoryStore.accessQueue.async { done.resume(returning: store.topicForEvent(eventId: id, title: title)) }
                    }
                }
            }

            if appState.activeMeetingId != nil {
                Button("End Meeting") {
                    Task { @MainActor in appState.endActiveMeetingIfNeeded() }
                }
                .buttonStyle(.whimbrelSecondary)
            } else {
                Button("Start Meeting") {
                    Task { @MainActor in appState.startManualMeeting() }
                }
                .buttonStyle(.whimbrelSecondary)
            }

            if !appState.inactiveCapabilities.isEmpty {
                Divider().overlay(Color.whimbrelBorder)
                Text("Inactive")
                    .font(.whimbrelBody(.caption).bold())
                    .foregroundStyle(Color.whimbrelTextPrimary)
                ForEach(Array(appState.inactiveCapabilities.enumerated()), id: \.offset) { _, item in
                    Text("• \(item.capability): \(item.reason)")
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelWarning)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            if let processingStage = appState.processingStage {
                Divider().overlay(Color.whimbrelBorder)
                HStack(spacing: Spacing.xs) {
                    ProgressView().controlSize(.small)
                    Text(processingStage)
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
            }

            Divider().overlay(Color.whimbrelBorder)
            HStack {
                Button("Settings…") { openSettings() }
                    .buttonStyle(.whimbrelSecondary)
                Button("Open Whimbrel") { openWindow(id: "timeline") }
                    .buttonStyle(.whimbrelSecondary)
                #if DEBUG
                Button("Dump Zoom window (AX)") { AXTreeDump.dumpZoomWindows() }
                    .buttonStyle(.whimbrelSecondary)
                Button("Record Zoom (60 s)") { AXTreeDump.recordZoomWindows() }
                    .buttonStyle(.whimbrelSecondary)
                #endif
                Spacer()
                Button("Quit") { NSApplication.shared.terminate(nil) }
                    .buttonStyle(.whimbrelSecondary)
            }
        }
        .padding(Spacing.lg)
        .frame(width: 320)
        .background(Color.whimbrelBackgroundPage)
    }
}
