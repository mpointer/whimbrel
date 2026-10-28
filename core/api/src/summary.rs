//! FR-MTG-5: the governed meeting summary — the one governed call whose spend-cap check
//! can trip *mid-operation* (per chunk of a long transcript), not just before the first
//! provider call. Moved from `core/ffi::whimbrel_llm_summarize_meeting` and its helpers
//! (M3.0); see docs/superpowers/specs/2026-07-27-meeting-summary-spend-cap-governance-design.md.

use serde::{Serialize, Serializer};

use crate::envelope::{ApiError, ApiResult};
use crate::routing::{build_chain, RoutingConfig};
use crate::util::{log_usage_for, now_unix_ms};

/// The two ways [`summarize_meeting_governed`] can fail, kept distinct so the chain loop
/// in [`summarize_meeting`] can tell a spend-cap rejection (never retryable, never worth
/// trying the next chain link) from an actual provider failure (which tries the next link
/// when `llm::falls_through` says it's that link's own problem) — LB0-5's distinction
/// applied to the one entry point whose spend-cap check can trip mid-operation.
#[derive(Debug)]
pub enum GovernedSummaryError {
    SpendCap(llm::SpendCapError),
    Llm(llm::LlmError),
}

/// Governed version of `llm::summarize_meeting`: identical map-reduce behavior, but
/// checks the spend cap and logs usage after EVERY actual LLM call, not once for the
/// whole operation. A cap hit here aborts immediately with no further spend; whatever
/// succeeded before the abort (or before a later call failure) has already been logged
/// and stays logged — which is why [`summarize_meeting`]'s chain loop does not call
/// `log_usage_for` itself on success.
#[allow(clippy::too_many_arguments)]
pub fn summarize_meeting_governed(
    store: &memory::Store,
    cap_config: &llm::SpendCapConfig,
    client: &dyn llm::LlmClient,
    provider: llm::Provider,
    model: &str,
    notes: &str,
    transcript: &str,
    slides: &str,
    custom_instructions: &str,
) -> Result<llm::MeetingSummary, GovernedSummaryError> {
    let max_chars = llm::max_transcript_chars_per_chunk();
    let chunks = llm::chunk_transcript(transcript, max_chars);

    if chunks.len() == 1 && chunks[0].len() <= max_chars {
        if let Err(cap_err) = llm::check_spend_cap(
            store,
            cap_config,
            None,
            Some("meeting_summary"),
            now_unix_ms(),
        ) {
            return Err(GovernedSummaryError::SpendCap(cap_err));
        }
        let summary =
            llm::summarize_whole_transcript(client, notes, &chunks[0], slides, custom_instructions)
                .map_err(GovernedSummaryError::Llm)?;
        log_usage_for(
            store,
            provider,
            model,
            "meeting_summary",
            summary.token_usage,
        );
        return Ok(summary);
    }

    let mut partials = Vec::with_capacity(chunks.len());
    let mut total_usage = llm::TokenUsage::default();
    for (i, chunk) in chunks.iter().enumerate() {
        if let Err(cap_err) = llm::check_spend_cap(
            store,
            cap_config,
            None,
            Some("meeting_summary"),
            now_unix_ms(),
        ) {
            return Err(GovernedSummaryError::SpendCap(cap_err));
        }
        let (summary, usage) = llm::summarize_chunk(
            client,
            i,
            chunks.len(),
            notes,
            chunk,
            custom_instructions,
        )
        .map_err(GovernedSummaryError::Llm)?;
        log_usage_for(store, provider, model, "meeting_summary", usage);
        total_usage = llm::add_token_usage(total_usage, usage);
        partials.push(summary);
    }

    if let Err(cap_err) = llm::check_spend_cap(
        store,
        cap_config,
        None,
        Some("meeting_summary"),
        now_unix_ms(),
    ) {
        return Err(GovernedSummaryError::SpendCap(cap_err));
    }
    let (mut summary, reduce_usage) =
        llm::reduce_chunk_summaries(client, notes, &partials, slides, custom_instructions)
            .map_err(GovernedSummaryError::Llm)?;
    log_usage_for(store, provider, model, "meeting_summary", reduce_usage);
    total_usage = llm::add_token_usage(total_usage, reduce_usage);
    summary.token_usage = total_usage;
    Ok(summary)
}

/// The chain loop's per-error decision in [`summarize_meeting`], a pure function so the
/// single invariant it exists for — a spend-cap trip must always abort the chain, never
/// try the next link — has regression coverage independent of the loop body.
pub enum ChainLoopAction {
    TryNextLink,
    StopWithSpendCap(llm::SpendCapError),
    StopWithProviderFailure,
}

/// See [`ChainLoopAction`]. A `SpendCap` error always stops the chain; an `Llm` error
/// stops or continues per `llm::falls_through`, the rule `llm::try_chain` uses: a failure
/// specific to one link (unreachable, a refused key, a missing model, 429/5xx, …) tries
/// the next link, and a request-shaped rejection stops.
pub fn chain_loop_action_for(err: GovernedSummaryError) -> ChainLoopAction {
    match err {
        GovernedSummaryError::SpendCap(cap_err) => ChainLoopAction::StopWithSpendCap(cap_err),
        GovernedSummaryError::Llm(e) if llm::falls_through(&e) => ChainLoopAction::TryNextLink,
        GovernedSummaryError::Llm(_) => ChainLoopAction::StopWithProviderFailure,
    }
}

/// A [`llm::MeetingSummary`] plus the provider/model that produced it. Serializes as the
/// summary's own object with `provider` and `model` inserted after the fact — those two
/// live on `Provider`/the model string, not on `MeetingSummary`, so adding them as struct
/// fields there would force every other `MeetingSummary` caller to carry them. The
/// `Serialize` impl goes through `serde_json::Value` exactly as the old string builder
/// did, so the emitted bytes (key order included) are unchanged.
#[derive(Debug, Clone)]
pub struct MeetingSummaryData {
    pub summary: llm::MeetingSummary,
    pub provider: llm::Provider,
    pub model: String,
}

impl MeetingSummaryData {
    fn to_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        let mut json_value = serde_json::to_value(&self.summary)?;
        if let Some(obj) = json_value.as_object_mut() {
            obj.insert(
                "provider".to_string(),
                serde_json::Value::String(self.provider.as_str().to_string()),
            );
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(self.model.clone()),
            );
        }
        Ok(json_value)
    }
}

impl Serialize for MeetingSummaryData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

/// Serializes a summary with `provider`/`model` attribution — the string form of
/// [`MeetingSummaryData`], kept for direct unit coverage without a live provider.
pub fn summary_json_with_attribution(
    summary: &llm::MeetingSummary,
    provider: llm::Provider,
    model: &str,
) -> Option<String> {
    serde_json::to_string(&MeetingSummaryData {
        summary: summary.clone(),
        provider,
        model: model.to_string(),
    })
    .ok()
}

/// Blends `notes` with `transcript` into a structured summary through the full
/// governance pipeline (spend-cap check, provider chain with failover, per-call usage
/// logging), then writes the summary onto the `meeting` row `meeting_id` and retitles a
/// placeholder-titled meeting from the TL;DR (#65). On any failure the meeting row's
/// summary fields are left untouched. `custom_instructions` is trimmed; whitespace-only
/// means "no instructions". A `SpendCap` error can come either before the first provider
/// is tried or mid-operation on a later chunk; both surface the same way.
#[allow(clippy::too_many_arguments)]
pub fn summarize_meeting(
    store: &memory::Store,
    meeting_id: i64,
    notes: &str,
    transcript: &str,
    slides: &str,
    ollama_model: &str,
    routing: &RoutingConfig,
    custom_instructions: &str,
) -> ApiResult<MeetingSummaryData> {
    let custom_instructions =
        llm::profile::compose_instructions(custom_instructions.trim(), &routing.profile);
    let custom_instructions = custom_instructions.as_str();
    let cap_config = routing.spend_cap_config();
    if let Err(cap_err) = llm::check_spend_cap(
        store,
        &cap_config,
        None,
        Some("meeting_summary"),
        now_unix_ms(),
    ) {
        eprintln!("[summarize_meeting] rejected by spend cap before any provider was tried");
        return Err(ApiError::SpendCap(cap_err));
    }

    let chain = build_chain(ollama_model, routing);
    let mut result: Option<(llm::MeetingSummary, llm::Provider, String)> = None;
    let mut last_cap_error: Option<llm::SpendCapError> = None;
    for entry in &chain {
        let outcome = summarize_meeting_governed(
            store,
            &cap_config,
            entry.client.as_ref(),
            entry.provider,
            &entry.model,
            notes,
            transcript,
            slides,
            custom_instructions,
        );
        let err = match outcome {
            Ok(summary) => {
                result = Some((summary, entry.provider, entry.model.clone()));
                break;
            }
            Err(err) => err,
        };
        // Captured before `err` is consumed by `chain_loop_action_for`, purely so the
        // log lines below can still include the original error text.
        let llm_err_text = match &err {
            GovernedSummaryError::Llm(e) => Some(e.to_string()),
            GovernedSummaryError::SpendCap(_) => None,
        };
        match chain_loop_action_for(err) {
            ChainLoopAction::StopWithSpendCap(cap_err) => {
                eprintln!("[summarize_meeting] spend cap tripped mid-operation, aborting chain");
                last_cap_error = Some(cap_err);
                break;
            }
            ChainLoopAction::TryNextLink => {
                eprintln!(
                    "[summarize_meeting] {} ({}) failed retryably, trying next link: {}",
                    entry.provider.as_str(),
                    entry.model,
                    llm_err_text.unwrap_or_default()
                );
                continue;
            }
            ChainLoopAction::StopWithProviderFailure => {
                eprintln!(
                    "[summarize_meeting] {} ({}) failed non-retryably, aborting chain: {}",
                    entry.provider.as_str(),
                    entry.model,
                    llm_err_text.unwrap_or_default()
                );
                break;
            }
        }
    }
    let Some((summary, provider, model)) = result else {
        return Err(match last_cap_error {
            Some(cap_err) => ApiError::SpendCap(cap_err),
            None => ApiError::ProviderFailure,
        });
    };
    let _ = store.update_meeting_summary(
        meeting_id,
        &summary.tldr,
        &summary.decisions,
        &summary.action_items,
        &summary.open_questions,
        provider.as_str(),
        &model,
    );
    // #65: retitle a manually-started meeting (still the literal placeholder title
    // "Meeting") now that a real summary exists. No-op for a calendar-titled or
    // already-renamed meeting — see set_meeting_title_if_placeholder's doc comment.
    let derived_title = llm::meeting_title(&summary);
    let _ = store.set_meeting_title_if_placeholder(meeting_id, &derived_title);
    crate::title_check::check_calendar_title(store, meeting_id, routing, &summary, &derived_title);
    Ok(MeetingSummaryData {
        summary,
        provider,
        model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Succeeds for its first `fail_after` calls, then returns a retryable error.
    struct MockClientFailingAfter {
        fail_after: usize,
        calls: std::cell::Cell<usize>,
    }
    impl llm::LlmClient for MockClientFailingAfter {
        fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            let call_index = self.calls.get();
            self.calls.set(call_index + 1);
            if call_index >= self.fail_after {
                return Err(llm::LlmError::Http {
                    status: 429,
                    body: "rate limited".to_string(),
                });
            }
            Ok(llm::Completion {
                text: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
                token_usage: llm::TokenUsage {
                    input_tokens: Some(10),
                    output_tokens: Some(5),
                },
            })
        }
    }

    fn long_transcript() -> String {
        let line = "[   0.0s] Me: this is one line of transcript text that repeats many times.";
        let transcript = std::iter::repeat_n(line, 2000)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(transcript.len() > llm::max_transcript_chars_per_chunk());
        transcript
    }

    /// Mirrors `core/ffi`'s `summarize_meeting_governed_logs_usage_per_chunk_even_when_a_
    /// later_chunk_fails`.
    #[test]
    fn governed_logs_usage_per_chunk_even_when_a_later_chunk_fails() {
        let store = memory::Store::open_in_memory().unwrap();
        let cap_config = llm::SpendCapConfig {
            user_daily_cents: None,
            global_daily_cents: None,
        };
        let client = MockClientFailingAfter {
            fail_after: 2,
            calls: std::cell::Cell::new(0),
        };
        let result = summarize_meeting_governed(
            &store,
            &cap_config,
            &client,
            llm::Provider::Ollama,
            "test-model",
            "notes",
            &long_transcript(),
            "",
            "",
        );
        assert!(matches!(result, Err(GovernedSummaryError::Llm(_))));
        assert_eq!(store.count_usage_log_rows().unwrap(), 2);
    }

    #[test]
    fn spend_cap_always_stops_the_chain() {
        let cap_err = llm::SpendCapError {
            spent_cents: 1,
            cap_cents: 1,
            scope: llm::SpendCapScope::Global,
        };
        assert!(matches!(
            chain_loop_action_for(GovernedSummaryError::SpendCap(cap_err)),
            ChainLoopAction::StopWithSpendCap(_)
        ));
        let retryable = llm::LlmError::Http {
            status: 429,
            body: String::new(),
        };
        assert!(matches!(
            chain_loop_action_for(GovernedSummaryError::Llm(retryable)),
            ChainLoopAction::TryNextLink
        ));
    }

    #[test]
    fn meeting_summary_data_serializes_with_provider_and_model_attribution() {
        let summary = llm::MeetingSummary {
            title: String::new(),
            tldr: "We agreed.".into(),
            decisions: vec!["ship".into()],
            action_items: vec![],
            open_questions: vec![],
            token_usage: llm::TokenUsage::default(),
        };
        let json =
            summary_json_with_attribution(&summary, llm::Provider::Ollama, "gemma4:12b").unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["tldr"], "We agreed.");
        assert_eq!(value["provider"], "ollama");
        assert_eq!(value["model"], "gemma4:12b");
        assert_eq!(value["decisions"][0], "ship");
        // The typed struct and the string builder are the same bytes.
        let data = MeetingSummaryData {
            summary,
            provider: llm::Provider::Ollama,
            model: "gemma4:12b".into(),
        };
        assert_eq!(serde_json::to_string(&data).unwrap(), json);
    }

    /// Mirrors `summarize_meeting_reports_spend_cap_envelope_before_any_provider_is_tried`:
    /// the meeting row is untouched on a cap rejection.
    #[test]
    fn summarize_meeting_reports_spend_cap_and_leaves_the_meeting_row_untouched() {
        let store = memory::Store::open_in_memory().unwrap();
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
                ts_unix_ms: now_unix_ms(),
            })
            .unwrap();
        let routing = RoutingConfig {
            global_daily_cap_cents: Some(1),
            ..RoutingConfig::default()
        };
        let result = summarize_meeting(
            &store,
            1,
            "notes",
            "transcript",
            "",
            "gemma4:12b",
            &routing,
            "  ",
        );
        assert!(matches!(result, Err(ApiError::SpendCap(_))));
        assert_eq!(store.count_usage_log_rows().unwrap(), 1);
    }
}
