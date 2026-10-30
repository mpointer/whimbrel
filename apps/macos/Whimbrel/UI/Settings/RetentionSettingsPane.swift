import SwiftUI
import AppKit

struct RetentionSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @State private var retentionStatus: String?
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
        VStack(alignment: .leading, spacing: Spacing.lg) {
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

            Button("Apply Retention Now") { applyRetentionNow() }
                .buttonStyle(.whimbrelSecondary)

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
        }
    }

    @ViewBuilder
    private func retentionRow(label: String, days: Binding<Int>, keepForever: Binding<Bool>, foreverLabel: String) -> some View {
        HStack {
            Text(label).frame(width: 220, alignment: .leading)
            Stepper("\(days.wrappedValue) days", value: days, in: 1...3650)
                .disabled(keepForever.wrappedValue)
            Toggle(foreverLabel, isOn: keepForever)
                .toggleStyle(.checkbox)
        }
    }

    private func applyRetentionNow() {
        appState.retention.applyNow(store: appState.coreStore) { result in
            guard let result else {
                retentionStatus = "Retention run failed (store unavailable or DB error)."
                return
            }
            retentionStatus = "Deleted \(result.captureItemsDeleted) capture item(s), \(result.meetingsDeleted) meeting(s), \(result.conversationsDeleted) conversation(s)."
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
            orphanStatus = "Scan failed: store unavailable."
            return
        }
        CoreMemoryStore.accessQueue.async {
            guard let result = store.scanOrphanedRecordings(recordingsDir: SpikePaths.root, mediaAudioDir: mediaAudioDir, excluding: liveWriteURLs) else {
                DispatchQueue.main.async {
                    orphanStatus = "Scan failed: could not read referenced file paths from the database."
                }
                return
            }
            DispatchQueue.main.async {
                orphanScanResult = result
                if result.orphaned.isEmpty {
                    orphanStatus = "No orphaned files found."
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
                orphanStatus = "Deleted \(deleted) file(s)."
                orphanScanResult = nil
            }
        }
    }

    private func scanForDuplicateMeetings() {
        guard let store = appState.coreStore else {
            duplicateStatus = "Scan failed: store unavailable."
            return
        }
        let ownDeviceId = appState.deviceId
        CoreMemoryStore.accessQueue.async {
            guard let groups = store.findDuplicateMeetingGroups(ownDeviceId: ownDeviceId) else {
                DispatchQueue.main.async {
                    duplicateStatus = "Scan failed: could not read meetings from the database."
                }
                return
            }
            DispatchQueue.main.async {
                duplicateScanResult = groups
                let extraCount = groups.reduce(0) { $0 + ($1.meetingIds.count - 1) }
                if groups.isEmpty {
                    duplicateStatus = "No duplicate meetings found."
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
                // most recently applied), delete the rest.
                for id in group.meetingIds.dropLast() {
                    // Plain row delete on purpose — do NOT route this through
                    // MeetingLifecycle.forgetMeeting for file cleanup. Duplicate
                    // groups are peer-sourced only (findDuplicateMeetingGroups
                    // filters `device_id != ownDeviceId`), and apply_synced_meeting
                    // always inserts audio_path as NULL, so there is no local audio
                    // file to orphan here. transcript_path can be non-null, but every
                    // member of a group is the same sync payload applied repeatedly —
                    // they all share one path string, so deleting files for a dropped
                    // duplicate would destroy the file the KEPT row still references.
                    if store.deleteMeeting(id: id) {
                        deletedCount += 1
                    }
                }
            }
            DispatchQueue.main.async {
                duplicateStatus = "Deleted \(deletedCount) duplicate meeting(s)."
                duplicateScanResult = nil
            }
        }
    }

    private func scanForMeetingsWithoutAudio() {
        guard let store = appState.coreStore else {
            noAudioStatus = "Scan failed: store unavailable."
            return
        }
        let ownDeviceId = appState.deviceId
        CoreMemoryStore.accessQueue.async {
            guard let results = store.findMeetingsWithoutAudio(ownDeviceId: ownDeviceId) else {
                DispatchQueue.main.async {
                    noAudioStatus = "Scan failed: could not read meetings from the database."
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
        CoreMemoryStore.accessQueue.async {
            var deletedCount = 0
            for result in results {
                // Plain row delete, same as cleanUpDuplicateMeetings above -- these
                // meetings have no audio_path by definition (that's the scan's whole
                // criterion), so there is no local audio file for deleteMeeting's
                // caller-owns-file-cleanup contract to worry about here.
                if store.deleteMeeting(id: result.id) {
                    deletedCount += 1
                }
            }
            DispatchQueue.main.async {
                noAudioStatus = "Deleted \(deletedCount) meeting(s)."
                noAudioScanResult = nil
            }
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
