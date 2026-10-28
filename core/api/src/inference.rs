//! The streaming inference run registry (docs/adr/0001, docs/adr/0002). Moved from
//! `core/ffi/src/inference_ffi.rs` (M3.0); the four C functions are thin wrappers.
//!
//! Takes no `memory::Store` anywhere — by construction. It owns its own process-lifetime
//! Tokio runtime and its own run registry, entirely separate from whatever the shell
//! serializes `Store` access through, so a hanging provider here cannot delay unrelated
//! store work (pinned by `start_does_not_block_unrelated_store_work_while_provider_hangs`
//! below and by its `ffi` twin).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use llm::streaming::{
    new_run_id, AnthropicStreamingClient, ChannelEventSink, GoogleStreamingClient, InferenceEvent,
    OllamaStreamingClient, OpenAIStreamingClient,
    StreamRequest, StreamingChainLink, StreamingLlmClient, StreamingRunSupervisor,
};
use llm::Provider;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::util::now_unix_ms;

struct RunHandle {
    receiver: Mutex<mpsc::Receiver<InferenceEvent>>,
    cancel: CancellationToken,
}

struct InferenceRunRegistry {
    runtime: tokio::runtime::Runtime,
    runs: Mutex<HashMap<String, RunHandle>>,
}

static REGISTRY: OnceLock<InferenceRunRegistry> = OnceLock::new();

fn registry() -> &'static InferenceRunRegistry {
    REGISTRY.get_or_init(|| InferenceRunRegistry {
        runtime: tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to start inference runtime"),
        runs: Mutex::new(HashMap::new()),
    })
}

/// What [`start`] hands back; serializes as `{"run_id":"…"}`.
#[derive(Debug, Clone, Serialize)]
pub struct StartedRun {
    pub run_id: String,
}

fn hosted_key(api_key: Option<&str>) -> Option<String> {
    api_key.filter(|k| !k.is_empty()).map(str::to_string)
}

fn with_base<C: StreamingLlmClient + 'static>(
    client: C,
    base_url_override: Option<&str>,
    with_base_url: fn(C, String) -> C,
) -> Arc<dyn StreamingLlmClient> {
    match base_url_override {
        Some(url) => Arc::new(with_base_url(client, url.to_string())),
        None => Arc::new(client),
    }
}

/// Starts a streaming inference run. `provider` is `"ollama"`, `"google"`, `"anthropic"`
/// or `"openai"`; `api_key` is required for a hosted provider and ignored for Ollama.
/// Returns `None` on invalid input (an unknown provider, or a hosted provider with no
/// key — a missing key can never be silently downgraded to a different provider, ADR
/// 0002 §7). `base_url_override` is `None` in production (Ollama defaults to
/// `http://localhost:11434`, Gemini to Google's API host); tests pass a fake server.
///
/// Returns immediately — the actual network/model work runs on this module's own Tokio
/// runtime, never on the caller's thread and never behind the store.
pub fn start(
    prompt: &str,
    model: &str,
    provider: &str,
    api_key: Option<&str>,
    base_url_override: Option<&str>,
) -> Option<StartedRun> {
    let (provider, client): (Provider, Arc<dyn StreamingLlmClient>) = match provider {
        "ollama" => (
            Provider::Ollama,
            match base_url_override {
                Some(url) => Arc::new(OllamaStreamingClient::with_base_url(url.to_string())),
                None => Arc::new(OllamaStreamingClient::new()),
            },
        ),
        "google" => {
            let client = GoogleStreamingClient::new(hosted_key(api_key)?);
            (Provider::Google, with_base(client, base_url_override, GoogleStreamingClient::with_base_url))
        }
        "anthropic" => {
            let client = AnthropicStreamingClient::new(hosted_key(api_key)?);
            (Provider::Anthropic, with_base(client, base_url_override, AnthropicStreamingClient::with_base_url))
        }
        "openai" => {
            let client = OpenAIStreamingClient::new(hosted_key(api_key)?);
            (Provider::OpenAI, with_base(client, base_url_override, OpenAIStreamingClient::with_base_url))
        }
        _ => return None,
    };

    let run_id = new_run_id();
    let (tx, rx) = mpsc::channel(256);
    let cancel = CancellationToken::new();

    let reg = registry();
    reg.runs.lock().unwrap().insert(
        run_id.clone(),
        RunHandle {
            receiver: Mutex::new(rx),
            cancel: cancel.clone(),
        },
    );

    let sink = Arc::new(ChannelEventSink::new(tx));
    let chain = vec![StreamingChainLink {
        provider,
        model: model.to_string(),
        client,
    }];
    let request = StreamRequest {
        run_id: run_id.clone(),
        model: model.to_string(),
        prompt: prompt.to_string(),
        started_at_unix_ms: now_unix_ms(),
    };

    reg.runtime.spawn(async move {
        let supervisor = StreamingRunSupervisor::new(sink);
        supervisor.drive(request, &chain, cancel, now_unix_ms).await;
    });

    Some(StartedRun { run_id })
}

/// Drains every event buffered for `run_id` since the last poll (possibly empty if
/// nothing new has arrived). Returns `None` only if `run_id` is unknown (never started,
/// or already released) so the caller can tell "no events yet" apart from "this run
/// doesn't exist".
pub fn poll(run_id: &str) -> Option<Vec<InferenceEvent>> {
    let runs = registry().runs.lock().unwrap();
    let handle = runs.get(run_id)?;
    let mut events = Vec::new();
    let mut receiver = handle.receiver.lock().unwrap();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    Some(events)
}

/// Requests cancellation of `run_id`. Returns `true` if the run was known (cancellation
/// requested; the terminal `Cancelled` event still arrives asynchronously via [`poll`] —
/// this does not block waiting for it). Returns `false` for an unknown/released run.
pub fn cancel(run_id: &str) -> bool {
    let runs = registry().runs.lock().unwrap();
    match runs.get(run_id) {
        Some(handle) => {
            handle.cancel.cancel();
            true
        }
        None => false,
    }
}

/// Frees the registry entry for `run_id`. Call once per run after observing its terminal
/// event (or after giving up on it); calling it twice, or before the run's task has
/// noticed cancellation, is safe — a no-op if already removed, and the background task
/// holds its own clone of `cancel` and finishes independently even after its channel
/// receiver is dropped.
pub fn release(run_id: &str) {
    registry().runs.lock().unwrap().remove(run_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A minimal fake Ollama server that accepts a connection and then never responds.
    fn spawn_hanging_server() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((_stream, _)) = listener.accept() {
                std::thread::sleep(Duration::from_secs(10));
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    /// Typed twin of `core/ffi`'s `inference_start_does_not_block_unrelated_store_work_
    /// while_provider_hangs` (docs/adr/0001 §1).
    #[test]
    fn start_does_not_block_unrelated_store_work_while_provider_hangs() {
        let base_url = spawn_hanging_server();
        let run = start("hello", "test-model", "ollama", None, Some(&base_url)).unwrap();

        let started = Instant::now();
        let store = memory::Store::open_in_memory().unwrap();
        assert!(store.list_routines().unwrap().is_empty());
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "unrelated store work must not be delayed by a hanging provider call; took {:?}",
            started.elapsed()
        );

        assert!(cancel(&run.run_id));
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_terminal = false;
        while Instant::now() < deadline {
            if poll(&run.run_id)
                .unwrap()
                .iter()
                .any(InferenceEvent::is_terminal)
            {
                saw_terminal = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(saw_terminal);
        release(&run.run_id);
        assert!(poll(&run.run_id).is_none());
    }

    #[test]
    fn start_refuses_google_without_a_key_and_unknown_providers() {
        assert!(start("hello", "gemini-2.5-flash", "google", None, None).is_none());
        assert!(start("hello", "gemini-2.5-flash", "google", Some(""), None).is_none());
        for provider in ["anthropic", "openai"] {
            assert!(start("hello", "m", provider, None, None).is_none(), "{provider} needs a key");
            assert!(start("hello", "m", provider, Some(""), None).is_none());
        }
        assert!(start("hello", "m", "carrier-pigeon", None, None).is_none());
    }

    #[test]
    fn unknown_run_ids_are_handled_without_panicking() {
        assert!(poll("no-such-run").is_none());
        assert!(!cancel("no-such-run"));
        release("no-such-run");
    }

    #[test]
    fn started_run_serializes_as_run_id_object() {
        let run = StartedRun {
            run_id: "abc".into(),
        };
        assert_eq!(serde_json::to_string(&run).unwrap(), r#"{"run_id":"abc"}"#);
    }
}
