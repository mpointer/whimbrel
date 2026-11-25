import SwiftUI
import WhimbrelUI

/// FR-RTN-8: browsable routine output history. Follows `MCPClientsSettingsPane`'s
/// structural pattern (header + caption, `ForEach`-based list of rows).
///
/// Final-review fix: this view's data source is `CoreMemoryStore.recentRoutineRuns`
/// (real, persisted history via `whimbrel_routines_recent_runs`), fetched on `.onAppear`
/// -- not `AppState.routineScheduler.recentRuns`, which is in-memory only and resets to
/// empty on every app relaunch, making a completed-but-unread routine run unreachable
/// through the UI after a relaunch. `routineScheduler.recentRuns` is still watched via
/// `.onChange` purely as a "a new run just completed, go refresh from the real source of
/// truth" signal, so the view still updates live within a session.
struct RoutinesInboxView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.threadLook) private var look
    @Environment(\.wTheme) private var theme
    @State private var expandedRunId: Int64?
    @State private var runs: [CoreRoutineRunWithName] = []
    /// Meeting Prep routines' ids: only their runs (or, from another device, runs named Meeting Prep) get "Open brief".
    @State private var prepRoutineIds: Set<Int64> = []

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Routines")
                .restyledFont(.display)
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text("Output from your scheduled routines, most recent first.")
                .restyledFont(.caption)
                .foregroundStyle(Color.whimbrelTextSecondary)

            Divider().overlay(Color.whimbrelBorder)

            if runs.isEmpty {
                Text("No routine runs yet.")
                    .restyledFont(.caption)
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                ForEach(runs) { run in
                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        Button {
                            expandedRunId = (expandedRunId == run.id) ? nil : run.id
                        } label: {
                            HStack(spacing: Spacing.xs) {
                                Text(formattedDate(run.ranAtUnixMs) + (run.originDeviceId.map { " · from \(appState.deviceLabel(for: $0))" } ?? ""))
                                    .restyledFont(.caption)
                                    .foregroundStyle(Color.whimbrelTextSecondary)
                                Image(systemName: Self.statusSymbol(run.status))
                                    .restyledFont(.caption)
                                    .foregroundStyle(statusInk(run.status))
                                    .accessibilityLabel(Self.statusLabel(run.status))
                                Spacer()
                            }
                        }
                        .buttonStyle(.plain)
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                            Text(run.displaySummary)
                                .restyledFont(.body)
                                .foregroundStyle(Color.whimbrelTextPrimary)
                                .lineLimit(expandedRunId == run.id ? nil : 2)
                            if run.status == "success", prepRoutineIds.contains(run.routineId) || run.routineName == "Meeting Prep", let key = TodayLogic.prepKey(runOutputJson: run.outputJson) {
                                Spacer()
                                Button("Open brief") { appState.showPrepBrief(key: key) }
                                    .restyledButton(.quiet)
                            }
                        }
                    }
                    Divider().overlay(Color.whimbrelBorder)
                }
            }
        }
        .onAppear { refreshRuns() }
        .onChange(of: appState.routineScheduler.recentRuns) { _, _ in refreshRuns() }
        // A sync may have pulled runs another device finished.
        .onChange(of: appState.contentSyncCoordinator.lastSyncedAt) { _, _ in refreshRuns() }
    }

    private func refreshRuns() {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let result = store.recentRoutineRuns(limit: 50)
            let prepIds = Set((store.listRoutines() ?? []).filter { $0.template == "meeting_prep" }.map(\.id))
            DispatchQueue.main.async {
                prepRoutineIds = prepIds
                if let result {
                    runs = result
                }
            }
        }
    }

    /// A run's outcome at a glance. `recent_routine_runs_across_all` (this list's backing
    /// query) returns terminal statuses only, so in practice this is success vs failure —
    /// without it the two render identically, since only `outputSummary` is shown. The
    /// `default` arm exists purely so an unrecognized status can't render as nothing.
    private static func statusSymbol(_ status: String) -> String {
        switch status {
        case "success": "checkmark.circle.fill"
        case "failed": "exclamationmark.triangle.fill"
        default: "circle"
        }
    }

    /// The status glyph's color: the theme's success and error colors with the Thread look, the old accent pair otherwise. The glyph
    /// and its spoken label say the state either way.
    private func statusInk(_ status: String) -> Color {
        let tone: WRunTone = status == "success" ? .success : (status == "failed" ? .failure : .other)
        guard WRestyle.runToneUsesStatusColor(look: look, tone: tone) else { return Self.statusColor(status) }
        return theme.color(tone == .success ? .success : .error)
    }

    private static func statusColor(_ status: String) -> Color {
        switch status {
        case "success": Color.whimbrelAccentSecondary
        case "failed": Color.whimbrelAccentPrimary
        default: Color.whimbrelTextSecondary
        }
    }

    private static func statusLabel(_ status: String) -> String {
        switch status {
        case "success": String(localized: "Completed")
        case "failed": String(localized: "Failed")
        default: status
        }
    }

    private func formattedDate(_ unixMs: Int64) -> String {
        Date(timeIntervalSince1970: Double(unixMs) / 1000).formatted(date: .abbreviated, time: .shortened)
    }
}
