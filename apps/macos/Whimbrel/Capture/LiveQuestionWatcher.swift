import Foundation
import os

/// Where a live question alert goes once found: a notification, an email, the meeting page.
@MainActor
protocol AlertDelivering {
    func alertDetected(_ alert: CoreLiveAlert)
    func alertDrafted(_ alert: CoreLiveAlert, answer: CoreChatAnswer?)
    func alertsCapped(meetingId: Int64, extra: Int)
}

/// Watches one recording's live transcript for questions put to the user ("Forward me a live
/// question"). Core picks the candidate lines and judges them; this keeps the meeting's state:
/// lines seen, candidates sent, restatements for dedupe and the alert ceiling. A candidate waits
/// until 30 s of later lines exist (or the recording stops) so the model can see whether someone
/// else answered. Candidates are judged one at a time, so dedupe sees every earlier alert; drafts
/// run on their own and never hold up the next detection. Times are meeting-absolute: live
/// `start_cs` counts from the current part's start, so a continued meeting's lines are shifted
/// by the part's offset before any rule sees them, and stored alerts use the same units.
/// See docs/superpowers/specs/2026-11-02-live-question-alerts-design.md.
@MainActor
final class LiveQuestionWatcher {
    /// At most this many alerts per meeting; the rest are stored and reported in one capped call.
    static let maxAlerts = 6
    /// Core's trailing context (`FOLLOWING_CS`, 30 s) and lead context (`CONTEXT_CS`, 2 min).
    private static let followingCs: Int64 = 3_000
    private static let contextCs: Int64 = 12_000
    /// The draft gets the last 5 minutes of the meeting as background.
    private static let recentCs: Int64 = 30_000
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "live-question")

    let meetingId: Int64
    private let meetingTitle: String
    private let names: [String]
    private let detect: ([CoreLiveLine], Int) async -> CoreLiveQuestionVerdict?
    private let draft: (CoreLiveQuestionVerdict, [CoreLiveLine]) async -> CoreChatAnswer?
    private let deliver: AlertDelivering
    private let now: () -> Date
    private let insert: (CoreNewLiveAlert) async -> Int64?
    private let saveDraft: (Int64, CoreChatAnswer) async -> Void
    /// The title says this is room audio, so the mic counts at once (core's rule).
    private let roomAudioTitle: Bool

    private var lines: [CoreLiveLine] = []
    private var seenIds = Set<Int64>()
    private var dispatched = Set<Int64>()
    private var stopped = false
    private var cancelled = false
    /// Set on the worker once every line before the stop has been added.
    private var linesComplete = false
    private var earlier: [(String, Int64)] = []
    private(set) var alertCount = 0
    /// The latest stored alert's `at_cs` when this watcher was built; candidates at or before it
    /// were judged by an earlier watcher for this meeting.
    private var highWaterCs: Int64 = -1
    /// Centiseconds from the meeting's start to this part's, from `seed`.
    private var partOffsetCs: Int64 = 0
    private var extra = 0
    private var worker: Task<Void, Never>?
    private var drafts: [Task<Void, Never>] = []

    /// `insert` stores the alert (nil on failure) and `saveDraft` adds its draft; `seed` returns
    /// the meeting's stored alerts, so a watcher rebuilt mid-meeting (alerts toggled, a restart,
    /// a continued meeting) keeps dedupe, the ceiling and what was already judged, and the
    /// current part's offset (`partOffsetCs(_:)`). All three go through the store on
    /// `CoreMemoryStore.accessQueue` in the app.
    init(
        meetingId: Int64, meetingTitle: String, names: [String],
        detect: @escaping ([CoreLiveLine], Int) async -> CoreLiveQuestionVerdict?,
        draft: @escaping (CoreLiveQuestionVerdict, [CoreLiveLine]) async -> CoreChatAnswer?,
        deliver: AlertDelivering, now: @escaping () -> Date,
        insert: @escaping (CoreNewLiveAlert) async -> Int64?,
        saveDraft: @escaping (Int64, CoreChatAnswer) async -> Void,
        seed: @escaping () async -> (alerts: [CoreLiveAlert], partOffsetCs: Int64)
    ) {
        self.meetingId = meetingId
        self.meetingTitle = meetingTitle
        self.names = names
        self.detect = detect
        self.draft = draft
        self.deliver = deliver
        self.now = now
        self.insert = insert
        self.saveDraft = saveDraft
        roomAudioTitle = meetingTitle.hasPrefix(Self.roomAudioPrefix)
        // The first job on the serial worker, so every line and detection sees it.
        enqueue { [self] in
            let (stored, offset) = await seed()
            partOffsetCs = offset
            earlier = stored.map { ($0.restatement, $0.createdAtUnixMs) }
            alertCount = min(stored.count, Self.maxAlerts)
            extra = stored.count - alertCount
            highWaterCs = stored.map(\.atCs).max() ?? -1
        }
    }

    /// Core's `ROOM_AUDIO_PREFIX`: a recording titled this way overheard the room on the mic.
    static let roomAudioPrefix = "Room audio only · "

    static func shouldRun(settings: LiveQuestionSettings, names: [String]) -> Bool {
        settings.enabled && !names.isEmpty
    }

    /// Centiseconds from the meeting's first part to its latest, the one recording now (core's
    /// `part_offset_cs`); 0 for a meeting in one part. Live rows add this to be meeting-absolute.
    static func partOffsetCs(_ parts: [CoreMeetingPart]) -> Int64 {
        guard let first = parts.min(by: { $0.partIndex < $1.partIndex }),
              let last = parts.max(by: { $0.partIndex < $1.partIndex }) else { return 0 }
        return max(0, (last.startUnixMs - first.startUnixMs) / 10)
    }

    /// The meeting's live rows (all of them or just new ones); rows already seen are skipped.
    /// Lines join on the worker, after `seed` has set the part's offset.
    func ingest(_ rows: [CoreLiveSegment]) {
        guard !stopped else { return }
        let fresh = rows.filter { seenIds.insert($0.id).inserted }
        guard !fresh.isEmpty else { return }
        enqueue { [self] in
            lines += fresh.map { CoreLiveLine(id: $0.id, channel: $0.channel, startCs: $0.startCs + partOffsetCs, text: $0.text) }
            lines.sort { ($0.startCs, $0.id) < ($1.startCs, $1.id) }
            await judgeReady()
        }
    }

    /// The recording stopped: judge every candidate still waiting for later lines, then report
    /// the alerts over the ceiling, once.
    func flush() {
        guard !stopped else { return }
        stopped = true
        enqueue { [self] in
            linesComplete = true
            await judgeReady()
            if extra > 0, !cancelled { deliver.alertsCapped(meetingId: meetingId, extra: extra) }
        }
    }

    /// Alerts were turned off: judge nothing more (a draft already running still finishes).
    func cancel() {
        stopped = true
        cancelled = true
    }

    /// Waits for every queued detection (tests).
    func settleDetection() async {
        await worker?.value
    }

    /// Waits for every queued detection and every draft (tests).
    func settle() async {
        await settleDetection()
        for task in drafts { await task.value }
    }

    /// Judges, one at a time, each candidate with 30 s of later lines (every one once stopped).
    private func judgeReady() async {
        guard let latest = lines.last?.startCs else { return }
        // Room audio: the title says so, or the recording ended with no system audio at all.
        // Core also treats a minute of mic-only lines as room audio, until the first system line.
        let roomAudio = roomAudioTitle || (linesComplete && !lines.contains { $0.channel == "system" })
        for index in CoreLiveQuestion.candidates(lines: lines, names: names, roomAudio: roomAudio) where index < lines.count {
            let candidate = lines[index]
            guard !dispatched.contains(candidate.id), stopped || latest >= candidate.startCs + Self.followingCs else { continue }
            dispatched.insert(candidate.id)
            let window = lines.filter { $0.startCs >= candidate.startCs - Self.contextCs && $0.startCs <= candidate.startCs + Self.followingCs }
            guard let windowIndex = window.firstIndex(of: candidate) else { continue }
            let recent = lines.filter { $0.startCs >= candidate.startCs - Self.recentCs && $0.startCs <= candidate.startCs + Self.followingCs }
            await judge(candidate, window: window, index: windowIndex, recent: recent)
        }
    }

    private func enqueue(_ job: @escaping @MainActor () async -> Void) {
        let previous = worker
        worker = Task { @MainActor in
            await previous?.value
            await job()
        }
    }

    private func judge(_ candidate: CoreLiveLine, window: [CoreLiveLine], index: Int, recent: [CoreLiveLine]) async {
        guard candidate.startCs > highWaterCs, !cancelled, let verdict = await detect(window, index), !cancelled else { return }
        let nowMs = Int64(now().timeIntervalSince1970 * 1000)
        guard !CoreLiveQuestion.isDuplicate(verdict.restatement, earlier: earlier, nowMs: nowMs) else { return }
        earlier.append((verdict.restatement, nowMs))
        let new = CoreNewLiveAlert(
            meetingId: meetingId, meetingTitle: meetingTitle, quote: verdict.quote, restatement: verdict.restatement,
            asker: verdict.asker, atCs: candidate.startCs, createdAtUnixMs: nowMs
        )
        let id = await insert(new)
        if id == nil { Self.log.error("live question alert not stored; delivering anyway") }
        // Over the ceiling: stored for the meeting page, no notification and no draft.
        guard alertCount < Self.maxAlerts else {
            extra += 1
            return
        }
        alertCount += 1
        let alert = CoreLiveAlert(
            id: id ?? -1, meetingId: meetingId, originDeviceId: nil, sourceId: id ?? -1, sourceMeetingId: meetingId,
            meetingTitle: meetingTitle, quote: new.quote, restatement: new.restatement, asker: new.asker, atCs: new.atCs,
            createdAtUnixMs: nowMs, draftJson: nil, emailStatus: nil, notifiedAtUnixMs: nil
        )
        deliver.alertDetected(alert)
        let draft = draft
        let saveDraft = saveDraft
        let deliver = deliver
        drafts.append(Task { @MainActor in
            let answer = await draft(verdict, recent)
            if let answer, let id { await saveDraft(id, answer) }
            deliver.alertDrafted(alert, answer: answer)
        })
    }
}
