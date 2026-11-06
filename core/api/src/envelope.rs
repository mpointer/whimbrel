//! The discriminated JSON envelope (LB0-5) shared by every governed LLM entry point:
//! `{"ok":true,"data":…}` on success, `{"ok":false,"error":{"kind":…}}` otherwise.
//!
//! The three string builders are byte-for-byte what `core/ffi` returned before this
//! crate existed; the tests at the bottom pin that. [`envelope_json`] is the typed
//! front door a Rust shell uses; the string builders stay public because `core/ffi`
//! still composes them directly for the functions that haven't moved yet.

use serde::Serialize;

/// Why a governed call produced no result: LB0-5's cap-vs-provider distinction, plus a
/// provider's monthly budget being spent.
#[derive(Debug, Clone, Copy)]
pub enum ApiError {
    /// The spend-cap pre-check (or a mid-operation re-check on a multi-chunk call)
    /// rejected the call. Carries scope and the exact spent/cap cents so a UI can say
    /// *which* cap and by how much, instead of a bare failure.
    SpendCap(llm::SpendCapError),
    /// Every link in the provider chain failed for a reason other than the spend cap
    /// (e.g. no local Ollama server running and no hosted fallback configured).
    ProviderFailure,
    /// Every provider the call could use is over its monthly budget, or the one provider
    /// it is pinned to is (`crate::budget`).
    BudgetSpent(crate::budget::BudgetSpent),
}

impl From<crate::budget::BudgetSpent> for ApiError {
    fn from(err: crate::budget::BudgetSpent) -> Self {
        ApiError::BudgetSpent(err)
    }
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

/// The `provider_failure` half of the envelope — see [`ok_envelope`]. Carries a
/// `reason` object ([`last_provider_failure`]) when this thread's latest chain walk
/// said why, e.g. `"reason":{"kind":"ollama_model_missing","model":"gemma4:12b"}`;
/// a shell that predates it ignores the extra key.
pub fn provider_failure_envelope() -> String {
    match last_provider_failure().and_then(|reason| serde_json::to_string(&reason).ok()) {
        Some(reason) => format!(r#"{{"ok":false,"error":{{"kind":"provider_failure","reason":{reason}}}}}"#),
        None => r#"{"ok":false,"error":{"kind":"provider_failure"}}"#.to_string(),
    }
}

/// Why the provider chain behind this thread's latest [`ApiError::ProviderFailure`]
/// failed, and forgets it: the first link's reason that names a fix (the provider the user
/// set up first), else the first. `None` when that failure never walked a chain. A Rust
/// shell calls this right after the error and shows
/// [`llm::ProviderFailureKind::user_message`].
pub fn last_provider_failure() -> Option<llm::ProviderFailureKind> {
    let failures = llm::take_last_chain_failures();
    let actionable = |k: &&llm::ProviderFailureKind| {
        !matches!(k, llm::ProviderFailureKind::Network { .. } | llm::ProviderFailureKind::Other { .. })
    };
    failures.iter().find(actionable).or(failures.first()).cloned()
}

/// The [`ApiError::ProviderFailure`] for a chain walk that failed, recording why for
/// [`last_provider_failure`]. The only way a reason is recorded.
pub fn chain_failed(failures: Vec<llm::ProviderFailureKind>) -> ApiError {
    llm::set_last_chain_failures(failures);
    ApiError::ProviderFailure
}

/// The `budget_spent` half of the envelope — see [`ok_envelope`].
pub fn budget_spent_error_envelope(err: &crate::budget::BudgetSpent) -> String {
    format!(
        r#"{{"ok":false,"error":{{"kind":"budget_spent","provider":"{}","spent_cents":{},"budget_cents":{}}}}}"#,
        err.provider.as_str(),
        err.spent_cents,
        err.budget_cents
    )
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
        Err(ApiError::BudgetSpent(err)) => Some(budget_spent_error_envelope(err)),
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
    fn provider_failure_envelope_says_why_once() {
        let _ = chain_failed(vec![
            llm::ProviderFailureKind::OllamaModelMissing { model: "gemma4:12b".into() },
            llm::ProviderFailureKind::OllamaNotRunning,
        ]);
        let failed: ApiResult<()> = Err(ApiError::ProviderFailure);
        assert_eq!(
            envelope_json(&failed).unwrap(),
            r#"{"ok":false,"error":{"kind":"provider_failure","reason":{"kind":"ollama_model_missing","model":"gemma4:12b"}}}"#
        );
        assert_eq!(
            envelope_json(&failed).unwrap(),
            r#"{"ok":false,"error":{"kind":"provider_failure"}}"#,
            "the reason is taken, so a later unrelated failure doesn't repeat it"
        );
    }

    #[test]
    fn last_provider_failure_prefers_a_reason_with_a_fix() {
        let network = llm::ProviderFailureKind::Network { provider: llm::Provider::Anthropic };
        let _ = chain_failed(vec![network.clone(), llm::ProviderFailureKind::OllamaNotRunning]);
        assert_eq!(last_provider_failure(), Some(llm::ProviderFailureKind::OllamaNotRunning));
        let _ = chain_failed(vec![network.clone()]);
        assert_eq!(last_provider_failure(), Some(network));
    }

    #[test]
    fn a_swallowed_walk_leaves_no_reason_for_an_unrelated_failure() {
        let chain = [llm::ChainLink {
            provider: llm::Provider::Ollama,
            model: "gemma4:12b".into(),
            client: Box::new(llm::OllamaClient::with_base_url("http://127.0.0.1:9", "gemma4:12b")),
        }];
        // A caller like topics or speaker names: the walk fails and the caller moves on.
        assert!(llm::try_chain(&chain, |l| l.client.complete("q")).is_none());
        let failed: ApiResult<()> = Err(ApiError::ProviderFailure);
        assert_eq!(envelope_json(&failed).unwrap(), r#"{"ok":false,"error":{"kind":"provider_failure"}}"#);
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

        let spent: ApiResult<Payload> = Err(crate::budget::BudgetSpent {
            provider: llm::Provider::OpenRouter,
            spent_cents: 2512,
            budget_cents: 2500,
        }
        .into());
        assert_eq!(
            envelope_json(&spent).unwrap(),
            r#"{"ok":false,"error":{"kind":"budget_spent","provider":"openrouter","spent_cents":2512,"budget_cents":2500}}"#
        );
    }
}
