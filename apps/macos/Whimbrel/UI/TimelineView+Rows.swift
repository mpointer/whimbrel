import AppKit
import SwiftUI
import os

/// Library list rows (captures, meetings, transcript hits, episodes) and their
/// formatters.
extension TimelineView {
    @ViewBuilder
    func entryRow(for entry: TimelineEntry) -> some View {
        switch entry {
        case .capture(let item):
            AccentBarRow(color: Color.whimbrelAccentSecondary) {
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    HStack {
                        Text(item.appBundleId).font(.whimbrelBody(.caption).bold())
                            .foregroundStyle(Color.whimbrelTextPrimary)
                        Text(item.windowTitle).font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                        Spacer()
                        Text(Self.formatted(item.tsUnixMs)).font(.whimbrelBody(.caption2))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                        DeviceChip(deviceId: item.deviceId)
                    }
                    Text(item.text)
                        .font(.whimbrelBody(.callout))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .lineLimit(2)
                    if let url = item.url {
                        Text(url).font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelLink)
                    }
                }
            }
            .whimbrelCard()
            .contentShape(Rectangle())
            .onTapGesture { openWindow(id: "capture-detail", value: item.id) }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { openWindow(id: "capture-detail", value: item.id) }
        case .meeting(let meeting):
            AccentBarRow(color: Color.whimbrelAccentPrimary) {
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    HStack {
                        Text(meeting.title)
                            .font(.whimbrelBody(.headline))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                        Spacer()
                        statusLabel(for: meeting.status)
                            .font(.whimbrelBody(.caption2))
                        DeviceChip(deviceId: meeting.deviceId)
                    }
                    Text(Self.formattedMeetingTime(startUnixMs: meeting.startUnixMs, endUnixMs: meeting.endUnixMs))
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                    Text("\(meeting.attendees.count) attendee\(meeting.attendees.count == 1 ? "" : "s")")
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
            .whimbrelCard()
            .contentShape(Rectangle())
            .onTapGesture { openWindow(id: "meeting-detail", value: meeting.id) }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { openWindow(id: "meeting-detail", value: meeting.id) }
            .contextMenu {
                // Same vocabulary as the shared `MeetingOptionsMenu`, but kept as a
                // plain context menu: right-clicking a list row is a different
                // interaction from that view's toolbar button.
                Button("Open in new window") {
                    openWindow(id: "meeting-detail", value: meeting.id)
                }
                Divider()
                if meeting.status != .recording {
                    Button("Forget meeting…", role: .destructive) {
                        meetingPendingDeletion = meeting
                    }
                }
            }
        case .transcriptSegment(let segment):
            // whimbrelAccentPrimary, matching the .meeting row's color — transcript
            // segments are meeting content, not screen-text capture content.
            AccentBarRow(color: Color.whimbrelAccentPrimary) {
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    HStack {
                        Text(segment.meetingTitle).font(.whimbrelBody(.caption).bold())
                            .foregroundStyle(Color.whimbrelTextPrimary)
                        Text(segment.speakerLabel).font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                        Spacer()
                        Text(Self.formattedElapsed(startCs: segment.startCs)).font(.whimbrelBody(.caption2))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                    }
                    Text(segment.text)
                        .font(.whimbrelBody(.callout))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .lineLimit(2)
                }
            }
            .whimbrelCard()
            .contentShape(Rectangle())
            .onTapGesture { openWindow(id: "meeting-detail", value: segment.meetingId) }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { openWindow(id: "meeting-detail", value: segment.meetingId) }
        case .snapshot(let hit):
            AccentBarRow(color: Color.whimbrelAccentPrimary) { SnapshotHitRow(hit: hit) }
            .whimbrelCard()
            .contentShape(Rectangle())
            .onTapGesture { openWindow(id: "meeting-detail", value: hit.meetingId) }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { openWindow(id: "meeting-detail", value: hit.meetingId) }
        }
    }

    var visibleEpisodes: [CoreEpisode] {
        showHiddenEpisodes ? episodes : episodes.filter { !hiddenEpisodeIds.contains($0.id) }
    }

    @ViewBuilder
    var episodesModeBody: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            HStack {
                Spacer()
                Toggle("Show hidden episodes", isOn: $showHiddenEpisodes)
                    .tint(Color.whimbrelAccentPrimary)
            }

            if visibleEpisodes.isEmpty && isLoadingEntries {
                ProgressView()
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 40)
            } else if visibleEpisodes.isEmpty {
                Text(appState.coreStore == nil ? Self.storeUnavailableMessage : "No episodes yet. They form from screen activity after a few minutes of use.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 40)
            } else {
                ScrollView {
                    LazyVStack(spacing: Spacing.sm) {
                        ForEach(visibleEpisodes) { episode in
                            episodeRow(for: episode)
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    func episodeRow(for episode: CoreEpisode) -> some View {
        AccentBarRow(color: Color.whimbrelAccentSecondary) {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                HStack {
                    Text(episode.windowTitle ?? episode.appBundleId).font(.whimbrelBody(.caption).bold())
                        .foregroundStyle(Color.whimbrelTextPrimary)
                    Text(episode.appBundleId).font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                    if hiddenEpisodeIds.contains(episode.id) {
                        WhimbrelMetadataChip(systemImage: "eye.slash", text: "Hidden")
                    }
                    Spacer()
                    Text(Self.formattedEpisodeTime(startUnixMs: episode.startTsUnixMs, endUnixMs: episode.endTsUnixMs))
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                    DeviceChip(deviceId: episode.deviceId)
                }
                Text(episodeSummaryText(episode))
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .lineLimit(2)
            }
        }
        .whimbrelCard()
        .contentShape(Rectangle())
        .onTapGesture { openWindow(id: "episode-detail", value: episode.id) }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { openWindow(id: "episode-detail", value: episode.id) }
    }

    func episodeSummaryText(_ episode: CoreEpisode) -> String {
        switch episode.status {
        case "summarized": return episode.summary ?? "Summary unavailable"
        case "summarize_failed": return "Summary unavailable"
        default: return "Summarizing…"
        }
    }

    static func formattedEpisodeTime(startUnixMs: Int64, endUnixMs: Int64) -> String {
        let start = Date(timeIntervalSince1970: TimeInterval(startUnixMs) / 1000)
        let end = Date(timeIntervalSince1970: TimeInterval(endUnixMs) / 1000)
        let startText = start.formatted(date: .omitted, time: .shortened)
        let minutes = max(0, Int((end.timeIntervalSince1970 - start.timeIntervalSince1970) / 60))
        return "\(startText) · \(minutes) min"
    }

    @ViewBuilder
    func statusLabel(for status: CoreMeetingStatus) -> some View {
        switch status {
        case .recording:
            HStack(spacing: Spacing.xs) {
                Circle().fill(Color.whimbrelError).frame(width: 6, height: 6)
                Text("Recording")
            }
            .foregroundStyle(Color.whimbrelError)
        case .completed:
            Text("Completed").foregroundStyle(Color.whimbrelTextSecondary)
        case .scheduled:
            Text("Scheduled").foregroundStyle(Color.whimbrelAccentHighlight)
        case .cancelled:
            Text("Cancelled").strikethrough().foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    static func formatted(_ tsUnixMs: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(tsUnixMs) / 1000)
        return date.formatted(date: .abbreviated, time: .shortened)
    }

    /// Start time, plus duration if the meeting has ended — e.g. "Jul 26, 2:30 PM · 45 min".
    /// Still-in-progress/scheduled meetings (`endUnixMs == nil`) show just the start time.
    static func formattedMeetingTime(startUnixMs: Int64, endUnixMs: Int64?) -> String {
        let start = Date(timeIntervalSince1970: TimeInterval(startUnixMs) / 1000)
        let startText = start.formatted(date: .abbreviated, time: .shortened)
        guard let endUnixMs else { return startText }
        let end = Date(timeIntervalSince1970: TimeInterval(endUnixMs) / 1000)
        let minutes = Int((end.timeIntervalSince1970 - start.timeIntervalSince1970) / 60)
        return "\(startText) · \(minutes) min"
    }

    /// `startCs` is centiseconds elapsed since the meeting's recording started (not an
    /// absolute timestamp — matches `TranscriptSegment`'s convention throughout the
    /// core), rendered as a "MM:SS into the meeting" marker.
    static func formattedElapsed(startCs: Int64) -> String {
        let totalSeconds = startCs / 100
        return String(format: "%d:%02d", totalSeconds / 60, totalSeconds % 60)
    }
}

/// A Timeline snapshot hit. The thumbnail loads off the main thread through the shared cache;
/// while loading, or when the image is missing (e.g. the folder was cleaned up), the row is text-only.
struct SnapshotHitRow: View {
    let hit: CoreMeetingSnapshotHit
    @State private var image: NSImage?

    var body: some View {
        HStack(alignment: .top, spacing: Spacing.sm) {
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(width: 96, height: 60)
                    .clipShape(RoundedRectangle(cornerRadius: 4))
            }
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text(hit.meetingTitle).font(.whimbrelBody(.caption).bold())
                    .foregroundStyle(Color.whimbrelTextPrimary)
                Text(hit.text)
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .lineLimit(2)
            }
        }
        .task(id: hit.filePath) { image = await SnapshotThumbnails.image(for: hit.filePath) }
    }
}
