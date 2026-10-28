import SwiftUI

/// Today's Ask launcher: one rounded field that hands its text to Library's Ask as a new
/// conversation. It never answers inline.
struct TodayComposer: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openSettings) private var openSettings
    var placeholder = SkinDefaults.whimbrel.voice.text(.askPlaceholder, name: "")
    let onSubmit: (String) -> Void
    @State private var text = ""

    private var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// Library's Ask can run on a local model, or on a hosted one with its key while local-only
    /// mode is off (the same rule `MeetingAskBar` applies when it sends).
    static func askAvailable(provider: RoutingSettings.Provider, localOnly: Bool, hasKey: Bool) -> Bool {
        provider == .ollama || (!localOnly && hasKey)
    }

    private var available: Bool {
        let routing = appState.routing
        let hasKey: Bool
        switch routing.defaultSelection().provider {
        case .ollama: hasKey = true
        case .anthropic: hasKey = routing.hasAnthropicKey
        case .openai: hasKey = routing.hasOpenAIKey
        case .google: hasKey = routing.hasGoogleKey
        }
        return Self.askAvailable(provider: routing.defaultSelection().provider, localOnly: routing.localOnlyMode, hasKey: hasKey)
    }

    private var modelLabel: String {
        let selection = appState.routing.defaultSelection()
        return selection.provider == .ollama ? selection.ollamaModel : selection.hostedModel
    }

    var body: some View {
        if available { composer } else { setUpPrompt }
    }

    /// With no usable provider the composer is a single button into Settings.
    private var setUpPrompt: some View { SetUpAIProviderButton { openSettings() } }

    private var composer: some View {
        HStack(spacing: Spacing.sm) {
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .onSubmit(send)
            Text(modelLabel)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .lineLimit(1)
            Button(action: send) {
                Image(skinRole: .ask)
                    .font(.system(size: 22))
                    .foregroundStyle(trimmed.isEmpty ? Color.whimbrelTextSecondary : Color.whimbrelAccentPrimary)
            }
            .buttonStyle(.plain)
            .disabled(trimmed.isEmpty)
            .help("Ask in Library")
            .accessibilityLabel("Ask")
        }
        .padding(.horizontal, Spacing.lg)
        .padding(.vertical, Spacing.md)
        .background(Color.whimbrelBackgroundCard)
        .clipShape(RoundedRectangle(cornerRadius: Radius.card))
        .overlay(RoundedRectangle(cornerRadius: Radius.card).stroke(Color.whimbrelBorder, lineWidth: 1))
    }

    private func send() {
        guard !trimmed.isEmpty else { return }
        onSubmit(trimmed)
        text = ""
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
