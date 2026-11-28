import SwiftUI
import WhimbrelUI

/// The model setup sheet: the same download call and the same dismissals. The system's
/// raw error text is not shown; the screen says what failed and what to do.
struct ModelSetupRedesignView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var isDownloading = false
    @State private var progress: Double = 0
    @State private var statusText = String(localized: "Whimbrel needs a few model files for transcription and speaker detection.")
    @State private var failed = false
    @State private var downloadTask: Task<Void, Never>?

    var body: some View {
        WNoticeScreenView(model: content).redesignThemed().frame(width: 640, height: 440)
    }

    private var content: WNoticeScreenModel {
        var m = WNoticeScreenModel()
        m.title = String(localized: "Set up models", comment: "Title of the screen that downloads the model files.")
        m.paragraphs = [statusText]
        m.fraction = isDownloading ? progress : nil
        m.errorLine = failed ? String(localized: "The model files did not download. Check your connection, then try again.", comment: "Shown under the model setup after a failed download.") : nil
        m.actions = [
            WScreenAction(id: "skip", title: String(localized: "Skip for now", comment: "Button that closes model setup without downloading."), isCancel: true) {
                downloadTask?.cancel()
                dismiss()
            },
            WScreenAction(
                id: "download",
                title: isDownloading ? String(localized: "Downloading…") : (failed ? String(localized: "Try again") : String(localized: "Download models", comment: "Button that starts the model download.")),
                kind: .primary, isEnabled: !isDownloading) { startDownload() },
        ]
        return m
    }

    private func startDownload() {
        isDownloading = true
        failed = false
        downloadTask = Task {
            do {
                try await ModelDownloadService.downloadAll { fraction, status in
                    progress = fraction
                    statusText = status
                }
                dismiss()
            } catch {
                isDownloading = false
                guard !Task.isCancelled, !(error is CancellationError) else { return }
                statusText = String(localized: "Setup couldn't finish.")
                failed = true
            }
        }
    }
}
