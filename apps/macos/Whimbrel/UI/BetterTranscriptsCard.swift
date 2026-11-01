import SwiftUI

/// The one offer of better transcripts. Dismissing it, or accepting, ends it for good.
struct BetterTranscriptsCard: View {
    @ObservedObject var download: ModelBootstrap
    @AppStorage(TranscriptionQuality.offerDismissedKey) private var dismissed = false
    @AppStorage(TranscriptionQuality.key) private var quality = TranscriptionQuality.automatic

    private var speechAnalyzer: Bool { SpeechAnalyzerEngine.isAvailable }

    var body: some View {
        if TranscriptionQuality.shouldOffer(
            dismissed: dismissed, quality: quality, customModel: TranscriptionService.usesCustomModel,
            speechAnalyzer: speechAnalyzer,
            offerTurbo: CoreMemoryStore.chooseEngine(TranscriptionEngines.inputs(detecting: false))?.offerTurbo ?? false
        ) {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text("Better transcripts are available.")
                    .font(.whimbrelBody(.headline))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                Text(speechAnalyzer
                    ? "Turn on Apple's speech engine. Your audio stays on this Mac."
                    : "Download a larger model, about 574 MB. Your audio stays on this Mac.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                HStack {
                    Button(speechAnalyzer ? "Turn On" : "Download") { accept() }
                        .buttonStyle(.whimbrelSecondary)
                    Button("Not Now") { dismissed = true }
                        .buttonStyle(.whimbrelSecondary)
                }
            }
            .padding(Spacing.md)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
        } else {
            TurboDownloadLine(download: download)
        }
    }

    private func accept() {
        quality = .automatic
        dismissed = true
        Task {
            if speechAnalyzer { await SpeechAnalyzerEngine.installLanguageAsset() } else { await download.start() }
        }
    }
}

struct TurboDownloadLine: View {
    @ObservedObject var download: ModelBootstrap

    var body: some View {
        switch download.phase {
        case .downloading(let fraction, _):
            ProgressView(String(localized: "Downloading the better transcription model…"), value: fraction)
                .font(.whimbrelBody(.caption))
        case .failed:
            Button("Couldn't download the better model. Try Again") { Task { await download.start() } }
                .buttonStyle(.whimbrelSecondary)
        case .idle, .ready:
            EmptyView()
        }
    }
}
