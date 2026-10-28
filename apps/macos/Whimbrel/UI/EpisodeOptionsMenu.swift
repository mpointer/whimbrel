import SwiftUI

struct EpisodeOptionsMenu: View {
    let episode: CoreEpisode
    let appState: AppState
    // Task 10 ruling: `CoreEpisode` deliberately does NOT gain a `hidden` field (that
    // would need a Rust/FFI change to the `episodes` JSON shape, out of scope). The
    // caller already knows this via `CoreMemoryStore.listHiddenEpisodeIds(deviceId:)`
    // (Task 8) and just passes the membership check result in.
    let isHidden: Bool
    let onHidden: () -> Void
    let onForgotten: () -> Void

    static let regenerateLabel = "Regenerate summary"
    static let hideLabel = "Hide episode"
    static let unhideLabel = "Unhide episode"
    static let forgetLabel = "Forget this activity range…"

    @State private var showForgetConfirmation = false
    @State private var isBusy = false
    @State private var operationFailed = false

    var body: some View {
        Menu {
            Button(Self.regenerateLabel) {
                regenerate()
            }
            .disabled(isBusy)

            Button(isHidden ? Self.unhideLabel : Self.hideLabel) {
                setHidden(!isHidden)
            }
            .disabled(isBusy)

            Divider()

            Button(Self.forgetLabel, role: .destructive) {
                showForgetConfirmation = true
            }
            .disabled(isBusy)
        } label: {
            Label("Episode options", systemImage: "ellipsis.circle")
        }
        .accessibilityLabel("Episode options")
        .confirmationDialog(
            "Forget this activity range?",
            isPresented: $showForgetConfirmation,
            titleVisibility: .visible
        ) {
            Button("Forget activity", role: .destructive) {
                forget()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This permanently deletes the captured activity for this time range, its search index entries, and this episode summary on this Mac. It does not affect other devices' independently captured activity for the same period.")
        }
        .alert("Couldn't complete that action", isPresented: $operationFailed) {
            Button("OK", role: .cancel) {}
        }
    }

    private func setHidden(_ hidden: Bool) {
        guard let store = appState.coreStore else { return }
        isBusy = true
        CoreMemoryStore.accessQueue.async {
            let ok = store.setEpisodeHidden(id: episode.id, hidden: hidden)
            DispatchQueue.main.async {
                isBusy = false
                if ok { onHidden() } else { operationFailed = true }
            }
        }
    }

    private func forget() {
        guard let store = appState.coreStore else { return }
        isBusy = true
        CoreMemoryStore.accessQueue.async {
            let ok = store.forgetEpisodeActivity(id: episode.id)
            DispatchQueue.main.async {
                isBusy = false
                if ok { onForgotten() } else { operationFailed = true }
            }
        }
    }

    private func regenerate() {
        guard let store = appState.coreStore else { return }
        isBusy = true
        CoreMemoryStore.accessQueue.async {
            let ok = store.regenerateEpisodeSummary(id: episode.id)
            DispatchQueue.main.async {
                isBusy = false
                if !ok { operationFailed = true }
                // No onForgotten/onHidden-style callback needed: the episode row
                // still exists, its status flips to pending_summary, and the
                // existing episode job/poll mechanism will refresh the summary
                // in place once the background job completes.
            }
        }
    }
}
