import SwiftUI

/// FR-CAP-5 / FR-UI-1: capture state is always visible in the menu bar, via the
/// StatusGlyph template image (Assets.xcassets — auto-tints for light/dark menu bar,
/// same as an SF Symbol would) dimmed when idle (neither activity capture nor a
/// meeting is running), plus a small color badge: red specifically while a meeting is
/// actively recording (the more sensitive, attention-worthy state), amber when
/// something's on but degraded or missing a permission it needs. Activity capture
/// running alone (no meeting) gets a small neutral dot (the trust surface spec,
/// docs/superpowers/specs/2026-11-04-trust-surface-design.md): capture of the screen is
/// always visible, in the color of ordinary text rather than a warning color, because it
/// is the app's normal state and still something a person should be able to see.
struct StatusIcon: View {
    @ObservedObject var state: AppState

    var body: some View {
        HStack(spacing: 4) {
            icon
            // Never a TimelineView here: see `AppState.menuBarElapsed`.
            if let elapsed = state.menuBarElapsed {
                Text("● " + elapsed)
                    .monospacedDigit()
            }
        }
    }

    private var icon: some View {
        Image("StatusGlyph")
            .renderingMode(.template)
            .opacity(isActive ? 1 : 0.4)
            .overlay(alignment: .topTrailing) {
                if let badgeColor {
                    Circle()
                        .fill(badgeColor)
                        .frame(width: 6, height: 6)
                        .offset(x: 2, y: -2)
                }
            }
    }

    private var isActive: Bool {
        state.activityCaptureEnabled || state.activeMeetingId != nil
    }

    private var badgeColor: Color? {
        Self.badgeColor(
            active: isActive,
            degraded: !state.permissions.allGranted || state.audio.degradedReason != nil || state.audio.micDegradedReason != nil,
            meetingRecording: state.activeMeetingId != nil,
            activityCapture: state.activityCaptureEnabled
        )
    }

    /// Red for a recording meeting, orange for a degraded state, a neutral dot for
    /// activity capture on its own, nothing when idle. Pure, so the test target can
    /// check the precedence without an `AppState`.
    static func badgeColor(active: Bool, degraded: Bool, meetingRecording: Bool, activityCapture: Bool) -> Color? {
        guard active else { return nil }
        if degraded { return .orange }
        if meetingRecording { return .red }
        return activityCapture ? .secondary : nil
    }
}
