import SwiftUI

/// One meeting: where and when it was recorded, then a Summary tab and a Transcript tab.
struct MeetingDetailView: View {
    enum Tab: String, CaseIterable { case summary = "Summary", transcript = "Transcript" }

    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    @StateObject private var model: MeetingDetailModel
    @State private var tab: Tab = .summary
    @State private var attendeesExpanded = false

    init(meetingId: Int64, sync: SyncModel, labels: DeviceLabels) {
        _sync = ObservedObject(wrappedValue: sync)
        self.labels = labels
        _model = StateObject(wrappedValue: MeetingDetailModel(meetingId: meetingId))
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-WhimbrelDemoTranscript") { _tab = State(initialValue: .transcript) }
        #endif
    }

    /// The components' words for this page. Every summary line here is Whimbrel's, said once in the
    /// legend above them, so a screen reader is not told "Whimbrel wrote" before each line.
    private var noteStrings: WStrings {
        var s = WStrings.app
        s.whimbrelWrote = { $0 }
        return s
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WTokens.Space.lg) {
                if let meeting = model.meeting {
                    VStack(alignment: .leading, spacing: WTokens.Space.sm) {
                        header(meeting)
                        attendees(meeting)
                    }
                    Picker("Section", selection: $tab) {
                        ForEach(Tab.allCases, id: \.self) { Text($0 == .summary ? String(localized: "Summary") : String(localized: "Transcript")).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    switch tab {
                    case .summary: summary(meeting)
                    case .transcript: transcript
                    }
                } else if model.loaded {
                    WStateView(kind: .empty, title: String(localized: "This meeting isn't on this iPhone."))
                } else {
                    WStateView(kind: .loading, title: String(localized: "Loading…"))
                }
            }
            .padding(.horizontal, WTokens.Space.lg)
            .padding(.vertical, WTokens.Space.md)
            .frame(maxWidth: WTokens.Size.contentMax, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .environment(\.wStrings, noteStrings)
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationBarTitleDisplayMode(.inline)
        .task(id: sync.readRevision) { await model.reload(sync: sync) }
    }

    private func header(_ meeting: PhoneMeeting) -> some View {
        let start = Date(timeIntervalSince1970: Double(meeting.startUnixMs) / 1000)
        var where_ = [String(localized: "Recorded on \(labels.label(for: meeting.deviceId))")]
        if let copies = meeting.alsoRecorded, !copies.isEmpty {
            where_.append("also on " + copies.map { labels.label(for: $0.deviceId) }.joined(separator: ", "))
        }
        return VStack(alignment: .leading, spacing: 6) {
            Text(meeting.title)
                .wReading(WTokens.TypeScale.readingTitle)
                .foregroundStyle(Color.theme(.textPrimary))
            Text(start.formatted(.dateTime.weekday(.wide).month(.abbreviated).day().hour().minute()))
                .wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
            Text(where_.joined(separator: " · "))
                .wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }

    /// Three names and a "+N more" toggle until expanded; per meeting (this view is one meeting's page).
    @ViewBuilder
    private func attendees(_ meeting: PhoneMeeting) -> some View {
        if !meeting.attendees.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text(ReadLogic.attendeeLine(meeting.attendees, expanded: attendeesExpanded))
                    .wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
                if let toggle = ReadLogic.attendeeToggle(meeting.attendees, expanded: attendeesExpanded) {
                    Button(toggle.label) { attendeesExpanded.toggle() }
                        .wType(WTokens.TypeScale.secondary)
                        .frame(minHeight: WTokens.Size.target)
                        .accessibilityLabel(toggle.accessibilityLabel)
                        .accessibilityInputLabels([toggle.label, toggle.accessibilityLabel])
                }
            }
        }
    }

    @ViewBuilder
    private func summary(_ meeting: PhoneMeeting) -> some View {
        let actions: [(String, Bool?)] = model.actionItems.isEmpty ? meeting.summaryActionItems.map { ($0, nil) } : model.actionItems.map { ($0.text, $0.completed) }
        if meeting.summaryTldr == nil, meeting.summaryDecisions.isEmpty, actions.isEmpty, meeting.summaryOpenQuestions.isEmpty {
            WStateView(kind: .empty, title: String(localized: "No summary yet"))
        } else {
            VStack(alignment: .leading, spacing: WTokens.Space.lg) {
                legend
                if let tldr = meeting.summaryTldr {
                    section("TL;DR") { WNoteBlock(kind: .paragraph, author: .whimbrel, text: tldr) }
                }
                if !meeting.summaryDecisions.isEmpty { bullets(String(localized: "Decisions"), meeting.summaryDecisions) }
                if !actions.isEmpty {
                    section(String(localized: "Action items")) {
                        ForEach(Array(actions.enumerated()), id: \.offset) { _, item in actionLine(item.0, done: item.1) }
                    }
                }
                if !meeting.summaryOpenQuestions.isEmpty { bullets(String(localized: "Open questions"), meeting.summaryOpenQuestions) }
            }
        }
    }

    /// Says once, in the ink and rule the lines below use, who wrote them.
    private var legend: some View {
        // The rule is as tall as the words, at any text size.
        HStack(spacing: WTokens.Space.sm) {
            Rectangle().fill(WFixed.aiInk.color).frame(width: 2).accessibilityHidden(true)
            Text("Written by Whimbrel").wType(WTokens.TypeScale.caption).fontWeight(.semibold).foregroundStyle(WFixed.aiInk.color)
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func section<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: WTokens.Space.xxs) {
            WNoteBlock(kind: .heading, author: .you, text: title)
            content()
        }
    }

    private func bullets(_ title: String, _ lines: [String]) -> some View {
        section(title) {
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in WNoteBlock(kind: .bullet, author: .whimbrel, text: line) }
        }
    }

    /// An action item as Whimbrel wrote it. It shows whether it is done and cannot be changed here,
    /// so it is a line with a mark, not a control. `done` nil (no synced state) draws a bullet.
    @ViewBuilder
    private func actionLine(_ text: String, done: Bool?) -> some View {
        if let done {
            HStack(alignment: .top, spacing: WTokens.Space.sm) {
                Rectangle().fill(WFixed.aiInk.color).frame(width: 2)
                Image(systemName: done ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(done ? Color.theme(.success) : WFixed.aiInk.color)
                    .padding(.top, 4)
                Text(text).wReading(WTokens.TypeScale.reading).foregroundStyle(WFixed.aiInk.color)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.vertical, WTokens.Space.xs)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(done ? String(localized: "Done: \(text)") : text)
        } else {
            WNoteBlock(kind: .bullet, author: .whimbrel, text: text)
        }
    }

    @ViewBuilder
    private var transcript: some View {
        if model.loaded, model.transcriptTotal == 0 {
            WStateView(kind: .empty, title: String(localized: "No transcript on this phone"))
        } else {
            LazyVStack(alignment: .leading, spacing: WTokens.Space.xs) {
                ForEach(model.transcript) { segment in
                    WTranscriptLine(time: timestamp(segment.startCs), speaker: segment.speakerLabel.isEmpty ? nil : segment.speakerLabel, text: segment.text)
                }
                if model.hasMoreTranscript {
                    Button("Load more") { Task { await model.loadMoreTranscript(sync: sync) } }
                        .buttonStyle(.w(.secondary, fill: true))
                }
            }
        }
    }

    private func timestamp(_ centiseconds: Int64) -> String {
        let seconds = Int(centiseconds / 100)
        return String(format: "%d:%02d", seconds / 60, seconds % 60)
    }
}
