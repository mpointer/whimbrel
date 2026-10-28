//! Draft target: core/llm/src/streaming.rs
//!
//! This replaces the UI-facing use of blocking `LlmClient::complete()` gradually. Keep
//! that trait for batch routines initially; route interactive chat and meeting-summary
//! generation through `StreamingLlmClient` first. The common event contract lets Swift
//! avoid knowing whether a provider speaks Ollama NDJSON or Gemini SSE.
//!
//! Cargo additions for core/llm/Cargo.toml:
//!   async-trait = "0.1"
//!   bytes = "1"
//!   futures-util = "0.3"
//!   reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls-native-roots"] }
//!   tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time"] }
//!   tokio-util = { version = "0.7", features = ["rt"] }
//!   uuid = { version = "1", features = ["v4", "serde"] }
//!
//! Dependency direction remains: llm -> memory. Do not introduce a Swift type or FFI
//! callback here. `core/ffi` owns run registration and C-ABI event polling.

use std::{pin::Pin, sync::Arc, time::Instant};

use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{LlmError, Provider, TokenUsage};

pub type InferenceRunId = Uuid;
pub type ProviderEventStream = Pin<Box<dyn Stream<Item = Result<ProviderEvent, LlmError>> + Send>>;

/// Immutable snapshot created before dispatch. It is the exact prompt/context/policy
/// input whose hash and provenance should be persisted by `InferenceRunStore`; do not
/// re-read the editable meeting/note while a run is streaming.
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub run_id: InferenceRunId,
    pub route: InferenceRoute,
    pub model: String,
    pub prompt: String,
    pub started_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceRoute {
    Ask,
    MeetingSummary,
    DailyDigest,
    EpisodeSummary,
    ThreadSummary,
}

/// Provider adapters emit protocol facts, not SwiftUI states. A provider must never
/// emit raw chain-of-thought; providers with a thinking/tool protocol map that material
/// to a policy-safe Progress label, a separate auditable tool status, or omit it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    /// The HTTP response/stream is accepted. This is distinct from task creation, so UI
    /// can say “Connecting…” versus “Generating…”.
    StreamOpened,
    /// Incremental display-safe output.
    TextDelta(String),
    /// Safe progress, e.g. local model warming. Coalesce aggressively at the bridge.
    Progress { label: String },
    /// Provider terminal metadata. Adapter emits this at most once and never after Error.
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
        /// A monotonically increasing sequence. FFI and Swift ignore a duplicate or
        /// stale sequence and additionally guard against a cancelled/newer run ID.
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
    Failed { class: InferenceFailureClass, message: String },
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
    CancelCleanupDelayed,
}

#[async_trait]
pub trait StreamingLlmClient: Send + Sync {
    fn provider(&self) -> Provider;

    /// Starts a provider stream. The adapter must observe `cancel` while waiting for
    /// headers and while receiving chunks. Dropping the returned stream must release the
    /// underlying HTTP body/socket. Returning a stream means *the adapter owns parsing
    /// Ollama NDJSON/Gemini SSE into ProviderEvent*, not the caller.
    async fn stream(
        &self,
        request: StreamRequest,
        cancel: CancellationToken,
    ) -> Result<ProviderEventStream, LlmError>;
}

/// Bounded, send-only notification surface used by core/ffi’s run registry. It is not
/// the durable record; `InferenceRunStore` persists state/checkpoints before terminal
/// delivery. A bounded channel prevents a pathological provider from consuming unbounded
/// memory while Swift is temporarily busy. Text deltas may be coalesced only before the
/// first UI delivery, never reordered.
pub trait EventSink: Send + Sync {
    fn try_emit(&self, event: InferenceEvent) -> Result<(), LlmError>;
}

pub struct ChannelEventSink {
    tx: mpsc::Sender<InferenceEvent>,
}

impl ChannelEventSink {
    pub fn new(tx: mpsc::Sender<InferenceEvent>) -> Self { Self { tx } }
}

impl EventSink for ChannelEventSink {
    fn try_emit(&self, event: InferenceEvent) -> Result<(), LlmError> {
        self.tx.try_send(event).map_err(|err| {
            LlmError::Provider(format!("inference event backpressure: {err}"))
        })
    }
}

/// One provider link supplied by the existing routing chain. The supervisor preserves
/// Kestrel’s current rule: fallback is legal only if a retryable failure occurs BEFORE
/// displayable output. Once output reaches the user, silently appending another model’s
/// continuation is prohibited; the run ends as Failed/partial and the user explicitly
/// retries.
pub struct StreamingChainLink {
    pub provider: Provider,
    pub model: String,
    pub client: Arc<dyn StreamingLlmClient>,
}

pub struct RunSupervisor {
    sink: Arc<dyn EventSink>,
}

impl RunSupervisor {
    pub fn new(sink: Arc<dyn EventSink>) -> Self { Self { sink } }

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
                run_id: request.run_id,
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
                Err(error) if output.is_empty() && crate::is_retryable(&error) => continue,
                Err(error) => {
                    self.emit_failure(&request, &output, error, &now_unix_ms);
                    return;
                }
            };
            let mut opened = false;
            let attempt_started = Instant::now();

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        // Dropping `stream` closes reqwest's response body. Individual
                        // adapters must also select on `cancel` while blocked in decoder
                        // reads, so cancellation does not wait for a provider heartbeat.
                        drop(stream);
                        self.emit_cancelled(&request, &output, &now_unix_ms);
                        return;
                    }
                    item = stream.next() => match item {
                        Some(Ok(ProviderEvent::StreamOpened)) => {
                            opened = true;
                            let _ = self.sink.try_emit(InferenceEvent::State {
                                run_id: request.run_id,
                                state: InferenceRunState::Streaming,
                                at_unix_ms: now_unix_ms(),
                                provider: Some(link.provider.as_str().to_owned()),
                                model: Some(link.model.clone()),
                                attempt,
                            });
                        }
                        Some(Ok(ProviderEvent::TextDelta(text))) => {
                            if !opened {
                                // Tolerate adapters that receive first text with headers;
                                // state is still emitted before delta for Swift determinism.
                                opened = true;
                                let _ = self.sink.try_emit(InferenceEvent::State {
                                    run_id: request.run_id,
                                    state: InferenceRunState::Streaming,
                                    at_unix_ms: now_unix_ms(),
                                    provider: Some(link.provider.as_str().to_owned()),
                                    model: Some(link.model.clone()),
                                    attempt,
                                });
                            }
                            output.push_str(&text);
                            sequence += 1;
                            if self.sink.try_emit(InferenceEvent::TextDelta {
                                run_id: request.run_id, sequence, text,
                            }).is_err() {
                                // Treat a dead/overloaded bridge as cancelled work: do not
                                // keep spending on a result that cannot be observed.
                                drop(stream);
                                self.emit_cancelled(&request, &output, &now_unix_ms);
                                return;
                            }
                        }
                        Some(Ok(ProviderEvent::Progress { label })) => {
                            let _ = self.sink.try_emit(InferenceEvent::Progress {
                                run_id: request.run_id, label,
                            });
                        }
                        Some(Ok(ProviderEvent::Finished { usage })) => {
                            self.emit_completed(&request, &output, usage, &now_unix_ms);
                            let _elapsed = attempt_started.elapsed(); // emit redacted metric
                            return;
                        }
                        Some(Err(error)) if output.is_empty() && crate::is_retryable(&error) => {
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
            run_id: request.run_id,
            outcome: TerminalOutcome::Cancelled,
            at_unix_ms: now(),
            final_text: output.to_owned(),
            usage: TokenUsage::default(),
        });
    }

    fn emit_completed(&self, request: &StreamRequest, output: &str, usage: TokenUsage, now: &impl Fn() -> i64) {
        let _ = self.sink.try_emit(InferenceEvent::Terminal {
            run_id: request.run_id,
            outcome: TerminalOutcome::Completed,
            at_unix_ms: now(),
            final_text: output.to_owned(),
            usage,
        });
    }

    fn emit_failure(&self, request: &StreamRequest, output: &str, error: LlmError, now: &impl Fn() -> i64) {
        let class = match error {
            LlmError::Network(_) => InferenceFailureClass::Network,
            LlmError::Http { status: 401 | 403, .. } => InferenceFailureClass::Authentication,
            LlmError::Http { status: 429, .. } => InferenceFailureClass::RateLimited,
            LlmError::Http { .. } | LlmError::Provider(_) | LlmError::NotImplemented(_) => InferenceFailureClass::Provider,
        };
        let _ = self.sink.try_emit(InferenceEvent::Terminal {
            run_id: request.run_id,
            outcome: TerminalOutcome::Failed { class, message: error.to_string() },
            at_unix_ms: now(),
            final_text: output.to_owned(),
            usage: TokenUsage::default(),
        });
    }
}

// Adapter parsing requirements, implemented in provider-specific modules:
//
// Ollama (`/api/generate`, `stream: true`): read response body incrementally by line;
// deserialize each NDJSON object, map `response` → TextDelta, and its terminal `done`
// record → Finished. At every body read, `tokio::select!` against `cancel.cancelled()`.
//
// Gemini (`streamGenerateContent?alt=sse`): parse SSE frames and map only display-safe
// candidate text parts to TextDelta. Never require Swift to parse provider framing.
//
// The adapter must always drop its `reqwest::Response`/body reader on cancellation;
// cancellation is a Kestrel terminal condition, not a request to wait for the model to
// finish computing server-side.
