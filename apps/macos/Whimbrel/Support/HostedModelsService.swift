import Foundation

/// Fetches the live list of available chat models from each provider's own
/// model-listing API (core/api::hosted_models, M3.0 Task 5). The request shape,
/// status handling and per-provider filtering live in Rust; this runs the blocking
/// bridge call off the main actor. Narrow, prescribed-model workflows should use an
/// explicit per-task model override instead of relying on whatever this returns.
enum HostedModelsService {
    struct FetchError: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    static func fetchAnthropicModels(apiKey: String) async throws -> [String] {
        try await fetch(provider: "anthropic", apiKey: apiKey)
    }

    static func fetchOpenAIModels(apiKey: String) async throws -> [String] {
        try await fetch(provider: "openai", apiKey: apiKey)
    }

    static func fetchGoogleModels(apiKey: String) async throws -> [String] {
        try await fetch(provider: "google", apiKey: apiKey)
    }

    static func fetchOpenRouterModels(apiKey: String) async throws -> [String] {
        try await fetch(provider: "openrouter", apiKey: apiKey)
    }

    /// Ollama has no API key (a local, unauthenticated server). Throws if Ollama isn't
    /// running; callers fall back to free-text model entry in that case.
    static func fetchOllamaModels(baseURL: URL = URL(string: "http://localhost:11434")!) async throws -> [String] {
        try await fetch(provider: "ollama", apiKey: nil, baseURLOverride: baseURL.absoluteString)
    }

    private static func fetch(provider: String, apiKey: String?, baseURLOverride: String? = nil) async throws -> [String] {
        let result = await Task.detached(priority: .userInitiated) {
            CoreMemoryStore.fetchHostedModels(provider: provider, apiKey: apiKey, baseURLOverride: baseURLOverride)
        }.value
        return try result.get()
    }
}
