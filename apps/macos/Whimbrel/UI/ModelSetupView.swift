import SwiftUI

/// Shown from `TimelineView`'s `.onAppear` whenever any of the 3 model files is
/// missing (fresh install on a new Mac, or a prior "Skip for Now") — see
/// `ModelDownloadService`. Never a hard lock: "Skip for now" dismisses to Timeline in
/// exactly today's pre-download behavior (transcription/diarization degrade or error
/// clearly, same as always), and the sheet can reappear on a later launch if models
/// are still missing.
struct ModelSetupView: View {
    @Environment(\.dismiss) private var dismiss

    @State private var isDownloading = false
    @State private var progress: Double = 0
    @State private var statusText = "Whimbrel needs a few model files for transcription and speaker detection."
    @State private var errorText: String?
    @State private var downloadTask: Task<Void, Never>?

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Set Up Models").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)

            Text(statusText)
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)

            if isDownloading {
                ProgressView(value: progress)
                    .frame(width: 360)
            }

            if let errorText {
                Text(errorText)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
            }

            HStack {
                Button("Skip for Now") {
                    downloadTask?.cancel()
                    dismiss()
                }
                .buttonStyle(.whimbrelSecondary)

                Button(isDownloading ? "Downloading…" : "Download Models") {
                    startDownload()
                }
                .buttonStyle(.whimbrelPrimary)
                .disabled(isDownloading)
            }
        }
        .padding(Spacing.xl)
        .frame(width: 440)
    }

    private func startDownload() {
        isDownloading = true
        errorText = nil
        downloadTask = Task {
            do {
                try await ModelDownloadService.downloadAll { fraction, status in
                    progress = fraction
                    statusText = status
                }
                dismiss()
            } catch {
                isDownloading = false
                // A cancel (from "Skip for Now") can surface either as our own
                // CancellationError (if it lands between steps) or as the
                // underlying URLSession cancellation error (if it lands mid
                // download) — in either case the sheet is already being
                // dismissed, so don't paint a bogus failure message over it.
                guard !Task.isCancelled, !(error is CancellationError) else { return }
                statusText = "Setup couldn't finish."
                errorText = String(describing: error)
            }
        }
    }
}
