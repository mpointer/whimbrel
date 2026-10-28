import AppKit
import SwiftUI
import os

/// Library's capture status header and the in-meeting notes section.
extension TimelineView {
    @ViewBuilder
    var captureStatusHeader: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack {
                Toggle("Activity capture", isOn: Binding(
                    get: { appState.activityCaptureEnabled },
                    // Deferred to the next run-loop turn rather than called inline — see
                    // the identical toggle in MenuBarContentView.swift for why: calling
                    // toggleActivityCapture() synchronously from within this Toggle's own
                    // gesture dispatch previously crashed with SIGSEGV in SwiftUI's
                    // _ButtonGesture/MainActor.assumeIsolated internals (macOS 26.5.2,
                    // crash report Whimbrel-2026-07-25-152222.ips).
                    set: { _ in Task { @MainActor in appState.toggleActivityCapture() } }
                ))
                .tint(Color.whimbrelAccentPrimary)
                Spacer()
            }
            Text(appState.statusSummary)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
            if let processingStage = appState.processingStage {
                HStack(spacing: Spacing.xs) {
                    ProgressView().controlSize(.small)
                    Text(processingStage)
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
            }

            if appState.activeMeetingId != nil, appState.audio.recordingOnBuiltInSpeakers, !headphonesWarningDismissed {
                HStack(alignment: .top, spacing: Spacing.xs) {
                    Text("Recording on speakers — the mic may pick up meeting audio. Headphones reduce this.")
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer()
                    Button("Dismiss") { headphonesWarningDismissed = true }
                        .buttonStyle(.whimbrelSecondary)
                        .controlSize(.small)
                }
            }

            if appState.activeMeetingId != nil {
                activeMeetingNotesSection
            } else {
                HStack {
                    Spacer()
                    Button("Start Meeting") {
                        Task { @MainActor in appState.startManualMeeting() }
                    }
                    .buttonStyle(.whimbrelPrimary)
                }
            }
        }
        .padding(Spacing.md)
        .background(Color.whimbrelBackgroundCard)
        .clipShape(RoundedRectangle(cornerRadius: Radius.control))
    }


    /// FR-MTG-5: shown only while a meeting is actively recording — the in-meeting
    /// notes pad and a manual "End Meeting" override for the automatic heuristics.
    /// Moved here from the (soon to be retired) debug window's identical section.
    @ViewBuilder
    var activeMeetingNotesSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            HStack {
                Text("Recording: \(appState.activeMeetingTitle ?? "Meeting")\(appState.activeMeetingIsContinued ? " \u{00B7} Continued" : "")")
                    .font(.whimbrelBody(.callout).bold())
                    .foregroundStyle(Color.whimbrelTextPrimary)
                Spacer()
                Button("End Meeting") {
                    Task { @MainActor in appState.endActiveMeetingIfNeeded() }
                }
                .buttonStyle(.whimbrelSecondary)
            }
            Text("Notes")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
            TextEditor(text: Binding(
                get: { appState.meetingNotes },
                set: { appState.meetingNotes = $0 }
            ))
            .font(.whimbrelBody(.callout))
            .frame(height: 80)
            .padding(Spacing.xs)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.control))
            .overlay(RoundedRectangle(cornerRadius: Radius.control).stroke(Color.whimbrelBorder, lineWidth: 1))
        }
    }
}
