import SwiftUI

/// One cleanup Free up space offers, with what it would free now.
struct FreeUpSpaceOption: Identifiable, Equatable {
    enum Kind: Hashable { case compactDatabase, compressAudio, deleteOldAudio(days: Int), deleteOldCaptures(days: Int) }
    var id: Kind { kind }
    let kind: Kind
    /// Bytes it frees, when known before running.
    let frees: Int64?
    let detail: String
}

enum FreeUpSpaceModel {
    static let dayChoices = [30, 90, 180]
    static let defaultDays = 90

    /// The cleanups worth offering now. One that would free nothing is left out, except
    /// compressing, which shows while any recording is still full quality.
    static func options(space: CoreDbSpace?, usage: CoreMediaUsage?, deletableAudio: (Int) -> Int64,
                        oldCaptures: (Int) -> (count: Int64, textBytes: Int64), days: Int) -> [FreeUpSpaceOption] {
        func size(_ b: Int64) -> String { ByteCountFormatter.string(fromByteCount: b, countStyle: .file) }
        var out: [FreeUpSpaceOption] = []
        if let space, space.freeBytes > 0 {
            out.append(.init(kind: .compactDatabase, frees: space.freeBytes,
                             detail: String(localized: "Frees \(size(space.freeBytes)) inside the database file.")))
        }
        if let usage, usage.audioFullBytes > 0 {
            out.append(.init(kind: .compressAudio, frees: nil,
                             detail: String(localized: "\(size(usage.audioFullBytes)) of recordings are full quality. Compressing keeps them playable at a smaller size.")))
        }
        let audio = deletableAudio(days)
        if audio > 0 {
            out.append(.init(kind: .deleteOldAudio(days: days), frees: audio,
                             detail: String(localized: "Recordings older than \(days) days: \(size(audio)). Transcripts, notes and summaries stay.")))
        }
        let captures = oldCaptures(days)
        if captures.count > 0 {
            out.append(.init(kind: .deleteOldCaptures(days: days), frees: captures.textBytes,
                             detail: String(localized: "\(captures.count) screen captures older than \(days) days (about \(size(captures.textBytes)) of text).")))
        }
        return out
    }
}

/// Settings → Storage → Free up space: one-off cleanups. Nothing here changes the retention settings.
struct FreeUpSpaceSheet: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss
    @State private var days = FreeUpSpaceModel.defaultDays
    @State private var options: [FreeUpSpaceOption] = []
    @State private var loading = true
    @State private var working = false
    @State private var confirming: FreeUpSpaceOption.Kind?
    @State private var result: String?
    @State private var before: Int64?

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text("Free up space").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Picker("Older than", selection: $days) {
                ForEach(FreeUpSpaceModel.dayChoices, id: \.self) { Text("\($0) days").tag($0) }
            }
            .pickerStyle(.segmented)
            .frame(maxWidth: 320)
            if loading {
                ProgressView().controlSize(.small)
            } else if options.isEmpty {
                Text("Nothing to free right now.").foregroundStyle(Color.whimbrelTextSecondary)
            }
            ForEach(options) { option in
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title(option.kind)).foregroundStyle(Color.whimbrelTextPrimary)
                        Text(option.detail).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer()
                    Button(buttonLabel(option.kind)) {
                        if needsConfirmation(option.kind) { confirming = option.kind } else { run(option.kind) }
                    }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(working)
                }
            }
            if working { ProgressView().controlSize(.small) }
            if let result { Text(result).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary) }
            HStack { Spacer(); Button("Done") { dismiss() }.keyboardShortcut(.defaultAction) }
        }
        .padding(Spacing.lg)
        .frame(minWidth: 480)
        .onAppear(perform: refresh)
        .onChange(of: days) { _, _ in refresh() }
        .confirmationDialog(confirmTitle, isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } }),
                            presenting: confirming) { kind in
            // The kind comes from `presenting`, not from `confirming`, which the binding may
            // already have cleared by the time the button's action runs.
            Button(String(localized: "Delete"), role: .destructive) { run(kind) }
            Button(String(localized: "Cancel"), role: .cancel) {}
        }
    }

    private func title(_ kind: FreeUpSpaceOption.Kind) -> String {
        switch kind {
        case .compactDatabase: String(localized: "Compact database")
        case .compressAudio: String(localized: "Compress recordings")
        case .deleteOldAudio: String(localized: "Delete old recordings")
        case .deleteOldCaptures: String(localized: "Delete old screen captures")
        }
    }

    private func buttonLabel(_ kind: FreeUpSpaceOption.Kind) -> String {
        switch kind {
        case .compactDatabase: String(localized: "Compact")
        case .compressAudio: String(localized: "Compress all")
        case .deleteOldAudio, .deleteOldCaptures: String(localized: "Delete…")
        }
    }

    private func needsConfirmation(_ kind: FreeUpSpaceOption.Kind) -> Bool {
        switch kind {
        case .deleteOldAudio, .deleteOldCaptures: true
        default: false
        }
    }

    private var confirmTitle: String {
        switch confirming {
        case .deleteOldAudio(let d): String(localized: "Delete the audio of meetings older than \(d) days? This can't be undone.")
        case .deleteOldCaptures(let d): String(localized: "Delete screen captures older than \(d) days from this Mac and your other devices? This can't be undone.")
        default: ""
        }
    }

    private func refresh() {
        guard let store = appState.coreStore else { loading = false; return }
        loading = true
        let days = days, media = appState.mediaSettings
        let preview = CoreMediaPolicy(compressAfterDays: nil, deleteAudioAfterDays: nil, shrinkImagesAfterDays: nil,
                                      deleteAudioPreviewAfterDays: days)
        let busy = appState.mediaRetention.busyMeetingIds()
        DispatchQueue.global(qos: .userInitiated).async {
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            let (space, plan, captures) = CoreMemoryStore.accessQueue.sync {
                (store.dbSpace(),
                 store.mediaPlan(policy: preview, nowUnixMs: nowMs, mediaRoot: media.folderURL, busy: busy),
                 store.capturesOlderThan(cutoffUnixMs: nowMs - Int64(days) * 86_400_000))
            }
            let total = StorageFootprint.measure(dataRoot: SpikePaths.root, mediaFolder: media.folderURL).total
            let built = FreeUpSpaceModel.options(
                space: space, usage: plan?.usage,
                deletableAudio: { _ in plan?.usage.audioDeletableBytes ?? 0 },
                oldCaptures: { _ in captures ?? (0, 0) }, days: days)
            DispatchQueue.main.async {
                options = built
                loading = false
                if !working { before = total }
            }
        }
    }

    private func run(_ kind: FreeUpSpaceOption.Kind) {
        guard let store = appState.coreStore else { return }
        working = true
        result = nil
        switch kind {
        case .compactDatabase:
            compactThenFinish(store, forced: true)
        case .compressAudio:
            runOneOff(CoreMediaPolicy(compressAfterDays: 0, deleteAudioAfterDays: nil, shrinkImagesAfterDays: 0, deleteAudioPreviewAfterDays: nil))
        case .deleteOldAudio(let d):
            runOneOff(CoreMediaPolicy(compressAfterDays: nil, deleteAudioAfterDays: d, shrinkImagesAfterDays: nil, deleteAudioPreviewAfterDays: d))
        case .deleteOldCaptures(let d):
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            MediaMover.queue.async {
                _ = CoreMemoryStore.accessQueue.sync {
                    store.applyRetention(captureItemDays: d, meetingDays: nil, conversationDays: nil, nowUnixMs: nowMs,
                                         recordingsDir: SpikePaths.root)
                }
                // Captures live inside the database: only a compact gives their space back to the
                // disk, so do it now (the disk rule still applies).
                DispatchQueue.main.async { compactThenFinish(store, forced: true) }
            }
        }
    }

    /// Asks for a one-off media run and waits for that run itself (another run in flight finishes
    /// first), then compacts if it pays off and reports what was freed.
    private func runOneOff(_ policy: CoreMediaPolicy) {
        guard let store = appState.coreStore else { return }
        let runner = appState.mediaRetention
        let target = runner.oneOffRunsFinished + 1
        runner.run(mode: .oneOff(policy))
        Task { @MainActor in
            while runner.oneOffRunsFinished < target { try? await Task.sleep(nanoseconds: 500_000_000) }
            compactThenFinish(store, forced: false)
        }
    }

    private func compactThenFinish(_ store: CoreMemoryStore, forced: Bool) {
        let media = appState.mediaSettings.folderURL
        DispatchQueue.global(qos: .userInitiated).async {
            let compacted = StorageCompaction.run(store, forced: forced)
            let after = StorageFootprint.measure(dataRoot: SpikePaths.root, mediaFolder: media).total
            DispatchQueue.main.async {
                working = false
                if forced, let compacted, !compacted.ran {
                    result = CompactCopy.message(compacted)
                } else if let before {
                    result = String(localized: "Freed \(ByteCountFormatter.string(fromByteCount: max(0, before - after), countStyle: .file)).")
                }
                refresh()
            }
        }
    }
}
