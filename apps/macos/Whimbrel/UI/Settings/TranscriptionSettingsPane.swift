import SwiftUI

struct TranscriptionSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @AppStorage(TranscriptionQuality.key) private var quality = TranscriptionQuality.automatic
    @State private var modelPath: URL = TranscriptionService.modelPath
    @State private var statusMessage: String?
    @State private var segmentationModelPath: URL = DiarizationService.segmentationModelPath
    @State private var embeddingModelPath: URL = DiarizationService.embeddingModelPath
    @State private var diarizationStatusMessage: String?
    @State private var multilingual = AppLanguage.multilingual()
    @State private var showModelSetup = false

    var body: some View {
        content.sheet(isPresented: $showModelSetup) { ModelSetupView() }
    }

    private var content: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Transcription").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Picker("Quality", selection: $quality) {
                Text("Automatic").tag(TranscriptionQuality.automatic)
                Text("Better").tag(TranscriptionQuality.better)
                Text("Fast").tag(TranscriptionQuality.fast)
            }
            .pickerStyle(.segmented)
            .onChange(of: quality) { _, chosen in
                if chosen == .better { Task { await appState.turboDownload.start() } }
            }
            Text(qualityDetail)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
            TurboDownloadLine(download: appState.turboDownload)

            Text("Whisper.cpp GGML model used to transcribe captured meeting audio. A file you choose here is used in place of the Quality setting.")
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

            Toggle("I have meetings in other languages", isOn: $multilingual)
                .onChange(of: multilingual) { _, on in
                    UserDefaults.standard.set(on, forKey: AppLanguage.multilingualKey)
                    if on, !ModelDownloadService.allModelsPresent { showModelSetup = true }
                }
            Text("Whimbrel detects each meeting's language and transcribes it as spoken, using a larger multilingual model. Off, every meeting is transcribed as English.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

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

    private var qualityDetail: String {
        switch quality {
        case .automatic: return String(localized: "The best engine this computer runs well.")
        case .better: return String(localized: "The most accurate model, a download of about 574 MB. Slower on older computers.")
        case .fast: return String(localized: "The small model. Quickest, least accurate.")
        }
    }

    private func chooseModel() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = String(localized: "Choose a GGML whisper model (.bin)")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        TranscriptionService.modelPath = url
        modelPath = url
        statusMessage = String(localized: "Model set to \(url.lastPathComponent).")
    }

    private func chooseSegmentationModel() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = String(localized: "Choose the pyannote segmentation ONNX model")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        DiarizationService.segmentationModelPath = url
        segmentationModelPath = url
        diarizationStatusMessage = String(localized: "Segmentation model set to \(url.lastPathComponent).")
    }

    private func chooseEmbeddingModel() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = []
        panel.allowsOtherFileTypes = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = String(localized: "Choose the speaker-embedding ONNX model")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        DiarizationService.embeddingModelPath = url
        embeddingModelPath = url
        diarizationStatusMessage = String(localized: "Speaker-embedding model set to \(url.lastPathComponent).")
    }
}
