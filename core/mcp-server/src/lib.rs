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
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetMeetingOutput {
    pub meeting: Option<memory::Meeting>,
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
}

#[tool_router]
impl WhimbrelMcpServer {
    #[tool(
        name = "search_memory",
        description = "Hybrid semantic+keyword search over everything Whimbrel has captured on this Mac: OCR'd screen text and transcribed meeting speech, ranked together. Each hit carries a `domain` (\"screen_text\" or \"meeting_speech\"), the row `id`, and for speech hits the `meeting_id`/`meeting_title` to pass to get_meeting or get_transcript. Returns raw matching snippets only; use assemble_context instead when you want one ranked, budget-packed context list that also folds in recent episodes, threads, and routine runs. Read-only, no LLM cost."
    )]
    async fn search_memory(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<SearchMemoryParams>,
    ) -> Result<Json<SearchMemoryOutput>, ErrorData> {
        self.check_scope(&parts, "memory")?;
        let include_transcripts = self.has_scope(&parts, "transcripts");
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
        Ok(Json(SearchMemoryOutput { results }))
    }

    #[tool(
        name = "list_meetings",
        description = "List recent meetings, newest first, with id, title, status, timestamps, and the generated summary fields when a summary exists. Includes meetings still recording. Read-only, no LLM cost."
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
        let meetings = store
            .recent_meetings(params.limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(ListMeetingsOutput { meetings }))
    }

    #[tool(
        name = "get_meeting",
        description = "Fetch one meeting's metadata and generated summary (tldr, decisions, action items, open questions) by id. Returns `meeting: null` for an unknown id rather than an error. Does not include the transcript; use get_transcript for that. Read-only, no LLM cost."
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
        let meeting = store
            .get_meeting(params.meeting_id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(GetMeetingOutput { meeting }))
    }

    #[tool(
        name = "get_transcript",
        description = "Fetch one page of a meeting's transcript segments in chronological order, with `total` for paging (offset + limit, default limit 20). Segments are raw speech-to-text, unsummarized. Read-only, no LLM cost."
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
        let (segments, total) = store
            .get_transcript_segments_page(params.meeting_id, params.offset, params.limit)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(Json(GetTranscriptOutput { segments, total }))
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
        description = "Unified retrieval for answering a question: hybrid keyword+semantic search over screen text and meeting speech, fused with recent episodes, email threads needing attention, and routine outputs, packed into one ranked context list of at most `context_limit` items (default 20). Each item has a `label` (its kind and source) and `text`. Prefer this over search_memory when the goal is to answer or summarize rather than to locate a specific row. Read-only, no LLM cost."
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
        );
        let items = context
            .into_iter()
            .map(|item| AssembleContextItem {
                reference: item.citation_ref(),
                label: item.display_label(),
                text: item.text().to_string(),
            })
            .collect();
        Ok(Json(AssembleContextOutput { items }))
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
