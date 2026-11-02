import SwiftUI

/// The empty Ask screen: one button for a model that runs here, and the way to a better one.
struct LocalModelCard: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject var download: ModelBootstrap
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        if AIProvidersSettingsPane.showsOnRamp(routing: appState.routing) {
            VStack(alignment: .leading, spacing: Spacing.sm) {
                Text("Ask needs a model.")
                    .font(.whimbrelBody(.headline))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                Text("A small model runs on this Mac with no account. Answers are slower and plainer than a hosted model's. It needs about 1.9 GB.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                switch download.phase {
                case .downloading(let fraction, _):
                    ProgressView(String(localized: "Downloading the local model…"), value: fraction)
                        .font(.whimbrelBody(.caption))
                default:
                    HStack {
                        if LocalModel.canOffer {
                            Button(download.phase == .failed ? "Try Again" : "Download a Small Local Model") {
                                Task { await download.start() }
                            }
                            .buttonStyle(.whimbrelSecondary)
                        }
                        SetUpAIProviderButton { openSettings() }
                    }
                }
            }
            .padding(Spacing.md)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
        }
    }
}
