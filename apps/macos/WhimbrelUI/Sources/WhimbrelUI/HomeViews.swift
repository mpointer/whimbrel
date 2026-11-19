import SwiftUI

// Home, from a `WHomeModel` and `WHomeActions`, built from the Thread components. The five element
// choices in `model.elements` change how it is drawn, never what it says or does.

public struct WHomeView<Live: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.wScrollsContent) private var scrolls
    let model: WHomeModel
    let actions: WHomeActions
    let scroll: WHomeScroll?
    let live: Live
    @State private var open: Set<String> = []

    public init(model: WHomeModel, actions: WHomeActions, scroll: WHomeScroll? = nil, @ViewBuilder live: () -> Live) {
        self.model = model
        self.actions = actions
        self.scroll = scroll
        self.live = live()
    }

    public var body: some View {
        ScrollViewReader { proxy in
            Group {
                if scrolls { ScrollView { column } } else { column }
            }
            .onChange(of: scroll) { _, request in
                guard let request else { return }
                if request.id != "today" { open.insert(request.id) }
                proxy.scrollTo(request.id == "today" ? "top" : request.id, anchor: .top)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(theme.backgroundPage)
        .environment(\.wActionStyle, model.elements.actions)
        .environment(\.wLabelStyle, model.elements.labels)
    }

    private var column: some View {
        VStack(alignment: .leading, spacing: theme.space(20)) {
            Color.clear.frame(height: 0).id("top").accessibilityHidden(true)
            if model.load == .loading { WProgressLine(label: strings.loading) }
            if case .failed(let title, let help) = model.load {
                WErrorState(title: title, help: help, onRetry: actions.retry)
            } else if model.elements.timeline == .ruled {
                WHomeRuledTimeline(model: model, actions: actions, open: $open) { live }
            } else {
                WHomeLineTimeline(model: model, actions: actions, open: $open) { live }
            }
            ForEach(model.sections) { section in WHomeSectionView(section: section, actions: actions) }
        }
        .frame(maxWidth: 760 * k, alignment: .leading)
        .padding(.horizontal, theme.space(28))
        .padding(.vertical, theme.space(8))
        .padding(.bottom, theme.space(24))
        .frame(maxWidth: .infinity, alignment: .center)
    }
}

extension WHomeView where Live == EmptyView {
    public init(model: WHomeModel, actions: WHomeActions, scroll: WHomeScroll? = nil) {
        self.init(model: model, actions: actions, scroll: scroll) { EmptyView() }
    }
}

/// One row of the line, with the hero in its place just above the now marker.
enum WHomeLineRow: Identifiable {
    case stop(WHomeStop)
    case hero

    var id: String {
        switch self {
        case .stop(let stop): return stop.id
        case .hero: return "hero"
        }
    }

    static func rows(_ model: WHomeModel) -> [WHomeLineRow] {
        var rows = model.timeline.map(WHomeLineRow.stop)
        let at = model.timeline.firstIndex { if case .now = $0 { return true } else { return false } } ?? rows.count
        if case .none = model.hero { return rows }
        rows.insert(.hero, at: at)
        return rows
    }
}

/// The hero's card, in the skin's choice for the next meeting.
struct WHomeNextCard: View {
    let meeting: WHomeMeeting
    let style: WNextMeetingStyle
    let actions: WHomeActions

    var body: some View {
        switch style {
        case .card:
            WMeetingCard(
                variant: .next, time: meeting.timeRange, title: meeting.title, attendees: meeting.attendees.map { WPerson(name: $0) }, eyebrow: meeting.eyebrow,
                primaryTitle: meeting.startTitle, primaryGlyph: "play.fill", onPrimary: meeting.startTitle == nil ? nil : { actions.startMeeting(meeting.id) },
                secondaryTitle: meeting.briefTitle, onSecondary: meeting.briefTitle == nil ? nil : { actions.openBrief(meeting.id) })
        case .scoreboard: WHomeScoreboard(meeting: meeting, actions: actions)
        case .infobox: WHomeInfobox(meeting: meeting, actions: actions)
        }
    }
}

/// The hero when it is not a next meeting: the live call, or the empty state for a day with no calendar or no meetings.
struct WHomeHeroState<Live: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let hero: WHomeHero
    let actions: WHomeActions
    let live: Live

    var body: some View {
        switch hero {
        case .live(let title, let elapsed):
            VStack(alignment: .leading, spacing: theme.space(12)) {
                HStack(spacing: theme.space(12)) {
                    WRecordingIndicator(state: .recording, elapsedSeconds: elapsed, size: .pill)
                    Text(title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).lineLimit(2)
                    Spacer(minLength: theme.space(8))
                    WAction(strings.open, kind: .secondary, action: actions.openLive)
                }
                .padding(theme.space(12))
                .wCard()
                .accessibilityElement(children: .contain)
                live
            }
        case .noCalendar(let title, let help, let actionTitle):
            WEmptyState(glyph: "calendar.badge.exclamationmark", title: title, help: help, actionTitle: actionTitle, action: actions.connectCalendar)
        case .noMeetings(let title, let help, let actionTitle):
            WEmptyState(glyph: "calendar", title: title, help: help, actionTitle: actionTitle, action: actions.startNote)
        case .next, .none:
            EmptyView()
        }
    }
}

/// "Yesterday" or "Last week": the meetings as chips, and an open state that lists them as past meeting rows.
struct WHomeHistoryRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let history: WHomeHistory
    @Binding var isOpen: Bool
    let actions: WHomeActions

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            Button { isOpen.toggle() } label: {
                HStack(alignment: .center, spacing: theme.space(10)) {
                    WGlyph(name: isOpen ? "chevron.down" : "chevron.right", size: 11, weight: .semibold).foregroundStyle(theme.textSecondary).frame(width: 14 * k)
                    Text(history.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                    Text(history.dateText).wFont(.secondary).foregroundStyle(theme.textSecondary)
                    if !isOpen {
                        WFlowLayout(spacing: theme.space(6), lineSpacing: theme.space(4)) {
                            ForEach(history.meetings) { m in
                                HStack(spacing: theme.space(5)) {
                                    WGlyph(name: "video", size: 11)
                                    Text(m.title).wFont(.secondary).lineLimit(1)
                                }
                                .foregroundStyle(theme.textPrimary)
                                .padding(.horizontal, theme.space(10))
                                .padding(.vertical, theme.space(3))
                                .overlay(RoundedRectangle(cornerRadius: WTokens.Radius.pill, style: .continuous).wBorder(theme.border, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 0.75)))
                            }
                        }
                    }
                    Spacer(minLength: 0)
                }
                .frame(minHeight: WTokens.Size.target * k)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(strings.joinDetails([history.title, history.dateText]))
            .accessibilityValue(isOpen ? strings.home.expanded : strings.home.collapsed)
            if isOpen {
                VStack(spacing: theme.space(8)) {
                    ForEach(history.meetings) { m in
                        WMeetingCard(
                            variant: .past, time: m.time, title: m.title, attendees: m.attendees.map { WPerson(name: $0) }, gist: m.gist,
                            primaryTitle: m.openTitle, onPrimary: m.openTitle == nil ? nil : { actions.openMeeting(m.id) })
                    }
                }
            }
        }
        .padding(.horizontal, theme.space(14))
        .padding(.vertical, theme.space(6))
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard()
        .accessibilityElement(children: .contain)
    }
}

// MARK: Ask prompt

/// The prompt variant of the ask bar: a mark, the field and a rule under it.
struct WPromptField: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Binding var text: String
    let placeholder: String
    let onSubmit: () -> Void

    var body: some View {
        HStack(spacing: theme.space(8)) {
            Text(">").wFont(.bodyStrong).foregroundStyle(theme.accentPrimary).accessibilityHidden(true)
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .wFont(.body)
                .foregroundStyle(theme.textPrimary)
                .accessibilityLabel(placeholder)
                .onSubmit(onSubmit)
            WKeycap(text: "\u{2318}K")
        }
        .padding(.horizontal, theme.space(12))
        .frame(minHeight: 34 * k)
        .background(theme.backgroundCard)
        .overlay(Rectangle().strokeBorder(theme.border, lineWidth: max(1, theme.borderWidth)))
    }
}

// MARK: Filters

struct WHomeFilterChips: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let people: [String]
    let apps: [WHomeAppChoice]
    @Binding var filter: WHomeFilter
    @State private var showPeople = false
    @State private var showApps = false
    @State private var showDates = false
    @State private var customFrom = Calendar.current.startOfDay(for: Date())
    @State private var customTo = Calendar.current.startOfDay(for: Date())

    private var appTitle: String { apps.first { $0.id == filter.appId }?.name ?? filter.appId ?? strings.home.appsFilter }

    private var dateTitle: String {
        switch filter.dates {
        case nil: return strings.home.datesFilter
        case .today: return strings.home.today
        case .yesterday: return strings.home.yesterday
        case .lastSevenDays: return strings.home.lastSevenDays
        case .custom(let from, let to):
            let style = Date.FormatStyle().month(.abbreviated).day()
            return "\(from.formatted(style)) - \(to.formatted(style))"
        }
    }

    var body: some View {
        WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
            WFilterChip(filter.person ?? strings.home.peopleFilter, glyph: "person.2", isSelected: filter.person != nil, onRemove: filter.person == nil ? nil : { filter.person = nil }) { showPeople = true }
                .popover(isPresented: $showPeople, arrowEdge: .bottom) { peopleList }
            WFilterChip(appTitle, glyph: "square.grid.2x2", isSelected: filter.appId != nil, onRemove: filter.appId == nil ? nil : { filter.appId = nil }) { showApps = true }
                .popover(isPresented: $showApps, arrowEdge: .bottom) { appList }
            WFilterChip(dateTitle, glyph: "calendar", isSelected: filter.dates != nil, onRemove: filter.dates == nil ? nil : { filter.dates = nil }) { showDates = true }
                .popover(isPresented: $showDates, arrowEdge: .bottom) { dateList }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.home.searchFilters)
    }

    private func choice(_ title: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: theme.space(8)) {
                WGlyph(name: "checkmark", size: 11, weight: .bold).opacity(selected ? 1 : 0)
                Text(title).wFont(.body).foregroundStyle(theme.textPrimary).lineLimit(1)
                Spacer(minLength: theme.space(12))
            }
            .padding(.horizontal, theme.space(10))
            .frame(minHeight: 28, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var peopleList: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                choice(strings.home.anyPerson, selected: filter.person == nil) { filter.person = nil; showPeople = false }
                ForEach(people, id: \.self) { name in
                    choice(name, selected: filter.person == name) { filter.person = name; showPeople = false }
                }
            }
            .padding(theme.space(6))
        }
        .frame(minWidth: 220, maxHeight: 320)
    }

    private var appList: some View {
        VStack(alignment: .leading, spacing: 0) {
            choice(strings.home.anyApp, selected: filter.appId == nil) { filter.appId = nil; showApps = false }
            ForEach(apps) { app in
                choice(app.name, selected: filter.appId == app.id) { filter.appId = app.id; showApps = false }
            }
        }
        .padding(theme.space(6))
        .frame(minWidth: 200)
    }

    private var dateList: some View {
        VStack(alignment: .leading, spacing: 0) {
            choice(strings.home.anyDate, selected: filter.dates == nil) { filter.dates = nil; showDates = false }
            choice(strings.home.today, selected: filter.dates == .today) { filter.dates = .today; showDates = false }
            choice(strings.home.yesterday, selected: filter.dates == .yesterday) { filter.dates = .yesterday; showDates = false }
            choice(strings.home.lastSevenDays, selected: filter.dates == .lastSevenDays) { filter.dates = .lastSevenDays; showDates = false }
            Divider().padding(.vertical, theme.space(6))
            VStack(alignment: .leading, spacing: theme.space(6)) {
                Text(strings.home.customRange).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                DatePicker(strings.home.rangeFrom, selection: $customFrom, displayedComponents: .date).wFont(.body)
                DatePicker(strings.home.rangeTo, selection: $customTo, displayedComponents: .date).wFont(.body)
                WButton(strings.home.applyRange, kind: .primary) {
                    filter.dates = .custom(from: customFrom, to: customTo)
                    showDates = false
                }
            }
            .padding(.horizontal, theme.space(10))
            .padding(.bottom, theme.space(6))
        }
        .padding(theme.space(6))
        .frame(minWidth: 240)
    }
}

// MARK: Next meeting variants

/// The scoreboard: the start time in large digits on the accent color. Text is `textOnAccent` on `accentPrimary`.
struct WHomeScoreboard: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wActionStyle) private var actionStyle
    let meeting: WHomeMeeting
    let actions: WHomeActions

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous)
        VStack(alignment: .leading, spacing: theme.space(10)) {
            HStack(alignment: .firstTextBaseline, spacing: theme.space(16)) {
                Text(meeting.time)
                    .font(.system(size: 44 * k, weight: .bold, design: .default).monospacedDigit())
                    .minimumScaleFactor(0.6)
                    .lineLimit(1)
                VStack(alignment: .leading, spacing: theme.space(2)) {
                    Text(meeting.title).wFont(.title).fixedSize(horizontal: false, vertical: true).accessibilityAddTraits(.isHeader)
                    if let eyebrow = meeting.eyebrow { Text(eyebrow).wFont(.body) }
                }
            }
            if !meeting.attendees.isEmpty {
                Text(meeting.attendees.joined(separator: ", ")).wFont(.body).fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: theme.space(14)) {
                Spacer(minLength: 0)
                if let brief = meeting.briefTitle { control(brief) { actions.openBrief(meeting.id) } }
                if let start = meeting.startTitle { control(start) { actions.startMeeting(meeting.id) } }
            }
        }
        .foregroundStyle(theme.textOnAccent)
        .padding(theme.space(16))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(theme.accentPrimary))
        .overlay { if theme.isHighContrast { shape.strokeBorder(theme.textPrimary, lineWidth: theme.shape.highContrastBorderWidth) } }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder
    private func control(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Group {
                if actionStyle == .links {
                    Text(title).underline().foregroundStyle(theme.textOnAccent)
                } else {
                    Text(title)
                        .foregroundStyle(theme.accentPrimary)
                        .padding(.horizontal, theme.space(12))
                        .frame(minHeight: WTokens.Size.target * k)
                        .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.textOnAccent))
                }
            }
            .wFont(.bodyStrong)
            .frame(minWidth: WTextLink.minimumTarget * k, minHeight: WTextLink.minimumTarget * k)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
    }
}

/// The info box: a title over label and value rows, on the card and page colors only.
struct WHomeInfobox: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let meeting: WHomeMeeting
    let actions: WHomeActions

    var body: some View {
        let rule = max(1, theme.borderWidth)
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: theme.space(2)) {
                Text(meeting.title).wFont(.title).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true).accessibilityAddTraits(.isHeader)
                if let eyebrow = meeting.eyebrow { Text(eyebrow).wFont(.secondary).foregroundStyle(theme.textSecondary) }
            }
            .padding(theme.space(12))
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(theme.backgroundPage)
            ForEach(meeting.facts) { fact in
                Rectangle().fill(theme.border).frame(height: rule)
                HStack(alignment: .firstTextBaseline, spacing: 0) {
                    Text(fact.label).wFont(.bodyStrong).foregroundStyle(theme.textSecondary)
                        .frame(width: 96 * k, alignment: .leading)
                        .padding(theme.space(10))
                        .background(theme.backgroundPage)
                    Text(fact.value).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                        .padding(theme.space(10))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(theme.backgroundCard)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(strings.joinDetails([fact.label, fact.value]))
            }
            if meeting.startTitle != nil || meeting.briefTitle != nil {
                Rectangle().fill(theme.border).frame(height: rule)
                HStack(spacing: theme.space(14)) {
                    if let start = meeting.startTitle { WAction(start, kind: .primary) { actions.startMeeting(meeting.id) } }
                    if let brief = meeting.briefTitle { WAction(brief, kind: .secondary) { actions.openBrief(meeting.id) } }
                    Spacer(minLength: 0)
                }
                .padding(theme.space(10))
                .background(theme.backgroundCard)
            }
        }
        .overlay(Rectangle().strokeBorder(theme.border, lineWidth: rule))
        .accessibilityElement(children: .contain)
    }
}

// MARK: Time line variants

/// What a screen reader says for one stop: its time first, then what it is.
enum WHomeSpoken {
    static func meeting(_ m: WHomeMeeting, strings: WStrings) -> String {
        strings.joinDetails([m.time, m.title] + (m.eyebrow.map { [$0] } ?? []))
    }
    static func block(_ b: WHomeBlock, strings: WStrings) -> String {
        strings.joinDetails([b.timeRange, strings.memory] + b.items.map(\.name))
    }
    static func now(time: String, strings: WStrings) -> String {
        strings.joinDetails([strings.now, time])
    }
}

struct WHomeLineTimeline<Live: View>: View {
    @Environment(\.wStrings) private var strings
    let model: WHomeModel
    let actions: WHomeActions
    @Binding var open: Set<String>
    let live: Live

    init(model: WHomeModel, actions: WHomeActions, open: Binding<Set<String>>, @ViewBuilder live: () -> Live) {
        self.model = model
        self.actions = actions
        self._open = open
        self.live = live()
    }

    var body: some View {
        let rows = WHomeLineRow.rows(model)
        let nowIndex = rows.firstIndex { if case .stop(.now) = $0 { return true } else { return false } } ?? rows.count
        WSpine {
            ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                let last = index == rows.count - 1
                let above = index < nowIndex
                let from = index > 0
                switch row {
                case .hero:
                    switch model.hero {
                    case .next(let m):
                        WSpineStop(time: m.time, node: .upcoming, connector: last ? .none : .dashed, connectsFromAbove: from) {
                            WHomeNextCard(meeting: m, style: model.elements.nextMeeting, actions: actions)
                        }
                        .accessibilityLabel(WHomeSpoken.meeting(m, strings: strings))
                    case .live:
                        WSpineStop(time: "", node: .recording, connector: last ? .none : .recording, connectsFromAbove: from) {
                            WHomeHeroState(hero: model.hero, actions: actions, live: live)
                        }
                    default:
                        WSpineStop(time: "", node: .upcoming, connector: last ? .none : .dashed, connectsFromAbove: from) {
                            WHomeHeroState(hero: model.hero, actions: actions, live: live)
                        }
                    }
                case .stop(let stop):
                    switch stop {
                    case .meeting(let m):
                        WSpineStop(
                            time: m.time, node: m.kind == .past ? .past : (m.kind == .running ? .now : .upcoming),
                            connector: last ? .none : (above ? .dashed : .solid), connectsFromAbove: from
                        ) {
                            WMeetingCard(
                                variant: m.kind == .past ? .past : .upcoming, time: m.time, title: m.title,
                                attendees: m.attendees.map { WPerson(name: $0) }, eyebrow: m.eyebrow, gist: m.gist,
                                primaryTitle: m.kind == .past ? m.openTitle : m.startTitle,
                                onPrimary: (m.kind == .past ? m.openTitle : m.startTitle) == nil ? nil : {
                                    if m.kind == .past { actions.openMeeting(m.id) } else { actions.startMeeting(m.id) }
                                }
                            )
                        }
                        .accessibilityLabel(WHomeSpoken.meeting(m, strings: strings))
                    case .memory(let b):
                        WSpineStop(time: "", caption: b.duration, node: .past, connector: last ? .none : .span, connectsFromAbove: from) {
                            WHomeBlockView(block: b, actions: actions)
                        }
                        .accessibilityLabel(WHomeSpoken.block(b, strings: strings))
                    case .now(let time, let label):
                        WNowMarker(time: time, label: label, connector: last ? .none : .solid)
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel(WHomeSpoken.now(time: time, strings: strings))
                    case .notice(let text):
                        WHomeNoticeStop(text: text, connectsFromAbove: from, last: last)
                    case .history(let h):
                        WSpineStop(time: "", node: .upcoming, connector: last ? .none : .solid, connectsFromAbove: from) {
                            WHomeHistoryRow(history: h, isOpen: binding(h.id), actions: actions)
                        }
                        .id(h.id)
                    }
                }
            }
        }
    }

    private func binding(_ id: String) -> Binding<Bool> {
        Binding(get: { open.contains(id) }, set: { if $0 { open.insert(id) } else { open.remove(id) } })
    }
}

struct WHomeNoticeStop: View {
    @Environment(\.wTheme) private var theme
    let text: String
    let connectsFromAbove: Bool
    let last: Bool

    var body: some View {
        WSpineStop(time: "", node: .past, connector: last ? .none : .dashed, connectsFromAbove: connectsFromAbove) {
            Text(text)
                .wFont(.body)
                .foregroundStyle(theme.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, theme.space(10))
        }
    }
}

/// A memory block with the Open action of the skin's choice.
struct WHomeBlockView: View {
    @Environment(\.wTheme) private var theme
    let block: WHomeBlock
    let actions: WHomeActions

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(2)) {
            WMemoryBlock(timeRange: block.timeRange, items: block.items)
                .overlay(alignment: .topTrailing) {
                    WAction(block.openTitle, kind: .quiet) { actions.openBlock(block.id) }
                        .padding(.trailing, theme.space(6))
                }
            if let more = block.more { Text(more).wFont(.secondary).foregroundStyle(theme.textSecondary).padding(.leading, theme.space(14)) }
        }
    }
}

/// The ruled time line: rows between rules, no nodes, in the same order as the line. The now row carries an accent fill.
struct WHomeRuledTimeline<Live: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let model: WHomeModel
    let actions: WHomeActions
    @Binding var open: Set<String>
    let live: Live

    init(model: WHomeModel, actions: WHomeActions, open: Binding<Set<String>>, @ViewBuilder live: () -> Live) {
        self.model = model
        self.actions = actions
        self._open = open
        self.live = live()
    }

    var body: some View {
        let rule = max(1, theme.borderWidth)
        VStack(spacing: 0) {
            Rectangle().fill(theme.border).frame(height: rule)
            ForEach(WHomeLineRow.rows(model)) { row in
                self.row(row)
                Rectangle().fill(theme.border).frame(height: rule)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.timeline)
    }

    @ViewBuilder
    private func row(_ row: WHomeLineRow) -> some View {
        switch row {
        case .hero:
            switch model.hero {
            case .next(let m):
                WHomeNextCard(meeting: m, style: model.elements.nextMeeting, actions: actions)
                    .padding(.vertical, theme.space(10))
                    .accessibilityLabel(WHomeSpoken.meeting(m, strings: strings))
            default:
                WHomeHeroState(hero: model.hero, actions: actions, live: live)
            }
        case .stop(let stop):
            stopRow(stop)
        }
    }

    @ViewBuilder
    private func stopRow(_ stop: WHomeStop) -> some View {
        switch stop {
        case .meeting(let m):
            line(time: m.time, emphasis: false) {
                HStack(alignment: .firstTextBaseline, spacing: theme.space(10)) {
                    VStack(alignment: .leading, spacing: theme.space(2)) {
                        Text(m.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                        if let gist = m.gist { Text(gist).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1) }
                        if !m.attendees.isEmpty { Text(m.attendees.joined(separator: ", ")).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1) }
                    }
                    Spacer(minLength: theme.space(8))
                    if let eyebrow = m.eyebrow { Text(eyebrow).wFont(.secondary).foregroundStyle(theme.textSecondary) }
                    if let title = m.kind == .past ? m.openTitle : m.startTitle {
                        WAction(title, kind: .quiet) { if m.kind == .past { actions.openMeeting(m.id) } else { actions.startMeeting(m.id) } }
                    }
                }
            }
            .accessibilityLabel(WHomeSpoken.meeting(m, strings: strings))
        case .memory(let b):
            line(time: b.timeRange, emphasis: false) {
                HStack(alignment: .firstTextBaseline, spacing: theme.space(10)) {
                    WFlowLayout(spacing: theme.space(12), lineSpacing: theme.space(2)) {
                        ForEach(b.items) { item in
                            HStack(spacing: theme.space(5)) {
                                WGlyph(name: item.glyph, size: 11).foregroundStyle(theme.textSecondary)
                                Text(item.name).wFont(.body).foregroundStyle(theme.textPrimary).lineLimit(1)
                            }
                        }
                    }
                    Spacer(minLength: theme.space(8))
                    Text(b.duration).wFont(.secondary).foregroundStyle(theme.textSecondary)
                    WAction(b.openTitle, kind: .quiet) { actions.openBlock(b.id) }
                }
            }
            .accessibilityLabel(WHomeSpoken.block(b, strings: strings))
        case .now(let time, let label):
            line(time: time, emphasis: true) {
                HStack(spacing: theme.space(8)) {
                    Text(strings.now).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                    Text(label).wFont(.body).foregroundStyle(theme.textSecondary)
                }
            }
            .background(theme.selection)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(WHomeSpoken.now(time: time, strings: strings))
        case .notice(let text):
            line(time: "", emphasis: false) {
                Text(text).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
        case .history(let h):
            WHomeHistoryRow(history: h, isOpen: Binding(get: { open.contains(h.id) }, set: { if $0 { open.insert(h.id) } else { open.remove(h.id) } }), actions: actions)
                .padding(.vertical, theme.space(8))
                .id(h.id)
        }
    }

    private func line<Content: View>(time: String, emphasis: Bool, @ViewBuilder content: () -> Content) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: theme.space(12)) {
            Text(time)
                .wFont(.time)
                .foregroundStyle(emphasis ? theme.accentPrimary : theme.textPrimary)
                .frame(width: 112 * k, alignment: .leading)
            content()
        }
        .padding(.vertical, theme.space(8))
        .padding(.horizontal, theme.space(4))
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
    }
}

// MARK: Sections below the line

struct WHomeSectionView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let section: WHomeSection
    let actions: WHomeActions

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(8)) {
            WSectionLabel(section.title)
            VStack(alignment: .leading, spacing: theme.space(10)) {
                ForEach(section.rows) { row in
                    VStack(alignment: .leading, spacing: theme.space(6)) {
                        if let heading = row.heading {
                            Text(heading).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                        }
                        HStack(alignment: .firstTextBaseline, spacing: theme.space(12)) {
                            if let time = row.time { Text(time).wFont(.time).foregroundStyle(theme.textSecondary).frame(width: 84 * k, alignment: .leading) }
                            VStack(alignment: .leading, spacing: theme.space(2)) {
                                Text(row.title).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                                if let detail = row.detail { Text(detail).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true) }
                            }
                            Spacer(minLength: theme.space(8))
                            if let title = row.actionTitle {
                                WAction(title, kind: .secondary, spokenName: row.actionName) { actions.rowAction(section.kind, row.id) }
                            }
                        }
                    }
                }
            }
            .padding(theme.space(14))
            .frame(maxWidth: .infinity, alignment: .leading)
            .wCard()
        }
        .accessibilityElement(children: .contain)
    }
}
