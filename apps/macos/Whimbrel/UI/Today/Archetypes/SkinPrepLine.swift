import SwiftUI

/// An event's prep state in a skin's type: "No prep yet" (next event only), or the "Prep ready" chip with Open brief /
/// Hide brief and the brief itself when expanded. Terminal, Glass and Field share it; the decision
/// (`prep`) comes from `TodayRows`.
struct SkinPrepLine: View {
    let prep: TodayEventRowModel.Prep?
    @State private var expanded = false

    var body: some View {
        switch prep {
        case let .ready(prep):
            HStack(spacing: Spacing.sm) {
                PrepReadyChip(prepKey: prep.eventId)
                Button(expanded ? "Hide brief" : "Open brief") { expanded.toggle() }.buttonStyle(.whimbrelQuiet)
            }
            if expanded { MeetingPrepView(prep: prep) }
        case .missing:
            SkinText("No prep yet", role: .label).foregroundStyle(Color.whimbrelTextSecondary)
        case nil:
            EmptyView()
        }
    }
}
