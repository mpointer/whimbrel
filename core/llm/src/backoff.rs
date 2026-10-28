//! Retry classification for outbound AI provider calls. Ported from
//! llm-governance-gateway's `src/backoff.ts` `isRetryable` — the gateway's actual
//! backoff-timing logic was never ported: every real caller in this crate retries the
//! next chain link immediately rather than waiting, so there's nothing here to time.

use crate::LlmError;

/// Retry only on 429 (rate limited) and 5xx (server) — never on other statuses, which
/// are caller errors that won't succeed on retry, and never on `Network`/`Provider`
/// errors (no HTTP status to classify), matching llm-governance-gateway's `isRetryable`
/// exactly: a non-HTTP-status error is never retryable there either.
pub fn is_retryable(err: &LlmError) -> bool {
    match err {
        LlmError::Http { status, .. } => *status == 429 || (*status >= 500 && *status < 600),
        _ => false,
    }
}
