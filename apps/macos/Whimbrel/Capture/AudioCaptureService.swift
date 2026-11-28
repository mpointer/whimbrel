import AVFoundation
import Foundation
import OSLog
import ScreenCaptureKit
import WhimbrelUI

/// FR-MTG-3 / FR-MTG-10 spike: system audio via ScreenCaptureKit and microphone via
/// AVAudioEngine, written as two separate timestamped files. If system audio can't be
/// captured (e.g. another process already holds the tap) this degrades to mic-only and
/// surfaces a reason instead of failing silently.
@MainActor
final class AudioCaptureService: NSObject, ObservableObject {
    @Published private(set) var systemAudioActive = false
    @Published private(set) var micActive = false
    @Published private(set) var degradedReason: String?
    /// Set when the microphone stream stops for good mid-recording (see
    /// `checkMicHealth`). Separate from `degradedReason`, which is system audio's.
    @Published private(set) var micDegradedReason: String?
    @Published private(set) var systemAudioFileURL: URL?
    @Published private(set) var micAudioFileURL: URL?
    /// Set once, at the top of `start()` — true if recording started while the Mac's
    /// default audio output was its own built-in speakers (headphones/AirPods/
    /// Bluetooth/USB all read false). Drives a dismissible UI warning (see
    /// TimelineView.swift's captureStatusHeader). Not re-checked if the output device
    /// changes mid-recording — a known, accepted limitation for this pass.
    @Published private(set) var recordingOnBuiltInSpeakers = false
    /// FR-MTG-5: last time either capture stream delivered a buffer with actual audible
    /// content (above `silenceAmplitudeThreshold`) — deliberately NOT the same thing as
    /// "is a device pinned running," which `AudioDeviceActivity`'s Core Audio check
    /// reports true for as long as our own `AVAudioEngine` tap is installed, even during
    /// real silence. `MeetingEndDetector` derives its silence-window heuristic from
    /// this instead of that device-running check.
    private(set) var lastNonSilentAt = Date()
    /// Loudest peak per track since the last `take()`, for the silence warning (`SilenceWatch`).
    let micPeak = PeakMeter()
    let systemPeak = PeakMeter()
    /// A second reading of the same peaks for the level meter. It has its own
    /// state, so reading it never disturbs the silence meter above, and it records nothing until
    /// `startLevelSampling()` runs.
    private let micLevelPeak = LevelPeak()
    private let systemLevelPeak = LevelPeak()
    /// The latest level, 0 to 1, about 8 times a second. Zero while paused or stopped. Read, drawn and
    /// thrown away: nothing stores it.
    @Published private(set) var inputLevel: Float = 0
    private var levelTimer: Timer?
    /// A recording pause. The streams
    /// are closed and the files stay open. This is not work memory's capture pause.
    @Published private(set) var isPaused = false {
        didSet { systemSink.setPaused(isPaused) }
    }
    /// Between `resumeRecording()` starting and the streams being back.
    @Published private(set) var isResuming = false
    /// When `start()` began: the zero of the stored audio's clock, which paused spans are measured from.
    private(set) var recordingStartedAt: Date?
    /// The 6 h cap or low disk ended this recording. Main actor, at most once per `start()`;
    /// the owner ends the meeting, which calls `stop()`.
    var onForcedStop: ((RecordingStop) -> Void)?
    private var forcedStopReported = false

    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "audio-capture")
    private let streamOutputQueue = DispatchQueue(label: "com.mpointer.kestrel.sck-audio-output")

    private var stream: SCStream?
    /// The system stream's writer and converter as the stream's own queue sees them (see
    /// `SystemAudioSink`). Changed only through `setSystemOutput`, which keeps it and the two
    /// properties below in step.
    private let systemSink = SystemAudioSink()
    private var systemWriter: RecordingWriter?
    /// 48 kHz stereo (what ScreenCaptureKit delivers) → the file's 16 kHz mono. One per
    /// recording: the resampler is stateful across buffers.
    private var systemConverter: AVAudioConverter?
    /// Frames in the system-audio file, padding included (despite the name). Copied from
    /// the writer; kept separately so it survives the writer being dropped on degrade.
    private var systemAudioBytesWritten = 0
    /// The system file's timeline, like the mic's: a reopen pads the gap from here.
    private var systemStartedAt = Date()
    private var lastSystemBufferAt = Date()
    private var systemReopens = 0
    private var systemFilter: SCContentFilter?
    private var systemConfig: SCStreamConfiguration?
    private var systemAudioWatchdogTask: Task<Void, Never>?

    private let engine = AVAudioEngine()
    private var micWriter: RecordingWriter?
    private var micTapInstalled = false
    /// The mic file's timeline: frames written (padding included) since `micStartedAt`,
    /// so a restart can pad the gap and later mic lines stay aligned with system audio's
    /// (the bleed filter and the merge match the two channels by time).
    private var micStartedAt = Date()
    /// Copied from `micWriter` in the tap closure (off the main actor), as before.
    private var micFramesWritten: AVAudioFramePosition = 0
    /// When the tap last delivered any buffer, audible or not. A muted mic still
    /// delivers buffers; a stopped engine delivers none.
    private var lastMicBufferAt = Date()
    private var micStallRestarts = 0
    private var micInputDeviceID: AudioDeviceID?
    private var healthTimer: Timer?
    private var engineConfigObserver: NSObjectProtocol?

    /// Who owns the service now (`AudioStartEpochs`): every `start()` and every `stop()` moves it on. A
    /// `start()` that resumes after an await and finds it moved is stale, and must publish nothing: it
    /// closes only what it opened itself and never turns the microphone on. The cases, with the epoch
    /// each step sees:
    ///  1. start1 (epoch 1) awaiting; stop() (2); start1 resumes: stale. It closes its own stream and file.
    ///  2. start1 (1) awaiting; stop() (2); start2 begins (3); start1 resumes: stale, closes its own stream
    ///     only, so start2's streams are untouched; start2 resumes: epoch 3 is current, so it publishes.
    ///  3. start1 (1) awaiting; start2 begins with no stop (2); start1 resumes: stale (superseded), as in 2.
    private var epochs = AudioStartEpochs()

    /// A recording that never started has no files: drop the paths of the last one, so nothing reads
    /// them as this recording's.
    func forgetLastRecording() {
        systemAudioFileURL = nil
        micAudioFileURL = nil
    }

    func start() async {
        let epoch = epochs.begin()
        degradedReason = nil
        micDegradedReason = nil
        if isPaused { isPaused = false }
        if isResuming { isResuming = false }
        recordingStartedAt = Date()
        recordingOnBuiltInSpeakers = AudioOutputRouteChecker.isUsingBuiltInSpeakers()
        // Reset per-session system-audio state up front: a previous session's URL
        // surviving into a session that degrades before creating its own file would make
        // AppState.stopCapture() record the *previous* meeting's audio path against this
        // meeting's row. Same reasoning applies to micAudioFileURL — see
        // startMicrophone's doc comment for the specific failure path this guards
        // against.
        systemAudioFileURL = nil
        systemAudioBytesWritten = 0
        micAudioFileURL = nil
        lastNonSilentAt = Date()
        forcedStopReported = false
        let timestamp = SpikePaths.sessionTimestamp()
        await startSystemAudio(timestamp: timestamp, epoch: epoch)
        // Stopped, or superseded by a newer start, while the system stream opened: the microphone must not
        // start, and startSystemAudio already published nothing.
        guard epochs.owns(epoch) else { return }
        startMicrophone(timestamp: timestamp)
        healthTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in
                // While paused no buffer should arrive, so the stall restarts and the device check are held.
                // The length cap and the disk check keep running: the cap counts the pause.
                if RecordingPauseRules.checksRun(paused: self?.isPaused ?? false) {
                    self?.checkMicHealth()
                    self?.checkSystemHealth()
                }
                self?.checkRecordingLimits()
            }
        }
    }

    func stop() {
        epochs.stop()
        stopLevelSampling()
        if isPaused { isPaused = false }
        if isResuming { isResuming = false }
        systemAudioWatchdogTask?.cancel()
        systemAudioWatchdogTask = nil
        healthTimer?.invalidate()
        healthTimer = nil
        if let engineConfigObserver {
            NotificationCenter.default.removeObserver(engineConfigObserver)
            self.engineConfigObserver = nil
        }

        if micTapInstalled {
            engine.inputNode.removeTap(onBus: 0)
            micTapInstalled = false
        }
        if engine.isRunning {
            engine.stop()
        }
        micActive = false
        micWriter = nil

        if let stream {
            Task {
                try? await stream.stopCapture()
            }
        }
        stream = nil
        systemAudioActive = false
        setSystemOutput(nil, nil)
        if let systemAudioFileURL {
            cleanUpEmptySystemAudioFile(at: systemAudioFileURL)
        }
    }

    // MARK: Recording pause

    /// What a resume did. `failed` carries the existing degraded reasons, already set on this service.
    enum ResumeOutcome: Equatable { case resumed, failed(reason: String), notPaused }

    /// Pauses the recording: removes the microphone tap and stops the engine (so macOS turns its
    /// microphone indicator off), stops the system stream, and keeps both files and the system stream's
    /// configuration. `micActive` and `systemAudioActive` stay as they are: a paused recording is
    /// still a recording for every reader, and a pause never marks the audio as degraded.
    func pauseRecording() {
        guard !isPaused, !isResuming, healthTimer != nil else { return }
        isPaused = true
        systemAudioWatchdogTask?.cancel()
        systemAudioWatchdogTask = nil
        if micTapInstalled {
            engine.inputNode.removeTap(onBus: 0)
            micTapInstalled = false
        }
        if engine.isRunning { engine.stop() }
        if let old = stream {
            stream = nil
            Task { try? await old.stopCapture() }
        }
        micLevelPeak.reset()
        systemLevelPeak.reset()
        micPeak.reset()
        systemPeak.reset()
        inputLevel = 0
        log.notice("[recording-pause] paused")
    }

    /// Resumes: pads both files to now with the existing padding so the recording keeps clock time,
    /// installs the tap again and starts the engine, and opens the system stream again. A stream that
    /// cannot start is reported with the existing degraded reasons.
    func resumeRecording() async -> ResumeOutcome {
        guard isPaused, !isResuming else { return .notPaused }
        isResuming = true
        defer { isResuming = false }
        var failure: String?

        if micActive, let writer = micWriter {
            padMicFile(writer, upTo: Date())
            lastMicBufferAt = Date()
            do {
                try installMicTap(writer: writer)
            } catch {
                log.error("Microphone could not start again after a pause: \(error.localizedDescription, privacy: .public)")
                micActive = false
                let reason = String(localized: "The microphone stopped (\(error.localizedDescription)). The recording keeps the audio captured before that.")
                micDegradedReason = reason
                failure = reason
            }
        }
        if systemAudioActive, let writer = systemWriter, let filter = systemFilter, let config = systemConfig {
            // Pad first, then open: buffers that arrive while the stream opens are dropped (isPaused is
            // still set), so the file never gets audio ahead of its silence.
            padSystemFile(writer, upTo: Date())
            do {
                let newStream = try await openSystemStream(filter: filter, config: config)
                guard systemWriter === writer, isPaused else {  // stopped while resuming
                    try? await newStream.stopCapture()
                    return .notPaused
                }
                padSystemFile(writer, upTo: Date())
                lastSystemBufferAt = Date()
                stream = newStream
            } catch {
                log.error("System audio could not start again after a pause: \(error.localizedDescription, privacy: .public)")
                let reason = describeSystemAudioFailure(error)
                degradeSystemAudio(reason: reason)
                failure = failure ?? reason
            }
        }
        guard isPaused else { return .notPaused }
        lastNonSilentAt = Date()
        micPeak.reset()
        systemPeak.reset()
        isPaused = false
        log.notice("[recording-pause] resumed failed=\(failure != nil, privacy: .public)")
        if let failure { return .failed(reason: failure) }
        return .resumed
    }

    /// The 6 h cap counts a pause: a recording paused past it ends, like one that recorded that long.
    nonisolated static func pausedPastCap(paused: Bool, startedAt: Date, now: Date, cap: TimeInterval = RecordingLimits.capSeconds) -> Bool {
        paused && now.timeIntervalSince(startedAt) >= cap
    }

    // MARK: Level

    /// Starts reading the level about 8 times a second.
    func startLevelSampling() {
        guard levelTimer == nil else { return }
        micLevelPeak.isEnabled = true
        systemLevelPeak.isEnabled = true
        levelTimer = Timer.scheduledTimer(withTimeInterval: Self.levelInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.sampleLevel() }
        }
    }

    func stopLevelSampling() {
        levelTimer?.invalidate()
        levelTimer = nil
        micLevelPeak.isEnabled = false
        systemLevelPeak.isEnabled = false
        micLevelPeak.reset()
        systemLevelPeak.reset()
        if inputLevel != 0 { inputLevel = 0 }
    }

    nonisolated static let levelInterval: TimeInterval = 0.125

    private func sampleLevel() {
        let peak = max(micLevelPeak.take(), systemLevelPeak.take())
        let next: Float = (isPaused || isResuming) ? 0 : WLevelMath.level(fromPeak: peak)
        // Publish only a visible change, so the views are not redrawn for noise.
        if abs(next - inputLevel) >= 0.02 || (next == 0) != (inputLevel == 0) { inputLevel = next }
    }

    // MARK: System audio (ScreenCaptureKit)

    private func startSystemAudio(timestamp: String, epoch: Int) async {
        // Hoisted out of the `do` so the success path and the `catch` cleanup reference
        // the exact same URL binding rather than each recomputing one.
        let fileURL = SpikePaths.audioFile(name: "system-audio-\(timestamp).caf")
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            // Stale: nothing opened yet, nothing to undo.
            guard epochs.owns(epoch) else { return }
            guard let display = content.displays.first else {
                degradedReason = String(localized: "No display available for system audio capture.")
                return
            }

            let filter = SCContentFilter(display: display, excludingApplications: [], exceptingWindows: [])
            let config = SCStreamConfiguration()
            config.capturesAudio = true
            config.sampleRate = 48_000
            config.channelCount = 2
            config.excludesCurrentProcessAudio = true
            // SCStream requires a video frame size even when only the audio output is consumed.
            config.width = 2
            config.height = 2
            config.minimumFrameInterval = CMTime(value: 1, timescale: 1)

            let writer = RecordingWriter(file: try RecordingWriter.makeFile(at: fileURL))
            guard let converter = Self.makeConverter(from: Self.systemStreamFormat, to: writer.file.processingFormat) else {
                throw SystemAudioError.unconvertible
            }
            // Publish the URL as soon as the file exists on disk, before the stream is set
            // up or started below (`addStreamOutput`/`startCapture` includes a real async
            // gap during which SCStream can already be delivering buffers to disk). The
            // orphan scanner's live-write safety net keys off this published property — if
            // we waited until after `startCapture()` to assign it, there'd be a window where
            // bytes are landing on disk but the scanner sees no live-write protection for
            // this file and could treat it as orphaned mid-recording.
            systemAudioFileURL = fileURL

            systemFilter = filter
            systemConfig = config
            systemReopens = 0
            let newStream = try await openSystemStream(filter: filter, config: config)
            // Stale: close this start's own stream and drop its own empty file. Its writer was never
            // published, so no buffer reached it; a newer start's state is not touched.
            guard epochs.owns(epoch) else {
                // Detach its outputs first, so no late buffer from it reaches a newer start's writer.
                try? newStream.removeStreamOutput(self, type: .audio)
                try? newStream.removeStreamOutput(self, type: .screen)
                try? await newStream.stopCapture()
                try? FileManager.default.removeItem(at: fileURL)
                if systemAudioFileURL == fileURL { systemAudioFileURL = nil }
                return
            }

            setSystemOutput(writer, converter)
            stream = newStream
            systemAudioActive = true
            systemAudioBytesWritten = 0
            systemStartedAt = Date()
            lastSystemBufferAt = Date()
            scheduleSystemAudioWatchdog(for: newStream, fileURL: fileURL)
        } catch {
            // Stale: this start's failure says nothing about the service now.
            guard epochs.owns(epoch) else {
                try? FileManager.default.removeItem(at: fileURL)
                if systemAudioFileURL == fileURL { systemAudioFileURL = nil }
                return
            }
            log.error("System audio capture unavailable: \(error.localizedDescription, privacy: .public)")
            systemAudioActive = false
            setSystemOutput(nil, nil)
            degradedReason = describeSystemAudioFailure(error)
            cleanUpEmptySystemAudioFile(at: fileURL)
        }
    }

    private func openSystemStream(filter: SCContentFilter, config: SCStreamConfiguration) async throws -> SCStream {
        let newStream = SCStream(filter: filter, configuration: config, delegate: self)
        try newStream.addStreamOutput(self, type: .audio, sampleHandlerQueue: streamOutputQueue)
        // SCStream still produces video frames internally even for this minimal
        // 2x2 config (see above) — without a registered .screen output, it logs
        // "stream output NOT found. Dropping frame" continuously. Register one
        // that immediately discards every frame (see stream(_:didOutputSampleBuffer:of:)
        // below): we never touch, hold, or persist video/screenshot data, this is
        // purely to satisfy SCStream's internal plumbing.
        try newStream.addStreamOutput(self, type: .screen, sampleHandlerQueue: streamOutputQueue)
        try await newStream.startCapture()
        return newStream
    }

    /// Reopens system audio into the same file after it stopped mid-recording (an
    /// error, or no buffers for 5 s; ScreenCaptureKit delivers buffers through silence,
    /// so a gap means the stream is gone). Once per recording: a second failure keeps
    /// the audio captured so far and degrades to mic-only, as before. Pads the gap
    /// with silence so the file stays aligned with the mic's timeline.
    private func reopenSystemAudio(reason: String, failure: String) async {
        guard systemReopens == 0, systemAudioBytesWritten > 0,
              let writer = systemWriter, let filter = systemFilter, let config = systemConfig else {
            degradeSystemAudio(reason: failure)
            return
        }
        systemReopens += 1
        log.notice("[system-reopen] reason=\(reason, privacy: .public)")
        systemAudioWatchdogTask?.cancel()
        systemAudioWatchdogTask = nil
        if let old = stream {
            stream = nil
            try? await old.stopCapture()
        }
        do {
            let newStream = try await openSystemStream(filter: filter, config: config)
            guard systemWriter === writer else {  // stopped while reopening
                try? await newStream.stopCapture()
                return
            }
            padSystemFile(writer, upTo: Date())
            lastSystemBufferAt = Date()
            stream = newStream
            systemAudioActive = true
        } catch {
            log.error("System audio reopen failed: \(error.localizedDescription, privacy: .public)")
            degradeSystemAudio(reason: describeSystemAudioFailure(error))
        }
    }

    private func degradeSystemAudio(reason: String) {
        systemAudioActive = false
        setSystemOutput(nil, nil)
        degradedReason = reason
        if let old = stream {
            stream = nil
            Task { try? await old.stopCapture() }
        }
    }

    /// The one place `systemWriter`/`systemConverter` change, so the stream's queue always sees
    /// the same pair the main actor does.
    private func setSystemOutput(_ writer: RecordingWriter?, _ converter: AVAudioConverter?) {
        systemWriter = writer
        systemConverter = converter
        systemSink.setOutput(writer: writer, converter: converter)
    }

    private func padSystemFile(_ writer: RecordingWriter, upTo now: Date) {
        // Through the sink's lock: a buffer still in flight on the stream's queue must not write
        // into the file at the same time.
        let startedAt = systemStartedAt
        let (written, total) = systemSink.withWriterLocked {
            let gap = AVAudioFramePosition(now.timeIntervalSince(startedAt) * writer.file.processingFormat.sampleRate) - writer.framesWritten
            return (writer.writeSilence(frames: gap), writer.framesWritten)
        }
        systemAudioBytesWritten = Int(total)
        log.notice("[system-reopen] padded \(written, privacy: .public) frames of silence")
    }

    #if DEBUG
    /// Live-test hook: stops the system-audio stream without telling this service, the
    /// way a stream that silently stops delivering looks from here.
    func simulateSystemAudioStall() async {
        try? await stream?.stopCapture()
    }
    #endif

    private func checkSystemHealth() {
        guard systemAudioActive, !isPaused, !isResuming, stream != nil, systemAudioBytesWritten > 0 else { return }
        if Self.streamHealth(lastBufferAt: lastSystemBufferAt, restarts: 0, now: Date()) != .ok {
            Task {
                await reopenSystemAudio(
                    reason: "no buffers for 5 s",
                    failure: String(localized: "System audio stopped arriving. The recording keeps the audio captured before that; the rest is microphone only.")
                )
            }
        }
    }

    /// FR-MTG-10: another process (Zoom local recording, Granola, a Core Audio process
    /// tap, …) can already hold the system-audio capture path. We can't always tell the
    /// exact cause apart from macOS's error surface, so any failure to start/keep the
    /// system-audio stream is treated as "unavailable, likely in use elsewhere" and we
    /// degrade to mic-only rather than losing the meeting recording entirely.
    private func describeSystemAudioFailure(_ error: Error) -> String {
        "System audio capture unavailable (\(error.localizedDescription)). Another process may already be capturing system audio. Falling back to microphone only."
    }

    /// Confirms real audio actually arrives after a stream reports itself started —
    /// SCStream can start successfully and stay "healthy" while another process silently
    /// absorbs the actual system-audio tap, delivering nothing and never throwing an
    /// error either (see docs/superpowers/specs/2026-07-25-reliable-daily-driver-design.md,
    /// part A). 5 seconds gives real audio — even silence, which still produces
    /// non-zero-length buffers — time to arrive; if nothing has by then, treat it the
    /// same as an explicit stream failure.
    private func scheduleSystemAudioWatchdog(for expectedStream: SCStream, fileURL: URL) {
        systemAudioWatchdogTask?.cancel()
        systemAudioWatchdogTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 5_000_000_000)
            guard let self, !Task.isCancelled, self.stream === expectedStream else { return }
            guard self.systemAudioBytesWritten == 0 else { return }
            self.log.error("System audio stream started but delivered no audio within 5s — treating as degraded.")
            Task { try? await expectedStream.stopCapture() }
            self.stream = nil
            self.setSystemOutput(nil, nil)
            self.systemAudioActive = false
            self.degradedReason = String(localized: "System audio started but no audio arrived after 5 seconds. Falling back to microphone only.")
            self.cleanUpEmptySystemAudioFile(at: fileURL)
        }
    }

    /// A degraded system-audio session (explicit error or the watchdog above) still
    /// leaves behind the header-only `.caf` opened before capture began — delete it so
    /// TranscriptionService never has to reason about a file that was never really
    /// recording (it also guards against this independently via
    /// AudioConversion.hasAudioContent, but there's no reason to leave the file on disk
    /// either way). Clears systemAudioFileURL too when it matches the deleted file, so
    /// AppState.stopCapture()'s mic-only fallback (audioPath = systemAudioFileURL?.path
    /// ?? micAudioFileURL?.path) doesn't point at a file that no longer exists.
    private func cleanUpEmptySystemAudioFile(at url: URL) {
        guard systemAudioBytesWritten == 0, !AudioConversion.hasAudioContent(url) else { return }
        try? FileManager.default.removeItem(at: url)
        if systemAudioFileURL == url {
            systemAudioFileURL = nil
        }
    }

    // MARK: Microphone (AVAudioEngine)

    /// Resets `micAudioFileURL` in `start()` before this runs (mirroring
    /// `systemAudioFileURL`'s existing reset): if `AVAudioFile(forWriting:)` below
    /// throws, this method returns without ever assigning a new value, and without that
    /// reset a *previous* session's stale URL would otherwise survive into this one.
    private func startMicrophone(timestamp: String) {
        let fileURL = SpikePaths.audioFile(name: "mic-audio-\(timestamp).caf")
        do {
            let writer = RecordingWriter(file: try RecordingWriter.makeFile(at: fileURL))
            micWriter = writer
            micAudioFileURL = fileURL
            micStartedAt = Date()
            micFramesWritten = 0
            lastMicBufferAt = Date()
            micStallRestarts = 0
            try installMicTap(writer: writer)
            micActive = true
        } catch {
            log.error("Microphone capture failed to start: \(error.localizedDescription, privacy: .public)")
            micActive = false
            return
        }
        // A device change (AirPods connecting or dropping, a headset unplugged) stops
        // the engine without an error; without this the mic file silently ends there.
        engineConfigObserver = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.restartMicrophone(reason: "input device changed") }
        }
    }

    /// Installs the tap and starts the engine on the input's *current* format, writing
    /// into `writer`'s file (16 kHz mono, fixed when the recording began), converting when
    /// the two differ. `AVAudioFile.write` raises an Objective-C exception on a format
    /// mismatch, so writing a different format directly would crash mid-meeting.
    private func installMicTap(writer: RecordingWriter) throws {
        let input = engine.inputNode
        let format = input.inputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw MicError.noInputDevice
        }
        let target = writer.file.processingFormat
        let converter: AVAudioConverter?
        if format == target {
            converter = nil
        } else {
            guard let made = Self.makeConverter(from: format, to: target) else { throw MicError.unconvertible(format) }
            converter = made
        }
        micInputDeviceID = AudioDeviceActivity.defaultInputDeviceID()
        log.notice("[mic-tap] device=\(self.micInputDeviceID.map(String.init) ?? "none", privacy: .public) format=\(format, privacy: .public) file=\(target, privacy: .public) converting=\(converter != nil, privacy: .public)")
        input.installTap(onBus: 0, bufferSize: 4096, format: format) { [weak self] buffer, _ in
            guard let self else { return }
            self.lastMicBufferAt = Date()
            guard let out = converter.map({ Self.downmixToRecordingFormat(buffer, converter: $0, target: target) }) ?? buffer else { return }
            // The writer is touched only here and on main while the tap is removed
            // (padMicFile), the same confinement micFramesWritten always had.
            writer.write(out)
            self.micFramesWritten = writer.framesWritten
            let (isAudible, peak) = Self.audibilityCheck(buffer)
            self.micPeak.record(peak)
            self.micLevelPeak.record(peak)
            if isAudible {
                self.lastNonSilentAt = Date()
            }
            self.logAmplitudeIfNeeded(source: "mic", peak: peak, isAudible: isAudible, lastLogAt: &self.lastMicAmplitudeLogAt)
        }
        micTapInstalled = true
        try engine.start()
    }

    private enum SystemAudioError: LocalizedError {
        case unconvertible
        var errorDescription: String? { "System audio's format can't be converted to the recording's." }
    }

    private enum MicError: LocalizedError {
        case noInputDevice
        case unconvertible(AVAudioFormat)

        var errorDescription: String? {
            switch self {
            case .noInputDevice: String(localized: "No microphone is available.")
            case .unconvertible(let format): String(localized: "The microphone's new format (\(format)) can't be converted to the recording's.")
            }
        }
    }

    /// Restarts the mic into the same file after a device change or a stall: pads the
    /// gap with silence so the file's timeline still matches the wall clock, then taps
    /// the input's current format. On failure the recording keeps what it has and the
    /// mic is marked degraded.
    private func restartMicrophone(reason: String) {
        guard micActive, !isPaused, !isResuming, let writer = micWriter else { return }
        log.notice("[mic-restart] reason=\(reason, privacy: .public)")
        if micTapInstalled {
            engine.inputNode.removeTap(onBus: 0)
            micTapInstalled = false
        }
        engine.stop()
        padMicFile(writer, upTo: Date())
        lastMicBufferAt = Date()
        do {
            try installMicTap(writer: writer)
        } catch {
            log.error("Microphone restart failed: \(error.localizedDescription, privacy: .public)")
            micActive = false
            micDegradedReason = String(localized: "The microphone stopped (\(error.localizedDescription)). The recording keeps the audio captured before that.")
        }
    }

    /// Writes silence up to where `now` falls on the mic file's timeline.
    private func padMicFile(_ writer: RecordingWriter, upTo now: Date) {
        let expected = AVAudioFramePosition(now.timeIntervalSince(micStartedAt) * writer.file.processingFormat.sampleRate)
        let written = writer.writeSilence(frames: expected - writer.framesWritten)
        micFramesWritten = writer.framesWritten
        log.notice("[mic-restart] padded \(written, privacy: .public) frames of silence")
    }

    private func checkMicHealth() {
        guard micActive, !isPaused, !isResuming else { return }
        // A device switch is expected and may happen many times a meeting, so it doesn't
        // spend the one stall restart below.
        if let current = AudioDeviceActivity.defaultInputDeviceID(), current != micInputDeviceID {
            restartMicrophone(reason: "default input device changed")
            return
        }
        switch Self.streamHealth(lastBufferAt: lastMicBufferAt, restarts: micStallRestarts, now: Date()) {
        case .ok:
            break
        case .restart:
            micStallRestarts += 1
            restartMicrophone(reason: "no buffers for 5 s")
        case .degraded:
            log.error("Microphone delivered no audio after a restart; giving up on it for this recording.")
            if micTapInstalled {
                engine.inputNode.removeTap(onBus: 0)
                micTapInstalled = false
            }
            engine.stop()
            micActive = false
            micDegradedReason = String(localized: "The microphone stopped delivering audio. The recording keeps the audio captured before that.")
        }
    }

    /// Health-timer check: the 6 h writer cap on either stream, or free disk below 2 GB.
    /// `capReached` is set on the mic tap's thread and read here on main: a benign flag
    /// read, like the frame counters always were.
    func checkRecordingLimits() {
        guard !forcedStopReported else { return }
        let reason: RecordingStop? = (systemWriter?.capReached == true || micWriter?.capReached == true
                || Self.pausedPastCap(paused: isPaused, startedAt: micWriter != nil ? micStartedAt : systemStartedAt, now: Date()))
            ? .durationCap
            : RecordingLimits.checkDisk(freeBytes: RecordingLimits.freeBytes())
        guard let reason else { return }
        forcedStopReported = true
        log.notice("[forced-stop] reason=\(String(describing: reason), privacy: .public)")
        onForcedStop?(reason)
    }

    enum StreamHealthAction: Equatable {
        case ok, restart, degraded
    }

    /// A stream that delivers no buffer for `stallSeconds` gets one restart; stalling
    /// again after that marks it degraded. Silence doesn't count as a stall (a muted mic
    /// still delivers buffers), so this can't fire just because nobody is talking.
    nonisolated static func streamHealth(
        lastBufferAt: Date,
        restarts: Int,
        now: Date,
        stallSeconds: TimeInterval = 5
    ) -> StreamHealthAction {
        guard now.timeIntervalSince(lastBufferAt) >= stallSeconds else { return .ok }
        return restarts == 0 ? .restart : .degraded
    }

    /// What ScreenCaptureKit is configured to deliver (see `startSystemAudio`).
    nonisolated static let systemStreamFormat = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2)!

    /// Both streams' converter to the file's 16 kHz mono. `downmix` mixes all channels;
    /// without it AVAudioConverter keeps only the left one.
    nonisolated static func makeConverter(from source: AVAudioFormat, to target: AVAudioFormat) -> AVAudioConverter? {
        let converter = AVAudioConverter(from: source, to: target)
        converter?.downmix = true
        return converter
    }

    nonisolated static func downmixToRecordingFormat(_ buffer: AVAudioPCMBuffer, converter: AVAudioConverter, target format: AVAudioFormat) -> AVAudioPCMBuffer? {
        let ratio = format.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up)) + 16
        guard let out = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { return nil }
        var supplied = false
        var error: NSError?
        let status = converter.convert(to: out, error: &error) { _, inputStatus in
            if supplied {
                inputStatus.pointee = .noDataNow
                return nil
            }
            supplied = true
            inputStatus.pointee = .haveData
            return buffer
        }
        return status == .error ? nil : out
    }

    /// Recalibrated 2026-08-06 against real measured data (see
    /// docs/superpowers/specs/2026-08-06-silence-threshold-recalibration-design.md): two
    /// independent live measurements on this Mac's built-in mic — a standalone probe using
    /// this same peak-detection logic, and a real production capture queried via `log show`
    /// mid-meeting — both showed ambient/quiet-room peaks of 0.044-0.075, all misclassified
    /// as audible under the previous 0.01 value (which is why the silence-based auto-end
    /// heuristic never fired). 0.1 sits comfortably above that observed ambient ceiling
    /// while staying well below real speech (0.13-0.33 observed).
    ///
    /// Trade-off, deliberately accepted: the previous 0.01 value erred toward treating
    /// ambiguous/quiet signal as audible (never silent), so it never falsely ended a real,
    /// quiet conversation, at the cost of never auto-ending despite genuine silence. This
    /// value reverses that bias — a soft-spoken participant peaking near 0.08-0.12 could now
    /// misread as silent, and after the silence window elapses (60s normally, 15s past a
    /// scheduled end time) the meeting could auto-end mid-conversation, a new failure mode
    /// that didn't exist before. Revisit if this proves too aggressive in practice.
    private static let silenceAmplitudeThreshold: Float = 0.1

    /// Throttles the diagnostic peak-amplitude log below — mic buffers arrive roughly
    /// every 85ms and system-audio ones faster still, so logging every buffer would
    /// flood Console. One line every 2s (matching MeetingEndDetector's isAudioActive
    /// window) is enough to reconstruct what the mic/system-audio channels actually saw
    /// during a real meeting's "silence."
    private var lastMicAmplitudeLogAt = Date.distantPast
    private var lastSystemAmplitudeLogAt = Date.distantPast

    /// Scans a captured buffer for its peak absolute sample value and whether it crosses
    /// `silenceAmplitudeThreshold`. Returns `(true, 0)` (fail-safe toward "still active")
    /// if the buffer's format can't be read as float PCM — an unrecognized format should
    /// never, by itself, cause a meeting to auto-end.
    nonisolated static func audibilityCheck(_ buffer: AVAudioPCMBuffer) -> (isAudible: Bool, peak: Float) {
        guard let channelData = buffer.floatChannelData else { return (true, 0) }
        let frameCount = Int(buffer.frameLength)
        guard frameCount > 0 else { return (false, 0) }
        var peak: Float = 0
        for channel in 0..<Int(buffer.format.channelCount) {
            let samples = channelData[channel]
            for i in 0..<frameCount {
                peak = max(peak, abs(samples[i]))
            }
        }
        return (peak > silenceAmplitudeThreshold, peak)
    }

    /// FR-MTG-5 diagnostics: added after live testing showed auto-end-detection never
    /// firing despite confirmed real silence and being past a meeting's scheduled end.
    /// Logs the actual peak amplitude seen so a real recalibration of
    /// `silenceAmplitudeThreshold` is based on real numbers, not another guess — this is
    /// what made the 2026-08-06 recalibration above possible. Deliberately NOT removed now
    /// that this Mac's threshold is confirmed: the same reasoning above already
    /// acknowledges this isn't necessarily correct for every microphone/environment, so
    /// this stays in place to support recalibrating again on a different device if needed.
    ///
    /// `.notice`, not `.debug` — a second live incident (2026-08-05) confirmed `.debug`
    /// messages aren't persisted to the unified log's data store, only visible while
    /// actively streaming, so a diagnostic meant to be reviewed *after* a failure needs
    /// a level the system actually keeps around by default.
    private func logAmplitudeIfNeeded(source: String, peak: Float, isAudible: Bool, lastLogAt: inout Date) {
        let now = Date()
        guard now.timeIntervalSince(lastLogAt) >= 2 else { return }
        lastLogAt = now
        log.notice("[silence-diagnostic] \(source, privacy: .public) peak=\(peak, privacy: .public) audible=\(isAudible, privacy: .public) threshold=\(Self.silenceAmplitudeThreshold, privacy: .public)")
    }
}

extension AudioCaptureService: SCStreamOutput {
    /// Runs on `streamOutputQueue`, about 48 times a second while recording. The conversion to
    /// 16 kHz mono, the file write and the peak scan all happen here, off the main thread; on the
    /// main actor they stalled the UI during every recording (perf review, 2026-11-27). Only the
    /// small bookkeeping (timestamps, counters, the level meters' peaks) hops to the main actor.
    nonisolated func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, sampleBuffer.isValid,
              let pcmBuffer = AudioSampleConverter.pcmBuffer(from: sampleBuffer) else { return }
        let outcome = systemSink.consume(pcmBuffer)
        if case .ignored = outcome { return }
        // Pre-conversion Float32 buffer: the silence check must never see the file's Int16.
        let (isAudible, peak) = Self.audibilityCheck(pcmBuffer)
        let arrivedAt = Date()
        Task { @MainActor in
            switch outcome {
            case .ignored:
                return
            case .formatMismatch(let writer, let delivered, let expected):
                // AVAudioConverter (like AVAudioFile.write(from:)) raises an Objective-C exception, not a
                // Swift error, on a buffer whose format differs from the one it was built for, so the sink
                // checks first and has already stopped writing. Degrade to mic-only here.
                guard self.systemWriter === writer else { return }
                self.log.error("System audio format mismatch: stream delivered \(delivered, privacy: .public), converter expects \(expected, privacy: .public).")
                self.setSystemOutput(nil, nil)
                self.systemAudioActive = false
                self.systemAudioWatchdogTask?.cancel()
                self.systemAudioWatchdogTask = nil
                self.degradedReason = String(localized: "System audio format mismatch — falling back to microphone only.")
                if let stream = self.stream {
                    self.stream = nil
                    Task { try? await stream.stopCapture() }
                }
                if let url = self.systemAudioFileURL {
                    self.cleanUpEmptySystemAudioFile(at: url)
                }
            case .delivered(let writer, let framesWritten):
                guard self.systemWriter === writer else { return }
                if let framesWritten { self.systemAudioBytesWritten = Int(framesWritten) }
                // A buffer arrived, so the stream is alive even if the write was capped or
                // the resampler held this one back (as the mic tap already counts it).
                self.lastSystemBufferAt = arrivedAt
                self.systemPeak.record(peak)
                self.systemLevelPeak.record(peak)
                if isAudible {
                    self.lastNonSilentAt = arrivedAt
                }
                self.logAmplitudeIfNeeded(source: "system", peak: peak, isAudible: isAudible, lastLogAt: &self.lastSystemAmplitudeLogAt)
            }
        }
    }
}

/// The system-audio stream's writer and converter, shared between the main actor (which opens,
/// pads, pauses and closes) and the stream's own serial queue (which converts and writes every
/// buffer). One lock serializes the two: a write on the queue and a pad or close on main never
/// touch the file at once. `RecordingWriter` and `AVAudioConverter` are not thread-safe, which is
/// exactly what the lock covers.
final class SystemAudioSink: @unchecked Sendable {
    enum Outcome {
        /// Paused, or no writer (stopped, degraded, not started): the buffer is dropped.
        case ignored
        /// The buffer's format is not the converter's input. The sink has stopped writing.
        case formatMismatch(writer: RecordingWriter, delivered: AVAudioFormat, expected: AVAudioFormat)
        /// The buffer went to `writer`; `framesWritten` is nil when the resampler held it back.
        case delivered(writer: RecordingWriter, framesWritten: AVAudioFramePosition?)
    }

    private let lock = NSLock()
    private var writer: RecordingWriter?
    private var converter: AVAudioConverter?
    private var paused = false

    func setOutput(writer: RecordingWriter?, converter: AVAudioConverter?) {
        lock.lock(); defer { lock.unlock() }
        self.writer = writer
        self.converter = converter
    }

    func setPaused(_ paused: Bool) {
        lock.lock(); defer { lock.unlock() }
        self.paused = paused
    }

    /// Runs `body` while no buffer can be written, for padding the file from the main actor.
    func withWriterLocked<T>(_ body: () -> T) -> T {
        lock.lock(); defer { lock.unlock() }
        return body()
    }

    func consume(_ buffer: AVAudioPCMBuffer) -> Outcome {
        lock.lock(); defer { lock.unlock() }
        guard !paused, let writer, let converter else { return .ignored }
        // The converter expects 48kHz/Float32/stereo (the file is 16 kHz mono, so compare against the
        // converter's input, never the file) while the buffer is whatever ScreenCaptureKit delivers.
        guard buffer.format == converter.inputFormat else {
            // Stop writing before anything else: the stream delivers ~48 buffers a second and every
            // one would otherwise hit the same mismatch until the main actor tears down.
            self.writer = nil
            self.converter = nil
            return .formatMismatch(writer: writer, delivered: buffer.format, expected: converter.inputFormat)
        }
        guard let out = AudioCaptureService.downmixToRecordingFormat(buffer, converter: converter, target: writer.file.processingFormat) else {
            return .delivered(writer: writer, framesWritten: nil)
        }
        writer.write(out)
        return .delivered(writer: writer, framesWritten: writer.framesWritten)
    }
}

extension AudioCaptureService: SCStreamDelegate {
    nonisolated func stream(_ stream: SCStream, didStopWithError error: Error) {
        Task { @MainActor in
            self.log.error("System audio stream stopped: \(error.localizedDescription, privacy: .public)")
            // Cancel the watchdog first: it would otherwise still fire after its 5s
            // window and overwrite this specific, actionable error with its own generic
            // "no audio arrived" message, losing the diagnostic FR-MTG-10 exists for.
            self.systemAudioWatchdogTask?.cancel()
            self.systemAudioWatchdogTask = nil
            guard self.stream === stream else { return }  // an old stream a reopen replaced
            await self.reopenSystemAudio(reason: error.localizedDescription, failure: self.describeSystemAudioFailure(error))
        }
    }
}
