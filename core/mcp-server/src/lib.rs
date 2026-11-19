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
//!
//! ## Clients excluded from hosted models (X6, spec 7.3)
//!
//! Tool results go to outside AI tools, so every tool treats them as hosted: material filed
//! under a client excluded from hosted models, or naming one, is never returned
//! (`llm::ContextFilter`, read from the store on each call; an unreadable setting returns
//! nothing). An excluded client's meeting reads as unknown (`get_meeting` null, `get_transcript`
//! empty). The daily digest is built from the day without that material and cached with the
//! exclusion state it was built under; a cached digest is served only under the same state (and
//! naming no excluded client), the state is read again after the model call before anything is
//! cached, and any change to what is excluded drops the cache (triggers in `memory`).

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
    let filter_store = Arc::clone(&store);
    build_router_with_filter_store(store, filter_store, llm_client)
}

/// [`build_router`] with the exclusion filter read through its own connection (`filter_store`),
/// so the filter's scans never hold the store the tools query.
pub fn build_router_with_filter_store(
    store: Arc<Mutex<memory::Store>>,
    filter_store: Arc<Mutex<memory::Store>>,
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
            )
            .with_filter_store(Arc::clone(&filter_store)))
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
    /// The connection the exclusion filter is read through. The binary gives it its own, so a
    /// filter's scans never hold the store the tools query (fix round 1, M5); tests share one.
    filter_store: Arc<Mutex<memory::Store>>,
    llm_client: Arc<dyn llm::LlmClient + Send + Sync>,
    tool_router: ToolRouter<Self>,
}

impl WhimbrelMcpServer {
    pub fn new(
        store: Arc<Mutex<memory::Store>>,
        llm_client: Arc<dyn llm::LlmClient + Send + Sync>,
    ) -> Self {
        Self {
            filter_store: Arc::clone(&store),
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
    /// Reads the exclusion filter through `filter_store` (its own connection in the binary).
    pub fn with_filter_store(mut self, filter_store: Arc<Mutex<memory::Store>>) -> Self {
        self.filter_store = filter_store;
        self
    }

    /// What may leave in a tool result: every tool's output is treated as hosted. Read before a
    /// tool takes the store lock, never while holding it.
    fn load_filter(&self) -> Result<llm::ContextFilter, ErrorData> {
        let store = self
            .filter_store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        Ok(llm::ContextFilter::load(&store))
    }

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

/// The MCP server's own rows in `mcp_daily_digest_cache`: the Windows Today recap keeps the plain
/// digest under the bare day, so the two never overwrite each other (fix round 1, M6).
fn mcp_digest_key(day: &str) -> String {
    format!("mcp:{day}")
}

/// A cached digest and the inputs it was built from (fix round 4, S2).
#[derive(Deserialize)]
struct CachedDigest {
    /// The captures it was built from ([`memory::topics::exclusion::input_ref`]).
    input_refs: Vec<String>,
    digest: llm::DailyDigest,
}

#[derive(Serialize)]
struct CachedDigestRef<'a> {
    input_refs: &'a [String],
    digest: &'a llm::DailyDigest,
}

/// The most captures a cached digest records; a digest built from more is returned, not cached
/// (never cached with a shortened list).
const MAX_DIGEST_INPUTS: usize = 20_000;

/// Whether none of `inputs` is excluded under `filter` (fix round 4, S2): new material of a client
/// that was not among them never matters. Nothing is while the setting is unreadable.
fn inputs_clean(store: &memory::Store, filter: &llm::ContextFilter, inputs: &[String]) -> bool {
    match filter.material() {
        // A capture that no longer exists cannot be checked: the digest is built again.
        Some(m) => {
            inputs.iter().all(|r| !m.excludes_input_ref(r))
                && store.missing_input_refs(inputs).is_ok_and(|missing| missing.is_empty())
        }
        None => !filter.is_closed(),
    }
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
        let filter = self.load_filter()?;
        let allow_google = self.has_scope(&parts, GOOGLE_DATA_SCOPE);
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let keep_capture = |c: &memory::CaptureItem| filter.keeps(&llm::ContextItem::Capture(c.clone()));
        let keep_segment = |t: &memory::MeetingTranscriptSegment| filter.keeps(&llm::ContextItem::Transcript(t.clone()));
        let mut results = Vec::new();
        for item in store
            .search_keyword(&params.query, params.limit)
            .unwrap_or_default()
            .into_iter()
            .filter(keep_capture)
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
            .into_iter()
            .filter(keep_capture)
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
                .into_iter()
                .filter(keep_segment)
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
                .into_iter()
                .filter(keep_segment)
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
        let filter = self.load_filter()?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let mut meetings = store
            .recent_meetings(params.limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .into_iter()
            .filter(|m| !meeting_excluded(&store, &filter, m))
            .collect::<Vec<_>>();
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
        let filter = self.load_filter()?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let mut meeting = store
            .get_meeting(params.meeting_id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .filter(|m| !meeting_excluded(&store, &filter, m));
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
        let filter = self.load_filter()?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let excluded = match store.get_meeting(params.meeting_id) {
            Ok(Some(m)) => meeting_excluded(&store, &filter, &m),
            Ok(None) => filter.is_closed() || filter.material().is_some_and(|x| x.meetings.contains(&params.meeting_id)),
            Err(e) => return Err(ErrorData::internal_error(e.to_string(), None)),
        };
        if excluded {
            return Ok(Json(GetTranscriptOutput { segments: Vec::new(), total: 0, google_data_withheld: 0, note: None }));
        }
        let (segments, total) = store
            .get_transcript_segments_page(params.meeting_id, params.offset, params.limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let mut segments: Vec<_> =
            segments.into_iter().filter(|t| filter.keeps(&llm::ContextItem::Transcript(t.clone()))).collect();
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
        let filter = self.load_filter()?;
        let store = self
            .store
            .lock()
            .map_err(|_| ErrorData::internal_error("store lock poisoned".to_string(), None))?;
        let items = store
            .timeline_range(params.start_unix_ms, params.end_unix_ms)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .into_iter()
            .filter(|c| filter.keeps(&llm::ContextItem::Capture(c.clone())))
            .collect();
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
        let internal = |e: String| ErrorData::internal_error(e, None);
        let lock = || self.store.lock().map_err(|_| internal("store lock poisoned".to_string()));
        let mut filter = self.load_filter()?;
        let store = lock()?;
        // Served while none of the captures it was built from is excluded now, and it names no
        // excluded client (fix round 4, S2). A cache with no inputs recorded is built again.
        if let Some(cached) = store.get_cached_daily_digest(&mcp_digest_key(&params.day)).map_err(|e| internal(e.to_string()))? {
            if let Ok(entry) = serde_json::from_str::<CachedDigest>(&cached) {
                // The backstop reads the digest's own decoded words, not the escaped cache entry
                // (fix round 6, J4).
                let words = format!("{}\n{}", entry.digest.summary, entry.digest.open_loops.join("\n"));
                if !filter.is_closed() && inputs_clean(&store, &filter, &entry.input_refs) && !filter.names_client(&words) {
                    return Ok(Json(GetDailyDigestOutput { digest: entry.digest }));
                }
            }
        }
        drop(store);
        // Built from the day without excluded material. The model call runs unlocked, so the
        // setting may change meanwhile: it is read again after the call, and a digest one of
        // whose inputs became excluded is neither cached nor returned; it is built once more.
        for _ in 0..2 {
            let items: Vec<memory::CaptureItem> = lock()?
                .timeline_range(params.start_unix_ms, params.end_unix_ms)
                .map_err(|e| internal(e.to_string()))?
                .into_iter()
                .filter(|c| filter.keeps(&llm::ContextItem::Capture(c.clone())))
                .collect();
            let inputs: Vec<String> =
                items.iter().map(|c| llm::exclusion::input_ref_of(&llm::ContextItem::Capture(c.clone()))).collect();

            let digest = llm::generate_daily_digest(self.llm_client.as_ref(), &items, "").map_err(|e| internal(e.to_string()))?;

            let now_filter = self.load_filter()?;
            let store = lock()?;
            if !inputs.is_empty() && !inputs_clean(&store, &now_filter, &inputs) {
                filter = now_filter;
                continue;
            }
            // An unreadable setting at the start: returned (built from nothing), never cached.
            if !filter.is_closed() && !now_filter.is_closed() && inputs.len() <= MAX_DIGEST_INPUTS {
                let entry = serde_json::to_string(&CachedDigestRef { input_refs: &inputs, digest: &digest })
                    .map_err(|e| internal(e.to_string()))?;
                let now = chrono::Utc::now().timestamp_millis();
                store.cache_daily_digest(&mcp_digest_key(&params.day), &entry, now).map_err(|e| internal(e.to_string()))?;
            }
            return Ok(Json(GetDailyDigestOutput { digest }));
        }
        Err(internal("the hosted-model exclusions changed while the digest was written; try again".to_string()))
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
        let filter = self.load_filter()?;
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
            &filter,
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

/// Whether meeting `m` is an excluded client's: filed under one, or its title, attendees,
/// notes, summary or any other text written about it (Store::meeting_authored_text) name one.
fn meeting_excluded(store: &memory::Store, filter: &llm::ContextFilter, m: &memory::Meeting) -> bool {
    if !filter.is_active() {
        return false;
    }
    // Every other text written about the meeting (fix round 4, S1); unreadable is the client's.
    let draft = match store.meeting_authored_text(m.id) {
        Ok(text) => text,
        Err(_) => return true,
    };
    let text = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{draft}",
        m.notes.as_deref().unwrap_or_default(),
        m.summary_tldr.as_deref().unwrap_or_default(),
        m.summary_decisions.join("\n"),
        m.summary_action_items.join("\n"),
        m.summary_open_questions.join("\n"),
        m.transcript_path.as_deref().unwrap_or_default(),
    );
    filter.excludes_meeting(m.id, &m.title, &m.attendees) || filter.names_client(&text)
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

    /// Keeps every prompt; answers with an empty digest.
    struct RecordingLlm(Arc<Mutex<Vec<String>>>);

    impl llm::LlmClient for RecordingLlm {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            self.0.lock().unwrap().push(prompt.to_string());
            Ok(llm::Completion { text: r#"{"summary": "A day.", "open_loops": []}"#.into(), token_usage: llm::TokenUsage::default() })
        }
    }

    /// Excludes client `key` during its first call (the setting changes while the model runs).
    struct ExcludingLlm {
        prompts: Arc<Mutex<Vec<String>>>,
        store: Arc<Mutex<memory::Store>>,
        key: String,
    }

    impl llm::LlmClient for ExcludingLlm {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            let mut prompts = self.prompts.lock().unwrap();
            if prompts.is_empty() {
                self.store
                    .lock()
                    .unwrap()
                    .set_client_control(&self.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
                    .unwrap();
            }
            prompts.push(prompt.to_string());
            Ok(llm::Completion { text: r#"{"summary": "A day.", "open_loops": []}"#.into(), token_usage: llm::TokenUsage::default() })
        }
    }

    /// The digest cache follows the exclusion state: a cached digest built under another state
    /// is built again even when it names no client; one built under this state is served; and a
    /// digest whose setting changed during the model call is neither cached nor returned, but
    /// built again under the new state.
    #[tokio::test]
    async fn the_digest_cache_follows_the_exclusion_state() {
        let now = chrono::Utc::now().timestamp_millis();
        let store = memory::Store::open_in_memory().unwrap();
        let globex = store.create_seed_topic("Globex", &[], None).unwrap();
        for (i, text) in ["Globex renewal notes", "giraffe pricing notes"].iter().enumerate() {
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "d".into(),
                    ts_unix_ms: now - 1_000 + i as i64,
                    app_bundle_id: "com.apple.Notes".into(),
                    window_title: "Notes".into(),
                    url: None,
                    text: text.to_string(),
                    hash: i as i64,
                    source: "ax".into(),
                })
                .unwrap();
        }
        // Built under some other state, naming nobody: still built again.
        store
            .cache_daily_digest("mcp:2026-11-12", r#"{"exclusion_version":"x0000000000000000","digest":{"summary":"Old.","open_loops":[]}}"#, 1)
            .unwrap();
        let store = Arc::new(Mutex::new(store));
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let llm = ExcludingLlm { prompts: prompts.clone(), store: store.clone(), key: globex.key.clone() };
        let server = WhimbrelMcpServer::new(store.clone(), Arc::new(llm));
        let call = || async {
            server
                .get_daily_digest(
                    Extension(parts_with_scopes(&["memory"])),
                    Parameters(GetDailyDigestParams { day: "2026-11-12".into(), start_unix_ms: 0, end_unix_ms: now + 1 }),
                )
                .await
                .unwrap()
                .0
        };
        let out = call().await;
        assert_eq!(out.digest.summary, "A day.", "the stale-state cache was not served");
        let p = prompts.lock().unwrap().clone();
        assert_eq!(p.len(), 2, "Globex was excluded during the first call: built again");
        assert!(p[0].contains("Globex") && !p[1].contains("Globex"), "{p:?}");
        // Cached with the inputs it was built from, and served with no model call.
        let cached = store.lock().unwrap().get_cached_daily_digest("mcp:2026-11-12").unwrap().unwrap();
        assert!(cached.contains("input_refs") && !cached.to_lowercase().contains("globex"), "{cached}");
        call().await;
        assert_eq!(prompts.lock().unwrap().len(), 2);
    }

    /// Fix round 6, J4: the cached digest's name backstop reads the digest's decoded words. A cached
    /// summary naming the client after an escaped newline is built again, not served.
    #[tokio::test]
    async fn a_cached_digest_naming_the_client_after_an_escape_is_built_again() {
        let store = memory::Store::open_in_memory().unwrap();
        let acme = store.create_seed_topic("Acme", &[], None).unwrap();
        store.set_client_control(&acme.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        let entry = serde_json::json!({"input_refs": [], "digest": {"summary": "The week.\n\nAcme renewal call.", "open_loops": []}}).to_string();
        assert!(entry.contains("\\n\\nAcme"), "{entry}");
        store.cache_daily_digest("mcp:2026-11-14", &entry, 1).unwrap();
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let server = WhimbrelMcpServer::new(Arc::new(Mutex::new(store)), Arc::new(RecordingLlm(prompts.clone())));
        let now = chrono::Utc::now().timestamp_millis();
        let Json(out) = server
            .get_daily_digest(Extension(parts_with_scopes(&["memory"])), Parameters(GetDailyDigestParams { day: "2026-11-14".into(), start_unix_ms: 0, end_unix_ms: now }))
            .await
            .unwrap();
        assert!(!out.digest.summary.contains("Acme"), "the cached digest naming Acme was served");
    }

    /// Captures the excluded client's material during every model call, as steady screen capture
    /// does while the user works on the client.
    struct CapturingLlm {
        prompts: Arc<Mutex<Vec<String>>>,
        store: Arc<Mutex<memory::Store>>,
    }

    impl llm::LlmClient for CapturingLlm {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            let mut prompts = self.prompts.lock().unwrap();
            let n = prompts.len() as i64;
            capture(&self.store.lock().unwrap(), 100 + n, &format!("Acme contract draft {n}"));
            prompts.push(prompt.to_string());
            Ok(llm::Completion { text: r#"{"summary": "A day.", "open_loops": []}"#.into(), token_usage: llm::TokenUsage::default() })
        }
    }

    fn capture(store: &memory::Store, hash: i64, text: &str) {
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "d".into(),
                ts_unix_ms: chrono::Utc::now().timestamp_millis() - 500,
                app_bundle_id: "com.apple.Notes".into(),
                window_title: "Notes".into(),
                url: None,
                text: text.to_string(),
                hash,
                source: "ax".into(),
            })
            .unwrap();
    }

    /// Fix round 4, S2: new material of an excluded client that is not among a digest's inputs
    /// neither fails the build ("try again") nor drops the cached digest. Control: an input later
    /// found to be the client's does drop it.
    #[tokio::test]
    async fn steady_capture_of_an_excluded_client_never_fails_the_digest() {
        let now = chrono::Utc::now().timestamp_millis();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let store = memory::Store::open(&path, dir.path()).unwrap();
        let c = rusqlite::Connection::open(&path).unwrap();
        let acme = store.create_seed_topic("Acme", &["Roadrunner Co".to_string()], None).unwrap();
        store.set_client_control(&acme.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        capture(&store, 2, "okapi planning notes");
        capture(&store, 1, "giraffe pricing notes");
        let store = Arc::new(Mutex::new(store));
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let server = WhimbrelMcpServer::new(store.clone(), Arc::new(CapturingLlm { prompts: prompts.clone(), store: store.clone() }));
        let call = || async {
            server
                .get_daily_digest(
                    Extension(parts_with_scopes(&["memory"])),
                    Parameters(GetDailyDigestParams { day: "2026-11-13".into(), start_unix_ms: 0, end_unix_ms: now + 60_000 }),
                )
                .await
        };
        let out = call().await.expect("built, not 'try again'").0;
        assert_eq!(out.digest.summary, "A day.");
        assert_eq!(prompts.lock().unwrap().len(), 1, "built once");
        // More of the client's captures arrive: the cached digest is still served.
        capture(&store.lock().unwrap(), 50, "Acme renewal call notes");
        call().await.unwrap();
        assert_eq!(prompts.lock().unwrap().len(), 1, "served from the cache");
        // Control: the digest's own input is found to be the client's through a lift no cache-drop
        // trigger covers (fix round 5, T1): an episode joins the giraffe capture to a new Acme
        // capture, so every capture of that episode is Acme's. The cached row survives up to the
        // read; the serve-side check is what rebuilds it.
        {
            let s = store.lock().unwrap();
            capture(&s, 60, "Acme kickoff notes");
            let item = |text: &str| -> i64 {
                c.query_row("SELECT id FROM items WHERE kind = 'capture' AND body_text = ?1", [text], |r| r.get(0)).unwrap()
            };
            let (from, to) = (item("giraffe pricing notes"), item("Acme kickoff notes"));
            c.execute(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('d', 'com.apple.Notes', 'Notes', 1, 2, ?1, ?2, 2, 'summarized', 1, 1)",
                [from, to],
            )
            .unwrap();
            assert!(s.get_cached_daily_digest("mcp:2026-11-13").unwrap().is_some(), "no trigger dropped the cache");
        }
        call().await.unwrap();
        assert_eq!(prompts.lock().unwrap().len(), 2, "an input became the client's");
        let p = prompts.lock().unwrap().clone();
        assert!(p[0].contains("giraffe") && !p[1].contains("giraffe") && p[1].contains("okapi"), "{p:?}");
    }

    /// X6 spec 7.3: tool results go to outside AI tools, so none carries a client excluded
    /// from hosted models: its meetings (filed under it, never naming it: the canary), or
    /// anything that names it.
    #[tokio::test]
    async fn mcp_tools_never_return_excluded_text() {
        const CANARY: &str = "zebracanary";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let store = memory::Store::open(&path, dir.path()).unwrap();
        let c = rusqlite::Connection::open(&path).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let acme = store.create_seed_topic("Acme", &[], None).unwrap();
        let meeting = |title: &str, line: &str| {
            let id = store
                .insert_meeting(&memory::NewMeeting {
                    device_id: "d".into(),
                    start_unix_ms: now - 3_600_000,
                    end_unix_ms: Some(now - 3_000_000),
                    title: title.into(),
                    attendees: vec![],
                    trigger: memory::MeetingTrigger::Calendar,
                    audio_path: None,
                    status: memory::MeetingStatus::Completed,
                })
                .unwrap();
            store
                .insert_transcript_segments(&[memory::NewTranscriptSegment { meeting_id: id, start_cs: 0, speaker_label: "S".into(), text: line.into() }])
                .unwrap();
            store.update_meeting_summary(id, line, &[], &[], &[], "ollama", "m").unwrap();
            id
        };
        let excluded = meeting("Quarterly sync", &format!("The {CANARY} pricing was agreed."));
        let other = meeting("Team standup", "The giraffe pricing review is next week.");
        // Fix round 4, S1: a meeting the client is named in only by its workspace notes.
        let drafted = meeting("Call", &format!("The {CANARY} quote stands."));
        store.save_meeting_personal_notes(drafted, "Acme Q3 pricing", 1).unwrap();
        let undrafted = meeting("Other call", "The okapi quote stands.");
        c.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES ('meeting', ?1, 1, 1)", [excluded]).unwrap();
        c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", [acme.topic_id, c.last_insert_rowid()])
            .unwrap();
        // B7: the third names the client only in its URL.
        let urls = [None, None, Some("https://acme.sharepoint.example/renewal".to_string())];
        for (i, text) in ["Notes on the Acme pricing renewal", "giraffe pricing notes", "renewal pricing draft"].iter().enumerate() {
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "d".into(),
                    ts_unix_ms: now - 1_000 + i as i64,
                    app_bundle_id: "com.apple.Notes".into(),
                    window_title: "Notes".into(),
                    url: urls[i].clone(),
                    text: text.to_string(),
                    hash: i as i64,
                    source: "ax".into(),
                })
                .unwrap();
        }
        store
            .set_client_control(&acme.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        // A digest cached with the client in it is built again.
        store.cache_daily_digest("2026-11-12", r#"{"summary":"The Acme call.","open_loops":[]}"#, 1).unwrap();
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let server = WhimbrelMcpServer::new(Arc::new(Mutex::new(store)), Arc::new(RecordingLlm(prompts.clone())));
        let all = || Extension(parts_with_scopes(&["memory", "meetings", "transcripts"]));
        let clean = |what: &str, text: String| {
            let lower = text.to_lowercase();
            assert!(!lower.contains(CANARY) && !lower.contains("acme"), "{what} returned excluded material: {text}");
        };

        let Json(out) = server.search_memory(all(), Parameters(SearchMemoryParams { query: "pricing".into(), limit: 20 })).await.unwrap();
        let texts: Vec<String> = out.results.iter().map(|h| format!("{} {:?}", h.text, h.meeting_title)).collect();
        assert!(texts.iter().any(|t| t.contains("giraffe")), "{texts:?}");
        clean("search_memory", texts.join("\n"));

        // Fix round 4, S1: named only in its workspace notes. Control: the undrafted call.
        let Json(out) = server.search_memory(all(), Parameters(SearchMemoryParams { query: "quote".into(), limit: 20 })).await.unwrap();
        let texts: Vec<String> = out.results.iter().map(|h| format!("{} {:?}", h.text, h.meeting_title)).collect();
        assert!(texts.iter().any(|t| t.contains("okapi")), "{texts:?}");
        clean("search_memory (workspace notes)", texts.join("\n"));

        let Json(out) = server.list_meetings(all(), Parameters(ListMeetingsParams { limit: 20 })).await.unwrap();
        let mut listed: Vec<i64> = out.meetings.iter().map(|m| m.id).collect();
        listed.sort();
        assert_eq!(listed, vec![other, undrafted]);

        let Json(out) = server.get_meeting(all(), Parameters(GetMeetingParams { meeting_id: excluded })).await.unwrap();
        assert!(out.meeting.is_none());
        let Json(out) = server.get_meeting(all(), Parameters(GetMeetingParams { meeting_id: other })).await.unwrap();
        assert!(out.meeting.is_some());
        let Json(out) = server.get_meeting(all(), Parameters(GetMeetingParams { meeting_id: drafted })).await.unwrap();
        assert!(out.meeting.is_none(), "named only in its workspace notes");
        let Json(out) = server
            .get_transcript(all(), Parameters(GetTranscriptParams { meeting_id: drafted, offset: 0, limit: 20 }))
            .await
            .unwrap();
        assert!(out.segments.is_empty());

        let Json(out) = server
            .get_transcript(all(), Parameters(GetTranscriptParams { meeting_id: excluded, offset: 0, limit: 20 }))
            .await
            .unwrap();
        assert!(out.segments.is_empty() && out.total == 0);

        let Json(out) = server.get_timeline(all(), Parameters(GetTimelineParams { start_unix_ms: 0, end_unix_ms: now + 1 })).await.unwrap();
        assert_eq!(out.items.len(), 1);
        clean("get_timeline", out.items.iter().map(|i| format!("{} {:?}", i.text, i.url)).collect::<Vec<_>>().join("\n"));

        let Json(out) = server
            .get_daily_digest(all(), Parameters(GetDailyDigestParams { day: "2026-11-12".into(), start_unix_ms: 0, end_unix_ms: now + 1 }))
            .await
            .unwrap();
        assert_eq!(out.digest.summary, "A day.", "the cached digest naming the client was built again");
        assert_eq!(prompts.lock().unwrap().len(), 1);
        clean("the digest prompt", prompts.lock().unwrap().join("\n"));

        let Json(out) = server
            .assemble_context(all(), Parameters(AssembleContextParams { query: "pricing".into(), context_limit: 20 }))
            .await
            .unwrap();
        assert!(!out.items.is_empty());
        clean("assemble_context", out.items.iter().map(|i| format!("{} {}", i.label, i.text)).collect::<Vec<_>>().join("\n"));
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
            // Cached with the inputs it was built from (none).
            store.cache_daily_digest("mcp:2026-08-02", r#"{"input_refs":[],"digest":{"summary":"cached","open_loops":[],"token_usage":{"input_tokens":null,"output_tokens":null}}}"#, 1234).unwrap();
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
