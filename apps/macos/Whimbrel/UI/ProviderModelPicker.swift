import SwiftUI

/// Reusable provider+model picker — extracted from `TimelineView`'s original Ask/Digest
/// picker so other screens (starting with `MeetingDetailView`'s regenerate control) can
/// reuse the identical behavior (live-fetched hosted model lists, Ollama free-text
/// fallback when its local server isn't reachable, missing-API-key hints) without
/// duplicating ~55 lines of markup. Fetches its own model lists on appear and whenever
/// the provider selection changes — the caller only owns the initial (default) selection
/// and reads the resulting choice back out through its own `selectedProvider`/
/// `ollamaModel`/`selectedHostedModel` bindings, since it's always constructed inline
/// rather than held as an instance callers could read a property off of.
struct ProviderModelPicker: View {
    @EnvironmentObject var appState: AppState
    @Binding var selectedProvider: RoutingSettings.Provider
    @Binding var ollamaModel: String
    @Binding var selectedHostedModel: String

    /// Live-fetched (`HostedModelsService`, via `RoutingSettings`) rather than
    /// hardcoded — model lineups change too often to pin in source. Ollama falls back
    /// to free-text entry if its local server isn't reachable (see the UI branch
    /// below); Anthropic/OpenAI have no such fallback since a made-up hosted model id
    /// just fails outright.
    private var hostedModelOptions: [String] {
        switch selectedProvider {
        case .ollama: return appState.routing.ollamaModels
        case .anthropic: return appState.routing.anthropicModels
        case .openai: return appState.routing.openaiModels
        case .google: return appState.routing.googleModels
        }
    }

    private var isLoadingHostedModels: Bool {
        switch selectedProvider {
        case .ollama: return appState.routing.isLoadingOllamaModels
        case .anthropic: return appState.routing.isLoadingAnthropicModels
        case .openai: return appState.routing.isLoadingOpenAIModels
        case .google: return appState.routing.isLoadingGoogleModels
        }
    }

    private var hostedModelsError: String? {
        switch selectedProvider {
        case .ollama: return appState.routing.ollamaModelsError
        case .anthropic: return appState.routing.anthropicModelsError
        case .openai: return appState.routing.openaiModelsError
        case .google: return appState.routing.googleModelsError
        }
    }

    private func clampSelectedModelIfNeeded() {
        guard !hostedModelOptions.isEmpty else { return }
        switch selectedProvider {
        case .ollama:
            if !hostedModelOptions.contains(ollamaModel) { ollamaModel = hostedModelOptions[0] }
        case .anthropic, .openai, .google:
            if !hostedModelOptions.contains(selectedHostedModel) { selectedHostedModel = hostedModelOptions[0] }
        }
    }

    /// Fetches the selected provider's model list if it hasn't been loaded yet this
    /// session. Ollama needs no key and is attempted unconditionally; Anthropic/OpenAI/
    /// Google only if a key is configured.
    private func refreshHostedModelsIfNeeded() {
        Task {
            switch selectedProvider {
            case .ollama where appState.routing.ollamaModels.isEmpty:
                await appState.routing.refreshOllamaModels()
                clampSelectedModelIfNeeded()
            case .anthropic where appState.routing.anthropicModels.isEmpty && appState.routing.hasAnthropicKey:
                await appState.routing.refreshAnthropicModels()
                clampSelectedModelIfNeeded()
            case .openai where appState.routing.openaiModels.isEmpty && appState.routing.hasOpenAIKey:
                await appState.routing.refreshOpenAIModels()
                clampSelectedModelIfNeeded()
            case .google where appState.routing.googleModels.isEmpty && appState.routing.hasGoogleKey:
                await appState.routing.refreshGoogleModels()
                clampSelectedModelIfNeeded()
            default:
                break
            }
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack {
                Text("Provider:").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                Picker("", selection: $selectedProvider) {
                    ForEach(RoutingSettings.Provider.allCases) { Text($0.displayName).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(width: 280)
                .onChange(of: selectedProvider) { _, _ in
                    clampSelectedModelIfNeeded()
                    refreshHostedModelsIfNeeded()
                }

                if isLoadingHostedModels {
                    ProgressView().controlSize(.small).frame(width: 160)
                } else if selectedProvider == .ollama && hostedModelOptions.isEmpty {
                    // Ollama has no fixed model catalog to fall back to (unlike
                    // Anthropic/OpenAI) — if its local server can't be reached,
                    // free-text entry is the only way to specify a model at all.
                    TextField("Ollama model", text: $ollamaModel)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 160)
                } else {
                    Picker("", selection: selectedProvider == .ollama ? $ollamaModel : $selectedHostedModel) {
                        ForEach(hostedModelOptions, id: \.self) { Text($0).tag($0) }
                    }
                    .labelsHidden()
                    .frame(width: 160)
                    .disabled(hostedModelOptions.isEmpty)
                }
            }
            if selectedProvider == .anthropic && !appState.routing.hasAnthropicKey {
                Text("No Anthropic API key configured — add one in Settings > AI & Routing.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelWarning)
            } else if selectedProvider == .openai && !appState.routing.hasOpenAIKey {
                Text("No OpenAI API key configured — add one in Settings > AI & Routing.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelWarning)
            } else if selectedProvider == .google && !appState.routing.hasGoogleKey {
                Text("No Google API key configured — add one in Settings > AI & Routing.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelWarning)
            } else if let hostedModelsError, selectedProvider == .ollama {
                Text("Couldn't reach Ollama: \(hostedModelsError) — enter a model name manually.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else if let hostedModelsError {
                Text("Couldn't load models: \(hostedModelsError)")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelError)
            } else if selectedProvider != .ollama && hostedModelOptions.isEmpty && !isLoadingHostedModels {
                Text("No models loaded yet.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
        .font(.whimbrelBody(.caption))
        .onAppear {
            clampSelectedModelIfNeeded()
            refreshHostedModelsIfNeeded()
            // Prefetch Ollama's list even when it isn't the initially-selected provider,
            // so switching to it later in this session doesn't need a fresh round trip.
            if selectedProvider != .ollama && appState.routing.ollamaModels.isEmpty {
                Task { await appState.routing.refreshOllamaModels() }
            }
        }
    }
}
