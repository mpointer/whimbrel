//! M1.4 groundwork: provider-agnostic LLM router (docs/04-architecture.md).
//!
//! `OllamaClient`, `AnthropicClient`, and `OpenAIClient` are all real, network-calling
//! clients now (FR-CHAT-2). The core crate deliberately doesn't source API keys itself —
//! that's a platform concern (Keychain on macOS) — callers construct a client with a key
//! they already have.

use chrono::TimeZone;
use memory::{CaptureItem, MeetingSnapshotHit, MeetingTranscriptSegment, ThreadEmailMessage};
use serde::{Deserialize, Serialize};

/// One retrieved context item going into a chat prompt — either a screen-text capture
/// or a meeting transcript segment. Kept as a tagged enum (not a shared/blended struct)
/// specifically so citations can unambiguously reference which table a cited id came
/// from — `capture_item.id` and `transcript_segment.id` are both independent
/// `AUTOINCREMENT` sequences and can collide.
#[derive(Debug, Clone)]
pub enum ContextItem {
    Capture(CaptureItem),
    Transcript(MeetingTranscriptSegment),
    RoutineRun(memory::RoutineRunWithName),
    Episode(memory::Episode),
    Thread(memory::Thread),
    Snapshot(MeetingSnapshotHit),
}

impl ContextItem {
    pub fn display_label(&self) -> String {
        match self {
            ContextItem::Capture(c) => c.app_bundle_id.clone(),
            ContextItem::Transcript(t) => {
                format!("meeting \"{}\", {}", t.meeting_title, t.speaker_label)
            }
            ContextItem::RoutineRun(r) => format!(
                "routine \"{}\" (run at {})",
                r.routine_name, r.ran_at_unix_ms
            ),
            ContextItem::Episode(e) => format!(
                "episode \"{}\"",
                e.window_title.as_deref().unwrap_or(&e.app_bundle_id)
            ),
            ContextItem::Thread(t) => format!(
                "thread: {}",
                t.subject.as_deref().unwrap_or(&t.last_message_from)
            ),
            ContextItem::Snapshot(h) => format!("slide in meeting \"{}\"", h.meeting_title),
        }
    }

    pub fn text(&self) -> &str {
        match self {
            ContextItem::Capture(c) => &c.text,
            ContextItem::Transcript(t) => &t.text,
            ContextItem::RoutineRun(r) => &r.output_summary,
            ContextItem::Episode(e) => e.summary.as_deref().unwrap_or("(summary pending)"),
            ContextItem::Thread(t) => t.summary.as_deref().unwrap_or("(summary pending)"),
            ContextItem::Snapshot(h) => &h.text,
        }
    }

    pub fn citation_ref(&self) -> ContextItemRef {
        match self {
            ContextItem::Capture(c) => ContextItemRef::Capture { id: c.id },
            ContextItem::Transcript(t) => ContextItemRef::Transcript { id: t.id },
            ContextItem::RoutineRun(r) => ContextItemRef::RoutineRun { id: r.id },
            ContextItem::Episode(e) => ContextItemRef::Episode { id: e.id },
            ContextItem::Thread(t) => ContextItemRef::Thread { id: t.id },
            ContextItem::Snapshot(h) => ContextItemRef::Snapshot { id: h.id },
        }
    }
}

/// A citation pointing back to one retrieved context item, tagged with which table
/// (`capture_item` or `transcript_segment`) the id belongs to — see [`ContextItem`]'s
/// doc comment for why a bare `i64` isn't safe here. `PartialOrd`/`Ord` (variant
/// declaration order, then field order) give `assemble_context` a deterministic tiebreak
/// for score ties, independent of `HashMap` iteration order in `rrf_fuse`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextItemRef {
    Capture { id: i64 },
    Transcript { id: i64 },
    RoutineRun { id: i64 },
    Episode { id: i64 },
    Thread { id: i64 },
    Snapshot { id: i64 },
}

mod backoff;
mod ledger;
mod routing;
pub mod streaming;
pub mod meeting_ask;
pub mod meeting_prep;
pub mod typesafe;
pub mod profile;
pub mod skin_maker;
pub mod images;

pub use backoff::is_retryable;
pub use ledger::{
    charge_skin_call, check_spend_cap, ChargeError, SpendCapConfig, SpendCapError, SpendCapScope, DEFAULT_GLOBAL_DAILY_CAP_CENTS,
    DEFAULT_USER_DAILY_CAP_CENTS,
};
pub use routing::{falls_through, try_chain, ChainLink, Provider};

#[derive(Debug)]
pub enum LlmError {
    Network(String),
    Provider(String),
    /// A non-2xx HTTP response from a provider, carrying the status code (so
    /// `backoff::is_retryable` can classify 429/5xx) and the response body.
    Http {
        status: u16,
        body: String,
    },
    NotImplemented(&'static str),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Network(msg) => write!(f, "network error: {msg}"),
            LlmError::Provider(msg) => write!(f, "provider error: {msg}"),
            LlmError::Http { status, body, .. } => write!(f, "HTTP {status}: {body}"),
            LlmError::NotImplemented(msg) => write!(f, "not implemented: {msg}"),
        }
    }
}

impl std::error::Error for LlmError {}

impl LlmError {
    /// The local Ollama can't serve ANY job right now, which says nothing about whether
    /// this job can succeed: it isn't reachable (`Network`), or the configured model
    /// isn't pulled (Ollama answers `/api/generate` with a 404). Such a job should stay
    /// pending rather than burn its attempts to a permanent failure.
    pub fn is_environmental(&self) -> bool {
        matches!(self, LlmError::Network(_) | LlmError::Http { status: 404, .. })
    }
}

/// Token counts from one provider call, when the provider reports them. `None` for a
/// field the provider didn't report (or a mock/test client) — never coerced to `0`,
/// since `0` would be indistinguishable from "the call really used zero tokens."
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct TokenUsage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

/// One provider call's raw text plus whatever token usage it reported.
#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub token_usage: TokenUsage,
}

pub trait LlmClient {
    fn complete(&self, prompt: &str) -> Result<Completion, LlmError>;
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
}

const OLLAMA_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Non-streaming, so nothing arrives until generation finishes: this bounds a whole
/// generation, generous for a local 12B model on a long meeting transcript.
const OLLAMA_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Calls a locally-running Ollama server (NFR-PRV-1: local-first). Default endpoint is
/// Ollama's standard `http://localhost:11434`.
pub struct OllamaClient {
    base_url: String,
    model: String,
}

impl OllamaClient {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            base_url: "http://localhost:11434".to_string(),
            model: model.into(),
        }
    }

    pub fn with_base_url(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
        }
    }
}

impl LlmClient for OllamaClient {
    fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
        let request = OllamaGenerateRequest {
            model: &self.model,
            prompt,
            stream: false,
        };
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(OLLAMA_CONNECT_TIMEOUT)
            .timeout_read(OLLAMA_READ_TIMEOUT)
            .build();
        let response = agent
            .post(&format!("{}/api/generate", self.base_url))
            .send_json(&request)
            .map_err(|e| match e {
                // Ollama accepted the request and then stalled or dropped it: count it
                // against the job's attempts. `Network` (Ollama not running) retries forever.
                ureq::Error::Transport(t) if t.kind() == ureq::ErrorKind::Io => {
                    LlmError::Provider(format!("Ollama request failed mid-response: {t}"))
                }
                e => ureq_error_to_llm(e),
            })?;
        let parsed: OllamaGenerateResponse = response
            .into_json()
            .map_err(|e| LlmError::Provider(format!("failed to parse Ollama response: {e}")))?;
        Ok(Completion {
            text: parsed.response,
            token_usage: TokenUsage::default(),
        })
    }
}

/// Maps a transport failure to `Network` and a non-2xx HTTP response (auth failure, bad
/// request, rate limit, etc.) to `Http` — carrying the status code, response body, and
/// any server-supplied `Retry-After` delay so callers can actually tell "wrong API key"
/// apart from "server unreachable" and `backoff::is_retryable` can classify the status.
fn ureq_error_to_llm(e: ureq::Error) -> LlmError {
    match e {
        ureq::Error::Status(status, response) => {
            let body = response
                .into_string()
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            LlmError::Http { status, body }
        }
        ureq::Error::Transport(t) => LlmError::Network(t.to_string()),
    }
}

#[derive(Debug, Serialize)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<AnthropicMessage<'a>>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

/// Calls the real Anthropic Messages API (FR-CHAT-2). The caller supplies an API key it
/// already has — this crate never reads one from the environment or a config file, so
/// key storage (e.g. macOS Keychain) stays entirely the caller's concern.
pub struct AnthropicClient {
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicClient {
    /// `max_tokens` is a cap, not a target: Anthropic bills tokens actually generated.
    /// On Opus 5 and later, thinking is on by default and its tokens count against this
    /// ceiling, so a low cap truncates a long response (the weekly reflection, a full
    /// meeting summary) before the closing `}`. 16384 is the documented headroom for a
    /// non-streaming call; a truncated reply now surfaces as a `stop_reason` error
    /// instead of a JSON-parse failure.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.anthropic.com".to_string(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: 16384,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

impl LlmClient for AnthropicClient {
    fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
        let request = AnthropicRequest {
            model: &self.model,
            max_tokens: self.max_tokens,
            messages: vec![AnthropicMessage {
                role: "user",
                content: prompt,
            }],
        };
        let response = ureq::post(&format!("{}/v1/messages", self.base_url))
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .send_json(&request)
            .map_err(ureq_error_to_llm)?;
        let parsed: AnthropicResponse = response
            .into_json()
            .map_err(|e| LlmError::Provider(format!("failed to parse Anthropic response: {e}")))?;
        match parsed.stop_reason.as_deref() {
            Some("max_tokens") => {
                return Err(LlmError::Provider(format!(
                    "Anthropic response truncated at max_tokens={}",
                    self.max_tokens
                )))
            }
            Some("refusal") => {
                return Err(LlmError::Provider(
                    "Anthropic declined this request (stop_reason: refusal)".to_string(),
                ))
            }
            _ => {}
        }
        let token_usage = TokenUsage {
            input_tokens: parsed.usage.as_ref().map(|u| u.input_tokens),
            output_tokens: parsed.usage.as_ref().map(|u| u.output_tokens),
        };
        let text = parsed
            .content
            .into_iter()
            .find(|block| block.kind == "text")
            .map(|block| block.text)
            .ok_or_else(|| {
                LlmError::Provider("Anthropic response had no text content block".to_string())
            })?;
        Ok(Completion { text, token_usage })
    }
}

#[derive(Debug, Serialize)]
struct OpenAIMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct OpenAIRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAIMessage<'a>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoiceMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    #[serde(default)]
    usage: Option<OpenAIUsage>,
}

/// Calls the real OpenAI Chat Completions API (FR-CHAT-2). Same key-sourcing rule as
/// `AnthropicClient`: the caller passes a key it already has.
pub struct OpenAIClient {
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAIClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.openai.com".to_string(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

impl LlmClient for OpenAIClient {
    fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
        let request = OpenAIRequest {
            model: &self.model,
            messages: vec![OpenAIMessage {
                role: "user",
                content: prompt,
            }],
        };
        let response = ureq::post(&format!("{}/v1/chat/completions", self.base_url))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(&request)
            .map_err(ureq_error_to_llm)?;
        let mut parsed: OpenAIResponse = response
            .into_json()
            .map_err(|e| LlmError::Provider(format!("failed to parse OpenAI response: {e}")))?;
        if parsed.choices.is_empty() {
            return Err(LlmError::Provider(
                "OpenAI response had no choices".to_string(),
            ));
        }
        let token_usage = TokenUsage {
            input_tokens: parsed.usage.as_ref().map(|u| u.prompt_tokens),
            output_tokens: parsed.usage.as_ref().map(|u| u.completion_tokens),
        };
        let text = parsed.choices.remove(0).message.content;
        Ok(Completion { text, token_usage })
    }
}

#[derive(Debug, Serialize)]
struct GooglePart<'a> {
    text: &'a str,
}

#[derive(Debug, Serialize)]
struct GoogleContent<'a> {
    parts: Vec<GooglePart<'a>>,
}

#[derive(Debug, Serialize)]
struct GoogleRequest<'a> {
    contents: Vec<GoogleContent<'a>>,
}

#[derive(Debug, Deserialize)]
struct GoogleResponsePart {
    text: String,
}

#[derive(Debug, Deserialize, Default)]
struct GoogleResponseContent {
    #[serde(default)]
    parts: Vec<GoogleResponsePart>,
}

#[derive(Debug, Deserialize)]
struct GoogleCandidate {
    #[serde(default)]
    content: GoogleResponseContent,
    /// Present only on the final chunk of a `streamGenerateContent` SSE stream (e.g.
    /// `"STOP"`); the blocking path ignores it.
    #[serde(rename = "finishReason", default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: u32,
}

#[derive(Debug, Deserialize)]
struct GoogleResponse {
    #[serde(default)]
    candidates: Vec<GoogleCandidate>,
    #[serde(rename = "usageMetadata", default)]
    usage_metadata: Option<GoogleUsageMetadata>,
}

/// Calls the real Google Gemini `generateContent` API (FR-CHAT-2's third hosted
/// provider). Same key-sourcing rule as `AnthropicClient`/`OpenAIClient`: the caller
/// passes a key it already has. Unlike those two, Gemini takes its key as a `?key=`
/// query parameter rather than an auth header — that's Google's own API shape, not a
/// stylistic choice here.
pub struct GoogleClient {
    base_url: String,
    api_key: String,
    model: String,
}

impl GoogleClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

impl LlmClient for GoogleClient {
    fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
        let request = GoogleRequest {
            contents: vec![GoogleContent {
                parts: vec![GooglePart { text: prompt }],
            }],
        };
        let response = ureq::post(&format!(
            "{}/v1beta/models/{}:generateContent",
            self.base_url, self.model
        ))
        .query("key", &self.api_key)
        .send_json(&request)
        .map_err(ureq_error_to_llm)?;
        let parsed: GoogleResponse = response
            .into_json()
            .map_err(|e| LlmError::Provider(format!("failed to parse Google response: {e}")))?;
        let token_usage = TokenUsage {
            input_tokens: parsed.usage_metadata.as_ref().map(|u| u.prompt_token_count),
            output_tokens: parsed
                .usage_metadata
                .as_ref()
                .map(|u| u.candidates_token_count),
        };
        let text = parsed
            .candidates
            .into_iter()
            .next()
            .and_then(|c| c.content.parts.into_iter().next())
            .map(|p| p.text)
            .ok_or_else(|| {
                LlmError::Provider("Google response had no candidate text".to_string())
            })?;
        Ok(Completion { text, token_usage })
    }
}

/// FR-CHAT-1: an answer grounded in retrieval, with citations linking claims back to
/// specific timeline items.
#[derive(Debug, Clone, Serialize)]
pub struct ChatAnswer {
    pub text: String,
    /// Context items (capture or transcript) the answer actually cited (via `[n]`
    /// markers), in the order first referenced. Retrieval itself (deciding which items
    /// go into `context`) is the caller's job — e.g. `memory::Store::search_keyword`/
    /// `search_semantic`/`search_transcript_keyword`/`search_transcript_semantic`.
    pub citations: Vec<ContextItemRef>,
    #[serde(default)]
    pub token_usage: TokenUsage,
}

/// One turn of prior conversation, oldest first. FR-CHAT: multi-turn chat — lets a
/// follow-up question ("what about the other one?") be answered with the earlier
/// exchange in view. Note this only feeds history to the *model*; retrieval of `context`
/// for the current turn is still based solely on the latest query (the caller's job, as
/// with the single-turn case) — a follow-up whose retrieval-relevant keywords are only
/// in an earlier turn may not pull in the right context items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: ChatRole,
    pub text: String,
}

fn tau_ms_for(item: &ContextItem) -> i64 {
    match item {
        ContextItem::Capture(_) => RECENCY_TAU_CAPTURE_MS,
        ContextItem::Transcript(_) | ContextItem::Snapshot(_) => RECENCY_TAU_TRANSCRIPT_MS,
        ContextItem::RoutineRun(_) => RECENCY_TAU_ROUTINE_RUN_MS,
        // assemble_context never puts Episode/Thread through this path (see step 12) --
        // they use their own dedicated decay scaling instead of tau_ms_for.
        ContextItem::Episode(_) | ContextItem::Thread(_) => {
            unreachable!("episodes/threads are decay-scored separately, never RRF-fused")
        }
    }
}

fn decayed_score(relevance: f64, ts_unix_ms: i64, now_unix_ms: i64, tau_ms: i64) -> f64 {
    let age_ms = (now_unix_ms - ts_unix_ms).max(0) as f64;
    relevance * (-age_ms / tau_ms as f64).exp()
}

/// Reciprocal rank fusion over any number of already-ranked input lists (rank 1 =
/// best in each list). Returns (item, ts_unix_ms, fused_rrf_score), deduplicated by
/// ContextItemRef identity -- an item present in multiple lists sums its per-list
/// reciprocal-rank contributions.
fn rrf_fuse(lists: &[&[(ContextItem, i64)]]) -> Vec<(ContextItem, i64, f64)> {
    let mut scores: std::collections::HashMap<ContextItemRef, (ContextItem, i64, f64)> =
        std::collections::HashMap::new();
    for list in lists {
        for (rank, (item, ts)) in list.iter().enumerate() {
            let rank = rank + 1; // 1-indexed
            let contribution = 1.0 / (RRF_K + rank as f64);
            let key = item.citation_ref();
            scores
                .entry(key)
                .and_modify(|(_, _, score)| *score += contribution)
                .or_insert_with(|| (item.clone(), *ts, contribution));
        }
    }
    scores.into_values().collect()
}

pub fn assemble_context(
    query: &str,
    now_unix_ms: i64,
    keyword_hits: &[(ContextItem, i64)],
    semantic_hits: &[(ContextItem, i64)],
    routine_runs: &[(ContextItem, i64)],
    episodes: &[memory::Episode],
    threads: &[memory::Thread],
) -> Vec<ContextItem> {
    let _ = query; // reserved for future L2 semantic filtering (a later slice); unused this slice

    let mut l1: Vec<(ContextItem, f64)> = rrf_fuse(&[keyword_hits, semantic_hits, routine_runs])
        .into_iter()
        .map(|(item, ts, rrf_score)| {
            let tau = tau_ms_for(&item);
            (item, decayed_score(rrf_score, ts, now_unix_ms, tau))
        })
        .collect();
    // Score ties broken by ContextItemRef ascending -- HashMap iteration order in
    // rrf_fuse is otherwise unspecified, and sort_by is stable, so without this the
    // output order on a tie would depend on that unspecified order.
    l1.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.citation_ref().cmp(&b.0.citation_ref()))
    });

    let mut l2: Vec<(ContextItem, f64)> = Vec::new();
    l2.extend(episodes.iter().map(|e| {
        (
            ContextItem::Episode(e.clone()),
            decayed_score(1.0, e.end_ts_unix_ms, now_unix_ms, RECENCY_TAU_EPISODE_MS),
        )
    }));
    l2.extend(threads.iter().map(|t| {
        (
            ContextItem::Thread(t.clone()),
            decayed_score(
                1.0,
                t.last_activity_ts_unix_ms,
                now_unix_ms,
                RECENCY_TAU_THREAD_MS,
            ),
        )
    }));
    l2.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.citation_ref().cmp(&b.0.citation_ref()))
    });

    // The question's own matches go first: recent episodes and threads are background,
    // and packing them first filled the budget and dropped answers that were found.
    // An item that doesn't fit is skipped, not the end of packing.
    let budget_chars = CONTEXT_BUDGET_TOKENS * CHARS_PER_TOKEN_ESTIMATE;
    let mut remaining_chars = budget_chars;
    let mut out = Vec::new();
    for (item, _score) in l1.into_iter().chain(l2.into_iter()) {
        let len = item.text().len();
        if len > remaining_chars {
            continue;
        }
        remaining_chars -= len;
        out.push(item);
    }
    out
}

/// Fetches every input `assemble_context` needs directly from `store`, then calls it.
/// Shared by `whimbrel_llm_answer` (core/ffi) and the `assemble_context` MCP tool
/// (core/mcp-server) so this fetch sequence exists in exactly one place.
///
/// `include_transcripts` gates the two transcript-sourced legs (meeting speech is more
/// sensitive than screen text) — callers without transcript access pass `false` to get
/// capture/routine-run/episode/thread context only, mirroring `search_memory`'s existing
/// scope-conditional pattern rather than hard-failing the whole call.
pub fn gather_and_assemble_context(
    store: &memory::Store,
    query: &str,
    now_unix_ms: i64,
    context_limit: usize,
    include_transcripts: bool,
) -> Vec<ContextItem> {
    let mut keyword_hits: Vec<(ContextItem, i64)> = Vec::new();
    keyword_hits.extend(
        store
            .search_keyword(query, context_limit)
            .unwrap_or_default()
            .into_iter()
            .map(|c| {
                let ts = c.ts_unix_ms;
                (ContextItem::Capture(c), ts)
            }),
    );
    if include_transcripts {
        keyword_hits.extend(
            store
                .search_transcript_keyword_with_meeting_ts(query, context_limit)
                .unwrap_or_default()
                .into_iter()
                .map(|(t, ts)| (ContextItem::Transcript(t), ts)),
        );
        // Slides are meeting content, so they ride the same gate as transcripts.
        keyword_hits.extend(
            store
                .search_snapshot_keyword_with_meeting_ts(query, context_limit)
                .unwrap_or_default()
                .into_iter()
                .map(|(h, ts)| (ContextItem::Snapshot(h), ts)),
        );
    }

    let mut semantic_hits: Vec<(ContextItem, i64)> = Vec::new();
    semantic_hits.extend(
        store
            .search_semantic(query, context_limit)
            .unwrap_or_default()
            .into_iter()
            .map(|c| {
                let ts = c.ts_unix_ms;
                (ContextItem::Capture(c), ts)
            }),
    );
    if include_transcripts {
        semantic_hits.extend(
            store
                .search_transcript_semantic_with_meeting_ts(query, context_limit)
                .unwrap_or_default()
                .into_iter()
                .map(|(t, ts)| (ContextItem::Transcript(t), ts)),
        );
        semantic_hits.extend(
            store
                .search_snapshot_semantic_with_meeting_ts(query, context_limit)
                .unwrap_or_default()
                .into_iter()
                .map(|(h, ts)| (ContextItem::Snapshot(h), ts)),
        );
    }

    let routine_runs: Vec<(ContextItem, i64)> = store
        .recent_routine_runs_across_all(context_limit)
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let ts = r.ran_at_unix_ms;
            (ContextItem::RoutineRun(r), ts)
        })
        .collect();

    let episodes = store.recent_episodes(10).unwrap_or_default();
    let threads = store.threads_needing_attention(10).unwrap_or_default();

    assemble_context(
        query,
        now_unix_ms,
        &keyword_hits,
        &semantic_hits,
        &routine_runs,
        &episodes,
        &threads,
    )
}

/// Time-boxed counterpart to `gather_and_assemble_context` -- every leg bounded to
/// `[start_unix_ms, end_unix_ms)` instead of "most recent N" / unbounded search. Built
/// for routines (`run_daily_brief`/`run_eod_recap`), which need a strict day boundary
/// `gather_and_assemble_context` doesn't provide.
///
/// `query` is almost always empty for a routine (no natural search question) -- when
/// empty, the keyword/semantic search legs are skipped entirely (an empty-string FTS5
/// query is a real SQL error, not empty results -- confirmed empirically) and capture
/// items instead come from `timeline_range` directly, feeding L1 as their own
/// ranked-by-recency source. A non-empty `query` additionally runs keyword/semantic
/// search bounded to the window, for future callers that do have a real question (not
/// used by this slice).
///
/// Output is budget-bounded: like `gather_and_assemble_context`, this runs everything
/// through `assemble_context`'s `CONTEXT_BUDGET_TOKENS` packing (the same budget chat
/// uses), capping the digest's input -- unlike the old digest path, which fed
/// `timeline_range`'s full, unbounded result straight into the prompt.
pub fn gather_and_assemble_context_for_window(
    store: &memory::Store,
    query: &str,
    now_unix_ms: i64,
    start_unix_ms: i64,
    end_unix_ms: i64,
    context_limit: usize,
    include_transcripts: bool,
) -> Vec<ContextItem> {
    let mut keyword_hits: Vec<(ContextItem, i64)> = Vec::new();
    let mut semantic_hits: Vec<(ContextItem, i64)> = Vec::new();

    if query.trim().is_empty() {
        keyword_hits.extend(
            store
                .timeline_range(start_unix_ms, end_unix_ms)
                .unwrap_or_default()
                .into_iter()
                .rev() // timeline_range is ASC (oldest first); rrf_fuse treats list position as
                // rank (position 0 = highest RRF contribution), so reverse to make the
                // newest item rank first -- aligning RRF with recency decay instead of
                // fighting it. Without this, a realistic day's worth of captures (200+)
                // causes RRF's ranking spread to dominate decay, and budget packing keeps
                // only the day's earliest hour or two.
                .map(|c| {
                    let ts = c.ts_unix_ms;
                    (ContextItem::Capture(c), ts)
                }),
        );
        if include_transcripts {
            // Own list (semantic_hits, unused otherwise on this branch) rather than
            // appended to keyword_hits: rrf_fuse ranks by position within a list, so
            // appending after 200+ captures would put every transcript at rank 200+,
            // starving it regardless of recency. As its own list, the newest transcript
            // gets rank 1, same as the newest capture.
            semantic_hits.extend(
                store
                    .transcript_segments_in_window(start_unix_ms, end_unix_ms)
                    .unwrap_or_default()
                    .into_iter()
                    .rev() // same rank-inversion fix as the capture-item leg above
                    .map(|(t, ts)| (ContextItem::Transcript(t), ts)),
            );
        }
    } else {
        keyword_hits.extend(
            store
                .search_keyword(query, context_limit)
                .unwrap_or_default()
                .into_iter()
                .filter(|c| c.ts_unix_ms >= start_unix_ms && c.ts_unix_ms < end_unix_ms)
                .map(|c| {
                    let ts = c.ts_unix_ms;
                    (ContextItem::Capture(c), ts)
                }),
        );
        semantic_hits.extend(
            store
                .search_semantic(query, context_limit)
                .unwrap_or_default()
                .into_iter()
                .filter(|c| c.ts_unix_ms >= start_unix_ms && c.ts_unix_ms < end_unix_ms)
                .map(|c| {
                    let ts = c.ts_unix_ms;
                    (ContextItem::Capture(c), ts)
                }),
        );
        if include_transcripts {
            keyword_hits.extend(
                store
                    .search_transcript_keyword_with_meeting_ts(query, context_limit)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(_, ts)| *ts >= start_unix_ms && *ts < end_unix_ms)
                    .map(|(t, ts)| (ContextItem::Transcript(t), ts)),
            );
            semantic_hits.extend(
                store
                    .search_transcript_semantic_with_meeting_ts(query, context_limit)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(_, ts)| *ts >= start_unix_ms && *ts < end_unix_ms)
                    .map(|(t, ts)| (ContextItem::Transcript(t), ts)),
            );
        }
    }

    let routine_runs: Vec<(ContextItem, i64)> = store
        .routine_runs_in_window(start_unix_ms, end_unix_ms, context_limit)
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let ts = r.ran_at_unix_ms;
            (ContextItem::RoutineRun(r), ts)
        })
        .collect();

    let episodes = store
        .episodes_in_window(start_unix_ms, end_unix_ms, 10)
        .unwrap_or_default();
    let threads = store
        .threads_needing_attention_in_window(start_unix_ms, end_unix_ms, 10)
        .unwrap_or_default();

    assemble_context(
        query,
        now_unix_ms,
        &keyword_hits,
        &semantic_hits,
        &routine_runs,
        &episodes,
        &threads,
    )
}

/// Chat queries like "what did I do today" name a day, but `gather_and_assemble_context`
/// (used for every chat query) does generic recency-weighted retrieval with no day
/// boundary — so "today" can silently pull in yesterday's or the day before's items too,
/// and the model has no signal that it should have refused to use them. Deliberately just
/// this one keyword check, not real NLP date parsing -- see `detect_date_scope`'s callers
/// for how it's used (switches retrieval to `gather_and_assemble_context_for_window` and
/// adds a framing instruction, the same fix `generate_daily_digest_from_context` already
/// has for the same problem).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryDateScope {
    Today,
    Yesterday,
}

/// `None` means no date scope was named -- caller keeps using the unbounded, generic
/// retrieval. Checks "yesterday" first so "today vs yesterday" (naming both) resolves to
/// the less-ambiguous, explicitly-named-second word rather than silently picking one.
pub fn detect_date_scope(query: &str) -> Option<QueryDateScope> {
    let q = query.to_lowercase();
    if q.contains("yesterday") {
        Some(QueryDateScope::Yesterday)
    } else if q.contains("today") {
        Some(QueryDateScope::Today)
    } else {
        None
    }
}

/// `[start, end)` UTC-ms bounds for `scope` relative to `now_unix_ms`, in the local
/// timezone -- the same midnight-anchored convention `whimbrel_routines_tick` already uses
/// for EodRecap ("today") and DailyBrief ("yesterday").
pub fn date_scope_bounds(scope: QueryDateScope, now_unix_ms: i64) -> (i64, i64) {
    use chrono::TimeZone;
    let now_local = chrono::Local
        .timestamp_millis_opt(now_unix_ms)
        .single()
        .unwrap_or_else(chrono::Local::now);
    let today_midnight = now_local
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|dt| now_local.timezone().from_local_datetime(&dt).single())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(now_unix_ms);
    match scope {
        QueryDateScope::Today => (today_midnight, now_unix_ms),
        QueryDateScope::Yesterday => (today_midnight - 24 * 60 * 60 * 1000, today_midnight),
    }
}

/// A human-readable label for `answer_with_citations`'s date-framing instruction, e.g.
/// "today, Thursday, August 27, 2026". `window_start_unix_ms` is `date_scope_bounds`'s
/// own `start` for that scope (always a local midnight), so formatting it names the
/// actual calendar date regardless of what moment "now" is.
pub fn format_date_scope_label(scope: QueryDateScope, window_start_unix_ms: i64) -> String {
    use chrono::TimeZone;
    let word = match scope {
        QueryDateScope::Today => "today",
        QueryDateScope::Yesterday => "yesterday",
    };
    match chrono::Local
        .timestamp_millis_opt(window_start_unix_ms)
        .single()
    {
        Some(dt) => format!("{word}, {}", dt.format("%A, %B %-d, %Y")),
        None => word.to_string(),
    }
}

/// Builds a numbered-context prompt from retrieved items plus any prior conversation
/// turns, asks the model to answer using only that context and cite item numbers in
/// brackets (`[1]`, `[2]`, ...), then maps cited numbers back to real context item
/// references. Retrieval quality is bounded by whatever search produced `context` (still
/// the placeholder embedding for semantic search — see `memory`'s module docs); this
/// function only handles synthesis + citation extraction. Pass an empty `history` for a
/// fresh conversation. `custom_instructions`, when non-empty, is inserted as an `"Additional instructions: ..."` block immediately after the citation-format instructions.
/// `date_scope_label` (from `format_date_scope_label`), when `Some`, adds a framing
/// instruction restricting the model to items that actually describe that day -- pass
/// `None` for a query that didn't name a specific day.
pub fn answer_with_citations(
    client: &dyn LlmClient,
    history: &[ChatTurn],
    query: &str,
    context: &[ContextItem],
    custom_instructions: &str,
    date_scope_label: Option<&str>,
) -> Result<ChatAnswer, LlmError> {
    let prompt = build_ask_prompt(
        history,
        query,
        context,
        custom_instructions,
        date_scope_label,
    );
    let completion = client.complete(&prompt)?;
    let citations = extract_citations(&completion.text, context);
    Ok(ChatAnswer {
        text: completion.text,
        citations,
        token_usage: completion.token_usage,
    })
}

/// The prompt-assembly half of `answer_with_citations`, pulled out so a streaming
/// caller (`whimbrel_llm_prepare_ask`) can build the exact same prompt without also
/// making the blocking `client.complete()` call — the two are cleanly separable
/// because citation extraction runs against the model's OUTPUT text after the fact
/// (see `extract_citations`), not anything computed during assembly.
pub fn build_ask_prompt(
    history: &[ChatTurn],
    query: &str,
    context: &[ContextItem],
    custom_instructions: &str,
    date_scope_label: Option<&str>,
) -> String {
    let instructions_block = if custom_instructions.is_empty() {
        String::new()
    } else {
        format!("Additional instructions: {custom_instructions}\n\n")
    };
    let mut prompt = String::from(
        "Answer the question using ONLY the numbered context items below. \
         Cite the item numbers you relied on in brackets, like [1] or [2][3]. \
         If the context doesn't contain the answer, say so plainly.\n\n",
    );
    if let Some(date_scope_label) = date_scope_label {
        prompt.push_str(&format!(
            "The question is about {date_scope_label}. Some items below (rolling thread \
             summaries especially) may describe activity from OTHER days too -- only use \
             items that actually describe {date_scope_label} in your answer; ignore \
             anything describing a different day, even if it appears inside an item's own \
             text.\n\n"
        ));
    }
    prompt.push_str(&instructions_block);
    for (i, item) in context.iter().enumerate() {
        prompt.push_str(&format!(
            "[{}] ({}) {}\n",
            i + 1,
            item.display_label(),
            item.text()
        ));
    }
    if !history.is_empty() {
        prompt.push_str("\nConversation so far:\n");
        for turn in history {
            let speaker = match turn.role {
                ChatRole::User => "User",
                ChatRole::Assistant => "Assistant",
            };
            prompt.push_str(&format!("{speaker}: {}\n", turn.text));
        }
    }
    prompt.push_str(&format!("\nQuestion: {query}\nAnswer:"));
    prompt
}

/// Parses `[n]`-style citation markers out of model output and maps them back to the
/// context item reference at that (1-based) context position. Out-of-range or malformed
/// markers are silently ignored; duplicates are collapsed, first-occurrence order kept.
pub fn extract_citations(text: &str, context: &[ContextItem]) -> Vec<ContextItemRef> {
    let mut citations = Vec::new();
    for part in text.split('[').skip(1) {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        if !part[digits.len()..].starts_with(']') {
            continue;
        }
        if let Ok(idx) = digits.parse::<usize>() {
            if idx >= 1 && idx <= context.len() {
                let citation_ref = context[idx - 1].citation_ref();
                if !citations.contains(&citation_ref) {
                    citations.push(citation_ref);
                }
            }
        }
    }
    citations
}

/// The title to give a placeholder-titled meeting: the model's own short `title` when it
/// gave a usable one, otherwise the first sentence of the TL;DR.
pub fn meeting_title(summary: &MeetingSummary) -> String {
    let title = summary.title.trim().trim_matches(|c| c == '"' || c == '\'' || c == '.').trim();
    if !title.is_empty() && title.chars().count() <= 70 {
        title.to_string()
    } else {
        derive_short_title_from_tldr(&summary.tldr)
    }
}

/// #65: derives a short meeting title from a generated `MeetingSummary.tldr`, for
/// retitling a manually-started meeting (which starts with the literal placeholder
/// title "Meeting" — see `memory::Store::set_meeting_title_if_placeholder`). No second
/// LLM call — the tldr is already a real sentence, so this is just the first sentence
/// of it, truncated to a title-appropriate length. Never returns an empty string for
/// non-empty input.
pub fn derive_short_title_from_tldr(tldr: &str) -> String {
    const MAX_LEN: usize = 70;
    let trimmed = tldr.trim();
    let first_sentence = trimmed
        .split(['.', '!', '?', '\n'])
        .next()
        .unwrap_or(trimmed)
        .trim();
    let candidate = if first_sentence.is_empty() {
        trimmed
    } else {
        first_sentence
    };
    if candidate.chars().count() <= MAX_LEN {
        candidate.to_string()
    } else {
        let truncated: String = candidate.chars().take(MAX_LEN).collect();
        format!("{}…", truncated.trim_end())
    }
}

/// FR-MTG-5: Granola-style blend of the user's rough notes with a meeting transcript
/// into a structured summary. Takes plain text so it has no structural dependency on
/// `transcribe` — the caller (or a test harness) is responsible for producing
/// `transcript` however it likes (e.g. `transcribe::Transcriber`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetingSummary {
    /// A short topic title (3-6 words) for retitling a placeholder-titled meeting; empty
    /// when the model didn't provide one.
    #[serde(default)]
    pub title: String,
    pub tldr: String,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub action_items: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub token_usage: TokenUsage,
}

/// Conservative token-budget constants for splitting a long transcript before sending
/// it to the model. There is no way to get an exact token count without a
/// provider/model-specific tokenizer, so this uses a conservative chars-per-token
/// estimate and assumes a small (4096-token) context window as the safe universal
/// floor — better to split more aggressively than necessary (a few extra, cheaper
/// chunk calls) than to risk exceeding a real context window and getting an empty
/// response back, which is exactly what happens when llama.cpp truncates a prompt and
/// immediately hits its length limit before generating any output.
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
const MAX_CONTEXT_TOKENS_ESTIMATE: usize = 4096;
const RESPONSE_AND_OVERHEAD_TOKENS_ESTIMATE: usize = 900;

/// assemble_context (docs/superpowers/specs/2026-08-15-assemble-context-design.md): the
/// standard reciprocal-rank-fusion constant. Chosen assuming 1-indexed ranks (first
/// result in a list has rank 1) -- a 0-indexed implementation would silently over-weight
/// top results relative to this constant.
const RRF_K: f64 = 60.0;

/// Recency-decay time constants per ContextItem kind, in milliseconds. Each item's RRF
/// (or, for episodes/threads, flat 1.0) relevance score is scaled by
/// exp(-age_ms / tau_ms) -- doc 09 section 6's `relevance × exp(-age/τ)`.
/// Captures and routine runs: 7 days. At 1 day a week-old exact match scored e^-7 and
/// sank below any fresh weak match.
const RECENCY_TAU_CAPTURE_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const RECENCY_TAU_TRANSCRIPT_MS: i64 = 3 * 24 * 60 * 60 * 1000;
const RECENCY_TAU_EPISODE_MS: i64 = 3 * 24 * 60 * 60 * 1000;
const RECENCY_TAU_THREAD_MS: i64 = 14 * 24 * 60 * 60 * 1000;
const RECENCY_TAU_ROUTINE_RUN_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Token budget for assemble_context's packed context output specifically -- not the
/// whole prompt. Leaves roughly (MAX_CONTEXT_TOKENS_ESTIMATE - this) of the existing
/// 4096-token conservative floor for the system prompt, conversation history, the
/// query, and the response.
const CONTEXT_BUDGET_TOKENS: usize = 1500;

pub fn max_transcript_chars_per_chunk() -> usize {
    MAX_CONTEXT_TOKENS_ESTIMATE
        .saturating_sub(RESPONSE_AND_OVERHEAD_TOKENS_ESTIMATE)
        .saturating_mul(CHARS_PER_TOKEN_ESTIMATE)
}

/// Splits `transcript` into chunks no larger than `max_chars`, breaking only at line
/// boundaries (never mid-line) so each chunk stays a coherent set of
/// `[timestamp] Speaker: text` lines rather than a truncated sentence. Returns a single
/// chunk (the whole transcript, unchanged) when it already fits.
pub fn chunk_transcript(transcript: &str, max_chars: usize) -> Vec<String> {
    if transcript.len() <= max_chars {
        return vec![transcript.to_string()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in transcript.lines() {
        if !current.is_empty() && current.len() + line.len() + 1 > max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Adds two `TokenUsage`s field-by-field. `None + None = None`; `None` on one side with
/// `Some` on the other keeps the known value rather than discarding it; `Some + Some`
/// sums. This never coerces an unknown value to `0`, matching `TokenUsage`'s own
/// documented invariant.
pub fn add_token_usage(a: TokenUsage, b: TokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: match (a.input_tokens, b.input_tokens) {
            (None, None) => None,
            (Some(x), None) | (None, Some(x)) => Some(x),
            (Some(x), Some(y)) => Some(x + y),
        },
        output_tokens: match (a.output_tokens, b.output_tokens) {
            (None, None) => None,
            (Some(x), None) | (None, Some(x)) => Some(x),
            (Some(x), Some(y)) => Some(x + y),
        },
    }
}

/// Max chars of slide OCR text passed into a meeting-summary prompt. Small enough to sit
/// beside a whole transcript or the partial summaries within the local model's 4096-token
/// budget; chunk prompts carry no slide text at all (they are sized to fill that budget).
pub const MAX_SLIDE_CHARS: usize = 3_000;

/// Trims `slides` and truncates it to `MAX_SLIDE_CHARS` chars, on a char boundary.
pub fn cap_slides(slides: &str) -> &str {
    let t = slides.trim();
    match t.char_indices().nth(MAX_SLIDE_CHARS) {
        Some((i, _)) => &t[..i],
        None => t,
    }
}

/// The prompt section for slide text, inserted right after the notes; empty (so the
/// prompt is byte-identical to the no-slides wording) when there is no slide text.
fn slides_prompt_block(slides: &str) -> String {
    let slides = cap_slides(slides);
    if slides.is_empty() {
        String::new()
    } else {
        format!("\n\nText from slides shown in the meeting (OCR, may be imperfect):\n{slides}")
    }
}

/// Summarizes one chunk of a (possibly split) transcript. When `chunk_count > 1`, the
/// prompt tells the model this is a partial excerpt, so it doesn't treat a missing
/// beginning/end as its own error. No slide text here: a chunk already fills the token
/// budget, so slides go to the reduce step instead.
pub fn summarize_chunk(
    client: &dyn LlmClient,
    chunk_index: usize,
    chunk_count: usize,
    notes: &str,
    chunk: &str,
    custom_instructions: &str,
) -> Result<(MeetingSummary, TokenUsage), LlmError> {
    let context_note = if chunk_count > 1 {
        format!(
            "This is part {n} of {total} of a longer meeting transcript, split into parts \
             for length — it may start or end mid-conversation.\n\n",
            n = chunk_index + 1,
            total = chunk_count,
        )
    } else {
        String::new()
    };
    let instructions_block = if custom_instructions.is_empty() {
        String::new()
    } else {
        format!("Additional instructions: {custom_instructions}\n\n")
    };
    let prompt = format!(
        "You are summarizing part of a meeting by blending the user's rough notes with \
         the transcript excerpt below. Respond with ONLY a JSON object, no other text, \
         no markdown fences, matching exactly this shape:\n\
         {{\"tldr\": \"...\", \"decisions\": [\"...\"], \"action_items\": [\"...\"], \"open_questions\": [\"...\"]}}\n\n\
         {context_note}{instructions_block}User's notes:\n{notes}\n\n\
         Transcript excerpt:\n{chunk}\n"
    );
    let completion = client.complete(&prompt)?;
    let summary = parse_meeting_summary(&completion.text)?;
    Ok((summary, completion.token_usage))
}

/// Combines multiple partial summaries (from `summarize_chunk`) into one final summary
/// via one more LLM call. The partial summaries are far shorter than the raw
/// transcript, so this reduce step comfortably fits in context even when the original
/// transcript didn't.
pub fn reduce_chunk_summaries(
    client: &dyn LlmClient,
    notes: &str,
    partials: &[MeetingSummary],
    slides: &str,
    custom_instructions: &str,
) -> Result<(MeetingSummary, TokenUsage), LlmError> {
    let combined = partials
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                "Part {n}:\ntldr: {tldr}\ndecisions: {decisions:?}\naction_items: {action_items:?}\nopen_questions: {open_questions:?}",
                n = i + 1,
                tldr = s.tldr,
                decisions = s.decisions,
                action_items = s.action_items,
                open_questions = s.open_questions,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let instructions_block = if custom_instructions.is_empty() {
        String::new()
    } else {
        format!("Additional instructions: {custom_instructions}\n\n")
    };
    let slides_block = slides_prompt_block(slides);
    let prompt = format!(
        "You are combining several partial summaries of different parts of the SAME \
         meeting into one final summary. Merge and deduplicate — do not just \
         concatenate. Respond with ONLY a JSON object, no other text, no markdown \
         fences, matching exactly this shape:\n\
         {{\"title\": \"...\", \"tldr\": \"...\", \"decisions\": [\"...\"], \"action_items\": [\"...\"], \"open_questions\": [\"...\"]}}\n\n\
         \"title\" names the meeting's main topic in 3 to 6 words, like a calendar event \
         title (e.g. \"Q3 roadmap review\"), not a sentence.\n\n\
         {instructions_block}User's notes:\n{notes}{slides_block}\n\n\
         Partial summaries:\n{combined}\n"
    );
    let completion = client.complete(&prompt)?;
    let summary = parse_meeting_summary(&completion.text)?;
    Ok((summary, completion.token_usage))
}

/// The single-call path for a transcript that fits entirely within budget — used both
/// by `summarize_meeting` directly and by `core/ffi`'s governed orchestrator (which
/// needs cap-checking between calls, not the map-reduce loop this delegates to for a
/// longer transcript). Exactly one copy of this prompt's wording exists so the two
/// callers can never silently drift apart.
pub fn summarize_whole_transcript(
    client: &dyn LlmClient,
    notes: &str,
    transcript: &str,
    slides: &str,
    custom_instructions: &str,
) -> Result<MeetingSummary, LlmError> {
    let instructions_block = if custom_instructions.is_empty() {
        String::new()
    } else {
        format!("Additional instructions: {custom_instructions}\n\n")
    };
    let slides_block = slides_prompt_block(slides);
    let prompt = format!(
        "You are summarizing a meeting by blending the user's rough notes with the \
         transcript below. Respond with ONLY a JSON object, no other text, no markdown \
         fences, matching exactly this shape:\n\
         {{\"title\": \"...\", \"tldr\": \"...\", \"decisions\": [\"...\"], \"action_items\": [\"...\"], \"open_questions\": [\"...\"]}}\n\n\
         \"title\" names the meeting's main topic in 3 to 6 words, like a calendar event \
         title (e.g. \"Q3 roadmap review\"), not a sentence.\n\n\
         {instructions_block}User's notes:\n{notes}{slides_block}\n\n\
         Transcript:\n{transcript}\n"
    );
    let completion = client.complete(&prompt)?;
    let mut summary = parse_meeting_summary(&completion.text)?;
    summary.token_usage = completion.token_usage;
    Ok(summary)
}

/// FR-MTG-5: blends the user's rough notes with the transcript into one summary. Long
/// transcripts are split into chunks that fit a conservative token budget (map),
/// summarized independently, then combined into one final summary via one more LLM
/// call (reduce) — see `chunk_transcript`'s doc comment for why. Short transcripts (the
/// common case) take a single call, identical to this function's original behavior.
/// `core/ffi::summarize_meeting_governed` is a spend-cap-checked, incrementally-logged
/// duplicate of this function's chunking/dispatch control flow (a deliberate tradeoff,
/// not an oversight — see that function's doc comment); if this function's
/// chunking/dispatch logic changes, that copy must be kept in sync too.
pub fn summarize_meeting(
    client: &dyn LlmClient,
    notes: &str,
    transcript: &str,
    slides: &str,
    custom_instructions: &str,
) -> Result<MeetingSummary, LlmError> {
    let max_chars = max_transcript_chars_per_chunk();
    let chunks = chunk_transcript(transcript, max_chars);

    if chunks.len() == 1 && chunks[0].len() <= max_chars {
        return summarize_whole_transcript(client, notes, &chunks[0], slides, custom_instructions);
    }

    let mut partials = Vec::with_capacity(chunks.len());
    let mut total_usage = TokenUsage::default();
    for (i, chunk) in chunks.iter().enumerate() {
        let (summary, usage) = summarize_chunk(
            client,
            i,
            chunks.len(),
            notes,
            chunk,
            custom_instructions,
        )?;
        total_usage = add_token_usage(total_usage, usage);
        partials.push(summary);
    }
    let (mut summary, reduce_usage) =
        reduce_chunk_summaries(client, notes, &partials, slides, custom_instructions)?;
    total_usage = add_token_usage(total_usage, reduce_usage);
    summary.token_usage = total_usage;
    Ok(summary)
}

fn parse_meeting_summary(text: &str) -> Result<MeetingSummary, LlmError> {
    let json_str = extract_json_object(text).ok_or_else(|| {
        LlmError::Provider(format!("model response contained no JSON object: {text:?}"))
    })?;
    serde_json::from_str(json_str)
        .map_err(|e| LlmError::Provider(format!("failed to parse summary JSON: {e}")))
}

/// Models sometimes wrap JSON in markdown fences or add preamble/postamble text;
/// pull out the outermost `{...}` rather than requiring an exact-match response.
pub(crate) fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end >= start).then(|| &text[start..=end])
}

/// FR-MEM-3: automatic end-of-day digest — what was worked on, and open loops.
/// Ambient capture itself never triggers per-event LLM calls (docs/04-architecture.md:
/// "summarization is batched"); this is meant to be called once, at end of day, over
/// that day's captured items.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct DailyDigest {
    pub summary: String,
    #[serde(default)]
    pub open_loops: Vec<String>,
    #[serde(default)]
    pub token_usage: TokenUsage,
}

/// Generates a digest from a day's worth of retrieved `CaptureItem`s (e.g.
/// `memory::Store::timeline_range` for that day's start/end). Returns a placeholder
/// digest without calling the model at all if `items` is empty — no reason to spend a
/// completion summarizing nothing.
pub fn generate_daily_digest(
    client: &dyn LlmClient,
    items: &[CaptureItem],
    custom_instructions: &str,
) -> Result<DailyDigest, LlmError> {
    if items.is_empty() {
        return Ok(DailyDigest {
            summary: "No activity captured today.".to_string(),
            open_loops: Vec::new(),
            token_usage: TokenUsage::default(),
        });
    }

    let instructions_block = if custom_instructions.is_empty() {
        String::new()
    } else {
        format!("Additional instructions: {custom_instructions}\n\n")
    };
    let mut prompt = String::from(
        "Below is a chronological log of what the user worked on today (app + text \
         snippets captured from their screen). Write a short end-of-day digest: what \
         was worked on, and any open loops or unfinished threads. Respond with ONLY a \
         JSON object, no markdown fences, no other text, matching exactly:\n\
         {\"summary\": \"...\", \"open_loops\": [\"...\"]}\n\n",
    );
    prompt.push_str(&instructions_block);
    for item in items {
        prompt.push_str(&format!("- ({}) {}\n", item.app_bundle_id, item.text));
    }

    let completion = client.complete(&prompt)?;
    let mut digest = parse_daily_digest(&completion.text)?;
    digest.token_usage = completion.token_usage;
    Ok(digest)
}

/// FR-CHAT-6: generates a short (5-8 word) title summarizing one Q&A exchange, for a
/// newly-created conversation's placeholder title. Deliberately a plain-text response,
/// not JSON — unlike `generate_daily_digest`, there's no structured shape to parse, just
/// a short string. Callers should treat any parse/provider failure as "keep the
/// truncated placeholder," never retry.
pub fn generate_conversation_title(
    client: &dyn LlmClient,
    query: &str,
    answer: &str,
) -> Result<(String, TokenUsage), LlmError> {
    let prompt = format!(
        "Generate a short title (5-8 words, no ending punctuation, no surrounding quotes) \
         summarizing this exchange:\n\nQ: {query}\nA: {answer}\n\n\
         Respond with ONLY the title text, nothing else."
    );
    let completion = client.complete(&prompt)?;
    let title = completion.text.trim().trim_matches('"').to_string();
    Ok((title, completion.token_usage))
}

/// `ContextItem`-aware counterpart to `generate_daily_digest` -- same shape
/// (short-circuits to a no-model-call placeholder on empty input, same `DailyDigest`
/// output type), but takes the 5-variant `ContextItem` enum instead of raw
/// `CaptureItem`, using `display_label()`/`text()` for prompt-building so
/// episodes/threads/routine-runs/meeting transcripts read naturally alongside screen
/// captures. `generate_daily_digest` itself is untouched -- this is additive, not a
/// replacement, since MCP's `get_daily_digest` tool is still an unrelated production
/// caller that needs the `CaptureItem`-only version.
/// `window_start_unix_ms` is the local calendar day this digest is actually about (the
/// same window bound the caller already used to gather `items`) -- named in the prompt
/// so the model can tell apart genuine same-day activity from content that only LOOKS
/// like it belongs (a rolling thread summary spanning many days, another routine's own
/// digest of a different window). Retrieval already bounds every item's *row* to this
/// window, but an item's *content* isn't always day-scoped -- a thread's `summary` is a
/// cumulative rolling narrative by design, for instance -- so this is a second,
/// content-level defense the SQL-level bound can't provide on its own.
pub fn generate_daily_digest_from_context(
    client: &dyn LlmClient,
    items: &[ContextItem],
    custom_instructions: &str,
    window_start_unix_ms: i64,
) -> Result<DailyDigest, LlmError> {
    if items.is_empty() {
        return Ok(DailyDigest {
            summary: "No activity captured today.".to_string(),
            open_loops: Vec::new(),
            token_usage: TokenUsage::default(),
        });
    }

    let date_label = chrono::Local
        .timestamp_millis_opt(window_start_unix_ms)
        .single()
        .map(|dt| dt.format("%A, %B %-d, %Y").to_string());

    let instructions_block = if custom_instructions.is_empty() {
        String::new()
    } else {
        format!("Additional instructions: {custom_instructions}\n\n")
    };
    let mut prompt = String::from(
        "Below is a log of the user's day -- screen activity, meeting \
         speech, episode and thread summaries, and routine outputs. Write a detailed \
         end-of-day recap a person could actually use to remember their day -- not a \
         one-line summary. Call out specific projects, PR/issue numbers, decisions, \
         and people by name instead of describing them generically (say \"merged PR \
         #67\", not \"made a code change\"; say \"decided to cut 0.5.5\", not \
         \"discussed a release\"). Group related activity together by project or \
         topic rather than listing it in strict chronological order. Separately call \
         out open loops: unresolved questions, promised follow-ups, decisions not yet \
         made, and work started but not finished. Respond with ONLY a JSON object, no \
         markdown fences, no other text, matching exactly:\n\
         {\"summary\": \"...\", \"open_loops\": [\"...\"]}\n\n",
    );
    if let Some(date_label) = &date_label {
        prompt.push_str(&format!(
            "The day being summarized is {date_label}. Some items below (rolling \
             thread summaries especially) may describe activity from OTHER days too --\
             only include things that actually happened on {date_label} in your \
             summary; ignore anything describing a different day, even if it appears \
             inside an item's own text.\n\n"
        ));
    }
    prompt.push_str(&instructions_block);
    for item in items {
        prompt.push_str(&format!("- ({}) {}\n", item.display_label(), item.text()));
    }

    let completion = client.complete(&prompt)?;
    let mut digest = parse_daily_digest(&completion.text)?;
    digest.token_usage = completion.token_usage;
    Ok(digest)
}

/// Weekly counterpart to `generate_daily_digest_from_context` -- same short-circuit,
/// same `DailyDigest` output shape and JSON contract (so `run_weekly_digest` can reuse
/// `run_eod_recap`'s email-draft plumbing verbatim), but over a 7-day window instead of
/// one day, and with a prompt asking for a genuinely structured weekly reflection
/// (what moved forward, what stalled, what to focus on next) rather than a short digest.
/// `week_end_unix_ms` is treated as exclusive, matching `timeline_range`'s convention.
pub fn generate_weekly_digest_from_context(
    client: &dyn LlmClient,
    items: &[ContextItem],
    custom_instructions: &str,
    week_start_unix_ms: i64,
    week_end_unix_ms: i64,
) -> Result<DailyDigest, LlmError> {
    if items.is_empty() {
        return Ok(DailyDigest {
            summary: "No activity captured this week.".to_string(),
            open_loops: Vec::new(),
            token_usage: TokenUsage::default(),
        });
    }

    let range_label = match (
        chrono::Local
            .timestamp_millis_opt(week_start_unix_ms)
            .single(),
        chrono::Local
            .timestamp_millis_opt(week_end_unix_ms - 1)
            .single(),
    ) {
        (Some(start), Some(end)) => Some(format!(
            "{} - {}, {}",
            start.format("%B %-d"),
            end.format("%B %-d"),
            end.format("%Y")
        )),
        _ => None,
    };

    let instructions_block = if custom_instructions.is_empty() {
        String::new()
    } else {
        format!("Additional instructions: {custom_instructions}\n\n")
    };
    let mut prompt = String::from(
        "Below is a log of the user's week -- screen activity, meeting speech, episode \
         and thread summaries, and routine outputs. Write a detailed weekly reflection \
         the user could actually use to remember and act on their week -- not a bulleted \
         status report. Cover, in this order:\n\
         1. A short opening paragraph on how the week actually went overall.\n\
         2. Where the time actually went -- grouped by project or theme, calling out \
         specific PR/issue numbers, decisions, and people by name (say \"merged PR #67\", \
         not \"made a code change\").\n\
         3. What moved forward this week, by project.\n\
         4. What stalled or is drifting -- anything sitting untouched across multiple \
         days, named specifically, not generically.\n\
         5. What to focus on next week -- a short, concrete, prioritized list.\n\
         6. One closing sentence: the bottom line for the week.\n\
         Respond with ONLY a JSON object, no markdown fences, no other text, matching \
         exactly:\n\
         {\"summary\": \"...\", \"open_loops\": [\"...\"]}\n\n\
         Put the full reflection (all 6 parts, as plain text with clear paragraph or \
         section breaks) in \"summary\". \"open_loops\" should list only the items from \
         part 4, as short standalone bullet strings.\n\n",
    );
    if let Some(range_label) = &range_label {
        prompt.push_str(&format!(
            "The week being summarized is {range_label}. Some items below (rolling \
             thread summaries especially) may describe activity from OTHER weeks too --\
             only include things that actually happened in this window; ignore anything \
             describing a different week, even if it appears inside an item's own text.\n\n"
        ));
    }
    prompt.push_str(&instructions_block);
    for item in items {
        prompt.push_str(&format!("- ({}) {}\n", item.display_label(), item.text()));
    }

    let completion = client.complete(&prompt)?;
    let mut digest = parse_daily_digest(&completion.text)?;
    digest.token_usage = completion.token_usage;
    Ok(digest)
}

fn parse_daily_digest(text: &str) -> Result<DailyDigest, LlmError> {
    let json_str = extract_json_object(text).ok_or_else(|| {
        LlmError::Provider(format!("model response contained no JSON object: {text:?}"))
    })?;
    serde_json::from_str(json_str)
        .map_err(|e| LlmError::Provider(format!("failed to parse digest JSON: {e}")))
}

/// L2 Slice B: one screen-activity episode's source items, summarized into 2-3
/// sentences plus any extracted references (docs/superpowers/specs/2026-08-12-l2-episodes-design.md).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct EpisodeSummary {
    pub summary: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub token_usage: TokenUsage,
}

/// Caps how many of an episode's items go into the summarization prompt (first N + last N,
/// preserving the session's start and end context) and how much of each item's text is
/// included -- an episode with no gap-based cap can span hours and thousands of rows.
/// ponytail: fixed sampling, not token-aware truncation; revisit if real episodes still
/// overflow context after this cap.
const MAX_EPISODE_SUMMARY_ITEMS: usize = 50;
const MAX_ITEM_TEXT_CHARS: usize = 500;

/// Summarizes one episode's source `CaptureItem`s. Unlike `generate_daily_digest`,
/// `items` is never empty here -- `memory::Store::run_episode_segment_job` never
/// finalizes an episode with zero items -- so there's no empty-input shortcut.
pub fn summarize_episode(
    client: &dyn LlmClient,
    items: &[CaptureItem],
    custom_instructions: &str,
) -> Result<EpisodeSummary, LlmError> {
    let instructions_block = if custom_instructions.is_empty() {
        String::new()
    } else {
        format!("Additional instructions: {custom_instructions}\n\n")
    };
    let mut prompt = String::from(
        "Below is a chronological log of screen activity from one continuous working \
         session (same app, no long gaps). Write a brief summary of what was being \
         done, and extract any URLs or file/document references mentioned. \
         Respond with ONLY a JSON object, no markdown fences, no other text, matching \
         exactly:\n\
         {\"summary\": \"...\", \"refs\": [\"...\"]}\n\n",
    );
    prompt.push_str(&instructions_block);
    // Keep the session's start and end context, drop the middle, so an all-day episode
    // can't overflow the model's context (and burn attempts to `summarize_failed`).
    let half = MAX_EPISODE_SUMMARY_ITEMS / 2;
    let sampled: Vec<&CaptureItem> = if items.len() > MAX_EPISODE_SUMMARY_ITEMS {
        items[..half]
            .iter()
            .chain(items[items.len() - half..].iter())
            .collect()
    } else {
        items.iter().collect()
    };
    for item in sampled {
        // Char-based, not byte-based: slicing bytes would panic mid-codepoint.
        let text: String = if item.text.chars().count() > MAX_ITEM_TEXT_CHARS {
            item.text
                .chars()
                .take(MAX_ITEM_TEXT_CHARS)
                .chain("…".chars())
                .collect()
        } else {
            item.text.clone()
        };
        prompt.push_str(&format!("- ({}) {}\n", item.app_bundle_id, text));
    }

    let completion = client.complete(&prompt)?;
    let mut summary = parse_episode_summary(&completion.text)?;
    summary.token_usage = completion.token_usage;
    Ok(summary)
}

fn parse_episode_summary(text: &str) -> Result<EpisodeSummary, LlmError> {
    let json_str = extract_json_object(text).ok_or_else(|| {
        LlmError::Provider(format!("model response contained no JSON object: {text:?}"))
    })?;
    serde_json::from_str(json_str)
        .map_err(|e| LlmError::Provider(format!("failed to parse episode summary JSON: {e}")))
}

/// L2 Threads: one email thread's prior rolling summary (if any) plus only the
/// messages newer than what was already folded in
/// (docs/superpowers/specs/2026-08-14-thread-distillation-design.md).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct ThreadSummary {
    pub summary: String,
    #[serde(default)]
    pub token_usage: TokenUsage,
}

/// Caps how much of each new message's body goes into the prompt -- an email body can
/// be arbitrarily long, unlike a screen-capture text snippet.
const MAX_THREAD_MESSAGE_TEXT_CHARS: usize = 1000;

/// Caps how many of a thread's NEW messages go into one summarization prompt (first N +
/// last N, preserving earliest and most-recent context) -- mirrors
/// MAX_EPISODE_SUMMARY_ITEMS's exact reasoning: an uncapped prompt on a long-neglected
/// thread's first catch-up can overflow the model's context and burn attempts to
/// summarize_failed.
const MAX_THREAD_SUMMARY_MESSAGES: usize = 50;

/// Incrementally updates a thread's rolling summary: feeds the PRIOR summary (if any)
/// plus only the NEW messages to the model, and asks for an updated summary --
/// deliberately never re-reads or re-summarizes older messages already folded into
/// `prior_summary`. This is what keeps a slow local model viable even for a thread
/// with hundreds of messages over months (docs/09-information-gathering-design.md
/// §3's explicit requirement). `new_messages` is expected to be non-empty by
/// convention -- `Store::claim_thread_summarize_jobs` filters out jobs that resolve to
/// zero new messages before they ever reach a caller of this function -- but that's
/// enforced one layer down, not by this function itself.
pub fn summarize_thread_incrementally(
    client: &dyn LlmClient,
    prior_summary: Option<&str>,
    new_messages: &[ThreadEmailMessage],
    custom_instructions: &str,
) -> Result<ThreadSummary, LlmError> {
    let instructions_block = if custom_instructions.is_empty() {
        String::new()
    } else {
        format!("Additional instructions: {custom_instructions}\n\n")
    };
    let mut prompt = String::from(
        "You maintain a running summary of an email thread. Respond with ONLY a JSON \
         object, no markdown fences, no other text, matching exactly:\n\
         {\"summary\": \"...\"}\n\n",
    );
    prompt.push_str(&instructions_block);
    match prior_summary {
        Some(prior) if !prior.is_empty() => {
            prompt.push_str("Here is the summary of the thread so far:\n");
            prompt.push_str(prior);
            prompt.push_str("\n\nHere are new messages that have arrived since. Write an UPDATED summary reflecting both the prior state and these new messages:\n\n");
        }
        _ => {
            prompt.push_str(
                "Here are the messages in a new email thread. Write a short summary:\n\n",
            );
        }
    }
    // Keep the earliest and most-recent new messages, drop the middle, so a
    // long-neglected thread's first catch-up can't overflow the model's context.
    let half = MAX_THREAD_SUMMARY_MESSAGES / 2;
    let sampled: Vec<&ThreadEmailMessage> = if new_messages.len() > MAX_THREAD_SUMMARY_MESSAGES {
        new_messages[..half]
            .iter()
            .chain(new_messages[new_messages.len() - half..].iter())
            .collect()
    } else {
        new_messages.iter().collect()
    };
    for message in sampled {
        let text: String = if message.body_text.chars().count() > MAX_THREAD_MESSAGE_TEXT_CHARS {
            message
                .body_text
                .chars()
                .take(MAX_THREAD_MESSAGE_TEXT_CHARS)
                .chain("…".chars())
                .collect()
        } else {
            message.body_text.clone()
        };
        prompt.push_str(&format!("- ({}) {}\n", message.from, text));
    }

    let completion = client.complete(&prompt)?;
    let mut summary = parse_thread_summary(&completion.text)?;
    summary.token_usage = completion.token_usage;
    Ok(summary)
}

fn parse_thread_summary(text: &str) -> Result<ThreadSummary, LlmError> {
    let json_str = extract_json_object(text).ok_or_else(|| {
        LlmError::Provider(format!("model response contained no JSON object: {text:?}"))
    })?;
    serde_json::from_str(json_str)
        .map_err(|e| LlmError::Provider(format!("failed to parse thread summary JSON: {e}")))
}

/// Cents per 1K tokens for a specific provider/model. Approximate — meant for cap
/// arithmetic, not billing accuracy; expected to become admin-configurable in a later
/// pass (see this plan's Global Constraints).
struct ModelPricing {
    in_cents_per_1k: f64,
    out_cents_per_1k: f64,
}

const FALLBACK_PRICING: ModelPricing = ModelPricing {
    in_cents_per_1k: 0.5,
    out_cents_per_1k: 1.5,
};

fn pricing_for(model: &str) -> ModelPricing {
    match model {
        // Anthropic's published per-1M rates (cached 2026-06-24), divided down to per-1K:
        // haiku-4-5 $1/$5, sonnet-5 $2/$10, opus-5 and opus-4.x $5/$25, fable $10/$50.
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => ModelPricing {
            in_cents_per_1k: 0.1,
            out_cents_per_1k: 0.5,
        },
        "claude-sonnet-5" => ModelPricing {
            in_cents_per_1k: 0.2,
            out_cents_per_1k: 1.0,
        },
        "claude-sonnet-4-6" => ModelPricing {
            in_cents_per_1k: 0.3,
            out_cents_per_1k: 1.5,
        },
        "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => ModelPricing {
            in_cents_per_1k: 0.5,
            out_cents_per_1k: 2.5,
        },
        "claude-fable-5-1" | "claude-fable-5" => ModelPricing {
            in_cents_per_1k: 1.0,
            out_cents_per_1k: 5.0,
        },
        "gpt-4o-mini" => ModelPricing {
            in_cents_per_1k: 0.015,
            out_cents_per_1k: 0.06,
        },
        "gpt-4.1" | "gpt-4o" => ModelPricing {
            in_cents_per_1k: 0.25,
            out_cents_per_1k: 1.0,
        },
        // Google's published per-1M-token rates, divided down to this table's per-1K
        // convention: gemini-2.5-flash $0.30/$2.50, gemini-2.5-pro $1.25/$10.00 (its
        // <=200k-token tier -- the cheaper of its two, matching this table's existing
        // "approximate, for cap arithmetic" posture elsewhere), gemini-2.0-flash
        // $0.10/$0.40.
        "gemini-2.5-flash" => ModelPricing {
            in_cents_per_1k: 0.03,
            out_cents_per_1k: 0.25,
        },
        "gemini-2.5-pro" => ModelPricing {
            in_cents_per_1k: 0.125,
            out_cents_per_1k: 1.0,
        },
        "gemini-2.0-flash" => ModelPricing {
            in_cents_per_1k: 0.01,
            out_cents_per_1k: 0.04,
        },
        _ => FALLBACK_PRICING,
    }
}

/// Estimated cost in whole cents (rounded up) for one call. Ollama is always free
/// (NFR-PRV-1: local-first) — this returns `0` for `Provider::Ollama` without consulting
/// any pricing table, matching llm-governance-gateway's "endpoint tokens are logged but
/// cost $0" convention for local/self-hosted endpoints.
pub fn estimate_cost_cents(provider: Provider, model: &str, usage: TokenUsage) -> i64 {
    if provider.is_local() {
        return 0;
    }
    let pricing = pricing_for(model);
    let in_cost = usage.input_tokens.unwrap_or(0) as f64 / 1000.0 * pricing.in_cents_per_1k;
    let out_cost = usage.output_tokens.unwrap_or(0) as f64 / 1000.0 * pricing.out_cents_per_1k;
    (in_cost + out_cost).ceil() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_unreachable_or_model_missing_ollama_is_environmental() {
        let http = |status| LlmError::Http { status, body: String::new() };
        assert!(LlmError::Network("refused".into()).is_environmental());
        assert!(http(404).is_environmental(), "Ollama answers 404 when the model isn't pulled");
        assert!(!http(500).is_environmental());
        assert!(!http(400).is_environmental());
        assert!(!LlmError::Provider("no JSON object".into()).is_environmental());
    }

    #[test]
    fn provider_is_local_reports_correctly() {
        assert!(Provider::Ollama.is_local());
        assert!(!Provider::Anthropic.is_local());
        assert!(!Provider::OpenAI.is_local());
        assert!(!Provider::Google.is_local());
    }

    #[test]
    fn ollama_is_always_free_regardless_of_token_counts() {
        let usage = TokenUsage {
            input_tokens: Some(100_000),
            output_tokens: Some(50_000),
        };
        assert_eq!(
            estimate_cost_cents(Provider::Ollama, "gemma4:12b", usage),
            0
        );
    }

    #[test]
    fn hosted_providers_estimate_nonzero_cost_from_token_counts() {
        let usage = TokenUsage {
            input_tokens: Some(1000),
            output_tokens: Some(1000),
        };
        assert!(estimate_cost_cents(Provider::Anthropic, "claude-haiku-4-5", usage) > 0);
        assert!(estimate_cost_cents(Provider::OpenAI, "gpt-4o-mini", usage) > 0);
    }

    #[test]
    fn unknown_model_falls_back_to_conservative_pricing_not_zero() {
        let usage = TokenUsage {
            input_tokens: Some(1000),
            output_tokens: Some(1000),
        };
        assert!(estimate_cost_cents(Provider::Anthropic, "some-future-model", usage) > 0);
    }

    #[test]
    fn missing_token_counts_estimate_as_zero_not_an_error() {
        // A provider that didn't report usage (or a mock) shouldn't crash cost
        // estimation — it just can't bill for tokens it doesn't know about.
        let usage = TokenUsage {
            input_tokens: None,
            output_tokens: None,
        };
        assert_eq!(
            estimate_cost_cents(Provider::Anthropic, "claude-haiku-4-5", usage),
            0
        );
    }

    #[test]
    fn anthropic_response_parses_the_first_text_block() {
        let json = r#"{"content":[{"type":"text","text":"pong"},{"type":"tool_use","text":""}]}"#;
        let parsed: AnthropicResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.content[0].kind, "text");
        assert_eq!(parsed.content[0].text, "pong");
    }

    #[test]
    fn openai_response_parses_the_first_choice() {
        let json = r#"{"choices":[{"message":{"content":"pong"}}]}"#;
        let parsed: OpenAIResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.choices[0].message.content, "pong");
    }

    /// Requires a real ANTHROPIC_API_KEY — not part of the default test run, same
    /// pattern as the Ollama `#[ignore]` tests. Run with:
    /// `ANTHROPIC_API_KEY=sk-... cargo test -p llm -- --ignored anthropic`
    #[test]
    #[ignore]
    fn anthropic_client_completes_a_real_prompt() {
        let api_key =
            std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY to run this test");
        let client = AnthropicClient::new(api_key, "claude-haiku-4-5");
        let result = client
            .complete("Reply with exactly one word: pong")
            .unwrap();
        assert!(!result.text.trim().is_empty());
    }

    /// Requires a real OPENAI_API_KEY — same pattern as above. Run with:
    /// `OPENAI_API_KEY=sk-... cargo test -p llm -- --ignored openai`
    #[test]
    #[ignore]
    fn openai_client_completes_a_real_prompt() {
        let api_key = std::env::var("OPENAI_API_KEY").expect("set OPENAI_API_KEY to run this test");
        let client = OpenAIClient::new(api_key, "gpt-4o-mini");
        let result = client
            .complete("Reply with exactly one word: pong")
            .unwrap();
        assert!(!result.text.trim().is_empty());
    }

    #[test]
    fn google_response_parses_the_first_candidates_first_part() {
        let json = r#"{"candidates":[{"content":{"parts":[{"text":"pong"}],"role":"model"}}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":1}}"#;
        let parsed: GoogleResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.candidates[0].content.parts[0].text, "pong");
        assert_eq!(
            parsed.usage_metadata.as_ref().unwrap().prompt_token_count,
            5
        );
        assert_eq!(
            parsed
                .usage_metadata
                .as_ref()
                .unwrap()
                .candidates_token_count,
            1
        );
    }

    /// Requires a real GOOGLE_API_KEY — same pattern as above. Run with:
    /// `GOOGLE_API_KEY=... cargo test -p llm -- --ignored google`
    #[test]
    #[ignore]
    fn google_client_completes_a_real_prompt() {
        let api_key = std::env::var("GOOGLE_API_KEY").expect("set GOOGLE_API_KEY to run this test");
        let client = GoogleClient::new(api_key, "gemini-2.5-flash");
        let result = client
            .complete("Reply with exactly one word: pong")
            .unwrap();
        assert!(!result.text.trim().is_empty());
    }

    fn item(id: i64, app: &str, text: &str) -> CaptureItem {
        CaptureItem {
            id,
            device_id: "test-device".into(),
            ts_unix_ms: 1000 * id,
            app_bundle_id: app.into(),
            window_title: "Some Window".into(),
            url: None,
            text: text.into(),
            hash: 0,
            source: None,
            local_seq: None,
        }
    }

    #[test]
    fn extract_citations_maps_bracket_numbers_to_context_ids() {
        let context = vec![
            ContextItem::Capture(item(101, "com.apple.Safari", "a")),
            ContextItem::Capture(item(202, "com.apple.Terminal", "b")),
        ];
        let citations = extract_citations("The build succeeded [2] per the report [1].", &context);
        assert_eq!(
            citations,
            vec![
                ContextItemRef::Capture { id: 202 },
                ContextItemRef::Capture { id: 101 }
            ]
        );
    }

    #[test]
    fn extract_citations_ignores_out_of_range_and_malformed_markers() {
        let context = vec![ContextItem::Capture(item(101, "com.apple.Safari", "a"))];
        let citations = extract_citations("See [99] and [abc] and [1] and [1] again.", &context);
        assert_eq!(citations, vec![ContextItemRef::Capture { id: 101 }]);
    }

    #[derive(Default)]
    struct MockClient {
        response: String,
        last_prompt: std::cell::RefCell<String>,
    }

    impl MockClient {
        fn responding_with(response: &str) -> Self {
            Self {
                response: response.to_string(),
                ..Default::default()
            }
        }

        fn last_prompt(&self) -> String {
            self.last_prompt.borrow().clone()
        }
    }

    impl LlmClient for MockClient {
        fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
            *self.last_prompt.borrow_mut() = prompt.to_string();
            Ok(Completion {
                text: self.response.clone(),
                token_usage: TokenUsage::default(),
            })
        }
    }

    struct MockClientWithCallCount {
        response: String,
        calls: std::cell::Cell<u32>,
    }

    impl LlmClient for MockClientWithCallCount {
        fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
            self.calls.set(self.calls.get() + 1);
            Ok(Completion {
                text: self.response.clone(),
                token_usage: TokenUsage {
                    input_tokens: Some(100),
                    output_tokens: Some(50),
                },
            })
        }
    }

    #[test]
    fn answer_with_citations_extracts_from_mock_response() {
        let client = MockClient {
            response: "Cargo build succeeded with zero errors [1].".to_string(),
            ..Default::default()
        };
        let context = vec![
            ContextItem::Capture(item(
                11,
                "com.apple.Terminal",
                "cargo build succeeded with zero errors",
            )),
            ContextItem::Capture(item(22, "com.apple.Safari", "unrelated quarterly report")),
        ];
        let answer =
            answer_with_citations(&client, &[], "did the build pass?", &context, "", None).unwrap();
        assert!(answer.text.contains("succeeded"));
        assert_eq!(answer.citations, vec![ContextItemRef::Capture { id: 11 }]);
    }

    /// `capture_item.id` and `transcript_segment.id` are independent AUTOINCREMENT
    /// sequences and can collide (see `ContextItem`'s doc comment) — this pins that
    /// exactly such a collision (both id `1` here) is resolved correctly by kind, not
    /// conflated into a single citation or attributed to the wrong context item.
    #[test]
    fn answer_with_citations_distinguishes_same_id_capture_and_transcript() {
        let client = MockClient {
            response: "See [1] and [2] for details.".to_string(),
            ..Default::default()
        };
        let context = vec![
            ContextItem::Capture(item(1, "com.apple.Terminal", "cargo build succeeded")),
            ContextItem::Transcript(MeetingTranscriptSegment {
                id: 1,
                meeting_id: 42,
                meeting_title: "Standup".to_string(),
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "let's ship Friday".to_string(),
            }),
        ];
        let answer =
            answer_with_citations(&client, &[], "what's the status?", &context, "", None).unwrap();
        assert_eq!(
            answer.citations,
            vec![
                ContextItemRef::Capture { id: 1 },
                ContextItemRef::Transcript { id: 1 }
            ]
        );
    }

    /// Pins the exact wire format `core/ffi`, Swift's `CoreContextItemRef`, and
    /// citation-matching all depend on — a future accidental variant rename or a
    /// `rename_all` change on `ContextItemRef` should fail this test immediately.
    #[test]
    fn context_item_routine_run_exposes_a_real_label_and_text() {
        let run = memory::RoutineRunWithName {
            id: 42,
            routine_id: 1,
            routine_name: "Daily Brief".to_string(),
            ran_at_unix_ms: 0,
            status: memory::RunStatus::Success,
            output_summary: "Shipped the routines scheduler.".to_string(),
            output_json: "{}".to_string(),
            origin_device_id: None,
        };
        let item = ContextItem::RoutineRun(run);
        assert_eq!(item.text(), "Shipped the routines scheduler.");
        assert!(item.display_label().contains("Daily Brief"));
        assert_eq!(item.citation_ref(), ContextItemRef::RoutineRun { id: 42 });
    }

    #[test]
    fn context_item_ref_serializes_to_tagged_json() {
        assert_eq!(
            serde_json::to_string(&ContextItemRef::Capture { id: 5 }).unwrap(),
            r#"{"kind":"capture","id":5}"#
        );
        assert_eq!(
            serde_json::to_string(&ContextItemRef::Transcript { id: 7 }).unwrap(),
            r#"{"kind":"transcript","id":7}"#
        );
    }

    #[test]
    fn answer_with_citations_includes_history_in_prompt() {
        struct CapturingClient {
            last_prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for CapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.last_prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "Yes, as I mentioned [1].".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = CapturingClient {
            last_prompt: std::cell::RefCell::new(String::new()),
        };
        let context = vec![ContextItem::Capture(item(
            11,
            "com.apple.Terminal",
            "cargo build succeeded with zero errors",
        ))];
        let history = vec![
            ChatTurn {
                role: ChatRole::User,
                text: "did the build pass?".to_string(),
            },
            ChatTurn {
                role: ChatRole::Assistant,
                text: "Yes, it passed [1].".to_string(),
            },
        ];
        let answer =
            answer_with_citations(&client, &history, "are you sure?", &context, "", None).unwrap();
        assert_eq!(answer.citations, vec![ContextItemRef::Capture { id: 11 }]);
        let prompt = client.last_prompt.borrow();
        assert!(prompt.contains("Conversation so far"));
        assert!(prompt.contains("User: did the build pass?"));
        assert!(prompt.contains("Assistant: Yes, it passed [1]."));
    }

    /// Requires a locally-running Ollama server — not part of the default test run.
    /// Verified manually against a real local Ollama instance during development;
    /// run with `cargo test -- --ignored` to re-check.
    #[test]
    #[ignore]
    fn answer_with_citations_works_against_real_ollama() {
        let client = OllamaClient::new("gemma4:12b");
        let context = vec![
            ContextItem::Capture(item(
                11,
                "com.apple.Terminal",
                "cargo build succeeded with zero errors",
            )),
            ContextItem::Capture(item(
                22,
                "com.apple.Safari",
                "quarterly revenue finance budget forecast",
            )),
        ];
        let answer = answer_with_citations(
            &client,
            &[],
            "Did the build succeed? Answer yes or no.",
            &context,
            "",
            None,
        )
        .unwrap();
        assert!(!answer.text.trim().is_empty());
        // Verified manually: gemma4:12b reliably answers "Yes [1]" here, correctly
        // citing capture_item 11 (the Terminal item) over 22 (unrelated). Not asserted
        // strictly since model phrasing can vary run to run.
        assert!(
            !answer.citations.is_empty(),
            "expected the model to cite at least one context item"
        );
    }

    /// Requires a locally-running Ollama server. A genuine two-turn conversation: the
    /// second question ("what about the other one?") only makes sense with the first
    /// exchange in view, which is exactly what `history` is for.
    #[test]
    #[ignore]
    fn answer_with_citations_multi_turn_follow_up_against_real_ollama() {
        let client = OllamaClient::new("gemma4:12b");
        let context = vec![
            ContextItem::Capture(item(
                11,
                "com.apple.Terminal",
                "cargo build succeeded with zero errors",
            )),
            ContextItem::Capture(item(
                22,
                "com.apple.Xcode",
                "swift build failed: missing bridging header",
            )),
        ];
        let first = answer_with_citations(
            &client,
            &[],
            "Did the Rust build succeed?",
            &context,
            "",
            None,
        )
        .unwrap();
        let history = vec![
            ChatTurn {
                role: ChatRole::User,
                text: "Did the Rust build succeed?".to_string(),
            },
            ChatTurn {
                role: ChatRole::Assistant,
                text: first.text.clone(),
            },
        ];
        let follow_up = answer_with_citations(
            &client,
            &history,
            "What about the other one?",
            &context,
            "",
            None,
        )
        .unwrap();
        assert!(!follow_up.text.trim().is_empty());
        assert!(
            !follow_up.citations.is_empty(),
            "expected the follow-up to still cite context"
        );
    }

    #[test]
    #[ignore]
    fn ollama_client_completes_a_real_prompt() {
        let client = OllamaClient::new("gemma4:12b");
        let result = client
            .complete("Reply with exactly one word: pong")
            .unwrap();
        assert!(!result.text.trim().is_empty());
    }

    #[test]
    fn extract_json_object_strips_markdown_fences_and_preamble() {
        let text = "Sure, here's the summary:\n```json\n{\"tldr\": \"hi\", \"decisions\": []}\n```\nHope that helps!";
        assert_eq!(
            extract_json_object(text),
            Some("{\"tldr\": \"hi\", \"decisions\": []}")
        );
    }

    #[test]
    fn meeting_title_prefers_the_models_short_title_and_falls_back_to_the_tldr() {
        let summary = |title: &str| MeetingSummary {
            title: title.to_string(),
            tldr: "The team reviewed the Q3 roadmap. Then lunch.".to_string(),
            decisions: vec![],
            action_items: vec![],
            open_questions: vec![],
            token_usage: TokenUsage::default(),
        };
        assert_eq!(meeting_title(&summary("\"Q3 roadmap review.\"")), "Q3 roadmap review");
        assert_eq!(meeting_title(&summary("  ")), "The team reviewed the Q3 roadmap");
        assert_eq!(meeting_title(&summary(&"x".repeat(71))), "The team reviewed the Q3 roadmap");
    }

    #[test]
    fn parse_meeting_summary_fills_in_missing_fields_as_empty() {
        let summary = parse_meeting_summary("{\"tldr\": \"Shipped the feature.\"}").unwrap();
        assert_eq!(summary.tldr, "Shipped the feature.");
        assert!(summary.decisions.is_empty());
        assert!(summary.action_items.is_empty());
        assert!(summary.open_questions.is_empty());
    }

    #[test]
    fn parse_meeting_summary_errors_on_no_json() {
        assert!(parse_meeting_summary("I don't understand the request.").is_err());
    }

    #[test]
    fn derive_short_title_from_tldr_takes_the_first_sentence() {
        let title = derive_short_title_from_tldr(
            "Team discussed Q3 roadmap. They also agreed on the mobile app redesign.",
        );
        assert_eq!(title, "Team discussed Q3 roadmap");
    }

    #[test]
    fn derive_short_title_from_tldr_truncates_a_long_single_sentence() {
        let long_sentence = "This meeting covered an extremely long single sentence about quarterly planning and roadmap prioritization decisions that goes on for a while";
        let title = derive_short_title_from_tldr(long_sentence);
        assert!(
            title.chars().count() <= 71,
            "expected <=70 chars plus ellipsis, got {}",
            title.chars().count()
        );
        assert!(title.ends_with('…'));
    }

    #[test]
    fn derive_short_title_from_tldr_handles_a_short_tldr_with_no_sentence_boundary() {
        let title = derive_short_title_from_tldr("Quiet meeting, no action items");
        assert_eq!(title, "Quiet meeting, no action items");
    }

    #[test]
    fn derive_short_title_from_tldr_is_never_empty_for_non_empty_input() {
        let title = derive_short_title_from_tldr(".");
        assert!(!title.is_empty());
    }

    #[test]
    fn summarize_meeting_uses_mock_client() {
        let client = MockClient {
            response: "{\"tldr\": \"Decided to ship Friday.\", \"decisions\": [\"Ship Friday\"], \"action_items\": [\"Mike: write release notes\"], \"open_questions\": []}".to_string(),
            ..Default::default()
        };
        let summary = summarize_meeting(
            &client,
            "notes: ship friday",
            "transcript: we agreed to ship friday",
            "",
            "",
        )
        .unwrap();
        assert_eq!(summary.tldr, "Decided to ship Friday.");
        assert_eq!(summary.decisions, vec!["Ship Friday"]);
        assert_eq!(summary.action_items, vec!["Mike: write release notes"]);
    }

    #[test]
    fn chunk_transcript_returns_one_chunk_when_under_budget() {
        let transcript = "short transcript, well under any real budget";
        let chunks = chunk_transcript(transcript, 1000);
        assert_eq!(chunks, vec![transcript.to_string()]);
    }

    #[test]
    fn chunk_transcript_splits_at_line_boundaries_when_over_budget() {
        let line = "[   0.0s] Me: this is one line of transcript text.";
        let transcript = std::iter::repeat(line)
            .take(10)
            .collect::<Vec<_>>()
            .join("\n");
        // Budget small enough to force a split, but each individual line fits within it.
        let max_chars = line.len() * 3;
        let chunks = chunk_transcript(&transcript, max_chars);
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        for chunk in &chunks {
            assert!(
                chunk.len() <= max_chars,
                "chunk exceeded max_chars: {} > {}",
                chunk.len(),
                max_chars
            );
            // Every chunk should consist of whole lines only (no mid-line truncation).
            for chunk_line in chunk.lines() {
                assert_eq!(chunk_line, line);
            }
        }
        // No line was dropped.
        let total_lines: usize = chunks.iter().map(|c| c.lines().count()).sum();
        assert_eq!(total_lines, 10);
    }

    #[test]
    fn summarize_meeting_chunks_a_long_transcript_and_reduces_to_one_summary() {
        let response = "{\"tldr\": \"Test summary.\", \"decisions\": [\"Decided X\"], \"action_items\": [\"Do Y\"], \"open_questions\": []}".to_string();
        let client = MockClientWithCallCount {
            response,
            calls: std::cell::Cell::new(0),
        };
        // Comfortably exceeds max_transcript_chars_per_chunk() (~13k chars), forcing
        // multiple chunks.
        let line = "[   0.0s] Me: this is a line of transcript text to pad out the length.";
        let transcript = std::iter::repeat(line)
            .take(2000)
            .collect::<Vec<_>>()
            .join("\n");

        let summary = summarize_meeting(&client, "some notes", &transcript, "", "").unwrap();

        assert_eq!(summary.tldr, "Test summary.");
        assert_eq!(summary.decisions, vec!["Decided X"]);
        assert_eq!(summary.action_items, vec!["Do Y"]);

        let calls = client.calls.get();
        assert!(
            calls >= 3,
            "expected multiple chunk calls plus one reduce call, got {calls}"
        );
        assert_eq!(summary.token_usage.input_tokens, Some(100 * calls));
        assert_eq!(summary.token_usage.output_tokens, Some(50 * calls));
    }

    #[test]
    fn summarize_meeting_map_reduces_even_when_transcript_is_one_oversized_unsplittable_line() {
        // One line, no newlines anywhere, deliberately far longer than any real budget —
        // chunk_transcript can't split it (it never breaks mid-line), so it returns exactly
        // one chunk that is itself over budget. Chunking itself doesn't help this specific
        // case either way — the oversized line goes to the model whole regardless. Before
        // this fix, `summarize_meeting` treated "exactly one chunk" as "no map-reduce
        // needed" and took the single-call fast path (`summarize_whole_transcript`'s exact
        // prompt wording). The fix routes it through the map-reduce path instead: one
        // `summarize_chunk` call (with `chunk_count == 1`, so no "part N of M" framing)
        // followed by one extra, separately billed `reduce_chunk_summaries` call.
        let one_giant_line = "word ".repeat(5000); // ~25,000 chars, well over any real budget
        let client = MockClientWithCallCount {
            response: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
            calls: std::cell::Cell::new(0),
        };
        let summary = summarize_meeting(&client, "notes", &one_giant_line, "", "").unwrap();
        assert_eq!(summary.tldr, "ok");
        // Map-reduce path: 1 summarize_chunk call + 1 reduce_chunk_summaries call = 2.
        // The (buggy) single-call fast path would have made exactly 1 call instead.
        assert_eq!(client.calls.get(), 2);
    }

    #[test]
    fn summarize_whole_transcript_prompt_matches_original_single_call_wording() {
        struct CapturingClient {
            captured_prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for CapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.captured_prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = CapturingClient {
            captured_prompt: std::cell::RefCell::new(String::new()),
        };
        summarize_whole_transcript(&client, "my notes", "my transcript", "", "").unwrap();
        let expected = "You are summarizing a meeting by blending the user's rough notes with the \
         transcript below. Respond with ONLY a JSON object, no other text, no markdown \
         fences, matching exactly this shape:\n\
         {\"title\": \"...\", \"tldr\": \"...\", \"decisions\": [\"...\"], \"action_items\": [\"...\"], \"open_questions\": [\"...\"]}\n\n\
         \"title\" names the meeting's main topic in 3 to 6 words, like a calendar event \
         title (e.g. \"Q3 roadmap review\"), not a sentence.\n\n\
         User's notes:\nmy notes\n\n\
         Transcript:\nmy transcript\n";
        assert_eq!(*client.captured_prompt.borrow(), expected);
    }

    #[test]
    fn summarize_whole_transcript_appends_custom_instructions_when_present() {
        struct CapturingClient {
            captured_prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for CapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.captured_prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = CapturingClient {
            captured_prompt: std::cell::RefCell::new(String::new()),
        };
        summarize_whole_transcript(
            &client,
            "my notes",
            "my transcript",
            "",
            "group decisions by topic",
        )
        .unwrap();
        let prompt = client.captured_prompt.borrow();
        assert!(
            prompt.contains("Additional instructions: group decisions by topic"),
            "custom instructions must appear in the built prompt: {prompt}"
        );
        assert!(
            prompt.find("Additional instructions:").unwrap()
                < prompt.find("User's notes:").unwrap(),
            "instructions block must come before the notes section"
        );
    }

    #[test]
    fn summarize_whole_transcript_empty_instructions_is_byte_identical_to_no_instructions_param() {
        struct CapturingClient {
            captured_prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for CapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.captured_prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = CapturingClient {
            captured_prompt: std::cell::RefCell::new(String::new()),
        };
        summarize_whole_transcript(&client, "my notes", "my transcript", "", "").unwrap();
        let expected = "You are summarizing a meeting by blending the user's rough notes with the \
         transcript below. Respond with ONLY a JSON object, no other text, no markdown \
         fences, matching exactly this shape:\n\
         {\"title\": \"...\", \"tldr\": \"...\", \"decisions\": [\"...\"], \"action_items\": [\"...\"], \"open_questions\": [\"...\"]}\n\n\
         \"title\" names the meeting's main topic in 3 to 6 words, like a calendar event \
         title (e.g. \"Q3 roadmap review\"), not a sentence.\n\n\
         User's notes:\nmy notes\n\n\
         Transcript:\nmy transcript\n";
        assert_eq!(
            *client.captured_prompt.borrow(),
            expected,
            "empty custom_instructions must reproduce today's exact wording"
        );
    }

    struct SlidesCapture {
        prompts: std::cell::RefCell<Vec<String>>,
    }
    impl LlmClient for SlidesCapture {
        fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
            self.prompts.borrow_mut().push(prompt.to_string());
            Ok(Completion {
                text: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
                token_usage: TokenUsage::default(),
            })
        }
    }
    fn slides_capture() -> SlidesCapture {
        SlidesCapture {
            prompts: std::cell::RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn summarize_whole_transcript_empty_slides_is_byte_identical_to_before() {
        let client = slides_capture();
        summarize_whole_transcript(&client, "my notes", "my transcript", "", "").unwrap();
        let expected = "You are summarizing a meeting by blending the user's rough notes with the \
         transcript below. Respond with ONLY a JSON object, no other text, no markdown \
         fences, matching exactly this shape:\n\
         {\"title\": \"...\", \"tldr\": \"...\", \"decisions\": [\"...\"], \"action_items\": [\"...\"], \"open_questions\": [\"...\"]}\n\n\
         \"title\" names the meeting's main topic in 3 to 6 words, like a calendar event \
         title (e.g. \"Q3 roadmap review\"), not a sentence.\n\n\
         User's notes:\nmy notes\n\n\
         Transcript:\nmy transcript\n";
        assert_eq!(client.prompts.borrow()[0], expected);
        // Whitespace-only slides trim to empty, so they must also change nothing.
        let client = slides_capture();
        summarize_whole_transcript(&client, "my notes", "my transcript", "  \n ", "").unwrap();
        assert_eq!(client.prompts.borrow()[0], expected);
    }

    #[test]
    fn summarize_whole_transcript_includes_slides_block_after_notes() {
        let client = slides_capture();
        summarize_whole_transcript(&client, "my notes", "my transcript", "Q3 roadmap", "").unwrap();
        let prompt = client.prompts.borrow()[0].clone();
        assert!(prompt.contains(
            "User's notes:\nmy notes\n\nText from slides shown in the meeting (OCR, may be imperfect):\nQ3 roadmap"
        ), "{prompt}");
        assert!(prompt.find("Q3 roadmap").unwrap() < prompt.find("Transcript:\n").unwrap());
    }

    #[test]
    fn reduce_includes_slides_block() {
        let client = slides_capture();
        let partials = vec![MeetingSummary {
            title: String::new(),
            tldr: "part 1".to_string(),
            decisions: vec![],
            action_items: vec![],
            open_questions: vec![],
            token_usage: TokenUsage::default(),
        }];
        reduce_chunk_summaries(&client, "notes", &partials, "Q3 roadmap", "").unwrap();
        let prompt = client.prompts.borrow()[0].clone();
        assert!(prompt.contains(
            "User's notes:\nnotes\n\nText from slides shown in the meeting (OCR, may be imperfect):\nQ3 roadmap\n\n"
        ), "{prompt}");
    }

    /// R14: chunk prompts are sized to the token budget with no room for slide text, so a
    /// long meeting sends slides only to the reduce call.
    #[test]
    fn summarize_meeting_sends_slides_to_reduce_only_never_chunks() {
        let client = slides_capture();
        let transcript = "Alice: we talked about the roadmap.\n".repeat(max_transcript_chars_per_chunk() / 12);
        summarize_meeting(&client, "notes", &transcript, "Q3 roadmap", "").unwrap();
        let prompts = client.prompts.borrow();
        assert!(prompts.len() >= 3, "expected chunk + reduce calls, got {}", prompts.len());
        let (reduce, chunks) = prompts.split_last().unwrap();
        for chunk in chunks {
            assert!(!chunk.contains("Text from slides"), "chunk prompt carried slides");
        }
        assert!(reduce.contains("Text from slides shown in the meeting (OCR, may be imperfect):\nQ3 roadmap"));
    }

    #[test]
    fn slide_cap_fits_the_local_token_budget() {
        assert_eq!(MAX_SLIDE_CHARS, 3_000);
    }

    #[test]
    fn cap_slides_truncates_on_char_boundary() {
        let long = "é".repeat(MAX_SLIDE_CHARS + 1);
        let capped = cap_slides(&long);
        assert_eq!(capped.chars().count(), MAX_SLIDE_CHARS);
        assert_eq!(cap_slides("  hi \n"), "hi");
    }

    #[test]
    fn summarize_chunk_appends_custom_instructions_when_present() {
        let client = MockClient {
            response: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
            ..Default::default()
        };
        // summarize_chunk doesn't expose the built prompt directly, so this proves the
        // call succeeds with the new parameter in place — the prompt-content assertion
        // lives on summarize_whole_transcript above, which shares the same instructions-
        // block-insertion logic.
        let (summary, _usage) =
            summarize_chunk(&client, 0, 2, "notes", "chunk text", "be terse").unwrap();
        assert_eq!(summary.tldr, "ok");
    }

    #[test]
    fn reduce_chunk_summaries_appends_custom_instructions_when_present() {
        let client = MockClient {
            response: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
            ..Default::default()
        };
        let partials = vec![MeetingSummary {
            title: String::new(),
            tldr: "part 1".to_string(),
            decisions: vec![],
            action_items: vec![],
            open_questions: vec![],
            token_usage: TokenUsage::default(),
        }];
        let (summary, _usage) =
            reduce_chunk_summaries(&client, "notes", &partials, "", "focus on blockers").unwrap();
        assert_eq!(summary.tldr, "ok");
    }

    /// Requires a locally-running Ollama server — not part of the default test run.
    /// Verified manually against a real local Ollama instance during development;
    /// run with `cargo test -- --ignored` to re-check. (A true whisper.cpp ->
    /// summarize_meeting pipeline was additionally verified manually via a one-off
    /// scratch harness combining core/transcribe and core/llm — see PR description —
    /// without adding a structural dependency between the two crates.)
    #[test]
    #[ignore]
    fn summarize_meeting_works_against_real_ollama() {
        let client = OllamaClient::new("gemma4:12b");
        let notes = "discussed Q3 roadmap, need to ship the new onboarding flow";
        let transcript = "Alice: I think we should ship the new onboarding flow by end of Q3. \
                           Bob: Agreed, I'll own the design review. Carol: What about the analytics \
                           dashboard, is that still in scope?";
        let summary = summarize_meeting(&client, notes, transcript, "", "").unwrap();
        // Verified manually: gemma4:12b correctly extracts the decision ("ship by end
        // of Q3"), the action item with its owner ("Bob to own the design review"),
        // and the open question ("is the analytics dashboard still in scope?") from
        // this synthetic transcript.
        assert!(!summary.tldr.trim().is_empty());
        assert!(!summary.decisions.is_empty());
        assert!(!summary.action_items.is_empty());
        assert!(!summary.open_questions.is_empty());
    }

    #[test]
    fn gather_and_assemble_context_for_window_empty_query_uses_timeline_range_for_captures() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 0,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: "at the start boundary".to_string(),
                hash: 1,
                source: "ax".to_string(),
            })
            .unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: "inside the window".to_string(),
                hash: 2,
                source: "ax".to_string(),
            })
            .unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 2_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: "at the end boundary".to_string(),
                hash: 3,
                source: "ax".to_string(),
            })
            .unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 5_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: "outside the window".to_string(),
                hash: 4,
                source: "ax".to_string(),
            })
            .unwrap();

        let items = gather_and_assemble_context_for_window(&store, "", 10_000, 0, 2_000, 20, true);

        let texts: Vec<&str> = items.iter().map(|i| i.text()).collect();
        assert!(
            texts.contains(&"at the start boundary"),
            "expected the start-boundary item, got {texts:?}"
        );
        assert!(
            texts.contains(&"inside the window"),
            "expected the in-window item, got {texts:?}"
        );
        assert!(
            !texts.contains(&"at the end boundary"),
            "expected the end-boundary item excluded, got {texts:?}"
        );
        assert!(
            !texts.contains(&"outside the window"),
            "expected the out-of-window item excluded, got {texts:?}"
        );
    }

    #[test]
    fn gather_and_assemble_context_for_window_non_empty_query_filters_search_results_to_the_window()
    {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 0,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: "fox at the start boundary".to_string(),
                hash: 1,
                source: "ax".to_string(),
            })
            .unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: "the quick brown fox".to_string(),
                hash: 2,
                source: "ax".to_string(),
            })
            .unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 2_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: "fox at the end boundary".to_string(),
                hash: 3,
                source: "ax".to_string(),
            })
            .unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 5_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: "another fox outside the window".to_string(),
                hash: 4,
                source: "ax".to_string(),
            })
            .unwrap();

        let items =
            gather_and_assemble_context_for_window(&store, "fox", 10_000, 0, 2_000, 20, true);

        let texts: Vec<&str> = items.iter().map(|i| i.text()).collect();
        assert!(
            texts.contains(&"fox at the start boundary"),
            "expected the start-boundary match, got {texts:?}"
        );
        assert!(
            texts.contains(&"the quick brown fox"),
            "expected the in-window match, got {texts:?}"
        );
        assert!(
            !texts.contains(&"fox at the end boundary"),
            "expected the end-boundary match excluded, got {texts:?}"
        );
        assert!(
            !texts.contains(&"another fox outside the window"),
            "expected the out-of-window match excluded, got {texts:?}"
        );
    }

    #[test]
    fn gather_and_assemble_context_for_window_empty_query_includes_transcripts_only_when_requested()
    {
        let store = memory::Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms: 0,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                start_cs: 100, // ts_unix_ms = 0 + 100*10 = 1_000, inside the window
                speaker_label: "Me".to_string(),
                text: "we discussed the quarterly approach".to_string(),
            }])
            .unwrap();

        let with_transcripts =
            gather_and_assemble_context_for_window(&store, "", 10_000, 0, 2_000, 20, true);
        let without_transcripts =
            gather_and_assemble_context_for_window(&store, "", 10_000, 0, 2_000, 20, false);

        assert!(
            with_transcripts
                .iter()
                .any(|i| matches!(i, ContextItem::Transcript(_))),
            "expected a Transcript item when include_transcripts=true, got {with_transcripts:?}"
        );
        assert!(
            !without_transcripts.iter().any(|i| matches!(i, ContextItem::Transcript(_))),
            "expected no Transcript items when include_transcripts=false, got {without_transcripts:?}"
        );
    }

    #[test]
    fn gather_and_assemble_context_for_window_empty_query_keeps_late_items_at_realistic_volume() {
        // Regression test for the RRF rank-inversion bug: timeline_range returns items
        // ASC (oldest first), and rrf_fuse treats list position as rank, so without
        // reversing the iterator the oldest items get the highest RRF contribution and
        // budget packing keeps only the earliest hour or two of a realistic day. Fails
        // if the `.rev()` in gather_and_assemble_context_for_window's capture leg is
        // removed (verified manually).
        let store = memory::Store::open_in_memory().unwrap();
        let window_end = 86_400_000; // one day, in ms
        for i in 0..200 {
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "test-device".to_string(),
                    ts_unix_ms: i * (window_end / 200),
                    app_bundle_id: "com.apple.Safari".to_string(),
                    window_title: "W".to_string(),
                    url: None,
                    // Padded to ~120 chars so 200 items (~24,000 chars) exceed the
                    // ~6,000-char budget (CONTEXT_BUDGET_TOKENS * CHARS_PER_TOKEN_ESTIMATE)
                    // and packing actually has to drop items -- at the unpadded ~20
                    // chars/item, all 200 fit regardless of order, and the test can't
                    // distinguish the fix from the bug.
                    text: format!("activity number {i:03} {}", "x".repeat(100)),
                    hash: i,
                    source: "ax".to_string(),
                })
                .unwrap();
        }

        let items = gather_and_assemble_context_for_window(
            &store, "", window_end, 0, window_end, 200, true,
        );

        let texts: Vec<&str> = items.iter().map(|i| i.text()).collect();
        assert!(
            texts.iter().any(|t| t.contains("activity number 199")),
            "expected a late-window item to survive budget packing (proves RRF rank-inversion fix works), got {} items total",
            texts.len()
        );
    }

    #[test]
    fn gather_and_assemble_context_for_window_empty_query_keeps_transcript_among_realistic_captures(
    ) {
        // Regression test for the RRF positional-starvation bug: appending the
        // transcript leg onto keyword_hits after 200 captures put it at rank 200+ in
        // rrf_fuse's single fused list, so it always lost to budget packing regardless
        // of recency. Fixed by giving transcripts their own list (semantic_hits), where
        // the newest transcript gets rank 1 just like the newest capture. Fails if the
        // transcript leg is moved back into keyword_hits (verified manually).
        let store = memory::Store::open_in_memory().unwrap();
        let window_end = 86_400_000; // one day, in ms
        for i in 0..200 {
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "test-device".to_string(),
                    ts_unix_ms: i * (window_end / 200),
                    app_bundle_id: "com.apple.Safari".to_string(),
                    window_title: "W".to_string(),
                    url: None,
                    text: format!("activity number {i:03} {}", "x".repeat(100)),
                    hash: i,
                    source: "ax".to_string(),
                })
                .unwrap();
        }

        let meeting_id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms: 0,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                // ts_unix_ms = 0 + 8_634_000*10 = 86_340_000 = window_end - 60s, i.e.
                // more recent than every capture above.
                start_cs: 8_634_000,
                speaker_label: "Me".to_string(),
                text: "most recent thing said in the meeting".to_string(),
            }])
            .unwrap();

        let items = gather_and_assemble_context_for_window(
            &store, "", window_end, 0, window_end, 200, true,
        );

        let texts: Vec<&str> = items.iter().map(|i| i.text()).collect();
        assert!(
            texts.contains(&"most recent thing said in the meeting"),
            "expected the most-recent transcript segment to survive budget packing, got {} items total",
            texts.len()
        );
    }

    #[test]
    fn daily_digest_skips_the_model_call_when_empty() {
        // A MockClient that would panic if called, to prove no completion happens.
        struct PanicIfCalled;
        impl LlmClient for PanicIfCalled {
            fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
                panic!("should not call the model for an empty day");
            }
        }
        let digest = generate_daily_digest(&PanicIfCalled, &[], "").unwrap();
        assert_eq!(digest.summary, "No activity captured today.");
        assert!(digest.open_loops.is_empty());
    }

    #[test]
    fn daily_digest_uses_mock_client() {
        let client = MockClient {
            response: "{\"summary\": \"Worked on the capture spike and reviewed PRs.\", \"open_loops\": [\"waiting on notarization\"]}"
                .to_string(),
            ..Default::default()
        };
        let items = vec![
            item(1, "com.apple.Xcode", "editing AudioCaptureService.swift"),
            item(2, "com.github.desktop", "reviewing pull request"),
        ];
        let digest = generate_daily_digest(&client, &items, "").unwrap();
        assert_eq!(
            digest.summary,
            "Worked on the capture spike and reviewed PRs."
        );
        assert_eq!(digest.open_loops, vec!["waiting on notarization"]);
    }

    #[test]
    fn generate_daily_digest_from_context_skips_the_model_call_when_empty() {
        struct PanicIfCalled;
        impl LlmClient for PanicIfCalled {
            fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
                panic!("should not call the model for an empty context");
            }
        }
        let digest = generate_daily_digest_from_context(&PanicIfCalled, &[], "", 1_000).unwrap();
        assert_eq!(digest.summary, "No activity captured today.");
        assert!(digest.open_loops.is_empty());
    }

    #[test]
    fn generate_daily_digest_from_context_uses_mock_client() {
        let client = MockClient {
            response: "{\"summary\": \"Worked on the capture spike and reviewed PRs.\", \"open_loops\": [\"waiting on notarization\"]}"
                .to_string(),
            ..Default::default()
        };
        let items = vec![
            ContextItem::Capture(item(
                1,
                "com.apple.Xcode",
                "editing AudioCaptureService.swift",
            )),
            ContextItem::Capture(item(2, "com.github.desktop", "reviewing pull request")),
        ];
        let digest = generate_daily_digest_from_context(&client, &items, "", 1_000).unwrap();
        assert_eq!(
            digest.summary,
            "Worked on the capture spike and reviewed PRs."
        );
        assert_eq!(digest.open_loops, vec!["waiting on notarization"]);
    }

    #[test]
    fn generate_daily_digest_from_context_prompt_uses_display_label_and_text_for_a_non_capture_kind(
    ) {
        let client = MockClient {
            response: "{\"summary\": \"ok\", \"open_loops\": []}".to_string(),
            ..Default::default()
        };
        let episode = memory::Episode {
            id: 42,
            device_id: "test-device".to_string(),
            app_bundle_id: "com.apple.Terminal".to_string(),
            window_title: Some("Terminal".to_string()),
            start_ts_unix_ms: 1_000,
            end_ts_unix_ms: 2_000,
            item_count: 3,
            summary: Some("Debugged the build.".to_string()),
            status: "summarized".to_string(),
        };
        let items = vec![ContextItem::Episode(episode)];
        generate_daily_digest_from_context(&client, &items, "", 1_000).unwrap();
        let prompt = client.last_prompt();
        assert!(
            prompt.contains("episode \"Terminal\""),
            "expected the episode's display_label in the prompt, got: {prompt}"
        );
        assert!(
            prompt.contains("Debugged the build."),
            "expected the episode's text in the prompt, got: {prompt}"
        );
    }

    /// Requires a locally-running Ollama server — not part of the default test run.
    /// Verified manually against a real local Ollama instance during development;
    /// run with `cargo test -- --ignored` to re-check.
    #[test]
    #[ignore]
    fn daily_digest_works_against_real_ollama() {
        let client = OllamaClient::new("gemma4:12b");
        let items = vec![
            item(
                1,
                "com.apple.Terminal",
                "cargo test passing, 32 tests green",
            ),
            item(
                2,
                "com.github.desktop",
                "opened PR for the WAL crash recovery test",
            ),
            item(
                3,
                "com.apple.Xcode",
                "still waiting on notarization to come back before dogfooding",
            ),
        ];
        let digest = generate_daily_digest(&client, &items, "").unwrap();
        // Verified manually: gemma4:12b's summary mentions the completed tests and PR,
        // and its open_loops correctly flags the pending notarization as unfinished.
        assert!(!digest.summary.trim().is_empty());
        assert!(!digest.open_loops.is_empty());
    }

    #[test]
    fn ollama_dropping_an_accepted_request_is_a_provider_error_not_network() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut conn, &mut buf);
        });
        let err = OllamaClient::with_base_url(url, "m").complete("hi").unwrap_err();
        assert!(matches!(err, LlmError::Provider(_)), "got {err:?}");
    }

    #[test]
    fn ollama_not_running_stays_a_network_error() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = OllamaClient::with_base_url(format!("http://127.0.0.1:{port}"), "m").complete("hi").unwrap_err();
        assert!(matches!(err, LlmError::Network(_)), "got {err:?}");
    }

    #[test]
    fn summarize_episode_uses_mock_client() {
        let client = MockClient {
            response: "{\"summary\": \"Reviewed and merged the sync PR.\", \"refs\": [\"https://github.com/example/pr/1\"]}".to_string(),
            ..Default::default()
        };
        let items = vec![
            item(1, "com.github.desktop", "reviewing pull request"),
            item(2, "com.github.desktop", "merged"),
        ];
        let summary = summarize_episode(&client, &items, "").unwrap();
        assert_eq!(summary.summary, "Reviewed and merged the sync PR.");
        assert_eq!(summary.refs, vec!["https://github.com/example/pr/1"]);
    }

    #[test]
    fn summarize_episode_fills_in_missing_refs_as_empty() {
        let client = MockClient {
            response: "{\"summary\": \"Wrote docs.\"}".to_string(),
            ..Default::default()
        };
        let items = vec![item(1, "com.apple.TextEdit", "writing docs")];
        let summary = summarize_episode(&client, &items, "").unwrap();
        assert_eq!(summary.summary, "Wrote docs.");
        assert!(summary.refs.is_empty());
    }

    #[test]
    fn summarize_episode_appends_custom_instructions_when_present() {
        let client = MockClient::responding_with(
            "{\"summary\": \"Reviewed and merged the sync PR.\", \"refs\": []}",
        );
        let items = vec![item(1, "com.github.desktop", "reviewing pull request")];
        summarize_episode(&client, &items, "be terse").unwrap();
        assert!(
            client.last_prompt().contains("Additional instructions: be terse"),
            "custom instructions must appear in the built prompt: {}",
            client.last_prompt()
        );
    }

    #[test]
    fn summarize_episode_empty_instructions_is_byte_identical_to_no_instructions_param() {
        let client = MockClient::responding_with(
            "{\"summary\": \"Reviewed and merged the sync PR.\", \"refs\": []}",
        );
        let items = vec![item(1, "com.github.desktop", "reviewing pull request")];
        summarize_episode(&client, &items, "").unwrap();
        let expected = "Below is a chronological log of screen activity from one continuous working \
         session (same app, no long gaps). Write a brief summary of what was being \
         done, and extract any URLs or file/document references mentioned. \
         Respond with ONLY a JSON object, no markdown fences, no other text, matching \
         exactly:\n\
         {\"summary\": \"...\", \"refs\": [\"...\"]}\n\n\
         - (com.github.desktop) reviewing pull request\n";
        assert_eq!(client.last_prompt(), expected, "empty custom_instructions must reproduce today's exact wording");
    }

    #[test]
    fn summarize_episode_caps_prompt_item_count_and_per_item_text() {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "{\"summary\": \"ok\"}".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };

        // One more item than the cap, each tagged so we can tell head from middle from tail,
        // and each with text far longer than MAX_ITEM_TEXT_CHARS.
        let total = MAX_EPISODE_SUMMARY_ITEMS + 11;
        let items: Vec<CaptureItem> = (0..total)
            .map(|i| {
                item(
                    i as i64,
                    "com.app.a",
                    &format!("marker-{i}-{}", "x".repeat(MAX_ITEM_TEXT_CHARS + 50)),
                )
            })
            .collect();

        summarize_episode(&client, &items, "").unwrap();
        let prompt = client.prompt.borrow().clone();

        let half = MAX_EPISODE_SUMMARY_ITEMS / 2;
        assert_eq!(
            prompt.lines().filter(|l| l.starts_with("- (")).count(),
            MAX_EPISODE_SUMMARY_ITEMS,
            "at most MAX_EPISODE_SUMMARY_ITEMS items may reach the prompt",
        );
        assert!(
            prompt.contains("marker-0-"),
            "the session's opening context must be kept"
        );
        assert!(
            prompt.contains(&format!("marker-{}-", total - 1)),
            "the session's closing context must be kept"
        );
        assert!(
            !prompt.contains(&format!("marker-{half}-")),
            "the middle must be the part that's dropped"
        );

        // Every included line is truncated to the per-item cap plus the ellipsis.
        for line in prompt.lines().filter(|l| l.starts_with("- (")) {
            let text = line.split_once(") ").unwrap().1;
            assert_eq!(text.chars().count(), MAX_ITEM_TEXT_CHARS + 1);
            assert!(
                text.ends_with('…'),
                "a truncated item's text must be marked as truncated"
            );
        }
    }

    #[test]
    fn summarize_episode_leaves_short_item_text_untouched() {
        let client = MockClient {
            response: "{\"summary\": \"ok\"}".to_string(),
            ..Default::default()
        };
        let items = vec![item(1, "com.apple.TextEdit", "short text")];
        // Guards the truncation branch against off-by-one: under the cap, nothing changes.
        assert_eq!(summarize_episode(&client, &items, "").unwrap().summary, "ok");
    }

    /// Requires a locally-running Ollama server -- not part of the default test run,
    /// same convention as `daily_digest_works_against_real_ollama`.
    #[test]
    #[ignore]
    fn summarize_episode_works_against_real_ollama() {
        let client = OllamaClient::new("gemma4:12b");
        let items = vec![
            item(
                1,
                "com.apple.Terminal",
                "cargo test passing, 32 tests green",
            ),
            item(2, "com.github.desktop", "opened PR for the sync fix"),
        ];
        let summary = summarize_episode(&client, &items, "").unwrap();
        assert!(!summary.summary.is_empty());
    }

    #[test]
    fn summarize_thread_incrementally_with_no_prior_summary_uses_mock_client() {
        let client = MockClient::responding_with(
            r#"{"summary": "Alice asked about the quarterly report."}"#,
        );
        let messages = vec![ThreadEmailMessage {
            item_id: 1,
            from: "alice@example.com".into(),
            ts_unix_ms: 1000,
            body_text: "Can you send the quarterly report?".into(),
        }];

        let result = summarize_thread_incrementally(&client, None, &messages, "").unwrap();

        assert_eq!(result.summary, "Alice asked about the quarterly report.");
    }

    #[test]
    fn summarize_thread_incrementally_includes_prior_summary_in_the_prompt() {
        let client = MockClient::responding_with(
            r#"{"summary": "Alice asked about the report; I sent it."}"#,
        );
        let messages = vec![ThreadEmailMessage {
            item_id: 2,
            from: "me@example.com".into(),
            ts_unix_ms: 2000,
            body_text: "Here it is.".into(),
        }];

        let result = summarize_thread_incrementally(
            &client,
            Some("Alice asked about the quarterly report."),
            &messages,
            "",
        )
        .unwrap();

        assert_eq!(result.summary, "Alice asked about the report; I sent it.");
        assert!(
            client
                .last_prompt()
                .contains("Alice asked about the quarterly report."),
            "the prior summary must appear in the prompt sent to the model"
        );
    }

    #[test]
    fn summarize_thread_incrementally_appends_custom_instructions_when_present() {
        let client = MockClient::responding_with(r#"{"summary": "ok"}"#);
        let messages = vec![ThreadEmailMessage {
            item_id: 1,
            from: "alice@example.com".into(),
            ts_unix_ms: 1000,
            body_text: "Can you send the report?".into(),
        }];
        summarize_thread_incrementally(&client, None, &messages, "be terse").unwrap();
        assert!(
            client.last_prompt().contains("Additional instructions: be terse"),
            "custom instructions must appear in the built prompt: {}",
            client.last_prompt()
        );
    }

    #[test]
    fn summarize_thread_incrementally_empty_instructions_is_byte_identical_to_no_instructions_param() {
        let client = MockClient::responding_with(r#"{"summary": "ok"}"#);
        let messages = vec![ThreadEmailMessage {
            item_id: 1,
            from: "alice@example.com".into(),
            ts_unix_ms: 1000,
            body_text: "Can you send the report?".into(),
        }];
        summarize_thread_incrementally(&client, None, &messages, "").unwrap();
        let expected = "You maintain a running summary of an email thread. Respond with ONLY a JSON \
         object, no markdown fences, no other text, matching exactly:\n\
         {\"summary\": \"...\"}\n\n\
         Here are the messages in a new email thread. Write a short summary:\n\n\
         - (alice@example.com) Can you send the report?\n";
        assert_eq!(client.last_prompt(), expected, "empty custom_instructions must reproduce today's exact wording");
    }

    #[test]
    #[ignore]
    fn summarize_thread_incrementally_works_against_real_ollama() {
        let client = OllamaClient::new("llama3.1:8b");
        let messages = vec![ThreadEmailMessage {
            item_id: 1,
            from: "alice@example.com".into(),
            ts_unix_ms: 1000,
            body_text: "Can you send the quarterly report by Friday?".into(),
        }];
        let result = summarize_thread_incrementally(&client, None, &messages, "").unwrap();
        assert!(!result.summary.is_empty());
    }

    fn fixture_episode(id: i64, end_ts_unix_ms: i64, summary: Option<&str>) -> memory::Episode {
        memory::Episode {
            id,
            device_id: "dev-1".to_string(),
            app_bundle_id: "com.apple.Terminal".to_string(),
            window_title: Some("Terminal".to_string()),
            start_ts_unix_ms: end_ts_unix_ms - 60_000,
            end_ts_unix_ms,
            item_count: 3,
            summary: summary.map(|s| s.to_string()),
            status: "summarized".to_string(),
        }
    }

    fn fixture_thread(
        id: i64,
        last_activity_ts_unix_ms: i64,
        summary: Option<&str>,
    ) -> memory::Thread {
        memory::Thread {
            id,
            source: "gmail".to_string(),
            external_thread_id: "ext-1".to_string(),
            account_email: "me@example.com".to_string(),
            subject: Some("Q3 planning".to_string()),
            message_count: 4,
            last_activity_ts_unix_ms,
            last_message_from: "alice@example.com".to_string(),
            summary: summary.map(|s| s.to_string()),
            status: "waiting_on_me".to_string(),
            summarize_status: "summarized".to_string(),
        }
    }

    fn cap(id: i64, ts: i64) -> (ContextItem, i64) {
        (
            ContextItem::Capture(CaptureItem {
                id,
                device_id: "dev-1".to_string(),
                ts_unix_ms: ts,
                app_bundle_id: "com.apple.Terminal".to_string(),
                window_title: "Terminal".to_string(),
                url: None,
                text: format!("capture {id}"),
                hash: 0,
                source: None,
                local_seq: None,
            }),
            ts,
        )
    }

    #[test]
    fn rrf_item_in_two_lists_outranks_item_in_only_one_at_same_rank() {
        let shared = cap(1, 1_000);
        let solo = cap(2, 1_000);
        let keyword = vec![shared.clone()];
        let semantic = vec![shared.clone()];
        let solo_only = vec![solo.clone()];
        let out = assemble_context("q", 1_000, &keyword, &semantic, &solo_only, &[], &[]);
        let ids: Vec<i64> = out
            .iter()
            .map(|i| match i.citation_ref() {
                ContextItemRef::Capture { id } => id,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            ids,
            vec![1, 2],
            "item present in both keyword+semantic must rank above an item in only routine_runs"
        );
    }

    #[test]
    fn rrf_disjoint_lists_merge_without_error() {
        let out = assemble_context(
            "q",
            1_000,
            &[cap(1, 1_000)],
            &[cap(2, 1_000)],
            &[cap(3, 1_000)],
            &[],
            &[],
        );
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn rrf_two_empty_lists_degrades_to_ranking_by_the_remaining_one() {
        let out = assemble_context(
            "q",
            1_000,
            &[cap(1, 1_000), cap(2, 900)],
            &[],
            &[],
            &[],
            &[],
        );
        let ids: Vec<i64> = out
            .iter()
            .map(|i| match i.citation_ref() {
                ContextItemRef::Capture { id } => id,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn exact_score_ties_break_deterministically_by_citation_ref_not_hashmap_order() {
        let now = 10_000_000;
        // Two captures, each rank-1 in its own disjoint list, same timestamp -> identical
        // RRF score and identical decayed score: an exact tie. Inserted id-descending (2
        // then 1) so a pass can't be an accident of insertion order happening to match
        // HashMap iteration order.
        let higher_id_first = cap(2, now);
        let lower_id_second = cap(1, now);
        let out = assemble_context(
            "q",
            now,
            &[higher_id_first],
            &[lower_id_second],
            &[],
            &[],
            &[],
        );
        let ids: Vec<i64> = out
            .iter()
            .map(|i| match i.citation_ref() {
                ContextItemRef::Capture { id } => id,
                _ => unreachable!(),
            })
            .collect();
        // Tie broken by ContextItemRef ascending (Capture { id: 1 } < Capture { id: 2 }),
        // per the `then_with` tiebreak in assemble_context's l1/l2 sorts -- independent of
        // rrf_fuse's HashMap iteration order, so this order is stable across runs.
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn decay_more_recent_item_ranks_higher_at_equal_rrf_score() {
        // Both items rank 1 in disjoint lists -- equal RRF score. Only age differs.
        let now = 10_000_000;
        let recent = cap(1, now - 1_000); // 1 second old
        let old = cap(2, now - RECENCY_TAU_CAPTURE_MS); // exactly one tau old
        let out = assemble_context("q", now, &[recent], &[old], &[], &[], &[]);
        let ids: Vec<i64> = out
            .iter()
            .map(|i| match i.citation_ref() {
                ContextItemRef::Capture { id } => id,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            ids,
            vec![1, 2],
            "the 1-second-old item must outrank the one-tau-old item despite equal RRF score"
        );
    }

    #[test]
    fn decay_thread_outlasts_episode_of_the_same_age() {
        let now = 10_000_000;
        // An age between episode-tau and thread-tau: episode has decayed far more than thread.
        let age = RECENCY_TAU_EPISODE_MS * 3; // well past episode's tau, nowhere near thread's (14d)
        let episode = fixture_episode(1, now - age, Some("episode summary"));
        let thread = fixture_thread(1, now - age, Some("thread summary"));
        let out = assemble_context(
            "q",
            now,
            &[],
            &[],
            &[],
            std::slice::from_ref(&episode),
            std::slice::from_ref(&thread),
        );
        // Both included (no L1 competing for budget here); thread must sort before episode.
        assert!(
            matches!(out[0], ContextItem::Thread(_)),
            "thread (14d tau) should outrank an equally-aged episode (3d tau)"
        );
        assert!(matches!(out[1], ContextItem::Episode(_)));
    }

    #[test]
    fn budget_excludes_an_item_entirely_rather_than_partially() {
        let now = 1_000;
        // Each capture's text is exactly half the budget in chars, so a 3rd copy can't fit.
        let half_budget_text = "x".repeat((CONTEXT_BUDGET_TOKENS * CHARS_PER_TOKEN_ESTIMATE) / 2);
        let make = |id: i64| {
            (
                ContextItem::Capture(CaptureItem {
                    id,
                    device_id: "dev-1".to_string(),
                    ts_unix_ms: now,
                    app_bundle_id: "com.apple.Terminal".to_string(),
                    window_title: "Terminal".to_string(),
                    url: None,
                    text: half_budget_text.clone(),
                    hash: 0,
                    source: None,
                    local_seq: None,
                }),
                now,
            )
        };
        let hits = vec![make(1), make(2), make(3)];
        let out = assemble_context("q", now, &hits, &[], &[], &[], &[]);
        assert_eq!(out.len(), 2, "only the first two half-budget items fit; the third must be excluded entirely, not truncated");
    }

    #[test]
    fn question_matches_precede_recent_episodes_regardless_of_individual_scores() {
        let now = 10_000_000;
        // The match is old (low score); the episode is fresh. The match must still lead:
        // packing episodes first used to fill the budget and drop found answers.
        let old_cap = cap(1, now - RECENCY_TAU_CAPTURE_MS * 5);
        let fresh_episode = fixture_episode(1, now, Some("fresh episode"));
        let out = assemble_context(
            "q",
            now,
            &[old_cap],
            &[],
            &[],
            std::slice::from_ref(&fresh_episode),
            &[],
        );
        assert!(
            matches!(out[0], ContextItem::Capture(_)),
            "L1 (the question's match) must be packed before L2 even when L2's raw score is far higher"
        );
        assert!(matches!(out[1], ContextItem::Episode(_)));
    }

    #[test]
    fn an_item_too_big_for_the_remaining_budget_is_skipped_not_the_end_of_packing() {
        let now = 10_000_000;
        let budget = CONTEXT_BUDGET_TOKENS * CHARS_PER_TOKEN_ESTIMATE;
        let big = fixture_episode(1, now, Some(&"x".repeat(budget)));
        let small = fixture_episode(2, now - 1, Some("small"));
        let out = assemble_context("q", now, &[cap(1, now)], &[], &[], &[big, small], &[]);
        assert_eq!(out.len(), 2, "the oversized episode is skipped; the capture and the small episode both fit");
    }

    #[test]
    fn empty_inputs_return_empty_and_single_item_returns_that_item() {
        assert!(assemble_context("q", 0, &[], &[], &[], &[], &[]).is_empty());
        let out = assemble_context("q", 1_000, &[cap(1, 1_000)], &[], &[], &[], &[]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn context_item_episode_exposes_label_text_and_citation_ref() {
        let item = ContextItem::Episode(fixture_episode(42, 1_000, Some("Debugged the build.")));
        assert_eq!(item.display_label(), "episode \"Terminal\"");
        assert_eq!(item.text(), "Debugged the build.");
        assert_eq!(item.citation_ref(), ContextItemRef::Episode { id: 42 });
    }

    #[test]
    fn context_item_episode_with_no_summary_yet_uses_placeholder_text() {
        let item = ContextItem::Episode(fixture_episode(1, 1_000, None));
        assert_eq!(item.text(), "(summary pending)");
    }

    #[test]
    fn context_item_thread_exposes_label_text_and_citation_ref() {
        let item = ContextItem::Thread(fixture_thread(7, 1_000, Some("Alice proposed Tuesday.")));
        assert_eq!(item.display_label(), "thread: Q3 planning");
        assert_eq!(item.text(), "Alice proposed Tuesday.");
        assert_eq!(item.citation_ref(), ContextItemRef::Thread { id: 7 });
    }

    #[test]
    fn gather_and_assemble_context_includes_capture_transcript_and_routine_run_items() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "Some Window".to_string(),
                url: None,
                text: "the quick brown fox jumps".to_string(),
                hash: 1,
                source: "ax".to_string(),
            })
            .unwrap();

        let meeting_id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms: 2_000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "the quick brown fox jumps over the lazy dog".to_string(),
            }])
            .unwrap();

        let routine_id = store
            .insert_routine(&memory::NewRoutine {
                name: "Build Watcher".to_string(),
                template: memory::RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        let run_id = store
            .insert_pending_routine_run(&memory::NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 3_000,
                output_summary: "the quick brown fox build succeeded".to_string(),
                pending_action_json: "{}".to_string(),
            })
            .unwrap();
        store
            .finalize_routine_run(
                run_id,
                memory::RunStatus::Success,
                "the quick brown fox build succeeded".to_string(),
                "{}".to_string(),
            )
            .unwrap();

        let items = gather_and_assemble_context(&store, "fox", 10_000, 10, true);

        assert!(
            items.iter().any(|i| matches!(i, ContextItem::Capture(_))),
            "expected a Capture item, got {items:?}"
        );
        assert!(
            items
                .iter()
                .any(|i| matches!(i, ContextItem::Transcript(_))),
            "expected a Transcript item, got {items:?}"
        );
        assert!(
            items
                .iter()
                .any(|i| matches!(i, ContextItem::RoutineRun(_))),
            "expected a RoutineRun item, got {items:?}"
        );
    }

    #[test]
    fn gather_and_assemble_context_omits_transcript_items_when_include_transcripts_is_false() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "Some Window".to_string(),
                url: None,
                text: "the quick brown fox jumps".to_string(),
                hash: 1,
                source: "ax".to_string(),
            })
            .unwrap();

        let meeting_id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms: 2_000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "the quick brown fox jumps over the lazy dog".to_string(),
            }])
            .unwrap();

        let items = gather_and_assemble_context(&store, "fox", 10_000, 10, false);

        assert!(
            items.iter().any(|i| matches!(i, ContextItem::Capture(_))),
            "expected a Capture item, got {items:?}"
        );
        assert!(
            !items
                .iter()
                .any(|i| matches!(i, ContextItem::Transcript(_))),
            "expected no Transcript items with include_transcripts=false, got {items:?}"
        );
    }

    #[test]
    fn gather_and_assemble_context_includes_snapshots_only_with_transcripts() {
        let store = memory::Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms: 2_000,
                end_unix_ms: None,
                title: "Board review".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_meeting_snapshot(&memory::NewMeetingSnapshot {
                meeting_id,
                captured_unix_ms: 3_000,
                file_path: "/tmp/s.png".to_string(),
                width: 10,
                height: 10,
                ocr_text: "Roadmap Q3 hiring plan".to_string(),
            })
            .unwrap();

        let with = gather_and_assemble_context(&store, "hiring", 10_000, 10, true);
        let snap = with
            .iter()
            .find_map(|i| match i {
                ContextItem::Snapshot(h) => Some(h),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected a Snapshot item, got {with:?}"));
        assert_eq!(snap.meeting_id, meeting_id);
        let item = ContextItem::Snapshot(snap.clone());
        assert_eq!(item.display_label(), "slide in meeting \"Board review\"");
        assert_eq!(item.citation_ref(), ContextItemRef::Snapshot { id: snap.id });

        let without = gather_and_assemble_context(&store, "hiring", 10_000, 10, false);
        assert!(!without.iter().any(|i| matches!(i, ContextItem::Snapshot(_))));
    }

    #[test]
    fn context_item_ref_snapshot_serializes_as_kind_snapshot() {
        assert_eq!(
            serde_json::to_string(&ContextItemRef::Snapshot { id: 9 }).unwrap(),
            r#"{"kind":"snapshot","id":9}"#
        );
    }

    #[test]
    fn gather_and_assemble_context_on_a_fresh_store_returns_empty_without_panicking() {
        let store = memory::Store::open_in_memory().unwrap();
        let items = gather_and_assemble_context(&store, "anything", 10_000, 10, true);
        assert!(items.is_empty());
    }

    #[test]
    fn answer_with_citations_includes_custom_instructions_in_the_prompt_when_present() {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "Answer [1].".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };
        let context = vec![ContextItem::Capture(item(
            1,
            "com.apple.Terminal",
            "build passed",
        ))];

        answer_with_citations(
            &client,
            &[],
            "did it pass?",
            &context,
            "Always answer in bullet points.",
            None,
        )
        .unwrap();

        assert!(client
            .prompt
            .borrow()
            .contains("Additional instructions: Always answer in bullet points.\n\n"));
    }

    #[test]
    fn answer_with_citations_omits_instructions_block_when_custom_instructions_is_empty() {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "Answer [1].".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };
        let context = vec![ContextItem::Capture(item(
            1,
            "com.apple.Terminal",
            "build passed",
        ))];

        answer_with_citations(&client, &[], "did it pass?", &context, "", None).unwrap();

        assert!(!client.prompt.borrow().contains("Additional instructions"));
    }

    #[test]
    fn generate_daily_digest_includes_custom_instructions_in_the_prompt_when_present() {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "{\"summary\": \"ok\", \"open_loops\": []}".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };
        let items = vec![item(1, "com.apple.Terminal", "build passed")];

        generate_daily_digest(&client, &items, "Keep it terse.").unwrap();

        assert!(client
            .prompt
            .borrow()
            .contains("Additional instructions: Keep it terse.\n\n"));
    }

    #[test]
    fn generate_daily_digest_omits_instructions_block_when_custom_instructions_is_empty() {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "{\"summary\": \"ok\", \"open_loops\": []}".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };
        let items = vec![item(1, "com.apple.Terminal", "build passed")];

        generate_daily_digest(&client, &items, "").unwrap();

        assert!(!client.prompt.borrow().contains("Additional instructions"));
    }

    #[test]
    fn generate_daily_digest_from_context_includes_custom_instructions_in_the_prompt_when_present()
    {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "{\"summary\": \"ok\", \"open_loops\": []}".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };
        let items = vec![ContextItem::Capture(item(
            1,
            "com.apple.Terminal",
            "build passed",
        ))];

        generate_daily_digest_from_context(&client, &items, "Keep it terse.", 1_000).unwrap();

        assert!(client
            .prompt
            .borrow()
            .contains("Additional instructions: Keep it terse.\n\n"));
    }

    #[test]
    fn generate_daily_digest_from_context_omits_instructions_block_when_custom_instructions_is_empty(
    ) {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "{\"summary\": \"ok\", \"open_loops\": []}".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };
        let items = vec![ContextItem::Capture(item(
            1,
            "com.apple.Terminal",
            "build passed",
        ))];

        generate_daily_digest_from_context(&client, &items, "", 1_000).unwrap();

        assert!(!client.prompt.borrow().contains("Additional instructions"));
    }

    #[test]
    fn generate_weekly_digest_from_context_skips_the_model_call_when_empty() {
        struct PanicIfCalled;
        impl LlmClient for PanicIfCalled {
            fn complete(&self, _prompt: &str) -> Result<Completion, LlmError> {
                panic!("should not call the model for an empty context");
            }
        }
        let digest =
            generate_weekly_digest_from_context(&PanicIfCalled, &[], "", 1_000, 2_000).unwrap();
        assert_eq!(digest.summary, "No activity captured this week.");
        assert!(digest.open_loops.is_empty());
    }

    #[test]
    fn generate_weekly_digest_from_context_uses_mock_client() {
        let client = MockClient {
            response:
                "{\"summary\": \"Shipped several PRs.\", \"open_loops\": [\"lock the demo date\"]}"
                    .to_string(),
            ..Default::default()
        };
        let items = vec![ContextItem::Capture(item(
            1,
            "com.github.desktop",
            "merged PR #67",
        ))];
        let digest =
            generate_weekly_digest_from_context(&client, &items, "", 0, 7 * 24 * 60 * 60 * 1000)
                .unwrap();
        assert_eq!(digest.summary, "Shipped several PRs.");
        assert_eq!(digest.open_loops, vec!["lock the demo date"]);
    }

    #[test]
    fn generate_weekly_digest_from_context_names_the_summarized_week_in_the_prompt() {
        let client = MockClient {
            response: "{\"summary\": \"ok\", \"open_loops\": []}".to_string(),
            ..Default::default()
        };
        let items = vec![ContextItem::Capture(item(
            1,
            "com.apple.Terminal",
            "shipped",
        ))];
        // 2026-08-17 00:00 through 2026-08-24 00:00 local, i.e. Aug 17-23.
        let week_start = chrono::Local
            .with_ymd_and_hms(2026, 8, 17, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        let week_end = chrono::Local
            .with_ymd_and_hms(2026, 8, 24, 0, 0, 0)
            .unwrap()
            .timestamp_millis();

        generate_weekly_digest_from_context(&client, &items, "", week_start, week_end).unwrap();

        let prompt = client.last_prompt();
        assert!(
            prompt.contains("The week being summarized is August 17 - August 23, 2026."),
            "expected the week range in the prompt, got: {prompt}"
        );
    }

    #[test]
    fn generate_weekly_digest_from_context_includes_custom_instructions_in_the_prompt_when_present()
    {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "{\"summary\": \"ok\", \"open_loops\": []}".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };
        let items = vec![ContextItem::Capture(item(
            1,
            "com.apple.Terminal",
            "build passed",
        ))];

        generate_weekly_digest_from_context(&client, &items, "Keep it terse.", 0, 1_000).unwrap();

        assert!(client
            .prompt
            .borrow()
            .contains("Additional instructions: Keep it terse.\n\n"));
    }

    #[test]
    fn detect_date_scope_finds_yesterday_before_today() {
        assert_eq!(
            detect_date_scope("what did I do today vs yesterday"),
            Some(QueryDateScope::Yesterday)
        );
    }

    #[test]
    fn detect_date_scope_finds_today() {
        assert_eq!(
            detect_date_scope("What have I done today?"),
            Some(QueryDateScope::Today)
        );
    }

    #[test]
    fn detect_date_scope_is_case_insensitive() {
        assert_eq!(
            detect_date_scope("TODAY, what happened?"),
            Some(QueryDateScope::Today)
        );
    }

    #[test]
    fn detect_date_scope_is_none_for_an_undated_query() {
        assert_eq!(detect_date_scope("what's the status of the release?"), None);
    }

    #[test]
    fn date_scope_bounds_today_is_local_midnight_through_now() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 27, 14, 0, 0)
            .unwrap()
            .timestamp_millis();
        let midnight = chrono::Local
            .with_ymd_and_hms(2026, 8, 27, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            date_scope_bounds(QueryDateScope::Today, now),
            (midnight, now)
        );
    }

    #[test]
    fn date_scope_bounds_yesterday_is_the_prior_full_calendar_day() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 27, 14, 0, 0)
            .unwrap()
            .timestamp_millis();
        let yesterday_midnight = chrono::Local
            .with_ymd_and_hms(2026, 8, 26, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        let today_midnight = chrono::Local
            .with_ymd_and_hms(2026, 8, 27, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            date_scope_bounds(QueryDateScope::Yesterday, now),
            (yesterday_midnight, today_midnight)
        );
    }

    #[test]
    fn format_date_scope_label_names_the_actual_calendar_date() {
        let midnight = chrono::Local
            .with_ymd_and_hms(2026, 8, 27, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            format_date_scope_label(QueryDateScope::Today, midnight),
            "today, Thursday, August 27, 2026"
        );
    }

    #[test]
    fn answer_with_citations_includes_the_date_scope_framing_when_present() {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "Nothing today [1].".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };
        let context = vec![ContextItem::Capture(item(
            1,
            "com.apple.Terminal",
            "shipped",
        ))];

        answer_with_citations(
            &client,
            &[],
            "what did I do today?",
            &context,
            "",
            Some("today, Thursday, August 27, 2026"),
        )
        .unwrap();

        assert!(client
            .prompt
            .borrow()
            .contains("The question is about today, Thursday, August 27, 2026."));
    }

    #[test]
    fn answer_with_citations_omits_the_date_scope_framing_when_absent() {
        struct PromptCapturingClient {
            prompt: std::cell::RefCell<String>,
        }
        impl LlmClient for PromptCapturingClient {
            fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
                *self.prompt.borrow_mut() = prompt.to_string();
                Ok(Completion {
                    text: "ok [1].".to_string(),
                    token_usage: TokenUsage::default(),
                })
            }
        }
        let client = PromptCapturingClient {
            prompt: std::cell::RefCell::new(String::new()),
        };
        let context = vec![ContextItem::Capture(item(
            1,
            "com.apple.Terminal",
            "shipped",
        ))];

        answer_with_citations(&client, &[], "what's the status?", &context, "", None).unwrap();

        assert!(!client.prompt.borrow().contains("The question is about"));
    }
}
