import Foundation
import os

/// Transcribes meeting parts one at a time, in the order they were queued, so a continued
/// meeting's part 2 waits for part 1. Each landed part replaces its own segments; once the
/// last outstanding part of a meeting lands and the meeting is no longer recording, its
/// speakers are named and the whole-meeting transcript (`meetingTranscriptText`) is summarized once.
final class MeetingTranscriptionQueue {
    struct Job {
        let meetingId: Int64
        let partIndex: Int64
        /// Nil for a part that recorded no audio: nothing to transcribe, but its ending may
        /// still be what lets the earlier parts' summary run.
        let audioPath: String?
        let notes: String
        /// "Still in a meeting?" stopped it: once the full transcript is stored, trim it back
        /// to the last speech (`StillInMeeting.autoTrimKeepMs`). In memory only, so a quit
        /// before then leaves it untrimmed.
        var autoTrim = false
    }

    /// `language` is a whisper code to force, nil to detect; the result carries the language used
    /// and the diarized voices.
    typealias Transcriber = (_ audioPath: String, _ expectedSpeakerCount: Int?, _ language: String?) throws
        -> (url: URL, lines: [TranscriptionService.Line], language: String?, voices: [CoreVoice])

    private let store: CoreMemoryStore
    private let transcribe: Transcriber
    /// Called on the queue's thread with (meetingId, notes, whole transcript).
    private let summarize: (Int64, String, String) -> Void
    /// Called on the queue's thread, before the summary, with a single-part meeting's voices.
    /// ponytail: a meeting with more than one part is not named: diarization restarts per part, so
    /// one name would land on different people. Multi-part naming waits for per-part speaker identity.
    private let nameSpeakers: (Int64, [CoreVoice]) -> Void
    /// Called on the queue's thread, after an auto-trim job's meeting is fully transcribed and
    /// before naming and the summary, with (meetingId, keepMs): trims the rows and files.
    private let trim: (Int64, Int64) -> Void
    /// Called on main: "Transcribing…" when a job starts, nil when it ends without a summary.
    private let onStage: (Int64, String?) -> Void
    /// Called on main when a part's transcription threw; the part stays untranscribed.
    private let onFailure: (Int64) -> Void
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "transcription-queue")
    /// Off: every part is forced to English with the English model, as before.
    private let multilingual: () -> Bool
    /// Whether the model this meeting needs is on disk.
    private let modelReady: () -> Bool
    /// Whether setup is still downloading models.
    private let modelsDownloading: () -> Bool
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

    init(store: CoreMemoryStore,
         transcribe: @escaping Transcriber = TranscriptionService.transcribe,
         summarize: @escaping (Int64, String, String) -> Void,
         nameSpeakers: @escaping (Int64, [CoreVoice]) -> Void = { _, _ in },
         trim: @escaping (Int64, Int64) -> Void = { _, _ in },
         onStage: @escaping (Int64, String?) -> Void,
         onFailure: @escaping (Int64) -> Void = { _ in },
         multilingual: @escaping () -> Bool = {
             usesMultilingual(enabled: AppLanguage.multilingual(),
                              modelPresent: FileManager.default.fileExists(atPath: TranscriptionService.multilingualModelPath.path))
         },
         modelReady: @escaping () -> Bool = { TranscriptionEngines.batch(language: "en").isReady },
         modelsDownloading: @escaping () -> Bool = { false }) {
        self.store = store
        self.transcribe = transcribe
        self.summarize = summarize
        self.nameSpeakers = nameSpeakers
        self.trim = trim
        self.onStage = onStage
        self.onFailure = onFailure
        self.multilingual = multilingual
        self.modelReady = modelReady
        self.modelsDownloading = modelsDownloading
    }

    /// A first meeting can end before setup's downloads do. Holds the queue until the model
    /// lands, so the meeting is transcribed rather than failing. Returns once the model is
    /// there, the download stops, or `limit` passes; a failed run then reports as it always did.
    @discardableResult
    static func waitForModel(ready: () -> Bool, downloading: () -> Bool, pause: () -> Void, limit: Int = 900) -> Bool {
        var polls = 0
        while !ready(), downloading(), polls < limit {
            pause()
            polls += 1
        }
        return ready()
    }

    /// Multilingual mode takes effect once its model is on disk; until then meetings are
    /// transcribed in English rather than failing.
    static func usesMultilingual(enabled: Bool, modelPresent: Bool) -> Bool { enabled && modelPresent }

    /// Meetings with a job queued or running.
    var pendingMeetingIds: Set<Int64> { lock.withLock { Set(outstanding.keys) } }

    func enqueue(_ job: Job) {
        lock.withLock { outstanding[job.meetingId, default: 0] += 1 }
        queue.addOperation { [self] in run(job) }
    }

    /// Re-transcribes every part of the meeting whose audio is still on disk. With `language`,
    /// the meeting's stored language is replaced first (`.some(nil)` is Automatic: detect again).
    func transcribeAgain(meetingId: Int64, notes: String, language: String?? = .none) {
        CoreMemoryStore.accessQueue.async { [self] in
            if case let .some(lang) = language { store.setMeetingSpokenLanguage(meetingId: meetingId, language: lang) }
            let parts = Self.partsWithAudio(store.meetingParts(meetingId: meetingId) ?? [])
            // Nothing to transcribe: release the caller's "processing" state.
            if parts.isEmpty { DispatchQueue.main.async { [onStage] in onStage(meetingId, nil) } }
            for part in parts {
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
    /// `noSpeech` (see `noSpeech(parts:transcript:)`) keeps it on offer after a run that heard nothing.
    static func canTranscribeAgain(status: CoreMeetingStatus, parts: [CoreMeetingPart], noSpeech: Bool = false) -> Bool {
        status == .completed && (noSpeech ? !partsWithAudio(parts).isEmpty : partsWithAudio(parts).contains { !$0.transcribed })
    }

    /// Every part with audio was transcribed, and together they have no words.
    static func noSpeech(parts: [CoreMeetingPart], transcript: String?) -> Bool {
        let withAudio = partsWithAudio(parts)
        return !withAudio.isEmpty && withAudio.allSatisfy(\.transcribed)
            && (transcript ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    static func partsWithAudio(_ parts: [CoreMeetingPart]) -> [CoreMeetingPart] {
        parts.filter { $0.audioPath.map(FileManager.default.fileExists(atPath:)) ?? false }
    }

    /// On accessQueue: the auto-trim's keep from the stored lines and this part's line ends.
    private func autoTrimKeepMs(_ job: Job, partLines: [TranscriptionService.Line]) -> Int64? {
        let id = job.meetingId
        let parts = store.meetingParts(meetingId: id) ?? []
        let offset = (parts.first { $0.partIndex == job.partIndex }?.startUnixMs ?? 0) - (parts.first?.startUnixMs ?? 0)
        var stored: [TranscriptionService.Line] = []
        while let page = store.getTranscriptSegmentsPage(meetingId: id, offset: stored.count, limit: 500), !page.segments.isEmpty {
            stored += page.segments.map { TranscriptionService.Line(speakerLabel: $0.speakerLabel, startCs: $0.startCs, text: $0.text) }
        }
        return StillInMeeting.autoTrimKeepMs(stored: stored, partLines: partLines, partOffsetMs: offset,
                                             lengthMs: store.meetingLengthMs(meetingId: id) ?? 0)
    }

    /// Tests only.
    func waitUntilIdle() { queue.waitUntilAllOperationsAreFinished() }

    private func run(_ job: Job) {
        let id = job.meetingId
        // Transcription gets the memory the local model holds; its next call starts it again.
        if job.audioPath != nil { CoreMemoryStore.stopLocalModel() }
        if job.audioPath != nil, !modelReady(), modelsDownloading() {
            let waiting = String(localized: "Waiting for the transcription model…")
            DispatchQueue.main.async { [onStage] in onStage(id, waiting) }
            Self.waitForModel(ready: modelReady, downloading: modelsDownloading, pause: { Thread.sleep(forTimeInterval: 2) })
        }
        if job.audioPath != nil { DispatchQueue.main.async { [onStage] in onStage(id, "Transcribing…") } }
        let stored = CoreMemoryStore.accessQueue.sync { store.getMeeting(id: id) }
        let attendeeCount = stored?.attendees.count ?? 0
        let expectedSpeakerCount = attendeeCount > 0 ? attendeeCount : stored?.estimatedSpeakerCount.map { Int($0) }
        let multilingual = multilingual()
        let language = multilingual ? CoreMemoryStore.accessQueue.sync { store.meetingSpokenLanguage(meetingId: id) } : "en"
        var result: (url: URL, lines: [TranscriptionService.Line], language: String?, voices: [CoreVoice])?
        if let audioPath = job.audioPath {
            do {
                result = try transcribe(audioPath, expectedSpeakerCount, language)
            } catch {
                Self.log.error("meeting \(id) part \(job.partIndex): transcription failed: \(String(describing: error), privacy: .public)")
                DispatchQueue.main.async { [onFailure] in onFailure(id) }
            }
        }

        let (finish, onePart): (Bool, Bool) = CoreMemoryStore.accessQueue.sync {
            // A failure leaves the part's existing segments and `transcribed` untouched.
            let notes = job.notes.isEmpty ? nil : job.notes
            store.setMeetingNotes(meetingId: id, notes: notes)
            if let result {
                let (url, lines, used, _) = result
                if multilingual, language == nil, let used, !lines.isEmpty {
                    store.setMeetingSpokenLanguage(meetingId: id, language: used)
                }
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
            let last = lock.withLock { () -> Bool in
                outstanding[id, default: 1] -= 1
                guard outstanding[id] == 0 else { return false }
                outstanding[id] = nil
                return true
            }
            let recording = store.getMeeting(id: id)?.status == .recording
            // A later part recording now owns the live rows.
            if result != nil, !recording { store.clearLiveSegments(meetingId: id) }
            return (last && !recording, (store.meetingParts(meetingId: id)?.count ?? 0) <= 1)
        }
        if finish, job.autoTrim,
           let keep = CoreMemoryStore.accessQueue.sync(execute: { autoTrimKeepMs(job, partLines: result?.lines ?? []) }) {
            trim(id, keep)
        }
        // Named off accessQueue: naming may wait on a model.
        if finish, onePart, let voices = result?.voices, !voices.isEmpty { nameSpeakers(id, voices) }

        let transcript: String? = !finish ? nil : CoreMemoryStore.accessQueue.sync {
            // Summarize whatever landed, even if a part failed. A silent meeting still has its
            // notes and slides. With no words, notes or slides there is nothing to summarize,
            // and a model asked to anyway invents a meeting (10/30 "Quick Debrief").
            let text = store.meetingTranscriptText(meetingId: id) ?? ""
            let blank = { (s: String) in s.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            guard !blank(text) || !blank(job.notes)
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
    /// Meetings whose last run heard nothing.
    @Published private(set) var noSpeechIds: Set<Int64> = []
    /// Meetings whose last run threw (this launch only; the log has the error).
    @Published private(set) var failedIds: Set<Int64> = []
    private var checked: Set<Int64> = []
    private let store: CoreMemoryStore

    init(store: CoreMemoryStore) { self.store = store }

    /// False until the first off-main check lands.
    func canTranscribeAgain(_ meeting: CoreMeeting) -> Bool {
        guard meeting.status == .completed, meeting.audioPath != nil else { return false }
        if !checked.contains(meeting.id) { refresh(meeting.id) }
        return ids.contains(meeting.id)
    }

    func markFailed(_ id: Int64) { failedIds.insert(id) }

    /// Rechecks one meeting, e.g. after one of its jobs finished.
    func refresh(_ id: Int64) {
        checked.insert(id)
        CoreMemoryStore.accessQueue.async { [store, weak self] in
            var pending = false, quiet = false
            if let m = store.getMeeting(id: id) {
                let parts = store.meetingParts(meetingId: id) ?? []
                pending = MeetingTranscriptionQueue.canTranscribeAgain(status: m.status, parts: parts)
                quiet = m.status == .completed
                    && MeetingTranscriptionQueue.noSpeech(parts: parts, transcript: store.meetingTranscriptText(meetingId: id))
            }
            DispatchQueue.main.async {
                guard let self else { return }
                let can = pending || quiet
                if self.ids.contains(id) != can { if can { self.ids.insert(id) } else { self.ids.remove(id) } }
                if self.noSpeechIds.contains(id) != quiet { if quiet { self.noSpeechIds.insert(id) } else { self.noSpeechIds.remove(id) } }
                if !pending { self.failedIds.remove(id) }
            }
        }
    }
}
