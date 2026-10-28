import AppKit
import SwiftUI

struct MenuBarContentView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openSettings) private var openSettings
    @Environment(\.openWindow) private var openWindow

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
