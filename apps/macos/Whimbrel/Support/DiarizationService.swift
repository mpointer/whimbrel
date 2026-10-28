import Foundation

/// FR-MTG-4 follow-on: model-path management for speaker diarization, mirroring
/// `TranscriptionService.modelPath`'s exact existing pattern — see
/// docs/superpowers/specs/2026-07-28-speaker-diarization-design.md. Two models are
/// needed (segmentation + speaker-embedding), neither checked into git; see
/// core/diarize/README.md for exactly what to fetch.
enum DiarizationService {
    private static let segmentationModelPathDefaultsKey = "com.mpointer.kestrel.diarizationSegmentationModelPath"
    private static let legacySegmentationModelPathDefaultsKey = "littlebird.diarizationSegmentationModelPath"
    private static let embeddingModelPathDefaultsKey = "com.mpointer.kestrel.diarizationEmbeddingModelPath"
    private static let legacyEmbeddingModelPathDefaultsKey = "littlebird.diarizationEmbeddingModelPath"

    static var segmentationModelPath: URL {
        get {
            ModelPathDefaultsMigration.resolve(
                newKey: segmentationModelPathDefaultsKey,
                legacyKey: legacySegmentationModelPathDefaultsKey,
                defaultPath: SpikePaths.root.appendingPathComponent("models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx")
            )
        }
        set {
            UserDefaults.standard.set(newValue.path, forKey: segmentationModelPathDefaultsKey)
        }
    }

    static var embeddingModelPath: URL {
        get {
            ModelPathDefaultsMigration.resolve(
                newKey: embeddingModelPathDefaultsKey,
                legacyKey: legacyEmbeddingModelPathDefaultsKey,
                defaultPath: SpikePaths.root.appendingPathComponent("models/wespeaker_en_voxceleb_resnet34_LM.onnx")
            )
        }
        set {
            UserDefaults.standard.set(newValue.path, forKey: embeddingModelPathDefaultsKey)
        }
    }

    /// Both model files must exist for diarization to be attempted at all — if either
    /// is missing, `TranscriptionService` skips diarization entirely and falls back to
    /// the plain `"Others"` label, exactly as if diarization had failed.
    static var modelsAvailable: Bool {
        FileManager.default.fileExists(atPath: segmentationModelPath.path)
            && FileManager.default.fileExists(atPath: embeddingModelPath.path)
    }
}
