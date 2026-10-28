import SwiftUI

struct RetentionSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @State private var retentionStatus: String?
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
                )
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
                )
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
                )
            )

            Button("Apply Retention Now") { applyRetentionNow() }
                .buttonStyle(.whimbrelSecondary)

            if let retentionStatus {
                Text(retentionStatus)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
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
    private func retentionRow(label: String, days: Binding<Int>, keepForever: Binding<Bool>) -> some View {
        HStack {
            Text(label).frame(width: 220, alignment: .leading)
            Stepper("\(days.wrappedValue) days", value: days, in: 1...3650)
                .disabled(keepForever.wrappedValue)
            Toggle("Keep forever", isOn: keepForever)
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

    private func scanForOrphans() {
        let liveWriteURLs = Set([appState.audio.systemAudioFileURL, appState.audio.micAudioFileURL].compactMap { $0 })
        guard let store = appState.coreStore else {
            orphanStatus = "Scan failed: store unavailable."
            return
        }
        CoreMemoryStore.accessQueue.async {
            guard let result = store.scanOrphanedRecordings(recordingsDir: SpikePaths.root, excluding: liveWriteURLs) else {
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
