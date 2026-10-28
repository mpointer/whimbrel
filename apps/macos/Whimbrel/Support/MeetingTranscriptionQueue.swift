import Foundation

/// Transcribes meeting parts one at a time, in the order they were queued, so a continued
/// meeting's part 2 waits for part 1. Each landed part replaces its own segments; once the
/// last outstanding part of a meeting lands and the meeting is no longer recording, the
/// whole-meeting transcript (`meetingTranscriptText`) is summarized once.
final class MeetingTranscriptionQueue {
    struct Job {
        let meetingId: Int64
        let partIndex: Int64
        /// Nil for a part that recorded no audio: nothing to transcribe, but its ending may
        /// still be what lets the earlier parts' summary run.
        let audioPath: String?
        let notes: String
    }

    typealias Transcriber = (_ audioPath: String, _ expectedSpeakerCount: Int?) throws -> (url: URL, lines: [TranscriptionService.Line])

    private let store: CoreMemoryStore
    private let transcribe: Transcriber
    /// Called on the queue's thread with (meetingId, notes, whole transcript).
    private let summarize: (Int64, String, String) -> Void
    /// Called on main: "Transcribing…" when a job starts, nil when it ends without a summary.
    private let onStage: (Int64, String?) -> Void
    private let queue: OperationQueue = {
        let q = OperationQueue()
        q.maxConcurrentOperationCount = 1
        q.qualityOfService = .userInitiated
        q.name = "com.mpointer.kestrel.meeting-transcription"
        return q
    }()
    private let lock = NSLock()
    /// Queued-or-running jobs per meeting: only the last one summarizes.
    private var outstanding: [Int64: Int] = [:]
    /// Meetings with a job in the current batch whose transcription succeeded (even silent).
    private var succeeded: Set<Int64> = []

    init(store: CoreMemoryStore,
         transcribe: @escaping Transcriber = TranscriptionService.transcribe,
         summarize: @escaping (Int64, String, String) -> Void,
         onStage: @escaping (Int64, String?) -> Void) {
        self.store = store
        self.transcribe = transcribe
        self.summarize = summarize
        self.onStage = onStage
    }

    /// Meetings with a job queued or running.
    var pendingMeetingIds: Set<Int64> { lock.withLock { Set(outstanding.keys) } }

    func enqueue(_ job: Job) {
        lock.withLock { outstanding[job.meetingId, default: 0] += 1 }
        queue.addOperation { [self] in run(job) }
    }

    /// Re-transcribes every part of the meeting whose audio is still on disk.
    func transcribeAgain(meetingId: Int64, notes: String) {
        CoreMemoryStore.accessQueue.async { [self] in
            for part in Self.partsWithAudio(store.meetingParts(meetingId: meetingId) ?? []) {
                enqueue(.init(meetingId: meetingId, partIndex: part.partIndex, audioPath: part.audioPath, notes: notes))
            }
        }
    }

    /// Summarizes a meeting whose parts are already transcribed (after a split).
    func summarizeExisting(meetingId: Int64, notes: String) {
        CoreMemoryStore.accessQueue.async { [self] in
            guard let text = store.meetingTranscriptText(meetingId: meetingId),
                  !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                DispatchQueue.main.async { [onStage] in onStage(meetingId, nil) }
                return
            }
            summarize(meetingId, notes, text)
        }
    }

    /// A completed meeting with a part whose audio is on disk but has no transcript.
    static func canTranscribeAgain(status: CoreMeetingStatus, parts: [CoreMeetingPart]) -> Bool {
        status == .completed && partsWithAudio(parts).contains { !$0.transcribed }
    }

    private static func partsWithAudio(_ parts: [CoreMeetingPart]) -> [CoreMeetingPart] {
        parts.filter { $0.audioPath.map(FileManager.default.fileExists(atPath:)) ?? false }
    }

    /// Tests only.
    func waitUntilIdle() { queue.waitUntilAllOperationsAreFinished() }

    private func run(_ job: Job) {
        let id = job.meetingId
        if job.audioPath != nil { DispatchQueue.main.async { [onStage] in onStage(id, "Transcribing…") } }
        let stored = CoreMemoryStore.accessQueue.sync { store.getMeeting(id: id) }
        let attendeeCount = stored?.attendees.count ?? 0
        let expectedSpeakerCount = attendeeCount > 0 ? attendeeCount : stored?.estimatedSpeakerCount.map { Int($0) }
        let result = job.audioPath.flatMap { try? transcribe($0, expectedSpeakerCount) }

        let transcript: String? = CoreMemoryStore.accessQueue.sync {
            // A failure leaves the part's existing segments and `transcribed` untouched.
            let notes = job.notes.isEmpty ? nil : job.notes
            store.setMeetingNotes(meetingId: id, notes: notes)
            if let result {
                let (url, lines) = result
                let segments = lines.map { NewTranscriptSegmentInput(startCs: $0.startCs, speakerLabel: $0.speakerLabel, text: $0.text) }
                _ = store.replacePartTranscriptSegments(meetingId: id, partIndex: job.partIndex, segments: segments)
                store.setMeetingPartTranscript(meetingId: id, partIndex: job.partIndex, transcriptPath: url.path)
                if job.partIndex == 1 {
                    // `meeting.transcript_path` stays part 1's (the legacy single-file view).
                    store.setMeetingTranscript(id: id, transcriptPath: url.path, notes: notes)
                    // A calendar recording where nothing came through system audio: this
                    // Mac wasn't in the call, its mic overheard one nearby.
                    let prefix = "Room audio only · "
                    if stored?.trigger == .calendar, let title = stored?.title, !title.hasPrefix(prefix), !lines.isEmpty,
                       lines.allSatisfy({ $0.speakerLabel == TranscriptionService.Speaker.me.rawValue }) {
                        store.setMeetingTitle(id: id, title: prefix + title)
                    }
                }
            }
            let (last, anySucceeded) = lock.withLock { () -> (Bool, Bool) in
                if result != nil { succeeded.insert(id) }
                outstanding[id, default: 1] -= 1
                guard outstanding[id] == 0 else { return (false, false) }
                outstanding[id] = nil
                return (true, succeeded.remove(id) != nil)
            }
            let recording = store.getMeeting(id: id)?.status == .recording
            // A later part recording now owns the live rows.
            if result != nil, !recording { store.clearLiveSegments(meetingId: id) }
            guard last, !recording else { return nil }
            // Summarize whatever landed, even if a part failed. A silent meeting still has its
            // notes and slides; only every part failing with neither has nothing to say.
            let text = store.meetingTranscriptText(meetingId: id) ?? ""
            let blank = { (s: String) in s.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            guard !blank(text) || anySucceeded || !blank(job.notes)
                    || !store.listMeetingSnapshots(meetingId: id).isEmpty else { return nil }
            return text
        }

        if let transcript {
            summarize(id, job.notes, transcript)
        } else {
            DispatchQueue.main.async { [onStage] in onStage(id, nil) }
        }
    }
}

/// Which meetings offer "Transcribe again". Views ask on main; the answer is computed on
/// accessQueue (parts + files on disk) and published, so a list row never blocks main.
@MainActor final class TranscribeAgainAvailability: ObservableObject {
    @Published private(set) var ids: Set<Int64> = []
    private var checked: Set<Int64> = []
    private let store: CoreMemoryStore

    init(store: CoreMemoryStore) { self.store = store }

    /// False until the first off-main check lands.
    func canTranscribeAgain(_ meeting: CoreMeeting) -> Bool {
        guard meeting.status == .completed, meeting.audioPath != nil else { return false }
        if !checked.contains(meeting.id) { refresh(meeting.id) }
        return ids.contains(meeting.id)
    }

    /// Rechecks one meeting, e.g. after one of its jobs finished.
    func refresh(_ id: Int64) {
        checked.insert(id)
        CoreMemoryStore.accessQueue.async { [store, weak self] in
            let can = store.getMeeting(id: id).map {
                MeetingTranscriptionQueue.canTranscribeAgain(status: $0.status, parts: store.meetingParts(meetingId: id) ?? [])
            } ?? false
            DispatchQueue.main.async {
                guard let self, self.ids.contains(id) != can else { return }
                if can { self.ids.insert(id) } else { self.ids.remove(id) }
            }
        }
    }
}
