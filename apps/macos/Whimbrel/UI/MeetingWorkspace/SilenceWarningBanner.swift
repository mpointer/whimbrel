import SwiftUI

/// Inline "Whimbrel isn't hearing anything" banner on the live meeting pane; shows while
/// `AppState.silenceWarning` is set and clears when sound returns.
struct SilenceWarningBanner: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        if let message = appState.silenceWarning {
            HStack(alignment: .top, spacing: Spacing.sm) {
                Image(systemName: "waveform.slash").foregroundStyle(Color.orange)
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    Text("Whimbrel isn't hearing anything").font(.whimbrelBody(.callout).bold())
                    Text(message).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                }
                Spacer(minLength: 0)
            }
            .padding(Spacing.sm)
            .background(Color.orange.opacity(0.12))
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("silence-warning-banner")
        }
    }
}
