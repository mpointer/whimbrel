import Foundation
import os

/// Storage retention's executor (Task 9): asks core's planner for this run's work, then
/// moves, compresses, deletes audio and shrinks images one meeting at a time, oldest first.
///
/// - All file work runs serially on `MediaMover.queue` (never `accessQueue`, never main), so a
///   meeting is never compressed and moved at once. Delete, split, snapshot delete and
///   whole-meeting retention also go through that queue (`MeetingLifecycle`,
///   `MeetingSnapshotsStrip`, `RetentionSettings`), so they wait for the meeting in flight.
/// - Busy meetings (recording, starting, transcribing or queued, summarizing) are skipped:
///   the planner gets `busyIds`, and `isBusy` is re-checked on main right before each meeting.
/// - The stored paths are re-read right before every audio step, so a compress after a move
///   gets the moved paths.
/// - A failed compress or shrink backs that step off for that meeting (1, 2, 4… days, at most
///   30), persisted. A change to the ages or the folder mid-run ends the run and plans again.
@MainActor
final class MediaRetentionRunner: ObservableObject {
    enum Mode { case daily, moveOnly }  // daily: ≤10 meetings / ≤10 min; moveOnly: move steps only, no limit

    /// Everything that touches the store or the files. Called on `MediaMover.queue` only.
    struct Ops {
        var plan: (CoreMediaPolicy, Int64, URL, Set<Int64>) -> CoreMediaPlan?
        /// The meeting row's and every part's stored audio path, read now.
        var audioPaths: (Int64) -> [String]
        /// The meeting's "Keep media", read now.
        var keepMedia: (Int64) -> Bool
        /// Every part is transcribed (a legacy meeting: it has a transcript), read now.
        var allTranscribed: (Int64) -> Bool
        var snapshots: (Int64) -> [CoreMeetingSnapshot]
        var moveAudio: (Int64, [String], URL) -> Bool
        var moveSnapshots: (Int64, URL) -> Bool
        var compress: (Int64, [String]) -> Bool
        var deleteAudio: (String) -> Void
        var shrink: (CoreMeetingSnapshot) -> Bool

        static func live(_ store: CoreMemoryStore) -> Ops {
            Ops(
                plan: { policy, nowMs, root, busy in
                    CoreMemoryStore.accessQueue.sync { store.mediaPlan(policy: policy, nowUnixMs: nowMs, mediaRoot: root, busy: busy) }
                },
                audioPaths: { id in
                    CoreMemoryStore.accessQueue.sync {
                        [store.getMeeting(id: id)?.audioPath] + (store.meetingParts(meetingId: id) ?? []).map(\.audioPath)
                    }.compactMap { $0 }
                },
                keepMedia: { id in CoreMemoryStore.accessQueue.sync { store.getMeeting(id: id)?.keepMedia ?? false } },
                // Fails closed: no parts read (an error) counts as not transcribed.
                allTranscribed: { id in
                    let parts = CoreMemoryStore.accessQueue.sync { store.meetingParts(meetingId: id) } ?? []
                    return !parts.isEmpty && parts.allSatisfy(\.transcribed)
                },
                snapshots: { id in CoreMemoryStore.accessQueue.sync { store.listMeetingSnapshots(meetingId: id) } },
                moveAudio: { MediaMover.moveAudio(meetingId: $0, audioPaths: $1, to: $2, store: store) },
                moveSnapshots: { MediaMover.moveSnapshots(meetingId: $0, to: $1, store: store) },
                compress: { AudioCompressor.compressMeeting(meetingId: $0, audioPaths: $1, store: store) },
                // Files only: `audio_path` stays set (a NULL path would list the meeting under
                // "Meetings without audio", which offers it for deletion).
                deleteAudio: { CoreMemoryStore.deleteMeetingFiles(audioPath: $0, transcriptPath: nil, snapshotsDir: nil, recordingsDir: SpikePaths.root) },
                shrink: { SnapshotThumbnailer.shrinkSnapshot($0, store: store) }
            )
        }
    }

    static let maxMeetings = 10
    static let maxSeconds: TimeInterval = 10 * 60
    private static let backoffKey = "com.mpointer.kestrel.media.backoff"
    nonisolated private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "MediaRetentionRunner")

    @Published private(set) var progress: (done: Int, total: Int)?   // nil when idle
    /// The step being worked on now, for Settings' progress line; nil when idle.
    @Published private(set) var step: CoreMediaStep?
    @Published private(set) var lastPlan: CoreMediaPlan?             // for Settings' usage and preview line
    /// The meeting whose files are being worked on now; Transcribe again waits for it.
    private(set) var workingMeetingId: Int64?
    /// The run (or plan refresh) in flight; nil when idle.
    private(set) var current: Task<Void, Never>?

    private let ops: Ops?
    private let settings: MediaSettings
    private let deviceId: String
    private let isBusy: @MainActor (Int64) -> Bool
    private let busyIds: @MainActor () -> Set<Int64>
    private let now: () -> Date
    private let defaults: UserDefaults
    private var cancelled = false

    init(store: CoreMemoryStore?, settings: MediaSettings, deviceId: String, isBusy: @escaping @MainActor (Int64) -> Bool,
         busyIds: @escaping @MainActor () -> Set<Int64>, now: @escaping () -> Date = Date.init,
         defaults: UserDefaults = .standard, ops: Ops? = nil) {
        self.ops = ops ?? store.map(Ops.live)
        self.settings = settings
        self.deviceId = deviceId
        self.isBusy = isBusy
        self.busyIds = busyIds
        self.now = now
        self.defaults = defaults
    }

    /// A call while a run or plan refresh is in flight runs once more after it ends, in the
    /// stronger mode asked for (`.daily` over `.moveOnly`).
    func run(mode: Mode) { start(mode, reconnectGate: nil) }

    /// Plan only, no file work. Dropped while a run or refresh is in flight.
    func refreshPlan() {
        guard current == nil, let ops else { return }
        current = Task { [weak self] in
            _ = await self?.makePlan(ops)
            self?.finished()
        }
    }

    /// The mount trigger: a move-only run, but only when the volume holds the media folder,
    /// or when it brings back a stored file the last plan couldn't reach (the run then
    /// does nothing unless the fresh plan has fewer disconnected meetings).
    func volumeMounted(_ volume: URL) {
        if Self.isInside(settings.folderURL, volume) {
            start(.moveOnly, reconnectGate: nil)
        } else if let gone = lastPlan?.disconnectedMeetings, gone > 0 {
            start(.moveOnly, reconnectGate: gone)
        }
    }

    /// Stops after the current meeting.
    func cancel() {
        cancelled = true
        rerun = nil
    }

    private var rerun: Mode?

    private func start(_ mode: Mode, reconnectGate: Int?) {
        guard let ops else { return }
        guard current == nil else {
            rerun = rerun == .daily ? .daily : mode
            return
        }
        cancelled = false
        current = Task { [weak self] in
            await self?.execute(mode, ops, reconnectGate: reconnectGate)
            self?.finished()
        }
    }

    private func finished() {
        progress = nil
        step = nil
        current = nil
        if let mode = rerun {
            rerun = nil
            run(mode: mode)
        }
    }

    private func makePlan(_ ops: Ops) async -> CoreMediaPlan? {
        // The planner plans no move into a folder that doesn't exist. Only the default is
        // created: a chosen folder may be an unmounted drive.
        if settings.folderPath == nil {
            try? FileManager.default.createDirectory(at: settings.folderURL, withIntermediateDirectories: true)
        }
        let policy = settings.policy, root = settings.folderURL, busy = busyIds()
        let nowMs = Int64(now().timeIntervalSince1970 * 1000)
        let plan = await Self.onMover { ops.plan(policy, nowMs, root, busy) }
        if let plan { lastPlan = plan } else { Self.log.error("media plan failed") }
        return plan
    }

    private func execute(_ mode: Mode, _ ops: Ops, reconnectGate: Int?) async {
        let started = now()
        let policy = settings.policy, root = settings.folderURL
        guard let plan = await makePlan(ops) else { return }
        pruneBackoff(keeping: Set(plan.work.map(\.meetingId)).union(busyIds()))
        if let gate = reconnectGate, plan.disconnectedMeetings >= gate { return }
        let audioDir = settings.audioURL(deviceId: deviceId), snapshotsRoot = settings.snapshotsURL
        let items: [(id: Int64, steps: [CoreMediaStep], snaps: [Int64])] = plan.work.compactMap { w in
            let steps = w.steps.filter { mode == .daily || Self.isMove($0) }.filter { !backedOff(w.meetingId, $0, at: started) }
            return steps.isEmpty ? nil : (w.meetingId, steps, w.shrinkSnapshotIds)
        }
        let total = mode == .daily ? min(items.count, Self.maxMeetings) : items.count
        var done = 0
        progress = (0, total)
        for item in items {
            if cancelled { break }
            if mode == .daily, done >= Self.maxMeetings || now().timeIntervalSince(started) >= Self.maxSeconds { break }
            // The plan was made for these ages and this folder: if either changed, plan again,
            // in this run's mode (a move-only run leaves the age steps to the next daily one).
            guard settings.policy == policy, settings.folderURL == root else {
                rerun = rerun == .daily ? .daily : mode
                break
            }
            guard !isBusy(item.id) else { continue }
            workingMeetingId = item.id
            let settings = settings
            let onStep: @Sendable (CoreMediaStep) -> Void = { [weak self] s in Task { @MainActor in self?.step = s } }
            let results = await Self.onMover {
                Self.perform(item.steps, meetingId: item.id, shrinkIds: item.snaps, ops: ops, audioDir: audioDir, snapshotsRoot: snapshotsRoot,
                             planned: policy, current: { settings.currentPolicy }, onStep: onStep)
            }
            workingMeetingId = nil
            recordOutcomes(item.id, results)
            // A meeting whose first step failed (a move that can't succeed) doesn't use up a
            // slot, or it would starve the newer meetings behind it on every run.
            if results.first?.ok != false { done += 1 }
            progress = (done, total)
            // Players and strips re-read their paths.
            MeetingLifecycle.postMeetingsChanged([item.id])
        }
    }

    nonisolated private static func isMove(_ step: CoreMediaStep) -> Bool { step == .moveAudio || step == .moveSnapshots }

    /// Runs one meeting's steps in order and returns each attempted step's result. A failed
    /// move ends the meeting's steps (the later ones need its paths); a failed compress or
    /// shrink skips only itself.
    nonisolated private static func perform(_ steps: [CoreMediaStep], meetingId id: Int64, shrinkIds: [Int64], ops: Ops,
                                            audioDir: URL, snapshotsRoot: URL, planned: CoreMediaPolicy,
                                            current: () -> CoreMediaPolicy,
                                            onStep: (CoreMediaStep) -> Void = { _ in }) -> [(step: CoreMediaStep, ok: Bool)] {
        dispatchPrecondition(condition: .onQueue(MediaMover.queue))
        // Re-read right before each step: a move rewrites them. Deduped: the row repeats a part's path.
        func paths() -> [String] { ops.audioPaths(id).reduce(into: []) { if !$0.contains($1) { $0.append($1) } } }
        var results: [(step: CoreMediaStep, ok: Bool)] = []
        for step in steps {
            // "Keep media" may have been turned on since the plan: it allows only moves.
            if !isMove(step), ops.keepMedia(id) { continue }
            // The ages may have changed during an earlier step: the step must still be on and due.
            guard stillDue(step, planned: planned, now: current()) else { continue }
            // Re-read right before the only step that deletes a last copy: a part continued
            // since the plan may not be transcribed.
            if step == .deleteAudio, !ops.allTranscribed(id) { continue }
            onStep(step)
            let ok: Bool
            switch step {
            case .moveAudio: ok = ops.moveAudio(id, paths(), audioDir)
            case .moveSnapshots: ok = ops.moveSnapshots(id, snapshotsRoot)
            case .compress: ok = ops.compress(id, paths())
            case .deleteAudio:
                paths().forEach(ops.deleteAudio)
                ok = true
            case .shrinkImages:
                ok = ops.snapshots(id).filter { shrinkIds.contains($0.id) }.map(ops.shrink).allSatisfy { $0 }
            }
            results.append((step, ok))
            if !ok {
                log.error("meeting \(id): \(String(describing: step)) failed")
                if isMove(step) { break }
            }
        }
        return results
    }

    /// A step planned under `planned` is still due under `now` when its age is still on and
    /// no longer than planned (a shorter age is due for anything a longer one was).
    nonisolated private static func stillDue(_ step: CoreMediaStep, planned: CoreMediaPolicy, now: CoreMediaPolicy) -> Bool {
        func due(_ p: Int?, _ n: Int?) -> Bool { p.flatMap { p in n.map { $0 <= p } } ?? false }
        switch step {
        case .compress: return due(planned.compressAfterDays, now.compressAfterDays)
        case .deleteAudio: return due(planned.deleteAudioAfterDays, now.deleteAudioAfterDays)
        case .shrinkImages: return due(planned.shrinkImagesAfterDays, now.shrinkImagesAfterDays)
        case .moveAudio, .moveSnapshots: return true
        }
    }

    /// Component-wise on resolved paths.
    nonisolated private static func isInside(_ path: URL, _ folder: URL) -> Bool {
        let p = path.resolvingSymlinksInPath().standardizedFileURL.pathComponents
        let f = folder.resolvingSymlinksInPath().standardizedFileURL.pathComponents
        return p.count >= f.count && Array(p.prefix(f.count)) == f
    }

    // MARK: Backoff: ["<meeting id>:<step>": [failures, next try (unix s)]]

    private var backoff: [String: [Double]] {
        get { defaults.dictionary(forKey: Self.backoffKey) as? [String: [Double]] ?? [:] }
        set { defaults.set(newValue, forKey: Self.backoffKey) }
    }

    private static func key(_ id: Int64, _ step: CoreMediaStep) -> String { "\(id):\(step.rawValue)" }

    private func backedOff(_ id: Int64, _ step: CoreMediaStep, at date: Date) -> Bool {
        guard let entry = backoff[Self.key(id, step)], entry.count == 2 else { return false }
        return date.timeIntervalSince1970 < entry[1]
    }

    /// Only a failed compress or shrink backs off (1, 2, 4… days, at most 30); a failed move
    /// (an unplugged drive) is simply tried again on the next trigger, such as the drive mounting.
    private func recordOutcomes(_ id: Int64, _ results: [(step: CoreMediaStep, ok: Bool)]) {
        var all = backoff
        for r in results where r.step == .compress || r.step == .shrinkImages {
            let key = Self.key(id, r.step)
            if r.ok {
                all[key] = nil
            } else {
                let failures = (all[key]?.first ?? 0) + 1
                all[key] = [failures, now().timeIntervalSince1970 + min(pow(2, failures - 1), 30) * 86_400]
            }
        }
        if all != backoff { backoff = all }
    }

    /// Drops entries for meetings the plan no longer has work for (deleted, done, kept).
    private func pruneBackoff(keeping ids: Set<Int64>) {
        let all = backoff
        let kept = all.filter { k, _ in k.split(separator: ":").first.flatMap { Int64($0) }.map(ids.contains) ?? false }
        if kept.count != all.count { backoff = kept }
    }

    nonisolated private static func onMover<T>(_ body: @escaping () -> T) async -> T {
        await withCheckedContinuation { c in MediaMover.queue.async { c.resume(returning: body()) } }
    }
}
