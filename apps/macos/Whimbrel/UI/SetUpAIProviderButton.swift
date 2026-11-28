import SwiftUI

/// Whether Ask can run. A local model, or a hosted one with its key while local-only mode is off (the same rule
/// `MeetingAskBar` applies when it sends).
enum AskAvailability {
    static func askAvailable(provider: RoutingSettings.Provider, localOnly: Bool, hasKey: Bool) -> Bool {
        provider == .ollama || (!localOnly && hasKey)
    }
}

/// "Set up an AI provider": the button shown where an AI feature has no usable provider.
struct SetUpAIProviderButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: Spacing.sm) {
                Text("Set up an AI provider")
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Spacer()
                Image(systemName: "arrow.up.right")
                    .foregroundStyle(Color.whimbrelAccentPrimary)
            }
            .padding(.horizontal, Spacing.lg)
            .padding(.vertical, Spacing.md)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
            .overlay(RoundedRectangle(cornerRadius: Radius.card).stroke(Color.whimbrelBorder, lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Add an API key or choose a local model in Settings")
    }
}
