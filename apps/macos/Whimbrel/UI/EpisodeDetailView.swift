import SwiftUI

/// Detail window for one episode, opened from Timeline's "Episodes" mode.
/// Fetches fresh by id (see `CoreMemoryStore.getEpisode(id:)`) rather than relying on
/// a value passed in at window-open time, mirroring `MeetingDetailView`'s exact
/// fetch-on-appear/loading/not-found shape. Offers regenerate/hide/forget actions via
/// `EpisodeOptionsMenu` (docs/superpowers/specs/2026-08-14-episodes-threads-ui-design.md).
struct EpisodeDetailView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss
    let episodeId: Int64

    @State private var loadedEpisode: CoreEpisode?
    @State private var loadFailed = false
    @State private var isHidden = false

    var body: some View {
        Group {
            if let loadedEpisode {
                EpisodeDetailContentView(episode: loadedEpisode)
            } else if loadFailed {
                Text("This episode is no longer available.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 480, idealWidth: 560, minHeight: 400, idealHeight: 560)
        .navigationTitle(loadedEpisode?.windowTitle ?? "Episode")
        .background(Color.whimbrelBackgroundPage)
        .overlay(alignment: .topTrailing) {
            HStack(spacing: Spacing.sm) {
                if let loadedEpisode {
                    EpisodeOptionsMenu(
                        episode: loadedEpisode,
                        appState: appState,
                        isHidden: isHidden,
                        onHidden: { dismiss() },
                        onForgotten: { dismiss() }
                    )
                }
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                    .accessibilityLabel("Close")
                        .font(.system(size: 20))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
                .buttonStyle(.plain)
                .help("Close")
            }
            .padding(Spacing.md)
        }
        .onAppear { loadEpisodeIfNeeded() }
    }

    private func loadEpisodeIfNeeded() {
        guard loadedEpisode == nil, !loadFailed else { return }
        guard let store = appState.coreStore else {
            loadFailed = true
            return
        }
        let id = episodeId
        let deviceId = appState.deviceId
        CoreMemoryStore.accessQueue.async {
            let result = store.getEpisode(id: id)
            // The `deviceId:` argument is vestigial -- the core ignores it (kept only
            // for ABI stability): hide/unhide applies to every episode this device
            // shows, and syncs to other devices by episode identity via the settings
            // sync (`episode.hidden.*` fields). A remote hide lands here on the next
            // load, not live; see `Store::set_episode_hidden` for the identity ceiling.
            let hiddenIds = Set(store.listHiddenEpisodeIds(deviceId: deviceId))
            DispatchQueue.main.async {
                if let result {
                    loadedEpisode = result
                    isHidden = hiddenIds.contains(result.id)
                } else {
                    loadFailed = true
                }
            }
        }
    }
}

private struct EpisodeDetailContentView: View {
    let episode: CoreEpisode

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.lg) {
                Text(episode.windowTitle ?? episode.appBundleId).font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
                Text(episode.appBundleId)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                HStack(spacing: Spacing.sm) {
                    Text(Self.formattedRange(startUnixMs: episode.startTsUnixMs, endUnixMs: episode.endTsUnixMs))
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                    DeviceChip(deviceId: episode.deviceId)
                }

                Divider().overlay(Color.whimbrelBorder)

                Text("Summary").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                switch episode.status {
                case "summarized":
                    Text(episode.summary ?? "Summary unavailable")
                        .font(.whimbrelBody(.callout))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                        .textSelection(.enabled)
                case "summarize_failed":
                    Text("Summary unavailable.")
                        .font(.whimbrelBody(.callout))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                default:
                    Text("Summarizing…")
                        .font(.whimbrelBody(.callout))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
            .padding(Spacing.xl)
        }
    }

    private static func formattedRange(startUnixMs: Int64, endUnixMs: Int64) -> String {
        let start = Date(timeIntervalSince1970: TimeInterval(startUnixMs) / 1000)
        let end = Date(timeIntervalSince1970: TimeInterval(endUnixMs) / 1000)
        let startText = start.formatted(date: .abbreviated, time: .shortened)
        let minutes = max(0, Int((end.timeIntervalSince1970 - start.timeIntervalSince1970) / 60))
        return "\(startText) · \(minutes) min"
    }
}
