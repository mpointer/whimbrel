import SwiftUI
import WhimbrelUI

/// FR-CAP-5 / FR-UI-1: capture state is always visible in the menu bar, via the
/// StatusGlyph template image (Assets.xcassets — auto-tints for light/dark menu bar,
/// same as an SF Symbol would) dimmed when idle (neither activity capture nor a
/// meeting is running), plus a small color badge: red specifically while a meeting is
/// actively recording (the more sensitive, attention-worthy state), amber when
/// something's on but degraded or missing a permission it needs. A recording is never
/// hidden by a problem: red wins, and the problem shows as a warning mark beside the
/// elapsed time. Activity capture
/// running alone (no meeting) gets a small neutral dot (the trust surface spec,
/// docs/superpowers/specs/2026-11-04-trust-surface-design.md): capture of the screen is
/// always visible, in the color of ordinary text rather than a warning color, because it
/// is the app's normal state and still something a person should be able to see.
struct StatusIcon: View {
    @ObservedObject var state: AppState

    var body: some View {
        HStack(spacing: 4) {
            // An accessible name for the icon.
            icon.accessibilityElement(children: .ignore).accessibilityLabel(accessibleName)
            // Never a TimelineView here: see `AppState.menuBarElapsed`.
            if let elapsed = state.menuBarElapsed {
                if Self.showsProblemMark(degraded: isDegraded, meetingRecording: state.activeMeetingId != nil) {
                    // The badge stays red for the recording, so the problem gets its own mark, by shape.
                    Image(systemName: "exclamationmark.triangle.fill")
                    Text(elapsed)
                        .monospacedDigit()
                } else {
                    Text("● " + elapsed)
                        .monospacedDigit()
                }
            }
        }
    }

    private var icon: some View {
        Image("StatusGlyph")
            .renderingMode(.template)
            .opacity(isActive ? 1 : 0.4)
            .overlay(alignment: .topTrailing) {
                if let badgeColor {
                    if state.recordingPaused {
                        // Paused has its own shape.
                        Image(systemName: "pause.fill").font(.system(size: 6, weight: .bold)).foregroundStyle(badgeColor == .red ? Color.primary : badgeColor)
                            .offset(x: 2, y: -2)
                    } else {
                        Circle()
                            .fill(badgeColor)
                            .frame(width: 6, height: 6)
                            .offset(x: 2, y: -2)
                    }
                }
            }
    }

    private var accessibleName: String {
        let strings = WhimbrelUIStrings.make()
        if state.recordingPaused { return strings.joinDetails(["Whimbrel", strings.recordingPaused]) }
        if state.activeMeetingId != nil { return strings.joinDetails(["Whimbrel", strings.recording] + (state.menuBarElapsed.map { [$0] } ?? [])) }
        return "Whimbrel"
    }

    private var isActive: Bool {
        state.activityCaptureEnabled || state.activeMeetingId != nil
    }

    private var isDegraded: Bool {
        !state.permissions.allGranted || state.audio.degradedReason != nil || state.audio.micDegradedReason != nil
    }

    private var badgeColor: Color? {
        Self.badgeColor(
            active: isActive,
            degraded: isDegraded,
            meetingRecording: state.activeMeetingId != nil,
            activityCapture: state.activityCaptureEnabled
        )
    }

    /// Red for a recording meeting, orange for a degraded state, a neutral dot for
    /// activity capture on its own, nothing when idle. A recording comes first: a missing
    /// permission or an audio problem must never take the recording color away. Pure, so
    /// the test target can check the precedence without an `AppState`.
    static func badgeColor(active: Bool, degraded: Bool, meetingRecording: Bool, activityCapture: Bool) -> Color? {
        guard active else { return nil }
        if meetingRecording { return .red }
        if degraded { return .orange }
        return activityCapture ? .secondary : nil
    }

    /// Whether the warning mark is drawn beside the elapsed time: a recording with a problem.
    /// Without a recording the orange badge already says it.
    static func showsProblemMark(degraded: Bool, meetingRecording: Bool) -> Bool {
        degraded && meetingRecording
    }
}
