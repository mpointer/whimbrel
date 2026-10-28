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
        case .idle, .listening: return "Provisional, about a minute behind"
        case .paused: return "Paused (Low Power Mode or heat)"
        case .unavailable(let reason): return reason
        }
    }

    private var emptyText: String {
        switch status {
        case .unavailable(let reason): return reason
        case .paused: return "Paused. It resumes automatically."
        default: return "Listening. The transcript starts appearing after the first minute."
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
    @ObservedObject var live: LiveTranscriptionService
    let meeting: CoreMeeting
    @State private var highlightedId: Int64?

    var body: some View {
        VStack(spacing: Spacing.sm) {
            LiveTranscriptPane(segments: live.segments, status: live.status, highlightedId: highlightedId)
            MeetingAskBar(session: live.askSession, meeting: meeting, segmentCount: live.segments.count, liveContext: true) { id in
                // Reset first so citing the same line twice re-scrolls.
                highlightedId = nil
                DispatchQueue.main.async { highlightedId = id }
            }
        }
        .padding(.horizontal, Spacing.xl)
        .padding(.top, Spacing.sm)
    }
}
