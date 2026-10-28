import SwiftUI

/// A centred page inside a double-rule border: small-caps headers, ❦ between sections, the skin's
/// serif throughout. Places every section; the hero empty states and the native slots are
/// `TodayChrome`'s.
struct AlmanacTodayLayout: TodayArchetypeView {
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

    var body: some View {
        let sections = content.nonEmptySections()
        VStack(spacing: Spacing.lg) {
            header.skinBadge()
            if sections.contains(.composer), let composer { composer }
            if sections.contains(.upNext), let next = TodayRows.upNext(content) {
                divider
                section(voice.text(.upNextHeader, name: "")) { eventRow(next) }
            }
            if sections.contains(.schedule) {
                // `.upNext` is placed above and shows `upcomingToday.first`, so the schedule lists the rest.
                let schedule = TodayRows.schedule(content, upNextPlaced: true)
                if !schedule.upcoming.isEmpty || !schedule.held.isEmpty {
                    divider
                    section(voice.text(.upcomingHeader, name: "")) {
                        ForEach(TodayRows.scheduleItems(schedule)) { item in
                            switch item {
                            case let .event(event): eventRow(event)
                            case .separator: SkinText(TodayRows.heldSeparator, role: .label, size: 12, smallCaps: true).foregroundStyle(Color.whimbrelTextSecondary)
                            case let .held(meeting): meetingRow(meeting)
                            }
                        }
                    }
                }
            }
            if sections.contains(.earlier) {
                divider
                section(voice.text(.earlierHeader, name: "")) {
                    ForEach(TodayRows.earlier(content), id: \.id) { meetingRow($0) }
                }
            }
            if sections.contains(.threads) {
                divider
                section("Open loops") {
                    ForEach(content.attentionThreads) { thread in
                        Button { actions.openThread(thread.id) } label: {
                            SkinText(thread.subject ?? thread.lastMessageFrom, lines: 2)
                                .foregroundStyle(Color.whimbrelTextPrimary)
                                .frame(maxWidth: .infinity)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .help("Open this thread")
                    }
                }
            }
            if sections.contains(.recap) {
                divider
                section(voice.text(.recapHeader, name: "")) { recap }
            }
            if sections.contains(.devices) {
                divider
                devicesFooter
            }
        }
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, Spacing.xl)
        .padding(.vertical, Spacing.xl)
        .background(Color.whimbrelBackgroundCard)
        .overlay(doubleRule)
    }

    // MARK: pieces

    /// Two nested rules, the outer heavier: the skin's `double` border.
    private var doubleRule: some View {
        let width = max(skin.shape.borderWidth, 1)
        return ZStack {
            RoundedRectangle(cornerRadius: skin.shape.radius).inset(by: 4).stroke(Color.whimbrelBorder, lineWidth: width * 2)
            RoundedRectangle(cornerRadius: skin.shape.radius).inset(by: 10).stroke(Color.whimbrelBorder, lineWidth: width)
        }
        .accessibilityHidden(true)
    }

    /// A date over the greeting; with no usable profile name the date is the title.
    private var header: some View {
        let dateLine = content.now.formatted(.dateTime.weekday(.wide).month(.wide).day())
        let greeting = TodayLogic.greeting(name: content.name, now: content.now, calendar: .current, voice: voice)
        return VStack(spacing: Spacing.xs) {
            if greeting != nil {
                SkinText(dateLine, role: .label, size: 12, smallCaps: true).foregroundStyle(Color.whimbrelTextSecondary)
            }
            SkinText(greeting ?? dateLine, role: .display, lines: 2, size: 26).foregroundStyle(Color.whimbrelTextPrimary)
        }
    }

    /// ❦ between two thin rules. The glyph is decoration, hidden from VoiceOver.
    @ViewBuilder
    private var divider: some View {
        if skin.decoration.dividers != .none {
            HStack(spacing: Spacing.md) {
                Rectangle().fill(Color.whimbrelBorder).frame(height: 1)
                Text("\u{2766}").font(skin.typography.display.font(size: 16)).foregroundStyle(Color.whimbrelAccentPrimary).accessibilityHidden(true)
                Rectangle().fill(Color.whimbrelBorder).frame(height: 1)
            }
            .accessibilityHidden(true)
        }
    }

    private func section<C: View>(_ title: String, @ViewBuilder _ rows: () -> C) -> some View {
        VStack(spacing: Spacing.md) {
            SkinText(skin.typography.caps.apply(title), role: .label, size: 13, smallCaps: skin.typography.caps == .small)
                .tracking(1)
                .foregroundStyle(Color.whimbrelAccentPrimary)
            rows()
        }
        .frame(maxWidth: .infinity)
    }

    private func eventRow(_ event: TodayEventRowModel) -> some View {
        VStack(spacing: 2) {
            SkinText(event.timeText, role: .numerals).foregroundStyle(Color.whimbrelAccentPrimary)
            SkinText(event.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
            if let relative = event.relative { SkinText(relative, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            if !event.attendees.isEmpty { SkinText(event.attendees, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            SkinPrepLine(prep: event.prep).multilineTextAlignment(.leading)
            if event.showsStart {
                Button("Start") { actions.startMeeting(event.event) }.buttonStyle(.whimbrelPrimary).padding(.top, Spacing.xs)
            }
        }
        .frame(maxWidth: .infinity)
    }

    private func meetingRow(_ meeting: TodayMeetingRowModel) -> some View {
        VStack(spacing: 2) {
            SkinText(meeting.timeText, role: .numerals).foregroundStyle(Color.whimbrelTextSecondary)
            SkinText(meeting.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
            if !meeting.subtitle.isEmpty { SkinText(meeting.subtitle, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            Button("Open") { actions.openMeeting(meeting.id) }.buttonStyle(.whimbrelQuiet).padding(.top, Spacing.xs)
        }
        .frame(maxWidth: .infinity)
    }

    /// Recap text is body text: no line limit, and left-aligned in a centred, width-capped column.
    @ViewBuilder
    private var recap: some View {
        switch TodayRows.recap(content) {
        case let .brief(text, origin):
            recapText(text)
            if let origin { SkinText("From \(origin)", role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
        case let .digest(text):
            recapText(text)
        case let .failed(message, retryEnabled):
            SkinText(message, lines: 6).foregroundStyle(Color.whimbrelError)
            Button("Try again") { actions.generateDigest() }
                .buttonStyle(.whimbrelSecondary)
                .disabled(!retryEnabled)
        case .none:
            EmptyView()
        }
    }

    private func recapText(_ text: String) -> some View {
        SkinText(text, lines: .max)
            .foregroundStyle(Color.whimbrelTextSecondary)
            .multilineTextAlignment(.leading)
            .frame(maxWidth: 520, alignment: .leading)
    }

    /// This Mac's activity on one line, plus how many other devices reported.
    private var devicesFooter: some View {
        let mine = content.deviceRows.first { $0.deviceId == content.deviceId }
        let others = content.deviceRows.filter { $0.deviceId != content.deviceId }.count
        // Whimbrel's own wording for "no activity", not the skin's voice (Standard's footer shows the same line).
        var line = mine.flatMap { $0.captures > 0 ? StandardTodayLayout.activityLine($0) : nil }
            ?? SkinDefaults.whimbrel.voice.text(.emptyEarlier, name: "")
        if others > 0 { line += " · \(others) other device\(others == 1 ? "" : "s")" }
        return VStack(spacing: Spacing.xs) {
            WhimbrelMetadataChip(text: content.deviceLabel(for: content.deviceId)).help(content.deviceId)
            SkinText(line, role: .label).foregroundStyle(Color.whimbrelTextSecondary)
        }
        .accessibilityElement(children: .combine)
    }
}
