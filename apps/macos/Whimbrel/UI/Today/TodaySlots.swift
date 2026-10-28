import SwiftUI

/// The parts of Today every layout must keep as Whimbrel renders them: the live meeting pane,
/// "Needs your decision" and the Ask composer. `TodayView` builds the live pane (it owns the
/// meeting state) and the composer (it needs `AppState`); a layout only places them.
struct TodaySlots: View {
    let live: AnyView?
    let decisionCount: Int
    let actions: TodayActions
    var composer: AnyView?

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xl) {
            liveSlot
            if decisionCount > 0 { decisionSlot }
        }
    }

    /// The live call, or a spinner while its draft loads.
    @ViewBuilder
    var liveSlot: some View {
        if let live {
            live
        } else {
            ProgressView().frame(maxWidth: .infinity, minHeight: 120)
        }
    }

    var decisionSlot: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: "Needs your decision")
            HStack {
                Text("\(decisionCount) pending approval\(decisionCount == 1 ? "" : "s")")
                    .font(.whimbrelBody(.body))
                Spacer()
                Button("Review") { actions.reviewApprovals() }
                    .buttonStyle(.whimbrelSecondary)
            }
        }
    }

    /// The empty state for a day with no calendar or no meetings; nothing for any other hero.
    @ViewBuilder
    func emptyState(for hero: TodayHero) -> some View {
        switch hero {
        case .noCalendar:
            WhimbrelEmptyState(
                systemImage: "calendar.badge.exclamationmark",
                title: "No calendar connected",
                message: "Connect a calendar in Settings to see upcoming meetings here.",
                actionTitle: "Open Settings",
                action: { actions.openSettings() }
            )
            .frame(maxWidth: .infinity, minHeight: 240)
        case .noMeetings:
            WhimbrelEmptyState(
                systemImage: "mic",
                title: SkinStore.shared.selected.voice.text(.emptyDay, name: ""),
                message: "Start a manual note to begin.",
                actionTitle: "Start note",
                action: { actions.startNote() }
            )
            .frame(maxWidth: .infinity, minHeight: 240)
        case .live, .upcoming, .none:
            EmptyView()
        }
    }
}
