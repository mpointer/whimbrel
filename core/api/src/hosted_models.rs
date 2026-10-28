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
}

impl HostedProvider {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "anthropic" => Some(Self::Anthropic),
            "openai" => Some(Self::OpenAi),
            "google" => Some(Self::Google),
            "ollama" => Some(Self::Ollama),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::OpenAi => "OpenAI",
            Self::Google => "Google",
            Self::Ollama => "Ollama",
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenAi => "https://api.openai.com",
            Self::Google => "https://generativelanguage.googleapis.com",
            Self::Ollama => "http://localhost:11434",
        }
    }
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
    };
    let body: serde_json::Value = match request.call() {
        Ok(response) => response
            .into_json()
            .map_err(|_| format!("{} model list response was not valid JSON.", provider.label()))?,
        Err(ureq::Error::Status(status, _)) => {
            return Err(format!("{} model list request failed (status {status}).", provider.label()))
        }
        Err(ureq::Error::Transport(_)) => {
            return Err(format!("{} model list request failed (status -1).", provider.label()))
        }
    };
    parse_models(provider, &body)
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
