//! Token/scope auth for the local MCP server (FR-MCPS-4's consent/scope/token model).
//! Consumes `memory::Store::get_mcp_client_token` (Task 1). See
//! `docs/superpowers/specs/2026-08-02-mcp-server-read-tools-design.md`'s "Component 3"
//! and "Error handling" sections for the product-level requirements this implements.
//!
//! ## Step 1's investigation: how does a `#[tool]` method learn which token authenticated it?
//!
//! Read directly out of the cached `rmcp-3.0.1` source
//! (`find ~/.cargo/registry/src -iname "rmcp-3.0.1" -type d`), not guessed:
//!
//! - `rmcp::service::RequestContext<R>` (`rmcp-3.0.1/src/service.rs:1109-1117`) carries
//!   `pub extensions: crate::model::Extensions` — a `Send + Sync`-bounded type-map
//!   (`rmcp-3.0.1/src/model/extension.rs`), not just metadata.
//! - `StreamableHttpService` (`rmcp-3.0.1/src/transport/streamable_http_server/tower.rs`)
//!   consumes the incoming `http::Request`, and — per that file's own module doc comment
//!   ("Accessing HTTP request data from tool handlers") plus the actual code at (e.g.)
//!   line 1086 (`request.request.extensions_mut().insert(parts)`) — inserts the raw
//!   `http::request::Parts` it split off into that same per-request `extensions` map.
//!   `Parts` itself carries its own `.extensions: http::Extensions`, the exact type axum's
//!   `Extension` layer/extractor writes into. So anything an axum/tower middleware layer
//!   inserts into the *raw HTTP request's* extensions, before the request reaches
//!   `StreamableHttpService`, is reachable inside a tool method as:
//!   `ctx.extensions.get::<http::request::Parts>().unwrap().extensions.get::<AuthContext>()`.
//! - `rmcp-macros`' `#[tool]`/`#[tool_router]` expansion (confirmed in
//!   `rmcp-3.0.1/src/handler/server/tool.rs`'s `impl_for!` macro, which generates
//!   `CallToolHandler` impls for tuples of up to 16 type parameters) resolves *each*
//!   parameter of a tool method independently via `FromContextPart<ToolCallContext<S>>`.
//!   `RequestContext<RoleServer>` itself has a `FromContextPart` impl (via
//!   `AsRequestContext`, `rmcp-3.0.1/src/handler/server/common.rs`), so a tool method can
//!   take `ctx: rmcp::service::RequestContext<rmcp::RoleServer>` as an ordinary extra
//!   parameter, alongside its existing `Parameters<T>` parameter, with zero macro changes.
//!
//! **This is Path (A)** from the task brief, not the documented fallback (B) — the real
//! per-request extension mechanism exists, is documented by rmcp itself (not an
//! undocumented internal), and needs no raw JSON-RPC body inspection. Consequence: the
//! `AuthLayer`/`AuthMiddleware` below only ever look at the `Authorization` header — they
//! never touch the request body, and don't need to know tool names or the JSON-RPC
//! envelope shape at all. Per-tool scope checking (which specific scope a given tool
//! requires) happens one layer up, in `lib.rs`'s `WhimbrelMcpServer::check_scope`, because
//! only each tool method itself knows which scope it requires — the middleware's job ends
//! at "is this a valid, non-revoked token, and if so what scopes did it get granted."
//!
//! One deviation from the brief's own suggested `check_scope(&self, ctx: &RequestContext,
//! ...)` signature: `lib.rs`'s tool methods take `Extension(parts): Extension<http::
//! request::Parts>` (from `rmcp::handler::server::tool::Extension`, itself resolved via
//! `FromContextPart` exactly like `RequestContext` would be) instead of the full
//! `RequestContext<RoleServer>`. Both are equally real production paths, but a full
//! `RequestContext` needs a live `rmcp::service::Peer`, which (per `rmcp-3.0.1/src/
//! service.rs`) is only constructible through a real connected session — `Peer::new` is
//! `pub(crate)`. `http::request::Parts` has no such requirement and is directly
//! buildable in a test (`Request::builder().body(()).unwrap().into_parts().0`), so this
//! keeps every tool method's scope check unit-testable without standing up a real
//! in-process MCP client/server handshake per test. See this task's report for the full
//! reasoning.

use memory::Store;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum::body::Body;
use http::{Request, Response, StatusCode};
use tower::{Layer, Service};

/// Resolved bearer-token identity for one request. Inserted into the raw HTTP request's
/// extensions by [`AuthLayer`] on a successful token check; read back out of
/// `RequestContext::extensions` (see module doc above) by each tool method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthContext {
    pub client_id: i64,
    pub scopes: Vec<String>,
}

impl AuthContext {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

/// Checks a resolved [`AuthContext`] (or its absence) against a tool's required scope.
/// Pure and independently testable — no rmcp/axum/http types involved — because this is
/// where the actual "does this token cover what this tool needs" decision lives.
///
/// `auth` is `None` when the auth middleware never ran (misconfiguration — every request
/// that reaches a tool handler should have passed through [`AuthMiddleware`] first) or,
/// equivalently in tests, when simulating "no token at all." This is intentionally a
/// *different* error from "token present, but missing the required scope" — the design
/// spec calls for these to be distinguishable.
pub fn authorize(auth: Option<&AuthContext>, required_scope: &str) -> Result<(), rmcp::ErrorData> {
    match auth {
        None => Err(rmcp::ErrorData::internal_error(
            "no auth context on this request — the auth middleware did not run".to_string(),
            None,
        )),
        Some(auth) if auth.has_scope(required_scope) => Ok(()),
        Some(_) => Err(rmcp::ErrorData::invalid_request(
            format!("token not authorized for scope '{required_scope}'"),
            None,
        )),
    }
}

// Token generation/hashing (`generate_token`/`hash_secret`/`verify_secret`) originally
// lived here but moved to `memory::token` (Task 5) so `core/ffi` can share them without
// depending on this crate. Re-exported so existing callers/tests in this module keep
// working unchanged.
pub use memory::token::{generate_token, hash_secret, verify_secret, GeneratedToken};

/// Parses a bearer token of the form `kmcp_<id>.<secret>` into its numeric client id and
/// plaintext secret. Anything else (wrong prefix, missing/non-numeric id, no `.`
/// separator, empty secret) is rejected as malformed.
fn parse_bearer_token(token: &str) -> Option<(i64, &str)> {
    let rest = token.strip_prefix("kmcp_")?;
    let (id_str, secret) = rest.split_once('.')?;
    if secret.is_empty() {
        return None;
    }
    let id = id_str.parse::<i64>().ok()?;
    Some((id, secret))
}

/// Why a request was rejected before it ever reached a tool call. Kept distinct from
/// [`authorize`]'s scope-mismatch error: this module's rejections are all "no valid token
/// at all" cases (missing header, malformed value, unknown id, revoked, wrong secret) and
/// surface as a plain HTTP 401 — a tool's required *scope* isn't known at this layer (only
/// the tool method itself knows that; see module doc), so scope mismatches are never
/// decided here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthRejection {
    MissingAuthorizationHeader,
    MalformedAuthorizationHeader,
    MalformedToken,
    UnknownOrRevokedToken,
    WrongSecret,
    StoreUnavailable,
}

impl AuthRejection {
    fn reason(self) -> &'static str {
        match self {
            AuthRejection::MissingAuthorizationHeader => "missing Authorization header",
            AuthRejection::MalformedAuthorizationHeader => {
                "Authorization header must be 'Bearer <token>'"
            }
            AuthRejection::MalformedToken => {
                "token is not in the expected 'kmcp_<id>.<secret>' format"
            }
            AuthRejection::UnknownOrRevokedToken => "unknown or revoked MCP client token",
            AuthRejection::WrongSecret => "invalid MCP client token",
            AuthRejection::StoreUnavailable => "token store unavailable",
        }
    }

    /// Builds a real MCP-protocol error envelope (`rmcp::model::JsonRpcError`, the same
    /// type `rmcp` itself uses for its own pre-dispatch rejections — see e.g.
    /// `invalid_request_jsonrpc_response` in `rmcp-3.0.1/src/transport/
    /// streamable_http_server/tower.rs`), not a bespoke JSON shape. The design spec calls
    /// for "a clear MCP-protocol error" here, and a strict JSON-RPC client parsing this
    /// response needs `{"jsonrpc":"2.0","error":{"code":...,"message":...}}`, not an
    /// unrecognized custom shape — this middleware rejects before the request body is
    /// ever parsed (Path A's whole point — see module doc), so there's no JSON-RPC
    /// request `id` to echo back; `id: None` is exactly the case rmcp's own doc comment
    /// on `JsonRpcError` describes ("omitted when the server cannot read the request
    /// id... invalid request").
    fn into_response(self) -> Response<Body> {
        // `StoreUnavailable` (a poisoned mutex/DB error) isn't an auth decision at all —
        // give it 500, not 401, so it isn't confused with "this token doesn't work," and
        // a distinct hint that doesn't send the user to Settings for a problem that has
        // nothing to do with their token.
        let (status, error_data) = match self {
            AuthRejection::StoreUnavailable => (
                StatusCode::INTERNAL_SERVER_ERROR,
                rmcp::ErrorData::internal_error(
                    self.reason(),
                    Some(serde_json::json!({
                        "hint": "This is a server-side problem (the token store is unavailable), not a token problem — check the Whimbrel MCP server's own logs.",
                    })),
                ),
            ),
            _ => (
                StatusCode::UNAUTHORIZED,
                rmcp::ErrorData::invalid_request(
                    self.reason(),
                    Some(serde_json::json!({
                        "hint": "Generate or check this client's token in Whimbrel Settings \u{2192} MCP Clients.",
                    })),
                ),
            ),
        };
        let envelope = rmcp::model::JsonRpcError::new(None, error_data);
        let body = serde_json::to_vec(&envelope).expect("serializing a JsonRpcError cannot fail");
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Body::from(body))
            .expect("building an error JSON-RPC response cannot fail")
    }
}

/// Resolves a request's `Authorization` header against the store, independent of any
/// axum/tower plumbing — the part of this module with actual branching logic worth
/// testing directly, and tested directly in `mod tests` below.
fn authenticate(
    store: &Mutex<Store>,
    headers: &http::HeaderMap,
) -> Result<AuthContext, AuthRejection> {
    let header_value = headers
        .get(http::header::AUTHORIZATION)
        .ok_or(AuthRejection::MissingAuthorizationHeader)?
        .to_str()
        .map_err(|_| AuthRejection::MalformedAuthorizationHeader)?;

    let token = header_value
        .strip_prefix("Bearer ")
        .ok_or(AuthRejection::MalformedAuthorizationHeader)?;

    let (client_id, secret) = parse_bearer_token(token).ok_or(AuthRejection::MalformedToken)?;

    let stored = {
        let store = store.lock().map_err(|_| AuthRejection::StoreUnavailable)?;
        store.get_mcp_client_token(client_id)
    };
    let stored = match stored {
        Ok(Some(row)) => row,
        Ok(None) => return Err(AuthRejection::UnknownOrRevokedToken),
        Err(_) => return Err(AuthRejection::StoreUnavailable),
    };
    if stored.revoked_at_unix_ms.is_some() {
        return Err(AuthRejection::UnknownOrRevokedToken);
    }
    if !verify_secret(secret, &stored.token_salt, &stored.token_hash) {
        return Err(AuthRejection::WrongSecret);
    }

    Ok(AuthContext {
        client_id: stored.id,
        scopes: stored.scopes,
    })
}

/// `tower::Layer` that wraps whatever inner service handles `/mcp` (Task 4 nests
/// rmcp's `StreamableHttpService` under an `axum::Router`; this layer wraps that
/// router/service, applied before the request ever reaches rmcp's own dispatch). Every
/// request must carry a valid, non-revoked bearer token or the request is rejected here
/// with 401 — the inner service is never called.
#[derive(Clone)]
pub struct AuthLayer {
    store: Arc<Mutex<Store>>,
}

impl AuthLayer {
    pub fn new(store: Arc<Mutex<Store>>) -> Self {
        Self { store }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthMiddleware {
            inner,
            store: Arc::clone(&self.store),
        }
    }
}

#[derive(Clone)]
pub struct AuthMiddleware<S> {
    inner: S,
    store: Arc<Mutex<Store>>,
}

impl<S> Service<Request<Body>> for AuthMiddleware<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let store = Arc::clone(&self.store);
        // `poll_ready` was called on `self.inner`, not on a clone — the standard tower
        // middleware idiom is to keep that already-ready service for this call and leave
        // a fresh (not-yet-polled) clone in `self.inner` for the next one.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            match authenticate(&store, req.headers()) {
                Ok(auth_context) => {
                    req.extensions_mut().insert(auth_context);
                    inner.call(req).await
                }
                Err(rejection) => Ok(rejection.into_response()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn store_with_token(scopes: Vec<&str>) -> (Arc<Mutex<Store>>, String) {
        let store = Store::open_in_memory().unwrap();
        let generated = generate_token();
        let id = store
            .insert_mcp_client_token(&memory::NewMcpClientToken {
                name: "test client".to_string(),
                token_hash: generated.hash.clone(),
                token_salt: generated.salt.clone(),
                scopes: scopes.into_iter().map(str::to_string).collect(),
            })
            .unwrap();
        let bearer = format!("kmcp_{id}.{secret}", secret = generated.plaintext_secret);
        (Arc::new(Mutex::new(store)), bearer)
    }

    // Token-hashing helper tests (generate_token/hash_secret/verify_secret) moved to
    // memory::token's own test module alongside the code (Task 5) — see
    // core/memory/src/token.rs.

    // --- bearer token parsing ---

    #[test]
    fn parses_a_well_formed_bearer_token() {
        assert_eq!(
            parse_bearer_token("kmcp_42.deadbeef"),
            Some((42, "deadbeef"))
        );
    }

    #[test]
    fn rejects_tokens_without_the_kmcp_prefix() {
        assert_eq!(parse_bearer_token("42.deadbeef"), None);
    }

    #[test]
    fn rejects_tokens_without_a_separator() {
        assert_eq!(parse_bearer_token("kmcp_42deadbeef"), None);
    }

    #[test]
    fn rejects_tokens_with_a_non_numeric_id() {
        assert_eq!(parse_bearer_token("kmcp_abc.deadbeef"), None);
    }

    #[test]
    fn rejects_tokens_with_an_empty_secret() {
        assert_eq!(parse_bearer_token("kmcp_42."), None);
    }

    // --- authorize() — scope check against a resolved AuthContext (Step 5, scope part) ---

    #[test]
    fn authorize_succeeds_when_the_granted_scopes_include_the_required_one() {
        let auth = AuthContext {
            client_id: 1,
            scopes: vec!["memory".to_string()],
        };
        assert!(authorize(Some(&auth), "memory").is_ok());
    }

    #[test]
    fn authorize_fails_distinctly_when_a_token_is_valid_but_lacks_the_scope() {
        let auth = AuthContext {
            client_id: 1,
            scopes: vec!["meetings".to_string()],
        };
        let err = authorize(Some(&auth), "memory").unwrap_err();
        assert!(err.message.contains("not authorized for scope"));

        let no_token_err = authorize(None, "memory").unwrap_err();
        assert_ne!(
            err.message, no_token_err.message,
            "wrong-scope and no-token-at-all must be distinguishable"
        );
    }

    #[test]
    fn authorize_fails_when_there_is_no_auth_context_at_all() {
        assert!(authorize(None, "memory").is_err());
    }

    // --- AuthMiddleware — the axum/tower layer (Step 5) ---

    async fn probe(store: Arc<Mutex<Store>>, header: Option<&str>) -> Response<Body> {
        let inner = tower::service_fn(|req: Request<Body>| async move {
            // Proves a successful auth check actually reaches the inner service, and that
            // the resolved AuthContext really is attached to the request by this point.
            let auth = req.extensions().get::<AuthContext>().cloned();
            let body = serde_json::to_string(&auth.map(|a| a.scopes)).unwrap();
            Ok::<_, std::convert::Infallible>(Response::new(Body::from(body)))
        });
        let mut svc = AuthLayer::new(store).layer(inner);

        let mut builder = Request::builder().method("POST").uri("/mcp");
        if let Some(header) = header {
            builder = builder.header(http::header::AUTHORIZATION, header);
        }
        let req = builder.body(Body::empty()).unwrap();

        svc.ready().await.unwrap().call(req).await.unwrap()
    }

    #[tokio::test]
    async fn valid_token_reaches_the_inner_service_with_its_granted_scopes_attached() {
        let (store, bearer) = store_with_token(vec!["memory", "meetings"]);
        let response = probe(store, Some(&format!("Bearer {bearer}"))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let scopes: Option<Vec<String>> = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            scopes,
            Some(vec!["memory".to_string(), "meetings".to_string()])
        );
    }

    #[tokio::test]
    async fn missing_authorization_header_is_rejected_with_401() {
        let (store, _bearer) = store_with_token(vec!["memory"]);
        let response = probe(store, None).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// A strict JSON-RPC client parsing this response needs a real MCP-protocol error
    /// envelope, not an unrecognized bespoke shape — asserts the body actually
    /// deserializes as `rmcp::model::JsonRpcError` (the same type rmcp's own internal
    /// pre-dispatch rejections use), with `id: None` (no request id was ever read,
    /// since this middleware rejects before the JSON-RPC body is parsed at all) and the
    /// specific rejection reason preserved in `error.message`.
    #[tokio::test]
    async fn a_401_rejection_body_is_a_real_jsonrpc_error_envelope() {
        let (store, _bearer) = store_with_token(vec!["memory"]);
        let response = probe(store, None).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let envelope: rmcp::model::JsonRpcError = serde_json::from_slice(&body)
            .expect("401 body must parse as a real JsonRpcError, not a bespoke JSON shape");
        assert!(envelope.id.is_none());
        assert!(envelope
            .error
            .message
            .contains("missing Authorization header"));
        let hint = envelope
            .error
            .data
            .as_ref()
            .and_then(|d| d.get("hint"))
            .and_then(|h| h.as_str());
        assert!(
            hint.is_some_and(|h| h.contains("Settings")),
            "hint text should still point at Settings"
        );
    }

    #[tokio::test]
    async fn malformed_bearer_token_is_rejected_with_401() {
        let (store, _bearer) = store_with_token(vec!["memory"]);
        let response = probe(store, Some("Bearer not-a-real-token")).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn revoked_token_is_rejected_with_401() {
        let (store, bearer) = store_with_token(vec!["memory"]);
        {
            let id: i64 = bearer
                .strip_prefix("kmcp_")
                .unwrap()
                .split_once('.')
                .unwrap()
                .0
                .parse()
                .unwrap();
            store.lock().unwrap().revoke_mcp_client_token(id).unwrap();
        }
        let response = probe(store, Some(&format!("Bearer {bearer}"))).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_secret_against_a_real_token_id_is_rejected_with_401() {
        let (store, bearer) = store_with_token(vec!["memory"]);
        let id = bearer
            .strip_prefix("kmcp_")
            .unwrap()
            .split_once('.')
            .unwrap()
            .0;
        let response = probe(
            store,
            Some(&format!("Bearer kmcp_{id}.wrongsecretwrongsecret")),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// A poisoned mutex (some other panic while a lock was held — an internal problem)
    /// must not be reported the same way as an actual bad token: 500, not 401, and a hint
    /// that doesn't send the user chasing a token problem that doesn't exist.
    #[tokio::test]
    async fn a_poisoned_store_is_rejected_with_500_and_a_hint_distinct_from_token_errors() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        {
            let poison = Arc::clone(&store);
            let _ = std::thread::spawn(move || {
                let _guard = poison.lock().unwrap();
                panic!("deliberately poisoning the mutex for this test");
            })
            .join();
        }

        let response = probe(store, Some("Bearer kmcp_1.deadbeef")).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let envelope: rmcp::model::JsonRpcError = serde_json::from_slice(&body).unwrap();
        assert!(envelope.error.message.contains("token store unavailable"));
        let hint = envelope
            .error
            .data
            .as_ref()
            .and_then(|d| d.get("hint"))
            .and_then(|h| h.as_str())
            .unwrap();
        assert!(!hint.contains("Settings"), "a store-unavailable hint shouldn't send the user to Settings looking for a token problem");
    }
}
