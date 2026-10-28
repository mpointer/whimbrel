import SwiftUI

/// One meeting: where and when it was recorded, then a Summary tab and a Transcript tab.
struct MeetingDetailView: View {
    enum Tab: String, CaseIterable { case summary = "Summary", transcript = "Transcript" }

    @ObservedObject var sync: SyncModel
    let labels: DeviceLabels
    @StateObject private var model: MeetingDetailModel
    @State private var tab: Tab = .summary

    init(meetingId: Int64, sync: SyncModel, labels: DeviceLabels) {
        _sync = ObservedObject(wrappedValue: sync)
        self.labels = labels
        _model = StateObject(wrappedValue: MeetingDetailModel(meetingId: meetingId))
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-WhimbrelDemoTranscript") { _tab = State(initialValue: .transcript) }
        #endif
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let meeting = model.meeting {
                    header(meeting)
                    Picker("Section", selection: $tab) {
                        ForEach(Tab.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    switch tab {
                    case .summary: summary(meeting)
                    case .transcript: transcript
                    }
                } else {
                    Text(model.loaded ? "This meeting isn't on this iPhone." : "Loading…")
                        .foregroundStyle(Color.theme(.textSecondary))
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationBarTitleDisplayMode(.inline)
        .task(id: sync.readRevision) { await model.reload(sync: sync) }
    }

    private func header(_ meeting: PhoneMeeting) -> some View {
        let start = Date(timeIntervalSince1970: Double(meeting.startUnixMs) / 1000)
        var where_ = ["Recorded on \(labels.label(for: meeting.deviceId))"]
        if let copies = meeting.alsoRecorded, !copies.isEmpty {
            where_.append("also on " + copies.map { labels.label(for: $0.deviceId) }.joined(separator: ", "))
        }
        return VStack(alignment: .leading, spacing: 6) {
            Text(meeting.title)
                .font(.system(.title2, design: .serif))
                .foregroundStyle(Color.theme(.textPrimary))
            Text(start.formatted(.dateTime.weekday(.wide).month(.abbreviated).day().hour().minute()))
                .font(.subheadline).foregroundStyle(Color.theme(.textSecondary))
            Text(where_.joined(separator: " · "))
                .font(.subheadline).foregroundStyle(Color.theme(.textSecondary))
            if !meeting.attendees.isEmpty {
                Text(meeting.attendees.joined(separator: ", "))
                    .font(.subheadline).foregroundStyle(Color.theme(.textSecondary))
            }
        }
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private func summary(_ meeting: PhoneMeeting) -> some View {
        let actionTexts = model.actionItems.isEmpty ? meeting.summaryActionItems.map { ($0, false) } : model.actionItems.map { ($0.text, $0.completed) }
        if meeting.summaryTldr == nil, meeting.summaryDecisions.isEmpty, actionTexts.isEmpty, meeting.summaryOpenQuestions.isEmpty {
            Text("No summary yet").foregroundStyle(Color.theme(.textSecondary))
        } else {
            if let tldr = meeting.summaryTldr { block("TL;DR") { Text(tldr).foregroundStyle(Color.theme(.textPrimary)) } }
            if !meeting.summaryDecisions.isEmpty { list("Decisions", meeting.summaryDecisions.map { ($0, nil) }) }
            if !actionTexts.isEmpty { list("Action items", actionTexts.map { ($0.0, $0.1) }) }
            if !meeting.summaryOpenQuestions.isEmpty { list("Open questions", meeting.summaryOpenQuestions.map { ($0, nil) }) }
        }
    }

    @ViewBuilder
    private var transcript: some View {
        if model.loaded, model.transcriptTotal == 0 {
            Text("No transcript on this phone").foregroundStyle(Color.theme(.textSecondary))
        } else {
            LazyVStack(alignment: .leading, spacing: 12) {
                ForEach(model.transcript) { segment in
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(segment.speakerLabel) · \(timestamp(segment.startCs))")
                            .font(.caption.weight(.semibold)).foregroundStyle(Color.theme(.textSecondary))
                        Text(segment.text).foregroundStyle(Color.theme(.textPrimary))
                    }
                    .accessibilityElement(children: .combine)
                }
                if model.hasMoreTranscript {
                    Button("Load more") { Task { await model.loadMoreTranscript(sync: sync) } }
                        .frame(maxWidth: .infinity, minHeight: 44)
                }
            }
        }
    }

    private func timestamp(_ centiseconds: Int64) -> String {
        let seconds = Int(centiseconds / 100)
        return String(format: "%d:%02d", seconds / 60, seconds % 60)
    }

    private func block<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title.uppercased()).font(.caption.weight(.semibold)).tracking(0.6).foregroundStyle(Color.theme(.textSecondary))
            content()
        }
    }

    /// `done` nil draws a bullet; true or false draws a check or an empty circle.
    private func list(_ title: String, _ lines: [(String, Bool?)]) -> some View {
        block(title) {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        switch line.1 {
                        case .none: Text("•").foregroundStyle(Color.theme(.textSecondary))
                        case .some(true): Image(systemName: "checkmark.circle.fill").foregroundStyle(Color.theme(.success))
                        case .some(false): Image(systemName: "circle").foregroundStyle(Color.theme(.textSecondary))
                        }
                        Text(line.0).foregroundStyle(Color.theme(.textPrimary))
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(line.1 == true ? "Done: \(line.0)" : line.0)
                }
            }
        }
    }
}
