import Foundation

/// FR-MTG-4 follow-on: downloads all 3 model files Whimbrel needs (whisper
/// transcription + 2 speaker-diarization models) to their existing default paths, for
/// a fresh install on a new Mac where none of them exist yet. The manifest (URLs, size
/// floors, the `.tar.bz2` unpack) and the download-verify-move sequence live in
/// core/api::models (M3.0 Task 3); this is the polling shim over its job registry,
/// shaped as one combined async operation so `ModelSetupView` reads exactly as before.
enum ModelDownloadService {
    private static let pollInterval: Duration = .milliseconds(200)

    enum DownloadError: Error, CustomStringConvertible {
        case failed(String)
        case bridgeUnavailable

        var description: String {
            switch self {
            case .failed(let message): return message
            case .bridgeUnavailable: return "Model download could not be started."
            }
        }
    }

    static var allModelsPresent: Bool {
        FileManager.default.fileExists(atPath: TranscriptionService.modelPath.path)
            && DiarizationService.modelsAvailable
    }

    /// Downloads and installs all 3 missing model files. Already-present files are
    /// skipped (so re-running after a partial failure only fetches what's still
    /// missing). `progress` is called on the main actor with an overall fraction
    /// (0...1) and a short status string suitable for direct display. Cancelling the
    /// calling `Task` cancels the download and throws `CancellationError`.
    @MainActor
    static func downloadAll(progress: @escaping (Double, String) -> Void) async throws {
        guard let jobId = CoreMemoryStore.startModelDownload(
            whisperModelPath: TranscriptionService.modelPath,
            segmentationModelPath: DiarizationService.segmentationModelPath,
            embeddingModelPath: DiarizationService.embeddingModelPath
        ) else {
            throw DownloadError.bridgeUnavailable
        }
        defer { CoreMemoryStore.releaseModelDownload(jobId: jobId) }

        var lastStatus = ""
        while true {
            if Task.isCancelled {
                CoreMemoryStore.cancelModelDownload(jobId: jobId)
                throw CancellationError()
            }
            guard let snapshot = CoreMemoryStore.pollModelDownload(jobId: jobId) else {
                throw DownloadError.bridgeUnavailable
            }
            if snapshot.status != lastStatus {
                lastStatus = snapshot.status
                progress(snapshot.fraction, snapshot.status)
            }
            if snapshot.done {
                if snapshot.cancelled { throw CancellationError() }
                if let error = snapshot.error { throw DownloadError.failed(error) }
                return
            }
            try? await Task.sleep(for: pollInterval)
        }
    }
}
