import SwiftUI

/// Wraps its children onto new lines when they don't fit, so chips never push the chevron off-screen.
private struct WrappingChipLayout: Layout {
    var spacing: CGFloat = Spacing.sm

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(width: proposal.width ?? .infinity, subviews: subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let result = arrange(width: bounds.width, subviews: subviews)
        for (index, origin) in result.origins.enumerated() {
            subviews[index].place(
                at: CGPoint(x: bounds.minX + origin.x, y: bounds.minY + origin.y),
                proposal: ProposedViewSize(width: min(result.sizes[index].width, bounds.width), height: nil))
        }
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> (size: CGSize, origins: [CGPoint], sizes: [CGSize]) {
        var origins: [CGPoint] = []
        var sizes: [CGSize] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        var maxX: CGFloat = 0
        for subview in subviews {
            var size = subview.sizeThatFits(ProposedViewSize(width: width.isFinite ? width : nil, height: nil))
            if width.isFinite { size.width = min(size.width, width) }
            sizes.append(size)
            if x > 0, x + size.width > width {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            origins.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
            maxX = max(maxX, x - spacing)
        }
        return (CGSize(width: maxX, height: y + rowHeight), origins, sizes)
    }
}

// MARK: - Copy

enum MeetingCardText {
    static func people(_ n: Int) -> String {
        n == 1 ? String(localized: "1 person") : String(localized: "\(n) people")
    }

    static func actionItems(_ n: Int) -> String {
        n == 1 ? String(localized: "1 action item") : String(localized: "\(n) action items")
    }

    static func duration(_ minutes: Int) -> String {
        String(localized: "\(minutes) min")
    }
}

// MARK: - Chips

/// A meeting's chips in a wrapping row. Each chip is its own button, so a tap on it doesn't also
/// open the card underneath.
struct MeetingChips: View {
    let chips: [MeetingChip]
    let onTap: (MeetingChip) -> Void

    var body: some View {
        if !chips.isEmpty {
            WrappingChipLayout {
                ForEach(chips, id: \.self) { chip in
                    chipView(chip)
                }
            }
        }
    }

    @ViewBuilder
    private func chipView(_ chip: MeetingChip) -> some View {
        switch chip {
        case let .prep(key):
            PrepReadyChip(prepKey: key, onTap: { onTap(chip) })
        case .summary:
            metadataButton(chip, systemImage: "text.alignleft", text: String(localized: "Summary"))
        case let .actionItems(count):
            metadataButton(chip, systemImage: "checklist", text: MeetingCardText.actionItems(count))
        case let .topic(id, name):
            TopicChip(topic: TopicRef(topicId: id, key: "", name: name), onTap: { onTap(chip) })
        }
    }

    private func metadataButton(_ chip: MeetingChip, systemImage: String, text: String) -> some View {
        Button { onTap(chip) } label: {
            WhimbrelMetadataChip(systemImage: systemImage, text: text)
        }
        .buttonStyle(.plain)
    }
}

// MARK: - Shared shell

/// The card/row hit target: a plain button behind the whole padded area, with the chips laid over
/// it as their own buttons. Hover highlight and chevron show only when the meeting goes somewhere.
private struct MeetingItemShell<Content: View>: View {
    let model: MeetingCardModel
    let cornerRadius: CGFloat
    let padding: CGFloat
    /// Draw the card chrome (fill and border) behind everything.
    let isCard: Bool
    let onOpen: () -> Void
    @ViewBuilder var content: Content
    @State private var isHovered = false

    private var isClickable: Bool { model.target != .none }
    private var spokenLabel: String { model.spokenLabel }

    var body: some View {
        HStack(alignment: .center, spacing: Spacing.md) {
            content
                .frame(maxWidth: .infinity, alignment: .leading)
            if isClickable {
                Image(systemName: "chevron.right")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
        }
        .padding(padding)
        .background { hitTarget }
        .background {
            if isClickable, isHovered {
                RoundedRectangle(cornerRadius: cornerRadius)
                    .fill(Color.whimbrelTextPrimary.opacity(0.06))
                    .allowsHitTesting(false)
            }
        }
        .background {
            if isCard { Color.clear.whimbrelCard().allowsHitTesting(false) }
        }
        .onHover { hovering in if isClickable { isHovered = hovering } }
    }

    /// One accessibility element (title and time) that is the click target when there is one.
    /// The chips stay separate, focusable buttons.
    @ViewBuilder
    private var hitTarget: some View {
        if isClickable {
            Button(action: onOpen) {
                Color.clear.contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(spokenLabel)
        } else {
            Color.clear.contentShape(Rectangle())
                .accessibilityElement()
                .accessibilityLabel(spokenLabel)
        }
    }
}

// MARK: - Card (Today)

/// A Today timeline card: title, "start to end - N people", chips, chevron.
struct MeetingCard: View {
    let model: MeetingCardModel
    let onOpen: () -> Void
    let onChip: (MeetingChip) -> Void

    var body: some View {
        MeetingItemShell(
            model: model, cornerRadius: SkinStore.shared.selected.shape.radius,
            padding: Spacing.lg * SkinStore.shared.selected.shape.density.scale,
            isCard: true, onOpen: onOpen
        ) {
            VStack(alignment: .leading, spacing: Spacing.sm) {
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    Text(model.title)
                        .font(.whimbrelBody(.body).weight(.medium))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(subtitle)
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                        .lineLimit(1)
                    if let line = model.statusLine {
                        Text(line)
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                            .lineLimit(2)
                    }
                }
                .allowsHitTesting(false)
                .accessibilityHidden(true)
                MeetingChips(chips: model.chips, onTap: onChip)
            }
        }
    }

    private var subtitle: String {
        var parts = [model.timeRange]
        if let people = model.peopleCount { parts.append(MeetingCardText.people(people)) }
        if let hint = model.hint { parts.append(hint) }
        return parts.joined(separator: " \u{00B7} ")
    }
}

// MARK: - Row (Meetings tab)

/// A compact Meetings row: 54pt time column (start, duration under it), title, people, chips, chevron.
/// The whole row, time column included, is the click target.
struct MeetingListRow: View {
    let model: MeetingCardModel
    let onOpen: () -> Void
    let onChip: (MeetingChip) -> Void

    var body: some View {
        MeetingItemShell(model: model, cornerRadius: Radius.control, padding: Spacing.sm, isCard: false, onOpen: onOpen) {
            HStack(alignment: .top, spacing: Spacing.md) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(model.startText)
                        .font(.whimbrelBody(.caption).weight(.medium))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                    if let minutes = model.durationMinutes {
                        Text(MeetingCardText.duration(minutes))
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                    }
                }
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .frame(width: 54, alignment: .leading)
                .allowsHitTesting(false)
                .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: Spacing.sm) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(model.title)
                            .font(.whimbrelBody(.body))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                            .lineLimit(2)
                            .multilineTextAlignment(.leading)
                            .fixedSize(horizontal: false, vertical: true)
                        if let people = model.peopleCount {
                            Text(MeetingCardText.people(people))
                                .font(.whimbrelBody(.caption))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                        }
                        if let line = model.statusLine {
                            Text(line)
                                .font(.whimbrelBody(.caption))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                                .lineLimit(2)
                        }
                    }
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
                    MeetingChips(chips: model.chips, onTap: onChip)
                }
            }
        }
        .overlay(alignment: .bottom) {
            Rectangle().fill(Color.whimbrelBorder).frame(height: 1).allowsHitTesting(false)
        }
    }
}

// MARK: - Previews

private enum MeetingCardPreviewData {
    static let past = MeetingCardModel(
        title: "Quarterly planning with the whole platform team and a very long agenda title that wraps",
        timeRange: "9:00 AM to 9:45 AM", durationMinutes: 45, peopleCount: 6, isPast: true,
        chips: [.summary, .actionItems(count: 3), .topic(id: 1, name: "Planning")],
        target: .meeting(id: 1), startText: "9:00 AM")
    static let upcoming = MeetingCardModel(
        title: "Design review", timeRange: "2:00 PM to 2:30 PM", durationMinutes: 30, peopleCount: 1, isPast: false,
        chips: [.prep(key: "evt-1"), .topic(id: 2, name: "Design")],
        target: .prepBrief(key: "evt-1"), startText: "2:00 PM")
    static let inert = MeetingCardModel(
        title: "Lunch", timeRange: "12:00 PM", durationMinutes: nil, peopleCount: nil, isPast: false,
        chips: [], target: .none, startText: "12:00 PM")
}

#Preview("Cards") {
    VStack(spacing: Spacing.md) {
        MeetingCard(model: MeetingCardPreviewData.past, onOpen: {}, onChip: { _ in })
        MeetingCard(model: MeetingCardPreviewData.upcoming, onOpen: {}, onChip: { _ in })
        MeetingCard(model: MeetingCardPreviewData.inert, onOpen: {}, onChip: { _ in })
    }
    .padding(Spacing.lg)
    .frame(width: 360)
    .background(Color.whimbrelBackgroundPage)
}

#Preview("Rows") {
    VStack(spacing: 0) {
        MeetingListRow(model: MeetingCardPreviewData.past, onOpen: {}, onChip: { _ in })
        MeetingListRow(model: MeetingCardPreviewData.upcoming, onOpen: {}, onChip: { _ in })
        MeetingListRow(model: MeetingCardPreviewData.inert, onOpen: {}, onChip: { _ in })
    }
    .padding(Spacing.lg)
    .frame(width: 420)
    .background(Color.whimbrelBackgroundPage)
}
