//! Request limits that depend on who is asking (security review M4). Runs just inside the auth
//! layer, so the token is known, and before rmcp reads or parses anything.
//!
//! **Body size.** Only a token with the `write` scope calling `add_items` may send a body larger
//! than [`READ_BODY_BYTES`] (4 MiB, rmcp's own default), up to [`crate::origin::MAX_REQUEST_BODY_BYTES`]
//! (16 MiB, a full call of 100 items of 64 KB). Every other request is held to 4 MiB while it is read,
//! before rmcp builds a JSON tree of it. Over 4 MiB, the tool name is checked with a scan that keeps
//! nothing but the `method` and `params.name` strings, so a large body never becomes a JSON tree
//! here.
//!
//! **In flight.** A token with the `write` scope may have [`IN_FLIGHT_PER_WRITE_TOKEN`] POST requests
//! in flight, and all such tokens together [`IN_FLIGHT_ALL_WRITE_TOKENS`]; a request past either is
//! refused at once with 429. A request stays in flight until its response body has been read or
//! dropped (a tool runs while rmcp streams the response). Read tokens are not limited here; their
//! bodies are small.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum::body::Body;
use http::{Request, Response, StatusCode};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::{Layer, Service};

use crate::auth::AuthContext;
use crate::origin::{refusal, MAX_REQUEST_BODY_BYTES};

/// The largest body any request but a write token's `add_items` may send.
pub const READ_BODY_BYTES: usize = 4 * 1024 * 1024;
/// POST requests one write token may have in flight.
pub const IN_FLIGHT_PER_WRITE_TOKEN: usize = 2;
/// POST requests all write tokens together may have in flight.
pub const IN_FLIGHT_ALL_WRITE_TOKENS: usize = 4;

#[derive(Clone)]
pub struct LimitLayer {
    state: Arc<LimitState>,
}

struct LimitState {
    all: Arc<Semaphore>,
    per_token: Mutex<HashMap<i64, Arc<Semaphore>>>,
}

impl Default for LimitLayer {
    fn default() -> Self {
        LimitLayer {
            state: Arc::new(LimitState { all: Arc::new(Semaphore::new(IN_FLIGHT_ALL_WRITE_TOKENS)), per_token: Mutex::new(HashMap::new()) }),
        }
    }
}

impl<S> Layer<S> for LimitLayer {
    type Service = LimitMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LimitMiddleware { inner, state: Arc::clone(&self.state) }
    }
}

#[derive(Clone)]
pub struct LimitMiddleware<S> {
    inner: S,
    state: Arc<LimitState>,
}

fn too_large() -> Response<Body> {
    refusal(
        StatusCode::PAYLOAD_TOO_LARGE,
        rmcp::ErrorData::invalid_request(
            "the request body is too large",
            Some(serde_json::json!({ "hint": "Only add_items, with a write token, may send more than 4 MiB (at most 16 MiB). Send fewer items per call." })),
        ),
    )
}

fn too_many() -> Response<Body> {
    refusal(
        StatusCode::TOO_MANY_REQUESTS,
        rmcp::ErrorData::invalid_request(
            "too many requests in flight for this client",
            Some(serde_json::json!({ "hint": "Wait for a call to finish before sending the next." })),
        ),
    )
}

/// Whether a JSON-RPC body is a `tools/call` of `add_items`, read without building a JSON tree:
/// only `method` and `params.name` are kept, every other value is skipped.
fn is_add_items_call(body: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Params {
        name: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Envelope {
        method: Option<String>,
        params: Option<Params>,
    }
    serde_json::from_slice::<Envelope>(body).is_ok_and(|e| {
        e.method.as_deref() == Some("tools/call") && e.params.and_then(|p| p.name).as_deref() == Some("add_items")
    })
}

fn declared_length(req: &Request<Body>) -> Option<u64> {
    req.headers().get(http::header::CONTENT_LENGTH)?.to_str().ok()?.trim().parse().ok()
}

impl LimitState {
    /// This write token's places, or `None` when it or all write tokens are at their limit.
    fn admit(&self, token_id: i64) -> Option<(OwnedSemaphorePermit, OwnedSemaphorePermit)> {
        let mine = {
            let mut map = self.per_token.lock().unwrap_or_else(|p| p.into_inner());
            Arc::clone(map.entry(token_id).or_insert_with(|| Arc::new(Semaphore::new(IN_FLIGHT_PER_WRITE_TOKEN))))
        };
        let mine = mine.try_acquire_owned().ok()?;
        let all = Arc::clone(&self.all).try_acquire_owned().ok()?;
        Some((mine, all))
    }
}

/// A response body that holds the request's places until it is read to the end or dropped.
struct Held<S> {
    inner: S,
    _places: Option<(OwnedSemaphorePermit, OwnedSemaphorePermit)>,
}

impl<S: futures_core::Stream + Unpin> futures_core::Stream for Held<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<S> Service<Request<Body>> for LimitMiddleware<S>
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

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            if req.method() != http::Method::POST {
                return inner.call(req).await;
            }
            let writer = req.extensions().get::<AuthContext>().filter(|a| a.has_scope("write")).map(|a| a.client_id);
            let cap = if writer.is_some() { MAX_REQUEST_BODY_BYTES } else { READ_BODY_BYTES };
            if declared_length(&req).is_some_and(|n| n > cap as u64) {
                return Ok(too_large());
            }
            let places = match writer {
                Some(id) => match state.admit(id) {
                    Some(p) => Some(p),
                    None => return Ok(too_many()),
                },
                None => None,
            };
            let (parts, body) = req.into_parts();
            let Ok(bytes) = axum::body::to_bytes(body, cap).await else {
                return Ok(too_large());
            };
            if bytes.len() > READ_BODY_BYTES && !is_add_items_call(&bytes) {
                return Ok(too_large());
            }
            let response = inner.call(Request::from_parts(parts, Body::from(bytes))).await?;
            if places.is_none() {
                return Ok(response);
            }
            let (parts, body) = response.into_parts();
            let held = Held { inner: body.into_data_stream(), _places: places };
            Ok(Response::from_parts(parts, Body::from_stream(held)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_add_items_scan_reads_only_the_method_and_name() {
        assert!(is_add_items_call(br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"add_items","arguments":{"items":[]}}}"#));
        assert!(is_add_items_call(br#"{"params":{"arguments":{"x":[1,2,{"name":"search_memory"}]},"name":"add_items"},"method":"tools/call"}"#));
        assert!(!is_add_items_call(br#"{"method":"tools/call","params":{"name":"search_memory"}}"#));
        assert!(!is_add_items_call(br#"{"method":"initialize","params":{"name":"add_items"}}"#));
        assert!(!is_add_items_call(br#"[{"method":"tools/call","params":{"name":"add_items"}}]"#), "a batch is not one call");
        assert!(!is_add_items_call(b"not json"));
    }
}
