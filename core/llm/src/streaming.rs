//! Provider-neutral streaming inference (docs/adr/0002-streaming-inference-and-cancellation.md).
//!
//! Adapted from docs/refactor-package/snippets/provider_neutral_streaming.rs. This
//! module is additive: `LlmClient::complete()` and every existing caller (routines,
//! `whimbrel_llm_answer`, `whimbrel_llm_summarize_meeting`) are untouched. Route interactive
//! chat through `StreamingLlmClient` first (adapters: Ollama NDJSON, Gemini SSE); batch routines stay
//! on the blocking path until explicitly migrated.
//!
//! Dependency direction is unchanged: llm -> memory. No Swift type or FFI callback lives
//! here — `core/ffi` owns run registration and C-ABI event polling.

use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{is_retryable, LlmError, Provider, TokenUsage};

pub type InferenceRunId = String;
pub type ProviderEventStream = Pin<Box<dyn Stream<Item = Result<ProviderEvent, LlmError>> + Send>>;

static RUN_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A process-unique run id. Not the `uuid` crate: this is a same-process opaque token
/// (never persisted, never compared across devices), and `memory`'s own outbox module
/// already established the "no new crate for opaque unique string" precedent.
pub fn new_run_id() -> InferenceRunId {
    let n = RUN_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("run-{nanos:x}-{n:x}")
}

/// Immutable snapshot created before dispatch. Do not re-read editable state (a note, a
/// meeting) while a run is streaming — the prompt is fixed at dispatch time.
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub run_id: InferenceRunId,
    pub model: String,
    pub prompt: String,
    pub started_at_unix_ms: i64,
}

/// Provider adapters emit protocol facts, not UI states. A provider must never emit raw
/// chain-of-thought; adapters with a thinking/tool protocol map that material to a
/// policy-safe Progress label or omit it — none of this slice's adapters have one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    /// The HTTP response/stream is accepted — distinct from task creation, so UI can say
    /// "Connecting..." versus "Generating...".
    StreamOpened,
    /// Incremental display-safe output.
    TextDelta(String),
    /// Safe progress, e.g. local model warming.
    Progress { label: String },
    /// Provider terminal metadata. Emitted at most once, never after an Err.
    Finished { usage: TokenUsage },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InferenceEvent {
    State {
        run_id: InferenceRunId,
        state: InferenceRunState,
        at_unix_ms: i64,
        provider: Option<String>,
        model: Option<String>,
        attempt: u32,
    },
    TextDelta {
        run_id: InferenceRunId,
        /// Monotonically increasing per run. FFI/Swift ignore a duplicate or
        /// stale sequence, and additionally guard on run_id (see ADR 0002 §6).
        sequence: u64,
        text: String,
    },
    Progress {
        run_id: InferenceRunId,
        label: String,
    },
    Terminal {
        run_id: InferenceRunId,
        outcome: TerminalOutcome,
        at_unix_ms: i64,
        final_text: String,
        usage: TokenUsage,
    },
}

impl InferenceEvent {
    pub fn run_id(&self) -> &str {
        match self {
            InferenceEvent::State { run_id, .. }
            | InferenceEvent::TextDelta { run_id, .. }
            | InferenceEvent::Progress { run_id, .. }
            | InferenceEvent::Terminal { run_id, .. } => run_id,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, InferenceEvent::Terminal { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceRunState {
    Queued,
    Connecting,
    Streaming,
    Cancelling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalOutcome {
    Completed,
    Cancelled,
    Failed {
        class: InferenceFailureClass,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceFailureClass {
    Network,
    Authentication,
    RateLimited,
    Provider,
    Parse,
    Timeout,
}

#[async_trait]
pub trait StreamingLlmClient: Send + Sync {
    fn provider(&self) -> Provider;

    /// Starts a provider stream. The adapter must observe `cancel` while waiting for
    /// headers and while receiving chunks. Dropping the returned stream must release the
    /// underlying HTTP body/socket. The adapter owns parsing its provider's wire format
    /// into `ProviderEvent`, not the caller.
    async fn stream(
        &self,
        request: StreamRequest,
        cancel: CancellationToken,
    ) -> Result<ProviderEventStream, LlmError>;
}

/// Bounded, send-only notification surface used by `core/ffi`'s run registry. Not the
/// durable record — there is no durable `inference_run` persistence in this slice (ADR
/// 0002, deferred work: INF-109 relaunch recovery is not implemented). A bounded channel
/// prevents a pathological provider from consuming unbounded memory while the consumer is
/// temporarily busy; a full channel is treated as "no one is watching" and ends the run as
/// cancelled rather than blocking provider I/O on a slow consumer.
pub trait EventSink: Send + Sync {
    fn try_emit(&self, event: InferenceEvent) -> Result<(), LlmError>;
}

pub struct ChannelEventSink {
    tx: mpsc::Sender<InferenceEvent>,
}

impl ChannelEventSink {
    pub fn new(tx: mpsc::Sender<InferenceEvent>) -> Self {
        Self { tx }
    }
}

impl EventSink for ChannelEventSink {
    fn try_emit(&self, event: InferenceEvent) -> Result<(), LlmError> {
        self.tx
            .try_send(event)
            .map_err(|err| LlmError::Provider(format!("inference event backpressure: {err}")))
    }
}

/// One provider link supplied by the caller's routing chain. Fallback is legal only if a
/// retryable failure occurs BEFORE displayable output; once output reaches the sink,
/// silently appending another model's continuation is prohibited — the run ends
/// Failed/partial and the caller explicitly retries (docs/adr/0002 §7, INF-009).
pub struct StreamingChainLink {
    pub provider: Provider,
    pub model: String,
    pub client: Arc<dyn StreamingLlmClient>,
}

pub struct StreamingRunSupervisor {
    sink: Arc<dyn EventSink>,
}

impl StreamingRunSupervisor {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self { sink }
    }

    pub async fn drive(
        &self,
        request: StreamRequest,
        chain: &[StreamingChainLink],
        cancel: CancellationToken,
        now_unix_ms: impl Fn() -> i64,
    ) {
        let mut output = String::new();
        let mut sequence = 0_u64;

        for (attempt_index, link) in chain.iter().enumerate() {
            if cancel.is_cancelled() {
                self.emit_cancelled(&request, &output, &now_unix_ms);
                return;
            }
            let attempt = (attempt_index + 1) as u32;
            let _ = self.sink.try_emit(InferenceEvent::State {
                run_id: request.run_id.clone(),
                state: InferenceRunState::Connecting,
                at_unix_ms: now_unix_ms(),
                provider: Some(link.provider.as_str().to_owned()),
                model: Some(link.model.clone()),
                attempt,
            });

            let mut provider_request = request.clone();
            provider_request.model = link.model.clone();
            let stream_result = tokio::select! {
                _ = cancel.cancelled() => {
                    self.emit_cancelled(&request, &output, &now_unix_ms);
                    return;
                }
                result = link.client.stream(provider_request, cancel.clone()) => result,
            };

            let mut stream = match stream_result {
                Ok(stream) => stream,
                Err(error) if output.is_empty() && is_retryable(&error) => continue,
                Err(error) => {
                    self.emit_failure(&request, &output, error, &now_unix_ms);
                    return;
                }
            };
            let mut opened = false;

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        // Dropping `stream` closes the underlying HTTP response body.
                        // Adapters additionally select on `cancel` while blocked on
                        // reads, so cancellation interrupts an in-progress read rather
                        // than waiting for the next chunk.
                        drop(stream);
                        self.emit_cancelled(&request, &output, &now_unix_ms);
                        return;
                    }
                    item = stream.next() => match item {
                        Some(Ok(ProviderEvent::StreamOpened)) => {
                            opened = true;
                            let _ = self.sink.try_emit(InferenceEvent::State {
                                run_id: request.run_id.clone(),
                                state: InferenceRunState::Streaming,
                                at_unix_ms: now_unix_ms(),
                                provider: Some(link.provider.as_str().to_owned()),
                                model: Some(link.model.clone()),
                                attempt,
                            });
                        }
                        Some(Ok(ProviderEvent::TextDelta(text))) => {
                            if !opened {
                                opened = true;
                                let _ = self.sink.try_emit(InferenceEvent::State {
                                    run_id: request.run_id.clone(),
                                    state: InferenceRunState::Streaming,
                                    at_unix_ms: now_unix_ms(),
                                    provider: Some(link.provider.as_str().to_owned()),
                                    model: Some(link.model.clone()),
                                    attempt,
                                });
                            }
                            output.push_str(&text);
                            sequence += 1;
                            if self
                                .sink
                                .try_emit(InferenceEvent::TextDelta {
                                    run_id: request.run_id.clone(),
                                    sequence,
                                    text,
                                })
                                .is_err()
                            {
                                // A dead/overloaded bridge is treated as cancelled work:
                                // do not keep spending on a result no one can observe.
                                drop(stream);
                                self.emit_cancelled(&request, &output, &now_unix_ms);
                                return;
                            }
                        }
                        Some(Ok(ProviderEvent::Progress { label })) => {
                            let _ = self.sink.try_emit(InferenceEvent::Progress {
                                run_id: request.run_id.clone(),
                                label,
                            });
                        }
                        Some(Ok(ProviderEvent::Finished { usage })) => {
                            self.emit_completed(&request, &output, usage, &now_unix_ms);
                            return;
                        }
                        Some(Err(error)) if output.is_empty() && is_retryable(&error) => {
                            // A retry-before-output falls through to the next link.
                            break;
                        }
                        Some(Err(error)) => {
                            self.emit_failure(&request, &output, error, &now_unix_ms);
                            return;
                        }
                        None => {
                            self.emit_failure(
                                &request,
                                &output,
                                LlmError::Provider("provider stream ended without terminal event".into()),
                                &now_unix_ms,
                            );
                            return;
                        }
                    }
                }
            }
        }
        self.emit_failure(
            &request,
            &output,
            LlmError::Provider("all eligible providers failed before output".into()),
            &now_unix_ms,
        );
    }

    fn emit_cancelled(&self, request: &StreamRequest, output: &str, now: &impl Fn() -> i64) {
        let _ = self.sink.try_emit(InferenceEvent::Terminal {
            run_id: request.run_id.clone(),
            outcome: TerminalOutcome::Cancelled,
            at_unix_ms: now(),
            final_text: output.to_owned(),
            usage: TokenUsage::default(),
        });
    }

    fn emit_completed(
        &self,
        request: &StreamRequest,
        output: &str,
        usage: TokenUsage,
        now: &impl Fn() -> i64,
    ) {
        let _ = self.sink.try_emit(InferenceEvent::Terminal {
            run_id: request.run_id.clone(),
            outcome: TerminalOutcome::Completed,
            at_unix_ms: now(),
            final_text: output.to_owned(),
            usage,
        });
    }

    fn emit_failure(
        &self,
        request: &StreamRequest,
        output: &str,
        error: LlmError,
        now: &impl Fn() -> i64,
    ) {
        let class = match error {
            LlmError::Network(_) => InferenceFailureClass::Network,
            LlmError::Http {
                status: 401 | 403, ..
            } => InferenceFailureClass::Authentication,
            LlmError::Http { status: 429, .. } => InferenceFailureClass::RateLimited,
            LlmError::Http { .. } | LlmError::Provider(_) | LlmError::NotImplemented(_) => {
                InferenceFailureClass::Provider
            }
        };
        let _ = self.sink.try_emit(InferenceEvent::Terminal {
            run_id: request.run_id.clone(),
            outcome: TerminalOutcome::Failed {
                class,
                message: error.to_string(),
            },
            at_unix_ms: now(),
            final_text: output.to_owned(),
            usage: TokenUsage::default(),
        });
    }
}

// --- Ollama NDJSON adapter -------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OllamaStreamLine {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

pub struct OllamaStreamingClient {
    base_url: String,
    http: reqwest::Client,
}

impl OllamaStreamingClient {
    pub fn new() -> Self {
        Self {
            base_url: "http://localhost:11434".to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
        }
    }
}

impl Default for OllamaStreamingClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Ollama's default context window is small enough to silently truncate a meeting prompt
/// (about 7-10k tokens), so every request asks for a larger one.
const OLLAMA_NUM_CTX: u32 = 16_384;

fn ollama_request_body(model: &str, prompt: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": true,
        "options": { "num_ctx": OLLAMA_NUM_CTX },
    })
}

#[async_trait]
impl StreamingLlmClient for OllamaStreamingClient {
    fn provider(&self) -> Provider {
        Provider::Ollama
    }

    async fn stream(
        &self,
        request: StreamRequest,
        cancel: CancellationToken,
    ) -> Result<ProviderEventStream, LlmError> {
        let body = ollama_request_body(&request.model, &request.prompt);

        let response = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(LlmError::Network("cancelled before connecting".into()));
            }
            result = self
                .http
                .post(format!("{}/api/generate", self.base_url))
                .json(&body)
                .send() => result.map_err(|e| LlmError::Network(e.to_string()))?,
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Http { status, body });
        }

        let byte_stream = response.bytes_stream();
        let parsed = ndjson_to_provider_events(byte_stream);
        Ok(Box::pin(parsed))
    }
}

/// Turns a raw byte stream into `ProviderEvent`s by buffering across chunk boundaries and
/// splitting on `\n`. Ollama's NDJSON framing (one complete JSON object per line) does not
/// guarantee one line per network chunk, so a line may span multiple `Bytes` items.
/// Cancellation is deliberately NOT re-checked per chunk here. The supervisor's own
/// `tokio::select!` around `stream.next()` (in `StreamingRunSupervisor::drive`) already
/// races cancellation against a genuinely-blocked read — when nothing has arrived, that
/// `stream.next()` future stays `Pending` and the supervisor's cancel branch fires and
/// drops this whole generator, closing the socket. Racing the same `CancellationToken` a
/// second time *inside* the generator that `stream.next()` polls is not merely redundant:
/// once cancelled, both selects can observe their cancel branch ready in the same poll,
/// and `tokio::select!` may pick either ready branch — occasionally resolving the
/// supervisor's `stream.next()` arm to `None` (a synchronously-completed drained
/// generator) instead of its `cancel.cancelled()` arm, misreporting a clean "stream
/// ended" instead of `Cancelled`. Single ownership of the cancel decision avoids the
/// race entirely.
fn ndjson_to_provider_events(
    mut byte_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send + 'static,
) -> impl Stream<Item = Result<ProviderEvent, LlmError>> + Send {
    async_stream::stream! {
        yield Ok(ProviderEvent::StreamOpened);
        let mut buffer = String::new();
        loop {
            let chunk = byte_stream.next().await;
            let Some(chunk) = chunk else {
                // Connection closed. If the last line never arrived with a trailing
                // newline but is a complete JSON object, process it; otherwise this is
                // a legitimate "stream ended without terminal event" case the
                // supervisor already handles by emitting a Failed terminal.
                if !buffer.trim().is_empty() {
                    if let Some(event) = parse_ollama_line(&buffer) {
                        yield event;
                    }
                }
                return;
            };
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    yield Err(LlmError::Network(e.to_string()));
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].to_string();
                buffer.drain(..=pos);
                if line.trim().is_empty() {
                    continue;
                }
                match parse_ollama_line(&line) {
                    Some(event) => {
                        let is_finished = matches!(event, Ok(ProviderEvent::Finished { .. }));
                        yield event;
                        if is_finished {
                            return;
                        }
                    }
                    None => {
                        yield Err(LlmError::Provider(format!("malformed Ollama NDJSON line: {line}")));
                        return;
                    }
                }
            }
        }
    }
}

fn parse_ollama_line(line: &str) -> Option<Result<ProviderEvent, LlmError>> {
    let parsed: OllamaStreamLine = match serde_json::from_str(line) {
        Ok(p) => p,
        Err(_) => return None,
    };
    if parsed.done {
        Some(Ok(ProviderEvent::Finished {
            usage: TokenUsage {
                input_tokens: parsed.prompt_eval_count,
                output_tokens: parsed.eval_count,
            },
        }))
    } else {
        Some(Ok(ProviderEvent::TextDelta(parsed.response)))
    }
}

// --- Google Gemini SSE adapter --------------------------------------------------

/// Streams `streamGenerateContent?alt=sse` from the Gemini API (issue #83, the ADR 0002
/// deferred adapter). Same key-sourcing rule as the blocking `GoogleClient`: the caller
/// passes a key it already has, and Gemini takes it as a `?key=` query parameter — so
/// the request URL is a secret and must never be logged. Reuses the parent module's
/// `GoogleRequest`/`GoogleResponse` wire structs: each SSE `data:` event is one
/// `GenerateContentResponse` chunk, and the final chunk carries `finishReason`.
pub struct GoogleStreamingClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl GoogleStreamingClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait]
impl StreamingLlmClient for GoogleStreamingClient {
    fn provider(&self) -> Provider {
        Provider::Google
    }

    async fn stream(
        &self,
        request: StreamRequest,
        cancel: CancellationToken,
    ) -> Result<ProviderEventStream, LlmError> {
        let body = crate::GoogleRequest {
            contents: vec![crate::GoogleContent {
                parts: vec![crate::GooglePart {
                    text: &request.prompt,
                }],
            }],
        };
        // Connect-phase cancellation only — the read loop is owned by the supervisor
        // (see `ndjson_to_provider_events`' comment on why the adapter must not race
        // the same token a second time).
        let response = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(LlmError::Network("cancelled before connecting".into()));
            }
            result = self
                .http
                .post(format!(
                    "{}/v1beta/models/{}:streamGenerateContent",
                    self.base_url, request.model
                ))
                .query(&[("alt", "sse"), ("key", self.api_key.as_str())])
                .json(&body)
                .send() => result.map_err(|e| LlmError::Network(e.to_string()))?,
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Http { status, body });
        }

        Ok(Box::pin(sse_to_provider_events(response.bytes_stream(), google_chunk_to_events)))
    }
}

/// Turns a raw SSE byte stream into `ProviderEvent`s, with `parse` mapping one event's
/// `data:` payload to events. Events are separated by a blank line; only `data:` fields
/// matter (Anthropic's `event:` names repeat the payload's `type`), and one event may
/// span several network chunks, so bytes are buffered until a full event is present.
/// Same single-owner cancellation rule as `ndjson_to_provider_events`.
fn sse_to_provider_events(
    mut byte_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send + 'static,
    mut parse: impl FnMut(&str, &mut TokenUsage) -> Vec<Result<ProviderEvent, LlmError>> + Send + 'static,
) -> impl Stream<Item = Result<ProviderEvent, LlmError>> + Send {
    async_stream::stream! {
        yield Ok(ProviderEvent::StreamOpened);
        let mut buffer = String::new();
        let mut usage = TokenUsage::default();
        loop {
            let chunk = byte_stream.next().await;
            let Some(chunk) = chunk else {
                // Connection closed: a trailing event without its blank-line terminator
                // is still a complete event if it parses.
                let tail = std::mem::take(&mut buffer);
                if let Some(data) = sse_event_data(&tail) {
                    for event in parse(&data, &mut usage) {
                        yield event;
                    }
                }
                return;
            };
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    yield Err(LlmError::Network(e.to_string()));
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            buffer = buffer.replace("\r\n", "\n");
            while let Some(pos) = buffer.find("\n\n") {
                let raw_event = buffer[..pos].to_string();
                buffer.drain(..pos + 2);
                let Some(data) = sse_event_data(&raw_event) else {
                    continue;
                };
                for event in parse(&data, &mut usage) {
                    let terminal = matches!(event, Ok(ProviderEvent::Finished { .. }) | Err(_));
                    yield event;
                    if terminal {
                        return;
                    }
                }
            }
        }
    }
}

/// Joins an SSE event's `data:` lines (multiple `data:` lines concatenate with `\n`).
/// `None` for an event with no data (a comment or keep-alive).
fn sse_event_data(raw_event: &str) -> Option<String> {
    let lines: Vec<&str> = raw_event
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|rest| rest.strip_prefix(' ').unwrap_or(rest))
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// One Gemini SSE chunk -> zero or more provider events. Usage metadata is cumulative
/// across chunks, so the latest seen wins and is attached to `Finished`.
fn google_chunk_to_events(
    data: &str,
    usage: &mut TokenUsage,
) -> Vec<Result<ProviderEvent, LlmError>> {
    let value: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => {
            return vec![Err(LlmError::Provider(format!(
                "malformed Gemini SSE chunk: {data}"
            )))]
        }
    };
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error")
            .to_string();
        return vec![Err(LlmError::Provider(format!("Gemini stream error: {message}")))];
    }
    let parsed: crate::GoogleResponse = match serde_json::from_value(value) {
        Ok(p) => p,
        Err(e) => {
            return vec![Err(LlmError::Provider(format!(
                "failed to parse Gemini chunk: {e}"
            )))]
        }
    };
    if let Some(reported) = parsed.usage_metadata {
        *usage = TokenUsage {
            input_tokens: Some(reported.prompt_token_count),
            output_tokens: Some(reported.candidates_token_count),
        };
    }
    let mut events = Vec::new();
    let mut finished = false;
    for candidate in parsed.candidates {
        for part in candidate.content.parts {
            if !part.text.is_empty() {
                events.push(Ok(ProviderEvent::TextDelta(part.text)));
            }
        }
        finished |= candidate.finish_reason.is_some();
    }
    if finished {
        events.push(Ok(ProviderEvent::Finished {
            usage: *usage,
        }));
    }
    events
}

/// Streams the Anthropic Messages API (`stream: true`). Same key-sourcing rule and
/// `max_tokens` headroom as the blocking `AnthropicClient`.
pub struct AnthropicStreamingClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl AnthropicStreamingClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.anthropic.com".to_string(),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait]
impl StreamingLlmClient for AnthropicStreamingClient {
    fn provider(&self) -> Provider {
        Provider::Anthropic
    }

    async fn stream(
        &self,
        request: StreamRequest,
        cancel: CancellationToken,
    ) -> Result<ProviderEventStream, LlmError> {
        let body = serde_json::json!({
            "model": request.model,
            "max_tokens": 16384,
            "stream": true,
            "messages": [{"role": "user", "content": request.prompt}],
        });
        let response = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(LlmError::Network("cancelled before connecting".into()));
            }
            result = self
                .http
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send() => result.map_err(|e| LlmError::Network(e.to_string()))?,
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Http { status, body });
        }
        Ok(Box::pin(sse_to_provider_events(response.bytes_stream(), anthropic_event_to_events)))
    }
}

/// One Anthropic SSE event -> provider events. Only `text_delta`s are shown: thinking
/// blocks are never forwarded (see `ProviderEvent`). A `max_tokens` or `refusal` stop
/// fails the run, as the blocking client does.
fn anthropic_event_to_events(
    data: &str,
    usage: &mut TokenUsage,
) -> Vec<Result<ProviderEvent, LlmError>> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
        return vec![Err(LlmError::Provider(format!("malformed Anthropic SSE event: {data}")))];
    };
    let tokens = |v: &serde_json::Value, key: &str| v.get(key).and_then(|n| n.as_u64()).map(|n| n as u32);
    match value.get("type").and_then(|t| t.as_str()) {
        Some("message_start") => {
            if let Some(u) = value.pointer("/message/usage") {
                usage.input_tokens = tokens(u, "input_tokens").or(usage.input_tokens);
                usage.output_tokens = tokens(u, "output_tokens").or(usage.output_tokens);
            }
            vec![]
        }
        Some("content_block_delta") => match value.pointer("/delta/type").and_then(|t| t.as_str()) {
            Some("text_delta") => value
                .pointer("/delta/text")
                .and_then(|t| t.as_str())
                .filter(|t| !t.is_empty())
                .map(|t| vec![Ok(ProviderEvent::TextDelta(t.to_string()))])
                .unwrap_or_default(),
            _ => vec![],
        },
        Some("message_delta") => {
            if let Some(u) = value.get("usage") {
                usage.input_tokens = tokens(u, "input_tokens").or(usage.input_tokens);
                usage.output_tokens = tokens(u, "output_tokens").or(usage.output_tokens);
            }
            match value.pointer("/delta/stop_reason").and_then(|r| r.as_str()) {
                Some("max_tokens") => vec![Err(LlmError::Provider(
                    "Anthropic response truncated at max_tokens=16384".to_string(),
                ))],
                Some("refusal") => vec![Err(LlmError::Provider(
                    "Anthropic declined this request (stop_reason: refusal)".to_string(),
                ))],
                _ => vec![],
            }
        }
        Some("message_stop") => vec![Ok(ProviderEvent::Finished { usage: *usage })],
        Some("error") => {
            let message = value.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("unknown error");
            vec![Err(LlmError::Provider(format!("Anthropic stream error: {message}")))]
        }
        _ => vec![],
    }
}

/// Streams OpenAI Chat Completions (`stream: true`, with usage on the last chunk). Same
/// key-sourcing rule as the blocking `OpenAIClient`.
pub struct OpenAIStreamingClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl OpenAIStreamingClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.openai.com".to_string(),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait]
impl StreamingLlmClient for OpenAIStreamingClient {
    fn provider(&self) -> Provider {
        Provider::OpenAI
    }

    async fn stream(
        &self,
        request: StreamRequest,
        cancel: CancellationToken,
    ) -> Result<ProviderEventStream, LlmError> {
        let body = serde_json::json!({
            "model": request.model,
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": [{"role": "user", "content": request.prompt}],
        });
        let response = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(LlmError::Network("cancelled before connecting".into()));
            }
            result = self
                .http
                .post(format!("{}/v1/chat/completions", self.base_url))
                .bearer_auth(&self.api_key)
                .json(&body)
                .send() => result.map_err(|e| LlmError::Network(e.to_string()))?,
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Http { status, body });
        }
        Ok(Box::pin(sse_to_provider_events(response.bytes_stream(), openai_chunk_to_events)))
    }
}

/// One OpenAI SSE chunk -> provider events. Text is in `choices[0].delta.content`;
/// usage arrives on a final chunk with no choices; `[DONE]` ends the stream.
fn openai_chunk_to_events(
    data: &str,
    usage: &mut TokenUsage,
) -> Vec<Result<ProviderEvent, LlmError>> {
    if data.trim() == "[DONE]" {
        return vec![Ok(ProviderEvent::Finished { usage: *usage })];
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
        return vec![Err(LlmError::Provider(format!("malformed OpenAI SSE chunk: {data}")))];
    };
    if let Some(error) = value.get("error") {
        let message = error.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return vec![Err(LlmError::Provider(format!("OpenAI stream error: {message}")))];
    }
    if let Some(u) = value.get("usage").filter(|u| !u.is_null()) {
        let tokens = |key: &str| u.get(key).and_then(|n| n.as_u64()).map(|n| n as u32);
        usage.input_tokens = tokens("prompt_tokens");
        usage.output_tokens = tokens("completion_tokens");
    }
    value
        .pointer("/choices/0/delta/content")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| vec![Ok(ProviderEvent::TextDelta(t.to_string()))])
        .unwrap_or_default()
}

#[cfg(test)]
pub mod test_support {
    //! `FakeOllamaNdjsonServer` — a tiny raw-socket HTTP/1.1 chunked-transfer server used
    //! by this module's tests. Deliberately not a mock-HTTP crate dependency: the wire
    //! format needed (chunked NDJSON) is simple enough that a background `std::thread`
    //! writing raw bytes is both the smallest and the most literal proof that the client
    //! reads a real socket, not a fake in-process stream.
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    pub enum ScriptStep {
        /// Sleep before continuing — simulates connect/first-token delay.
        Delay(Duration),
        /// Write one NDJSON line (a `response` chunk, `done: false`).
        Chunk(String),
        /// Write the terminal `done: true` line and end the HTTP chunked body cleanly.
        Done {
            prompt_eval_count: u32,
            eval_count: u32,
        },
        /// Write bytes that are not valid JSON, then continue the script.
        Malformed(String),
        /// Never send the terminating chunk; sleep effectively forever. Used to prove
        /// cancellation interrupts the client read rather than waiting for the server.
        HangForever,
        /// Close the TCP connection immediately without a terminal chunk.
        AbruptClose,
        /// Write these bytes verbatim as one HTTP chunk — scripts SSE frames (including
        /// deliberately split ones) for the Gemini adapter. The body is terminated
        /// cleanly once the script runs out.
        Raw(String),
    }

    pub struct FakeOllamaNdjsonServer {
        pub base_url: String,
    }

    impl FakeOllamaNdjsonServer {
        /// Starts a background thread that accepts exactly one connection and replays
        /// `script` as HTTP/1.1 chunked-transfer-encoded bytes.
        pub fn start(script: Vec<ScriptStep>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake ollama server");
            let port = listener.local_addr().unwrap().port();
            std::thread::spawn(move || {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                // Drain the request headers (don't care about content).
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);

                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/x-ndjson\r\n\r\n");

                for step in script {
                    match step {
                        ScriptStep::Delay(d) => std::thread::sleep(d),
                        ScriptStep::Chunk(text) => {
                            let line = format!(
                                "{{\"model\":\"test\",\"response\":{},\"done\":false}}\n",
                                serde_json::to_string(&text).unwrap()
                            );
                            if write_chunk(&mut stream, &line).is_err() {
                                return;
                            }
                        }
                        ScriptStep::Done {
                            prompt_eval_count,
                            eval_count,
                        } => {
                            let line = format!(
                                "{{\"model\":\"test\",\"response\":\"\",\"done\":true,\"prompt_eval_count\":{prompt_eval_count},\"eval_count\":{eval_count}}}\n"
                            );
                            if write_chunk(&mut stream, &line).is_err() {
                                return;
                            }
                            let _ = stream.write_all(b"0\r\n\r\n");
                            return;
                        }
                        ScriptStep::Malformed(text) => {
                            let line = format!("{text}\n");
                            if write_chunk(&mut stream, &line).is_err() {
                                return;
                            }
                        }
                        ScriptStep::HangForever => {
                            std::thread::sleep(Duration::from_secs(30));
                            return;
                        }
                        ScriptStep::AbruptClose => {
                            return;
                        }
                        ScriptStep::Raw(bytes) => {
                            if write_chunk(&mut stream, &bytes).is_err() {
                                return;
                            }
                        }
                    }
                }
                let _ = stream.write_all(b"0\r\n\r\n");
            });
            Self {
                base_url: format!("http://127.0.0.1:{port}"),
            }
        }
    }

    fn write_chunk(stream: &mut std::net::TcpStream, data: &str) -> std::io::Result<()> {
        let framed = format!("{:x}\r\n{}\r\n", data.len(), data);
        stream.write_all(framed.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{FakeOllamaNdjsonServer, ScriptStep};
    use super::*;

    #[test]
    fn the_ollama_request_asks_for_a_16k_context_window() {
        let body = ollama_request_body("llama3", "hi");
        assert_eq!(body["options"]["num_ctx"], 16_384);
        assert_eq!(body["model"], "llama3");
        assert_eq!(body["stream"], true);
    }
    use std::sync::Mutex;
    use std::time::Duration;

    struct CollectingSink {
        events: Mutex<Vec<InferenceEvent>>,
    }
    impl CollectingSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }
        fn events(&self) -> Vec<InferenceEvent> {
            self.events.lock().unwrap().clone()
        }
    }
    impl EventSink for CollectingSink {
        fn try_emit(&self, event: InferenceEvent) -> Result<(), LlmError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    fn chain_for(base_url: &str) -> Vec<StreamingChainLink> {
        vec![StreamingChainLink {
            provider: Provider::Ollama,
            model: "test-model".to_string(),
            client: Arc::new(OllamaStreamingClient::with_base_url(base_url)),
        }]
    }

    /// INF-001/INF-003: a normal short answer produces State(Connecting) ->
    /// State(Streaming) -> TextDelta* -> Terminal(Completed) with the concatenated text
    /// and reported usage.
    #[tokio::test]
    async fn successful_stream_emits_ordered_events_and_final_usage() {
        let server = FakeOllamaNdjsonServer::start(vec![
            ScriptStep::Chunk("Hello".to_string()),
            ScriptStep::Chunk(", world".to_string()),
            ScriptStep::Done {
                prompt_eval_count: 5,
                eval_count: 3,
            },
        ]);
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let cancel = CancellationToken::new();
        let request = StreamRequest {
            run_id: new_run_id(),
            model: "test-model".to_string(),
            prompt: "hi".to_string(),
            started_at_unix_ms: now(),
        };

        supervisor
            .drive(request.clone(), &chain_for(&server.base_url), cancel, now)
            .await;

        let events = sink.events();
        assert!(matches!(
            events[0],
            InferenceEvent::State {
                state: InferenceRunState::Connecting,
                ..
            }
        ));
        assert!(events.iter().any(|e| matches!(
            e,
            InferenceEvent::State {
                state: InferenceRunState::Streaming,
                ..
            }
        )));
        let deltas: String = events
            .iter()
            .filter_map(|e| match e {
                InferenceEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, "Hello, world");
        let terminal = events.last().unwrap();
        match terminal {
            InferenceEvent::Terminal {
                outcome,
                final_text,
                usage,
                ..
            } => {
                assert_eq!(*outcome, TerminalOutcome::Completed);
                assert_eq!(final_text, "Hello, world");
                assert_eq!(usage.input_tokens, Some(5));
                assert_eq!(usage.output_tokens, Some(3));
            }
            other => panic!("expected Terminal, got {other:?}"),
        }
    }

    /// INF-002: a delayed connection still shows Connecting before any delta, and the
    /// call does not block the executor (proven by the test itself completing).
    #[tokio::test]
    async fn delayed_first_chunk_still_reports_connecting_before_any_delta() {
        let server = FakeOllamaNdjsonServer::start(vec![
            ScriptStep::Delay(Duration::from_millis(50)),
            ScriptStep::Chunk("late but real".to_string()),
            ScriptStep::Done {
                prompt_eval_count: 1,
                eval_count: 1,
            },
        ]);
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let request = StreamRequest {
            run_id: new_run_id(),
            model: "test-model".to_string(),
            prompt: "hi".to_string(),
            started_at_unix_ms: now(),
        };
        supervisor
            .drive(
                request,
                &chain_for(&server.base_url),
                CancellationToken::new(),
                now,
            )
            .await;
        let events = sink.events();
        let connecting_idx = events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    InferenceEvent::State {
                        state: InferenceRunState::Connecting,
                        ..
                    }
                )
            })
            .unwrap();
        let first_delta_idx = events
            .iter()
            .position(|e| matches!(e, InferenceEvent::TextDelta { .. }))
            .unwrap();
        assert!(connecting_idx < first_delta_idx);
    }

    /// INF-006: a malformed chunk after valid content ends the run with a coherent
    /// Failed terminal (not a panic, not a silent hang), and the already-received partial
    /// text is retained in the terminal event.
    #[tokio::test]
    async fn malformed_chunk_after_valid_content_fails_coherently_and_keeps_partial_text() {
        let server = FakeOllamaNdjsonServer::start(vec![
            ScriptStep::Chunk("partial answer".to_string()),
            ScriptStep::Malformed("not json at all {".to_string()),
        ]);
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let request = StreamRequest {
            run_id: new_run_id(),
            model: "test-model".to_string(),
            prompt: "hi".to_string(),
            started_at_unix_ms: now(),
        };
        supervisor
            .drive(
                request,
                &chain_for(&server.base_url),
                CancellationToken::new(),
                now,
            )
            .await;
        let events = sink.events();
        match events.last().unwrap() {
            InferenceEvent::Terminal {
                outcome: TerminalOutcome::Failed { class, .. },
                final_text,
                ..
            } => {
                assert_eq!(*class, InferenceFailureClass::Provider);
                assert_eq!(final_text, "partial answer");
            }
            other => panic!("expected Failed terminal, got {other:?}"),
        }
    }

    /// INF-103: cancelling after the stream opens but before any delta must not render a
    /// false "completed" message — the terminal outcome is Cancelled with empty text.
    #[tokio::test]
    async fn cancel_after_open_before_first_delta_yields_cancelled_not_completed() {
        let server = FakeOllamaNdjsonServer::start(vec![ScriptStep::HangForever]);
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let cancel = CancellationToken::new();
        let request = StreamRequest {
            run_id: new_run_id(),
            model: "test-model".to_string(),
            prompt: "hi".to_string(),
            started_at_unix_ms: now(),
        };
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            cancel_clone.cancel();
        });
        let started = std::time::Instant::now();
        supervisor
            .drive(request, &chain_for(&server.base_url), cancel, now)
            .await;
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "cancellation must interrupt the read, not wait for the 30s hang; took {elapsed:?}"
        );

        let events = sink.events();
        match events.last().unwrap() {
            InferenceEvent::Terminal {
                outcome: TerminalOutcome::Cancelled,
                final_text,
                ..
            } => {
                assert_eq!(final_text, "");
            }
            other => panic!("expected Cancelled terminal, got {other:?}"),
        }
    }

    /// INF-104: cancelling mid-stream (after some deltas already arrived) still yields
    /// Cancelled with only the text received before cancellation, and — because the
    /// server keeps trying to send after the client has disconnected — no further delta
    /// reaches the sink after the Terminal event (proving late chunks are discarded, not
    /// merely unlucky timing).
    #[tokio::test]
    async fn cancel_mid_stream_discards_late_chunks_after_terminal() {
        let server = FakeOllamaNdjsonServer::start(vec![
            ScriptStep::Chunk("first ".to_string()),
            ScriptStep::Delay(Duration::from_millis(80)),
            // These are sent by the fake server regardless of whether the client is
            // still connected; the client-side cancellation must have already dropped
            // the stream by the time these would arrive.
            ScriptStep::Chunk("late-chunk-should-not-render".to_string()),
            ScriptStep::Done {
                prompt_eval_count: 1,
                eval_count: 1,
            },
        ]);
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let cancel = CancellationToken::new();
        let request = StreamRequest {
            run_id: new_run_id(),
            model: "test-model".to_string(),
            prompt: "hi".to_string(),
            started_at_unix_ms: now(),
        };
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel_clone.cancel();
        });
        supervisor
            .drive(request, &chain_for(&server.base_url), cancel, now)
            .await;

        // Give the fake server's later (post-cancel) write a chance to have happened if
        // it were somehow still being read.
        tokio::time::sleep(Duration::from_millis(150)).await;

        let events = sink.events();
        assert!(events.last().unwrap().is_terminal());
        let all_text: String = events
            .iter()
            .filter_map(|e| match e {
                InferenceEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !all_text.contains("late-chunk-should-not-render"),
            "a chunk sent after client-side cancellation must never reach the sink: {all_text:?}"
        );
    }

    /// INF-108: cancel then immediately start a fresh run with a new run_id — the old
    /// run's events (even if somehow still in flight) must never appear under the new
    /// run_id, and the new run's own events are exactly its own.
    #[tokio::test]
    async fn new_run_after_cancel_has_a_distinct_run_id_and_no_contamination() {
        let server1 = FakeOllamaNdjsonServer::start(vec![ScriptStep::HangForever]);
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let cancel1 = CancellationToken::new();
        let request1 = StreamRequest {
            run_id: new_run_id(),
            model: "test-model".to_string(),
            prompt: "first".to_string(),
            started_at_unix_ms: now(),
        };
        let cancel1_clone = cancel1.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel1_clone.cancel();
        });
        supervisor
            .drive(
                request1.clone(),
                &chain_for(&server1.base_url),
                cancel1,
                now,
            )
            .await;

        let server2 = FakeOllamaNdjsonServer::start(vec![
            ScriptStep::Chunk("second run text".to_string()),
            ScriptStep::Done {
                prompt_eval_count: 1,
                eval_count: 1,
            },
        ]);
        let request2 = StreamRequest {
            run_id: new_run_id(),
            model: "test-model".to_string(),
            prompt: "second".to_string(),
            started_at_unix_ms: now(),
        };
        assert_ne!(request1.run_id, request2.run_id);
        supervisor
            .drive(
                request2.clone(),
                &chain_for(&server2.base_url),
                CancellationToken::new(),
                now,
            )
            .await;

        let events = sink.events();
        let run2_deltas: String = events
            .iter()
            .filter(|e| e.run_id() == request2.run_id)
            .filter_map(|e| match e {
                InferenceEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(run2_deltas, "second run text");
        // No event carrying run1's id has any text (run1 was cancelled before output).
        assert!(events
            .iter()
            .filter(|e| e.run_id() == request1.run_id)
            .all(|e| !matches!(e, InferenceEvent::TextDelta { .. })));
    }

    /// INF-009: once output has been emitted, a mid-stream provider failure must not
    /// fall through to a second chain link — it ends Failed with the partial text kept.
    #[tokio::test]
    async fn failure_after_output_does_not_fall_through_to_next_provider() {
        let server1 = FakeOllamaNdjsonServer::start(vec![
            ScriptStep::Chunk("first provider text".to_string()),
            ScriptStep::AbruptClose,
        ]);
        let server2 = FakeOllamaNdjsonServer::start(vec![
            ScriptStep::Chunk("SHOULD NOT APPEAR".to_string()),
            ScriptStep::Done {
                prompt_eval_count: 1,
                eval_count: 1,
            },
        ]);
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let chain = vec![
            StreamingChainLink {
                provider: Provider::Ollama,
                model: "m1".to_string(),
                client: Arc::new(OllamaStreamingClient::with_base_url(&server1.base_url)),
            },
            StreamingChainLink {
                provider: Provider::Ollama,
                model: "m2".to_string(),
                client: Arc::new(OllamaStreamingClient::with_base_url(&server2.base_url)),
            },
        ];
        let request = StreamRequest {
            run_id: new_run_id(),
            model: "m1".to_string(),
            prompt: "hi".to_string(),
            started_at_unix_ms: now(),
        };
        supervisor
            .drive(request, &chain, CancellationToken::new(), now)
            .await;

        let events = sink.events();
        let all_text: String = events
            .iter()
            .filter_map(|e| match e {
                InferenceEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(all_text, "first provider text");
        assert!(!all_text.contains("SHOULD NOT APPEAR"));
        match events.last().unwrap() {
            InferenceEvent::Terminal {
                outcome: TerminalOutcome::Failed { .. },
                final_text,
                ..
            } => {
                assert_eq!(final_text, "first provider text");
            }
            other => panic!("expected Failed terminal (no silent fallback), got {other:?}"),
        }
    }

    fn google_chain_for(base_url: &str) -> Vec<StreamingChainLink> {
        vec![StreamingChainLink {
            provider: Provider::Google,
            model: "gemini-test".to_string(),
            client: Arc::new(GoogleStreamingClient::new("test-key").with_base_url(base_url)),
        }]
    }

    async fn drive_google(script: Vec<ScriptStep>) -> Vec<InferenceEvent> {
        let server = FakeOllamaNdjsonServer::start(script);
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let request = StreamRequest {
            run_id: new_run_id(),
            model: "gemini-test".to_string(),
            prompt: "hi".to_string(),
            started_at_unix_ms: now(),
        };
        supervisor
            .drive(request, &google_chain_for(&server.base_url), CancellationToken::new(), now)
            .await;
        sink.events()
    }

    fn deltas(events: &[InferenceEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                InferenceEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Gemini's `alt=sse` framing: one `data:` JSON chunk per event, `\r\n` line ends,
    /// text in `candidates[].content.parts[].text`, `finishReason` + cumulative
    /// `usageMetadata` on the last chunk.
    #[tokio::test]
    async fn gemini_sse_stream_emits_ordered_deltas_and_final_usage() {
        let events = drive_google(vec![
            ScriptStep::Raw(
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":1}}\r\n\r\n".into(),
            ),
            ScriptStep::Raw(
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\", world\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":3}}\r\n\r\n".into(),
            ),
        ])
        .await;
        assert!(matches!(
            events[0],
            InferenceEvent::State { state: InferenceRunState::Connecting, provider: Some(ref p), .. } if p == "google"
        ));
        assert_eq!(deltas(&events), "Hello, world");
        match events.last().unwrap() {
            InferenceEvent::Terminal { outcome, final_text, usage, .. } => {
                assert_eq!(*outcome, TerminalOutcome::Completed);
                assert_eq!(final_text, "Hello, world");
                assert_eq!(usage.input_tokens, Some(5));
                assert_eq!(usage.output_tokens, Some(3));
            }
            other => panic!("expected Terminal, got {other:?}"),
        }
    }

    /// One SSE event split across two TCP chunks (and a keep-alive comment in between)
    /// is reassembled before parsing rather than reported as malformed.
    #[tokio::test]
    async fn gemini_sse_event_split_across_chunks_is_reassembled() {
        let events = drive_google(vec![
            ScriptStep::Raw("data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"split".into()),
            ScriptStep::Delay(Duration::from_millis(40)),
            ScriptStep::Raw(" text\"}]}}]}\n\n: keep-alive\n\n".into()),
            ScriptStep::Raw("data: {\"candidates\":[{\"content\":{\"parts\":[]},\"finishReason\":\"STOP\"}]}\n\n".into()),
        ])
        .await;
        assert_eq!(deltas(&events), "split text");
        assert!(matches!(
            events.last().unwrap(),
            InferenceEvent::Terminal { outcome: TerminalOutcome::Completed, .. }
        ));
    }

    /// An in-band `{"error": ...}` event (how Gemini reports a bad key or quota mid-stream)
    /// ends the run Failed/Provider with the provider's message, keeping any partial text.
    #[tokio::test]
    async fn gemini_in_band_error_event_fails_the_run_with_the_provider_message() {
        let events = drive_google(vec![
            ScriptStep::Raw("data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"partial\"}]}}]}\n\n".into()),
            ScriptStep::Raw("data: {\"error\":{\"code\":400,\"message\":\"API key not valid\",\"status\":\"INVALID_ARGUMENT\"}}\n\n".into()),
        ])
        .await;
        match events.last().unwrap() {
            InferenceEvent::Terminal { outcome: TerminalOutcome::Failed { class, message }, final_text, .. } => {
                assert_eq!(*class, InferenceFailureClass::Provider);
                assert!(message.contains("API key not valid"), "{message}");
                assert_eq!(final_text, "partial");
            }
            other => panic!("expected Failed terminal, got {other:?}"),
        }
    }

    async fn drive_with(provider: Provider, script: Vec<ScriptStep>) -> Vec<InferenceEvent> {
        let server = FakeOllamaNdjsonServer::start(script);
        let client: Arc<dyn StreamingLlmClient> = match provider {
            Provider::Anthropic => Arc::new(AnthropicStreamingClient::new("test-key").with_base_url(&server.base_url)),
            Provider::OpenAI => Arc::new(OpenAIStreamingClient::new("test-key").with_base_url(&server.base_url)),
            other => panic!("no SSE test client for {other:?}"),
        };
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let request = StreamRequest { run_id: new_run_id(), model: "test-model".to_string(), prompt: "hi".to_string(), started_at_unix_ms: now() };
        let chain = vec![StreamingChainLink { provider, model: "test-model".to_string(), client }];
        supervisor.drive(request, &chain, CancellationToken::new(), now).await;
        sink.events()
    }

    /// Anthropic's framing: `event:` + `data:` lines, input usage on `message_start`,
    /// output usage on `message_delta`, `message_stop` ends. Thinking deltas never show.
    #[tokio::test]
    async fn anthropic_sse_stream_emits_text_deltas_only_and_final_usage() {
        let events = drive_with(Provider::Anthropic, vec![
            ScriptStep::Raw("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":1}}}\n\n".into()),
            ScriptStep::Raw("event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n".into()),
            ScriptStep::Raw("event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"secret reasoning\"}}\n\n".into()),
            ScriptStep::Raw("event: ping\ndata: {\"type\":\"ping\"}\n\n".into()),
            ScriptStep::Raw("event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n".into()),
            ScriptStep::Raw("event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\", world\"}}\n\n".into()),
            ScriptStep::Raw("event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":30}}\n\n".into()),
            ScriptStep::Raw("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".into()),
        ])
        .await;
        assert!(matches!(
            events[0],
            InferenceEvent::State { state: InferenceRunState::Connecting, provider: Some(ref p), .. } if p == "anthropic"
        ));
        assert_eq!(deltas(&events), "Hello, world");
        match events.last().unwrap() {
            InferenceEvent::Terminal { outcome, final_text, usage, .. } => {
                assert_eq!(*outcome, TerminalOutcome::Completed);
                assert_eq!(final_text, "Hello, world");
                assert_eq!((usage.input_tokens, usage.output_tokens), (Some(12), Some(30)));
            }
            other => panic!("expected Terminal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn anthropic_refusal_stop_fails_the_run() {
        let events = drive_with(Provider::Anthropic, vec![
            ScriptStep::Raw("data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"I\"}}\n\n".into()),
            ScriptStep::Raw("data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"},\"usage\":{\"output_tokens\":1}}\n\n".into()),
            ScriptStep::Raw("data: {\"type\":\"message_stop\"}\n\n".into()),
        ])
        .await;
        match events.last().unwrap() {
            InferenceEvent::Terminal { outcome: TerminalOutcome::Failed { class, message }, .. } => {
                assert_eq!(*class, InferenceFailureClass::Provider);
                assert!(message.contains("refusal"), "{message}");
            }
            other => panic!("expected Failed terminal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn anthropic_in_band_error_event_fails_the_run() {
        let events = drive_with(Provider::Anthropic, vec![
            ScriptStep::Raw("event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n".into()),
        ])
        .await;
        assert!(matches!(
            events.last().unwrap(),
            InferenceEvent::Terminal { outcome: TerminalOutcome::Failed { message, .. }, .. } if message.contains("Overloaded")
        ));
    }

    /// OpenAI's framing: text in `choices[0].delta.content`, usage on a last chunk with
    /// empty `choices`, then `data: [DONE]`.
    #[tokio::test]
    async fn openai_sse_stream_emits_ordered_deltas_and_final_usage() {
        let events = drive_with(Provider::OpenAI, vec![
            ScriptStep::Raw("data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}],\"usage\":null}\n\n".into()),
            ScriptStep::Raw("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}],\"usage\":null}\n\n".into()),
            ScriptStep::Raw("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\", world\"},\"finish_reason\":null}],\"usage\":null}\n\n".into()),
            ScriptStep::Raw("data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":null}\n\n".into()),
            ScriptStep::Raw("data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":3,\"total_tokens\":12}}\n\n".into()),
            ScriptStep::Raw("data: [DONE]\n\n".into()),
        ])
        .await;
        assert_eq!(deltas(&events), "Hello, world");
        match events.last().unwrap() {
            InferenceEvent::Terminal { outcome, usage, .. } => {
                assert_eq!(*outcome, TerminalOutcome::Completed);
                assert_eq!((usage.input_tokens, usage.output_tokens), (Some(9), Some(3)));
            }
            other => panic!("expected Terminal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn openai_in_band_error_fails_the_run_keeping_partial_text() {
        let events = drive_with(Provider::OpenAI, vec![
            ScriptStep::Raw("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"}}]}\n\n".into()),
            ScriptStep::Raw("data: {\"error\":{\"message\":\"Rate limit reached\",\"type\":\"requests\"}}\n\n".into()),
        ])
        .await;
        match events.last().unwrap() {
            InferenceEvent::Terminal { outcome: TerminalOutcome::Failed { message, .. }, final_text, .. } => {
                assert!(message.contains("Rate limit reached"), "{message}");
                assert_eq!(final_text, "partial");
            }
            other => panic!("expected Failed terminal, got {other:?}"),
        }
    }

    /// Opt-in live smoke test against the real Gemini API — same convention as the
    /// Ollama one below. Run with:
    /// `WHIMBREL_GOOGLE_SMOKE_KEY=<key> cargo test -p llm -- --ignored live_gemini --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_gemini_streams_a_real_short_answer() {
        let api_key = std::env::var("WHIMBREL_GOOGLE_SMOKE_KEY").expect("set WHIMBREL_GOOGLE_SMOKE_KEY");
        let model = std::env::var("WHIMBREL_GOOGLE_SMOKE_MODEL").unwrap_or_else(|_| "gemini-2.5-flash".into());
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let request = StreamRequest {
            run_id: new_run_id(),
            model: model.clone(),
            prompt: "Reply with exactly the two words: hello world".to_string(),
            started_at_unix_ms: now(),
        };
        let chain = vec![StreamingChainLink {
            provider: Provider::Google,
            model,
            client: Arc::new(GoogleStreamingClient::new(api_key)),
        }];
        supervisor.drive(request, &chain, CancellationToken::new(), now).await;
        let events = sink.events();
        for event in &events {
            println!("{event:?}");
        }
        assert!(events.iter().any(|e| matches!(e, InferenceEvent::TextDelta { .. })));
        match events.last().unwrap() {
            InferenceEvent::Terminal { outcome: TerminalOutcome::Completed, final_text, usage, .. } => {
                assert!(!final_text.is_empty());
                assert!(usage.output_tokens.is_some(), "Gemini reports usage on the final chunk");
            }
            other => panic!("expected Completed terminal against the real Gemini API, got {other:?}"),
        }
    }

    /// Opt-in live smoke tests against the real Anthropic and OpenAI APIs, same convention
    /// as the Gemini one. Run with:
    /// `WHIMBREL_ANTHROPIC_SMOKE_KEY=<key> cargo test -p llm -- --ignored live_anthropic --nocapture`
    /// `WHIMBREL_OPENAI_SMOKE_KEY=<key> cargo test -p llm -- --ignored live_openai --nocapture`
    /// (`WHIMBREL_ANTHROPIC_SMOKE_MODEL` / `WHIMBREL_OPENAI_SMOKE_MODEL` override the model.)
    async fn live_hosted_stream(provider: Provider, client: Arc<dyn StreamingLlmClient>, model: String) {
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let request = StreamRequest {
            run_id: new_run_id(),
            model: model.clone(),
            prompt: "Reply with exactly the two words: hello world".to_string(),
            started_at_unix_ms: now(),
        };
        let chain = vec![StreamingChainLink { provider, model, client }];
        supervisor.drive(request, &chain, CancellationToken::new(), now).await;
        let events = sink.events();
        for event in &events {
            println!("{event:?}");
        }
        assert!(events.iter().any(|e| matches!(e, InferenceEvent::TextDelta { .. })));
        match events.last().unwrap() {
            InferenceEvent::Terminal { outcome: TerminalOutcome::Completed, final_text, usage, .. } => {
                assert!(!final_text.is_empty());
                assert!(usage.output_tokens.is_some(), "the provider reports usage");
            }
            other => panic!("expected Completed terminal against the real API, got {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_anthropic_streams_a_real_short_answer() {
        let key = std::env::var("WHIMBREL_ANTHROPIC_SMOKE_KEY").expect("set WHIMBREL_ANTHROPIC_SMOKE_KEY");
        let model = std::env::var("WHIMBREL_ANTHROPIC_SMOKE_MODEL").unwrap_or_else(|_| "claude-haiku-4-5-20251001".into());
        live_hosted_stream(Provider::Anthropic, Arc::new(AnthropicStreamingClient::new(key)), model).await;
    }

    #[tokio::test]
    #[ignore]
    async fn live_openai_streams_a_real_short_answer() {
        let key = std::env::var("WHIMBREL_OPENAI_SMOKE_KEY").expect("set WHIMBREL_OPENAI_SMOKE_KEY");
        let model = std::env::var("WHIMBREL_OPENAI_SMOKE_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
        live_hosted_stream(Provider::OpenAI, Arc::new(OpenAIStreamingClient::new(key)), model).await;
    }

    /// Opt-in live smoke test against a real local Ollama daemon — same convention as
    /// `llm::lib`'s existing `#[ignore]`d Anthropic/OpenAI/Google tests. Not part of the
    /// default test run (per the hand-off: "do not claim a live provider is validated
    /// solely by mocks; document and run an opt-in ... smoke procedure separately").
    /// Run with:
    /// `WHIMBREL_OLLAMA_SMOKE_MODEL=<model> cargo test -p llm -- --ignored live_ollama --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_ollama_streams_a_real_short_answer() {
        let model = std::env::var("WHIMBREL_OLLAMA_SMOKE_MODEL")
            .expect("set WHIMBREL_OLLAMA_SMOKE_MODEL to a model name from `ollama list`");
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let request = StreamRequest {
            run_id: new_run_id(),
            model: model.clone(),
            prompt: "Reply with exactly the two words: hello world".to_string(),
            started_at_unix_ms: now(),
        };
        let chain = vec![StreamingChainLink {
            provider: Provider::Ollama,
            model,
            client: Arc::new(OllamaStreamingClient::new()),
        }];
        supervisor
            .drive(request, &chain, CancellationToken::new(), now)
            .await;
        let events = sink.events();
        for event in &events {
            println!("{event:?}");
        }
        assert!(events
            .iter()
            .any(|e| matches!(e, InferenceEvent::State { state: InferenceRunState::Streaming, .. })));
        assert!(events.iter().any(|e| matches!(e, InferenceEvent::TextDelta { .. })));
        match events.last().unwrap() {
            InferenceEvent::Terminal { outcome: TerminalOutcome::Completed, final_text, .. } => {
                assert!(!final_text.is_empty(), "a real completed run must have non-empty text");
            }
            other => panic!("expected Completed terminal against a real Ollama daemon, got {other:?}"),
        }
    }

    /// Live cancellation proof: cancel a real in-flight local-model stream shortly after
    /// it starts producing text, and assert Whimbrel stops rendering promptly (docs/adr/0002
    /// §5, "no rendered late delta"). Run alongside the test above.
    #[tokio::test]
    #[ignore]
    async fn live_ollama_cancel_mid_stream_yields_cancelled_promptly() {
        let model = std::env::var("WHIMBREL_OLLAMA_SMOKE_MODEL")
            .expect("set WHIMBREL_OLLAMA_SMOKE_MODEL to a model name from `ollama list`");
        let sink = Arc::new(CollectingSink::new());
        let supervisor = StreamingRunSupervisor::new(sink.clone());
        let cancel = CancellationToken::new();
        let request = StreamRequest {
            run_id: new_run_id(),
            model: model.clone(),
            prompt: "Write a long, detailed 500-word essay about lighthouses.".to_string(),
            started_at_unix_ms: now(),
        };
        let chain = vec![StreamingChainLink {
            provider: Provider::Ollama,
            model,
            client: Arc::new(OllamaStreamingClient::new()),
        }];
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            cancel_clone.cancel();
        });
        let started = std::time::Instant::now();
        supervisor.drive(request, &chain, cancel, now).await;
        let elapsed = started.elapsed();
        println!("cancel-to-terminal elapsed: {elapsed:?}");
        for event in sink.events() {
            println!("{event:?}");
        }
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "cancellation against a real local model must not wait for the full 500-word generation; took {elapsed:?}"
        );
        assert!(matches!(
            sink.events().last().unwrap(),
            InferenceEvent::Terminal { outcome: TerminalOutcome::Cancelled, .. }
        ));
    }
}
