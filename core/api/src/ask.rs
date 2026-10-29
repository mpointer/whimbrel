//! FR-CHAT: grounded chat — the blocking `answer`, the streaming `prepare_ask` /
//! `finalize_ask` / `discard_ask` split (#74), and conversation titles (FR-CHAT-6).
//! Moved from `core/ffi`'s `whimbrel_llm_answer` family (M3.0); the C functions are thin
//! wrappers. Argument decoding (`history_json`, the `-1` token sentinels) stays in `ffi`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

use crate::envelope::{ApiError, ApiResult};
use crate::routing::{build_chain_for, RoutingConfig};
use crate::util::{log_usage_for, now_unix_ms};

/// Retrieval (keyword, falling back to semantic) using `query` alone, then the spend-cap
/// check, then the provider chain, then usage logging. `history` is fed to the model for
/// conversational continuity but does not affect retrieval. Returns [`llm::ChatAnswer`]
/// unchanged — its serialized shape (`text`, `citations`, `token_usage`) is exactly what
/// the C boundary already emits.
pub fn answer(
    store: &memory::Store,
    query: &str,
    ollama_model: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<llm::ChatAnswer> {
    let (context, date_scope_label) = assemble_ask_context(store, query, context_limit);

    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("ask"), now_unix_ms())?;

    let chain = build_chain_for(llm::TaskKind::Chat, ollama_model, routing);
    let Some((answer, entry)) = llm::try_chain(&chain, |entry| {
        llm::answer_with_citations(
            entry.client.as_ref(),
            history,
            query,
            &context,
            &llm::profile::compose_instructions(&routing.custom_instructions, &routing.profile),
            date_scope_label.as_deref(),
        )
    }) else {
        return Err(ApiError::ProviderFailure);
    };
    log_usage_for(
        store,
        entry.provider,
        &entry.model,
        "ask",
        answer.token_usage,
    );
    Ok(answer)
}

/// Shared phase-A logic for both the blocking [`answer`] and the streaming
/// [`prepare_ask`]: date-scoped ("today"/"yesterday") or generic context retrieval.
fn assemble_ask_context(
    store: &memory::Store,
    query: &str,
    context_limit: usize,
) -> (Vec<llm::ContextItem>, Option<String>) {
    match llm::detect_date_scope(query) {
        Some(scope) => {
            let (start, end) = llm::date_scope_bounds(scope, now_unix_ms());
            let items = llm::gather_and_assemble_context_for_window(
                store,
                "",
                now_unix_ms(),
                start,
                end,
                context_limit,
                true,
            );
            (items, Some(llm::format_date_scope_label(scope, start)))
        }
        None => (
            llm::gather_and_assemble_context(store, query, now_unix_ms(), context_limit, true),
            None,
        ),
    }
}

/// #74: `Vec<llm::ContextItem>` isn't `Serialize` (deliberately — see its doc comment),
/// so it can't cross a shell boundary as JSON and back. [`prepare_ask`] stashes it here,
/// keyed by a one-shot `prepare_id`, so [`finalize_ask`] can extract citations against
/// the SAME context after the streamed answer completes. Entries are removed by
/// `finalize_ask`/`discard_ask`; an app relaunch mid-stream drops any never-finalized
/// entry along with the process, which is fine — there's nothing durable to leak.
struct PreparedAsk {
    context: Vec<llm::ContextItem>,
    /// The provider the streamed answer is attributed to for usage/cost logging (#83):
    /// the routing config's preferred provider, the same one the shell then streams
    /// against — forced to Ollama in local-only mode, mirroring `build_chain`.
    provider: llm::Provider,
}

static PREPARED_ASKS: OnceLock<Mutex<HashMap<String, PreparedAsk>>> = OnceLock::new();

fn prepared_asks() -> &'static Mutex<HashMap<String, PreparedAsk>> {
    PREPARED_ASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn provider_for_streamed_answer(routing: &RoutingConfig) -> llm::Provider {
    if routing.sensitive {
        return llm::Provider::Ollama;
    }
    match routing.preferred_provider.as_str() {
        "anthropic" => llm::Provider::Anthropic,
        "openai" => llm::Provider::OpenAI,
        "google" => llm::Provider::Google,
        "openrouter" => llm::Provider::OpenRouter,
        _ => llm::Provider::Ollama,
    }
}

/// What [`prepare_ask`] hands back: the one-shot id to pass to [`finalize_ask`] (or
/// [`discard_ask`]) exactly once, and the exact prompt `answer_with_citations` would
/// have sent, for the caller to stream against. Field order is the wire order.
#[derive(Debug, Clone, Serialize)]
pub struct PreparedAskData {
    pub prepare_id: String,
    pub prompt: String,
}

/// #74: the assembly half of [`answer`], for a caller that wants to stream the actual
/// model call (via `inference`) instead of blocking on it. Runs retrieval + the spend-cap
/// check (both DB-bound), builds the prompt, and stashes the assembled context for
/// [`finalize_ask`] to extract citations from later.
pub fn prepare_ask(
    store: &memory::Store,
    query: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<PreparedAskData> {
    let (context, date_scope_label) = assemble_ask_context(store, query, context_limit);

    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("ask"), now_unix_ms())?;

    let prompt = llm::build_ask_prompt(
        history,
        query,
        &context,
        &llm::profile::compose_instructions(&routing.custom_instructions, &routing.profile),
        date_scope_label.as_deref(),
    );
    let prepare_id = llm::streaming::new_run_id();
    prepared_asks().lock().unwrap().insert(
        prepare_id.clone(),
        PreparedAsk {
            context,
            provider: provider_for_streamed_answer(routing),
        },
    );
    Ok(PreparedAskData { prepare_id, prompt })
}

/// What [`prepare_meeting_ask`] hands back: [`PreparedAskData`] plus whether the
/// transcript had to be narrowed to fit the model's context.
#[derive(Debug, Clone, Serialize)]
pub struct PreparedMeetingAskData {
    pub prepare_id: String,
    pub prompt: String,
    pub narrowed: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum MeetingAskError {
    /// The meeting does not exist or has no usable transcript: no prompt is built.
    NoTranscript,
    Api(ApiError),
}

impl From<ApiError> for MeetingAskError {
    fn from(e: ApiError) -> Self {
        MeetingAskError::Api(e)
    }
}

/// The meeting-scoped counterpart of [`prepare_ask`]: the citable context is this one
/// meeting's transcript (redacted, narrowed to the budget), with its summary and the
/// user's notes as uncitable background. The stash, [`finalize_ask`] and the streaming
/// path are the same as for `prepare_ask`.
pub fn prepare_meeting_ask(
    store: &memory::Store,
    meeting_id: i64,
    query: &str,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> Result<PreparedMeetingAskData, MeetingAskError> {
    let meeting = store
        .get_meeting(meeting_id)
        .ok()
        .flatten()
        .ok_or(MeetingAskError::NoTranscript)?;
    // Real rows when the meeting has them, otherwise the provisional live rows (negative ids).
    let segments = store
        .transcript_for_meeting(meeting_id)
        .map_err(|_| MeetingAskError::NoTranscript)?;
    if segments.is_empty() {
        return Err(MeetingAskError::NoTranscript);
    }

    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("ask"), now_unix_ms()).map_err(ApiError::from)?;

    let context = llm::meeting_ask::assemble_meeting_context(segments, query, llm::meeting_ask::CONTEXT_BUDGET_CHARS);
    // Every segment larger than the budget: nothing citable fits, same as no transcript.
    if context.items.is_empty() {
        return Err(MeetingAskError::NoTranscript);
    }
    let draft = store.get_meeting_note_draft(meeting_id).ok().flatten();
    let summary = draft
        .as_ref()
        .and_then(|d| d.summary_edited.clone())
        .or_else(|| meeting.summary_tldr.clone())
        .unwrap_or_default();
    let notes = draft
        .as_ref()
        .and_then(|d| d.personal_notes.clone())
        .or_else(|| meeting.notes.clone())
        .unwrap_or_default();
    let prompt = llm::meeting_ask::build_meeting_ask_prompt(
        history,
        query,
        &context.items,
        &llm::profile::compose_instructions(&routing.custom_instructions, &routing.profile),
        &meeting.title,
        &summary,
        &notes,
    );
    let prepare_id = llm::streaming::new_run_id();
    prepared_asks().lock().unwrap().insert(
        prepare_id.clone(),
        PreparedAsk { context: context.items, provider: provider_for_streamed_answer(routing) },
    );
    Ok(PreparedMeetingAskData { prepare_id, prompt, narrowed: context.narrowed })
}

/// [`finalize_ask`]'s result: a plain object, not the `{"ok":…}` envelope — there is no
/// failure mode worth distinguishing from the caller's perspective.
#[derive(Debug, Clone, Serialize)]
pub struct FinalizedAsk {
    pub citations: Vec<llm::ContextItemRef>,
}

/// #74: the citation-extraction half, run once a streaming answer started via
/// [`prepare_ask`] reaches its `Completed` terminal event. Looks up (and removes —
/// one-shot) the context stashed by `prepare_id`, matches `[n]`-style citation markers in
/// `final_text` against it exactly as `answer_with_citations` does for the blocking path,
/// and logs usage/cost attributed to the provider the prepare call's routing named.
/// An unknown/already-finalized `prepare_id` yields empty citations rather than an error:
/// a caller that lost track of its own id still wants a well-formed answer to render.
pub fn finalize_ask(
    store: &memory::Store,
    prepare_id: &str,
    final_text: &str,
    model: &str,
    usage: llm::TokenUsage,
) -> FinalizedAsk {
    // The guard is dropped at the end of this statement — nothing below runs while the
    // map is locked (ADR 0001).
    let prepared = prepared_asks().lock().unwrap().remove(prepare_id);
    let citations = match &prepared {
        Some(prepared) => llm::extract_citations(final_text, &prepared.context),
        None => Vec::new(),
    };
    if let Some(prepared) = prepared {
        log_usage_for(store, prepared.provider, model, "ask", usage);
    }
    FinalizedAsk { citations }
}

/// #74: discards a prepared ask's stashed context without extracting citations — call
/// this on a cancelled or failed-with-no-output streaming run, since [`finalize_ask`]
/// (the only other remover) never runs on those paths. A no-op for an unknown id.
pub fn discard_ask(prepare_id: &str) {
    prepared_asks().lock().unwrap().remove(prepare_id);
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationTitle {
    pub title: String,
}

/// FR-CHAT-6: a short title for a newly-created conversation's first exchange. Same
/// spend-cap-check + provider-chain + usage-logging shape as every other governed call.
/// Callers treat ANY error as "leave the placeholder" and never retry.
pub fn generate_conversation_title(
    store: &memory::Store,
    ollama_model: &str,
    query: &str,
    answer: &str,
    routing: &RoutingConfig,
) -> ApiResult<ConversationTitle> {
    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(
        store,
        &cap_config,
        None,
        Some("conversation_title"),
        now_unix_ms(),
    )?;

    let chain = build_chain_for(llm::TaskKind::ConversationTitle, ollama_model, routing);
    let Some(((title, token_usage), entry)) = llm::try_chain(&chain, |entry| {
        llm::generate_conversation_title(entry.client.as_ref(), query, answer)
    }) else {
        return Err(ApiError::ProviderFailure);
    };
    log_usage_for(
        store,
        entry.provider,
        &entry.model,
        "conversation_title",
        token_usage,
    );
    Ok(ConversationTitle { title })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_capture_text(text: &str) -> memory::Store {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: now_unix_ms(),
                app_bundle_id: "com.apple.Notes".to_string(),
                window_title: "Notes".to_string(),
                url: None,
                text: text.to_string(),
                hash: 1,
                source: "ax".to_string(),
            })
            .unwrap();
        store
    }

    fn meeting_with_transcript(store: &memory::Store, lines: &[&str]) -> i64 {
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Acme sync".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        let segments: Vec<memory::NewTranscriptSegment> = lines
            .iter()
            .enumerate()
            .map(|(i, text)| memory::NewTranscriptSegment {
                meeting_id: id,
                start_cs: i as i64 * 100,
                speaker_label: "Ben".into(),
                text: (*text).into(),
            })
            .collect();
        store.insert_transcript_segments(&segments).unwrap();
        id
    }

    #[test]
    fn a_meeting_ask_over_live_rows_cites_negative_ids() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: None,
                title: "Acme sync".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_live_segments(&[memory::live::NewLiveSegment {
                meeting_id: id,
                channel: "mic".into(),
                chunk_index: 0,
                start_cs: 0,
                end_cs: 100,
                speaker_label: "Me".into(),
                text: "we agreed on Friday".into(),
            }])
            .unwrap();

        let data = prepare_meeting_ask(&store, id, "when is it due?", &[], &RoutingConfig::default()).unwrap();
        assert!(data.prompt.contains("we agreed on Friday"), "{}", data.prompt);
        let finalized = finalize_ask(&store, &data.prepare_id, "Friday [1].", "m", llm::TokenUsage::default());
        assert_eq!(finalized.citations.len(), 1);
        match finalized.citations[0] {
            llm::ContextItemRef::Transcript { id } => assert!(id < 0, "live citations carry negative ids"),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn cfo_routing(profile: &str) -> RoutingConfig {
        RoutingConfig { profile: profile.into(), custom_instructions: "be terse".into(), ..RoutingConfig::default() }
    }

    #[test]
    fn profile_reaches_the_timeline_ask_prompt_and_is_absent_when_blank() {
        let _serial = serialize_prepared_asks();
        let store = store_with_capture_text("the quarterly roadmap review");
        let with = prepare_ask(&store, "roadmap", 5, &[], &cfo_routing("Role: CFO")).unwrap();
        assert!(with.prompt.contains("About the user") && with.prompt.contains("Role: CFO"), "{}", with.prompt);
        assert!(with.prompt.contains("be terse"), "custom instructions survive");
        let without = prepare_ask(&store, "roadmap", 5, &[], &cfo_routing("")).unwrap();
        assert!(!without.prompt.contains("About the user"));
    }

    #[test]
    fn profile_reaches_the_meeting_ask_prompt_and_is_absent_when_blank() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting_with_transcript(&store, &["we agreed on Friday"]);
        let with = prepare_meeting_ask(&store, id, "when?", &[], &cfo_routing("Role: CFO")).unwrap();
        assert!(with.prompt.contains("About the user") && with.prompt.contains("Role: CFO"), "{}", with.prompt);
        let without = prepare_meeting_ask(&store, id, "when?", &[], &cfo_routing("")).unwrap();
        assert!(!without.prompt.contains("About the user"));
    }

    #[test]
    fn a_meeting_ask_builds_a_prompt_from_its_transcript_and_stashes_it_for_citations() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting_with_transcript(&store, &["we agreed on Friday", "Ben sends the deck"]);

        let data = prepare_meeting_ask(&store, id, "when is it due?", &[], &RoutingConfig::default()).unwrap();

        assert!(data.prompt.contains("[1] (meeting \"Acme sync\", Ben) we agreed on Friday"), "{}", data.prompt);
        assert!(!data.narrowed);
        let finalized = finalize_ask(&store, &data.prepare_id, "Friday [1] and Ben sends it [2]. Also [9].", "m", llm::TokenUsage::default());
        assert_eq!(finalized.citations.len(), 2, "[9] is out of range and ignored");
        assert!(finalized.citations.iter().all(|c| matches!(c, llm::ContextItemRef::Transcript { .. })));
    }

    #[test]
    fn a_meeting_prompt_redacts_secrets_in_the_transcript_summary_and_notes() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let anthropic = "sk-ant-api03-abcdefghijklmnopqrstuv1234";
        let github = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123";
        let aws = "AKIAIOSFODNN7EXAMPLE";
        let id = meeting_with_transcript(&store, &[&format!("my key is {anthropic}")]);
        store.save_meeting_note_overrides(id, Some(&format!("summary {github}")), None, None, 1).unwrap();
        store.save_meeting_personal_notes(id, &format!("notes {aws}"), 2).unwrap();

        let data = prepare_meeting_ask(&store, id, "what is the key?", &[], &RoutingConfig::default()).unwrap();

        for secret in [anthropic, github, aws] {
            assert!(!data.prompt.contains(secret), "{secret} leaked: {}", data.prompt);
        }
        assert!(data.prompt.contains("[REDACTED"), "{}", data.prompt);
    }

    #[test]
    fn a_meeting_with_no_transcript_never_produces_a_prompt() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting_with_transcript(&store, &[]);
        let before = prepared_asks().lock().unwrap().len();
        assert!(matches!(prepare_meeting_ask(&store, id, "q", &[], &RoutingConfig::default()), Err(MeetingAskError::NoTranscript)));
        assert!(matches!(prepare_meeting_ask(&store, 9_999, "q", &[], &RoutingConfig::default()), Err(MeetingAskError::NoTranscript)));
        assert_eq!(prepared_asks().lock().unwrap().len(), before);
    }

    #[test]
    fn a_transcript_whose_every_segment_exceeds_the_budget_is_treated_as_no_transcript() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let huge = "x".repeat(llm::meeting_ask::CONTEXT_BUDGET_CHARS + 1);
        let id = meeting_with_transcript(&store, &[huge.as_str()]);
        let before = prepared_asks().lock().unwrap().len();
        assert!(matches!(prepare_meeting_ask(&store, id, "q", &[], &RoutingConfig::default()), Err(MeetingAskError::NoTranscript)));
        assert_eq!(prepared_asks().lock().unwrap().len(), before);
    }

    #[test]
    fn a_meeting_ask_obeys_the_spend_cap_and_stashes_nothing() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting_with_transcript(&store, &["hello"]);
        let routing = tripped_cap_routing(&store);
        let before = prepared_asks().lock().unwrap().len();
        let result = prepare_meeting_ask(&store, id, "q", &[], &routing);
        assert!(matches!(result, Err(MeetingAskError::Api(ApiError::SpendCap(_)))));
        assert_eq!(prepared_asks().lock().unwrap().len(), before);
    }

    fn tripped_cap_routing(store: &memory::Store) -> RoutingConfig {
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
        RoutingConfig {
            global_daily_cap_cents: Some(1),
            ..RoutingConfig::default()
        }
    }

    /// Mirrors `core/ffi`'s `llm_answer_returns_spend_cap_envelope_when_cap_is_already_
    /// tripped`: the cap check runs before any provider client is touched, so no live
    /// Ollama server is needed for this to be deterministic.
    #[test]
    fn answer_reports_spend_cap_before_any_provider_is_tried() {
        let store = store_with_capture_text("the quarterly roadmap review");
        let routing = tripped_cap_routing(&store);
        let result = answer(&store, "roadmap", "gemma4:12b", 5, &[], &routing);
        assert!(matches!(result, Err(ApiError::SpendCap(_))));
    }

    /// `PREPARED_ASKS` is process-global and tests run in parallel, so the tests that
    /// read or write it take this lock. Poison-tolerant, so one failure doesn't fail the rest.
    fn serialize_prepared_asks() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn prepare_ask_reports_spend_cap_without_stashing_anything() {
        let _serial = serialize_prepared_asks();
        let store = store_with_capture_text("the quarterly roadmap review");
        let routing = tripped_cap_routing(&store);
        let before = prepared_asks().lock().unwrap().len();
        let result = prepare_ask(&store, "roadmap", 5, &[], &routing);
        assert!(matches!(result, Err(ApiError::SpendCap(_))));
        assert_eq!(prepared_asks().lock().unwrap().len(), before);
    }

    /// Mirrors `finalize_ask_extracts_citations_matching_the_prepared_context` and
    /// `finalize_ask_is_one_shot_…` from the `ffi` suite at the typed layer.
    #[test]
    fn prepare_then_finalize_round_trips_citations_and_is_one_shot() {
        let _serial = serialize_prepared_asks();
        let store = store_with_capture_text("the quarterly roadmap review");
        let routing = RoutingConfig::default();
        let prepared = prepare_ask(&store, "roadmap", 5, &[], &routing).unwrap();
        assert!(prepared.prompt.contains("roadmap"));
        assert!(!prepared.prepare_id.is_empty());

        let first = finalize_ask(
            &store,
            &prepared.prepare_id,
            "It was about the roadmap [1].",
            "gemma4:12b",
            llm::TokenUsage::default(),
        );
        assert_eq!(first.citations.len(), 1);
        assert_eq!(
            serde_json::to_string(&first)
                .unwrap()
                .starts_with(r#"{"citations":[{"#),
            true
        );

        let second = finalize_ask(
            &store,
            &prepared.prepare_id,
            "It was about the roadmap [1].",
            "gemma4:12b",
            llm::TokenUsage::default(),
        );
        assert!(second.citations.is_empty());
    }

    #[test]
    fn discard_ask_removes_stashed_context_and_unknown_id_is_a_no_op() {
        let _serial = serialize_prepared_asks();
        let store = store_with_capture_text("the quarterly roadmap review");
        let prepared = prepare_ask(&store, "roadmap", 5, &[], &RoutingConfig::default()).unwrap();
        discard_ask(&prepared.prepare_id);
        discard_ask("never-existed");
        let finalized = finalize_ask(
            &store,
            &prepared.prepare_id,
            "[1]",
            "gemma4:12b",
            llm::TokenUsage::default(),
        );
        assert!(finalized.citations.is_empty());
    }

    #[test]
    fn prepared_ask_data_serializes_prepare_id_before_prompt() {
        let data = PreparedAskData {
            prepare_id: "id".into(),
            prompt: "p".into(),
        };
        assert_eq!(
            serde_json::to_string(&data).unwrap(),
            r#"{"prepare_id":"id","prompt":"p"}"#
        );
    }

    #[test]
    fn streamed_answer_provider_follows_preference_unless_sensitive() {
        let mut routing = RoutingConfig {
            preferred_provider: "google".into(),
            ..RoutingConfig::default()
        };
        assert_eq!(
            provider_for_streamed_answer(&routing),
            llm::Provider::Google
        );
        for (name, provider) in [("anthropic", llm::Provider::Anthropic), ("openai", llm::Provider::OpenAI)] {
            routing.preferred_provider = name.into();
            assert_eq!(provider_for_streamed_answer(&routing), provider, "{name} spend is logged to {name}");
        }
        routing.sensitive = true;
        assert_eq!(
            provider_for_streamed_answer(&routing),
            llm::Provider::Ollama
        );
    }

    #[test]
    fn conversation_title_reports_spend_cap_before_any_provider_is_tried() {
        let store = memory::Store::open_in_memory().unwrap();
        let routing = tripped_cap_routing(&store);
        let result = generate_conversation_title(&store, "gemma4:12b", "q", "a", &routing);
        assert!(matches!(result, Err(ApiError::SpendCap(_))));
    }
}
