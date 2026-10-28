import Foundation

/// Transcribes a recording in chunks while it is being written and stores provisional rows
/// (local only). Reads the in-progress recording files through `LiveAudioReader`; never
/// touches the capture code. A main-thread timer resolves the recording file URLs (they
/// may not exist yet when a meeting starts) and hands each tick to one utility-queue
/// `Runner`; every store call goes through `CoreMemoryStore.accessQueue` like the rest of
/// the app. Any failure turns the status to `.unavailable` and never affects the
/// recording. See docs/superpowers/specs/2026-10-06-live-transcription-design.md.
@MainActor
final class LiveTranscriptionService: ObservableObject {
    enum Status: Equatable {
        case idle
        case listening
        case paused
        case unavailable(String)
    }

    @Published private(set) var segments: [CoreLiveSegment] = []
    @Published private(set) var status: Status = .idle
    /// The live meeting's Ask. Here, not in the bar, so a window rebuild (a skin switch) keeps an
    /// answer streaming; torn down with the rest of the live state in `stop()`.
    let askSession = MeetingAskSession()

    private static let baseInterval = 5.0

    private let store: () -> CoreMemoryStore?
    private let modelPath: () -> String
    private let systemURL: () -> URL?
    private let micURL: () -> URL?
    private let work = DispatchQueue(label: "com.mpointer.kestrel.live-transcription", qos: .utility)
    private var timer: Timer?
    private var runner: Runner?
    private var interval = LiveTranscriptionService.baseInterval
    private var inFlight = false
    private var generation = 0

    init(
        store: @escaping () -> CoreMemoryStore?,
        modelPath: @escaping () -> String,
        systemURL: @escaping () -> URL?,
        micURL: @escaping () -> URL?
    ) {
        self.store = store
        self.modelPath = modelPath
        self.systemURL = systemURL
        self.micURL = micURL
    }

    func start(meetingId: Int64) {
        stop()
        guard FileManager.default.fileExists(atPath: modelPath()) else {
            status = .unavailable("Live transcript needs the transcription model.")
            return
        }
        generation += 1
        segments = []
        status = .listening
        runner = Runner(meetingId: meetingId, model: modelPath(), store: store())
        interval = Self.baseInterval
        schedule()
    }

    func stop() {
        generation += 1
        timer?.invalidate()
        timer = nil
        // An in-flight tick must not write rows after a stop (or after the setting was turned off).
        runner?.cancel()
        runner = nil
        inFlight = false
        // Live text must not linger in memory (or behind the UI) once live transcription stops.
        segments = []
        askSession.teardown()
        if status != .idle { status = .idle }
    }

    private func schedule() {
        timer?.invalidate()
        // Common run-loop modes, so ticks keep firing while a menu is open or a scroll is tracked.
        let t = Timer(timeInterval: interval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.fire() }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    private func fire() {
        guard let runner, !inFlight else { return }
        inFlight = true
        let myGeneration = generation
        let system = systemURL()
        let mic = micURL()
        let paused = LivePausePolicy.shouldPause(
            lowPowerMode: ProcessInfo.processInfo.isLowPowerModeEnabled,
            thermal: ProcessInfo.processInfo.thermalState
        )
        work.async { [weak self] in
            runner.attach(system: system, mic: mic)
            let outcome = runner.tick(paused: paused)
            DispatchQueue.main.async {
                guard let self, self.generation == myGeneration else { return }
                self.inFlight = false
                self.apply(outcome)
            }
        }
    }

    private func apply(_ outcome: Runner.Outcome) {
        switch outcome {
        case .listening(let rows, let slowest):
            status = .listening
            if let rows { segments = rows }
            if let slowest {
                let next = LiveCadence.nextInterval(
                    base: Self.baseInterval, current: interval,
                    chunkSeconds: slowest.chunkSeconds, tookSeconds: slowest.tookSeconds
                )
                if next != interval {
                    interval = next
                    schedule()
                }
            }
        case .paused:
            status = .paused
        case .failed(let message):
            status = .unavailable(message)
            timer?.invalidate()
            timer = nil
        }
    }

    /// The state touched only on the work queue.
    final class Runner: @unchecked Sendable {
        enum Outcome {
            case listening([CoreLiveSegment]?, (chunkSeconds: Double, tookSeconds: Double)?)
            case paused
            case failed(String)
        }

        private struct Channel {
            let name: String
            let label: String
            let reader: LiveAudioReader
        }

        private let meetingId: Int64
        private let model: String
        private let store: CoreMemoryStore?
        private var channels: [String: Channel] = [:]
        private var buffers: [String: LiveChannelBuffer] = [:]
        private var chunkIndex: [String: Int64] = [:]
        /// Absolute centisecond where the previous chunk of a channel ended, when that chunk
        /// ended with an overlap (the next chunk re-reads a little, and that text is dropped).
        private var keepFromCs: [String: Int64] = [:]
        private var transcriber: CoreLiveTranscriber?
        private var failures = 0
        private let cancelLock = NSLock()
        private var cancelled = false

        /// After this, `tick` writes nothing. Safe to call from any thread.
        func cancel() {
            cancelLock.lock()
            cancelled = true
            cancelLock.unlock()
        }

        private var isCancelled: Bool {
            cancelLock.lock()
            defer { cancelLock.unlock() }
            return cancelled
        }

        init(meetingId: Int64, model: String, store: CoreMemoryStore?) {
            self.meetingId = meetingId
            self.model = model
            self.store = store
        }

        func attach(system: URL?, mic: URL?) {
            if channels["system"] == nil, let system {
                channels["system"] = Channel(name: "system", label: TranscriptionService.Speaker.others.rawValue, reader: LiveAudioReader(url: system))
            }
            if channels["mic"] == nil, let mic {
                channels["mic"] = Channel(name: "mic", label: TranscriptionService.Speaker.me.rawValue, reader: LiveAudioReader(url: mic))
            }
        }

        func tick(paused: Bool) -> Outcome {
            // Keep reading audio into the (capped) buffers even while paused, so nothing recent is lost.
            for channel in channels.values {
                if let new = try? channel.reader.readNewSamples(maxSeconds: 30), !new.isEmpty {
                    buffers[channel.name, default: LiveChannelBuffer()].append(new)
                }
            }
            if paused { return .paused }
            if transcriber == nil {
                transcriber = CoreLiveTranscriber(modelPath: model)
                if transcriber == nil { return .failed("Live transcript could not load the transcription model.") }
            }
            var wrote = false
            var slowest: (chunkSeconds: Double, tookSeconds: Double)?
            for channel in channels.values {
                guard var buffer = buffers[channel.name],
                      let cut = CoreMemoryStore.liveNextCut(samples: buffer.samples, sampleRate: 16_000) else { continue }
                let taken = buffer.take(count: cut.at, keepOverlap: cut.overlap)
                buffers[channel.name] = buffer
                let startCs = LiveChannelBuffer.centiseconds(forSample: taken.startSample)
                let started = Date()
                guard let json = transcriber?.transcribe(
                    samples: taken.samples, startOffsetCs: startCs,
                    threads: LiveCadence.threads(forCores: ProcessInfo.processInfo.activeProcessorCount)
                ) else {
                    failures += 1
                    if failures >= 3 { return .failed("Live transcript paused after repeated errors.") }
                    continue
                }
                failures = 0
                let took = Date().timeIntervalSince(started)
                let chunkSeconds = Double(taken.samples.count) / Double(LiveChannelBuffer.sampleRate)
                if slowest == nil || took / max(chunkSeconds, 1) > (slowest!.tookSeconds / max(slowest!.chunkSeconds, 1)) {
                    slowest = (chunkSeconds, took)
                }
                let kept = trimOverlap(json: json, channel: channel.name)
                let index = (chunkIndex[channel.name] ?? -1) + 1
                chunkIndex[channel.name] = index
                // The next chunk may re-read `overlap` samples: only text after the end of this one is new.
                let endCs = LiveChannelBuffer.centiseconds(forSample: taken.startSample + cut.at)
                keepFromCs[channel.name] = cut.overlap > 0 ? endCs : 0
                if let store {
                    // Checked inside the queued write, so it is serialized with the clear that
                    // follows a stop or the setting being turned off: a write that runs after the
                    // clear sees `cancelled` and writes nothing, one that ran before is cleared.
                    let ok = CoreMemoryStore.accessQueue.sync { [self] in
                        if isCancelled { return false }
                        return store.insertLiveSegments(
                            meetingId: meetingId, channel: channel.name, chunkIndex: index,
                            segmentsJSON: kept, speakerLabel: channel.label
                        )
                    }
                    if ok { wrote = true }
                }
            }
            var rows: [CoreLiveSegment]?
            if wrote, !isCancelled, let store {
                rows = CoreMemoryStore.accessQueue.sync { store.listLiveSegments(meetingId: meetingId) }
            }
            return .listening(rows, slowest)
        }

        /// Drops text from the re-read overlap that the previous chunk already produced.
        private func trimOverlap(json: String, channel: String) -> String {
            guard let from = keepFromCs[channel], from > 0,
                  let data = json.data(using: .utf8),
                  var segments = try? JSONDecoder().decode([TimedSegment].self, from: data) else { return json }
            segments = segments.filter { ($0.startCs + $0.endCs) / 2 >= from }
            guard let out = try? JSONEncoder().encode(segments), let text = String(data: out, encoding: .utf8) else { return json }
            return text
        }

        private struct TimedSegment: Codable {
            let startCs: Int64
            let endCs: Int64
            let text: String
            enum CodingKeys: String, CodingKey {
                case startCs = "start_cs", endCs = "end_cs", text
            }
        }
    }
}
