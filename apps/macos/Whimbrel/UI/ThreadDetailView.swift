import SwiftUI

/// Read-only detail window for one thread, opened from At a Glance's "Needs your
/// attention" section. Fetches fresh by id (see `CoreMemoryStore.getThread(id:)`)
/// rather than relying on a value passed in at window-open time, mirroring
/// `EpisodeDetailView`'s exact fetch-on-appear/loading/not-found shape — no reply,
/// compose, or "mark resolved" action, since this view is read-only
/// (docs/superpowers/specs/2026-08-14-episodes-threads-ui-design.md).
struct ThreadDetailView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss
    let threadId: Int64

    @State private var loadedThread: CoreThread?
    @State private var loadFailed = false

    var body: some View {
        Group {
            if let loadedThread {
                ThreadDetailContentView(thread: loadedThread)
            } else if loadFailed {
                Text("This thread is no longer available.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 480, idealWidth: 560, minHeight: 400, idealHeight: 560)
        .navigationTitle(loadedThread?.subject ?? "Thread")
        .background(Color.whimbrelBackgroundPage)
        .overlay(alignment: .topTrailing) {
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
            .padding(Spacing.md)
        }
        .onAppear { loadThreadIfNeeded() }
    }

    private func loadThreadIfNeeded() {
        guard loadedThread == nil, !loadFailed else { return }
        guard let store = appState.coreStore else {
            loadFailed = true
            return
        }
        let id = threadId
        CoreMemoryStore.accessQueue.async {
            let result = store.getThread(id: id)
            DispatchQueue.main.async {
                if let result {
                    loadedThread = result
                } else {
                    loadFailed = true
                }
            }
        }
    }
}

private struct ThreadDetailContentView: View {
    let thread: CoreThread

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.lg) {
                Text(thread.subject ?? thread.lastMessageFrom).font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
                Text("\(thread.messageCount) message\(thread.messageCount == 1 ? "" : "s") · \(Self.statusLabel(thread.status))")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Text(Self.formattedTime(thread.lastActivityTsUnixMs))
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)

                Divider().overlay(Color.whimbrelBorder)

                Text("Summary").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                switch thread.summarizeStatus {
                case "summarized":
                    Text(thread.summary ?? "Summary unavailable")
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

    private static func statusLabel(_ status: String) -> String {
        switch status {
        case "waiting_on_me": "Waiting on you"
        case "waiting_on_them": "Waiting on them"
        case "resolved": "Resolved"
        case "fyi": "FYI"
        default: status
        }
    }

    private static func formattedTime(_ unixMs: Int64) -> String {
        Date(timeIntervalSince1970: Double(unixMs) / 1000).formatted(date: .abbreviated, time: .shortened)
    }
}
