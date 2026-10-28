import SwiftUI

/// Consolidates the existing RoutinesInboxView + PendingApprovalsView into one
/// outcome-centered surface — every existing field/safeguard is reused verbatim (no new
/// approval mechanism), this is a layout consolidation only. Routine settings (enable/
/// disable, auto-approve, schedule) remain in Settings → Routine Settings
/// (`RoutinesSettingsPane` in `SettingsView.swift`); they are NOT embedded here in
/// v0.4.0, unlike an earlier draft of this comment claimed.
struct AutomationsView: View {
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.xl) {
                PendingApprovalsView()
                Divider().overlay(Color.whimbrelBorder)
                RoutinesInboxView()
            }
            .padding(Spacing.xl)
        }
        .background(Color.whimbrelBackgroundPage)
    }
}
