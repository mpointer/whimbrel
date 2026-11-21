import SwiftUI

/// Where a stop sits on the thread.
enum WSpineNode { case upcoming, now, past, recording }

/// One stop on the time line: a time label and node on the left, content beside it. The connector
/// is dashed ahead of now and solid behind it. At accessibility text sizes the time moves above the
/// content so the content keeps the full width.
struct WSpineStop<Content: View>: View {
    @Environment(\.dynamicTypeSize) private var typeSize
    let time: String
    var node: WSpineNode = .upcoming
    var showsNode = true
    @ViewBuilder var content: Content

    private var nodeColor: Color { node == .recording ? WFixed.recording.color : Color.theme(.accentPrimary) }

    var body: some View {
        if typeSize.isAccessibilitySize {
            VStack(alignment: .leading, spacing: WTokens.Space.xs) {
                Text(time).wType(WTokens.TypeScale.time).monospacedDigit().foregroundStyle(Color.theme(.textSecondary))
                content
            }
        } else {
            HStack(alignment: .top, spacing: WTokens.Space.sm) {
                Text(time)
                    .wType(node == .past ? WTokens.TypeScale.caption : WTokens.TypeScale.time)
                    .monospacedDigit()
                    .foregroundStyle(node == .past ? Color.theme(.textSecondary) : Color.theme(.textPrimary))
                    .multilineTextAlignment(.trailing)
                    .frame(width: WTokens.Size.spine - 8, alignment: .trailing)
                    .padding(.top, WTokens.Space.md)
                ZStack(alignment: .top) {
                    connector
                    if showsNode {
                        Circle()
                            .fill(node == .upcoming || node == .past ? Color.theme(.backgroundPage) : nodeColor)
                            .overlay(Circle().strokeBorder(nodeColor, lineWidth: 2))
                            .frame(width: 12, height: 12)
                            .padding(.top, WTokens.Space.md + 2)
                    }
                }
                .frame(width: 12)
                .accessibilityHidden(true)
                content
            }
        }
    }

    @ViewBuilder private var connector: some View {
        if node == .past {
            Capsule().fill(Color.theme(.accentPrimary).opacity(0.5)).frame(width: 4)
        } else {
            Rectangle().fill(.clear).frame(width: 2)
                .overlay { VLine().stroke(Color.theme(.accentPrimary).opacity(0.5), style: StrokeStyle(lineWidth: 2, dash: [4, 4])) }
        }
    }
}

private struct VLine: Shape {
    func path(in rect: CGRect) -> Path {
        var p = Path()
        p.move(to: CGPoint(x: rect.midX, y: rect.minY))
        p.addLine(to: CGPoint(x: rect.midX, y: rect.maxY))
        return p
    }
}

/// The "Now" pill that divides what is coming from what has happened.
struct WNowMarker: View {
    @Environment(\.wStrings) private var strings
    let time: String

    var body: some View {
        Text("\(strings.now) · \(time)")
            .wType(WTokens.TypeScale.time)
            .monospacedDigit()
            .foregroundStyle(Color.theme(.textOnAccent))
            .padding(.horizontal, WTokens.Space.md)
            .padding(.vertical, WTokens.Space.xs)
            .background(Color.theme(.accentPrimary), in: Capsule())
            .frame(maxWidth: .infinity)
            .accessibilityAddTraits(.isHeader)
    }
}

/// A meeting on the thread. `next` is the large card with actions; the others are rows.
struct WMeetingCard<Actions: View>: View {
    enum Variant { case next, upcoming, past }
    let variant: Variant
    let title: String
    /// "Next, in 8 minutes" on the next card.
    var status: String?
    /// "with Dana Whitfield + 3 · in 3 hr 38 min", or a one-line gist for a past meeting.
    var detail: String?
    var attendees: [String] = []
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(alignment: .leading, spacing: variant == .next ? WTokens.Space.sm : WTokens.Space.xxs) {
            if let status {
                Text(status)
                    .wType(WTokens.TypeScale.caption).fontWeight(.semibold)
                    .foregroundStyle(Color.theme(.accentPrimary))
                    .padding(.horizontal, WTokens.Space.sm).padding(.vertical, WTokens.Space.xxs)
                    .background(Color.theme(.highlight), in: Capsule())
            }
            Text(title)
                .wType(variant == .next ? WTokens.TypeScale.title : WTokens.TypeScale.subtitle)
                .foregroundStyle(Color.theme(.textPrimary))
            if let detail {
                Text(detail).wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
            }
            if variant == .next, !attendees.isEmpty {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: WTokens.Space.xs) { ForEach(attendees, id: \.self) { WSpeakerChip(name: $0) } }
                    VStack(alignment: .leading, spacing: WTokens.Space.xs) { ForEach(attendees, id: \.self) { WSpeakerChip(name: $0) } }
                }
            }
            actions
        }
        .padding(variant == .next ? WTokens.Space.lg : WTokens.Space.md)
        .frame(maxWidth: .infinity, minHeight: WTokens.Size.target, alignment: .leading)
        .wCard(level: variant == .next ? WTokens.Elevation.level2 : WTokens.Elevation.level1)
        .accessibilityElement(children: .contain)
    }
}

extension WMeetingCard where Actions == EmptyView {
    init(variant: Variant, title: String, status: String? = nil, detail: String? = nil, attendees: [String] = []) {
        self.init(variant: variant, title: title, status: status, detail: detail, attendees: attendees) { EmptyView() }
    }
}

/// A stretch of work the desktops saw: a time range, the device, and the apps and items in it.
/// Each item has a dot and always its app's name; the dot color is decoration.
struct WMemoryBlock: View {
    struct Item: Identifiable {
        enum Dot { case blue, green, purple, amber }
        let id = UUID()
        let dot: Dot
        let text: String
    }

    @Environment(\.wStrings) private var strings
    let range: String
    var device: String?
    let items: [Item]

    var body: some View {
        VStack(alignment: .leading, spacing: WTokens.Space.xs) {
            Text([range, strings.memory, device].compactMap { $0 }.joined(separator: " · "))
                .wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
            ForEach(items) { item in
                HStack(alignment: .firstTextBaseline, spacing: WTokens.Space.sm) {
                    Circle().fill(color(item.dot)).frame(width: 8, height: 8).accessibilityHidden(true)
                    Text(item.text).wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textPrimary))
                }
            }
        }
        .padding(WTokens.Space.md)
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard()
        .accessibilityElement(children: .combine)
    }

    private func color(_ dot: Item.Dot) -> Color {
        switch dot {
        case .blue: WFixed.appBlue.color
        case .green: WFixed.appGreen.color
        case .purple: WFixed.appPurple.color
        case .amber: WFixed.appAmber.color
        }
    }
}

/// One line of a transcript: a time, an optional speaker (phone recordings have none), the text.
struct WTranscriptLine: View {
    enum LineState { case normal, cited, live }
    @Environment(\.wStrings) private var strings
    let time: String
    var speaker: String?
    let text: String
    var state: LineState = .normal

    var body: some View {
        VStack(alignment: .leading, spacing: WTokens.Space.xs) {
            HStack(spacing: WTokens.Space.sm) {
                Text(time).wType(WTokens.TypeScale.caption).monospacedDigit().foregroundStyle(Color.theme(.textSecondary))
                if let speaker { WSpeakerChip(name: speaker) }
                if state == .cited { Text(strings.cited).wType(WTokens.TypeScale.caption).fontWeight(.semibold).foregroundStyle(WFixed.aiInk.color) }
            }
            Text(text).wType(WTokens.TypeScale.body).foregroundStyle(Color.theme(.textPrimary))
            if state == .live {
                Text(strings.listening).wType(WTokens.TypeScale.caption).foregroundStyle(Color.theme(.textSecondary))
            }
        }
        .padding(WTokens.Space.sm)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(state == .cited ? WFixed.aiTint.color : .clear, in: RoundedRectangle(cornerRadius: WTokens.Radius.control, style: .continuous))
        .accessibilityElement(children: .combine)
    }
}
