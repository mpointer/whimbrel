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

/// A cheap, dependency-free trace id — no `uuid` dependency and no need for
/// cryptographic uniqueness for a local debugging aid, just something distinct per call
/// for correlating a usage_log row back to a specific request in logs.
pub fn uuid_like_trace_id(now_ms: i64) -> String {
    format!("{now_ms:x}")
}
