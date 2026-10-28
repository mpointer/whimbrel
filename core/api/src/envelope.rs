//! The discriminated JSON envelope (LB0-5) shared by every governed LLM entry point:
//! `{"ok":true,"data":…}` on success, `{"ok":false,"error":{"kind":…}}` otherwise.
//!
//! The three string builders are byte-for-byte what `core/ffi` returned before this
//! crate existed; the tests at the bottom pin that. [`envelope_json`] is the typed
//! front door a Rust shell uses; the string builders stay public because `core/ffi`
//! still composes them directly for the functions that haven't moved yet.

use serde::Serialize;

/// Why a governed call produced no result. Deliberately just two variants — LB0-5
/// distinguishes cap-vs-provider, not per-provider detail.
#[derive(Debug, Clone, Copy)]
pub enum ApiError {
    /// The spend-cap pre-check (or a mid-operation re-check on a multi-chunk call)
    /// rejected the call. Carries scope and the exact spent/cap cents so a UI can say
    /// *which* cap and by how much, instead of a bare failure.
    SpendCap(llm::SpendCapError),
    /// Every link in the provider chain failed for a reason other than the spend cap
    /// (e.g. no local Ollama server running and no hosted fallback configured).
    ProviderFailure,
}

impl From<llm::SpendCapError> for ApiError {
    fn from(err: llm::SpendCapError) -> Self {
        ApiError::SpendCap(err)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Wraps an already-serialized JSON payload as `{"ok":true,"data":<payload>}`.
/// `data_json` must already be valid JSON (a serialized struct), not raw text.
pub fn ok_envelope(data_json: &str) -> String {
    format!(r#"{{"ok":true,"data":{data_json}}}"#)
}

/// The `spend_cap` half of the envelope — see [`ok_envelope`].
pub fn spend_cap_error_envelope(err: &llm::SpendCapError) -> String {
    let scope = match err.scope {
        llm::SpendCapScope::User => "user",
        llm::SpendCapScope::Global => "global",
    };
    format!(
        r#"{{"ok":false,"error":{{"kind":"spend_cap","scope":"{scope}","spent_cents":{},"cap_cents":{}}}}}"#,
        err.spent_cents, err.cap_cents
    )
}

/// The `provider_failure` half of the envelope — see [`ok_envelope`].
pub fn provider_failure_envelope() -> String {
    r#"{"ok":false,"error":{"kind":"provider_failure"}}"#.to_string()
}

/// Encodes a typed result as the envelope. Returns `None` only if `data` fails to
/// serialize — the C layer maps that to a bare NULL, its existing convention for
/// "internal encoding failure" as distinct from a well-formed `{"ok":false,…}`.
pub fn envelope_json<T: Serialize>(result: &ApiResult<T>) -> Option<String> {
    match result {
        Ok(data) => serde_json::to_string(data)
            .ok()
            .map(|json| ok_envelope(&json)),
        Err(ApiError::SpendCap(err)) => Some(spend_cap_error_envelope(err)),
        Err(ApiError::ProviderFailure) => Some(provider_failure_envelope()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_envelope_wraps_data_json_verbatim() {
        assert_eq!(
            ok_envelope(r#"{"text":"hi"}"#),
            r#"{"ok":true,"data":{"text":"hi"}}"#
        );
    }

    #[test]
    fn spend_cap_error_envelope_encodes_user_scope() {
        let err = llm::SpendCapError {
            spent_cents: 42,
            cap_cents: 200,
            scope: llm::SpendCapScope::User,
        };
        assert_eq!(
            spend_cap_error_envelope(&err),
            r#"{"ok":false,"error":{"kind":"spend_cap","scope":"user","spent_cents":42,"cap_cents":200}}"#
        );
    }

    #[test]
    fn spend_cap_error_envelope_encodes_global_scope() {
        let err = llm::SpendCapError {
            spent_cents: 5000,
            cap_cents: 5000,
            scope: llm::SpendCapScope::Global,
        };
        assert_eq!(
            spend_cap_error_envelope(&err),
            r#"{"ok":false,"error":{"kind":"spend_cap","scope":"global","spent_cents":5000,"cap_cents":5000}}"#
        );
    }

    #[test]
    fn provider_failure_envelope_is_the_fixed_shape() {
        assert_eq!(
            provider_failure_envelope(),
            r#"{"ok":false,"error":{"kind":"provider_failure"}}"#
        );
    }

    #[derive(Serialize)]
    struct Payload {
        text: &'static str,
    }

    #[test]
    fn envelope_json_matches_the_string_builders() {
        let ok: ApiResult<Payload> = Ok(Payload { text: "hi" });
        assert_eq!(
            envelope_json(&ok).unwrap(),
            r#"{"ok":true,"data":{"text":"hi"}}"#
        );

        let cap: ApiResult<Payload> = Err(llm::SpendCapError {
            spent_cents: 42,
            cap_cents: 200,
            scope: llm::SpendCapScope::User,
        }
        .into());
        assert_eq!(
            envelope_json(&cap).unwrap(),
            r#"{"ok":false,"error":{"kind":"spend_cap","scope":"user","spent_cents":42,"cap_cents":200}}"#
        );

        let failed: ApiResult<Payload> = Err(ApiError::ProviderFailure);
        assert_eq!(
            envelope_json(&failed).unwrap(),
            r#"{"ok":false,"error":{"kind":"provider_failure"}}"#
        );
    }
}
