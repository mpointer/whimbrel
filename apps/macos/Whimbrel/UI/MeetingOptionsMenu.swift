import SwiftUI

/// Shared "Meeting options" menu used by all four meeting surfaces (Today's embedded
/// workspace, Meetings list detail, the standalone meeting-detail window, and
/// Timeline). Fixes the pre-existing regression where only the standalone
/// `MeetingDetailView` window had any delete affordance -- this is the single delete
/// UI pattern across the app now, wired to Task 5's `MeetingLifecycle.forgetMeeting`.
struct MeetingOptionsMenu: View {
    let meeting: CoreMeeting
    @ObservedObject var appState: AppState
    let onForgotten: () -> Void

    @State private var showForgetConfirmation = false
    @State private var isForgetting = false
    @State private var forgetFailed = false
    @State private var showSplitConfirmation = false
    @State private var partCount = 1
    @State private var splitMoveFailed = false
    /// This device's own copy's flag, read from the store (the lead may be a peer's
    /// copy, and `meeting` is the parent's snapshot). nil until loaded.
    @State private var ownKeepMedia: Bool?

    @Environment(\.openWindow) private var openWindow

    static func isForgetDisabled(for meeting: CoreMeeting) -> Bool {
        meeting.status == .recording
    }

    static func canOfferSplit(meeting: CoreMeeting, partCount: Int) -> Bool {
        meeting.status == .completed && partCount >= 2
    }

    /// Splitting re-summarizes both meetings, so wait for any running job.
    static func isSplitDisabled(processingMeetingId: Int64?) -> Bool {
        processingMeetingId != nil
    }

    private func loadMenuState() {
        guard let store = appState.coreStore else { return }
        let id = meeting.id
        let ownId = meeting.ownCopyId(deviceId: appState.deviceId)
        CoreMemoryStore.accessQueue.async {
            let n = store.meetingParts(meetingId: id)?.count ?? 1
            let keep = ownId.flatMap { store.getMeeting(id: $0)?.keepMedia }
            DispatchQueue.main.async { partCount = n; ownKeepMedia = keep }
        }
    }

    var body: some View {
        Menu {
            Button("Open in new window") {
                openWindow(id: "meeting-detail", value: meeting.id)
            }
            if appState.canTranscribeAgain(meeting) {
                Button("Transcribe again") {
                    appState.transcribeAgain(meeting)
                }
                .disabled(appState.processingMeetingId != nil)
            }
            if Self.canOfferSplit(meeting: meeting, partCount: partCount) {
                Button("Split into separate meetings") {
                    showSplitConfirmation = true
                }
                .disabled(Self.isSplitDisabled(processingMeetingId: appState.processingMeetingId))
            }
            if let keep = ownKeepMedia {
                Toggle("Keep media", isOn: Binding(
                    get: { keep },
                    set: { ownKeepMedia = $0; appState.setKeepMedia(meeting, keep: $0) { loadMenuState() } }
                ))
            }
            Divider()
            Button("Forget meeting…", role: .destructive) {
                showForgetConfirmation = true
            }
            .disabled(Self.isForgetDisabled(for: meeting) || isForgetting)
        } label: {
            Label("Meeting options", systemImage: "ellipsis.circle")
        }
        .accessibilityLabel("Meeting options")
        .onAppear { loadMenuState() }
        .onChange(of: meeting.id) { _, _ in ownKeepMedia = nil; loadMenuState() }
        .onChange(of: appState.processingMeetingId) { _, _ in loadMenuState() }
        .confirmationDialog(
            "Split the last part into its own meeting?",
            isPresented: $showSplitConfirmation,
            titleVisibility: .visible
        ) {
            Button("Split meeting") { appState.splitLastPart(of: meeting) { splitMoveFailed = true } }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Both meetings will be summarized again.")
        }
        .confirmationDialog(
            "Forget this meeting?",
            isPresented: $showForgetConfirmation,
            titleVisibility: .visible
        ) {
            Button("Forget meeting", role: .destructive) {
                forgetMeeting()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This permanently removes the meeting note, action items, transcript, local audio file, and search/index entries for this meeting. This does not affect calendar events.")
        }
        .alert("Couldn't move the snapshots", isPresented: $splitMoveFailed) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Couldn't move the snapshots for the split meeting; they're still in the original folder.")
        }
        .alert("Couldn't forget meeting", isPresented: $forgetFailed) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("The meeting record could not be deleted. Please try again.")
        }
    }

    private func forgetMeeting() {
        guard let store = appState.coreStore else { return }
        isForgetting = true
        MeetingLifecycle.forgetMeeting(
            store: store,
            meetingId: meeting.id
        ) { result in
            isForgetting = false
            switch result {
            case .deleted, .deletedWithFileCleanupWarning:
                onForgotten()
            case .failed:
                forgetFailed = true
            }
        }
    }
}
