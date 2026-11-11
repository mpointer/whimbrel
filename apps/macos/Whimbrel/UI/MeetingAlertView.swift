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
                VStack(alignment: .leading, spacing: Spacing.sm) {
                    HStack(alignment: .top, spacing: Spacing.md) {
                        iconView(for: meeting)
                            .frame(width: 32, height: 32)

                        VStack(alignment: .leading, spacing: Spacing.xs) {
                            Text(controller.earlierTitle != nil ? "Recording as a New Meeting"
                                 : meeting.source != .detected ? "Meeting Starting Soon" : "Meeting Detected")
                                .font(.whimbrelBody(.callout).bold())
                                .foregroundStyle(Color.whimbrelTextPrimary)
                            Text(meeting.title)
                                .font(.whimbrelBody(.caption))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                                .lineLimit(1)
                            if let earlierTitle = controller.earlierTitle {
                                Text("Ended “\(earlierTitle)” and saved it separately.")
                                    .font(.whimbrelBody(.caption2))
                                    .foregroundStyle(Color.whimbrelTextSecondary)
                                    .lineLimit(1)
                            }
                            if let seconds = controller.secondsRemaining {
                                Text("Auto-starting in \(seconds)s…")
                                    .font(.whimbrelBody(.caption2))
                                    .foregroundStyle(Color.whimbrelTextSecondary)
                            }
                        }

                        Spacer(minLength: Spacing.sm)

                        Group {
                            if controller.earlierTitle != nil {
                                Button("Keep together") { controller.acceptTapped() }
                                    .buttonStyle(.whimbrelSecondary)
                                    .font(.whimbrelBody(.caption2))
                                    .help("Undo the split: record this as part of the earlier meeting")
                            } else {
                                startRecordingControl
                            }
                        }
                        .fixedSize()
                    }

                    // Its own row under the text, so up to three buttons never squeeze the
                    // title or wrap their labels.
                    HStack(spacing: Spacing.xs) {
                        Spacer(minLength: 0)
                        if controller.prepKey != nil {
                            Button("Open brief") { controller.openPrepTapped() }
                                .buttonStyle(.whimbrelSecondary)
                                .font(.whimbrelBody(.caption2))
                        }
                        if let join = controller.joinTarget {
                            Button("Join") { controller.joinTapped() }
                                .buttonStyle(.whimbrelSecondary)
                                .font(.whimbrelBody(.caption2))
                                .accessibilityLabel("Join on \(join.provider.displayName)")
                        }
                        Button("Dismiss") { controller.dismissTapped() }
                            .buttonStyle(.whimbrelSecondary)
                            .font(.whimbrelBody(.caption2))
                    }
                    .fixedSize(horizontal: false, vertical: true)
                    .lineLimit(1)
                }
            }
            .whimbrelCard()
            .frame(width: 420)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// One capsule: the main tap starts recording immediately, and a chevron beside it opens
    /// the one alternate action. Two plain controls, not `Menu(primaryAction:)` with
    /// `.menuStyle(.borderlessButton)`: that style draws its label with AppKit's own control
    /// text color and ignores `foregroundStyle`, so in light mode the label came out in the
    /// system color (not the theme's on-accent color) on the accent fill and was hard to read.
    /// Plain button styles honor `foregroundStyle`, and `textOnAccent` on `accentPrimary` is
    /// already held to 4.5:1 in every palette by `ThemeContrast.requiredPairs`.
    private var startRecordingControl: some View {
        HStack(spacing: 0) {
            Button { controller.acceptTapped() } label: {
                Text("Start Recording")
                    .font(.whimbrelBody(.callout).weight(.medium))
                    .padding(.leading, Spacing.md)
                    .padding(.trailing, Spacing.sm)
                    .padding(.vertical, Spacing.sm)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            Rectangle()
                .fill(Color.whimbrelTextOnAccent.opacity(0.35))
                .frame(width: 1, height: 16)
            Menu {
                Button("Start Recording & Open Whimbrel") { controller.acceptAndOpenTimelineTapped() }
            } label: {
                Image(systemName: "chevron.down")
                    .font(.system(size: 10, weight: .semibold))
                    .padding(.horizontal, Spacing.sm)
                    .padding(.vertical, Spacing.sm)
                    .contentShape(Rectangle())
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel("Start Recording & Open Whimbrel")
        }
        .foregroundStyle(Color.whimbrelTextOnAccent)
        .background(Color.whimbrelAccentPrimary)
        .clipShape(Capsule())
        .contentShape(Capsule())
    }

    /// Real running-app icon for a detected meeting (matching the bundle IDs
    /// `MeetingHeuristicDetector` tracks); for a
    /// calendar-triggered one, Whimbrel's own icon, since there's no running app to attribute it to.
    @ViewBuilder
    private func iconView(for meeting: QualifyingMeeting) -> some View {
        if let bundleID = meeting.appBundleID,
           let appURL = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) {
            Image(nsImage: NSWorkspace.shared.icon(forFile: appURL.path))
                .resizable()
                .scaledToFit()
        } else {
            // A calendar meeting has no running app to show, so it shows Whimbrel's own icon.
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .scaledToFit()
        }
    }
}
