import SwiftUI

/// Mirrors `RunStatus` (core/memory) exactly, plus `.unavailable` for capability that
/// doesn't exist yet on this build (e.g. no connector connected) — `.unavailable` is
/// also the safe fallback for a `nil` run or an unrecognized status string, so a future
/// server-side status never crashes the UI, it just reads as "not actionable right now".
enum ActionProposalState: Equatable {
    case unavailable
    case pendingApproval
    case inFlight
    case completed
    case dismissed
    case failed
}

/// Maps a fetched `CoreRoutineRun?` to display state. Pure and testable — the ONLY
/// place this mapping happens.
func deriveActionProposalState(run: CoreRoutineRun?) -> ActionProposalState {
    guard let run else { return .unavailable }
    switch run.status {
    case "pending_approval": return .pendingApproval
    case "executing": return .inFlight
    case "success": return .completed
    case "rejected": return .dismissed
    case "failed": return .failed
    default: return .unavailable
    }
}

/// Everything an `ActionProposalCard` needs to render one proposal honestly — title,
/// why it's proposed, its local sources, and its external destination (if any). Not
/// itself an executable action; `onApprove`/`onDismiss` are wired by the caller
/// directly to `RoutineScheduler.executeWriteAction`/`store.rejectRoutineRun`
/// (see `PendingApprovalsView`, this card's direct template) — this type never writes.
struct ActionProposal {
    let title: String
    let reason: String
    let localSources: [String]
    let externalDestination: String?
    let run: CoreRoutineRun?
    let unavailableReason: String?
}

/// Reusable across meeting action items today and routine/connector proposals later.
/// Never causes an external write itself — every write-capable button routes through
/// the caller's closures, which the caller wires to the EXISTING
/// `RoutineScheduler.executeWriteAction`/`PendingApprovalsView.reject` mechanism.
struct ActionProposalCard: View {
    let proposal: ActionProposal
    var onApprove: (() -> Void)?
    var onDismiss: (() -> Void)?
    var onReview: (() -> Void)?

    private var state: ActionProposalState { deriveActionProposalState(run: proposal.run) }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            HStack {
                Text(proposal.title)
                    .font(.whimbrelBody(.body).weight(.medium))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                Spacer()
                statusChip
            }
            Text(proposal.reason)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
            if !proposal.localSources.isEmpty {
                HStack(spacing: Spacing.xs) {
                    ForEach(proposal.localSources, id: \.self) { source in
                        WhimbrelSourceChip(systemImage: "doc.text", text: source, onTap: nil)
                    }
                }
            }
            if let destination = proposal.externalDestination {
                WhimbrelMetadataChip(systemImage: "arrow.up.right", text: "To: \(destination)")
            }
            actions
        }
        .padding(Spacing.lg)
        .whimbrelCard()
        .accessibilityIdentifier("action-proposal-card")
    }

    @ViewBuilder
    private var statusChip: some View {
        switch state {
        case .unavailable:
            WhimbrelMetadataChip(text: proposal.unavailableReason ?? "Not available yet")
        case .pendingApproval:
            WhimbrelMetadataChip(systemImage: "clock", text: "Needs approval")
        case .inFlight:
            WhimbrelMetadataChip(systemImage: "arrow.triangle.2.circlepath", text: "In progress")
        case .completed:
            WhimbrelMetadataChip(systemImage: "checkmark", text: "Completed")
        case .dismissed:
            WhimbrelMetadataChip(text: "Dismissed")
        case .failed:
            WhimbrelMetadataChip(systemImage: "exclamationmark.triangle", text: "Failed")
        }
    }

    @ViewBuilder
    private var actions: some View {
        switch state {
        case .pendingApproval:
            HStack {
                if let onReview {
                    Button("Review", action: onReview).buttonStyle(.whimbrelQuiet)
                }
                if let onApprove {
                    Button("Approve", action: onApprove).buttonStyle(.whimbrelPrimary)
                }
                if let onDismiss {
                    Button("Dismiss", action: onDismiss).buttonStyle(.whimbrelSecondary)
                }
            }
        case .unavailable, .inFlight, .completed, .dismissed, .failed:
            EmptyView()
        }
    }
}
