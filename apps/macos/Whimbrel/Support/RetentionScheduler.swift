import Foundation
import OSLog

/// FR-MEM-4 follow-on: runs retention automatically, matching this app's other
/// `Timer`-based background services (`MeetingHeuristicDetector`, `MeetingTriggerService`)
/// rather than requiring the user to remember to click "Apply Retention Now." Runs once
/// shortly after launch (not up to a full day after a user's first-ever launch), then
/// on a normal daily repeating timer from that point. Unconditional — retention isn't
/// gated on capture state the way meeting-detection is; there's no reason a currently
/// -recording meeting should block *other*, unrelated meetings/captures from being
/// retention-swept. Also sweeps orphaned recording files (no matching `meeting` row at
/// all) on the same timer — previously only reachable via a manual "Scan for Orphaned
/// Files" Settings button.
@MainActor
final class RetentionScheduler: ObservableObject {
    private static let initialDelaySeconds: TimeInterval = 30
    private static let pollInterval: TimeInterval = 24 * 60 * 60

    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "retention-scheduler")
    private var initialTimer: Timer?
    private var repeatingTimer: Timer?
    private var deviceId = ""
    private weak var runner: MediaRetentionRunner?

    /// The 30-second initial run is media retention's launch trigger; the repeating one its daily trigger.
    func start(retention: RetentionSettings, media: MediaSettings, deviceId: String, store: CoreMemoryStore?, audio: AudioCaptureService,
               runner: MediaRetentionRunner) {
        stop()
        self.deviceId = deviceId
        self.runner = runner
        initialTimer = Timer.scheduledTimer(withTimeInterval: Self.initialDelaySeconds, repeats: false) { [weak self] _ in
            Task { @MainActor in self?.runAndScheduleRepeating(retention: retention, media: media, store: store, audio: audio) }
        }
    }

    func stop() {
        initialTimer?.invalidate()
        initialTimer = nil
        repeatingTimer?.invalidate()
        repeatingTimer = nil
    }

    private func runAndScheduleRepeating(retention: RetentionSettings, media: MediaSettings, store: CoreMemoryStore?, audio: AudioCaptureService) {
        run(retention: retention, media: media, store: store, audio: audio)
        repeatingTimer = Timer.scheduledTimer(withTimeInterval: Self.pollInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.run(retention: retention, media: media, store: store, audio: audio) }
        }
    }

    private func run(retention: RetentionSettings, media: MediaSettings, store: CoreMemoryStore?, audio: AudioCaptureService) {
        retention.applyNow(store: store) { [weak self] result in
            if result != nil {
                self?.log.info("Automatic daily retention run completed")
            } else {
                self?.log.error("Automatic daily retention run failed (store unavailable or DB error)")
            }
        }
        sweepOrphanedFiles(store: store, mediaAudioDir: media.audioURL(deviceId: deviceId), audio: audio)
        runner?.run(mode: .daily)
    }

    /// Same scan/delete the manual "Scan for Orphaned Files" Settings button does
    /// (core/api::retention::scan_orphans), run automatically with no confirmation
    /// prompt — matches `retention.applyNow`'s own hands-off deletion above.
    /// `liveWriteURLs` must be snapshotted here, on the main actor, before hopping to
    /// `CoreMemoryStore.accessQueue` — `AudioCaptureService` is main-actor-isolated.
    private func sweepOrphanedFiles(store: CoreMemoryStore?, mediaAudioDir: URL, audio: AudioCaptureService) {
        guard let store else { return }
        let liveWriteURLs = Set([audio.systemAudioFileURL, audio.micAudioFileURL].compactMap { $0 })
        CoreMemoryStore.accessQueue.async { [weak self] in
            guard let scan = store.scanOrphanedRecordings(recordingsDir: SpikePaths.root, mediaAudioDir: mediaAudioDir, excluding: liveWriteURLs) else {
                self?.log.error("Automatic orphan sweep failed: could not read referenced file paths from the database")
                return
            }
            guard !scan.orphaned.isEmpty else { return }
            let deleted = CoreMemoryStore.deleteOrphanedRecordings(scan.orphaned)
            self?.log.info("Automatic orphan sweep deleted \(deleted, privacy: .public) of \(scan.orphaned.count, privacy: .public) file(s), \(scan.totalBytes, privacy: .public) bytes")
        }
    }
}
