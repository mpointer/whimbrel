import AVFoundation
import Speech

enum DictationState: Equatable { case idle, starting, listening, denied, unavailable }

/// Dictates into the Ask composer with on-device speech recognition only: audio never leaves the iPhone.
/// Permissions are requested on the first tap, not at launch.
@MainActor final class Dictation: ObservableObject {
    @Published var transcript = ""
    @Published private(set) var state: DictationState = .idle

    private let recognizer = SFSpeechRecognizer()
    private let engine = AVAudioEngine()
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var startToken = 0
    private let permissions: () async -> (SFSpeechRecognizerAuthorizationStatus, Bool)
    private let onDevice: () -> Bool
    private let beginHook: ((Dictation) -> Void)?

    /// The hooks exist so tests can drive start/stop without real audio or permission prompts.
    init(permissions: (() async -> (SFSpeechRecognizerAuthorizationStatus, Bool))? = nil,
         onDevice: (() -> Bool)? = nil, begin: ((Dictation) -> Void)? = nil) {
        self.permissions = permissions ?? {
            let speech = await withCheckedContinuation { c in SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0) } }
            return (speech, await AVAudioApplication.requestRecordPermission())
        }
        self.onDevice = onDevice ?? { SFSpeechRecognizer()?.supportsOnDeviceRecognition ?? false }
        self.beginHook = begin
    }

    /// The draft while dictating: what was typed before, then the words heard so far.
    nonisolated static func merge(base: String, transcript: String) -> String {
        [base, transcript].filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// After the user returns from Settings: pick up a grant without prompting again.
    func recheck() {
        guard state == .denied else { return }
        state = Self.state(speech: SFSpeechRecognizer.authorizationStatus(),
                           mic: AVAudioApplication.shared.recordPermission == .granted, onDevice: onDevice())
    }

    nonisolated static func state(speech: SFSpeechRecognizerAuthorizationStatus, mic: Bool, onDevice: Bool) -> DictationState {
        if speech == .denied || speech == .restricted || !mic { return .denied }
        return onDevice ? .idle : .unavailable
    }

    func start() {
        guard state != .listening, state != .starting else { return }
        state = .starting
        startToken += 1
        let token = startToken
        Task {
            let (speech, mic) = await permissions()
            // stop() while the prompt was up cancels this start.
            guard token == startToken, state == .starting else { return }
            let next = Self.state(speech: speech, mic: mic, onDevice: onDevice())
            guard next == .idle else { state = next; return }
            if let beginHook { beginHook(self) } else { begin() }
        }
    }

    func stop() {
        if state == .starting { startToken += 1; state = .idle; return }
        guard state == .listening else { return }
        engine.stop()
        engine.inputNode.removeTap(onBus: 0)
        request?.endAudio()
        task?.cancel()
        request = nil
        task = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        state = .idle
    }

    private nonisolated static func tap(_ req: SFSpeechAudioBufferRecognitionRequest) -> AVAudioNodeTapBlock {
        { buffer, _ in req.append(buffer) }
    }

    private func begin() {
        guard let recognizer else { state = .unavailable; return }
        let req = SFSpeechAudioBufferRecognitionRequest()
        req.requiresOnDeviceRecognition = true
        req.shouldReportPartialResults = true
        do {
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.record, mode: .measurement, options: .duckOthers)
            try session.setActive(true, options: .notifyOthersOnDeactivation)
            let input = engine.inputNode
            input.removeTap(onBus: 0)
            input.installTap(onBus: 0, bufferSize: 1024, format: input.outputFormat(forBus: 0), block: Self.tap(req))
            engine.prepare()
            try engine.start()
        } catch {
            engine.inputNode.removeTap(onBus: 0)
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            state = .unavailable
            return
        }
        request = req
        state = .listening
        task = recognizer.recognitionTask(with: req) { [weak self] result, error in
            Task { @MainActor in
                guard let self, self.request === req else { return }
                if let result { self.transcript = result.bestTranscription.formattedString }
                if error != nil || result?.isFinal == true { self.stop() }
            }
        }
    }
}
