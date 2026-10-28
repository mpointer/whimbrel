import SwiftUI

struct TranscriptionSettingsPane: View {
    @State private var modelPath: URL = TranscriptionService.modelPath
    @State private var statusMessage: String?
    @State private var segmentationModelPath: URL = DiarizationService.segmentationModelPath
    @State private var embeddingModelPath: URL = DiarizationService.embeddingModelPath
    @State private var diarizationStatusMessage: String?

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Transcription").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Whisper.cpp GGML model used to transcribe captured meeting audio.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

            LabeledContent("Model file") {
                Text(modelPath.path)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(FileManager.default.fileExists(atPath: modelPath.path) ? Color.whimbrelTextPrimary : Color.whimbrelError)
                    .textSelection(.enabled)
            }

            Button("Choose Model File…") { chooseModel() }
                .buttonStyle(.whimbrelSecondary)

            if let statusMessage {
                Text(statusMessage)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }

            Divider().overlay(Color.whimbrelBorder)

            Text("Speaker Diarization").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("Real per-speaker labels for the system-audio channel. Whimbrel can fetch these two models automatically via the first-run \"Set Up Models\" prompt, or see core/diarize/README.md to fetch them manually. If either is missing, transcripts fall back to a single \"Others\" label, exactly as before this feature existed.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

            LabeledContent("Segmentation model") {
                Text(segmentationModelPath.path)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(FileManager.default.fileExists(atPath: segmentationModelPath.path) ? Color.whimbrelTextPrimary : Color.whimbrelError)
                    .textSelection(.enabled)
            }
            Button("Choose Segmentation Model…") { chooseSegmentationModel() }
                .buttonStyle(.whimbrelSecondary)

            LabeledContent("Speaker-embedding model") {
                Text(embeddingModelPath.path)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(FileManager.default.fileExists(atPath: embeddingModelPath.path) ? Color.whimbrelTextPrimary : Color.whimbrelError)
                    .textSelection(.enabled)
            }
            Button("Choose Speaker-Embedding Model…") { chooseEmbeddingModel() }
                .buttonStyle(.whimbrelSecondary)

            if let diarizationStatusMessage {
                Text(diarizationStatusMessage)
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    private func chooseModel() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = "Choose a GGML whisper model (.bin)"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        TranscriptionService.modelPath = url
        modelPath = url
        statusMessage = "Model set to \(url.lastPathComponent)."
    }

    private func chooseSegmentationModel() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = "Choose the pyannote segmentation ONNX model"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        DiarizationService.segmentationModelPath = url
        segmentationModelPath = url
        diarizationStatusMessage = "Segmentation model set to \(url.lastPathComponent)."
    }

    private func chooseEmbeddingModel() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = "Choose the speaker-embedding ONNX model"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        DiarizationService.embeddingModelPath = url
        embeddingModelPath = url
        diarizationStatusMessage = "Speaker-embedding model set to \(url.lastPathComponent)."
    }
}
