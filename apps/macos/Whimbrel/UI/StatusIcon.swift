import SwiftUI

/// FR-CAP-5 / FR-UI-1: capture state is always visible in the menu bar, via the
/// StatusGlyph template image (Assets.xcassets — auto-tints for light/dark menu bar,
/// same as an SF Symbol would) dimmed when idle (neither activity capture nor a
/// meeting is running), plus a small color badge: red specifically while a meeting is
/// actively recording (the more sensitive, attention-worthy state), amber when
/// something's on but degraded or missing a permission it needs. Activity capture
/// running alone (no meeting) gets no badge — it's the app's ambient default state,
/// not something that needs at-a-glance attention.
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
        guard isActive else { return nil }
        if !state.permissions.allGranted || state.audio.degradedReason != nil || state.audio.micDegradedReason != nil { return .orange }
        return state.activeMeetingId != nil ? .red : nil
    }
}
