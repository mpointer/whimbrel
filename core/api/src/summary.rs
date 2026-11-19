//! FR-MTG-5: the governed meeting summary — the one governed call whose spend-cap check
//! can trip *mid-operation* (per chunk of a long transcript), not just before the first
//! provider call. Moved from `core/ffi::whimbrel_llm_summarize_meeting` and its helpers
//! (M3.0); see docs/superpowers/specs/2026-07-27-meeting-summary-spend-cap-governance-design.md.

use serde::{Serialize, Serializer};

use crate::envelope::{ApiError, ApiResult};
use crate::routing::{build_chain_for, RoutingConfig};
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

/// The model that answered on `client`: its stand-in when the provider had no `model`.
fn answered(client: &dyn llm::LlmClient, model: &str) -> String {
    client.substituted_model().unwrap_or_else(|| model.to_string())
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
    // Sized from the model's context window: one call when the transcript fits it.
    let max_chars = llm::summary_chunk_chars(provider);
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
        log_usage_for(store, provider, &answered(client, model), "meeting_summary", summary.token_usage);
        crate::model_defaults::note_answer(store, provider, model, client.substituted_model().as_deref(), now_unix_ms());
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
        // Each part after the first is told how the one before it ended.
        let previous = partials.last().map(|p: &llm::MeetingSummary| p.tldr.as_str());
        let (summary, usage) = llm::summarize_chunk_with_context(
            client,
            i,
            chunks.len(),
            notes,
            chunk,
            custom_instructions,
            previous,
        )
        .map_err(GovernedSummaryError::Llm)?;
        log_usage_for(store, provider, &answered(client, model), "meeting_summary", usage);
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
    log_usage_for(store, provider, &answered(client, model), "meeting_summary", reduce_usage);
    crate::model_defaults::note_answer(store, provider, model, client.substituted_model().as_deref(), now_unix_ms());
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

/// Reason codes of a [`SummaryFallback`]: content-free, stable strings a shell keys its
/// wording on. `unauthorized`, `not_signed_in` and `no_device_key` are fixed by signing in.
pub mod fallback_reason {
    /// No Google ID token: the user is not signed in.
    pub const NOT_SIGNED_IN: &str = "not_signed_in";
    /// Signed in, but this device has no key registered.
    pub const NO_DEVICE_KEY: &str = "no_device_key";
    /// Whimbrel AI's monthly budget for this device is spent, so its link was dropped.
    pub const BUDGET: &str = "budget";
    /// The link may not receive Google data (Limited Use).
    pub const GOOGLE_DATA_WITHHELD: &str = "google_data_withheld";
    /// Local-only mode: no cloud model is used at all (forced re-summaries only).
    pub const LOCAL_ONLY: &str = "local_only";
    /// The call timed out (a network timeout or HTTP 408).
    pub const TIMEOUT: &str = "timeout";
    /// The server could not be reached.
    pub const NETWORK: &str = "network";
    /// HTTP 401 or 403: the sign-in was refused (for example it expired).
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// HTTP 402: the prepaid credit ran out.
    pub const CREDIT_EXHAUSTED: &str = "credit_exhausted";
    /// HTTP 429.
    pub const RATE_LIMITED: &str = "rate_limited";
    /// The model is not offered to this account.
    pub const MODEL_UNAVAILABLE: &str = "model_unavailable";
    /// Any other failure; `http_status` says which HTTP class when there was one.
    pub const UNAVAILABLE: &str = "unavailable";
    /// A forced call whose preferred provider is not set up (other than Whimbrel's sign-in).
    pub const NOT_CONFIGURED: &str = "not_configured";
}

/// Why a summary meant for `preferred` was written by `used`. Content-free: provider ids,
/// the model that answered, a [`fallback_reason`] code, and the HTTP status when the
/// failing call had one. Never an error text, a prompt or a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SummaryFallback {
    pub preferred: String,
    pub used: String,
    pub model: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

impl From<memory::summary_fallback::SummaryFallbackRow> for SummaryFallback {
    fn from(r: memory::summary_fallback::SummaryFallbackRow) -> Self {
        SummaryFallback { preferred: r.preferred, used: r.used, model: r.model, reason: r.reason, http_status: r.http_status }
    }
}

impl From<&SummaryFallback> for memory::summary_fallback::SummaryFallbackRow {
    fn from(f: &SummaryFallback) -> Self {
        memory::summary_fallback::SummaryFallbackRow {
            preferred: f.preferred.clone(),
            used: f.used.clone(),
            model: f.model.clone(),
            reason: f.reason.clone(),
            http_status: f.http_status,
        }
    }
}

/// The fallback behind `meeting_id`'s stored summary; `None` when there is none, or the
/// summary has since been replaced by another provider's.
pub fn stored_summary_fallback(store: &memory::Store, meeting_id: i64) -> Option<SummaryFallback> {
    store.summary_fallback(meeting_id).ok().flatten().map(SummaryFallback::from)
}

/// A reason code (and HTTP status, when there was one) for one failed call. Pure.
pub fn reason_for_error(provider: llm::Provider, err: &llm::LlmError) -> (&'static str, Option<u16>) {
    use fallback_reason as r;
    match err {
        llm::LlmError::Network(msg) if msg.to_lowercase().contains("timed out") => (r::TIMEOUT, None),
        llm::LlmError::Network(_) => (r::NETWORK, None),
        llm::LlmError::Http { status: 408, .. } => (r::TIMEOUT, Some(408)),
        llm::LlmError::Http { status: 402, .. } => (r::CREDIT_EXHAUSTED, Some(402)),
        _ if llm::is_model_not_found(provider, err) => (r::MODEL_UNAVAILABLE, None),
        llm::LlmError::Http { status: status @ (401 | 403), .. } => (r::UNAUTHORIZED, Some(*status)),
        llm::LlmError::Http { status: 429, .. } => (r::RATE_LIMITED, Some(429)),
        llm::LlmError::Http { status, .. } => (r::UNAVAILABLE, Some(*status)),
        llm::LlmError::Provider(_) | llm::LlmError::NotImplemented(_) => (r::UNAVAILABLE, None),
        // The exclusion gate refused the call: content-free, and no client is named.
        llm::LlmError::Refused(_) => (r::UNAVAILABLE, None),
    }
}

/// Why Whimbrel AI has no link although it was picked, from the routing alone: a missing
/// token or device key, else (both there) its budget dropped it.
fn missing_link_reason(routing: &RoutingConfig) -> &'static str {
    if routing.whimbrel_id_token.is_none() {
        fallback_reason::NOT_SIGNED_IN
    } else if routing.whimbrel_device_key.is_none() {
        fallback_reason::NO_DEVICE_KEY
    } else {
        fallback_reason::BUDGET
    }
}

type ReasonCode = (&'static str, Option<u16>);

thread_local! {
    static LAST_FAILURE_REASON: std::cell::RefCell<Option<ReasonCode>> = const { std::cell::RefCell::new(None) };
}

/// Why this thread's latest failed summary could not use the preferred provider (a
/// [`fallback_reason`] code and the HTTP status, if any), and forgets it. The C layer adds
/// it to a forced call's error envelope.
pub fn take_last_summary_failure_reason() -> Option<ReasonCode> {
    LAST_FAILURE_REASON.with(|r| r.borrow_mut().take())
}

fn set_last_failure_reason(reason: Option<ReasonCode>) {
    LAST_FAILURE_REASON.with(|r| *r.borrow_mut() = reason);
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
    /// Set (and serialized as `fallback`) only when the summary was meant for Whimbrel AI
    /// and another provider wrote it; absent otherwise, so every other payload is unchanged.
    pub fallback: Option<SummaryFallback>,
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
            if let Some(fallback) = &self.fallback {
                obj.insert("fallback".to_string(), serde_json::to_value(fallback)?);
            }
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
        fallback: None,
    })
    .ok()
}

/// Blends `notes` with `transcript` into a structured summary through the full
/// governance pipeline (spend-cap check, provider chain with failover, per-call usage
/// logging), then writes the summary onto the `meeting` row `meeting_id` and retitles a
/// placeholder-titled meeting from the TL;DR (#65). On any failure the meeting row's
/// summary fields are left untouched, and blank notes, transcript and slides fail without a
/// provider call. `custom_instructions` is trimmed; whitespace-only
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
    summarize_meeting_inner(store, meeting_id, notes, transcript, slides, ollama_model, routing, custom_instructions, false)
}

/// [`summarize_meeting`] for "Re-summarize with Whimbrel": the chain is only the preferred
/// provider's link, with no fallback and no other link. When that link is not there (not
/// signed in, no device key, local-only mode) or fails, the call fails as
/// [`ApiError::ProviderFailure`] (or [`ApiError::BudgetSpent`] when its budget is spent),
/// and [`take_last_summary_failure_reason`] says why. The Limited Use rules are the same
/// as for the normal call: a link that may not receive Google data is skipped, so this
/// never sends a meeting somewhere the chain would not. On success the stored fallback
/// notice is cleared, since the preferred provider wrote the summary.
#[allow(clippy::too_many_arguments)]
pub fn summarize_meeting_preferred(
    store: &memory::Store,
    meeting_id: i64,
    notes: &str,
    transcript: &str,
    slides: &str,
    ollama_model: &str,
    routing: &RoutingConfig,
    custom_instructions: &str,
) -> ApiResult<MeetingSummaryData> {
    summarize_meeting_inner(store, meeting_id, notes, transcript, slides, ollama_model, routing, custom_instructions, true)
}

/// The chain for a forced call: the first link, when it is the preferred provider's.
fn forced_chain(chain: Vec<llm::ChainLink>, routing: &RoutingConfig) -> ApiResult<Vec<llm::ChainLink>> {
    let wanted = llm::Provider::parse(&routing.preferred_provider).unwrap_or(llm::Provider::Ollama);
    let reason = if routing.sensitive && wanted != llm::Provider::Ollama && wanted != llm::Provider::Local {
        fallback_reason::LOCAL_ONLY
    } else if wanted == llm::Provider::Whimbrel {
        missing_link_reason(routing)
    } else {
        fallback_reason::NOT_CONFIGURED
    };
    match chain.into_iter().next() {
        Some(first) if first.provider == wanted => Ok(vec![first]),
        _ => {
            eprintln!("[summarize_meeting] forced {} re-summary refused: {reason}", wanted.as_str());
            set_last_failure_reason(Some((reason, None)));
            Err(crate::chain_failed(vec![llm::ProviderFailureKind::NoProviderConfigured]))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn summarize_meeting_inner(
    store: &memory::Store,
    meeting_id: i64,
    notes: &str,
    transcript: &str,
    slides: &str,
    ollama_model: &str,
    routing: &RoutingConfig,
    custom_instructions: &str,
    forced: bool,
) -> ApiResult<MeetingSummaryData> {
    set_last_failure_reason(None);
    // No words, notes or slides: there is nothing to summarize, and a model asked anyway
    // writes one from the instructions alone (the profile included). Fails like a provider
    // failure, so the row is untouched and callers already handle it.
    if [notes, transcript, slides].iter().all(|s| s.trim().is_empty()) {
        return Err(ApiError::ProviderFailure);
    }
    let custom_instructions =
        llm::profile::compose_instructions(custom_instructions.trim(), &routing.profile, &routing.output_language);
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

    // Classify first (spec 7.1): decided once for the whole meeting, before any hosted call. A
    // meeting filed under an excluded client, or whose title, attendees, notes, transcript or
    // slides name one or carry a client contact's address, is summarised on this computer or
    // not at all. Deciding per chunk instead would send the earlier chunks to every hosted link
    // in turn before a later chunk was refused. A forced re-summary ("Re-summarize with
    // Whimbrel") is decided the same way first, then pinned to the preferred link.
    summarize_meeting_routed_as(
        store,
        meeting_id,
        notes,
        transcript,
        slides,
        routing,
        custom_instructions,
        forced,
        || build_chain_for(llm::TaskKind::MeetingSummary, ollama_model, routing, &crate::exclusion::current_gate(store)),
        || crate::routing::build_local_chain(ollama_model, routing),
    )
}

/// [`summarize_meeting`] with the chains injected: `local_chain` for an excluded client's
/// meeting, `chain` for any other.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn summarize_meeting_routed(
    store: &memory::Store,
    meeting_id: i64,
    notes: &str,
    transcript: &str,
    slides: &str,
    routing: &RoutingConfig,
    custom_instructions: &str,
    chain: impl FnOnce() -> Vec<llm::ChainLink>,
    local_chain: impl FnOnce() -> Vec<llm::ChainLink>,
) -> ApiResult<MeetingSummaryData> {
    summarize_meeting_routed_as(store, meeting_id, notes, transcript, slides, routing, custom_instructions, false, chain, local_chain)
}

/// [`summarize_meeting_routed`]; `forced`: a "Re-summarize with Whimbrel" call, pinned to the
/// preferred provider's link ([`forced_chain`]) after the exclusion decision. An excluded
/// client's meeting is never forced onto a hosted link: when the preferred provider is not
/// on this computer the call is refused as an exclusion refusal and nothing is sent.
#[allow(clippy::too_many_arguments)]
pub(crate) fn summarize_meeting_routed_as(
    store: &memory::Store,
    meeting_id: i64,
    notes: &str,
    transcript: &str,
    slides: &str,
    routing: &RoutingConfig,
    custom_instructions: &str,
    forced: bool,
    chain: impl FnOnce() -> Vec<llm::ChainLink>,
    local_chain: impl FnOnce() -> Vec<llm::ChainLink>,
) -> ApiResult<MeetingSummaryData> {
    let local_only = meeting_is_excluded(store, meeting_id, &[notes, transcript, slides]);
    let mut chain = if local_only { local_chain() } else { chain() };
    if forced {
        chain = match forced_chain(chain, routing) {
            Ok(chain) => chain,
            Err(_) if local_only => {
                // The reason is the exclusion, not a sign-in or set-up problem.
                set_last_failure_reason(None);
                return Err(crate::exclusion::refused());
            }
            Err(e) => return Err(e),
        };
    }
    summarize_meeting_pinned(store, meeting_id, notes, transcript, slides, routing, custom_instructions, chain, local_only, forced)
}

/// Whether meeting `meeting_id` with this material belongs to an excluded client (or the setting
/// cannot be read): its membership, title and attendees, and every word of `material`.
pub(crate) fn meeting_is_excluded(store: &memory::Store, meeting_id: i64, material: &[&str]) -> bool {
    let filter = crate::exclusion::context_filter(store);
    if !filter.is_active() {
        return false;
    }
    let (attendees, carries) = match store.get_meeting(meeting_id) {
        Ok(Some(m)) => {
            let carries = filter.excludes_meeting(m.id, &m.title, &m.attendees);
            // The addresses live in the attendee details (`attendees` holds display names).
            let mut attendees = m.attendees;
            attendees.extend(
                store.meeting_attendee_details(meeting_id).unwrap_or_default().into_iter().filter_map(|a| a.email),
            );
            (attendees, carries)
        }
        Ok(None) => (Vec::new(), filter.material().is_none_or(|x| x.meetings.contains(&meeting_id))),
        Err(_) => (Vec::new(), true),
    };
    // Every other text written about the meeting (fix round 4, S1); unreadable is the client's.
    let draft = match store.meeting_authored_text(meeting_id) {
        Ok(text) => text,
        Err(_) => return true,
    };
    carries || crate::exclusion::classify_with(&filter, &format!("{}\n{draft}", material.join("\n")), &attendees)
}

/// Whether a summary of meeting `meeting_id` would be refused right now: it belongs to an
/// excluded client (see [`meeting_is_excluded`]) and the on-device chain is empty, so
/// [`summarize_meeting`] sends nothing. Recomputed on every call, so it clears itself once a
/// local model is turned on or installed. Reads the stored notes and transcript; names no client.
pub fn summary_refused_for_exclusion(
    store: &memory::Store,
    meeting_id: i64,
    ollama_model: &str,
    routing: &RoutingConfig,
) -> bool {
    summary_refused_with(store, meeting_id, || crate::routing::build_local_chain(ollama_model, routing).is_empty())
}

pub(crate) fn summary_refused_with(store: &memory::Store, meeting_id: i64, local_chain_is_empty: impl FnOnce() -> bool) -> bool {
    let notes = store.get_meeting(meeting_id).ok().flatten().and_then(|m| m.notes).unwrap_or_default();
    let transcript = store.meeting_transcript_text(meeting_id).unwrap_or_default();
    meeting_is_excluded(store, meeting_id, &[&notes, &transcript]) && local_chain_is_empty()
}

/// [`summarize_meeting`] down `chain`. `local_only`: the meeting is an excluded client's, so
/// `chain` holds on-device links only; empty, the meeting is not summarised and nothing is sent.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn summarize_meeting_with(
    store: &memory::Store,
    meeting_id: i64,
    notes: &str,
    transcript: &str,
    slides: &str,
    routing: &RoutingConfig,
    custom_instructions: &str,
    chain: Vec<llm::ChainLink>,
    local_only: bool,
) -> ApiResult<MeetingSummaryData> {
    summarize_meeting_pinned(store, meeting_id, notes, transcript, slides, routing, custom_instructions, chain, local_only, false)
}

/// [`summarize_meeting_with`]; `forced`: the chain is the preferred link alone and is pinned,
/// so that link over its budget fails the call (reason `budget`) instead of leaving no link.
#[allow(clippy::too_many_arguments)]
fn summarize_meeting_pinned(
    store: &memory::Store,
    meeting_id: i64,
    notes: &str,
    transcript: &str,
    slides: &str,
    routing: &RoutingConfig,
    custom_instructions: &str,
    chain: Vec<llm::ChainLink>,
    local_only: bool,
    forced: bool,
) -> ApiResult<MeetingSummaryData> {
    // A withheld note from an earlier call never describes this one.
    crate::envelope::take_google_data_withheld();
    set_last_failure_reason(None);
    let cap_config = routing.spend_cap_config();
    if local_only && chain.is_empty() {
        return Err(crate::exclusion::refused());
    }
    let chain = match crate::budget::apply_budgets(store, chain, forced, now_unix_ms()) {
        Ok(chain) => chain,
        Err(spent) => {
            if forced {
                set_last_failure_reason(Some((fallback_reason::BUDGET, None)));
            }
            return Err(spent.into());
        }
    };
    summarize_over_chain(store, &chain, &cap_config, meeting_id, notes, transcript, slides, custom_instructions, routing, local_only)
}

/// The chain walk: per link first Limited Use (a link that may not receive the meeting's Google
/// Calendar text is skipped), then the exclusion gate on the whole meeting for a hosted link.
#[allow(clippy::too_many_arguments)]
fn summarize_over_chain(
    store: &memory::Store,
    chain: &[llm::ChainLink],
    cap_config: &llm::SpendCapConfig,
    meeting_id: i64,
    notes: &str,
    transcript: &str,
    slides: &str,
    custom_instructions: &str,
    routing: &RoutingConfig,
    local_only: bool,
) -> ApiResult<MeetingSummaryData> {
    crate::envelope::take_google_data_withheld();
    // A hosted link gets the meeting whole or not at all: the gate sees every word first.
    let gate = crate::exclusion::current_gate(store);
    let whole = [notes, transcript, slides, custom_instructions].join("\n");
    // Limited Use: a meeting that may have come from Google Calendar has its invitees' names
    // and title in view when it is summarized, so a link that may not receive Google data is
    // skipped (the next allowed link answers); when none does, the call fails as a provider
    // failure and [`crate::envelope::take_google_data_withheld`] says why.
    let calendar_text_policy = store.meeting_may_hold_google_calendar_data(meeting_id).then(|| routing.google_data_policy());
    let mut withheld_any = false;
    let mut result: Option<(llm::MeetingSummary, llm::Provider, String)> = None;
    let mut last_cap_error: Option<llm::SpendCapError> = None;
    let mut failures = Vec::new();
    if chain.is_empty() {
        failures.push(llm::ProviderFailureKind::NoProviderConfigured);
    }
    // The fallback notice: only for a summary meant for Whimbrel AI (and never in local-only
    // mode, where a local answer is what the user asked for, nor for an excluded client's
    // meeting, which is summarised on this computer by design). `preferred_reason` is why the
    // preferred provider did not answer: no link at all (not signed in, no device key, its
    // budget), a link skipped for Limited Use, or the first error its call returned.
    let preferred = llm::Provider::parse(&routing.preferred_provider);
    let notice_applies = preferred == Some(llm::Provider::Whimbrel) && !routing.sensitive && !local_only;
    let mut preferred_reason: Option<ReasonCode> =
        (notice_applies && !chain.iter().any(|l| l.provider == llm::Provider::Whimbrel)).then(|| (missing_link_reason(routing), None));
    for entry in chain {
        if calendar_text_policy.is_some_and(|policy| !llm::limited_use::link_may_receive_google_data(entry, policy)) {
            withheld_any = true;
            failures.push(llm::ProviderFailureKind::Other { provider: entry.provider, status: None });
            if Some(entry.provider) == preferred {
                preferred_reason.get_or_insert((fallback_reason::GOOGLE_DATA_WITHHELD, None));
            }
            continue;
        }
        if llm::is_hosted(entry.provider, &entry.model) {
            if let Err(refusal) = gate.check(&whole) {
                failures.push(refusal.failure_kind());
                continue;
            }
        }
        let outcome = summarize_meeting_governed(
            store,
            cap_config,
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
        // Classified (and logged) before `err` is consumed by `chain_loop_action_for`. The
        // lines below print the kind, never the error text: a transport error's URL can
        // carry a key (`?key=`).
        let kind = match &err {
            GovernedSummaryError::Llm(e) => {
                let kind = llm::note_link_failure(entry.provider, &entry.model, e);
                failures.push(kind.clone());
                if Some(entry.provider) == preferred {
                    preferred_reason.get_or_insert(reason_for_error(entry.provider, e));
                }
                Some(kind)
            }
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
                    "[summarize_meeting] {} ({}) failed retryably, trying next link: {:?}",
                    entry.provider.as_str(),
                    entry.model,
                    kind
                );
                continue;
            }
            ChainLoopAction::StopWithProviderFailure => {
                eprintln!(
                    "[summarize_meeting] {} ({}) failed non-retryably, aborting chain: {:?}",
                    entry.provider.as_str(),
                    entry.model,
                    kind
                );
                break;
            }
        }
    }
    let Some((summary, provider, model)) = result else {
        return Err(match last_cap_error {
            Some(cap_err) => ApiError::SpendCap(cap_err),
            None => {
                if withheld_any {
                    crate::envelope::note_google_data_withheld();
                }
                set_last_failure_reason(preferred_reason);
                crate::chain_failed(failures)
            }
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
    let fallback = (notice_applies && provider != llm::Provider::Whimbrel).then(|| {
        let (reason, http_status) = preferred_reason.unwrap_or((fallback_reason::UNAVAILABLE, None));
        // The code and providers only: nothing of the meeting or the provider's error text.
        llm::note_fallback("whimbrel", provider, reason, http_status);
        eprintln!("[summarize_meeting] meant for whimbrel, written by {} ({model}): reason={reason} status={http_status:?}", provider.as_str());
        SummaryFallback { preferred: "whimbrel".into(), used: provider.as_str().into(), model: model.clone(), reason: reason.into(), http_status }
    });
    // Stored with the summary, so the notice survives a restart; a summary the preferred
    // provider wrote forgets an older notice.
    let row = fallback.as_ref().map(memory::summary_fallback::SummaryFallbackRow::from);
    let _ = store.set_summary_fallback(meeting_id, row.as_ref());
    // #65: retitle a manually-started meeting (still the literal placeholder title
    // "Meeting") now that a real summary exists. No-op for a calendar-titled or
    // already-renamed meeting — see set_meeting_title_if_placeholder's doc comment.
    let derived_title = llm::meeting_title(&summary);
    let _ = store.set_meeting_title_if_placeholder(meeting_id, &derived_title);
    // The title check sends the summary to TypeSafe (hosted): never for an excluded client's meeting.
    if !local_only {
        crate::title_check::check_calendar_title(store, meeting_id, routing, &summary, &derived_title);
    }
    Ok(MeetingSummaryData {
        summary,
        provider,
        model,
        fallback,
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
            action_items_detail: vec![],
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
            fallback: None,
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

    /// The 10/30 "Quick Debrief": a silent recording with no notes or slides was sent to
    /// the model, which summarized the user's profile instead. Nothing is asked then: the
    /// spend-cap pre-check (which would reject this call) is never reached.
    #[test]
    fn summarize_meeting_with_nothing_to_summarize_never_asks_a_model() {
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
        let routing = RoutingConfig { global_daily_cap_cents: Some(1), ..RoutingConfig::default() };
        let result = summarize_meeting(&store, 1, " ", "\n", "", "gemma4:12b", &routing, "");
        assert!(matches!(result, Err(ApiError::ProviderFailure)), "{result:?}");
        assert_eq!(store.count_usage_log_rows().unwrap(), 1);
    }

    /// The summary's own chain loop: an OpenAI-only chain whose budget is spent fails with
    /// BudgetSpent before any provider is tried (no network: the usage log is unchanged).
    #[test]
    fn summary_loop_respects_budget() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: "openai".into(),
                model: "gpt-5-mini".into(),
                route: Some("ask".into()),
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: 2500,
                cache_hit: false,
                trace_id: "t".into(),
                ts_unix_ms: now_unix_ms(),
            })
            .unwrap();
        store.set_provider_budgets(&[("openai".into(), 2500)]).unwrap();
        let routing = RoutingConfig {
            openai_key: Some("k".into()),
            use_local_model: false,
            user_daily_cap_cents: Some(0),
            global_daily_cap_cents: Some(0),
            ..RoutingConfig::default()
        };
        let result = summarize_meeting(&store, 1, "notes", "transcript", "", "gemma4:12b", &routing, "");
        let Err(ApiError::BudgetSpent(spent)) = result else { panic!("{result:?}") };
        assert_eq!((spent.provider, spent.spent_cents, spent.budget_cents), (llm::Provider::OpenAI, 2500, 2500));
        assert_eq!(store.count_usage_log_rows().unwrap(), 1);
    }

    // --- Limited Use: meetings that may have come from Google Calendar ---

    struct Counting(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl llm::LlmClient for Counting {
        fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(llm::Completion {
                text: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
                token_usage: llm::TokenUsage::default(),
            })
        }
    }

    fn calendar_meeting_of_unknown_source(store: &memory::Store) -> i64 {
        store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Zebra Review".into(),
                attendees: vec!["Ana".into()],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap()
    }

    fn over<const N: usize>(store: &memory::Store, id: i64, chain: [llm::ChainLink; N]) -> ApiResult<MeetingSummaryData> {
        summarize_meeting_with(store, id, "notes", "transcript", "", &RoutingConfig::default(), "", chain.into(), false)
    }

    fn counted(provider: llm::Provider, n: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> llm::ChainLink {
        llm::ChainLink { provider, model: "m".into(), client: Box::new(Counting(n.clone())) }
    }

    #[test]
    fn a_google_calendar_meeting_is_summarized_only_by_a_link_that_may_receive_it() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let store = memory::Store::open_in_memory().unwrap();
        let id = calendar_meeting_of_unknown_source(&store);
        let (restricted, allowed) = (std::sync::Arc::new(AtomicUsize::new(0)), std::sync::Arc::new(AtomicUsize::new(0)));

        // Only a restricted link: no summary, and the shell is told why.
        let r = over(&store, id, [counted(llm::Provider::OpenRouter, &restricted)]);
        assert!(matches!(r, Err(ApiError::ProviderFailure)));
        assert_eq!(restricted.load(Ordering::SeqCst), 0, "OpenRouter was sent the meeting");
        assert!(crate::envelope::provider_failure_envelope().contains(r#""google_data_withheld":true"#));
        assert!(!crate::envelope::provider_failure_envelope().contains("google_data_withheld"), "reported once");

        // The chain falls through to an allowed link.
        let r = over(&store, id, [counted(llm::Provider::OpenRouter, &restricted), counted(llm::Provider::Anthropic, &allowed)]);
        assert_eq!(r.unwrap().provider, llm::Provider::Anthropic);
        assert_eq!((restricted.load(Ordering::SeqCst), allowed.load(Ordering::SeqCst)), (0, 1));
        assert!(!crate::envelope::take_google_data_withheld(), "a summary that succeeds reports nothing");

        // A meeting recorded as another calendar's is summarized by any link.
        assert!(store.set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_OTHER).unwrap());
        over(&store, id, [counted(llm::Provider::OpenRouter, &restricted)]).unwrap();
        assert_eq!(restricted.load(Ordering::SeqCst), 1);
    }

    /// Counts calls and keeps prompts; every answer is a valid summary.
    struct PromptKeeper {
        prompts: std::cell::RefCell<Vec<String>>,
    }
    impl llm::LlmClient for PromptKeeper {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            self.prompts.borrow_mut().push(prompt.to_string());
            Ok(llm::Completion {
                text: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [{\"owner\": \"Ben\", \"task\": \"send it\"}], \"open_questions\": []}".into(),
                token_usage: llm::TokenUsage::default(),
            })
        }
    }

    fn governed_calls(provider: llm::Provider) -> (usize, llm::MeetingSummary, Vec<String>) {
        let store = memory::Store::open_in_memory().unwrap();
        let cap = llm::SpendCapConfig { user_daily_cents: None, global_daily_cents: None };
        let client = PromptKeeper { prompts: Default::default() };
        // About 62,500 characters: the size of a 67-minute meeting's transcript.
        let transcript = "[   0.0s] Ben: this is one line of transcript text that repeats many times.\n".repeat(830);
        assert!((60_000..65_000).contains(&transcript.len()));
        let summary = summarize_meeting_governed(&store, &cap, &client, provider, "m", "notes", &transcript, "", "").unwrap();
        let prompts = client.prompts.borrow().clone();
        (prompts.len(), summary, prompts)
    }

    #[test]
    fn a_hosted_model_summarizes_a_long_meeting_in_one_call() {
        for provider in [llm::Provider::Anthropic, llm::Provider::Whimbrel, llm::Provider::OpenAI] {
            let (calls, summary, _) = governed_calls(provider);
            assert_eq!(calls, 1, "{provider:?}");
            assert_eq!(summary.action_items, vec!["Ben: send it"]);
            assert_eq!(summary.action_items_detail.len(), 1);
        }
    }

    #[test]
    fn a_local_model_splits_by_its_window_and_each_later_chunk_gets_the_previous_tldr() {
        let (calls, _, prompts) = governed_calls(llm::Provider::Ollama);
        // 62.5k characters over a 37k chunk: two chunks and a reduce, not the old (8192-900)*4 three-plus.
        assert_eq!(calls, 3);
        assert!(!prompts[0].contains("previous part"));
        assert!(prompts[1].contains("previous part ended with this summary: ok"), "{}", prompts[1]);
        assert!(prompts[2].contains("Part 1:") && prompts[2].contains("\"task\":\"send it\""), "{}", prompts[2]);
    }

    // --- The fallback notice: a summary meant for Whimbrel AI that another provider wrote ---

    /// Fails every call with the error `make` builds.
    struct Failing(fn() -> llm::LlmError);
    impl llm::LlmClient for Failing {
        fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            Err((self.0)())
        }
    }

    fn manual_meeting(store: &memory::Store) -> i64 {
        store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Standup".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap()
    }

    fn whimbrel_routing(token: bool, key: bool) -> RoutingConfig {
        RoutingConfig {
            preferred_provider: "whimbrel".into(),
            whimbrel_id_token: token.then(|| "tok".into()),
            whimbrel_device_key: key.then(|| "key".into()),
            ..RoutingConfig::default()
        }
    }

    fn walk(store: &memory::Store, id: i64, chain: &[llm::ChainLink], routing: &RoutingConfig) -> ApiResult<MeetingSummaryData> {
        summarize_over_chain(store, chain, &llm::SpendCapConfig::default(), id, "notes", "transcript", "", "", routing, false)
    }

    fn failing_link(provider: llm::Provider, make: fn() -> llm::LlmError) -> llm::ChainLink {
        llm::ChainLink { provider, model: "whimbrel-model".into(), client: Box::new(Failing(make)) }
    }

    fn counter() -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
        std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0))
    }

    #[test]
    fn a_missing_sign_in_or_device_key_or_budget_says_why_the_local_model_answered() {
        // Whimbrel has no link in the chain: the routing says which of three causes.
        for (token, key, reason) in [
            (false, false, "not_signed_in"),
            (false, true, "not_signed_in"),
            (true, false, "no_device_key"),
            (true, true, "budget"),
        ] {
            let store = memory::Store::open_in_memory().unwrap();
            let id = manual_meeting(&store);
            let data = walk(&store, id, &[counted(llm::Provider::Ollama, &counter())], &whimbrel_routing(token, key)).unwrap();
            let fallback = data.fallback.expect("a fallback");
            assert_eq!((fallback.preferred.as_str(), fallback.used.as_str(), fallback.reason.as_str()), ("whimbrel", "ollama", reason));
            assert_eq!(fallback.http_status, None);
            assert_eq!(fallback.model, "m");
        }
    }

    #[test]
    fn a_whimbrel_call_that_failed_says_which_way_it_failed() {
        let cases: [(fn() -> llm::LlmError, &str, Option<u16>); 8] = [
            (|| llm::LlmError::Http { status: 429, body: "secret body".into() }, "rate_limited", Some(429)),
            (|| llm::LlmError::Http { status: 503, body: String::new() }, "unavailable", Some(503)),
            (|| llm::LlmError::Http { status: 408, body: String::new() }, "timeout", Some(408)),
            (|| llm::LlmError::Network("operation timed out".into()), "timeout", None),
            (|| llm::LlmError::Network("connection refused".into()), "network", None),
            (|| llm::LlmError::Http { status: 401, body: String::new() }, "unauthorized", Some(401)),
            (|| llm::LlmError::Http { status: 402, body: String::new() }, "credit_exhausted", Some(402)),
            (|| llm::LlmError::Provider("unreadable".into()), "unavailable", None),
        ];
        for (make, reason, status) in cases {
            let store = memory::Store::open_in_memory().unwrap();
            let id = manual_meeting(&store);
            let chain = [failing_link(llm::Provider::Whimbrel, make), counted(llm::Provider::Ollama, &counter())];
            let data = walk(&store, id, &chain, &whimbrel_routing(true, true)).unwrap();
            let fallback = data.fallback.expect("a fallback");
            assert_eq!((fallback.reason.as_str(), fallback.http_status), (reason, status));
            assert_eq!(fallback.used, "ollama");
            // Content-free: nothing of the error text reaches the notice.
            assert!(!serde_json::to_string(&fallback).unwrap().contains("secret"));
        }
    }

    #[test]
    fn no_notice_when_whimbrel_answered_was_not_picked_or_local_only_was() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = manual_meeting(&store);
        let whimbrel_answers = walk(&store, id, &[counted(llm::Provider::Whimbrel, &counter())], &whimbrel_routing(true, true)).unwrap();
        assert!(whimbrel_answers.fallback.is_none());
        assert!(!serde_json::to_string(&whimbrel_answers).unwrap().contains("fallback"));
        // Ollama picked: a local answer is the plan, so there is nothing to explain.
        let ollama_picked = walk(&store, id, &[counted(llm::Provider::Ollama, &counter())], &RoutingConfig::default()).unwrap();
        assert!(ollama_picked.fallback.is_none());
        // Local-only mode with Whimbrel still picked: local is what the user asked for.
        let local_only = RoutingConfig { sensitive: true, ..whimbrel_routing(false, false) };
        assert!(walk(&store, id, &[counted(llm::Provider::Ollama, &counter())], &local_only).unwrap().fallback.is_none());
    }

    #[test]
    fn the_notice_is_in_the_result_json_and_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("summary-fallback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.sqlite");
        let id = {
            let store = memory::Store::open(&path, &dir).unwrap();
            let id = manual_meeting(&store);
            let chain = [failing_link(llm::Provider::Whimbrel, || llm::LlmError::Http { status: 502, body: String::new() }), counted(llm::Provider::Ollama, &counter())];
            let data = walk(&store, id, &chain, &whimbrel_routing(true, true)).unwrap();
            let json: serde_json::Value = serde_json::to_value(&data).unwrap();
            assert_eq!(
                json["fallback"],
                serde_json::json!({"preferred": "whimbrel", "used": "ollama", "model": "m", "reason": "unavailable", "http_status": 502})
            );
            id
        };
        // A new connection (an app restart) reads the same notice.
        let store = memory::Store::open(&path, &dir).unwrap();
        let stored = stored_summary_fallback(&store, id).expect("persisted");
        assert_eq!((stored.preferred.as_str(), stored.used.as_str(), stored.reason.as_str(), stored.http_status), ("whimbrel", "ollama", "unavailable", Some(502)));
        // A summary Whimbrel writes later clears it; one another provider writes leaves it stale.
        store.update_meeting_summary(id, "t", &[], &[], &[], "anthropic", "claude").unwrap();
        assert!(stored_summary_fallback(&store, id).is_none(), "stale after another provider's summary");
        walk(&store, id, &[counted(llm::Provider::Ollama, &counter())], &whimbrel_routing(false, false)).unwrap();
        assert!(stored_summary_fallback(&store, id).is_some());
        walk(&store, id, &[counted(llm::Provider::Whimbrel, &counter())], &whimbrel_routing(true, true)).unwrap();
        assert!(stored_summary_fallback(&store, id).is_none(), "the preferred provider's summary clears it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_forced_resummary_uses_only_the_preferred_link_and_never_falls_back() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = manual_meeting(&store);
        let routing = whimbrel_routing(true, true);
        let local = counter();

        // The preferred link answers: one link, no fallback, no local call.
        let chain = forced_chain(vec![counted(llm::Provider::Whimbrel, &counter()), counted(llm::Provider::Ollama, &local)], &routing).unwrap();
        assert_eq!(chain.len(), 1);
        let data = walk(&store, id, &chain, &routing).unwrap();
        assert_eq!((data.provider, data.fallback), (llm::Provider::Whimbrel, None));

        // The preferred link fails: the call fails with its reason, and the local model is never asked.
        let chain = forced_chain(
            vec![failing_link(llm::Provider::Whimbrel, || llm::LlmError::Http { status: 429, body: String::new() }), counted(llm::Provider::Ollama, &local)],
            &routing,
        )
        .unwrap();
        assert!(matches!(walk(&store, id, &chain, &routing), Err(ApiError::ProviderFailure)));
        assert_eq!(take_last_summary_failure_reason(), Some(("rate_limited", Some(429))));
        assert_eq!(local.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn a_forced_resummary_without_the_preferred_link_fails_with_the_reason() {
        for (routing, reason) in [
            (whimbrel_routing(false, false), "not_signed_in"),
            (whimbrel_routing(true, false), "no_device_key"),
            (RoutingConfig { sensitive: true, ..whimbrel_routing(true, true) }, "local_only"),
        ] {
            let local = counter();
            // What build_chain gives without the Whimbrel link: the local model first.
            let refused = forced_chain(vec![counted(llm::Provider::Ollama, &local)], &routing);
            assert!(matches!(refused, Err(ApiError::ProviderFailure)));
            assert_eq!(take_last_summary_failure_reason(), Some((reason, None)));
            assert_eq!(local.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
    }

    /// The whole entry point, offline: a forced call with no sign-in builds no Whimbrel link,
    /// so it fails before any network, logs no usage and leaves the meeting untouched.
    #[test]
    fn summarize_meeting_preferred_without_sign_in_fails_before_any_call() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = manual_meeting(&store);
        let result = summarize_meeting_preferred(&store, id, "notes", "transcript", "", "gemma4:12b", &whimbrel_routing(false, true), "");
        assert!(matches!(result, Err(ApiError::ProviderFailure)), "{result:?}");
        assert_eq!(take_last_summary_failure_reason(), Some(("not_signed_in", None)));
        assert_eq!(store.count_usage_log_rows().unwrap(), 0);
        assert!(stored_summary_fallback(&store, id).is_none());
    }
}
