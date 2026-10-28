import SwiftUI

enum RegenerationConflictChoice {
    case replace
    case keepMine
    case compare
}

/// True only when there's something a fresh regeneration could actually clobber:
/// a draft exists, it has at least one structured override (personal notes alone
/// never conflicts with a generated summary), and the user hasn't already reconciled
/// with this exact generation. Pure and testable — the sole place this decision is made.
func regenerationNeedsConflictPrompt(existingDraft: CoreMeetingNoteDraft?, newGenerationUnixMs: Int64) -> Bool {
    guard let existingDraft else { return false }
    let hasStructuredOverride = existingDraft.summaryEdited != nil
        || existingDraft.decisionsEdited != nil
        || existingDraft.openQuestionsEdited != nil
    guard hasStructuredOverride else { return false }
    return existingDraft.lastRegeneratedSeenUnixMs != newGenerationUnixMs
}

/// Presented when `regenerationNeedsConflictPrompt` is true. "Compare" shows a
/// read-only side-by-side (no merge UI in v0.4.0, per the spec's deferred-items list).
struct RegenerationConflictSheet: View {
    let previousSummary: String?
    let newSummary: String?
    let onChoose: (RegenerationConflictChoice) -> Void

    @State private var showingCompare = false

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("New AI summary generated")
                .font(.whimbrelDisplay(.title3))
            Text("You've edited this meeting's summary. What should happen to your edits?")
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextSecondary)

            if showingCompare {
                compareView
            }

            HStack(spacing: Spacing.md) {
                Button("Keep my edits") { onChoose(.keepMine) }
                    .buttonStyle(.whimbrelSecondary)
                    .accessibilityIdentifier("regeneration-conflict-keep-mine")
                Button("Compare") { showingCompare.toggle() }
                    .buttonStyle(.whimbrelQuiet)
                    .accessibilityIdentifier("regeneration-conflict-compare")
                Button("Replace with new summary") { onChoose(.replace) }
                    .buttonStyle(.whimbrelPrimary)
                    .accessibilityIdentifier("regeneration-conflict-replace")
            }
        }
        .padding(Spacing.xl)
        .frame(minWidth: 420)
    }

    private var compareView: some View {
        HStack(alignment: .top, spacing: Spacing.lg) {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                WhimbrelSectionLabel(text: "Your edited version")
                Text(previousSummary ?? "—").font(.whimbrelBody(.caption))
            }
            VStack(alignment: .leading, spacing: Spacing.xs) {
                WhimbrelSectionLabel(text: "New generated version")
                Text(newSummary ?? "—").font(.whimbrelBody(.caption))
            }
        }
        .padding(Spacing.md)
        .background(Color.whimbrelBackgroundPage)
        .clipShape(RoundedRectangle(cornerRadius: Radius.control))
    }
}
