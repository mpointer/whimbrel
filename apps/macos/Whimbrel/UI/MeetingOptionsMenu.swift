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
    @State private var hasAudio = false
    /// "Merge with previous meeting": this device's copy and the meeting it can join. nil until loaded.
    @State private var mergeTarget: (ownId: Int64, previousId: Int64, title: String, start: Date, audioOnDisk: Bool)?
    @State private var showMergeConfirmation = false

    @Environment(\.openWindow) private var openWindow

    /// The per-meeting "Transcript language" choices; nil is Automatic (detect).
    static let transcriptLanguages: [(code: String?, title: String)] = [
        (nil, String(localized: "Automatic")), ("en", "English"), ("es", "Español"), ("fr", "Français"),
    ]

    /// Only in multilingual mode; English mode always transcribes English.
    static func showsTranscriptLanguage(multilingual: Bool) -> Bool { multilingual }

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

    /// Merging re-transcribes and re-summarizes, so wait for any running job and for media
    /// work on either meeting.
    /// Both meetings need audio on disk: the merged meeting is transcribed again from it.
    static func canOfferMerge(previousId: Int64?, ownId: Int64?, processingMeetingId: Int64?, mediaWorkingId: Int64?,
                              audioOnDisk: Bool) -> Bool {
        guard let previousId, let ownId, processingMeetingId == nil, audioOnDisk else { return false }
        return mediaWorkingId != previousId && mediaWorkingId != ownId
    }

    private func loadMenuState() {
        guard let store = appState.coreStore else { return }
        let id = meeting.id
        let ownId = meeting.ownCopyId(deviceId: appState.deviceId)
        CoreMemoryStore.accessQueue.async {
            let stored = store.meetingParts(meetingId: id)
            let n = stored?.count ?? 1
            let parts = stored ?? []
            let audio = parts.contains { $0.audioPath.map(FileManager.default.fileExists(atPath:)) ?? false }
            let keep = ownId.flatMap { store.getMeeting(id: $0)?.keepMedia }
            let deviceId = appState.deviceId
            let merge = ownId.flatMap { own -> (Int64, Int64, String, Date, Bool)? in
                guard let prev = store.previousMergeableMeeting(meetingId: own, deviceId: deviceId),
                      let m = store.getMeeting(id: prev) else { return nil }
                let onDisk = [own, prev].allSatisfy { id in
                    MeetingTranscriptionQueue.partsWithAudio(store.meetingParts(meetingId: id) ?? []).isEmpty == false
                }
                return (own, prev, m.title, Date(timeIntervalSince1970: TimeInterval(m.startUnixMs) / 1000), onDisk)
            }
            DispatchQueue.main.async {
                partCount = n; ownKeepMedia = keep; hasAudio = audio
                mergeTarget = merge.map { (ownId: $0.0, previousId: $0.1, title: $0.2, start: $0.3, audioOnDisk: $0.4) }
            }
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
            if Self.showsTranscriptLanguage(multilingual: AppLanguage.multilingual()), hasAudio, meeting.status == .completed {
                Menu("Transcript language") {
                    ForEach(Self.transcriptLanguages, id: \.title) { option in
                        Button(option.title) { appState.transcribeAgain(meeting, language: option.code) }
                    }
                }
                .disabled(appState.processingMeetingId != nil)
            }
            if Self.canOfferSplit(meeting: meeting, partCount: partCount) {
                Button("Split into separate meetings") {
                    showSplitConfirmation = true
                }
                .disabled(Self.isSplitDisabled(processingMeetingId: appState.processingMeetingId))
            }
            if let target = mergeTarget {
                Button("Merge with previous meeting") {
                    showMergeConfirmation = true
                }
                .disabled(!Self.canOfferMerge(previousId: target.previousId, ownId: target.ownId,
                                              processingMeetingId: appState.processingMeetingId,
                                              mediaWorkingId: appState.mediaRetention.workingMeetingId,
                                              audioOnDisk: target.audioOnDisk))
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
        .onChange(of: meeting.id) { _, _ in ownKeepMedia = nil; mergeTarget = nil; loadMenuState() }
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
            "Merge with the previous meeting?",
            isPresented: $showMergeConfirmation,
            titleVisibility: .visible
        ) {
            Button("Merge") {
                if let target = mergeTarget { appState.mergeWithPrevious(ownId: target.ownId, into: target.previousId) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            if let target = mergeTarget {
                Text("This meeting's recording becomes the next part of “\(target.title)” at \(target.start.formatted(date: .omitted, time: .shortened)). Its slides and notes move too, and this meeting is removed from this Mac. The merged meeting is transcribed and summarized again. You can't undo this.")
            }
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
