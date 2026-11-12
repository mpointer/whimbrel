import Foundation

/// Where a streamed Ask answer goes: the stream provider, plus the credential it needs. For
/// Whimbrel AI the ID token rides `apiKey` and the device key `baseURLOverride`.
struct AskStreamRoute: Equatable {
    let provider: InferenceStreamClient.Provider
    let apiKey: String?
    let baseURLOverride: String?
}

/// #74/#83: the stream route for an Ask on `provider` (every provider streams; the core has
/// NDJSON and SSE adapters). Shared by the Ask tab and the hotkey panel.
///
/// Local-only mode never streams to a hosted provider: a hosted provider then gives nil
/// (the Ask tab's blocking path's `build_chain` forces Ollama in that mode). A hosted route
/// with no key also gives nil. `replacesOllama` is `LocalModel.replacesOllama(routing)`;
/// `loadKey` reads a key from the Keychain.
func askStreamRoute(
    provider: RoutingSettings.Provider,
    modelLabel: String,
    localOnlyMode: Bool,
    replacesOllama: Bool,
    whimbrelCredentials: (idToken: String, deviceKey: String)?,
    loadKey: (KeychainService.Provider) -> String?
) -> AskStreamRoute? {
    func hosted(_ provider: InferenceStreamClient.Provider, _ keychain: KeychainService.Provider) -> AskStreamRoute? {
        guard !localOnlyMode, let key = loadKey(keychain), !key.isEmpty else { return nil }
        return AskStreamRoute(provider: provider, apiKey: key, baseURLOverride: nil)
    }
    switch provider {
    case .ollama: return AskStreamRoute(provider: replacesOllama ? .local : .ollama, apiKey: nil, baseURLOverride: nil)
    case .whimbrel:
        guard !localOnlyMode, let whimbrelCredentials else { return nil }
        return AskStreamRoute(provider: .whimbrel, apiKey: whimbrelCredentials.idToken, baseURLOverride: whimbrelCredentials.deviceKey)
    case .google: return hosted(.google, .google)
    case .anthropic: return hosted(.anthropic, .anthropic)
    case .openai: return hosted(.openai, .openai)
    // No model picked yet: the blocking path's chain leaves OpenRouter out.
    case .openrouter: return modelLabel.isEmpty ? nil : hosted(.openrouter, .openrouter)
    }
}
