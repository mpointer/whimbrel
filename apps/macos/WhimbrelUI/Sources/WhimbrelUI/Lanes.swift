import SwiftUI

/// One lane per speaker above the transcript, with the marked moments as ticks. Clicking a span opens its line.
/// The lanes are one stop for the keyboard: left and right move along in time, up and down change lane, Return opens
/// the line. A lane is drawn in one Canvas, not one control per span, because a long meeting has thousands of spans.
/// For a screen reader each lane is one adjustable element: its name is the speaker, its value the time of the span it
/// is on, swiping up or down steps to the next or previous span of that speaker, and its default action opens the
/// line. A continued meeting has one group per part.
public struct WSpeakerLanes: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let groups: [WLaneGroup]
    let highlightedId: String?
    let onOpenSpan: (String) -> Void
    let onOpenMark: (String) -> Void
    @State private var current: String?
    /// The span a screen reader is on, per lane.
    @State private var cursors: [String: String] = [:]
    @FocusState private var isFocused: Bool

    public init(groups: [WLaneGroup], highlightedId: String? = nil, onOpenSpan: @escaping (String) -> Void, onOpenMark: @escaping (String) -> Void) {
        self.groups = groups
        self.highlightedId = highlightedId
        self.onOpenSpan = onOpenSpan
        self.onOpenMark = onOpenMark
    }

    private static let labelWidth: CGFloat = 112
    private var laneHeight: CGFloat { 16 * k }

    private func color(_ lane: Int) -> Color {
        switch lane % 5 {
        case 0: return theme.accentPrimary
        case 1: return theme.appColor(.blue)
        case 2: return theme.appColor(.purple)
        case 3: return theme.appColor(.green)
        default: return theme.appColor(.amber)
        }
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            ForEach(groups) { group in groupView(group) }
        }
        .padding(theme.space(12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard(radius: .card)
        .overlay {
            if isFocused {
                RoundedRectangle(cornerRadius: theme.shape.cardRadius + 2, style: .continuous)
                    .strokeBorder(theme.focus, lineWidth: 2.5).padding(-2).allowsHitTesting(false).accessibilityHidden(true)
            }
        }
        .focusable()
        .focused($isFocused)
        .focusEffectDisabled()
        .onChange(of: isFocused) { _, focused in
            if focused, current == nil { current = highlightedId ?? WLaneMath.timeOrdered(groups).first?.id }
        }
        .onKeyPress(.leftArrow) { move(WLaneMath.neighbor(of: current, by: -1, in: groups)) }
        .onKeyPress(.rightArrow) { move(WLaneMath.neighbor(of: current, by: 1, in: groups)) }
        .onKeyPress(.upArrow) { move(WLaneMath.otherLane(from: current, by: -1, in: groups)) }
        .onKeyPress(.downArrow) { move(WLaneMath.otherLane(from: current, by: 1, in: groups)) }
        .onKeyPress(.return) {
            guard let current else { return .ignored }
            onOpenSpan(current)
            return .handled
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.note.speakerLanes)
        .accessibilityHint(strings.note.lanesHint)
    }

    private func move(_ id: String?) -> KeyPress.Result {
        guard let id else { return .ignored }
        current = id
        return .handled
    }

    private func groupView(_ group: WLaneGroup) -> some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            if let divider = group.divider {
                Text(divider).wFont(.caption).foregroundStyle(theme.textSecondary).frame(maxWidth: .infinity)
            }
            if !group.ticks.isEmpty {
                HStack(spacing: theme.space(8)) {
                    Color.clear.frame(width: Self.labelWidth * k, height: 1)
                    GeometryReader { geo in
                        ForEach(group.ticks) { mark in
                            let x = position(mark.atCs, in: group, width: geo.size.width)
                            Button { onOpenMark(mark.id) } label: {
                                Capsule().fill(theme.accentHighlight).frame(width: 3 * k, height: 12 * k)
                                    .frame(width: 14 * k, height: 14 * k).contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .focusable(false)
                            .accessibilityLabel(strings.joinDetails([strings.live.momentAt(mark.clock), mark.label].filter { !$0.isEmpty }))
                            .accessibilityHint(strings.note.openInTranscriptAt(mark.clock))
                            .position(x: x, y: 7 * k)
                        }
                    }
                    .frame(height: 14 * k)
                }
            }
            ForEach(Array(group.lanes.enumerated()), id: \.element.id) { laneIndex, lane in
                HStack(spacing: theme.space(8)) {
                    Text(lane.speaker).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1)
                        .frame(width: Self.labelWidth * k, alignment: .leading)
                        .accessibilityHidden(true)
                    track(lane, index: laneIndex, in: group)
                }
            }
            HStack(spacing: theme.space(8)) {
                Color.clear.frame(width: Self.labelWidth * k, height: 1)
                Text(WLaneMath.clock(cs: group.startCs)).wFont(.caption).foregroundStyle(theme.textSecondary)
                Spacer(minLength: 0)
                Text(WLaneMath.clock(cs: group.endCs)).wFont(.caption).foregroundStyle(theme.textSecondary)
            }
            .accessibilityHidden(true)
        }
    }

    private func position(_ cs: Int64, in group: WLaneGroup, width: CGFloat) -> CGFloat {
        let range = max(1, group.endCs - group.startCs)
        return min(max(0, CGFloat(cs - group.startCs) / CGFloat(range)), 1) * width
    }

    private func tap(at x: CGFloat, width: CGFloat, lane: WLane, in group: WLaneGroup) {
        guard width > 0 else { return }
        let range = Double(max(1, group.endCs - group.startCs))
        let cs = group.startCs + Int64(Double(x / width) * range)
        let tolerance = Int64(Double(4 * k / width) * range)
        guard let hit = WLaneMath.span(at: cs, in: lane, toleranceCs: tolerance) else { return }
        current = hit.id
        onOpenSpan(hit.id)
    }

    private func draw(_ lane: WLane, index: Int, in group: WLaneGroup, context: GraphicsContext, size: CGSize) {
        let h = size.height
        let trackColor = theme.border.opacity(theme.isHighContrast ? 0.5 : 0.3)
        context.fill(Path(roundedRect: CGRect(x: 0, y: h / 2 - 1, width: size.width, height: 2), cornerRadius: 1), with: .color(trackColor))
        func rect(_ span: WLaneSpan) -> CGRect {
            let x = position(span.startCs, in: group, width: size.width)
            let w = max(4, position(span.endCs, in: group, width: size.width) - x)
            return CGRect(x: min(x, max(0, size.width - w)), y: 0, width: w, height: h)
        }
        var all = Path()
        for span in lane.spans { all.addRoundedRect(in: rect(span), cornerSize: CGSize(width: 3, height: 3)) }
        context.fill(all, with: .color(color(index)))
        if let id = highlightedId, let span = lane.spans.first(where: { $0.id == id }) {
            let outline = Path(roundedRect: rect(span).insetBy(dx: 1, dy: 1), cornerRadius: 2)
            context.stroke(outline, with: .color(theme.textPrimary), lineWidth: theme.isHighContrast ? 2.5 : 2)
        }
        if isFocused, let id = current, let span = lane.spans.first(where: { $0.id == id }) {
            let outline = Path(roundedRect: rect(span).insetBy(dx: 1, dy: 1), cornerRadius: 2)
            context.stroke(outline, with: .color(theme.focus), lineWidth: theme.isHighContrast ? 2.5 : 2)
        }
    }

    private func spoken(_ lane: WLane) -> String {
        guard let id = cursors[lane.id] ?? lane.spans.first?.id, let span = lane.spans.first(where: { $0.id == id }) else { return "" }
        return span.time
    }

    private func track(_ lane: WLane, index: Int, in group: WLaneGroup) -> some View {
        GeometryReader { geo in
            Canvas { context, size in draw(lane, index: index, in: group, context: context, size: size) }
                .contentShape(Rectangle())
                .gesture(SpatialTapGesture().onEnded { tap(at: $0.location.x, width: geo.size.width, lane: lane, in: group) })
        }
        .frame(height: laneHeight)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(lane.speaker)
        .accessibilityValue(spoken(lane))
        .accessibilityAdjustableAction { direction in
            let delta = direction == .increment ? 1 : -1
            if let next = WLaneMath.step(in: lane, from: cursors[lane.id], by: delta) { cursors[lane.id] = next }
        }
        .accessibilityAction(.default) {
            if let id = cursors[lane.id] ?? lane.spans.first?.id { onOpenSpan(id) }
        }
    }
}
