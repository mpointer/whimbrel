import SwiftUI

/// Which of the skin's four typography faces a `SkinText` uses.
enum SkinTextRole {
    case display, body, numerals, label

    var face: SkinFace {
        let typography = SkinStore.shared.selected.typography
        switch self {
        case .display: return typography.display
        case .body: return typography.body
        case .numerals: return typography.numerals
        case .label: return typography.label
        }
    }

    /// Default point size; a caller can pass its own.
    var size: CGFloat {
        switch self {
        case .display: return 22
        case .body: return 14
        case .numerals: return 13
        case .label: return 11
        }
    }
}

/// Text in one of the skin's faces. Every archetype draws its voice strings and content titles with
/// this, so a line that does not fit is cut with an ellipsis and its full text is in the tooltip.
/// Colour comes from the caller (`.foregroundStyle`).
struct SkinText: View {
    let text: String
    let role: SkinTextRole
    let lines: Int
    let size: CGFloat?
    /// Small-cap glyphs (the skin's `caps: small`); the text itself is unchanged.
    let smallCaps: Bool

    init(_ s: String, role: SkinTextRole = .body, lines: Int = 1, size: CGFloat? = nil, smallCaps: Bool = false) {
        text = s
        self.role = role
        self.lines = lines
        self.size = size
        self.smallCaps = smallCaps
    }

    var body: some View {
        Text(text)
            .font(smallCaps ? role.face.font(size: size ?? role.size).smallCaps() : role.face.font(size: size ?? role.size))
            .lineLimit(lines)
            .truncationMode(.tail)
            .fixedSize(horizontal: false, vertical: true)
            .help(text)
    }
}

/// A decorative block cursor. It blinks only for a `subtle` skin with Reduce Motion off.
struct TerminalCursor: View {
    let motion: SkinMotion
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    static func animates(motion: SkinMotion, reduceMotion: Bool) -> Bool {
        motion == .subtle && !reduceMotion
    }

    private static let period = 0.55

    var body: some View {
        let block = Rectangle().fill(Color.whimbrelAccentPrimary).frame(width: 9, height: 18)
        Group {
            if Self.animates(motion: motion, reduceMotion: reduceMotion) {
                SwiftUI.TimelineView(.periodic(from: .now, by: Self.period)) { context in
                    block.opacity(Int(context.date.timeIntervalSinceReferenceDate / Self.period) % 2 == 0 ? 1 : 0.15)
                }
            } else {
                block
            }
        }
        .accessibilityHidden(true)
    }
}

/// One mono column: `[ SECTION ]` labels, dotted rules, and an Ask framed as a shell prompt
/// (`>`, a block cursor, then the real composer). Places every section; the hero empty states and
/// the native slots are `TodayChrome`'s.
struct TerminalTodayLayout: TodayArchetypeView {
    let content: TodayContent
    let actions: TodayActions
    let composer: AnyView?

    init(content: TodayContent, actions: TodayActions, composer: AnyView?) {
        self.content = content
        self.actions = actions
        self.composer = composer
    }

    static var placedSections: Set<TodaySection> { Set(TodaySection.allCases).subtracting([.followups]) }  // follow-ups are TodayChrome's leftover

    private var skin: Skin { SkinStore.shared.selected }
    private var voice: SkinVoice { skin.voice }
    private let timeWidth: CGFloat = 76

    var body: some View {
        let sections = content.nonEmptySections()
        VStack(alignment: .leading, spacing: Spacing.lg) {
            header.skinBadge()
            if sections.contains(.composer), let composer { prompt(composer) }
            if sections.contains(.upNext), let next = TodayRows.upNext(content) {
                rule
                section(voice.text(.upNextHeader, name: "")) { eventRow(next) }
            }
            if sections.contains(.schedule) {
                // `.upNext` is placed above and shows `upcomingToday.first`, so the schedule lists the rest.
                let schedule = TodayRows.schedule(content, upNextPlaced: true)
                if !schedule.upcoming.isEmpty || !schedule.held.isEmpty {
                    rule
                    section(voice.text(.upcomingHeader, name: "")) {
                        ForEach(TodayRows.scheduleItems(schedule)) { item in
                            switch item {
                            case let .event(event): eventRow(event)
                            case .separator: SkinText(TodayRows.heldSeparator, role: .label).foregroundStyle(Color.whimbrelTextSecondary)
                            case let .held(meeting): meetingRow(meeting)
                            }
                        }
                    }
                }
            }
            if sections.contains(.earlier) {
                rule
                section(voice.text(.earlierHeader, name: "")) {
                    ForEach(TodayRows.earlier(content), id: \.id) { meetingRow($0) }
                }
            }
            if sections.contains(.threads) {
                rule
                section("Open loops") {
                    ForEach(content.attentionThreads) { thread in
                        Button { actions.openThread(thread.id) } label: {
                            HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                                Text(">").font(skin.typography.body.font(size: 14)).foregroundStyle(Color.whimbrelAccentPrimary).accessibilityHidden(true)
                                SkinText(thread.subject ?? thread.lastMessageFrom, lines: 2)
                                    .foregroundStyle(Color.whimbrelTextPrimary)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            if sections.contains(.recap) {
                rule
                section(voice.text(.recapHeader, name: "")) { recap }
            }
            rule
            devicesFooter
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: pieces

    /// A date under the greeting; with no usable profile name the date is the title.
    private var header: some View {
        let dateLine = content.now.formatted(.dateTime.weekday(.wide).month(.wide).day())
        let greeting = TodayLogic.greeting(name: content.name, now: content.now, calendar: .current, voice: voice)
        return VStack(alignment: .leading, spacing: Spacing.xs) {
            if greeting != nil {
                SkinText("# \(dateLine)", role: .label, size: 12).foregroundStyle(Color.whimbrelTextSecondary)
            }
            SkinText(greeting ?? dateLine, role: .display, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
        }
    }

    private func prompt(_ composer: AnyView) -> some View {
        HStack(spacing: Spacing.sm) {
            Text(">").font(skin.typography.display.font(size: 20)).foregroundStyle(Color.whimbrelAccentPrimary).accessibilityHidden(true)
            TerminalCursor(motion: skin.motion)
            composer
        }
    }

    private func section<C: View>(_ title: String, @ViewBuilder _ rows: () -> C) -> some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            SkinText("[ \(skin.typography.caps.apply(title)) ]", role: .label, size: 12)
                .foregroundStyle(Color.whimbrelAccentPrimary)
            rows()
        }
    }

    @ViewBuilder
    private var rule: some View {
        if skin.decoration.dividers != .none {
            DottedRule().stroke(Color.whimbrelBorder, style: StrokeStyle(lineWidth: 1, dash: [2, 4])).frame(height: 1)
        }
    }

    private func eventRow(_ event: TodayEventRowModel) -> some View {
        HStack(alignment: .top, spacing: Spacing.md) {
            SkinText(event.timeText, role: .numerals)
                .foregroundStyle(Color.whimbrelAccentPrimary)
                .frame(width: timeWidth, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                SkinText(event.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
                if let relative = event.relative { SkinText(relative, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
                if !event.attendees.isEmpty { SkinText(event.attendees, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
                SkinPrepLine(prep: event.prep)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if event.showsStart {
                Button("Start") { actions.startMeeting(event.event) }.buttonStyle(.whimbrelPrimary)
            }
        }
    }

    private func meetingRow(_ meeting: TodayMeetingRowModel) -> some View {
        HStack(alignment: .top, spacing: Spacing.md) {
            SkinText(meeting.timeText, role: .numerals)
                .foregroundStyle(Color.whimbrelTextSecondary)
                .frame(width: timeWidth, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                SkinText(meeting.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
                if !meeting.subtitle.isEmpty { SkinText(meeting.subtitle, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button("Open") { actions.openMeeting(meeting.id) }.buttonStyle(.whimbrelQuiet)
        }
    }

    @ViewBuilder
    private var recap: some View {
        switch TodayRows.recap(content) {
        case let .brief(text, origin):
            SkinText(text, lines: .max).foregroundStyle(Color.whimbrelTextSecondary)
            if let origin { SkinText("From \(origin)", role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
        case let .digest(text):
            SkinText(text, lines: .max).foregroundStyle(Color.whimbrelTextSecondary)
        case let .failed(message, retryEnabled):
            SkinText(message, lines: 6).foregroundStyle(Color.whimbrelError)
            Button("Try again") { actions.generateDigest() }
                .buttonStyle(.whimbrelSecondary)
                .disabled(!retryEnabled)
        case .none:
            EmptyView()
        }
    }

    /// This Mac's activity on one line, plus how many other devices reported.
    private var devicesFooter: some View {
        let mine = content.deviceRows.first { $0.deviceId == content.deviceId }
        let others = content.deviceRows.filter { $0.deviceId != content.deviceId }.count
        var line = mine.map(StandardTodayLayout.activityLine) ?? voice.text(.emptyEarlier, name: "")
        if others > 0 { line += " · \(others) other device\(others == 1 ? "" : "s")" }
        return HStack(spacing: Spacing.sm) {
            WhimbrelMetadataChip(text: content.deviceLabel(for: content.deviceId)).help(content.deviceId)
            SkinText(line, role: .label).foregroundStyle(Color.whimbrelTextSecondary)
        }
        .accessibilityElement(children: .combine)
    }
}

/// A horizontal line across its rect, for a dashed stroke.
private struct DottedRule: Shape {
    func path(in rect: CGRect) -> Path {
        Path { $0.move(to: CGPoint(x: rect.minX, y: rect.midY)); $0.addLine(to: CGPoint(x: rect.maxX, y: rect.midY)) }
    }
}
