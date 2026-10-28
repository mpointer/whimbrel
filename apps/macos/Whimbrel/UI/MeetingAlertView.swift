import AppKit
import SwiftUI

/// The floating meeting-alert panel's content (hosted in an `NSPanel` by
/// `MeetingAlertPanelController`, not a normal SwiftUI `Window` scene). Pure
/// presentation — all state lives on `controller`.
struct MeetingAlertView: View {
    @ObservedObject var controller: MeetingAlertPanelController

    var body: some View {
        if let meeting = controller.meeting {
            AccentBarRow(color: meeting.source != .detected ? Color.whimbrelAccentPrimary : Color.whimbrelAccentSecondary) {
                HStack(alignment: .top, spacing: Spacing.md) {
                    iconView(for: meeting)
                        .frame(width: 32, height: 32)

                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        Text(meeting.source != .detected ? "Meeting Starting Soon" : "Meeting Detected")
                            .font(.whimbrelBody(.callout).bold())
                            .foregroundStyle(Color.whimbrelTextPrimary)
                        Text(meeting.title)
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                            .lineLimit(1)
                        if let seconds = controller.secondsRemaining {
                            Text("Auto-starting in \(seconds)s…")
                                .font(.whimbrelBody(.caption2))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                        }
                    }

                    Spacer(minLength: Spacing.sm)

                    VStack(alignment: .trailing, spacing: Spacing.xs) {
                        startRecordingControl
                        Button("Dismiss") { controller.dismissTapped() }
                            .buttonStyle(.whimbrelSecondary)
                            .font(.whimbrelBody(.caption2))
                    }
                }
            }
            .whimbrelCard()
            .frame(width: 360)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// A single button whose main tap starts recording immediately; its attached
    /// dropdown (SwiftUI's `Menu(_:primaryAction:)`) offers the one alternate action —
    /// mirrors the reference screenshot's "Start Recording [chevron] / & open
    /// Littlebird" control.
    private var startRecordingControl: some View {
        Menu {
            Button("Start Recording & Open Whimbrel") { controller.acceptAndOpenTimelineTapped() }
        } label: {
            Text("Start Recording")
                .font(.whimbrelBody(.callout).weight(.medium))
        } primaryAction: {
            controller.acceptTapped()
        }
        .menuStyle(.borderlessButton)
        .padding(.horizontal, Spacing.lg)
        .padding(.vertical, Spacing.sm)
        .background(Color.whimbrelAccentPrimary)
        .foregroundStyle(Color.whimbrelTextOnAccent)
        .clipShape(Capsule())
        .contentShape(Capsule())
    }

    /// Real running-app icon for a detected meeting (matching the bundle IDs
    /// `MeetingHeuristicDetector` tracks); a generic calendar glyph for a
    /// calendar-triggered one, since there's no running app to attribute it to.
    @ViewBuilder
    private func iconView(for meeting: QualifyingMeeting) -> some View {
        if let bundleID = meeting.appBundleID,
           let appURL = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) {
            Image(nsImage: NSWorkspace.shared.icon(forFile: appURL.path))
                .resizable()
                .scaledToFit()
        } else {
            Image(systemName: "calendar")
                .resizable()
                .scaledToFit()
                .foregroundStyle(Color.whimbrelAccentPrimary)
        }
    }
}
