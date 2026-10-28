import SwiftUI

/// Today as it has always looked: greeting, then the live call or an Ask composer (and an empty
/// state when there is nothing to show), today's schedule card (events with Start and prep
/// status, plus meetings already held) and an "Earlier" catch-up list, with approvals, open
/// loops, the daily recap and a device footer below. Reads `TodayContent`, calls `TodayActions`
/// and places the native `TodaySlots`; it owns no store access.
struct StandardTodayLayout: View {
    let content: TodayContent
    let actions: TodayActions
    let slots: TodaySlots
    @State private var briefExpanded = false
    private var voice: SkinVoice { SkinStore.shared.selected.voice }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.xl) {
                brand
                header.skinBadge()
                if content.showComposer, let composer = slots.composer {
                    composer
                }
                heroSection
                scheduleCard
                earlierSection
                if content.pendingApprovalCount > 0 {
                    slots.decisionSlot
                }
                if !content.attentionThreads.isEmpty {
                    attentionThreadsSection
                }
                if !content.followups.isEmpty {
                    followupsSection
                }
                if let briefRecap = content.briefRecap {
                    briefRecapSection(briefRecap)
                } else if let digest = content.digest {
                    dailyRecapSection(digest)
                } else if let digestError = content.digestError {
                    dailyRecapErrorSection(digestError)
                }
                devicesFooter
            }
            .padding(Spacing.xl)
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(Color.whimbrelBackgroundPage)
    }

    @ViewBuilder
    private var heroSection: some View {
        switch content.hero {
        case .live:
            slots.liveSlot
        case .noCalendar, .noMeetings:
            slots.emptyState(for: content.hero)
        case .upcoming, .none:
            EmptyView()
        }
    }

    /// The app icon and wordmark, the same pairing as the website's. The icon is the app's own,
    /// so macOS picks the small-size art (head and bill) at this size.
    private var brand: some View {
        HStack(spacing: Spacing.sm) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 32, height: 32)
                .accessibilityHidden(true)
            Text("Whimbrel")
                .font(.whimbrelDisplay(.title3).weight(.semibold))
                .foregroundStyle(Color.whimbrelTextPrimary)
        }
    }

    /// A greeting under the date; with no usable profile name the date is the title.
    private var header: some View {
        let now = content.now
        let dateLine = now.formatted(.dateTime.weekday(.wide).month(.wide).day())
        let greeting = TodayLogic.greeting(name: content.name, now: now, calendar: .current, voice: voice)
        return VStack(alignment: .leading, spacing: Spacing.xs) {
            if greeting != nil {
                Text(dateLine)
                    .font(.whimbrelBody(.subheadline))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
            Text(greeting ?? dateLine)
                .font(.whimbrelPageTitle)
                .foregroundStyle(Color.whimbrelTextPrimary)
        }
    }

    /// Today's calendar events (earliest first) and the meetings already held today, in one card.
    /// Recomputed on every tick, so a finished event leaves the list without another publish.
    private var scheduleCard: some View {
        SwiftUI.TimelineView(.periodic(from: .now, by: 30)) { context in
            let now = content.pinnedNow ?? context.date
            let agenda = content.agenda(now: now)
            let upcoming = content.upcoming(now: now)
            let events = agenda.upcomingToday.compactMap { id in upcoming.first { $0.id == id } }
            if !events.isEmpty || !agenda.pastToday.isEmpty {
                VStack(alignment: .leading, spacing: Spacing.sm) {
                    WhimbrelSectionLabel(text: voice.text(.upcomingHeader, name: ""))
                    VStack(alignment: .leading, spacing: Spacing.md) {
                        ForEach(events, id: \.id) { eventRow($0, isNext: $0.id == events.first?.id, now: now) }
                        if !events.isEmpty, !agenda.pastToday.isEmpty {
                            WhimbrelSectionLabel(text: "Earlier today")
                        }
                        ForEach(agenda.pastToday, id: \.self) { recordedRow($0) }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .whimbrelCard()
                }
            }
        }
    }

    /// A meeting already held today, laid out like an event row (bar, time, title, attendees, Open).
    @ViewBuilder
    private func recordedRow(_ id: Int64) -> some View {
        if let meeting = content.recentMeetings.first(where: { $0.id == id }) {
            let transcribing = content.processingMeetingId == id
            HStack(alignment: .top, spacing: Spacing.md) {
                AccentBarRow(color: .whimbrelBorder) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(MeetingRowText.time(meeting.startUnixMs))
                            .font(.whimbrelBody(.subheadline).weight(.semibold))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                        Text(meeting.title)
                            .font(.whimbrelBody(.body))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                            .lineLimit(2)
                        let subtitle = transcribing ? "Transcribing…" : MeetingRowText.subtitle(for: meeting)
                        if !subtitle.isEmpty {
                            Text(subtitle)
                                .font(.whimbrelBody(.subheadline))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                                .lineLimit(1)
                        }
                    }
                }
                Spacer(minLength: Spacing.md)
                Button("Open") { actions.openMeeting(id) }
                    .buttonStyle(.whimbrelQuiet)
            }
        }
    }

    private func eventRow(_ event: TodayUpcoming, isNext: Bool, now: Date) -> some View {
        let source = content.events.first { $0.id == event.id }
        let relative = source.flatMap { TodayLogic.relativeStart(event.start, end: $0.endDate, now: now) }
        let attendees = MeetingRowText.attendeeSummary(source?.attendees ?? [])
        return HStack(alignment: .top, spacing: Spacing.md) {
            AccentBarRow(color: .whimbrelAccentPrimary) {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: Spacing.sm) {
                        Text(MeetingRowText.time(event.start))
                            .font(.whimbrelBody(.subheadline).weight(.semibold))
                            .foregroundStyle(Color.whimbrelAccentPrimary)
                        if let relative {
                            Text(relative)
                                .font(.whimbrelBody(.subheadline))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                        }
                    }
                    Text(event.title)
                        .font(.whimbrelBody(.body))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .lineLimit(2)
                    if !attendees.isEmpty {
                        Text(attendees)
                            .font(.whimbrelBody(.subheadline))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                            .lineLimit(1)
                    }
                    if isNext { prepLine(for: event) }
                }
            }
            Spacer(minLength: Spacing.md)
            if content.activeMeetingId == nil {
                Button("Start") { actions.startMeeting(event) }
                    .buttonStyle(.whimbrelPrimary)
            }
        }
    }

    /// Only the next event has prep data: a status label, and its brief when one is ready.
    @ViewBuilder
    private func prepLine(for event: TodayUpcoming) -> some View {
        if let prep = content.nextPrep, prep.eventId == event.id, prep.brief != nil {
            HStack(spacing: Spacing.sm) {
                Text("Prep ready")
                    .font(.whimbrelBody(.subheadline))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Button(briefExpanded ? "Hide brief" : "Open brief") { briefExpanded.toggle() }
                    .buttonStyle(.whimbrelQuiet)
            }
            if briefExpanded { MeetingPrepView(prep: prep) }
        } else {
            Text("No prep yet")
                .font(.whimbrelBody(.subheadline))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    @ViewBuilder
    private var earlierSection: some View {
        let agenda = content.agenda
        if !agenda.earlier.isEmpty {
            VStack(alignment: .leading, spacing: Spacing.sm) {
                WhimbrelSectionLabel(text: voice.text(.earlierHeader, name: ""))
                ForEach(agenda.earlier, id: \.self) { meetingRow($0) }
            }
        }
    }

    @ViewBuilder
    private func meetingRow(_ id: Int64) -> some View {
        if let meeting = content.recentMeetings.first(where: { $0.id == id }) {
            let transcribing = content.processingMeetingId == id
            MeetingRowView(
                title: meeting.title,
                subtitle: transcribing ? "Transcribing…" : MeetingRowText.subtitle(for: meeting),
                time: MeetingRowText.time(meeting.startUnixMs),
                actionLabel: "Open",
                action: { actions.openMeeting(id) }
            )
        }
    }

    /// This Mac's activity on one line, plus how many other devices reported.
    private var devicesFooter: some View {
        let rows = content.deviceRows
        let mine = rows.first { $0.deviceId == content.deviceId }
        let others = rows.filter { $0.deviceId != content.deviceId }.count
        var line = mine.map(Self.activityLine) ?? voice.text(.emptyEarlier, name: "")
        if others > 0 { line += " · \(others) other device\(others == 1 ? "" : "s")" }
        return HStack(spacing: Spacing.sm) {
            WhimbrelMetadataChip(text: content.deviceLabel(for: content.deviceId))
                .help(content.deviceId)
            Text(line)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
        .accessibilityElement(children: .combine)
    }

    private func dailyRecapSection(_ digest: CoreDailyDigest) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: voice.text(.recapHeader, name: ""))
            Text(digest.summary).font(.whimbrelBody(.body)).foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    private func briefRecapSection(_ run: CoreRoutineRunWithName) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: voice.text(.recapHeader, name: ""))
            Text(run.outputSummary).font(.whimbrelBody(.body)).foregroundStyle(Color.whimbrelTextSecondary)
            if let origin = run.originDeviceId {
                Text("From \(content.deviceLabel(for: origin))").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    private func dailyRecapErrorSection(_ message: String) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: voice.text(.recapHeader, name: ""))
            Text(message).font(.whimbrelBody(.body)).foregroundStyle(Color.whimbrelError)
            Button("Try again") { actions.generateDigest() }
                .buttonStyle(.whimbrelSecondary)
                .disabled(content.isGeneratingDigest)
        }
    }

    private var attentionThreadsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: "Open loops")
            ForEach(content.attentionThreads) { thread in
                Button {
                    actions.openThread(thread.id)
                } label: {
                    Text(thread.subject ?? thread.lastMessageFrom)
                        .font(.whimbrelBody(.body))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Open this thread")
            }
        }
    }

    private var followupsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            WhimbrelSectionLabel(text: "Follow-ups")
            ForEach(content.followups) { followup in
                TodayFollowupRow(followup: followup, actions: actions)
            }
        }
    }

    static func activityLine(_ row: CoreDeviceActivity) -> String {
        guard row.captures > 0 else { return SkinStore.shared.selected.voice.text(.emptyEarlier, name: "") }
        let last = Date(timeIntervalSince1970: TimeInterval(row.lastTsUnixMs) / 1000)
        let noun = row.captures == 1 ? "capture" : "captures"
        return "\(row.captures.formatted()) \(noun) · last \(last.formatted(date: .omitted, time: .shortened))"
    }
}
