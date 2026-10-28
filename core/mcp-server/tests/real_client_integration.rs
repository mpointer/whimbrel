//! Real, live integration test (Task 4, Step 2): starts the real server — built with
//! `mcp_server::build_router`, the same function `src/bin/whimbrel-mcp-server.rs`'s
//! `main()` calls, so this test's router can't silently drift from the real one the
//! shipped binary serves — bound to an ephemeral port rather than the real `4319` to
//! avoid colliding with a locally running instance — backed by a real temp-file `Store`
//! with real inserted fixture rows, then connects a real `rmcp` client over
//! streamable-HTTP and calls each of the seven tools, asserting on the real data returned.
//! Nothing here is mocked; `get_daily_digest`'s LLM leg is exercised via its cache-hit
//! path (pre-seeded, exactly like `lib.rs`'s own
//! `get_daily_digest_returns_cached_value_without_calling_the_llm_client` unit test)
//! rather than a fake network call, so a `NoopLlmClient` never actually needs to run.

use std::sync::{Arc, Mutex};

use mcp_server::build_router;
use rmcp::{
    model::{CallToolRequestParams, ClientInfo},
    service::ClientInitializeError,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};

/// Never actually called in this test — `get_daily_digest` only reaches the LLM client
/// on a cache miss, and the fixture below pre-seeds a cached digest for the exact day
/// queried. Stands in for a real client only to satisfy `WhimbrelMcpServer::new`'s
/// signature, same as `lib.rs`'s own `NoopLlmClient` test double.
struct NoopLlmClient;

impl llm::LlmClient for NoopLlmClient {
    fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
        Err(llm::LlmError::NotImplemented(
            "NoopLlmClient is test-only and never expected to be called",
        ))
    }
}

/// Spins up the real server — a real temp-file-backed `Store` seeded with real fixture
/// rows through the real `Store` API, wrapped by `WhimbrelMcpServer`, served behind the
/// real router `mcp_server::build_router` builds (the `StreamableHttpService` nested
/// under `/mcp` with Task 3's real `AuthLayer` applied to that outer router — the exact
/// same call `src/bin/whimbrel-mcp-server.rs`'s `main()` makes, not a hand-mirrored copy
/// of it) — bound to an OS-assigned ephemeral port. Returns the server's `/mcp` base
/// URL, a real bearer token authorized for every scope this test needs, the meeting id
/// the fixture data was inserted under, and a second real bearer token scoped *only* to
/// "memory" (used to prove scope checks actually discriminate, not just that some
/// `AuthContext` traverses the chain).
async fn spawn_real_server() -> (String, String, i64, String) {
    let dir = tempfile::tempdir().expect("create a temp dir for the test database");
    let db_path = dir.path().join("core-memory.sqlite");
    // No bundled embedding model in this test environment — `Store::open` already
    // degrades gracefully on a missing models dir (see `core/memory/src/lib.rs`'s
    // `open_in_memory` doc comment); `search_memory`'s keyword-search leg (exercised
    // below) doesn't depend on it.
    let store = memory::Store::open(&db_path, std::path::Path::new(""))
        .expect("open a real sqlite-backed Store at a real temp-file path");

    let meeting_id = store
        .insert_meeting(&memory::NewMeeting {
            device_id: "test-device".to_string(),
            start_unix_ms: 1_000,
            end_unix_ms: Some(2_000),
            title: "Integration Test Standup".to_string(),
            attendees: vec!["Alice".to_string()],
            trigger: memory::MeetingTrigger::Manual,
            audio_path: None,
            status: memory::MeetingStatus::Recording,
        })
        .unwrap();
    store
        .insert_transcript_segments(&[memory::NewTranscriptSegment {
            meeting_id,
            start_cs: 0,
            speaker_label: "Alice".to_string(),
            text: "let's ship the mcp server".to_string(),
        }])
        .unwrap();
    store
        .insert_capture_item(&memory::NewCaptureItem {
            device_id: "test-device".to_string(),
            ts_unix_ms: 1_500,
            app_bundle_id: "com.apple.Safari".to_string(),
            window_title: "Some Window".to_string(),
            url: None,
            text: "the quick brown fox jumps".to_string(),
            hash: 1,
            source: "ax".to_string(),
        })
        .unwrap();
    store
        .cache_daily_digest(
            "2026-08-02",
            r#"{"summary":"integration test digest","open_loops":[],"token_usage":{"input_tokens":null,"output_tokens":null}}"#,
            1_234,
        )
        .unwrap();

    let generated = mcp_server::auth::generate_token();
    let client_id = store
        .insert_mcp_client_token(&memory::NewMcpClientToken {
            name: "integration test client".to_string(),
            token_hash: generated.hash.clone(),
            token_salt: generated.salt.clone(),
            scopes: vec![
                "memory".to_string(),
                "meetings".to_string(),
                "transcripts".to_string(),
            ],
        })
        .unwrap();
    let bearer = format!(
        "kmcp_{client_id}.{secret}",
        secret = generated.plaintext_secret
    );

    // A second, real, separately-hashed token scoped *only* to "memory" — used to prove
    // scope checking actually discriminates end-to-end through the real transport (not
    // just that `AuthContext` traverses the chain at all, which the seven-tool call above
    // would already demonstrate even if scopes were silently ignored).
    let memory_only_generated = mcp_server::auth::generate_token();
    let memory_only_client_id = store
        .insert_mcp_client_token(&memory::NewMcpClientToken {
            name: "integration test client (memory-only)".to_string(),
            token_hash: memory_only_generated.hash.clone(),
            token_salt: memory_only_generated.salt.clone(),
            scopes: vec!["memory".to_string()],
        })
        .unwrap();
    let memory_only_bearer = format!(
        "kmcp_{memory_only_client_id}.{secret}",
        secret = memory_only_generated.plaintext_secret
    );

    let store = Arc::new(Mutex::new(store));
    let llm_client: Arc<dyn llm::LlmClient + Send + Sync> = Arc::new(NoopLlmClient);

    // Shares the exact router construction `src/bin/whimbrel-mcp-server.rs`'s `main()`
    // uses (see `mcp_server::build_router`'s doc comment) — this test can no longer
    // silently drift from the real router the shipped binary actually serves.
    let app = build_router(Arc::clone(&store), llm_client);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral localhost port");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // The server task borrows the temp dir's path only at startup (the sqlite file is
    // already created by `Store::open` above); keep it alive for this test process so
    // the directory isn't cleaned up out from under a still-running server task. Each
    // `cargo test` invocation is a fresh process, so leaking this one `TempDir` per test
    // is bounded and reclaimed by the OS when the test binary exits.
    std::mem::forget(dir);

    (
        format!("http://{addr}/mcp"),
        bearer,
        meeting_id,
        memory_only_bearer,
    )
}

fn tool_args(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_value(value).expect("test fixture arguments are always a JSON object")
}

#[tokio::test]
async fn real_client_calls_all_seven_tools_against_a_real_running_server() {
    let (base_url, bearer, meeting_id, memory_only_bearer) = spawn_real_server().await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(base_url.clone()).auth_header(bearer),
    );
    let client = ClientInfo::default().serve(transport).await.expect(
        "a real rmcp client connects over streamable-HTTP and completes the initialize handshake",
    );

    // search_memory
    let result = client
        .call_tool(
            CallToolRequestParams::new("search_memory").with_arguments(tool_args(
                serde_json::json!({ "query": "fox", "limit": 20 }),
            )),
        )
        .await
        .expect("search_memory call succeeds");
    let structured = result
        .structured_content
        .expect("search_memory returns structured content");
    assert!(
        structured["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["domain"] == "screen_text" && hit["text"] == "the quick brown fox jumps"),
        "expected the real inserted capture item back in search_memory's results, got: {structured}"
    );

    // list_meetings
    let result = client
        .call_tool(
            CallToolRequestParams::new("list_meetings")
                .with_arguments(tool_args(serde_json::json!({ "limit": 10 }))),
        )
        .await
        .expect("list_meetings call succeeds");
    let meetings = result
        .structured_content
        .expect("list_meetings returns structured content");
    let meetings = meetings["meetings"].as_array().unwrap();
    assert_eq!(meetings.len(), 1);
    assert_eq!(meetings[0]["id"].as_i64(), Some(meeting_id));
    assert_eq!(meetings[0]["title"], "Integration Test Standup");

    // get_meeting
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_meeting")
                .with_arguments(tool_args(serde_json::json!({ "meeting_id": meeting_id }))),
        )
        .await
        .expect("get_meeting call succeeds");
    let meeting = result
        .structured_content
        .expect("get_meeting returns structured content");
    assert_eq!(meeting["meeting"]["title"], "Integration Test Standup");

    // get_transcript
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_transcript").with_arguments(tool_args(
                serde_json::json!({
                    "meeting_id": meeting_id,
                    "offset": 0,
                    "limit": 10,
                }),
            )),
        )
        .await
        .expect("get_transcript call succeeds");
    let transcript = result
        .structured_content
        .expect("get_transcript returns structured content");
    assert_eq!(transcript["total"], 1);
    assert_eq!(
        transcript["segments"][0]["text"],
        "let's ship the mcp server"
    );

    // get_timeline
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_timeline").with_arguments(tool_args(
                serde_json::json!({
                    "start_unix_ms": 0,
                    "end_unix_ms": 10_000,
                }),
            )),
        )
        .await
        .expect("get_timeline call succeeds");
    let items = result
        .structured_content
        .expect("get_timeline returns structured content");
    assert!(
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["text"] == "the quick brown fox jumps"),
        "expected the real inserted capture item back in get_timeline's results, got: {items}"
    );

    // get_daily_digest — cache-hit path, so no real LLM call is ever made.
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_daily_digest").with_arguments(tool_args(
                serde_json::json!({
                    "day": "2026-08-02",
                    "start_unix_ms": 0,
                    "end_unix_ms": 1,
                }),
            )),
        )
        .await
        .expect("get_daily_digest call succeeds");
    let digest = result
        .structured_content
        .expect("get_daily_digest returns structured content");
    assert_eq!(digest["digest"]["summary"], "integration test digest");

    // assemble_context
    let result = client
        .call_tool(
            CallToolRequestParams::new("assemble_context").with_arguments(tool_args(
                serde_json::json!({
                    "query": "fox",
                    "context_limit": 20,
                }),
            )),
        )
        .await
        .expect("assemble_context call succeeds");
    let context = result
        .structured_content
        .expect("assemble_context returns structured content");
    assert!(
        context["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["text"].as_str().map_or(false, |t| t.contains("fox"))),
        "expected items mentioning 'fox' in assemble_context's results, got: {context}"
    );

    let _ = client.cancel().await;

    // Scope discrimination, proven through the real transport (not the unit tests in
    // `lib.rs`, which fabricate `Parts` directly rather than going through auth
    // middleware + rmcp's own extension plumbing): a second, real, separately-hashed
    // token scoped only to "memory" must be rejected calling `get_transcript` (which
    // requires "transcripts"), even though the very same token would succeed on, say,
    // `get_timeline`. This is the assertion the tool-level unit tests can't cover, since
    // they never traverse `AuthMiddleware` at all.
    let memory_only_transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(base_url).auth_header(memory_only_bearer),
    );
    let memory_only_client = ClientInfo::default()
        .serve(memory_only_transport)
        .await
        .expect("the memory-only token still completes the initialize handshake — auth, not scope, gates that");
    let scope_err = memory_only_client
        .call_tool(
            CallToolRequestParams::new("get_transcript").with_arguments(tool_args(
                serde_json::json!({
                    "meeting_id": meeting_id,
                    "offset": 0,
                    "limit": 10,
                }),
            )),
        )
        .await
        .expect_err("a token scoped only to \"memory\" must be rejected calling get_transcript");
    match scope_err {
        rmcp::ServiceError::McpError(data) => {
            assert!(
                data.message.contains("not authorized for scope"),
                "expected the real per-tool scope rejection, got: {data:?}"
            );
        }
        other => panic!(
            "expected ServiceError::McpError carrying the real scope rejection, got: {other:?}"
        ),
    }
    let _ = memory_only_client.cancel().await;
}

/// Proves Task 3's auth wiring is really enforced end-to-end through this real HTTP
/// server, not just unit-tested in isolation: a client with no bearer token at all never
/// gets past the `initialize` handshake, because `AuthLayer` rejects the request with a
/// 401 before it ever reaches rmcp's own dispatch.
#[tokio::test]
async fn a_client_with_no_bearer_token_cannot_even_complete_the_initialize_handshake() {
    let (base_url, _bearer, _meeting_id, _memory_only_bearer) = spawn_real_server().await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(base_url),
    );
    let err = ClientInfo::default()
        .serve(transport)
        .await
        .expect_err("a request with no Authorization header must be rejected before the MCP handshake completes");

    // Not just "it errored" — the initialize handshake's own JSON-RPC response really is
    // the real 401 body `AuthLayer` builds (see `auth.rs`'s `AuthRejection::into_response`),
    // surfaced by rmcp as `ClientInitializeError::JsonRpcError`. Confirmed live via
    // `eprintln!("{err:?}")` before writing this assertion: `JsonRpcError(ErrorData {
    // code: ErrorCode(-32600), message: "missing Authorization header", .. })`.
    match err {
        ClientInitializeError::JsonRpcError(data) => {
            assert!(
                data.message.contains("missing Authorization header"),
                "expected the real AuthLayer rejection reason, got: {data:?}"
            );
        }
        other => panic!("expected ClientInitializeError::JsonRpcError carrying AuthLayer's real 401 body, got: {other:?}"),
    }
}

/// Regression guard for two real bugs that shipped past every prior review/test and
/// were only caught by a live external MCP client actually connecting (see commits
/// `c95632b` and `76cc70a`): (1) four tools used to return a bare `Json<Vec<T>>` /
/// `Json<Option<T>>`, which the MCP spec forbids — `outputSchema` must be an object
/// schema at the top level, and a bare collection/option produces an array/nullable
/// schema instead; (2) `get_daily_digest`'s `digest` field used to be `serde_json::
/// Value`, which `schemars` renders as an empty/fully-unconstrained property schema
/// that real clients' schema validators reject outright. Both are now fixed, but
/// nothing asserted on schema *shape* before this test — only on the values inside
/// `structured_content` — so a regression of either bug could ship silently. This
/// calls the real client's `list_tools()` over the real running server and checks,
/// for every one of the 7 tools, that its `outputSchema` is present, is an object
/// schema at the top level, and has no unconstrained/empty property.
#[tokio::test]
async fn every_tools_output_schema_is_a_well_formed_object_schema() {
    let (base_url, bearer, _meeting_id, _memory_only_bearer) = spawn_real_server().await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(base_url).auth_header(bearer),
    );
    let client = ClientInfo::default().serve(transport).await.expect(
        "a real rmcp client connects over streamable-HTTP and completes the initialize handshake",
    );

    let tools = client
        .list_tools(None)
        .await
        .expect("list_tools succeeds against a real running server")
        .tools;

    let expected_tool_names = [
        "search_memory",
        "list_meetings",
        "get_meeting",
        "get_transcript",
        "get_timeline",
        "get_daily_digest",
        "assemble_context",
    ];
    for expected_name in expected_tool_names {
        let tool = tools
            .iter()
            .find(|t| t.name == expected_name)
            .unwrap_or_else(|| {
                panic!("expected server to advertise a tool named {expected_name}, got: {tools:?}")
            });

        let output_schema = tool
            .output_schema
            .as_ref()
            .unwrap_or_else(|| panic!("{expected_name}: expected an outputSchema, got none"));

        assert_eq!(
            output_schema.get("type"),
            Some(&serde_json::Value::String("object".to_string())),
            "{expected_name}: outputSchema's top-level \"type\" must be \"object\" (a bare Vec/Option-returning \
             tool produces an array/nullable schema instead — this is the exact defect class that shipped in \
             commit c95632b), got: {output_schema:?}"
        );

        let properties = output_schema
            .get("properties")
            .and_then(|p| p.as_object())
            .unwrap_or_else(|| panic!("{expected_name}: expected outputSchema.properties to be an object, got: {output_schema:?}"));

        // Guards against the check below passing vacuously: an empty `properties` object
        // would make the per-property loop iterate zero times and assert nothing at all.
        // Every one of the 6 tools' output structs has at least one field, so this must
        // be non-empty on a well-formed schema.
        assert!(
            !properties.is_empty(),
            "{expected_name}: outputSchema.properties is empty, got: {output_schema:?}"
        );

        for (property_name, property_schema) in properties {
            let is_unconstrained = *property_schema == serde_json::json!({})
                || *property_schema == serde_json::Value::Bool(true);
            assert!(
                !is_unconstrained,
                "{expected_name}: property \"{property_name}\" has an empty/fully-unconstrained schema \
                 ({property_schema:?}) — this is exactly what schemars emits for a raw serde_json::Value field \
                 (the exact defect class that shipped in commit 76cc70a, get_daily_digest's `digest` field)"
            );
        }
    }

    let _ = client.cancel().await;
}
