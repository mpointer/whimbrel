//! Live model lists from each provider's own model-listing endpoint (M3.0 part 2,
//! Task 5), moved from apps/macos `HostedModelsService.swift`. Network only, no
//! `Store` anywhere; the API key is an argument. Blocking (`ureq`): the shell calls
//! this off its main thread.

use std::time::Duration;

/// Which provider's list to fetch. `base_url` overrides the provider's default host
/// (Ollama's is `http://localhost:11434`; the others' are their public APIs) and is
/// otherwise a test-only hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedProvider {
    Anthropic,
    OpenAi,
    Google,
    Ollama,
    OpenRouter,
}

impl HostedProvider {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "anthropic" => Some(Self::Anthropic),
            "openai" => Some(Self::OpenAi),
            "google" => Some(Self::Google),
            "ollama" => Some(Self::Ollama),
            "openrouter" => Some(Self::OpenRouter),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::OpenAi => "OpenAI",
            Self::Google => "Google",
            Self::Ollama => "Ollama",
            Self::OpenRouter => "OpenRouter",
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenAi => "https://api.openai.com",
            Self::Google => "https://generativelanguage.googleapis.com",
            Self::Ollama => "http://localhost:11434",
            Self::OpenRouter => "https://openrouter.ai",
        }
    }
}

/// One OpenRouter catalogue entry: the namespaced id and its price in cents per 1K
/// tokens, as `llm`'s table counts. `:free` models come back at zero.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OpenRouterModel {
    pub id: String,
    pub in_cents_per_1k: f64,
    pub out_cents_per_1k: f64,
}

/// OpenRouter's catalogue with prices, for the model picker and the price cache the
/// shell stores in the routing config. `fetch_models(HostedProvider::OpenRouter, ..)`
/// is the ids-only view of the same list.
pub fn fetch_openrouter_catalog(api_key: &str, base_url: Option<&str>) -> Result<Vec<OpenRouterModel>, String> {
    let body = fetch_body(HostedProvider::OpenRouter, api_key, base_url).map_err(|e| e.message)?;
    parse_openrouter_catalog(&body)
}

/// OpenRouter prices are dollars per token as strings (`"0.0000005"`); this table is
/// cents per 1K, so multiply by 100 and by 1000.
pub fn parse_openrouter_catalog(body: &serde_json::Value) -> Result<Vec<OpenRouterModel>, String> {
    let malformed = || "OpenRouter model list response had an unexpected shape.".to_string();
    let per_token_to_cents_per_1k = |v: &serde_json::Value| -> Option<f64> {
        let dollars = match v {
            serde_json::Value::String(s) => s.trim().parse::<f64>().ok()?,
            serde_json::Value::Number(n) => n.as_f64()?,
            _ => return None,
        };
        Some(dollars * 100.0 * 1000.0)
    };
    let mut models: Vec<OpenRouterModel> = body["data"]
        .as_array()
        .ok_or_else(malformed)?
        .iter()
        .map(|m| {
            let id = m["id"].as_str().map(str::to_string).ok_or_else(malformed)?;
            let input = per_token_to_cents_per_1k(&m["pricing"]["prompt"]).unwrap_or(0.0);
            let output = per_token_to_cents_per_1k(&m["pricing"]["completion"]).unwrap_or(0.0);
            Ok(OpenRouterModel { id, in_cents_per_1k: input, out_cents_per_1k: output })
        })
        .collect::<Result<_, String>>()?;
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

/// OpenAI's `/v1/models` lists every model family with no capability flag; exclude
/// the obviously non-chat ones by id substring rather than pinning chat model names,
/// so new chat models show up without a code change.
const OPENAI_EXCLUDED_SUBSTRINGS: [&str; 9] = [
    "whisper", "tts", "embedding", "dall-e", "moderation", "davinci", "babbage", "ada", "curie",
];

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Fetches the sorted model ids the user can pick for `provider`. `api_key` is ignored
/// for Ollama (a local, unauthenticated server). The error string is display-ready.
pub fn fetch_models(provider: HostedProvider, api_key: &str, base_url: Option<&str>) -> Result<Vec<String>, String> {
    let body = fetch_body(provider, api_key, base_url).map_err(|e| e.message)?;
    parse_models(provider, &body)
}

/// What saving a key found out, for the key row. `Valid` carries the model list the
/// picker fills from (and, for OpenRouter, the prices the shell caches).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum KeyCheck {
    Valid { models: Vec<String>, #[serde(skip_serializing_if = "Vec::is_empty")] prices: Vec<OpenRouterModel> },
    /// 401 or 403: the provider rejected the key.
    Refused { message: String },
    /// No answer at all. The key is still worth saving; models load when online.
    Unreachable { message: String },
    /// Any other failure, with the display-ready message.
    Other { message: String },
}

/// Validates `api_key` by fetching `provider`'s model list, mapping the outcome to the
/// three things the key row says. BLOCKING, like `fetch_models`.
pub fn validate_key(provider: HostedProvider, api_key: &str, base_url: Option<&str>) -> KeyCheck {
    let name = provider.label();
    let body = match fetch_body(provider, api_key, base_url) {
        Ok(body) => body,
        Err(FetchError { status: Some(401 | 403), .. }) => {
            return KeyCheck::Refused { message: format!("{name} refused this key. Check it on {name}'s key page.") }
        }
        Err(FetchError { status: None, .. }) => {
            return KeyCheck::Unreachable {
                message: format!("Couldn't reach {name}. The key is saved; models will load when you're online."),
            }
        }
        Err(FetchError { message, .. }) => return KeyCheck::Other { message },
    };
    let prices = if provider == HostedProvider::OpenRouter { parse_openrouter_catalog(&body).unwrap_or_default() } else { Vec::new() };
    match parse_models(provider, &body) {
        Ok(models) => KeyCheck::Valid { models, prices },
        Err(message) => KeyCheck::Other { message },
    }
}

/// A failed model-list fetch: `status` is the HTTP status when the provider answered,
/// `None` when nothing did; `message` is display-ready either way.
struct FetchError {
    status: Option<u16>,
    message: String,
}

fn fetch_body(provider: HostedProvider, api_key: &str, base_url: Option<&str>) -> Result<serde_json::Value, FetchError> {
    let base = base_url.unwrap_or(provider.default_base_url()).trim_end_matches('/');
    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    let request = match provider {
        HostedProvider::Anthropic => agent
            .get(&format!("{base}/v1/models"))
            .set("x-api-key", api_key)
            .set("anthropic-version", "2023-06-01"),
        HostedProvider::OpenAi => agent
            .get(&format!("{base}/v1/models"))
            .set("Authorization", &format!("Bearer {api_key}")),
        // Google's ListModels takes its key as a query parameter (its own API shape).
        HostedProvider::Google => agent.get(&format!("{base}/v1beta/models")).query("key", api_key),
        HostedProvider::Ollama => agent.get(&format!("{base}/api/tags")),
        // The catalogue is public, but a key makes the response reflect the account
        // (and a bad key is refused), which is what validation on save needs.
        HostedProvider::OpenRouter => agent
            .get(&format!("{base}/api/v1/models"))
            .set("Authorization", &format!("Bearer {api_key}")),
    };
    match request.call() {
        Ok(response) => response.into_json().map_err(|_| FetchError {
            status: Some(200),
            message: format!("{} model list response was not valid JSON.", provider.label()),
        }),
        Err(ureq::Error::Status(status, _)) => Err(FetchError {
            status: Some(status),
            message: format!("{} model list request failed (status {status}).", provider.label()),
        }),
        Err(ureq::Error::Transport(_)) => Err(FetchError {
            status: None,
            message: format!("{} model list request failed (status -1).", provider.label()),
        }),
    }
}

/// The provider-specific decoding and filtering, separated so it is testable without
/// a server.
pub fn parse_models(provider: HostedProvider, body: &serde_json::Value) -> Result<Vec<String>, String> {
    let malformed = || format!("{} model list response had an unexpected shape.", provider.label());
    let mut ids: Vec<String> = match provider {
        HostedProvider::Anthropic => body["data"]
            .as_array()
            .ok_or_else(malformed)?
            .iter()
            .map(|m| m["id"].as_str().map(str::to_string).ok_or_else(malformed))
            .collect::<Result<_, _>>()?,
        HostedProvider::OpenAi => body["data"]
            .as_array()
            .ok_or_else(malformed)?
            .iter()
            .map(|m| m["id"].as_str().map(str::to_string).ok_or_else(malformed))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|id| !OPENAI_EXCLUDED_SUBSTRINGS.iter().any(|s| id.contains(s)))
            .collect(),
        // Names come back prefixed `models/`; strip it so the stored id matches what
        // the Google client expects. Keep only models that support generateContent.
        HostedProvider::Google => body["models"]
            .as_array()
            .ok_or_else(malformed)?
            .iter()
            .filter(|m| {
                m["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|methods| methods.iter().any(|x| x == "generateContent"))
            })
            .map(|m| {
                m["name"]
                    .as_str()
                    .map(|n| n.strip_prefix("models/").unwrap_or(n).to_string())
                    .ok_or_else(malformed)
            })
            .collect::<Result<_, _>>()?,
        HostedProvider::Ollama => body["models"]
            .as_array()
            .ok_or_else(malformed)?
            .iter()
            .map(|m| m["name"].as_str().map(str::to_string).ok_or_else(malformed))
            .collect::<Result<_, _>>()?,
        HostedProvider::OpenRouter => parse_openrouter_catalog(body)?.into_iter().map(|m| m.id).collect(),
    };
    ids.sort();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// One-shot server: records the request head, replies with `status` and `body`.
    fn serve(status: u16, body: &str) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let body = body.to_string();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
            let _ = stream.write_all(
                format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes(),
            );
        });
        (url, rx)
    }

    #[test]
    fn anthropic_sends_key_headers_and_returns_sorted_ids() {
        let (url, rx) = serve(200, r#"{"data":[{"id":"claude-b"},{"id":"claude-a"}]}"#);
        let ids = fetch_models(HostedProvider::Anthropic, "sk-test", Some(&url)).unwrap();
        assert_eq!(ids, ["claude-a", "claude-b"]);
        let head = rx.recv().unwrap();
        assert!(head.starts_with("GET /v1/models "), "{head}");
        assert!(head.contains("x-api-key: sk-test"), "{head}");
        assert!(head.contains("anthropic-version: 2023-06-01"), "{head}");
    }

    #[test]
    fn openai_sends_bearer_and_filters_non_chat_families() {
        let (url, rx) = serve(
            200,
            r#"{"data":[{"id":"gpt-5"},{"id":"whisper-1"},{"id":"text-embedding-3-small"},{"id":"dall-e-3"},{"id":"tts-1"},{"id":"gpt-4.1-mini"},{"id":"omni-moderation-latest"},{"id":"davinci-002"},{"id":"babbage-002"},{"id":"ada"},{"id":"curie"}]}"#,
        );
        let ids = fetch_models(HostedProvider::OpenAi, "sk-o", Some(&url)).unwrap();
        assert_eq!(ids, ["gpt-4.1-mini", "gpt-5"]);
        let head = rx.recv().unwrap();
        assert!(head.contains("Authorization: Bearer sk-o"), "{head}");
    }

    #[test]
    fn google_uses_query_key_strips_models_prefix_and_keeps_generate_content_only() {
        let (url, rx) = serve(
            200,
            r#"{"models":[{"name":"models/gemini-2.5-flash","supportedGenerationMethods":["generateContent","countTokens"]},{"name":"models/embedding-001","supportedGenerationMethods":["embedContent"]},{"name":"models/no-methods"},{"name":"gemini-plain","supportedGenerationMethods":["generateContent"]}]}"#,
        );
        let ids = fetch_models(HostedProvider::Google, "g-key", Some(&url)).unwrap();
        assert_eq!(ids, ["gemini-2.5-flash", "gemini-plain"]);
        let head = rx.recv().unwrap();
        assert!(head.starts_with("GET /v1beta/models?key=g-key "), "{head}");
    }

    #[test]
    fn ollama_lists_pulled_models_from_api_tags_without_a_key() {
        let (url, rx) = serve(200, r#"{"models":[{"name":"llama3:8b"},{"name":"gemma:2b"}]}"#);
        let ids = fetch_models(HostedProvider::Ollama, "", Some(&url)).unwrap();
        assert_eq!(ids, ["gemma:2b", "llama3:8b"]);
        let head = rx.recv().unwrap();
        assert!(head.starts_with("GET /api/tags "), "{head}");
        assert!(!head.to_lowercase().contains("authorization"));
    }

    #[test]
    fn openrouter_sends_bearer_and_parses_ids_with_prices_in_cents_per_1k() {
        let (url, rx) = serve(
            200,
            r#"{"data":[{"id":"vendor/big","pricing":{"prompt":"0.000003","completion":"0.000015"}},{"id":"vendor/small:free","pricing":{"prompt":"0","completion":"0"}},{"id":"vendor/odd","pricing":{"prompt":null}}]}"#,
        );
        let catalog = fetch_openrouter_catalog("sk-or-1", Some(&url)).unwrap();
        assert_eq!(catalog.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["vendor/big", "vendor/odd", "vendor/small:free"]);
        let big = &catalog[0];
        assert!((big.in_cents_per_1k - 0.3).abs() < 1e-9, "{big:?}");
        assert!((big.out_cents_per_1k - 1.5).abs() < 1e-9, "{big:?}");
        assert_eq!((catalog[2].in_cents_per_1k, catalog[2].out_cents_per_1k), (0.0, 0.0));
        assert_eq!((catalog[1].in_cents_per_1k, catalog[1].out_cents_per_1k), (0.0, 0.0), "a missing price reads as unknown, not an error");
        let head = rx.recv().unwrap();
        assert!(head.starts_with("GET /api/v1/models "), "{head}");
        assert!(head.contains("Authorization: Bearer sk-or-1"), "{head}");

        let (url, _rx) = serve(200, r#"{"data":[{"id":"b","pricing":{}},{"id":"a","pricing":{}}]}"#);
        assert_eq!(fetch_models(HostedProvider::OpenRouter, "k", Some(&url)).unwrap(), ["a", "b"]);
        assert!(parse_openrouter_catalog(&serde_json::json!({"data":[{"pricing":{}}]})).is_err());
    }

    #[test]
    fn validate_key_maps_refused_unreachable_valid_and_other() {
        let (url, _rx) = serve(401, r#"{"error":"bad key"}"#);
        assert_eq!(
            validate_key(HostedProvider::Anthropic, "bad", Some(&url)),
            KeyCheck::Refused { message: "Anthropic refused this key. Check it on Anthropic's key page.".into() }
        );
        let (url, _rx) = serve(403, "{}");
        assert!(matches!(validate_key(HostedProvider::OpenAi, "bad", Some(&url)), KeyCheck::Refused { .. }));

        assert_eq!(
            validate_key(HostedProvider::Google, "k", Some("http://127.0.0.1:1")),
            KeyCheck::Unreachable { message: "Couldn't reach Google. The key is saved; models will load when you're online.".into() }
        );

        let (url, _rx) = serve(200, r#"{"data":[{"id":"claude-b"},{"id":"claude-a"}]}"#);
        assert_eq!(
            validate_key(HostedProvider::Anthropic, "sk-ant-1", Some(&url)),
            KeyCheck::Valid { models: vec!["claude-a".into(), "claude-b".into()], prices: vec![] }
        );

        let (url, _rx) = serve(200, r#"{"data":[{"id":"vendor/m","pricing":{"prompt":"0.000001","completion":"0.000002"}}]}"#);
        match validate_key(HostedProvider::OpenRouter, "sk-or-1", Some(&url)) {
            KeyCheck::Valid { models, prices } => {
                assert_eq!(models, ["vendor/m"]);
                assert_eq!(prices[0].id, "vendor/m");
                assert!((prices[0].in_cents_per_1k - 0.1).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }

        let (url, _rx) = serve(500, "{}");
        assert!(matches!(validate_key(HostedProvider::OpenAi, "k", Some(&url)), KeyCheck::Other { .. }));
        let (url, _rx) = serve(200, "not json");
        assert!(matches!(validate_key(HostedProvider::OpenAi, "k", Some(&url)), KeyCheck::Other { .. }));

        let json = serde_json::to_value(KeyCheck::Refused { message: "m".into() }).unwrap();
        assert_eq!(json, serde_json::json!({"status": "refused", "message": "m"}));
        let json = serde_json::to_value(KeyCheck::Valid { models: vec!["a".into()], prices: vec![] }).unwrap();
        assert_eq!(json, serde_json::json!({"status": "valid", "models": ["a"]}));
    }

    #[test]
    fn non_2xx_status_is_a_display_ready_error() {
        let (url, _rx) = serve(401, r#"{"error":"bad key"}"#);
        assert_eq!(
            fetch_models(HostedProvider::Anthropic, "bad", Some(&url)).unwrap_err(),
            "Anthropic model list request failed (status 401)."
        );
    }

    #[test]
    fn unreachable_server_is_a_display_ready_error() {
        let err = fetch_models(HostedProvider::Ollama, "", Some("http://127.0.0.1:1")).unwrap_err();
        assert_eq!(err, "Ollama model list request failed (status -1).");
    }

    #[test]
    fn malformed_body_is_an_error() {
        let (url, _rx) = serve(200, r#"{"unexpected":true}"#);
        assert!(fetch_models(HostedProvider::OpenAi, "k", Some(&url)).unwrap_err().contains("unexpected shape"));
        assert!(parse_models(HostedProvider::Anthropic, &serde_json::json!({"data":[{"nope":1}]})).is_err());
    }

    #[test]
    fn provider_names_parse() {
        assert_eq!(HostedProvider::parse("openai"), Some(HostedProvider::OpenAi));
        assert_eq!(HostedProvider::parse("OpenAI"), None);
    }
}
