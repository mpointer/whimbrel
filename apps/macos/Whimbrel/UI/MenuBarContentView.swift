import AppKit
import SwiftUI
import WhimbrelUI

struct MenuBarContentView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openSettings) private var openSettings
    @Environment(\.openWindow) private var openWindow
    @State private var nextTopic: TopicRef?
    /// The next meeting's brief key, when it has a readable brief.
    @State private var nextPrepKey: String?
    /// When this menu window appeared, to tell which app the user was in before it.
    @State private var openedAt = Date()

    /// The next calendar event that has not ended (the list looks back 15 minutes).
    private var next: QualifyingMeeting? {
        let now = Date()
        return appState.allUpcomingMeetings.filter { $0.endDate > now }.min { $0.startDate < $1.startDate }
    }

    /// Redesign flag only: the timed pause choices, and the time a pause ends. Not offered while capture is off.
    @ViewBuilder
    private var capturePauseControls: some View {
        let state = CapturePauseModel.state(captureOn: appState.activityCaptureEnabled, resumeAt: appState.captureResumeAt)
        if let at = appState.captureResumeAt, state == .paused {
            Text(String(localized: "Paused until \(CapturePauseModel.untilText(at, now: Date()))", comment: "Work memory state in the sidebar footer. The placeholder is a clock time, for example 10:52 AM."))
                .font(.whimbrelBody(.caption).weight(.semibold))
                .foregroundStyle(Color.whimbrelTextPrimary)
        }
        if state != .off {
            Menu(String(localized: "Pause", comment: "Button on the recording bar that pauses a recording.")) {
                Button(String(localized: "Pause for 15 minutes")) { appState.pauseCapture(.fifteenMinutes) }
                Button(String(localized: "Pause for 1 hour")) { appState.pauseCapture(.oneHour) }
                Button(String(localized: "Pause until tomorrow")) { appState.pauseCapture(.untilTomorrow) }
                if state == .paused {
                    Divider()
                    Button(String(localized: "Resume now")) { appState.resumeCaptureNow() }
                }
            }
            .recordingControl()
        }
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
            .recordingControl()

            if appState.redesignEnabled { capturePauseControls }

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
                    if let nextPrepKey {
                        Button("Open brief") { appState.showPrepBrief(key: nextPrepKey) }
                            .buttonStyle(.whimbrelSecondary)
                    }
                }
                // Also rerun when a brief may have landed: this Mac's prep tick, or a sync.
                .task(id: "\(TodayLogic.prepKey(eventId: next.id, start: next.startDate))|\(appState.routineScheduler.recentRuns.first?.id ?? 0)|\(appState.contentSyncCoordinator.lastSyncedAt?.timeIntervalSince1970 ?? 0)") {
                    guard let store = appState.coreStore else { nextTopic = nil; nextPrepKey = nil; return }
                    let (id, title) = (next.id, next.title)
                    let key = TodayLogic.prepKey(eventId: next.id, start: next.startDate)
                    let (topic, prepKey) = await withCheckedContinuation { done in
                        CoreMemoryStore.accessQueue.async {
                            done.resume(returning: (store.topicForEvent(eventId: id, title: title), store.meetingPrep(key: key)?.brief != nil ? key : nil))
                        }
                    }
                    nextTopic = topic
                    nextPrepKey = prepKey
                }
            }

            Button("Ask about this window…") { appState.openHotkeyPanelFromMenu(menuOpenedAt: openedAt) }
                .buttonStyle(.whimbrelSecondary)

            // Recorded time, Pause or Resume and the in-meeting window (redesign flag only).
            if appState.redesignEnabled, appState.activeMeetingId != nil {
                if let elapsed = appState.menuBarElapsed {
                    Text("\(appState.recordingPaused ? String(localized: "Paused") : String(localized: "Recording")) \(elapsed)")
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
                Button(appState.recordingPaused ? String(localized: "Resume") : String(localized: "Pause")) {
                    Task { @MainActor in appState.toggleRecordingPause() }
                }
                .buttonStyle(.whimbrelSecondary)
                .disabled(!appState.canPauseRecording && !appState.recordingPaused)
                Button(String(localized: "Show in-meeting window")) {
                    Task { @MainActor in appState.meetingBar.showFromMenu() }
                }
                .buttonStyle(.whimbrelSecondary)
            }
            if appState.activeMeetingId != nil {
                Button("End Meeting") {
                    Task { @MainActor in appState.endActiveMeetingIfNeeded() }
                }
                .buttonStyle(.whimbrelSecondary)
            } else if appState.canStopStartingRecording {
                // Audio is already on and the meeting is still being saved: a Stop stops the audio now.
                Button("End Meeting") {
                    Task { @MainActor in appState.endActiveMeetingIfNeeded() }
                }
                .buttonStyle(.whimbrelSecondary)
            } else if appState.isStopPendingForRow {
                // Stopped, audio off, the meeting row still being written: nothing to start or stop.
                Button("Saving…") {}
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(true)
            } else {
                Button("Start Meeting") {
                    Task { @MainActor in appState.startManualMeeting() }
                }
                .buttonStyle(.whimbrelSecondary)
                .recordingControl()
                RecordingOffCaption()
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
        .onAppear { openedAt = Date() }
    }
}
