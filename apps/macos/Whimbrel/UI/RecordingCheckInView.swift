import SwiftUI

/// The "Still recording?" panel's content (hosted by `RecordingCheckInPanelController`).
/// Pure presentation; all state lives on `controller`.
struct RecordingCheckInView: View {
    @ObservedObject var controller: RecordingCheckInPanelController

    var body: some View {
        if let seconds = controller.secondsRemaining {
            AccentBarRow(color: Color.whimbrelAccentPrimary) {
                HStack(alignment: .top, spacing: Spacing.md) {
                    Image(systemName: "record.circle")
                        .resizable()
                        .scaledToFit()
                        .foregroundStyle(Color.whimbrelAccentPrimary)
                        .frame(width: 32, height: 32)

                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        Text("Still recording?")
                            .font(.whimbrelBody(.callout).bold())
                            .foregroundStyle(Color.whimbrelTextPrimary)
                        Text(String(format: "Stopping in %d:%02d", seconds / 60, seconds % 60))
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                            .monospacedDigit()
                    }

                    Spacer(minLength: Spacing.sm)

                    VStack(alignment: .trailing, spacing: Spacing.xs) {
                        Button("Keep recording") { controller.keepTapped() }
                            .buttonStyle(.whimbrelPrimary)
                            .font(.whimbrelBody(.callout).weight(.medium))
                        Button("Stop now") { controller.stopTapped() }
                            .buttonStyle(.whimbrelSecondary)
                            .font(.whimbrelBody(.caption2))
                    }
                }
            }
            .whimbrelCard()
            .frame(width: 360)
            .fixedSize(horizontal: false, vertical: true)
        }
    }
}
