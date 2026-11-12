//! Small shared helpers every governed call uses: wall-clock time and usage logging.
//! Moved verbatim from `core/ffi` (M3.0).

pub fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Logs one governed call's real usage after it succeeds. `route` tags the usage_log
/// row (e.g. `"ask"`/`"digest"`) for per-feature spend breakdowns without another
/// schema change.
pub fn log_usage_for(
    store: &memory::Store,
    provider: llm::Provider,
    model: &str,
    route: &str,
    usage: llm::TokenUsage,
) {
    let cost_cents = llm::estimate_cost_cents(provider, model, usage);
    // Computed once and shared by the trace id and ts_unix_ms below — two separate
    // now_unix_ms() calls could disagree by up to a millisecond, and if they landed in
    // the exact same millisecond, uuid_like_trace_id's format would collide for two
    // different calls.
    let now_ms = now_unix_ms();
    let _ = store.log_usage(&memory::NewUsageLogEntry {
        user_id: None,
        provider: provider.as_str().to_string(),
        model: model.to_string(),
        route: Some(route.to_string()),
        input_tokens: usage.input_tokens.map(i64::from),
        output_tokens: usage.output_tokens.map(i64::from),
        estimated_cost_cents: cost_cents,
        cache_hit: false,
        trace_id: uuid_like_trace_id(now_ms),
        ts_unix_ms: now_ms,
    });
}

/// [`log_usage_for`] under a span parent: every call one operation makes (a routine run,
/// an eval run, later an agent run) passes the same `parent_trace_id`, and
/// `Store::sum_usage_by_parent` rolls them up.
pub fn log_usage_in(
    store: &memory::Store,
    parent_trace_id: &str,
    provider: llm::Provider,
    model: &str,
    route: &str,
    usage: llm::TokenUsage,
) {
    let cost_cents = llm::estimate_cost_cents(provider, model, usage);
    let now_ms = now_unix_ms();
    let _ = store.log_usage_with_parent(
        &memory::NewUsageLogEntry {
            user_id: None,
            provider: provider.as_str().to_string(),
            model: model.to_string(),
            route: Some(route.to_string()),
            input_tokens: usage.input_tokens.map(i64::from),
            output_tokens: usage.output_tokens.map(i64::from),
            estimated_cost_cents: cost_cents,
            cache_hit: false,
            trace_id: uuid_like_trace_id(now_ms),
            ts_unix_ms: now_ms,
        },
        parent_trace_id,
    );
}

/// The model `link` actually answered with: its fallback when the provider had no
/// `link.model` (`llm::LlmClient::substituted_model`), else `link.model`.
pub fn answering_model(link: &llm::ChainLink) -> String {
    link.client.substituted_model().unwrap_or_else(|| link.model.clone())
}

/// [`log_usage_for`] for a chain link that answered: usage under the model that answered,
/// and that model's substitution recorded (or a recorded one cleared) for Settings
/// (`crate::model_defaults::note_answer`).
pub fn log_link_usage(store: &memory::Store, link: &llm::ChainLink, route: &str, usage: llm::TokenUsage) {
    let answered = link.client.substituted_model();
    log_usage_for(store, link.provider, answered.as_deref().unwrap_or(&link.model), route, usage);
    crate::model_defaults::note_answer(store, link.provider, &link.model, answered.as_deref(), now_unix_ms());
}

/// [`log_link_usage`] under a span parent ([`log_usage_in`]).
pub fn log_link_usage_in(store: &memory::Store, parent_trace_id: &str, link: &llm::ChainLink, route: &str, usage: llm::TokenUsage) {
    let answered = link.client.substituted_model();
    log_usage_in(store, parent_trace_id, link.provider, answered.as_deref().unwrap_or(&link.model), route, usage);
    crate::model_defaults::note_answer(store, link.provider, &link.model, answered.as_deref(), now_unix_ms());
}

/// A cheap, dependency-free trace id — no `uuid` dependency and no need for
/// cryptographic uniqueness for a local debugging aid, just something distinct per call
/// for correlating a usage_log row back to a specific request in logs.
pub fn uuid_like_trace_id(now_ms: i64) -> String {
    format!("{now_ms:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers, as if after standing `substitute` in for its model.
    struct Answered {
        substitute: Option<&'static str>,
    }

    impl llm::LlmClient for Answered {
        fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            Ok(llm::Completion { text: "a".into(), token_usage: llm::TokenUsage::default() })
        }
        fn substituted_model(&self) -> Option<String> {
            self.substitute.map(String::from)
        }
    }

    fn link(substitute: Option<&'static str>) -> llm::ChainLink {
        llm::ChainLink { provider: llm::Provider::Google, model: "gemini-2.5-pro".into(), client: Box::new(Answered { substitute }) }
    }

    #[test]
    fn usage_goes_under_the_model_that_answered_and_a_substitution_is_recorded() {
        let store = memory::Store::open_in_memory().unwrap();
        let usage = llm::TokenUsage { input_tokens: Some(1000), output_tokens: Some(1000) };
        log_link_usage(&store, &link(Some("gemini-flash-latest")), "ask", usage);
        let rows = store.list_usage(0, i64::MAX, 10).unwrap();
        assert_eq!(rows[0].model, "gemini-flash-latest");
        assert_eq!(answering_model(&link(Some("gemini-flash-latest"))), "gemini-flash-latest");
        let seen = crate::model_defaults::unavailable(&store);
        assert_eq!((seen[0].model.as_str(), seen[0].using.as_str()), ("gemini-2.5-pro", "gemini-flash-latest"));

        log_link_usage_in(&store, "parent", &link(None), "routine", usage);
        assert_eq!(store.list_usage(0, i64::MAX, 10).unwrap()[0].model, "gemini-2.5-pro");
        assert!(crate::model_defaults::unavailable(&store).is_empty(), "the chosen model answered again");
    }
}
