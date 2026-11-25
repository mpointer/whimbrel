//! The server's front door, before the token is looked up (bridge spec section 10, "a hostile
//! local process" and browsers).
//!
//! **No web page may reach the server.** Local MCP clients (Claude Code, Claude Desktop, the
//! stdio launcher) send no `Origin` header; every browser `fetch`, form post and CORS preflight
//! does. So any request carrying an `Origin`, whatever its value (`null`, a loopback origin, any
//! site, bytes that are not text), is refused with 403 and a JSON-RPC error envelope. The value is
//! never parsed: being present is enough. rmcp itself refuses a `Host` other than `localhost`,
//! `127.0.0.1` or `::1` (DNS rebinding), one layer further in.
//!
//! **No oversized body is read.** A request that declares a body over [`MAX_REQUEST_BODY_BYTES`]
//! in `Content-Length` is refused with 413 here, before the token is looked up. A chunked body
//! with no length is held to the same cap by rmcp while it streams
//! (`StreamableHttpServerConfig::max_request_body_bytes`, set in `build_router_with_filter_store`).
//! Inside the auth layer, `limits.rs` holds every request but a write token's `add_items` to 4 MiB.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use http::{Request, Response, StatusCode};
use tower::{Layer, Service};

/// The largest request body the server reads: 16 MiB. A full `add_items` call is 100 items of at
/// most 64 KiB of text each (6.25 MiB as raw text); JSON escaping of quotes, newlines and other
/// control characters roughly doubles text that is dense with them, and the other fields add at
/// most about 0.5 MiB. rmcp's own default (4 MiB) would refuse a legal full call.
pub const MAX_REQUEST_BODY_BYTES: usize = 16 * 1024 * 1024;

const WEB_PAGE_MESSAGE: &str = "requests from a web page are not accepted";

/// A refusal with a JSON-RPC error envelope (no request id).
pub(crate) fn refusal(status: StatusCode, error: rmcp::ErrorData) -> Response<Body> {
    let envelope = rmcp::model::JsonRpcError::new(None, error);
    let body = serde_json::to_vec(&envelope).unwrap_or_default();
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/json"));
    response
}

fn web_page_refusal() -> Response<Body> {
    refusal(
        StatusCode::FORBIDDEN,
        rmcp::ErrorData::invalid_request(
            WEB_PAGE_MESSAGE,
            Some(serde_json::json!({ "hint": "Whimbrel's MCP server only accepts local apps, not browsers." })),
        ),
    )
}

fn too_large_refusal() -> Response<Body> {
    refusal(
        StatusCode::PAYLOAD_TOO_LARGE,
        rmcp::ErrorData::invalid_request(
            format!("the request body is larger than {MAX_REQUEST_BODY_BYTES} bytes"),
            Some(serde_json::json!({ "hint": "Send fewer items per call." })),
        ),
    )
}

/// Whether a request declares a body over [`MAX_REQUEST_BODY_BYTES`]. A `Content-Length` that is
/// not a number is left to the HTTP stack (hyper refuses it before it gets here).
fn declares_too_large(req: &Request<Body>) -> bool {
    req.headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .is_some_and(|n| n > MAX_REQUEST_BODY_BYTES as u64)
}

/// Refuses any request with an `Origin` header (403), then any that declares an oversized body
/// (413), before the inner service (the token check) runs.
#[derive(Clone, Copy, Default)]
pub struct OriginLayer;

impl<S> Layer<S> for OriginLayer {
    type Service = OriginMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        OriginMiddleware { inner }
    }
}

#[derive(Clone)]
pub struct OriginMiddleware<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for OriginMiddleware<S>
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
        if req.headers().contains_key(http::header::ORIGIN) {
            return Box::pin(async { Ok(web_page_refusal()) });
        }
        if declares_too_large(&req) {
            return Box::pin(async { Ok(too_large_refusal()) });
        }
        // As in `auth.rs`: keep the service `poll_ready` readied for this call.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move { inner.call(req).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Sends one request through the layer to an inner service that counts its calls.
    async fn probe(req: Request<Body>) -> (Response<Body>, usize) {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let inner = tower::service_fn(move |_req: Request<Body>| {
            seen.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, std::convert::Infallible>(Response::new(Body::from("inner"))) }
        });
        let mut svc = OriginLayer.layer(inner);
        let response = svc.ready().await.unwrap().call(req).await.unwrap();
        (response, calls.load(Ordering::SeqCst))
    }

    async fn error_of(response: Response<Body>) -> rmcp::model::JsonRpcError {
        assert_eq!(response.headers()[http::header::CONTENT_TYPE], "application/json");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn post() -> http::request::Builder {
        Request::builder().method("POST").uri("/mcp")
    }

    #[tokio::test]
    async fn a_request_with_any_origin_is_refused_before_the_token_is_read() {
        let origins: Vec<http::HeaderValue> = vec![
            http::HeaderValue::from_static("https://evil.example"),
            http::HeaderValue::from_static("null"),
            http::HeaderValue::from_static("http://127.0.0.1:4319"),
            http::HeaderValue::from_static("http://localhost:4319"),
            http::HeaderValue::from_static(""),
            http::HeaderValue::from_bytes(b"\xff\xfe not text").unwrap(),
        ];
        for origin in origins {
            let req = post()
                .header(http::header::ORIGIN, origin.clone())
                .header(http::header::AUTHORIZATION, "Bearer kmcp_1.secret")
                .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#))
                .unwrap();
            let (response, calls) = probe(req).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{origin:?}");
            assert_eq!(calls, 0, "{origin:?}: the token check never ran");
            let err = error_of(response).await;
            assert!(err.id.is_none());
            assert!(err.error.message.contains("web page"), "{}", err.error.message);
        }
    }

    #[tokio::test]
    async fn a_preflight_is_refused() {
        let req = Request::builder()
            .method("OPTIONS")
            .uri("/mcp")
            .header(http::header::ORIGIN, "https://evil.example")
            .header(http::header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(http::header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization, content-type")
            .body(Body::empty())
            .unwrap();
        let (response, calls) = probe(req).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(calls, 0);
        assert!(response.headers().get(http::header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    }

    #[tokio::test]
    async fn a_request_without_origin_passes() {
        let req = post().header(http::header::CONTENT_LENGTH, MAX_REQUEST_BODY_BYTES.to_string()).body(Body::empty()).unwrap();
        let (response, calls) = probe(req).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn a_declared_oversized_body_is_refused_before_the_token_is_read() {
        for length in [(MAX_REQUEST_BODY_BYTES + 1).to_string(), u64::MAX.to_string()] {
            let req = post().header(http::header::CONTENT_LENGTH, length.as_str()).body(Body::empty()).unwrap();
            let (response, calls) = probe(req).await;
            assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE, "{length}");
            assert_eq!(calls, 0);
            let err = error_of(response).await;
            assert!(err.error.message.contains("larger than"), "{}", err.error.message);
        }
    }

    #[test]
    fn a_full_add_items_call_fits_under_the_cap() {
        let max_text = memory::mcp_items::MAX_TEXT_BYTES * memory::mcp_items::MAX_ITEMS_PER_CALL;
        assert!(MAX_REQUEST_BODY_BYTES >= 2 * max_text + 512 * 1024, "{MAX_REQUEST_BODY_BYTES} < {max_text} doubled");
    }
}
