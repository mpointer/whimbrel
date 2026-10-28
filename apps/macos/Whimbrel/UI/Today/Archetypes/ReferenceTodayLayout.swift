import SwiftUI

/// An encyclopedia article: a serif title over a rule, a Contents box, an infobox for up next
/// (floated right at wide content widths, stacked above otherwise), ruled sections, and actions as
/// bracketed links. Places every section; the hero empty states and the native slots are
/// `TodayChrome`'s.
struct ReferenceTodayLayout: TodayArchetypeView {
    let content: TodayContent
    let actions: TodayActions
    let composer: AnyView?

    init(content: TodayContent, actions: TodayActions, composer: AnyView?) {
        self.content = content
        self.actions = actions
        self.composer = composer
    }

    static var placedSections: Set<TodaySection> { Set(TodaySection.allCases).subtracting([.followups]) }  // follow-ups are TodayChrome's leftover

    enum InfoboxPlacement: Equatable { case floated, stacked }

    /// The infobox floats right from this content width up.
    static let floatThreshold: CGFloat = 700
    static let infoboxWidth: CGFloat = 240

    static func infoboxPlacement(width: CGFloat) -> InfoboxPlacement {
        width >= floatThreshold ? .floated : .stacked
    }

    private var skin: Skin { SkinStore.shared.selected }
    private var voice: SkinVoice { skin.voice }

    /// A section that is drawn, in page order, with its Contents entry.
    private struct Entry: Identifiable {
        let section: TodaySection
        let title: String
        var id: TodaySection { section }
    }

    private var schedule: (upcoming: [TodayEventRowModel], held: [TodayMeetingRowModel]) {
        // The infobox shows `upcomingToday.first`, so the schedule lists the rest.
        TodayRows.schedule(content, upNextPlaced: true)
    }

    private var entries: [Entry] {
        let present = content.nonEmptySections()
        var out: [Entry] = []
        if present.contains(.composer), composer != nil { out.append(Entry(section: .composer, title: "Ask")) }
        if present.contains(.upNext), TodayRows.upNext(content) != nil { out.append(Entry(section: .upNext, title: voice.text(.upNextHeader, name: ""))) }
        if present.contains(.schedule), !(schedule.upcoming.isEmpty && schedule.held.isEmpty) { out.append(Entry(section: .schedule, title: voice.text(.upcomingHeader, name: ""))) }
        if present.contains(.earlier) { out.append(Entry(section: .earlier, title: voice.text(.earlierHeader, name: ""))) }
        if present.contains(.threads) { out.append(Entry(section: .threads, title: "Open loops")) }
        if present.contains(.recap) { out.append(Entry(section: .recap, title: voice.text(.recapHeader, name: ""))) }
        if present.contains(.devices) { out.append(Entry(section: .devices, title: "Devices")) }
        return out
    }

    var body: some View {
        ScrollViewReader { proxy in
            VStack(alignment: .leading, spacing: Spacing.xl) {
                titleBlock.skinBadge()
                intro(proxy)
                bodySections
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: pieces

    /// The date over the greeting; with no usable profile name the date is the title.
    private var titleBlock: some View {
        let dateLine = content.now.formatted(.dateTime.weekday(.wide).month(.wide).day())
        let greeting = TodayLogic.greeting(name: content.name, now: content.now, calendar: .current, voice: voice)
        return VStack(alignment: .leading, spacing: Spacing.xs) {
            SkinText(greeting ?? dateLine, role: .display, lines: 2, size: 30).foregroundStyle(Color.whimbrelTextPrimary)
            if greeting != nil { SkinText(dateLine, role: .label, size: 12).foregroundStyle(Color.whimbrelTextSecondary) }
            rule
        }
    }

    private var rule: some View {
        Rectangle().fill(Color.whimbrelBorder).frame(height: 1).accessibilityHidden(true)
    }

    /// Contents with the infobox beside it (floated) or above it (stacked), then the composer.
    /// Identity constraint: the composer (typed Ask text) and the infobox (expanded prep) hold state,
    /// so neither may move between `if` branches when the placement flips. The composer sits outside
    /// the switching container, and the infobox and Contents are the two fixed children of one
    /// `ReferenceIntroLayout`, which picks the arrangement from the width it is offered. That also
    /// means the first frame is already right: there is no measured-width state to seed.
    @ViewBuilder
    private func intro(_ proxy: ScrollViewProxy) -> some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            if let next = TodayRows.upNext(content) {
                ReferenceIntroLayout {
                    infobox(next)
                    contentsBox(proxy)
                }
            } else if !entries.isEmpty {
                contentsBox(proxy)
            }
            if entries.contains(where: { $0.section == .composer }), let composer { composer.id(TodaySection.composer) }
        }
    }

    private func contentsBox(_ proxy: ScrollViewProxy) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            SkinText("Contents", role: .label, size: 13).foregroundStyle(Color.whimbrelTextPrimary)
            ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                // Scrolls Today's ScrollView (in `TodayChrome`) to the section; this view sits inside it.
                Button { proxy.scrollTo(entry.section, anchor: .top) } label: {
                    HStack(spacing: Spacing.xs) {
                        Text("\(index + 1)").foregroundStyle(Color.whimbrelTextSecondary).accessibilityHidden(true)
                        SkinText(entry.title, size: 13).foregroundStyle(Color.whimbrelLink)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Go to \(entry.title)")
            }
        }
        .padding(Spacing.md)
        .background(Color.whimbrelBackgroundCard)
        .overlay(Rectangle().stroke(Color.whimbrelBorder, lineWidth: 1))
        .frame(maxWidth: 260, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func infobox(_ event: TodayEventRowModel) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            SkinText(voice.text(.upNextHeader, name: ""), role: .label, size: 13)
                .foregroundStyle(Color.whimbrelTextOnAccent)
                .padding(Spacing.sm)
                .frame(maxWidth: .infinity)
                .background(Color.whimbrelAccentPrimary)
            VStack(alignment: .leading, spacing: Spacing.xs) {
                SkinText(event.title, role: .display, lines: 3, size: 17).foregroundStyle(Color.whimbrelTextPrimary)
                SkinText(event.timeText, role: .numerals).foregroundStyle(Color.whimbrelTextPrimary)
                if let relative = event.relative { SkinText(relative, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
                if !event.attendees.isEmpty { SkinText(event.attendees, role: .label, lines: 2).foregroundStyle(Color.whimbrelTextSecondary) }
                ReferencePrepLine(prep: event.prep)
                if event.showsStart { ReferenceLink("Start", role: .primary) { actions.startMeeting(event.event) } }
            }
            .padding(Spacing.sm)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Color.whimbrelBackgroundCard)
        .overlay(Rectangle().stroke(Color.whimbrelBorder, lineWidth: 1))
        .id(TodaySection.upNext)
    }

    @ViewBuilder
    private var bodySections: some View {
        let present = Set(entries.map(\.section))
        if present.contains(.schedule) {
            section(.schedule, voice.text(.upcomingHeader, name: "")) {
                ForEach(TodayRows.scheduleItems(schedule)) { item in
                    switch item {
                    case let .event(event): eventRow(event)
                    case .separator: SkinText(TodayRows.heldSeparator, role: .label, size: 12).foregroundStyle(Color.whimbrelTextSecondary)
                    case let .held(meeting): meetingRow(meeting)
                    }
                }
            }
        }
        if present.contains(.earlier) {
            section(.earlier, voice.text(.earlierHeader, name: "")) {
                ForEach(TodayRows.earlier(content), id: \.id) { meetingRow($0) }
            }
        }
        if present.contains(.threads) {
            section(.threads, "Open loops") {
                ForEach(content.attentionThreads) { thread in
                    Button { actions.openThread(thread.id) } label: {
                        SkinText(thread.subject ?? thread.lastMessageFrom, lines: 2)
                            .foregroundStyle(Color.whimbrelLink)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help("Open this thread")
                }
            }
        }
        if present.contains(.recap) {
            section(.recap, voice.text(.recapHeader, name: "")) { recap }
        }
        if present.contains(.devices) {
            section(.devices, "Devices") { devicesFooter }
        }
    }

    /// A serif heading over a rule, then the rows; the id is the Contents entry's target.
    private func section<C: View>(_ which: TodaySection, _ title: String, @ViewBuilder _ rows: () -> C) -> some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                SkinText(title, role: .display, lines: 2, size: 21).foregroundStyle(Color.whimbrelTextPrimary)
                rule
            }
            rows()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .id(which)
    }

    private func eventRow(_ event: TodayEventRowModel) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                SkinText(event.timeText, role: .numerals).foregroundStyle(Color.whimbrelTextSecondary)
                SkinText(event.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
            }
            if let relative = event.relative { SkinText(relative, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            if !event.attendees.isEmpty { SkinText(event.attendees, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            if event.showsStart { ReferenceLink("Start", role: .primary) { actions.startMeeting(event.event) } }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func meetingRow(_ meeting: TodayMeetingRowModel) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                SkinText(meeting.timeText, role: .numerals).foregroundStyle(Color.whimbrelTextSecondary)
                SkinText(meeting.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
            }
            if !meeting.subtitle.isEmpty { SkinText(meeting.subtitle, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            ReferenceLink("Open") { actions.openMeeting(meeting.id) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Recap text is body text: no line limit, leading-aligned.
    @ViewBuilder
    private var recap: some View {
        switch TodayRows.recap(content) {
        case let .brief(text, origin):
            recapText(text)
            if let origin { SkinText("From \(origin)", role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
        case let .digest(text):
            recapText(text)
        case let .failed(message, retryEnabled):
            SkinText(message, lines: .max).foregroundStyle(Color.whimbrelError)
            ReferenceLink("Try again", enabled: retryEnabled) { actions.generateDigest() }
        case .none:
            EmptyView()
        }
    }

    private func recapText(_ text: String) -> some View {
        SkinText(text, lines: .max)
            .foregroundStyle(Color.whimbrelTextPrimary)
            .multilineTextAlignment(.leading)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// This Mac's activity on one line, plus how many other devices reported.
    private var devicesFooter: some View {
        let mine = content.deviceRows.first { $0.deviceId == content.deviceId }
        let others = content.deviceRows.filter { $0.deviceId != content.deviceId }.count
        // Whimbrel's own wording for "no activity", not the skin's voice (Standard's footer shows the same line).
        var line = mine.flatMap { $0.captures > 0 ? StandardTodayLayout.activityLine($0) : nil }
            ?? SkinDefaults.whimbrel.voice.text(.emptyEarlier, name: "")
        if others > 0 { line += " · \(others) other device\(others == 1 ? "" : "s")" }
        return VStack(alignment: .leading, spacing: Spacing.xs) {
            WhimbrelMetadataChip(text: content.deviceLabel(for: content.deviceId)).help(content.deviceId)
            SkinText(line, role: .label).foregroundStyle(Color.whimbrelTextSecondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

/// Two children, infobox then Contents. Offered `floatThreshold` or more, Contents sits left and the
/// infobox right (`infoboxWidth` wide); narrower, the infobox is above Contents at full width.
struct ReferenceIntroLayout: Layout {
    private let gap = Spacing.lg

    /// The width to lay out at: the proposal's, or with none (an ideal-size query) the children's ideal widths.
    private func width(_ proposal: ProposedViewSize, _ subviews: Subviews) -> CGFloat {
        if let w = proposal.width { return w }
        let info = subviews[0].sizeThatFits(.unspecified).width
        let contents = subviews[1].sizeThatFits(.unspecified).width
        let floatedWidth = info + gap + contents
        return ReferenceTodayLayout.infoboxPlacement(width: floatedWidth) == .floated ? floatedWidth : max(info, contents)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard subviews.count == 2 else { return .zero }
        let w = width(proposal, subviews)
        if ReferenceTodayLayout.infoboxPlacement(width: w) == .floated {
            let info = subviews[0].sizeThatFits(ProposedViewSize(width: ReferenceTodayLayout.infoboxWidth, height: nil))
            let contents = subviews[1].sizeThatFits(ProposedViewSize(width: w - ReferenceTodayLayout.infoboxWidth - gap, height: nil))
            return CGSize(width: w, height: max(info.height, contents.height))
        }
        let info = subviews[0].sizeThatFits(ProposedViewSize(width: w, height: nil))
        let contents = subviews[1].sizeThatFits(ProposedViewSize(width: w, height: nil))
        return CGSize(width: w, height: info.height + gap + contents.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 2 else { return }
        if ReferenceTodayLayout.infoboxPlacement(width: proposal.width ?? bounds.width) == .floated {
            let side = ReferenceTodayLayout.infoboxWidth
            subviews[0].place(at: CGPoint(x: bounds.maxX - side, y: bounds.minY), anchor: .topLeading, proposal: ProposedViewSize(width: side, height: nil))
            subviews[1].place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width - side - gap, height: nil))
        } else {
            let info = subviews[0].sizeThatFits(ProposedViewSize(width: bounds.width, height: nil))
            subviews[0].place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: nil))
            subviews[1].place(at: CGPoint(x: bounds.minX, y: bounds.minY + info.height + gap), anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: nil))
        }
    }
}

/// An action as a bracketed link: `[Open]`. The brackets are decoration; the accessible label is
/// exactly Whimbrel's. Disabled is drawn without the underline and faded, not just recoloured.
struct ReferenceLink: View {
    /// The button's role colour: a primary action (Start) keeps the palette's primary colour.
    enum Role {
        case link, primary
        var token: ThemeToken {
            switch self {
            case .link: return .link
            case .primary: return .accentPrimary
            }
        }
    }

    let label: String
    let enabled: Bool
    let role: Role
    let action: () -> Void

    init(_ label: String, enabled: Bool = true, role: Role = .link, action: @escaping () -> Void) {
        self.label = label
        self.enabled = enabled
        self.role = role
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 1) {
                Text("[").accessibilityHidden(true)
                SkinText(label, size: 13).underline(enabled)
                Text("]").accessibilityHidden(true)
            }
            .foregroundStyle(Color.theme(role.token))
            .opacity(enabled ? 1 : 0.5)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityLabel(label)
    }
}

/// The next event's prep state with `[Open brief]` / `[Hide brief]` links (decision from `TodayRows`).
struct ReferencePrepLine: View {
    let prep: TodayEventRowModel.Prep?
    @State private var expanded = false

    var body: some View {
        switch prep {
        case let .ready(prep):
            HStack(spacing: Spacing.sm) {
                SkinText("Prep ready", role: .label).foregroundStyle(Color.whimbrelTextSecondary)
                ReferenceLink(expanded ? "Hide brief" : "Open brief") { expanded.toggle() }
            }
            if expanded { MeetingPrepView(prep: prep) }
        case .missing:
            SkinText("No prep yet", role: .label).foregroundStyle(Color.whimbrelTextSecondary)
        case nil:
            EmptyView()
        }
    }
}
