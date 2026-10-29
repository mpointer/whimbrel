import Foundation

/// Swift mirror of `llm::streaming::InferenceEvent` (core/llm/src/streaming.rs,
/// docs/adr/0002). Tagged on `kind`, matching serde's `#[serde(tag = "kind",
/// rename_all = "snake_case")]` encoding exactly.
enum InferenceEvent: Decodable, Equatable {
    case state(runId: String, state: RunState, atUnixMs: Int64, provider: String?, model: String?, attempt: Int)
    case textDelta(runId: String, sequence: UInt64, text: String)
    case progress(runId: String, label: String)
    case terminal(runId: String, outcome: TerminalOutcome, atUnixMs: Int64, finalText: String, usage: TokenUsage)

    /// Mirrors `llm::TokenUsage` — both fields optional (Ollama and a cancelled/failed run
    /// report none). Needed once a paid provider streams (#83), so `finalizeAsk` can log
    /// real counts instead of `-1`.
    struct TokenUsage: Decodable, Equatable {
        var inputTokens: Int64?
        var outputTokens: Int64?

        private enum CodingKeys: String, CodingKey {
            case inputTokens = "input_tokens", outputTokens = "output_tokens"
        }
    }

    enum RunState: String, Decodable, Equatable {
        case queued, connecting, streaming, cancelling
    }

    /// Mirrors `TerminalOutcome` — `#[serde(tag = "kind", rename_all = "snake_case")]`.
    enum TerminalOutcome: Decodable, Equatable {
        case completed
        case cancelled
        case failed(classification: String, message: String)

        private enum CodingKeys: String, CodingKey {
            case kind, `class`, message
        }

        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            switch try container.decode(String.self, forKey: .kind) {
            case "completed": self = .completed
            case "cancelled": self = .cancelled
            case "failed":
                let classification = try container.decode(String.self, forKey: .class)
                let message = try container.decode(String.self, forKey: .message)
                self = .failed(classification: classification, message: message)
            default:
                // Forward-compat default, matching this codebase's convention for
                // unrecognized string enums (see CoreLLMError.init(from:)).
                self = .failed(classification: "unknown", message: "unrecognized terminal outcome")
            }
        }
    }

    private enum CodingKeys: String, CodingKey {
        case kind, runId = "run_id", state, atUnixMs = "at_unix_ms", provider, model, attempt
        case sequence, text, label, outcome, finalText = "final_text", usage
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let runId = try container.decode(String.self, forKey: .runId)
        switch try container.decode(String.self, forKey: .kind) {
        case "state":
            self = .state(
                runId: runId,
                state: try container.decode(RunState.self, forKey: .state),
                atUnixMs: try container.decode(Int64.self, forKey: .atUnixMs),
                provider: try container.decodeIfPresent(String.self, forKey: .provider),
                model: try container.decodeIfPresent(String.self, forKey: .model),
                attempt: try container.decode(Int.self, forKey: .attempt)
            )
        case "text_delta":
            self = .textDelta(
                runId: runId,
                sequence: try container.decode(UInt64.self, forKey: .sequence),
                text: try container.decode(String.self, forKey: .text)
            )
        case "progress":
            self = .progress(runId: runId, label: try container.decode(String.self, forKey: .label))
        case "terminal":
            self = .terminal(
                runId: runId,
                outcome: try container.decode(TerminalOutcome.self, forKey: .outcome),
                atUnixMs: try container.decode(Int64.self, forKey: .atUnixMs),
                finalText: try container.decode(String.self, forKey: .finalText),
                usage: try container.decodeIfPresent(TokenUsage.self, forKey: .usage) ?? TokenUsage()
            )
        default:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: container, debugDescription: "unrecognized InferenceEvent kind")
        }
    }

    var runId: String {
        switch self {
        case .state(let runId, _, _, _, _, _): return runId
        case .textDelta(let runId, _, _): return runId
        case .progress(let runId, _): return runId
        case .terminal(let runId, _, _, _, _): return runId
        }
    }
}

/// Thin wrapper over `whimbrel_inference_start/poll/cancel/release` (docs/adr/0001,
/// docs/adr/0002). Deliberately free functions, not `CoreMemoryStore` methods: these
/// calls take no `StoreHandle` and must never be routed through
/// `CoreMemoryStore.accessQueue` — doing so would rebuild exactly the convoy this
/// refactor exists to remove. Call from any background `Task`, not the main actor
/// (network I/O happens off-thread already, but polling still does JSON decode work).
enum InferenceStreamClient {
    /// Providers the core has a streaming adapter for (docs/adr/0002; Gemini per #83).
    enum Provider: String {
        case ollama, google, anthropic, openai, openrouter
    }

    /// Starts a streaming run. Returns the new run's id, or nil on invalid input — which
    /// includes a hosted provider with no `apiKey`: the core refuses rather than silently
    /// starting against a different provider. `baseURLOverride` is nil in production.
    static func start(prompt: String, model: String, provider: Provider = .ollama, apiKey: String? = nil, baseURLOverride: String? = nil) -> String? {
        guard let json = withOptionalCString(apiKey, { key in
            withOptionalCString(baseURLOverride) { base in
                prompt.withCString { p in
                    model.withCString { m in
                        provider.rawValue.withCString { pr in
                            whimbrel_inference_start(p, m, pr, key, base)
                        }
                    }
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        guard let data = String(cString: json).data(using: .utf8),
              let value = try? JSONDecoder().decode([String: String].self, from: data) else { return nil }
        return value["run_id"]
    }

    /// Drains buffered events for `runId`. Empty array if nothing new; also empty (not
    /// nil) for an unknown run id — callers that need to distinguish "run finished and
    /// was released" from "nothing new yet" should track that themselves via the
    /// terminal event they already observed.
    static func poll(runId: String) -> [InferenceEvent] {
        guard let json = runId.withCString({ whimbrel_inference_poll($0) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        guard let data = String(cString: json).data(using: .utf8) else { return [] }
        return (try? JSONDecoder().decode([InferenceEvent].self, from: data)) ?? []
    }

    static func cancel(runId: String) {
        _ = runId.withCString { whimbrel_inference_cancel($0) }
    }

    private static func withOptionalCString<R>(_ value: String?, _ body: (UnsafePointer<CChar>?) -> R) -> R {
        guard let value else { return body(nil) }
        return value.withCString { body($0) }
    }

    static func release(runId: String) {
        runId.withCString { whimbrel_inference_release($0) }
    }
}
