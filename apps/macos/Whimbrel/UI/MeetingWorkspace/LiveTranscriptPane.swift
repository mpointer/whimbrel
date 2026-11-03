import SwiftUI

/// The provisional transcript shown while a meeting is being recorded. Lines are Me / Others
/// with a time; the header says the text is provisional and about a minute behind. See
/// docs/superpowers/specs/2026-10-06-live-transcription-design.md.
struct LiveTranscriptPane: View {
    let segments: [CoreLiveSegment]
    let status: LiveTranscriptionService.Status
    /// A cited line, as the negative id the meeting Ask returns for a live row (`-rowid`).
    var highlightedId: Int64?
    @State private var expanded = true

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Button { expanded.toggle() } label: {
                HStack(spacing: Spacing.xs) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right").font(.caption)
                    Text("Live transcript").font(.whimbrelBody(.callout).bold())
                    Text(subtitle).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    Spacer()
                }
                .foregroundStyle(Color.whimbrelTextPrimary)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("live-transcript-toggle")
            if expanded {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: Spacing.sm) {
                            if segments.isEmpty {
                                Text(emptyText).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                            }
                            ForEach(segments) { segment in
                                HStack(alignment: .firstTextBaseline, spacing: Spacing.sm) {
                                    Text("\(segment.speakerLabel) · \(Self.clock(segment.startCs))")
                                        .font(.whimbrelBody(.caption))
                                        .foregroundStyle(Color.whimbrelTextSecondary)
                                        .frame(width: 96, alignment: .leading)
                                    Text(segment.text)
                                        .font(.whimbrelBody(.body))
                                        .foregroundStyle(Color.whimbrelTextPrimary)
                                        .fixedSize(horizontal: false, vertical: true)
                                }
                                .padding(Spacing.xs)
                                .background(highlightedId == -segment.id ? Color.whimbrelHighlight : Color.clear)
                                .id(segment.id)
                            }
                            Color.clear.frame(height: 1).id("live-bottom")
                        }
                    }
                    .frame(maxHeight: 220)
                    .onChange(of: segments.count) { _, _ in proxy.scrollTo("live-bottom", anchor: .bottom) }
                    .onChange(of: highlightedId) { _, id in
                        guard let id else { return }
                        withAnimation { proxy.scrollTo(-id, anchor: .center) }
                    }
                }
            }
        }
        .padding(Spacing.md)
        .background(Color.whimbrelBackgroundCard)
        .clipShape(RoundedRectangle(cornerRadius: Radius.card))
        .overlay(RoundedRectangle(cornerRadius: Radius.card).stroke(Color.whimbrelBorder))
    }

    private var subtitle: String {
        switch status {
        case .idle, .listening: return String(localized: "Provisional, about a minute behind")
        case .paused: return String(localized: "Paused (Low Power Mode or heat)")
        case .unavailable(let reason): return reason
        }
    }

    private var emptyText: String {
        switch status {
        case .unavailable(let reason): return reason
        case .paused: return String(localized: "Paused. It resumes automatically.")
        default: return String(localized: "Listening. The transcript starts appearing after the first minute.")
        }
    }

    private static func clock(_ startCs: Int64) -> String {
        let seconds = Int(startCs / 100)
        return String(format: "%d:%02d", seconds / 60, seconds % 60)
    }
}

/// What the During state shows when live transcription is on: the live pane and the meeting
/// Ask bar over the transcript so far. Built where `appState` is in the environment and
/// passed into `ActiveMeetingView` as a slot.
struct LiveMeetingSection: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject var live: LiveTranscriptionService
    let meeting: CoreMeeting
    @State private var highlightedId: Int64?
    @State private var alerts: [LiveAlertsSection.Row] = []
    /// Live rows count from this part's start; alerts and links from the meeting's. Nil until loaded.
    @State private var partOffsetCs: Int64?

    var body: some View {
        VStack(spacing: Spacing.sm) {
            LiveTranscriptPane(segments: live.segments, status: live.status, highlightedId: highlightedId)
            LiveAlertsSection(
                rows: alerts,
                paused: LiveAlertsSection.pausedLabel(
                    status: live.status,
                    alertsOn: LiveQuestionWatcher.shouldRun(settings: appState.liveQuestionSettings, names: appState.profile.liveQuestionNames)),
                onJump: seek
            )
            MeetingAskBar(session: live.askSession, meeting: meeting, segmentCount: live.segments.count, liveContext: true) { id in
                highlight(id)
            }
        }
        .padding(.horizontal, Spacing.xl)
        .padding(.top, Spacing.sm)
        .onAppear {
            loadAlerts()
            loadPartOffset()
        }
        .onReceive(LiveAlertsSection.refresh) { _ in
            let alertsOn = LiveQuestionWatcher.shouldRun(settings: appState.liveQuestionSettings, names: appState.profile.liveQuestionNames)
            if LiveAlertsSection.shouldPoll(recording: meeting.id == appState.activeMeetingId, alertsOn: alertsOn) { loadAlerts() }
        }
        // A notification or link for this meeting: scroll to that moment once its line exists.
        .onReceive(appState.$pendingMeetingSeek) { consumeSeek($0) }
        .onChange(of: live.segments.count) { _, _ in consumeSeek(appState.pendingMeetingSeek) }
        .onChange(of: partOffsetCs) { _, _ in consumeSeek(appState.pendingMeetingSeek) }
    }

    private func loadAlerts() {
        LiveAlertsSection.load(store: appState.coreStore, meetingId: meeting.id) { if alerts != $0 { alerts = $0 } }
    }

    private func loadPartOffset() {
        guard let store = appState.coreStore else { return partOffsetCs = 0 }
        let meetingId = meeting.id
        CoreMemoryStore.accessQueue.async {
            let offset = LiveQuestionWatcher.partOffsetCs(store.meetingParts(meetingId: meetingId) ?? [])
            DispatchQueue.main.async { partOffsetCs = offset }
        }
    }

    /// Highlights a live line by its cited id (`-rowid`).
    private func highlight(_ id: Int64) {
        // Reset first so citing the same line twice re-scrolls.
        highlightedId = nil
        DispatchQueue.main.async { highlightedId = id }
    }

    /// `atCs` is meeting-absolute; the live rows are part-relative.
    private func seek(_ atCs: Int64) {
        if let id = LiveAlertsSection.nearest(live.segments.map { ($0.id, $0.startCs) }, to: atCs - (partOffsetCs ?? 0)) { highlight(-id) }
    }

    private func consumeSeek(_ request: (meetingId: Int64, atCs: Int64)?) {
        guard let request, request.meetingId == meeting.id, !live.segments.isEmpty, partOffsetCs != nil else { return }
        seek(request.atCs)
        DispatchQueue.main.async { appState.pendingMeetingSeek = nil }
    }
}
