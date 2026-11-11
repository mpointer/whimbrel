import SwiftUI

/// Today's schedule as a timeline: a line down the leading edge, a card and a dot for each meeting
/// in start order, and a "Now" marker between the ones behind and the ones ahead. Pure layout over
/// `TodayRows.timeline`; clicks go out through `TodayActions`.
struct TodayTimeline: View {
    let items: [TodayTimelineEntry]
    let now: Date
    let content: TodayContent
    let actions: TodayActions
    @Binding var briefExpanded: Bool

    private static let gutter: CGFloat = 20
    private static let dot: CGFloat = 10

    var body: some View {
        let markerIndex = TodayRows.timelineNowIndex(items, now: now)
        VStack(alignment: .leading, spacing: Spacing.md) {
            ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                if index == markerIndex { nowMarker }
                row(item)
            }
            if markerIndex >= items.count { nowMarker }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(alignment: .leading) {
            Rectangle()
                .fill(Color.whimbrelBorder)
                .frame(width: 2)
                .frame(maxHeight: .infinity)
                .padding(.leading, Self.gutter / 2 - 1)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }

    private var nowMarker: some View {
        HStack(spacing: Spacing.sm) {
            Circle()
                .fill(Color.whimbrelAccentPrimary)
                .frame(width: Self.dot, height: Self.dot)
                .frame(width: Self.gutter)
            Text("Now \u{00B7} \(MeetingRowText.time(now))")
                .font(.whimbrelBody(.caption).weight(.semibold))
                .foregroundStyle(Color.whimbrelAccentPrimary)
        }
        .accessibilityElement(children: .combine)
    }

    private func row(_ item: TodayTimelineEntry) -> some View {
        HStack(alignment: .top, spacing: Spacing.sm) {
            dot(item.model)
            VStack(alignment: .leading, spacing: Spacing.sm) {
                MeetingCard(model: item.model, onOpen: { open(item.model.target) }, onChip: { chip in tap(chip, item) })
                extras(item)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Upcoming: an accent ring. Running: a filled accent dot. Past: a filled secondary dot.
    /// The page fill hides the line behind a ring.
    private func dot(_ model: MeetingCardModel) -> some View {
        Group {
            if model.isRunning {
                Circle().fill(Color.whimbrelAccentPrimary)
            } else if model.isPast {
                Circle().fill(Color.whimbrelTextSecondary)
            } else {
                Circle().fill(Color.whimbrelBackgroundPage)
                    .overlay(Circle().stroke(Color.whimbrelAccentPrimary, lineWidth: 2))
            }
        }
        .frame(width: Self.dot, height: Self.dot)
        .frame(width: Self.gutter)
        .padding(.top, Spacing.lg * SkinStore.shared.selected.shape.density.scale)
        .accessibilityHidden(true)
    }

    /// What sits under a card: Open for a held meeting; Start, and the brief toggle for the next event.
    @ViewBuilder
    private func extras(_ item: TodayTimelineEntry) -> some View {
        switch item.kind {
        case let .held(id):
            HStack(spacing: Spacing.sm) {
                if content.processingMeetingId == id {
                    Text("Transcribing…")
                        .font(.whimbrelBody(.subheadline))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
                Spacer(minLength: Spacing.md)
                Button("Open") { actions.openMeeting(id) }
                    .buttonStyle(.whimbrelQuiet)
            }
        case let .event(event):
            let showsStart = content.activeMeetingId == nil
            if item.isNext || showsStart {
                HStack(alignment: .top, spacing: Spacing.sm) {
                    if item.isNext { briefToggle(event, hasPrep: item.model.chips.contains { if case .prep = $0 { return true } else { return false } }) }
                    Spacer(minLength: Spacing.md)
                    if showsStart {
                        Button("Start") { actions.startMeeting(event) }
                            .buttonStyle(.whimbrelPrimary)
                    }
                }
            }
            if item.isNext, briefExpanded, let prep = nextBrief(for: event) { MeetingPrepView(prep: prep) }
        }
    }

    /// The next event's Open brief / Hide brief toggle, or "No prep yet" (the chip itself is on the card).
    @ViewBuilder
    private func briefToggle(_ event: TodayUpcoming, hasPrep: Bool) -> some View {
        if hasPrep {
            Button(briefExpanded ? "Hide brief" : "Open brief") { briefExpanded.toggle() }
                .buttonStyle(.whimbrelQuiet)
        } else {
            Text("No prep yet")
                .font(.whimbrelBody(.subheadline))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    private func nextBrief(for event: TodayUpcoming) -> CoreMeetingPrep? {
        content.prep(eventId: event.id, start: event.start)
            ?? content.nextPrep.flatMap { TodayLogic.prep($0, isFor: event) && $0.brief != nil ? $0 : nil }
    }

    private func open(_ target: MeetingCardTarget) {
        switch target {
        case let .meeting(id): actions.openMeeting(id)
        case let .prepBrief(key): actions.showPrepBrief(key)
        case let .topic(id): actions.openTopic(id)
        case .none: break
        }
    }

    private func tap(_ chip: MeetingChip, _ item: TodayTimelineEntry) {
        switch chip {
        case let .prep(key): actions.showPrepBrief(key)
        case let .topic(id, _): actions.openTopic(id)
        case .summary, .actionItems:
            if case let .held(id) = item.kind { actions.openMeeting(id) }
        }
    }
}

/// The "Prep ready" chip for a meeting row in the other layouts: the shared `MeetingChips`, routed
/// through `TodayActions` like the timeline's chips.
struct TodayPrepChips: View {
    let prep: CoreMeetingPrep
    let actions: TodayActions

    var body: some View {
        MeetingChips(chips: [.prep(key: prep.eventId)]) { chip in
            if case let .prep(key) = chip { actions.showPrepBrief(key) }
        }
    }
}
