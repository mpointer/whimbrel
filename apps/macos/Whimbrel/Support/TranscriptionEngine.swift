import AVFoundation
import Foundation
import OSLog
import Speech

/// The Settings picker, device-local. Existing installs start on Fast, so nothing changes until
/// they accept the offer. See docs/superpowers/specs/2026-10-31-better-transcripts-design.md.
enum TranscriptionQuality: String, CaseIterable, Identifiable, Encodable {
    case automatic, better, fast

    static let key = "transcription.quality"
    static let offerDismissedKey = "transcription.offerDismissed"
    /// large-v3-turbo's estimated realtime speed here, once `EngineBenchmark` has run.
    static let turboSpeedKey = "transcription.turboSpeed"

    var id: String { rawValue }

    static func current(defaults: UserDefaults = .standard) -> TranscriptionQuality {
        defaults.string(forKey: key).flatMap(TranscriptionQuality.init(rawValue:)) ?? .automatic
    }

    /// Run at launch, before setup can download anything: a model already on disk means an existing install.
    static func migrate(defaults: UserDefaults = .standard, modelExists: Bool) {
        guard defaults.string(forKey: key) == nil else { return }
        defaults.set((modelExists ? TranscriptionQuality.fast : .automatic).rawValue, forKey: key)
    }

    /// The one-time card: an existing install on a Mac with `SpeechAnalyzer`, or any install
    /// fast enough for large-v3-turbo that doesn't have it.
    static func shouldOffer(dismissed: Bool, quality: TranscriptionQuality, customModel: Bool, speechAnalyzer: Bool, offerTurbo: Bool) -> Bool {
        !dismissed && !customModel && ((quality == .fast && speechAnalyzer) || offerTurbo)
    }
}

/// One way of turning audio into text. Called off the main thread; every call may block.
protocol TranscriptionEngine: AnyObject {
    /// False when the engine has nothing to run with, like a model file not on disk.
    var isReady: Bool { get }
    /// This engine can't detect a language, so a caller with none has to find it first.
    var needsLanguage: Bool { get }
    /// A 16 kHz mono WAV. A nil `language` detects it. Returns the language used.
    func transcribe(wav: URL, language: String?) -> (segments: [CoreTranscriptSegment], language: String?)?
    func prepareLive(detectLanguage: Bool) -> Bool
    /// One live chunk: segments JSON with timestamps offset by `startOffsetCs`.
    func transcribe(samples: [Float], startOffsetCs: Int64, threads: Int32) -> String?
}

final class WhisperEngine: TranscriptionEngine {
    private let model: URL
    private var live: CoreLiveTranscriber?

    init(model: URL) { self.model = model }

    var isReady: Bool { FileManager.default.fileExists(atPath: model.path) }
    var needsLanguage: Bool { false }

    func transcribe(wav: URL, language: String?) -> (segments: [CoreTranscriptSegment], language: String?)? {
        // The small default is English-only and takes no language.
        if model == TranscriptionService.modelPath, language == "en" {
            return CoreMemoryStore.transcribeWav(modelPath: model, wavPath: wav).map { ($0, "en") }
        }
        return CoreMemoryStore.transcribeWavLang(modelPath: model, wavPath: wav, language: language)
    }

    func prepareLive(detectLanguage: Bool) -> Bool {
        live = CoreLiveTranscriber(modelPath: model.path)
        if detectLanguage { live?.setLanguage(nil) }
        return live != nil
    }

    func transcribe(samples: [Float], startOffsetCs: Int64, threads: Int32) -> String? {
        live?.transcribe(samples: samples, startOffsetCs: startOffsetCs, threads: threads)
    }
}

/// `primary`, then `fallback` when it fails, so a meeting never loses its transcript to a
/// missing language asset or an unsupported language.
final class FallbackEngine: TranscriptionEngine {
    private let primary: TranscriptionEngine
    private let fallback: TranscriptionEngine
    private var liveOnFallback = false
    private var detectLanguage = false

    init(primary: TranscriptionEngine, fallback: TranscriptionEngine) {
        self.primary = primary
        self.fallback = fallback
    }

    var isReady: Bool { primary.isReady || fallback.isReady }
    var needsLanguage: Bool { primary.needsLanguage }

    func transcribe(wav: URL, language: String?) -> (segments: [CoreTranscriptSegment], language: String?)? {
        primary.transcribe(wav: wav, language: language) ?? fallback.transcribe(wav: wav, language: language)
    }

    func prepareLive(detectLanguage: Bool) -> Bool {
        self.detectLanguage = detectLanguage
        if primary.prepareLive(detectLanguage: detectLanguage) { return true }
        liveOnFallback = true
        return fallback.prepareLive(detectLanguage: detectLanguage)
    }

    func transcribe(samples: [Float], startOffsetCs: Int64, threads: Int32) -> String? {
        if !liveOnFallback, let json = primary.transcribe(samples: samples, startOffsetCs: startOffsetCs, threads: threads) { return json }
        if !liveOnFallback {
            liveOnFallback = true
            guard fallback.prepareLive(detectLanguage: detectLanguage) else { return nil }
        }
        return fallback.transcribe(samples: samples, startOffsetCs: startOffsetCs, threads: threads)
    }
}

/// Apple's on-device `SpeechTranscriber`, never the server-capable `SFSpeechRecognizer`.
final class SpeechAnalyzerEngine: TranscriptionEngine {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "transcription")

    struct Span: Equatable {
        let text: String
        let start: Double
        let end: Double

        /// A result carries one range per word: the segment spans all of them.
        init?(text: String, ranges: [(start: Double, end: Double)]) {
            guard let start = ranges.map(\.start).min(), let end = ranges.map(\.end).max() else { return nil }
            self.text = text
            self.start = start
            self.end = end
        }
    }

    /// The live transcript's language, English unless told otherwise.
    private let liveLanguage = "en"

    static var isAvailable: Bool {
        if #available(macOS 26, *) { return SpeechTranscriber.isAvailable }
        return false
    }

    var isReady: Bool { Self.isAvailable }
    var needsLanguage: Bool { true }

    func transcribe(wav: URL, language: String?) -> (segments: [CoreTranscriptSegment], language: String?)? {
        guard let language, let spans = Self.spans(file: wav, language: language, limit: 1800) else { return nil }
        return (Self.segments(from: spans, offsetCs: 0), language)
    }

    func prepareLive(detectLanguage: Bool) -> Bool { Self.isAvailable && !detectLanguage }

    func transcribe(samples: [Float], startOffsetCs: Int64, threads: Int32) -> String? {
        guard !samples.isEmpty else { return "[]" }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".wav")
        defer { try? FileManager.default.removeItem(at: url) }
        guard Self.write(samples, to: url), let spans = Self.spans(file: url, language: liveLanguage, limit: 60),
              let data = try? JSONEncoder().encode(Self.segments(from: spans, offsetCs: startOffsetCs)) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Installs the language model through the OS. Failing here is fine: whisper covers until it lands.
    static func installLanguageAsset() async {
        guard #available(macOS 26, *), isAvailable,
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: locale(for: "en")) else { return }
        try? await install(transcriber(locale))
    }

    static func segments(from spans: [Span], offsetCs: Int64) -> [CoreTranscriptSegment] {
        spans.compactMap { span in
            let text = span.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { return nil }
            return CoreTranscriptSegment(
                startCs: offsetCs + Int64((span.start * 100).rounded()),
                endCs: offsetCs + Int64((span.end * 100).rounded()),
                text: text
            )
        }
    }

    static func locale(for language: String, uiTag: String = AppLanguage.uiTag()) -> Locale {
        Locale(identifier: uiTag.hasPrefix(language) ? uiTag : language)
    }

    private static func write(_ samples: [Float], to url: URL) -> Bool {
        guard let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false),
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count)),
              let file = try? AVAudioFile(forWriting: url, settings: format.settings) else { return false }
        buffer.frameLength = buffer.frameCapacity
        samples.withUnsafeBufferPointer { buffer.floatChannelData![0].update(from: $0.baseAddress!, count: samples.count) }
        return (try? file.write(from: buffer)) != nil
    }

    /// Blocks, so never call it on the main thread. A stuck run gives up after `limit`, and
    /// the caller falls back to whisper.
    private static func spans(file: URL, language: String, limit: TimeInterval) -> [Span]? {
        guard #available(macOS 26, *), isAvailable else { return nil }
        dispatchPrecondition(condition: .notOnQueue(.main))
        let done = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var result: [Span]?
        let work = Task.detached {
            do { result = try await run(file: file, language: language) } catch {
                log.error("SpeechAnalyzer failed: \(String(describing: error), privacy: .public)")
            }
            done.signal()
        }
        guard done.wait(timeout: .now() + limit) == .success else {
            work.cancel()
            return nil
        }
        return result
    }

    @available(macOS 26, *)
    private static func transcriber(_ locale: Locale) -> SpeechTranscriber {
        SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [], attributeOptions: [.audioTimeRange])
    }

    @available(macOS 26, *)
    private static func install(_ transcriber: SpeechTranscriber) async throws {
        try await AssetInventory.assetInstallationRequest(supporting: [transcriber])?.downloadAndInstall()
    }

    @available(macOS 26, *)
    private static func run(file: URL, language: String) async throws -> [Span]? {
        guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: locale(for: language)) else { return nil }
        let transcriber = transcriber(locale)
        try await install(transcriber)
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        let reader = Task { () throws -> [Span] in
            var spans: [Span] = []
            for try await result in transcriber.results {
                let ranges = result.text.runs.compactMap(\.audioTimeRange).map { ($0.start.seconds, $0.end.seconds) }
                if let span = Span(text: String(result.text.characters), ranges: ranges) { spans.append(span) }
            }
            return spans
        }
        try await analyzer.start(inputAudioFile: AVAudioFile(forReading: file), finishAfterFile: true)
        return try await reader.value
    }
}

/// Picks engines through core's chooser (`transcribe::engine`).
enum TranscriptionEngines {
    static func inputs(defaults: UserDefaults = .standard, detecting: Bool, speechAnalyzer: Bool = SpeechAnalyzerEngine.isAvailable) -> CoreEngineInputs {
        let exists = { (url: URL) in FileManager.default.fileExists(atPath: url.path) }
        return CoreEngineInputs(
            quality: TranscriptionQuality.current(defaults: defaults),
            customModel: TranscriptionService.usesCustomModel,
            speechAnalyzer: speechAnalyzer,
            turboPresent: exists(TranscriptionService.turboModelPath),
            basePresent: exists(TranscriptionService.multilingualModelPath),
            detecting: detecting,
            turboSpeed: defaults.object(forKey: TranscriptionQuality.turboSpeedKey) as? Double
        )
    }

    static func batch(language: String?) -> TranscriptionEngine {
        engine(detecting: language == nil, language: language) { $0.batch }
    }

    static func live(detecting: Bool) -> TranscriptionEngine {
        engine(detecting: detecting, language: detecting ? nil : "en") { $0.live }
    }

    private static func engine(detecting: Bool, language: String?, pick: (CoreEngineChoice) -> CoreEngine) -> TranscriptionEngine {
        let whisper = pick(CoreMemoryStore.chooseEngine(inputs(detecting: detecting, speechAnalyzer: false)) ?? .current)
        let floor = WhisperEngine(model: model(whisper, language: language))
        guard pick(CoreMemoryStore.chooseEngine(inputs(detecting: detecting)) ?? .current) == .speechAnalyzer else { return floor }
        return FallbackEngine(primary: SpeechAnalyzerEngine(), fallback: floor)
    }

    /// The English-only default can't serve another language, so anything else uses `base`, as before.
    private static func model(_ engine: CoreEngine, language: String?) -> URL {
        switch engine {
        case .turbo: return TranscriptionService.turboModelPath
        case .base: return TranscriptionService.multilingualModelPath
        case .speechAnalyzer, .current:
            return language == "en" ? TranscriptionService.modelPath : TranscriptionService.multilingualModelPath
        }
    }
}

/// `base` listens to the start of a recording, for an engine that can't detect the language itself.
enum LanguageDetection {
    static func language(of wav: URL) -> String? {
        guard let samples = EngineBenchmark.samples(in: wav, seconds: 120),
              let live = CoreLiveTranscriber(modelPath: TranscriptionService.multilingualModelPath.path) else { return nil }
        live.setLanguage(nil)
        let threads = LiveCadence.threads(forCores: ProcessInfo.processInfo.activeProcessorCount)
        guard live.transcribe(samples: samples, startOffsetCs: 0, threads: threads) != nil else { return nil }
        return live.language
    }
}

/// Times a small model on the bundled clip, once, to see whether this machine can run large-v3-turbo.
enum EngineBenchmark {
    /// A public-domain speech recording, 11 s of 16 kHz mono.
    static var clip: URL? { Bundle.main.url(forResource: "engine-benchmark", withExtension: "wav") }

    static func needed(defaults: UserDefaults = .standard, speechAnalyzer: Bool = SpeechAnalyzerEngine.isAvailable) -> Bool {
        !speechAnalyzer && !TranscriptionService.usesCustomModel && defaults.object(forKey: TranscriptionQuality.turboSpeedKey) == nil
    }

    static func runIfNeeded(defaults: UserDefaults = .standard) {
        guard needed(defaults: defaults), let clip, let samples = samples(in: clip) else { return }
        let base = TranscriptionService.multilingualModelPath
        let isBase = FileManager.default.fileExists(atPath: base.path)
        guard let live = CoreLiveTranscriber(modelPath: (isBase ? base : TranscriptionService.modelPath).path) else { return }
        let threads = LiveCadence.threads(forCores: ProcessInfo.processInfo.activeProcessorCount)
        // The first run can be slow while the GPU compiles shaders, which isn't the speed being measured.
        guard live.transcribe(samples: samples, startOffsetCs: 0, threads: threads) != nil else { return }
        let started = Date()
        guard live.transcribe(samples: samples, startOffsetCs: 0, threads: threads) != nil else { return }
        let speed = Double(samples.count) / 16_000 / max(Date().timeIntervalSince(started), 0.001)
        defaults.set(CoreMemoryStore.estimatedTurboSpeed(smallModelSpeed: speed, smallModelIsBase: isBase), forKey: TranscriptionQuality.turboSpeedKey)
    }

    /// The first `seconds` of a 16 kHz mono file, or all of it.
    static func samples(in url: URL, seconds: Double = .infinity) -> [Float]? {
        guard let file = try? AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false) else { return nil }
        let frames = AVAudioFrameCount(min(Double(file.length), seconds * 16_000))
        guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: frames),
              (try? file.read(into: buffer, frameCount: frames)) != nil, let channel = buffer.floatChannelData?[0] else { return nil }
        return Array(UnsafeBufferPointer(start: channel, count: Int(buffer.frameLength)))
    }
}
