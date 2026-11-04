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
            WhimbrelSectionLabel(text: String(localized: "Needs your decision"))
            HStack {
                Text("\(decisionCount) pending approvals")
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
                title: String(localized: "No calendar connected"),
                message: String(localized: "Connect a calendar to see upcoming meetings here."),
                actionTitle: String(localized: "Connect Calendar"),
                action: { actions.requestCalendar() }
            )
            .frame(maxWidth: .infinity, minHeight: 240)
        case .noMeetings:
            WhimbrelEmptyState(
                systemImage: "mic",
                title: SkinStore.shared.selected.voice.text(.emptyDay, name: ""),
                message: String(localized: "Start a manual note to begin."),
                actionTitle: String(localized: "Start note"),
                action: { actions.startNote() }
            )
            .frame(maxWidth: .infinity, minHeight: 240)
        case .live, .upcoming, .none:
            EmptyView()
        }
    }
}
