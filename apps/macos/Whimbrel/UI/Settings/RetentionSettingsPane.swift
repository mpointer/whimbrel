import SwiftUI
import AppKit

struct RetentionSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @State private var retentionStatus: String?
    @State private var compactStatus: String?
    @State private var footprint: StorageFootprint?
    @State private var lowVolumes: [StorageFootprint.LowVolume] = []
    @State private var compacting = false
    @State private var mediaFolderRefusal: String?
    @State private var orphanScanResult: CoreOrphanScan?
    @State private var orphanStatus: String?
    @State private var showingOrphanDeleteConfirmation = false
    @State private var duplicateScanResult: [CoreMemoryStore.DuplicateMeetingGroup]?
    @State private var duplicateStatus: String?
    @State private var showingDuplicateDeleteConfirmation = false
    @State private var noAudioScanResult: [CoreMemoryStore.MeetingWithoutAudioSummary]?
    @State private var noAudioStatus: String?
    @State private var showingNoAudioDeleteConfirmation = false

    var body: some View {
        content
            .onAppear(perform: measureStorage)
            .sheet(isPresented: $appState.showFreeUpSpace, onDismiss: measureStorage) {
                FreeUpSpaceSheet().environmentObject(appState)
            }
    }

    private var content: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            storageBlock

            Divider()

            Text("Retention").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Each data class has its own retention window; hard-deletes rows older than it (propagates to the search indexes too).")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            retentionRow(
                label: "Screen/audio capture items",
                days: Binding(
                    get: { appState.retention.captureItemDays },
                    set: { appState.retention.captureItemDays = $0 }
                ),
                keepForever: Binding(
                    get: { appState.retention.captureItemKeepForever },
                    set: { appState.retention.captureItemKeepForever = $0 }
                ),
                foreverLabel: "Keep forever"
            )
            retentionRow(
                label: "Meetings",
                days: Binding(
                    get: { appState.retention.meetingDays },
                    set: { appState.retention.meetingDays = $0 }
                ),
                keepForever: Binding(
                    get: { appState.retention.meetingKeepForever },
                    set: { appState.retention.meetingKeepForever = $0 }
                ),
                foreverLabel: "Keep forever"
            )
            retentionRow(
                label: "Conversations",
                days: Binding(
                    get: { appState.retention.conversationDays },
                    set: { appState.retention.conversationDays = $0 }
                ),
                keepForever: Binding(
                    get: { appState.retention.conversationKeepForever },
                    set: { appState.retention.conversationKeepForever = $0 }
                ),
                foreverLabel: "Keep forever"
            )

            HStack {
                Button("Apply Retention Now") { applyRetentionNow() }
                    .buttonStyle(.whimbrelSecondary)
                Button("Compact Database") { compactNow() }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(compacting || appState.coreStore == nil)
                if compacting { ProgressView().controlSize(.small) }
            }
            if let compactStatus {
                Text(compactStatus)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }

            if let retentionStatus {
                Text(retentionStatus)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }

            Divider()

            Text("Media").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Meeting audio and slide snapshots are kept in this folder, in Audio and Snapshots. Network shares work but are not recommended.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Text("Media folder")
                Text(appState.mediaSettings.folderURL.path)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                Button("Choose…") { chooseMediaFolder() }
                    .buttonStyle(.whimbrelSecondary)
                    .controlSize(.small)
                Button("Reset") {
                    mediaFolderRefusal = nil
                    setMediaFolder(nil)
                }
                .buttonStyle(.whimbrelSecondary)
                .controlSize(.small)
                .disabled(appState.mediaSettings.folderPath == nil)
            }

            if let mediaFolderRefusal {
                Text(mediaFolderRefusal)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }

            retentionRow(label: "Compress audio after", days: media(\.compressAfterDays),
                         keepForever: media(\.keepFullAudioForever), foreverLabel: "Keep full quality forever")
            retentionRow(label: "Delete audio after", days: media(\.deleteAudioAfterDays),
                         keepForever: media(\.neverDeleteAudio), foreverLabel: "Never delete")
            if let plan = appState.mediaRetention.lastPlan,
               let note = MediaRetentionCopy.deleteSwitchNote(plan.usage, deleteOn: !appState.mediaSettings.neverDeleteAudio) {
                Text(note)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
            retentionRow(label: "Shrink images after", days: media(\.shrinkImagesAfterDays),
                         keepForever: media(\.keepFullImagesForever), foreverLabel: "Keep full quality forever")

            if let plan = appState.mediaRetention.lastPlan {
                Text(MediaRetentionCopy.usage(plan.usage))
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Text(MediaRetentionCopy.preview(plan, policy: appState.mediaSettings.policy))
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }

            if let progress = appState.mediaRetention.progress {
                HStack {
                    Text(MediaRetentionCopy.progress(step: appState.mediaRetention.step, done: progress.done, total: progress.total))
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                    Button("Cancel") { appState.mediaRetention.cancel() }
                        .buttonStyle(.whimbrelSecondary)
                        .controlSize(.small)
                }
            }

            Divider()

            Text("Orphaned recording files").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Recording files with no matching meeting record at all — e.g. from a crashed or interrupted session. Retention above only removes files for meetings it actually deletes; this finds files that were never tracked by any meeting in the first place.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Button("Scan for Orphaned Files") { scanForOrphans() }
                    .buttonStyle(.whimbrelSecondary)
                Button("Clean Up") { showingOrphanDeleteConfirmation = true }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled((orphanScanResult?.orphaned.isEmpty ?? true))
            }

            if let orphanStatus {
                Text(orphanStatus)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }

            Divider()

            Text("Duplicate meetings").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("A sync bug (fixed 2026-08-05) could duplicate a peer's meeting once per lifecycle update — this finds and removes any leftover duplicates from before the fix. Never affects this device's own meetings.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Button("Scan for Duplicate Meetings") { scanForDuplicateMeetings() }
                    .buttonStyle(.whimbrelSecondary)
                Button("Clean Up") { showingDuplicateDeleteConfirmation = true }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled((duplicateScanResult?.isEmpty ?? true))
            }

            if let duplicateStatus {
                Text(duplicateStatus)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }

            Divider()

            Text("Meetings without audio").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Meeting entries that never captured any audio — e.g. a calendar-triggered meeting that started and ended before actually recording anything. Still-recording meetings are never included.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Button("Scan for Meetings Without Audio") { scanForMeetingsWithoutAudio() }
                    .buttonStyle(.whimbrelSecondary)
                Button("Clean Up") { showingNoAudioDeleteConfirmation = true }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled((noAudioScanResult?.isEmpty ?? true))
            }

            if let noAudioStatus {
                Text(noAudioStatus)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
        .onAppear { appState.mediaRetention.refreshPlan() }
        .onChange(of: appState.mediaSettings.policy) { _, _ in appState.mediaRetention.refreshPlan() }
        // A run plans before it works; plan again once it ends so the lines show what is left.
        .onChange(of: appState.mediaRetention.progress == nil) { _, idle in
            if idle { appState.mediaRetention.refreshPlan() }
        }
        .alert("Delete Orphaned Files?", isPresented: $showingOrphanDeleteConfirmation, presenting: orphanScanResult) { result in
            Button("Delete \(result.orphaned.count) File(s)", role: .destructive) { deleteOrphans() }
            Button("Cancel", role: .cancel) {}
        } message: { result in
            Text("This will permanently delete \(result.orphaned.count) file(s) totaling \(formattedByteCount(result.totalBytes)). This cannot be undone.")
        }
        .alert("Delete Duplicate Meetings?", isPresented: $showingDuplicateDeleteConfirmation, presenting: duplicateScanResult) { groups in
            let extraCount = groups.reduce(0) { $0 + ($1.meetingIds.count - 1) }
            Button("Delete \(extraCount) Duplicate(s)", role: .destructive) { cleanUpDuplicateMeetings() }
            Button("Cancel", role: .cancel) {}
        } message: { groups in
            let extraCount = groups.reduce(0) { $0 + ($1.meetingIds.count - 1) }
            Text("This will permanently delete \(extraCount) duplicate meeting(s) — the most recently applied copy in each group is kept. This cannot be undone.")
        }
        .alert("Delete Meetings Without Audio?", isPresented: $showingNoAudioDeleteConfirmation, presenting: noAudioScanResult) { results in
            Button("Delete \(results.count) Meeting(s)", role: .destructive) { cleanUpMeetingsWithoutAudio() }
            Button("Cancel", role: .cancel) {}
        } message: { results in
            Text("This will permanently delete \(results.count) meeting(s) with no recorded audio, along with any transcript/notes/summary they have. This cannot be undone.")
                + Text(verbatim: "\n\n") + Text("This removes them from all your devices.")
        }
    }

    @ViewBuilder
    private func retentionRow(label: LocalizedStringKey, days: Binding<Int>, keepForever: Binding<Bool>, foreverLabel: LocalizedStringKey) -> some View {
        HStack {
            Text(label).frame(width: 220, alignment: .leading)
            Stepper(SettingsCopy.retentionDays(days.wrappedValue), value: days, in: 1...3650)
                .disabled(keepForever.wrappedValue)
            Toggle(foreverLabel, isOn: keepForever)
                .toggleStyle(.checkbox)
        }
    }

    private func applyRetentionNow() {
        appState.retention.applyNow(store: appState.coreStore,
                                    compactAllowed: !appState.audio.micActive && !appState.audio.systemAudioActive) { result in
            guard let result else {
                retentionStatus = String(localized: "Retention run failed (store unavailable or DB error).")
                return
            }
            retentionStatus = String(localized: "Deleted \(result.captureItemsDeleted) capture item(s), \(result.meetingsDeleted) meeting(s), \(result.conversationsDeleted) conversation(s).")
        }
    }

    /// "Whimbrel uses X on this Mac", by part, the low-disk line, and Free up space.
    @ViewBuilder
    private var storageBlock: some View {
        Text("Storage").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
        if let footprint {
            Text(StorageCopy.total(footprint)).foregroundStyle(Color.whimbrelTextPrimary)
            Text(StorageCopy.parts(footprint))
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            ProgressView().controlSize(.small)
        }
        ForEach(lowVolumes, id: \.url) { low in
            Text(StorageCopy.lowDisk(low))
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelWarning)
                .fixedSize(horizontal: false, vertical: true)
        }
        Button("Free Up Space…") { appState.showFreeUpSpace = true }
            .buttonStyle(.whimbrelSecondary)
            .disabled(appState.coreStore == nil)
    }

    private func measureStorage() {
        let media = appState.mediaSettings.folderURL
        DispatchQueue.global(qos: .utility).async {
            let measured = StorageFootprint.measure(dataRoot: SpikePaths.root, mediaFolder: media)
            let low = StorageFootprint.lowDiskVolumes(dataRoot: SpikePaths.root, mediaFolder: media)
            DispatchQueue.main.async {
                footprint = measured
                lowVolumes = low
            }
        }
    }

    /// The user's "Compact Database": runs even when core wouldn't bother, never without the disk
    /// to hold the rewrite. Off the main thread; it holds the store for the whole rewrite.
    private func compactNow() {
        guard let store = appState.coreStore else { return }
        compacting = true
        compactStatus = String(localized: "Compacting… this can take a minute.")
        DispatchQueue.global(qos: .userInitiated).async {
            let result = StorageCompaction.run(store, forced: true)
            DispatchQueue.main.async {
                compacting = false
                compactStatus = CompactCopy.message(result)
            }
        }
    }

    /// A refused folder is not saved; its message shows under the row.
    private func chooseMediaFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        mediaFolderRefusal = MediaSettings.refusal(for: url, dataFolder: SpikePaths.root)
        if mediaFolderRefusal == nil { setMediaFolder(url.path) }
    }

    private func media<T>(_ key: ReferenceWritableKeyPath<MediaSettings, T>) -> Binding<T> {
        Binding(get: { appState.mediaSettings[keyPath: key] }, set: { appState.mediaSettings[keyPath: key] = $0 })
    }

    /// Saves the folder and lets the runner move existing files there. A run already in
    /// flight sees the change, stops and runs again.
    private func setMediaFolder(_ path: String?) {
        appState.mediaSettings.folderPath = path
        appState.mediaRetention.run(mode: .moveOnly)
    }

    private func scanForOrphans() {
        let liveWriteURLs = Set([appState.audio.systemAudioFileURL, appState.audio.micAudioFileURL].compactMap { $0 })
        let mediaAudioDir = appState.mediaSettings.audioURL(deviceId: appState.deviceId)
        guard let store = appState.coreStore else {
            orphanStatus = String(localized: "Scan failed: store unavailable.")
            return
        }
        CoreMemoryStore.accessQueue.async {
            guard let result = store.scanOrphanedRecordings(recordingsDir: SpikePaths.root, mediaAudioDir: mediaAudioDir, excluding: liveWriteURLs) else {
                DispatchQueue.main.async {
                    orphanStatus = String(localized: "Scan failed: could not read referenced file paths from the database.")
                }
                return
            }
            DispatchQueue.main.async {
                orphanScanResult = result
                if result.orphaned.isEmpty {
                    orphanStatus = String(localized: "No orphaned files found.")
                } else {
                    orphanStatus = "\(result.orphaned.count) orphaned file(s) found, \(formattedByteCount(result.totalBytes)) total."
                }
            }
        }
    }

    private func deleteOrphans() {
        guard let result = orphanScanResult else { return }
        let pathsToDelete = result.orphaned
        CoreMemoryStore.accessQueue.async {
            let deleted = CoreMemoryStore.deleteOrphanedRecordings(pathsToDelete)
            DispatchQueue.main.async {
                orphanStatus = String(localized: "Deleted \(deleted) file(s).")
                orphanScanResult = nil
            }
        }
    }

    private func scanForDuplicateMeetings() {
        guard let store = appState.coreStore else {
            duplicateStatus = String(localized: "Scan failed: store unavailable.")
            return
        }
        let ownDeviceId = appState.deviceId
        CoreMemoryStore.accessQueue.async {
            guard let groups = store.findDuplicateMeetingGroups(ownDeviceId: ownDeviceId) else {
                DispatchQueue.main.async {
                    duplicateStatus = String(localized: "Scan failed: could not read meetings from the database.")
                }
                return
            }
            DispatchQueue.main.async {
                duplicateScanResult = groups
                let extraCount = groups.reduce(0) { $0 + ($1.meetingIds.count - 1) }
                if groups.isEmpty {
                    duplicateStatus = String(localized: "No duplicate meetings found.")
                } else {
                    duplicateStatus = "\(groups.count) duplicate set(s) found, \(extraCount) extra copy/copies."
                }
            }
        }
    }

    private func cleanUpDuplicateMeetings() {
        guard let result = duplicateScanResult, let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            var deletedCount = 0
            for group in result {
                // meetingIds is sorted ascending by id — keep the last (highest id,
                // most recently applied), fold the rest into it.
                guard let keep = group.meetingIds.last else { continue }
                for id in group.meetingIds.dropLast() {
                    // A row fold, never MeetingLifecycle.forgetMeeting's file cleanup:
                    // duplicate groups are peer-sourced only (findDuplicateMeetingGroups
                    // filters `device_id != ownDeviceId`), and apply_synced_meeting
                    // always inserts audio_path as NULL, so there is no local audio
                    // file to orphan, while every copy shares one transcript_path string
                    // that the kept row still references. The fold matters because the
                    // copies are not identical: a peer's transcript lines were applied to
                    // whichever copy existed then, so a plain delete lost them (10/30).
                    if store.foldDuplicateMeeting(keep: keep, drop: id) {
                        deletedCount += 1
                    }
                }
            }
            DispatchQueue.main.async {
                duplicateStatus = String(localized: "Deleted \(deletedCount) duplicate meeting(s).")
                duplicateScanResult = nil
            }
        }
    }

    private func scanForMeetingsWithoutAudio() {
        guard let store = appState.coreStore else {
            noAudioStatus = String(localized: "Scan failed: store unavailable.")
            return
        }
        let ownDeviceId = appState.deviceId
        CoreMemoryStore.accessQueue.async {
            guard let results = store.findMeetingsWithoutAudio(ownDeviceId: ownDeviceId) else {
                DispatchQueue.main.async {
                    noAudioStatus = String(localized: "Scan failed: could not read meetings from the database.")
                }
                return
            }
            DispatchQueue.main.async {
                noAudioScanResult = results
                noAudioStatus = results.isEmpty
                    ? "No meetings without audio found."
                    : "\(results.count) meeting(s) without audio found."
            }
        }
    }

    private func cleanUpMeetingsWithoutAudio() {
        guard let results = noAudioScanResult, let store = appState.coreStore else { return }
        // Forgotten, not just deleted: they go from every device, with any transcript or snapshot files.
        MeetingLifecycle.forgetMeetings(store: store, meetingIds: results.map(\.id)) { deletedCount in
            noAudioStatus = String(localized: "Deleted \(deletedCount) meeting(s).")
            noAudioScanResult = nil
        }
    }

    private func formattedByteCount(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }
}

/// The Media section's usage and preview lines.
enum MediaRetentionCopy {
    static func usage(_ u: CoreMediaUsage) -> String {
        func size(_ bytes: Int64) -> String { ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file) }
        return "Audio: \(size(u.audioFullBytes)) full quality, \(size(u.audioCompressedBytes)) compressed. Images: \(size(u.imageBytes))."
    }

    /// Counts meetings, not steps: a meeting moving both audio and images is one move.
    /// Next to the "Delete audio after" switch while it's off.
    static func deleteSwitchNote(_ u: CoreMediaUsage, deleteOn: Bool) -> String? {
        guard !deleteOn else { return nil }
        guard u.audioDeletableBytes > 0 else { return "Turning this on would free nothing yet." }
        return "Turning this on would free \(ByteCountFormatter.string(fromByteCount: u.audioDeletableBytes, countStyle: .file)) now."
    }

    static func progress(step: CoreMediaStep?, done: Int, total: Int) -> String {
        let verb: String
        switch step {
        case .compress: verb = "Compressing…"
        case .shrinkImages: verb = "Shrinking images…"
        case .deleteAudio: verb = "Deleting audio…"
        case .moveAudio, .moveSnapshots: verb = "Moving files…"
        case nil: verb = "Working…"
        }
        return "\(verb) \(done) of \(total)"
    }

    /// Everything due now (a run takes up to `MediaRetentionRunner.maxMeetings` of it), the
    /// pace, that existing recordings count, and deletion by name whenever it's on.
    static func preview(_ p: CoreMediaPlan, policy: CoreMediaPolicy) -> String {
        func count(_ steps: Set<CoreMediaStep>) -> Int { p.work.filter { !steps.isDisjoint(with: $0.steps) }.count }
        func meetings(_ n: Int) -> String { n == 1 ? "1 meeting" : "\(n) meetings" }
        let compress = count([.compress]), delete = count([.deleteAudio]), shrink = count([.shrinkImages])
        let move = count([.moveAudio, .moveSnapshots])
        let clauses = [
            compress > 0 ? "compress \(meetings(compress))" : nil,
            delete > 0 ? "delete audio of \(delete)" : nil,
            shrink > 0 ? "shrink images of \(shrink)" : nil,
            move > 0 ? "move \(move)" : nil,
        ].compactMap { $0 }
        var text = clauses.isEmpty ? "Nothing due now."
            : "Due now: \(clauses.joined(separator: ", ")) (\(meetings(p.work.count)) in all)."
        text += " Up to \(MediaRetentionRunner.maxMeetings) meetings per run; runs at launch, daily and after each meeting."
        text += " Includes existing recordings."
        if let days = policy.deleteAudioAfterDays {
            text += " Audio of transcribed meetings older than \(days) days is deleted permanently."
        }
        if p.disconnectedMeetings > 0 { text += " \(meetings(p.disconnectedMeetings)) on a disconnected drive." }
        return text
    }
}

/// Settings copy with counts: plural forms come from the String Catalog.
enum SettingsCopy {
    static func retentionDays(_ n: Int) -> String {
        String(localized: "\(n) days")
    }

    static func eventsFound(_ n: Int) -> String {
        String(localized: "\(n) events found in the next 7 days.")
    }
}
