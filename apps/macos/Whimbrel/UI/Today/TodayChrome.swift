import SwiftUI

/// An archetype's Today body. It places some `TodaySection`s itself; `TodayChrome` lists the
/// rest in `TodayLeftovers`. `.upNext` is the first of today's upcoming events and `.schedule`
/// is every upcoming event plus the meetings already held today, so an archetype that places both
/// shows the next meeting twice unless it skips that event in the schedule.
protocol TodayArchetypeView: View {
    init(content: TodayContent, actions: TodayActions, composer: AnyView?)
    static var placedSections: Set<TodaySection> { get }
}

enum TodayChromePart: Hashable { case slots, archetype, leftovers }

/// Today for the selected skin's archetype. `standard` is `StandardTodayLayout` unchanged. Any
/// other archetype gets the native slots first (live call, approvals, banners), then the
/// archetype view, then a plain list of the sections it did not place.
struct TodayChrome: View {
    let content: TodayContent
    let actions: TodayActions
    let slots: TodaySlots

    /// Banners on Today: none. The recap error is part of the `.recap` section, not a banner.
    static func order(for archetype: SkinArchetype) -> [TodayChromePart] {
        archetype == .standard ? [.archetype] : [.slots, .archetype, .leftovers]
    }

    /// The sections `TodayLeftovers` lists: non-empty, minus what the archetype places.
    static func leftovers(of content: TodayContent, placed: Set<TodaySection>) -> [TodaySection] {
        content.nonEmptySections().filter { section in
            guard !placed.contains(section) else { return false }
            // The schedule list never repeats up next, so with nothing after it and nothing held it has nothing to draw.
            if section == .schedule { let rest = leftoverSchedule(of: content); return !rest.upcoming.isEmpty || !rest.held.isEmpty }
            return true
        }
    }

    /// The `.schedule` leftover's rows. The next event is never repeated: `.upNext` shows it, whether
    /// an archetype places it or it is a leftover too (it exists exactly when `upcomingToday` is non-empty).
    static func leftoverSchedule(of content: TodayContent) -> (upcoming: [TodayEventRowModel], held: [TodayMeetingRowModel]) {
        TodayRows.schedule(content, upNextPlaced: true)
    }

    /// The skin's header image over the layout, outside its scrolling like a top-level screen's.
    var body: some View {
        VStack(spacing: 0) {
            SkinImagerySlot(placement: .header)
            layout
        }
    }

    /// `standard` draws no background image (Whimbrel stays as it was); the others draw it full-bleed
    /// behind the content, under its scrim.
    @ViewBuilder
    private var layout: some View {
        let archetype = SkinStore.shared.selected.archetype
        if archetype == .standard {
            StandardTodayLayout(content: content, actions: actions, slots: slots)
        } else {
            let (view, placed) = Self.archetypeView(archetype, content: content, actions: actions, composer: slots.composer)
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.xl) {
                    ForEach(Self.order(for: archetype), id: \.self) { part in
                        switch part {
                        case .slots:
                            VStack(alignment: .leading, spacing: Spacing.xl) {
                                if content.liveMeetingId != nil { slots.liveSlot }
                                if content.pendingApprovalCount > 0 { slots.decisionSlot }
                                slots.emptyState(for: content.hero)
                            }
                        case .archetype: view
                        case .leftovers:
                            TodayLeftovers(content: content, actions: actions, slots: slots, placed: placed)
                        }
                    }
                }
                .padding(Spacing.xl)
                .frame(maxWidth: 760, alignment: .leading)
                .frame(maxWidth: .infinity)
            }
            .background {
                ZStack {
                    Color.whimbrelBackgroundPage
                    SkinImagerySlot(placement: .background)
                }
            }
        }
    }

    /// Tasks 6-10 replace the placeholder, one case each.
    private static func archetypeView(
        _ archetype: SkinArchetype, content: TodayContent, actions: TodayActions, composer: AnyView?
    ) -> (AnyView, Set<TodaySection>) {
        switch archetype {
        case .terminal:
            return (AnyView(TerminalTodayLayout(content: content, actions: actions, composer: composer)), TerminalTodayLayout.placedSections)
        case .glass:
            return (AnyView(GlassTodayLayout(content: content, actions: actions, composer: composer)), GlassTodayLayout.placedSections)
        case .field:
            return (AnyView(FieldTodayLayout(content: content, actions: actions, composer: composer)), FieldTodayLayout.placedSections)
        case .almanac:
            return (AnyView(AlmanacTodayLayout(content: content, actions: actions, composer: composer)), AlmanacTodayLayout.placedSections)
        case .reference:
            return (AnyView(ReferenceTodayLayout(content: content, actions: actions, composer: composer)), ReferenceTodayLayout.placedSections)
        default:
            return (AnyView(PlaceholderArchetype(content: content, actions: actions, composer: composer)), PlaceholderArchetype.placedSections)
        }
    }
}

/// Stands in for every non-standard archetype until its layout exists: places nothing.
struct PlaceholderArchetype: TodayArchetypeView {
    let content: TodayContent
    let actions: TodayActions
    init(content: TodayContent, actions: TodayActions, composer: AnyView?) {
        self.content = content
        self.actions = actions
    }
    static var placedSections: Set<TodaySection> { [] }
    var body: some View { EmptyView() }
}

/// A plain Whimbrel-styled list of the sections an archetype did not place.
struct TodayLeftovers: View {
    let content: TodayContent
    let actions: TodayActions
    let slots: TodaySlots
    let placed: Set<TodaySection>
    private var voice: SkinVoice { SkinStore.shared.selected.voice }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xl) {
            ForEach(TodayChrome.leftovers(of: content, placed: placed), id: \.self) { section in
                self.section(section)
            }
        }
    }

    @ViewBuilder
    private func section(_ section: TodaySection) -> some View {
        switch section {
        case .greeting:
            Text(TodayLogic.greeting(name: content.name, now: content.now, calendar: .current, voice: voice)
                ?? content.now.formatted(.dateTime.weekday(.wide).month(.wide).day()))
                .font(.whimbrelPageTitle)
                .foregroundStyle(Color.whimbrelTextPrimary)
        case .composer:
            if let composer = slots.composer { composer }
        case .upNext:
            if let next = TodayRows.upNext(content) {
                block("Up next") {
                    HStack {
                        row(next.timeText, next.title)
                        if next.showsStart {
                            Button("Start") { actions.startMeeting(next.event) }.buttonStyle(.whimbrelPrimary)
                        }
                    }
                }
            }
        case .schedule:
            let schedule = TodayChrome.leftoverSchedule(of: content)
            block(voice.text(.upcomingHeader, name: "")) {
                ForEach(TodayRows.scheduleItems(schedule)) { item in
                    switch item {
                    case let .event(event):
                        HStack {
                            row(event.timeText, event.title)
                            if event.showsStart {
                                Button("Start") { actions.startMeeting(event.event) }.buttonStyle(.whimbrelPrimary)
                            }
                        }
                    case .separator: WhimbrelSectionLabel(text: TodayRows.heldSeparator)
                    case let .held(meeting): meetingRow(meeting)
                    }
                }
            }
        case .earlier:
            block(voice.text(.earlierHeader, name: "")) {
                ForEach(TodayRows.earlier(content), id: \.id) { meetingRow($0) }
            }
        case .threads:
            block("Open loops") {
                ForEach(content.attentionThreads) { thread in
                    Button { actions.openThread(thread.id) } label: {
                        Text(thread.subject ?? thread.lastMessageFrom)
                            .font(.whimbrelBody(.body))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                }
            }
        case .followups:
            block("Follow-ups") {
                ForEach(content.followups) { TodayFollowupRow(followup: $0, actions: actions) }
            }
        case .recap:
            block(voice.text(.recapHeader, name: "")) {
                switch TodayRows.recap(content) {
                case let .brief(text, _), let .digest(text):
                    body(text)
                case let .failed(message, retryEnabled):
                    Text(message).font(.whimbrelBody(.body)).foregroundStyle(Color.whimbrelError)
                    Button("Try again") { actions.generateDigest() }
                        .buttonStyle(.whimbrelSecondary)
                        .disabled(!retryEnabled)
                case .none:
                    EmptyView()
                }
            }
        case .devices:
            let others = content.deviceRows.filter { $0.deviceId != content.deviceId }.count
            HStack(spacing: Spacing.sm) {
                WhimbrelMetadataChip(text: content.deviceLabel(for: content.deviceId))
                if others > 0 {
                    Text("\(others) other device\(others == 1 ? "" : "s")")
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
        }
    }

    private func block<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: title)
            content()
        }
    }

    private func body(_ text: String) -> some View {
        Text(text).font(.whimbrelBody(.body)).foregroundStyle(Color.whimbrelTextSecondary)
    }

    private func row(_ time: String, _ title: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.md) {
            Text(time).font(.whimbrelBody(.subheadline).weight(.semibold)).foregroundStyle(Color.whimbrelTextSecondary)
            Text(title).font(.whimbrelBody(.body)).foregroundStyle(Color.whimbrelTextPrimary).lineLimit(2)
            Spacer(minLength: 0)
        }
    }

    private func meetingRow(_ meeting: TodayMeetingRowModel) -> some View {
        HStack {
            row(meeting.timeText, meeting.title)
            Button("Open") { actions.openMeeting(meeting.id) }.buttonStyle(.whimbrelQuiet)
        }
    }
}

/// One open Away Mode to-do with its Done button.
struct TodayFollowupRow: View {
    let followup: CoreMeetingFollowup
    let actions: TodayActions

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.md) {
            Text(followup.text)
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button("Done") { actions.completeFollowup(followup.id) }.buttonStyle(.whimbrelSecondary)
        }
    }
}
