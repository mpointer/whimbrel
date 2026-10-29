//! FR-MEM-3: the end-of-day digest, through the same governance pipeline as every other
//! LLM call (spend-cap check → provider chain → usage logging). Moved from
//! `core/ffi::whimbrel_llm_daily_digest` (M3.0); the C function is now a thin wrapper.

use crate::envelope::{ApiError, ApiResult};
use crate::routing::{build_chain_for, RoutingConfig};
use crate::util::{log_usage_for, now_unix_ms};

/// Digest over `[start_unix_ms, end_unix_ms)`; the caller computes the bounds (e.g.
/// local-time day boundaries). Skips the model call *and* the spend-cap check entirely
/// when there were no items in range — no reason to spend, or to be refused, over
/// nothing. `ollama_model` names the Ollama link's model; hosted links come from
/// `routing` (see [`build_chain_for`]).
pub fn daily_digest(
    store: &memory::Store,
    ollama_model: &str,
    start_unix_ms: i64,
    end_unix_ms: i64,
    routing: &RoutingConfig,
) -> ApiResult<llm::DailyDigest> {
    let items = llm::gather_and_assemble_context_for_window(
        store,
        "",
        now_unix_ms(),
        start_unix_ms,
        end_unix_ms,
        20,
        true,
    );

    if items.is_empty() {
        return Ok(llm::DailyDigest {
            summary: "No activity captured today.".to_string(),
            open_loops: Vec::new(),
            token_usage: llm::TokenUsage::default(),
        });
    }

    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("digest"), now_unix_ms())?;

    let chain = build_chain_for(llm::TaskKind::Digest, ollama_model, routing);
    let Some((digest, entry)) = llm::try_chain(&chain, |entry| {
        llm::generate_daily_digest_from_context(
            entry.client.as_ref(),
            &items,
            &llm::profile::compose_instructions(&routing.daily_digest_instructions, &routing.profile),
            start_unix_ms,
        )
    }) else {
        return Err(ApiError::ProviderFailure);
    };
    log_usage_for(
        store,
        entry.provider,
        &entry.model,
        "digest",
        digest.token_usage,
    );
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors `core/ffi`'s `daily_digest_empty_items_skips_spend_cap_even_when_over_cap`
    /// at the typed layer: an empty window short-circuits before the cap check, so a
    /// tripped cap can't turn "nothing happened today" into a spend_cap error.
    #[test]
    fn empty_window_returns_placeholder_without_consulting_the_cap() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = now_unix_ms();
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                route: Some("ask".into()),
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: 10_000,
                cache_hit: false,
                trace_id: "t".into(),
                ts_unix_ms: now,
            })
            .unwrap();
        let routing = RoutingConfig {
            global_daily_cap_cents: Some(1),
            ..RoutingConfig::default()
        };
        // A window with no items at all; the store is otherwise empty.
        let digest = daily_digest(&store, "gemma4:12b", 0, 1, &routing).unwrap();
        assert_eq!(digest.summary, "No activity captured today.");
        assert!(digest.open_loops.is_empty());
    }
}
