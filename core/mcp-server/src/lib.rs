//! Local MCP server (FR-MCPS-1..4) — read-only tools over Whimbrel's own
//! memory/meeting data, served over streamable-HTTP-on-localhost. See
//! docs/superpowers/specs/2026-08-02-mcp-server-read-tools-design.md.
//!
//! `Store` holds a `rusqlite::Connection` directly (Send, not Sync), but
//! `rmcp::ServerHandler` requires `Sync` — every tool call takes the mutex,
//! does its query, and releases it. MCP tool calls are human-interaction-paced,
//! not a high-throughput hot path, so a plain blocking mutex is the right
//! amount of concurrency control here, not a channel-based worker actor.
//!
//! `get_daily_digest` needs a `dyn llm::LlmClient` to synthesize uncached digests.
//! `core/llm` deliberately doesn't source API keys itself (see its own module doc) —
//! key material lives behind the macOS Keychain, reached only through `core/ffi`'s
//! C boundary, and provider-chain assembly (`build_chain` in `core/ffi/src/lib.rs`)
//! operates on a private `RoutingConfig` parsed from Settings JSON that only Swift
//! has. None of that is reachable from this crate. So rather than guessing a
//! construction path here, `WhimbrelMcpServer::new` takes an already-constructed
//! `Arc<dyn llm::LlmClient + Send + Sync>` — Task 4's binary entry point (whatever
//! process hosts this server) is responsible for building the concrete client
//! (`OllamaClient` today; hosted-provider fallback chains and spend-cap enforcement
//! are FFI-layer/app policy, out of scope for this read-only tool crate).
//!
//! ## Task 3: per-tool scope enforcement
//!
//! `auth.rs`'s module doc records the real investigation into how a `#[tool]` method
//! learns which bearer token authenticated its call (Path A: rmcp's own per-request
//! `Extensions` mechanism, not the raw-JSON-RPC-body fallback). Every tool method below
//! takes an `Extension(parts): Extension<http::request::Parts>` parameter and calls
//! `self.check_scope(&parts, "<scope>")?` as its first line — the granted-scopes lookup
//! itself, plus what makes a scope check fail distinctly from "no token at all," lives in
//! `auth::authorize`.
//!
//! Scope-per-tool assignment (not specified verbatim anywhere upstream — a real decision,
//! recorded in this task's report): `list_meetings`/`get_meeting` require `"meetings"`;
//! `get_transcript` requires `"transcripts"` (raw meeting speech, the most sensitive of
//! the three); `get_timeline`/`get_daily_digest` require `"memory"` (screen-text/day-view
//! data). `search_memory` requires `"memory"` unconditionally (it's the tool's primary,
//! named scope) but additionally only includes its `meeting_speech` hits (which read
//! `transcript_segment`, the same table `get_transcript` reads) when the token also holds
//! `"transcripts"` — otherwise those legs are silently dropped rather than either leaking
//! meeting speech to a memory-only-scoped client or hard-failing a search that's still
//! partially answerable from screen text alone. `assemble_context` follows the same pattern:
//! requires `"memory"` unconditionally but only includes transcript-sourced items when the
//! token also holds `"transcripts"`, silently omitting them otherwise. Uniquely among
//! `"memory"`-scoped tools, `assemble_context` is the first to expose episode summaries,
//! email-thread data (subject, participants, and LLM summary), and routine-run outputs —
//! data no prior tool under `"memory"` has served.

use rmcp::{
    handler::server::{
        router::tool::ToolRouter,
        tool::Extension,
        wrapper::{Json, Parameters},
    },
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, ServerHandler,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub mod auth;

/// Builds the real `axum::Router` this server is served behind: a `StreamableHttpService`
/// nested under `/mcp`, wrapped by Task 3's `AuthLayer` on the *outer* router (see
/// `auth.rs`'s module doc for why the auth layer can only compose one layer up, after
/// `.nest_service` has already folded the service into a `Router`, not directly with the
/// raw `StreamableHttpService`).
///
/// Both `src/bin/whimbrel-mcp-server.rs`'s `main()` and `tests/real_client_integration.rs`'s
/// `spawn_real_server()` used to build this identical router independently — real
/// duplication that could silently drift (the test could keep passing against a router
/// shape the real binary no longer builds, which is exactly the kind of gap that let two
/// real tool-schema bugs ship past review). Both now call this one function instead; the
/// binary still does its own `TcpListener::bind`/`axum::serve` on the real port, since
/// only the router-building part was ever duplicated.
pub fn build_router(
    store: Arc<Mutex<memory::Store>>,
    llm_client: Arc<dyn llm::LlmClient + Send + Sync>,
) -> axum::Router {
    let session_manager = Arc::new(LocalSessionManager::default());
    let config = StreamableHttpServerConfig::default();
    let service_store = Arc::clone(&store);
    let service_llm = Arc::clone(&llm_client);
    let service = StreamableHttpService::new(
        move || {
            Ok(WhimbrelMcpServer::new(
                Arc::clone(&service_store),
                Arc::clone(&service_llm),
            ))
        },
        session_manager,
        config,
    );
    axum::Router::new()
        .nest_service("/mcp", service)
        .layer(auth::AuthLayer::new(Arc::clone(&store)))
}

pub struct WhimbrelMcpServer {
    store: Arc<Mutex<memory::Store>>,
    llm_client: Arc<dyn llm::LlmClient + Send + Sync>,
    tool_router: ToolRouter<Self>,
}

impl WhimbrelMcpServer {
    pub fn new(
        store: Arc<Mutex<memory::Store>>,
        llm_client: Arc<dyn llm::LlmClient + Send + Sync>,
    ) -> Self {
        Self {
            store,
            llm_client,
            tool_router: Self::tool_router(),
        }
    }

    /// Reads the [`auth::AuthContext`] `AuthMiddleware` stashed into the raw HTTP
    /// request's extensions (see `auth.rs`'s module doc for exactly how it rides along
    /// into a tool method's `Extension<http::request::Parts>` parameter) and checks it
    /// grants `required_scope`. Called as the first line of every `#[tool]` method.
    ///
    /// Takes `&http::request::Parts` rather than the brief's suggested
    /// `&RequestContext` — same information, but `Parts` is trivially constructible in
    /// tests (`Request::builder().body(()).unwrap().into_parts().0`), whereas a real
    /// `RequestContext<RoleServer>` needs a live `rmcp::service::Peer`, only obtainable
    /// through a real connected session (`Peer::new` is `pub(crate)` in rmcp 3.0.1).
    fn check_scope(
        &self,
        parts: &http::request::Parts,
        required_scope: &str,
    ) -> Result<(), ErrorData> {
        auth::authorize(parts.extensions.get::<auth::AuthContext>(), required_scope)
    }

    /// Non-erroring variant for `search_memory`'s optional meeting-speech legs — a
    /// missing/insufficient scope here means "quietly skip these results," not "fail the
    /// whole call," since a memory-only-scoped client still gets a fully valid answer
    /// from screen-text results alone.
    fn has_scope(&self, parts: &http::request::Parts, scope: &str) -> bool {
        parts
            .extensions
            .get::<auth::AuthContext>()
            .is_some_and(|auth| auth.has_scope(scope))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchMemoryParams {
    /// Free-text query; matched by keyword and by embedding similarity.
    pub query: String,
    /// Maximum hits to return (default 20).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchMemoryHit {
    pub domain: String, // "screen_text" or "meeting_speech"
    pub id: i64,
    pub meeting_id: Option<i64>,
    pub meeting_title: Option<String>,
    pub text: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchMemoryOutput {
    pub results: Vec<SearchMemoryHit>,
    /// How many meetings' calendar-derived text (title, attendees, summary) was left out
    /// (see [`GOOGLE_DATA_SCOPE`]); 0 when none was or the token carries the scope.
    pub google_data_withheld: usize,
    /// Present when `google_data_withheld` is not 0: says what was left out and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListMeetingsParams {
    /// Maximum meetings to return, newest first (default 20).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetMeetingParams {
    /// Meeting id from list_meetings or a search_memory hit.
    pub meeting_id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTranscriptParams {
    /// Meeting id from list_meetings or a search_memory hit.
    pub meeting_id: i64,
    /// Zero-based index of the first segment to return (default 0).
    #[serde(default)]
    pub offset: usize,
    /// Maximum segments to return (default 20).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetTranscriptOutput {
    pub segments: Vec<memory::MeetingTranscriptSegment>,
    pub total: usize,
    /// How many meetings' calendar-derived text (title, attendees, summary) was left out
    /// (see [`GOOGLE_DATA_SCOPE`]); 0 when none was or the token carries the scope.
    pub google_data_withheld: usize,
    /// Present when `google_data_withheld` is not 0: says what was left out and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTimelineParams {
    /// Range start, Unix milliseconds (inclusive).
    pub start_unix_ms: i64,
    /// Range end, Unix milliseconds (exclusive).
    pub end_unix_ms: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetDailyDigestParams {
    /// "YYYY-MM-DD" — the day to summarize; also the cache key.
    pub day: String,
    /// Start of that day, Unix milliseconds (inclusive).
    pub start_unix_ms: i64,
    /// End of that day, Unix milliseconds (exclusive).
    pub end_unix_ms: i64,
}

// The MCP spec requires each tool's outputSchema to be an object schema at the top
// level (structured tool results are returned as a JSON object) — a bare Vec/Option
// produces an array/nullable-typed schema instead, which real external clients (e.g.
// Claude Code) reject as malformed. Every tool below returns one of these named,
// single-field wrapper structs rather than a bare collection/option, found via a real
// live-client connection during Task 8's exit-gate verification (search_memory and
// get_transcript were already correct — they already returned a named struct).
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ListMeetingsOutput {
    pub meetings: Vec<memory::Meeting>,
    /// How many meetings' calendar-derived text (title, attendees, summary) was left out
    /// (see [`GOOGLE_DATA_SCOPE`]); 0 when none was or the token carries the scope.
    pub google_data_withheld: usize,
    /// Present when `google_data_withheld` is not 0: says what was left out and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetMeetingOutput {
    pub meeting: Option<memory::Meeting>,
    /// How many meetings' calendar-derived text (title, attendees, summary) was left out
    /// (see [`GOOGLE_DATA_SCOPE`]); 0 when none was or the token carries the scope.
    pub google_data_withheld: usize,
    /// Present when `google_data_withheld` is not 0: says what was left out and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetTimelineOutput {
    pub items: Vec<memory::CaptureItem>,
}

// serde_json::Value has no fixed shape, so schemars emits an unconstrained/empty
// schema for it — real external clients (Claude Code's own schema validator) reject
// that as an invalid property schema. llm::DailyDigest is the real, concrete type the
// cached JSON always deserializes to, so use it directly instead.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetDailyDigestOutput {
    pub digest: llm::DailyDigest,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AssembleContextParams {
    /// The question or topic to gather context for.
    pub query: String,
    /// Maximum context items in the packed list (default 20).
    #[serde(default = "default_limit")]
    pub context_limit: usize,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AssembleContextItem {
    #[serde(flatten)]
    pub reference: llm::ContextItemRef,
    pub label: String,
    pub text: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AssembleContextOutput {
    pub items: Vec<AssembleContextItem>,
    /// How many items were left out because they hold Gmail or Google Calendar text (see
    /// [`GOOGLE_DATA_SCOPE`]); 0 when none were or the token carries the scope.
    pub google_data_withheld: usize,
    /// Present when `google_data_withheld` is not 0: says what was left out and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The token scope that lets `assemble_context` return items built from Gmail or Google
/// Calendar. The MCP client is an app outside the AI providers the user chose in Whimbrel,
/// and Google's Limited Use rules allow that text only to a model that does not train on
/// it, which Whimbrel cannot check here. So it is off unless the user grants this scope to
/// that one client; no token carries it by default.
pub const GOOGLE_DATA_SCOPE: &str = "google_data";

/// The `note` for a tool that left calendar-derived meeting text out.
fn calendar_text_note(withheld: usize) -> Option<String> {
    (withheld > 0).then(|| {
        format!(
            "Title, attendees and summary of {withheld} meeting(s) that may have come from Google Calendar were left out. \
             Whimbrel does not send them to MCP clients unless the client's token has the `{GOOGLE_DATA_SCOPE}` scope."
        )
    })
}

/// Limited Use: leaves out the text of `meeting` that came from, or was written with, Google
/// Calendar: the title, the attendees and the summary. Returns whether it did.
fn withhold_calendar_text(store: &memory::Store, meeting: &mut memory::Meeting) -> bool {
    if !store.meeting_may_hold_google_calendar_data(meeting.id) {
        return false;
    }
    meeting.title = llm::limited_use::WITHHELD_TITLE.to_string();
    meeting.attendees.clear();
    meeting.summary_tldr = None;
    meeting.summary_decisions.clear();
    meeting.summary_action_items.clear();
    meeting.summary_open_questions.clear();
    true
}

#[tool_router]
impl WhimbrelMcpServer {
    #[tool(
        name = "search_memory",
        description = "Hybrid semantic+keyword search over everything Whimbrel has captured on this Mac: OCR'd screen text and transcribed meeting speech, ranked together. Each hit carries a `domain` (\"screen_text\" or \"meeting_speech\"), the row `id`, and for speech hits the `meeting_id`/`meeting_title` to pass to get_meeting or get_transcript. Returns raw matching snippets only; use assemble_context instead when you want one ranked, budget-packed context list that also folds in recent episodes, threads, and routine runs. For a meeting that may have come from Google Calendar, the title, attendees and summary are left out unless the client's token has the `google_data` scope; `google_data_withheld` and `note` say so. Read-only, no LLM cost."
    )]
    async fn search_memory(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<SearchMemoryParams>,
    ) -> Result<Json<SearchMemoryOutput>, ErrorData> {
        self.check_scope(&parts, "memory")?;
        let include_transcripts = self.has_scope(&parts, "transcripts");
        let allow_google = self.has_scope(&parts, GOOGLE_DATA_SCOPE);
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let mut results = Vec::new();
        for item in store
            .search_keyword(&params.query, params.limit)
            .unwrap_or_default()
        {
            results.push(SearchMemoryHit {
                domain: "screen_text".to_string(),
                id: item.id,
                meeting_id: None,
                meeting_title: None,
                text: item.text,
            });
        }
        for item in store
            .search_semantic(&params.query, params.limit)
            .unwrap_or_default()
        {
            results.push(SearchMemoryHit {
                domain: "screen_text".to_string(),
                id: item.id,
                meeting_id: None,
                meeting_title: None,
                text: item.text,
            });
        }
        // Meeting speech lives in the same `transcript_segment` table `get_transcript`
        // reads, which requires "transcripts" scope — so these two legs only run if this
        // token was granted that too, rather than letting search_memory bypass the
        // per-tool scope boundary get_transcript itself enforces.
        if include_transcripts {
            for seg in store
                .search_transcript_keyword(&params.query, params.limit)
                .unwrap_or_default()
            {
                results.push(SearchMemoryHit {
                    domain: "meeting_speech".to_string(),
                    id: seg.id,
                    meeting_id: Some(seg.meeting_id),
                    meeting_title: Some(seg.meeting_title),
                    text: seg.text,
                });
            }
            for seg in store
                .search_transcript_semantic(&params.query, params.limit)
                .unwrap_or_default()
            {
                results.push(SearchMemoryHit {
                    domain: "meeting_speech".to_string(),
                    id: seg.id,
                    meeting_id: Some(seg.meeting_id),
                    meeting_title: Some(seg.meeting_title),
                    text: seg.text,
                });
            }
        }
        results.truncate(params.limit.max(1) * 4);
        // Limited Use: a speech hit keeps its words, but not the title of a meeting that
        // may have come from Google Calendar.
        let mut google_data_withheld = 0;
        if !allow_google {
            let mut seen = std::collections::HashSet::new();
            for hit in &mut results {
                if let Some(meeting_id) = hit.meeting_id {
                    if store.meeting_may_hold_google_calendar_data(meeting_id) {
                        hit.meeting_title = Some(llm::limited_use::WITHHELD_TITLE.to_string());
                        seen.insert(meeting_id);
                    }
                }
            }
            google_data_withheld = seen.len();
        }
        Ok(Json(SearchMemoryOutput { results, google_data_withheld, note: calendar_text_note(google_data_withheld) }))
    }

    #[tool(
        name = "list_meetings",
        description = "List recent meetings, newest first, with id, title, status, timestamps, and the generated summary fields when a summary exists. Includes meetings still recording. For a meeting that may have come from Google Calendar, the title, attendees and summary are left out unless the client's token has the `google_data` scope; `google_data_withheld` and `note` say so. Read-only, no LLM cost."
    )]
    async fn list_meetings(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<ListMeetingsParams>,
    ) -> Result<Json<ListMeetingsOutput>, ErrorData> {
        self.check_scope(&parts, "meetings")?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let mut meetings = store
            .recent_meetings(params.limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let google_data_withheld = if self.has_scope(&parts, GOOGLE_DATA_SCOPE) {
            0
        } else {
            meetings.iter_mut().map(|m| withhold_calendar_text(&store, m)).filter(|&hit| hit).count()
        };
        Ok(Json(ListMeetingsOutput { meetings, google_data_withheld, note: calendar_text_note(google_data_withheld) }))
    }

    #[tool(
        name = "get_meeting",
        description = "Fetch one meeting's metadata and generated summary (tldr, decisions, action items, open questions) by id. Returns `meeting: null` for an unknown id rather than an error. Does not include the transcript; use get_transcript for that. For a meeting that may have come from Google Calendar, the title, attendees and summary are left out unless the client's token has the `google_data` scope; `google_data_withheld` and `note` say so. Read-only, no LLM cost."
    )]
    async fn get_meeting(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<GetMeetingParams>,
    ) -> Result<Json<GetMeetingOutput>, ErrorData> {
        self.check_scope(&parts, "meetings")?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let mut meeting = store
            .get_meeting(params.meeting_id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let google_data_withheld = match &mut meeting {
            Some(m) if !self.has_scope(&parts, GOOGLE_DATA_SCOPE) => usize::from(withhold_calendar_text(&store, m)),
            _ => 0,
        };
        Ok(Json(GetMeetingOutput { meeting, google_data_withheld, note: calendar_text_note(google_data_withheld) }))
    }

    #[tool(
        name = "get_transcript",
        description = "Fetch one page of a meeting's transcript segments in chronological order, with `total` for paging (offset + limit, default limit 20). Segments are raw speech-to-text, unsummarized. For a meeting that may have come from Google Calendar, the title, attendees and summary are left out unless the client's token has the `google_data` scope; `google_data_withheld` and `note` say so. Read-only, no LLM cost."
    )]
    async fn get_transcript(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<GetTranscriptParams>,
    ) -> Result<Json<GetTranscriptOutput>, ErrorData> {
        self.check_scope(&parts, "transcripts")?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let (mut segments, total) = store
            .get_transcript_segments_page(params.meeting_id, params.offset, params.limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        // Limited Use: the words stay; the title of a meeting that may have come from
        // Google Calendar does not.
        let mut google_data_withheld = 0;
        if !segments.is_empty()
            && !self.has_scope(&parts, GOOGLE_DATA_SCOPE)
            && store.meeting_may_hold_google_calendar_data(params.meeting_id)
        {
            for seg in &mut segments {
                seg.meeting_title = llm::limited_use::WITHHELD_TITLE.to_string();
            }
            google_data_withheld = 1;
        }
        Ok(Json(GetTranscriptOutput { segments, total, google_data_withheld, note: calendar_text_note(google_data_withheld) }))
    }

    #[tool(
        name = "get_timeline",
        description = "Captured screen-text items (app bundle id + OCR'd text) between two Unix-millisecond timestamps, in time order. Screen text only; meeting speech is not included. Read-only, no LLM cost."
    )]
    async fn get_timeline(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<GetTimelineParams>,
    ) -> Result<Json<GetTimelineOutput>, ErrorData> {
        self.check_scope(&parts, "memory")?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let items = store
            .timeline_range(params.start_unix_ms, params.end_unix_ms)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(GetTimelineOutput { items }))
    }

    #[tool(
        name = "get_daily_digest",
        description = "LLM-written end-of-day digest (summary + open loops) for one day. The first call for a day makes a real model call over that day's captured screen-text items; later calls for the same day return the cached digest with no model call. `day` is the cache key; `start_unix_ms`/`end_unix_ms` bound which items are summarized and should cover that same day."
    )]
    async fn get_daily_digest(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<GetDailyDigestParams>,
    ) -> Result<Json<GetDailyDigestOutput>, ErrorData> {
        self.check_scope(&parts, "memory")?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        if let Some(cached) = store
            .get_cached_daily_digest(&params.day)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
        {
            let digest: llm::DailyDigest = serde_json::from_str(&cached)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            return Ok(Json(GetDailyDigestOutput { digest }));
        }
        let items = store
            .timeline_range(params.start_unix_ms, params.end_unix_ms)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        drop(store); // release the lock before the (potentially slow) LLM call

        let digest = llm::generate_daily_digest(self.llm_client.as_ref(), &items, "")
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let digest_json = serde_json::to_string(&digest)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let now = chrono::Utc::now().timestamp_millis();
        store
            .cache_daily_digest(&params.day, &digest_json, now)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(GetDailyDigestOutput { digest }))
    }

    #[tool(
        name = "assemble_context",
        description = "Unified retrieval for answering a question: hybrid keyword+semantic search over screen text and meeting speech, fused with recent episodes, email threads needing attention, and routine outputs, packed into one ranked context list of at most `context_limit` items (default 20). Each item has a `label` (its kind and source) and `text`. Prefer this over search_memory when the goal is to answer or summarize rather than to locate a specific row. Items built from Gmail or Google Calendar (email threads and routine outputs, and the titles of calendar meetings) are left out unless the client's token has the `google_data` scope; `google_data_withheld` and `note` say how many. Read-only, no LLM cost."
    )]
    async fn assemble_context(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AssembleContextParams>,
    ) -> Result<Json<AssembleContextOutput>, ErrorData> {
        self.check_scope(&parts, "memory")?;
        let include_transcripts = self.has_scope(&parts, "transcripts");
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let now = chrono::Utc::now().timestamp_millis();
        let context = llm::gather_and_assemble_context(
            &store,
            &params.query,
            now,
            params.context_limit,
            include_transcripts,
            // Raw email text is not part of the "memory" scope and no token scope grants
            // it yet, so MCP clients keep getting thread summaries only.
            false,
        );
        // Limited Use: Gmail threads and the routine output written from Gmail and Calendar
        // are Google user data. Leave them out unless this client's token opts in.
        let allow_google = self.has_scope(&parts, GOOGLE_DATA_SCOPE);
        let total = context.len();
        let context = llm::limited_use::filter_context_in(&store, &context, allow_google);
        let google_data_withheld = total - context.len();
        let note = (google_data_withheld > 0).then(|| {
            format!(
                "{google_data_withheld} item(s) built from Gmail or Google Calendar were left out. \
                 Whimbrel does not send them to MCP clients unless the client's token has the \
                 `{GOOGLE_DATA_SCOPE}` scope."
            )
        });
        let items = context
            .into_iter()
            .map(|item| AssembleContextItem {
                reference: item.citation_ref(),
                label: item.display_label(),
                text: item.text().to_string(),
            })
            .collect();
        Ok(Json(AssembleContextOutput { items, google_data_withheld, note }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for WhimbrelMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            rmcp::model::Implementation::new("whimbrel-mcp-server", env!("CARGO_PKG_VERSION")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// No real LLM calls are exercised by these tool-handler tests (`get_daily_digest`
    /// only reaches the client on a cache miss, and none of the tests below hit that
    /// path) — this stands in for a real client just to satisfy `WhimbrelMcpServer::new`.
    struct NoopLlmClient;

    impl llm::LlmClient for NoopLlmClient {
        fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            Err(llm::LlmError::NotImplemented(
                "NoopLlmClient is test-only and never expected to be called",
            ))
        }
    }

    /// Builds the `Store` first so test setup can insert fixture rows directly through
    /// it, then hands the same `Arc<Mutex<Store>>` to `WhimbrelMcpServer::new` — avoids
    /// adding any test-only accessor to `WhimbrelMcpServer`'s private fields.
    fn test_server_with_store() -> (WhimbrelMcpServer, Arc<Mutex<memory::Store>>) {
        let store = Arc::new(Mutex::new(memory::Store::open_in_memory().unwrap()));
        let server = WhimbrelMcpServer::new(Arc::clone(&store), Arc::new(NoopLlmClient));
        (server, store)
    }

    fn test_server() -> WhimbrelMcpServer {
        test_server_with_store().0
    }

    /// Builds a fake `http::request::Parts` carrying an `auth::AuthContext` with the
    /// given scopes — the exact two-level shape (`Parts` holding its own
    /// `.extensions: http::Extensions`, itself holding `AuthContext`) that
    /// `AuthMiddleware` produces on the real request path (see `auth.rs`'s module doc).
    /// No live `rmcp::service::Peer`/session needed — `check_scope`/`has_scope` only ever
    /// read `parts.extensions`, never anything peer- or session-related, so this is a
    /// faithful test of the real extraction path, not a shortcut around it.
    fn parts_with_scopes(scopes: &[&str]) -> http::request::Parts {
        let (mut parts, _body) = http::Request::builder().body(()).unwrap().into_parts();
        parts.extensions.insert(auth::AuthContext {
            client_id: 1,
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
        });
        parts
    }

    /// `Parts` with no `AuthContext` at all — simulates a request that somehow reached a
    /// tool method without going through `AuthMiddleware` (misconfiguration).
    fn parts_with_no_auth() -> http::request::Parts {
        http::Request::builder().body(()).unwrap().into_parts().0
    }

    #[tokio::test]
    async fn list_meetings_returns_empty_vec_for_a_fresh_store() {
        let server = test_server();
        let Json(output) = server
            .list_meetings(
                Extension(parts_with_scopes(&["meetings"])),
                Parameters(ListMeetingsParams { limit: 10 }),
            )
            .await
            .unwrap();
        assert!(output.meetings.is_empty());
    }

    #[tokio::test]
    async fn list_meetings_rejects_a_token_without_the_meetings_scope() {
        let server = test_server();
        let err = server
            .list_meetings(
                Extension(parts_with_scopes(&["memory"])),
                Parameters(ListMeetingsParams { limit: 10 }),
            )
            .await
            .err()
            .unwrap();
        assert!(err.message.contains("not authorized for scope"));
    }

    #[tokio::test]
    async fn list_meetings_rejects_a_request_with_no_auth_context_at_all() {
        let server = test_server();
        let err = server
            .list_meetings(
                Extension(parts_with_no_auth()),
                Parameters(ListMeetingsParams { limit: 10 }),
            )
            .await
            .err()
            .unwrap();
        assert!(
            !err.message.contains("not authorized for scope"),
            "missing-token and wrong-scope must be distinct errors"
        );
    }

    #[tokio::test]
    async fn get_meeting_returns_none_for_an_unknown_id() {
        let server = test_server();
        let Json(output) = server
            .get_meeting(
                Extension(parts_with_scopes(&["meetings"])),
                Parameters(GetMeetingParams { meeting_id: 999 }),
            )
            .await
            .unwrap();
        assert!(output.meeting.is_none());
    }

    #[tokio::test]
    async fn get_transcript_returns_the_right_page_and_total() {
        let (server, store) = test_server_with_store();
        let meeting_id = {
            let store = store.lock().unwrap();
            let id = store
                .insert_meeting(&memory::NewMeeting {
                    device_id: "test-device".to_string(),
                    start_unix_ms: 1000,
                    end_unix_ms: None,
                    title: "Standup".to_string(),
                    attendees: vec![],
                    trigger: memory::MeetingTrigger::Manual,
                    audio_path: None,
                    status: memory::MeetingStatus::Recording,
                })
                .unwrap();
            store
                .insert_transcript_segments(&[
                    memory::NewTranscriptSegment {
                        meeting_id: id,
                        start_cs: 0,
                        speaker_label: "Me".to_string(),
                        text: "hello".to_string(),
                    },
                    memory::NewTranscriptSegment {
                        meeting_id: id,
                        start_cs: 100,
                        speaker_label: "Me".to_string(),
                        text: "world".to_string(),
                    },
                ])
                .unwrap();
            id
        };
        let Json(result) = server
            .get_transcript(
                Extension(parts_with_scopes(&["transcripts"])),
                Parameters(GetTranscriptParams {
                    meeting_id,
                    offset: 0,
                    limit: 10,
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.total, 2);
        assert_eq!(result.segments.len(), 2);
    }

    #[tokio::test]
    async fn get_transcript_rejects_a_token_scoped_only_for_meetings() {
        let (server, store) = test_server_with_store();
        let meeting_id = {
            let store = store.lock().unwrap();
            store
                .insert_meeting(&memory::NewMeeting {
                    device_id: "test-device".to_string(),
                    start_unix_ms: 1000,
                    end_unix_ms: None,
                    title: "Standup".to_string(),
                    attendees: vec![],
                    trigger: memory::MeetingTrigger::Manual,
                    audio_path: None,
                    status: memory::MeetingStatus::Recording,
                })
                .unwrap()
        };
        let err = server
            .get_transcript(
                Extension(parts_with_scopes(&["meetings"])),
                Parameters(GetTranscriptParams {
                    meeting_id,
                    offset: 0,
                    limit: 10,
                }),
            )
            .await
            .err()
            .unwrap();
        assert!(err.message.contains("not authorized for scope"));
    }

    #[tokio::test]
    async fn search_memory_finds_an_inserted_capture_item_by_keyword() {
        let (server, store) = test_server_with_store();
        {
            let store = store.lock().unwrap();
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "test-device".to_string(),
                    ts_unix_ms: 1000,
                    app_bundle_id: "com.apple.Safari".to_string(),
                    window_title: "Some Window".to_string(),
                    url: None,
                    text: "the quick brown fox".to_string(),
                    hash: 1,
                    source: "ax".to_string(),
                })
                .unwrap();
        }
        let Json(result) = server
            .search_memory(
                Extension(parts_with_scopes(&["memory"])),
                Parameters(SearchMemoryParams {
                    query: "fox".to_string(),
                    limit: 20,
                }),
            )
            .await
            .unwrap();
        assert!(result
            .results
            .iter()
            .any(|hit| hit.domain == "screen_text" && hit.text.contains("fox")));
    }

    #[tokio::test]
    async fn search_memory_omits_meeting_speech_hits_without_the_transcripts_scope() {
        let (server, store) = test_server_with_store();
        let meeting_id = {
            let store = store.lock().unwrap();
            let id = store
                .insert_meeting(&memory::NewMeeting {
                    device_id: "test-device".to_string(),
                    start_unix_ms: 1000,
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
                    meeting_id: id,
                    start_cs: 0,
                    speaker_label: "Me".to_string(),
                    text: "the quick brown fox".to_string(),
                }])
                .unwrap();
            id
        };

        // memory-only scope: meeting speech is silently dropped, not a hard failure.
        let Json(memory_only) = server
            .search_memory(
                Extension(parts_with_scopes(&["memory"])),
                Parameters(SearchMemoryParams {
                    query: "fox".to_string(),
                    limit: 20,
                }),
            )
            .await
            .unwrap();
        assert!(memory_only
            .results
            .iter()
            .all(|hit| hit.domain != "meeting_speech"));

        // memory + transcripts: the same meeting-speech hit is now included.
        let Json(with_transcripts) = server
            .search_memory(
                Extension(parts_with_scopes(&["memory", "transcripts"])),
                Parameters(SearchMemoryParams {
                    query: "fox".to_string(),
                    limit: 20,
                }),
            )
            .await
            .unwrap();
        assert!(with_transcripts
            .results
            .iter()
            .any(|hit| hit.domain == "meeting_speech" && hit.meeting_id == Some(meeting_id)));
    }

    #[tokio::test]
    async fn assemble_context_rejects_a_token_without_the_memory_scope() {
        let server = test_server();
        let err = server
            .assemble_context(
                Extension(parts_with_scopes(&["meetings"])),
                Parameters(AssembleContextParams {
                    query: "fox".to_string(),
                    context_limit: 20,
                }),
            )
            .await
            .err()
            .unwrap();
        assert!(err.message.contains("not authorized for scope"));
    }

    #[tokio::test]
    async fn assemble_context_finds_an_inserted_capture_item_by_keyword() {
        let (server, store) = test_server_with_store();
        {
            let store = store.lock().unwrap();
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "test-device".to_string(),
                    ts_unix_ms: 1000,
                    app_bundle_id: "com.apple.Safari".to_string(),
                    window_title: "Some Window".to_string(),
                    url: None,
                    text: "the quick brown fox".to_string(),
                    hash: 1,
                    source: "ax".to_string(),
                })
                .unwrap();
        }
        let Json(result) = server
            .assemble_context(
                Extension(parts_with_scopes(&["memory"])),
                Parameters(AssembleContextParams {
                    query: "fox".to_string(),
                    context_limit: 20,
                }),
            )
            .await
            .unwrap();
        let hit = result.items.iter().find(|i| {
            matches!(i.reference, llm::ContextItemRef::Capture { .. }) && i.text.contains("fox")
        });
        assert!(
            hit.is_some(),
            "expected a Capture item mentioning 'fox', got {:?}",
            result.items
        );
    }

    #[tokio::test]
    async fn assemble_context_omits_transcript_sourced_items_without_the_transcripts_scope() {
        let (server, store) = test_server_with_store();
        {
            let store = store.lock().unwrap();
            let id = store
                .insert_meeting(&memory::NewMeeting {
                    device_id: "test-device".to_string(),
                    start_unix_ms: 1000,
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
                    meeting_id: id,
                    start_cs: 0,
                    speaker_label: "Me".to_string(),
                    text: "the quick brown fox".to_string(),
                }])
                .unwrap();
        }

        // memory-only scope: transcript-sourced items are silently omitted, not a hard failure.
        let Json(memory_only) = server
            .assemble_context(
                Extension(parts_with_scopes(&["memory"])),
                Parameters(AssembleContextParams {
                    query: "fox".to_string(),
                    context_limit: 20,
                }),
            )
            .await
            .unwrap();
        assert!(memory_only
            .items
            .iter()
            .all(|i| !matches!(i.reference, llm::ContextItemRef::Transcript { .. })));

        // memory + transcripts: the same transcript-sourced item is now included.
        let Json(with_transcripts) = server
            .assemble_context(
                Extension(parts_with_scopes(&["memory", "transcripts"])),
                Parameters(AssembleContextParams {
                    query: "fox".to_string(),
                    context_limit: 20,
                }),
            )
            .await
            .unwrap();
        assert!(with_transcripts
            .items
            .iter()
            .any(|i| matches!(i.reference, llm::ContextItemRef::Transcript { .. })));
    }

    /// Limited Use: Gmail threads and routine output built from Gmail or Calendar stay
    /// out of what an external MCP client receives unless its token carries `google_data`,
    /// and the output says that something was left out.
    #[tokio::test]
    async fn assemble_context_withholds_google_derived_items_without_the_google_data_scope() {
        let (server, store) = test_server_with_store();
        {
            let store = store.lock().unwrap();
            store
                .upsert_email_items(&[memory::NewEmailItem {
                    device_id: "test-device".into(),
                    external_id: "m-1".into(),
                    ts_unix_ms: chrono::Utc::now().timestamp_millis() - 1_000,
                    from: "Priya <priya@x.com>".into(),
                    to: "me@x.com".into(),
                    subject: "Falcon budget".into(),
                    body_text: "GMAILSECRET the Falcon budget is 40k".into(),
                    gmail_thread_id: "thread-1".into(),
                    account_email: "me@x.com".into(),
                    ..Default::default()
                }])
                .unwrap();
            while store.run_thread_state_sync_job(chrono::Utc::now().timestamp_millis()).unwrap() {}
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "test-device".to_string(),
                    ts_unix_ms: chrono::Utc::now().timestamp_millis() - 2_000,
                    app_bundle_id: "com.apple.Safari".to_string(),
                    window_title: "Some Window".to_string(),
                    url: None,
                    text: "Falcon budget spreadsheet on screen".to_string(),
                    hash: 1,
                    source: "ax".to_string(),
                })
                .unwrap();
            let routine = store
                .insert_routine(&memory::NewRoutine {
                    name: "Daily Brief".into(),
                    template: memory::RoutineTemplate::DailyBrief,
                    schedule_hhmm: "08:00".into(),
                    enabled: true,
                })
                .unwrap();
            store
                .insert_routine_run(&memory::NewRoutineRun {
                    routine_id: routine,
                    ran_at_unix_ms: chrono::Utc::now().timestamp_millis() - 3_000,
                    status: memory::RunStatus::Success,
                    output_summary: "ROUTINEBRIEF Falcon budget thread from Priya".into(),
                    output_json: "{}".into(),
                })
                .unwrap();
        }
        let server = &server;
        let call = |scopes: &'static [&'static str]| {
            async move {
                let Json(out) = server
                    .assemble_context(
                        Extension(parts_with_scopes(scopes)),
                        Parameters(AssembleContextParams { query: "Falcon budget".to_string(), context_limit: 20 }),
                    )
                    .await
                    .unwrap();
                out
            }
        };
        let is_google = |i: &AssembleContextItem| {
            matches!(i.reference, llm::ContextItemRef::Thread { .. } | llm::ContextItemRef::RoutineRun { .. })
        };

        let held = call(&["memory"]).await;
        assert!(held.items.iter().any(|i| i.text.contains("Falcon budget spreadsheet")), "screen text still flows");
        assert!(held.items.iter().all(|i| !is_google(i)), "{:?}", held.items);
        assert!(held.google_data_withheld >= 2, "{}", held.google_data_withheld);
        assert!(held.note.as_deref().is_some_and(|n| n.contains("Gmail") && n.contains("google_data")));

        let allowed = call(&["memory", "google_data"]).await;
        assert!(allowed.items.iter().any(|i| matches!(i.reference, llm::ContextItemRef::Thread { .. })));
        assert!(allowed.items.iter().any(|i| matches!(i.reference, llm::ContextItemRef::RoutineRun { .. })));
        assert_eq!(allowed.google_data_withheld, 0);
        assert!(allowed.note.is_none());
    }

    /// A calendar meeting of unknown source (so Google Calendar text): title "Zebra Review",
    /// attendee "Ana Zebrowski", a summary, one transcript segment saying "fox".
    fn insert_google_meeting(store: &Arc<Mutex<memory::Store>>) -> i64 {
        let store = store.lock().unwrap();
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: Some(2000),
                title: "Zebra Review".to_string(),
                attendees: vec!["Ana Zebrowski".to_string()],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        store.update_meeting_summary(id, "Zebra TLDR", &[], &[], &[], "ollama", "m").unwrap();
        store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id: id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "the quick brown fox".to_string(),
            }])
            .unwrap();
        id
    }

    fn debug_has_no_calendar_text<T: std::fmt::Debug>(out: &T) {
        let text = format!("{out:?}");
        for word in ["Zebra Review", "Zebrowski", "Zebra TLDR"] {
            assert!(!text.contains(word), "{word} reached the client: {text}");
        }
    }

    /// Limited Use: the meeting tools hold a calendar meeting's title, attendees and
    /// summary (Google Calendar text) unless the token has `google_data`.
    #[tokio::test]
    async fn meeting_tools_withhold_google_calendar_text_without_the_google_data_scope() {
        let (server, store) = test_server_with_store();
        let id = insert_google_meeting(&store);
        let plain = || parts_with_scopes(&["memory", "meetings", "transcripts"]);
        let opted = || parts_with_scopes(&["memory", "meetings", "transcripts", "google_data"]);

        let Json(list) = server.list_meetings(Extension(plain()), Parameters(ListMeetingsParams { limit: 10 })).await.unwrap();
        debug_has_no_calendar_text(&list.meetings);
        assert_eq!(list.meetings.len(), 1, "the meeting itself is listed");
        assert_eq!(list.google_data_withheld, 1);
        assert!(list.note.is_some());

        let Json(one) = server.get_meeting(Extension(plain()), Parameters(GetMeetingParams { meeting_id: id })).await.unwrap();
        debug_has_no_calendar_text(&one.meeting);
        assert_eq!((one.meeting.as_ref().map(|m| m.id), one.google_data_withheld), (Some(id), 1));

        let Json(tr) = server
            .get_transcript(Extension(plain()), Parameters(GetTranscriptParams { meeting_id: id, offset: 0, limit: 10 }))
            .await
            .unwrap();
        debug_has_no_calendar_text(&tr.segments);
        assert_eq!(tr.segments.len(), 1);
        assert_eq!(tr.google_data_withheld, 1);

        let Json(found) = server
            .search_memory(Extension(plain()), Parameters(SearchMemoryParams { query: "fox".into(), limit: 10 }))
            .await
            .unwrap();
        debug_has_no_calendar_text(&found.results);
        assert!(found.results.iter().any(|h| h.domain == "meeting_speech"));
        assert!(found.google_data_withheld >= 1 && found.note.is_some());

        let Json(ctx) = server
            .assemble_context(Extension(plain()), Parameters(AssembleContextParams { query: "fox".into(), context_limit: 10 }))
            .await
            .unwrap();
        debug_has_no_calendar_text(&ctx.items);
        assert!(ctx.items.iter().any(|i| i.text.contains("fox")));

        // With the scope, everything flows and nothing is reported withheld.
        let Json(list) = server.list_meetings(Extension(opted()), Parameters(ListMeetingsParams { limit: 10 })).await.unwrap();
        assert_eq!((list.meetings[0].title.as_str(), list.google_data_withheld, list.note.is_none()), ("Zebra Review", 0, true));
        let Json(one) = server.get_meeting(Extension(opted()), Parameters(GetMeetingParams { meeting_id: id })).await.unwrap();
        assert_eq!(one.meeting.unwrap().attendees, vec!["Ana Zebrowski".to_string()]);
        let Json(tr) = server
            .get_transcript(Extension(opted()), Parameters(GetTranscriptParams { meeting_id: id, offset: 0, limit: 10 }))
            .await
            .unwrap();
        assert_eq!(tr.segments[0].meeting_title, "Zebra Review");
    }

    /// A meeting recorded as coming from another calendar is not Google data.
    #[tokio::test]
    async fn meeting_tools_pass_a_meeting_from_another_calendar() {
        let (server, store) = test_server_with_store();
        let id = insert_google_meeting(&store);
        store.lock().unwrap().set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_OTHER).unwrap();
        let Json(one) = server
            .get_meeting(Extension(parts_with_scopes(&["meetings"])), Parameters(GetMeetingParams { meeting_id: id }))
            .await
            .unwrap();
        assert_eq!((one.meeting.unwrap().title.as_str(), one.google_data_withheld), ("Zebra Review", 0));
    }

    #[tokio::test]
    async fn get_timeline_returns_items_within_range() {
        let (server, store) = test_server_with_store();
        {
            let store = store.lock().unwrap();
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "test-device".to_string(),
                    ts_unix_ms: 5000,
                    app_bundle_id: "com.apple.Safari".to_string(),
                    window_title: "Some Window".to_string(),
                    url: None,
                    text: "in range".to_string(),
                    hash: 1,
                    source: "ax".to_string(),
                })
                .unwrap();
        }
        let Json(output) = server
            .get_timeline(
                Extension(parts_with_scopes(&["memory"])),
                Parameters(GetTimelineParams {
                    start_unix_ms: 0,
                    end_unix_ms: 10_000,
                }),
            )
            .await
            .unwrap();
        assert_eq!(output.items.len(), 1);
        assert_eq!(output.items[0].text, "in range");
    }

    #[tokio::test]
    async fn get_daily_digest_returns_cached_value_without_calling_the_llm_client() {
        let (server, store) = test_server_with_store();
        {
            let store = store.lock().unwrap();
            store.cache_daily_digest("2026-08-02", r#"{"summary":"cached","open_loops":[],"token_usage":{"input_tokens":null,"output_tokens":null}}"#, 1234).unwrap();
        }
        let Json(output) = server
            .get_daily_digest(
                Extension(parts_with_scopes(&["memory"])),
                Parameters(GetDailyDigestParams {
                    day: "2026-08-02".to_string(),
                    start_unix_ms: 0,
                    end_unix_ms: 1,
                }),
            )
            .await
            .unwrap();
        assert_eq!(output.digest.summary, "cached");
    }
}
