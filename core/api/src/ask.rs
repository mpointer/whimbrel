//! FR-CHAT: grounded chat — the blocking `answer`, the streaming `prepare_ask` /
//! `finalize_ask` / `discard_ask` split (#74), and conversation titles (FR-CHAT-6).
//! Moved from `core/ffi`'s `whimbrel_llm_answer` family (M3.0); the C functions are thin
//! wrappers. Argument decoding (`history_json`, the `-1` token sentinels) stays in `ffi`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

use crate::budget::{apply_budgets, check_provider};
use crate::envelope::{ApiError, ApiResult};
use crate::exclusion::{ExclusionOutcome, RequestOrigin};
use crate::routing::{build_chain_for, build_local_chain, RoutingConfig};
use crate::util::{log_usage_for, now_unix_ms};

/// Retrieval (keyword and semantic, fused) using `query` alone, then the spend-cap
/// check, then the provider chain, then usage logging. `history` is fed to the model for
/// conversational continuity but does not affect retrieval. Returns [`llm::ChatAnswer`]
/// unchanged — its serialized shape (`text`, `citations`, `token_usage`) is exactly what
/// the C boundary already emits. Asked from the Ask bar; [`answer_from`] takes the origin
/// and also says what client exclusion did.
pub fn answer(
    store: &memory::Store,
    query: &str,
    ollama_model: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<llm::ChatAnswer> {
    answer_from(store, query, RequestOrigin::AskBar, ollama_model, context_limit, history, routing).map(|(a, _)| a)
}

/// [`answer`] for a question asked from `origin`. A question that carries an excluded client
/// (spec 7.2) is answered by the on-device links only, with all of memory, and refused when there
/// is none: it never falls back to a hosted model. Otherwise a hosted link answers from memory
/// without any excluded client's material, and the outcome says how much was left out.
pub fn answer_from(
    store: &memory::Store,
    query: &str,
    origin: RequestOrigin,
    ollama_model: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<(llm::ChatAnswer, ExclusionOutcome)> {
    answer_from_filter(store, crate::exclusion::context_filter(store), query, origin, ollama_model, context_limit, history, routing)
}

/// The link that produced an answer: its provider id and the model that answered (the one
/// the provider stood in when the requested one did not exist). Never the client's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnsweredBy {
    pub provider: String,
    pub model: String,
    /// The link runs on this computer (Ollama or the bundled model).
    pub local: bool,
}

impl AnsweredBy {
    fn of(link: &llm::ChainLink) -> Self {
        AnsweredBy {
            provider: link.provider.as_str().to_string(),
            model: link.client.substituted_model().unwrap_or_else(|| link.model.clone()),
            local: link.provider.is_local(),
        }
    }
}

/// [`llm::ChatAnswer`] with what exclusion did and which model answered, for shells that
/// label the answer with the model that really produced it.
#[derive(Debug, Clone, Serialize)]
pub struct DetailedAnswer {
    #[serde(flatten)]
    pub answer: llm::ChatAnswer,
    pub exclusion: ExclusionOutcome,
    pub answered_by: AnsweredBy,
}

/// [`answer`] that also reports the exclusion outcome and the answering model. Same routing
/// and exclusion behaviour as [`answer_from`]; asked from the Ask bar.
pub fn answer_detailed(
    store: &memory::Store,
    query: &str,
    ollama_model: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<DetailedAnswer> {
    let filter = crate::exclusion::context_filter(store);
    let (answer, exclusion, answered_by) =
        answer_from_filter_by(store, filter, query, RequestOrigin::AskBar, ollama_model, context_limit, history, routing)?;
    Ok(DetailedAnswer { answer, exclusion, answered_by })
}

/// Whether a question asked in the Ask bar (with the conversation so far) names an excluded
/// client, so core answers it on this computer or refuses it. A cheap check on names the
/// store already holds; true when the setting cannot be read. Never returns the name.
pub fn question_names_excluded_client(store: &memory::Store, query: &str, history: &[llm::ChatTurn]) -> bool {
    let filter = crate::exclusion::context_filter(store);
    request_carries(store, &filter, query, history, &RequestOrigin::AskBar)
}

/// [`answer_from`] through `filter`, read once by the caller (fix round 3).
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_from_filter(
    store: &memory::Store,
    filter: llm::ContextFilter,
    query: &str,
    origin: RequestOrigin,
    ollama_model: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<(llm::ChatAnswer, ExclusionOutcome)> {
    answer_from_filter_by(store, filter, query, origin, ollama_model, context_limit, history, routing)
        .map(|(answer, outcome, _)| (answer, outcome))
}

/// [`answer_from_filter`] that also says which link answered.
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_from_filter_by(
    store: &memory::Store,
    filter: llm::ContextFilter,
    query: &str,
    origin: RequestOrigin,
    ollama_model: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<(llm::ChatAnswer, ExclusionOutcome, AnsweredBy)> {
    answer_with_filter_by(
        store,
        filter,
        query,
        &origin,
        context_limit,
        history,
        routing,
        || build_chain_for(llm::TaskKind::Chat, ollama_model, routing, &crate::exclusion::current_gate(store)),
        || build_local_chain(ollama_model, routing),
    )
}

/// The text of a request beyond retrieval: the question and the conversation so far.
pub(crate) fn request_text(query: &str, history: &[llm::ChatTurn]) -> String {
    let mut text = query.to_string();
    for turn in history {
        text.push('\n');
        text.push_str(&turn.text);
    }
    text
}

/// Whether a request carries an excluded client: its words or history name one, it is asked from
/// one's page or meeting, or an earlier turn of the conversation was answered on this computer
/// from an excluded client's material (fix round 1, B2: such an answer never rides to a hosted
/// model as history).
pub(crate) fn request_carries(
    store: &memory::Store,
    filter: &llm::ContextFilter,
    query: &str,
    history: &[llm::ChatTurn],
    origin: &RequestOrigin,
) -> bool {
    crate::exclusion::carries_with(store, filter, &request_text(query, history), origin)
        || (filter.is_active() && history.iter().any(|t| turn_carries(store, filter, t)))
}

/// Whether an earlier turn may not ride to a hosted model: an answer made on this computer from
/// an excluded client's material, or any answer not marked clean for every client excluded now
/// (fix round 2, N7: an answer from before a client was excluded is not clean, whatever made
/// it). The user's own words carry only by naming a client or holding a known contact's address
/// (checked with the question).
fn turn_carries(store: &memory::Store, filter: &llm::ContextFilter, turn: &llm::ChatTurn) -> bool {
    if store.is_local_only_text(&turn.text).unwrap_or(true) {
        return true;
    }
    match turn.role {
        llm::ChatRole::User => false,
        llm::ChatRole::Assistant => !text_is_clean(store, filter, &turn.text),
    }
}

/// A clean stamp ready to write: the recorded inputs, and the word fingerprints of its `text:`
/// inputs. Nothing is written until [`mark_clean`] / [`finalize_ask`] writes the stamp, so a
/// cancelled stream, a capped or unstamped answer leaves no fingerprints (fix round 5, N1).
#[derive(Clone)]
pub(crate) struct CleanStamp {
    stamp: String,
    words: Vec<(i64, Vec<u32>)>,
}

#[cfg(test)]
impl CleanStamp {
    pub(crate) fn refs(&self) -> Vec<String> {
        memory::topics::exclusion::stamp_input_refs(&self.stamp).unwrap_or_default()
    }
    pub(crate) fn kept_words(&self) -> &[(i64, Vec<u32>)] {
        &self.words
    }
}

/// The stamp an answer built from `context` (plus `extra` inputs) after `history` carries: the
/// inputs it was built from (fix round 4, S2), clean while none of them is excluded. A clean
/// earlier answer in `history` adds its own inputs; one with no record, a closed filter, or more
/// inputs than the cap gives no stamp (never clean while a client is excluded).
///
/// `texts` are words the answer was built from that have no stored row (the question, a window's
/// redacted text), and so is every user turn of `history` (a live draft's meeting background):
/// recorded as `text:` inputs whose word fingerprints (never the words) are checked by name
/// whenever the stamp is. The fingerprints are kept only while some client is excluded: with
/// nothing excluded the `text:` inputs are recorded without them, so the answer stops being clean
/// once a client is excluded (fail-closed, and no trace for users who never exclude anyone).
pub(crate) fn clean_stamp(
    filter: &llm::ContextFilter,
    store: &memory::Store,
    context: &[llm::ContextItem],
    extra: impl IntoIterator<Item = String>,
    texts: &[&str],
    history: &[llm::ChatTurn],
) -> Option<CleanStamp> {
    if filter.is_closed() {
        return None;
    }
    let mut refs: Vec<String> = context.iter().map(llm::exclusion::input_ref_of).chain(extra).collect();
    let mut words = Vec::new();
    let user_turns = history.iter().filter(|t| t.role == llm::ChatRole::User).map(|t| t.text.as_str());
    for text in texts.iter().copied().chain(user_turns).filter(|t| !t.trim().is_empty()) {
        let (r, hash, fingerprints) = memory::topics::exclusion::input_text_ref(text);
        refs.push(r);
        if filter.is_active() {
            words.push((hash, fingerprints));
        }
    }
    for turn in history.iter().filter(|t| t.role == llm::ChatRole::Assistant) {
        let stamp = store.clean_text_stamp(&turn.text).ok().flatten()?;
        refs.extend(memory::topics::exclusion::stamp_input_refs(&stamp)?);
    }
    let stamp = memory::topics::exclusion::input_refs_stamp(refs)?;
    Some(CleanStamp { stamp, words })
}

/// Whether `text` (an answer) is clean for the material `filter` excludes now: none of the inputs
/// it recorded is excluded (fix round 4, S2). Anything is while nothing is excluded.
fn text_is_clean(store: &memory::Store, filter: &llm::ContextFilter, text: &str) -> bool {
    match filter.material() {
        Some(m) => store.clean_text_stamp(text).ok().flatten().is_some_and(|stamp| stamp_is_clean(store, filter, m, &stamp)),
        None => !filter.is_closed(),
    }
}

/// Whether none of `stamp`'s recorded inputs is excluded now: stored rows by id, kept word
/// fingerprints (`text:` inputs) by name. Words that cannot be read, or rows that no longer exist, are not
/// clean.
fn stamp_is_clean(store: &memory::Store, filter: &llm::ContextFilter, m: &memory::topics::exclusion::ExcludedMaterial, stamp: &str) -> bool {
    let Some(refs) = memory::topics::exclusion::stamp_input_refs(stamp) else { return false };
    // An input whose row is gone cannot be checked (deleted, pruned): not clean.
    if !store.missing_input_refs(&refs).is_ok_and(|missing| missing.is_empty()) {
        return false;
    }
    refs.iter().all(|r| match r.strip_prefix("text:") {
        Some(hash) => hash
            .parse::<i64>()
            .ok()
            .and_then(|h| store.input_words(h).ok().flatten())
            .is_some_and(|fingerprints| !filter.names_client_in_fingerprints(&fingerprints)),
        None => !m.excludes_input_ref(r),
    })
}

/// Records `answer`'s inputs ([`clean_stamp`]) and the word fingerprints it owns; `None` records
/// nothing (not clean).
pub(crate) fn mark_clean(store: &memory::Store, stamp: Option<CleanStamp>, answer: &str) {
    let Some(stamp) = stamp else { return };
    if let Err(e) = store.mark_clean_text(answer, &stamp.stamp, &stamp.words) {
        eprintln!("[exclusion] could not mark a clean answer: {e}");
    }
}

/// Marks `answer` as made on this computer from an excluded client's material (best effort: a
/// failed write is logged, and the answer is still returned).
pub(crate) fn mark_local_only(store: &memory::Store, answer: &str) {
    if let Err(e) = store.mark_local_only_text(answer) {
        eprintln!("[exclusion] could not mark a local-only answer: {e}");
    }
}

/// [`answer_with_filter`] with the filter read here. Tests only (the exclusion sweep); the callers read the
/// filter themselves.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_with(
    store: &memory::Store,
    query: &str,
    origin: &RequestOrigin,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    chain: impl FnOnce() -> Vec<llm::ChainLink>,
    local_chain: impl FnOnce() -> Vec<llm::ChainLink>,
) -> ApiResult<(llm::ChatAnswer, ExclusionOutcome)> {
    answer_with_filter(store, crate::exclusion::context_filter(store), query, origin, context_limit, history, routing, chain, local_chain)
}

/// [`answer_with`] through `filter`, read once by the caller for the origin, the decision, the
/// context and the stamp (fix round 3).
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_with_filter(
    store: &memory::Store,
    filter: llm::ContextFilter,
    query: &str,
    origin: &RequestOrigin,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    chain: impl FnOnce() -> Vec<llm::ChainLink>,
    local_chain: impl FnOnce() -> Vec<llm::ChainLink>,
) -> ApiResult<(llm::ChatAnswer, ExclusionOutcome)> {
    answer_with_filter_by(store, filter, query, origin, context_limit, history, routing, chain, local_chain)
        .map(|(answer, outcome, _)| (answer, outcome))
}

/// [`answer_with_filter`] that also says which link answered.
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_with_filter_by(
    store: &memory::Store,
    filter: llm::ContextFilter,
    query: &str,
    origin: &RequestOrigin,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    chain: impl FnOnce() -> Vec<llm::ChainLink>,
    local_chain: impl FnOnce() -> Vec<llm::ChainLink>,
) -> ApiResult<(llm::ChatAnswer, ExclusionOutcome, AnsweredBy)> {
    let carries = request_carries(store, &filter, query, history, origin);

    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("ask"), now_unix_ms())?;

    let instructions = llm::profile::compose_instructions(&routing.custom_instructions, &routing.profile, &routing.output_language);
    let policy = routing.google_data_policy();
    if carries {
        let chain = apply_budgets(store, local_chain(), false, now_unix_ms())?;
        if chain.is_empty() {
            return Err(crate::exclusion::refused());
        }
        let open = llm::ContextFilter::none();
        let (context, label) = assemble_ask_context(store, query, context_limit, &open);
        let (answer, entry) = llm::try_chain_with_failures(&chain, |entry| {
            // Limited Use applies on this computer too (an Ollama host off loopback).
            let may_receive = llm::limited_use::link_may_receive_google_data(entry, policy);
            let context = llm::limited_use::filter_context_in(store, &context, may_receive);
            let (history, history_google) = history_for_link(store, history, may_receive);
            let answer = llm::answer_with_citations(entry.client.as_ref(), &history, query, &context, &instructions, label.as_deref())?;
            record_answer_provenance(store, &answer.text, history_google || context.iter().any(|c| c.is_google_user_data()));
            Ok(answer)
        })
        .map_err(crate::chain_failed)?;
        crate::util::log_link_usage(store, entry, "ask", answer.token_usage);
        mark_local_only(store, &answer.text);
        return Ok((answer, ExclusionOutcome::LocalOnly, AnsweredBy::of(entry)));
    }

    let chain = apply_budgets(store, chain(), false, now_unix_ms())?;
    let hosted = crate::exclusion::chain_is_hosted(&chain);
    // Hosted links answer from memory without excluded material; on-device links from all of it.
    let open = llm::ContextFilter::none();
    let (hosted_context, label) = assemble_ask_context(store, query, context_limit, if hosted { &filter } else { &open });
    let local_context = if hosted && filter.is_active() && chain.iter().any(|l| !llm::is_hosted(l.provider, &l.model)) {
        Some(assemble_ask_context(store, query, context_limit, &open).0)
    } else {
        None
    };
    let (answer, entry) = llm::try_chain_with_failures(&chain, |entry| {
        let context = match &local_context {
            Some(local) if !llm::is_hosted(entry.provider, &entry.model) => local,
            _ => &hosted_context,
        };
        // Limited Use: each link also gets only the Google user data it may receive, and its
        // citations point into that same context.
        let may_receive = llm::limited_use::link_may_receive_google_data(entry, policy);
        let context = llm::limited_use::filter_context_in(store, context, may_receive);
        let (history, history_google) = history_for_link(store, history, may_receive);
        let answer = llm::answer_with_citations(entry.client.as_ref(), &history, query, &context, &instructions, label.as_deref())?;
        // The answer inherits the taint of the context and the turns this link received.
        record_answer_provenance(store, &answer.text, history_google || context.iter().any(|c| c.is_google_user_data()));
        Ok(answer)
    })
    .map_err(crate::chain_failed)?;
    crate::util::log_link_usage(store, entry, "ask", answer.token_usage);
    let answered_hosted = llm::is_hosted(entry.provider, &entry.model);
    // An on-device link answered from a context holding excluded material: the answer is
    // the client's as far as any later hosted call goes, and the outcome says so.
    let used = local_context.as_ref().filter(|_| !answered_hosted).unwrap_or(&hosted_context);
    if !answered_hosted && filter.is_active() && (filter.is_closed() || used.iter().any(|i| !filter.keeps(i))) {
        mark_local_only(store, &answer.text);
        return Ok((answer, ExclusionOutcome::LocalOnly, AnsweredBy::of(entry)));
    }
    mark_clean(store, clean_stamp(&filter, store, used, [], &[query], history), &answer.text);
    let outcome = ExclusionOutcome::after_hosted(answered_hosted, &filter);
    Ok((answer, outcome, AnsweredBy::of(entry)))
}

/// Limited Use: `history` as one link may receive it, and whether the turns kept hold
/// Google data. An answer written from Gmail or Calendar text (or whose provenance is not
/// recorded) is dropped with the question that prompted it for a link that may not
/// receive Google data; the turn stays for one that may. See
/// `memory::Store::answer_may_hold_google_data`.
pub(crate) fn history_for_link(
    store: &memory::Store,
    history: &[llm::ChatTurn],
    may_receive_google: bool,
) -> (Vec<llm::ChatTurn>, bool) {
    let mut kept: Vec<llm::ChatTurn> = Vec::with_capacity(history.len());
    let mut holds_google = false;
    for turn in history {
        if turn.role == llm::ChatRole::Assistant && store.answer_may_hold_google_data(&turn.text) {
            if !may_receive_google {
                if kept.last().is_some_and(|t| t.role == llm::ChatRole::User) {
                    kept.pop();
                }
                continue;
            }
            holds_google = true;
        }
        kept.push(turn.clone());
    }
    (kept, holds_google)
}

/// Records what an answer was built from, so it is found again as a turn of the history or
/// as the answer a title is made from. Best effort: an answer that cannot be recorded
/// counts as Google when it comes back.
pub(crate) fn record_answer_provenance(store: &memory::Store, answer: &str, google: bool) {
    let _ = store.record_answer_provenance(answer, google, now_unix_ms());
}

/// Shared phase-A logic for both the blocking [`answer`] and the streaming
/// [`prepare_ask`]: date-scoped ("today"/"yesterday") or generic context retrieval, through
/// `filter` (open for a prompt that stays on this computer).
fn assemble_ask_context(
    store: &memory::Store,
    query: &str,
    context_limit: usize,
    filter: &llm::ContextFilter,
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
                filter,
            );
            (items, Some(llm::format_date_scope_label(scope, start)))
        }
        None => (
            llm::gather_and_assemble_context(store, query, now_unix_ms(), context_limit, true, true, filter),
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
pub(crate) struct PreparedAsk {
    pub(crate) context: Vec<llm::ContextItem>,
    /// The provider the streamed answer is attributed to for usage/cost logging (#83):
    /// the routing config's preferred provider, the same one the shell then streams
    /// against — forced to Ollama in local-only mode, mirroring `build_chain`.
    pub(crate) provider: llm::Provider,
    /// What exclusion did to this prompt; `LocalOnly` marks the streamed answer at
    /// [`finalize_ask`] so it never reaches a hosted model later.
    pub(crate) exclusion: ExclusionOutcome,
    /// The inputs this prompt was built from ([`clean_stamp`]), so [`finalize_ask`] can mark the
    /// answer clean with them; `None` when it is not clean (local-only, closed, over the cap).
    pub(crate) clean_keys: Option<CleanStamp>,
    /// Limited Use: `Some(history_google)` when the answer's provenance is recorded at
    /// `finalize_ask` (an Ask over memory or a window), where `history_google` is whether
    /// the earlier turns sent with it held Google data. `None` for a meeting Ask.
    pub(crate) history_google: Option<bool>,
}

/// What a prepare stashes for [`finalize_ask`] to mark a non-local-only answer clean with.
pub(crate) fn clean_keys_for(
    store: &memory::Store,
    filter: &llm::ContextFilter,
    exclusion: ExclusionOutcome,
    context: &[llm::ContextItem],
    extra: impl IntoIterator<Item = String>,
    texts: &[&str],
    history: &[llm::ChatTurn],
) -> Option<CleanStamp> {
    if exclusion == ExclusionOutcome::LocalOnly {
        return None;
    }
    clean_stamp(filter, store, context, extra, texts, history)
}

static PREPARED_ASKS: OnceLock<Mutex<HashMap<String, PreparedAsk>>> = OnceLock::new();

pub(crate) fn prepared_asks() -> &'static Mutex<HashMap<String, PreparedAsk>> {
    PREPARED_ASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn provider_for_streamed_answer(routing: &RoutingConfig) -> llm::Provider {
    if routing.sensitive {
        return llm::Provider::Ollama;
    }
    match routing.preferred_provider.as_str() {
        "anthropic" => llm::Provider::Anthropic,
        "openai" => llm::Provider::OpenAI,
        "google" => llm::Provider::Google,
        "openrouter" => llm::Provider::OpenRouter,
        "whimbrel" => llm::Provider::Whimbrel,
        _ => llm::Provider::Ollama,
    }
}

/// What [`prepare_ask`] hands back: the one-shot id to pass to [`finalize_ask`] (or
/// [`discard_ask`]) exactly once, and the exact prompt `answer_with_citations` would
/// have sent, for the caller to stream against. Field order is the wire order. `exclusion`
/// says what client exclusion did, for the shell to show locally (never in the prompt).
#[derive(Debug, Clone, Serialize)]
pub struct PreparedAskData {
    pub prepare_id: String,
    pub prompt: String,
    pub exclusion: ExclusionOutcome,
}

/// #74: the assembly half of [`answer`], for a caller that wants to stream the actual
/// model call (via `inference`) instead of blocking on it. Runs retrieval + the spend-cap
/// check (both DB-bound), builds the prompt, and stashes the assembled context for
/// [`finalize_ask`] to extract citations from later. Asked from the Ask bar; see
/// [`prepare_ask_from`].
pub fn prepare_ask(
    store: &memory::Store,
    query: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<PreparedAskData> {
    prepare_ask_with(store, query, RequestOrigin::AskBar, context_limit, history, routing, None)
}

/// [`prepare_ask`] with this call's own Limited Use settings (`call`, read from this
/// call's routing JSON): its streamed Ollama model and host decide whether the prompt may
/// hold Gmail threads, never a value another call may have changed meanwhile.
pub fn prepare_ask_for(
    store: &memory::Store,
    query: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    call: &crate::routing::LimitedUseSettings,
) -> ApiResult<PreparedAskData> {
    prepare_ask_with(store, query, RequestOrigin::AskBar, context_limit, history, routing, Some(call))
}

/// [`prepare_ask`] for a question asked from `origin`. The shell streams against the routing's
/// provider: when that leaves this computer, retrieval leaves excluded material out, and a
/// question that carries an excluded client is refused here (`excluded_client`); the shell may
/// ask again with local-only routing. A stream on this computer gets all of memory.
pub fn prepare_ask_from(
    store: &memory::Store,
    query: &str,
    origin: RequestOrigin,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<PreparedAskData> {
    prepare_ask_with(store, query, origin, context_limit, history, routing, None)
}

fn prepare_ask_with(
    store: &memory::Store,
    query: &str,
    origin: RequestOrigin,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    call: Option<&crate::routing::LimitedUseSettings>,
) -> ApiResult<PreparedAskData> {
    crate::exclusion::refresh_published_gate(store);
    let filter = crate::exclusion::context_filter(store);
    if crate::exclusion::local_only_mode_refuses(routing) {
        return Err(crate::exclusion::refused());
    }
    let carries = request_carries(store, &filter, query, history, &origin);
    let hosted = crate::exclusion::streamed_is_hosted(routing);
    if carries && hosted {
        return Err(crate::exclusion::refused());
    }
    let open = llm::ContextFilter::none();
    let (context, date_scope_label) = assemble_ask_context(store, query, context_limit, if hosted { &filter } else { &open });

    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("ask"), now_unix_ms())?;
    // The shell streams against this one provider: pinned, no fallthrough.
    let provider = provider_for_streamed_answer(routing);
    check_provider(store, provider, now_unix_ms())?;
    // Limited Use: the prompt (and the citable context) holds Gmail threads only when the
    // provider streamed against may receive Google user data.
    let may_receive_google = crate::routing::streamed_may_receive_google_data(provider, call);
    let context = llm::limited_use::filter_context_in(store, &context, may_receive_google);
    // Earlier answers built from Google data follow the same rule.
    let (history, history_google) = history_for_link(store, history, may_receive_google);

    let prompt = llm::build_ask_prompt(
        &history,
        query,
        &context,
        &llm::profile::compose_instructions(&routing.custom_instructions, &routing.profile, &routing.output_language),
        date_scope_label.as_deref(),
    );
    // The shell streams this prompt to `provider`: a hosted one that must not see it is refused here.
    crate::exclusion::check_stream(store, provider, &prompt)?;
    let exclusion = if hosted {
        ExclusionOutcome::after_hosted(true, &filter)
    } else {
        note_local_stream(&filter, carries, &context, &prompt)
    };
    let prepare_id = llm::streaming::new_run_id();
    let clean_keys = clean_keys_for(store, &filter, exclusion, &context, [], &[query], &history);
    prepared_asks().lock().unwrap().insert(
        prepare_id.clone(),
        PreparedAsk { context, provider, exclusion, clean_keys, history_google: Some(history_google) },
    );
    Ok(PreparedAskData { prepare_id, prompt, exclusion })
}

/// A prompt built for a stream on this computer: when it carries an excluded client's
/// material, it is marked so an Ollama cloud model never gets it, and the outcome is
/// `LocalOnly`.
pub(crate) fn note_local_stream(
    filter: &llm::ContextFilter,
    carries: bool,
    context: &[llm::ContextItem],
    prompt: &str,
) -> ExclusionOutcome {
    if !filter.is_active() {
        return ExclusionOutcome::Allowed;
    }
    let holds_excluded = carries || filter.is_closed() || context.iter().any(|i| !filter.keeps(i));
    if holds_excluded {
        crate::exclusion::mark_local_only(prompt);
        ExclusionOutcome::LocalOnly
    } else {
        ExclusionOutcome::Allowed
    }
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
    prepare_meeting_ask_with(store, meeting_id, query, history, routing, |provider, carries| {
        streamed_link(store, routing, provider, carries)
    })
}

/// [`prepare_meeting_ask`] with the link a long transcript is read on injected:
/// `link_for(provider, carries)` ([`streamed_link`] in production).
pub(crate) fn prepare_meeting_ask_with(
    store: &memory::Store,
    meeting_id: i64,
    query: &str,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    link_for: impl FnOnce(llm::Provider, bool) -> Option<llm::ChainLink>,
) -> Result<PreparedMeetingAskData, MeetingAskError> {
    crate::exclusion::refresh_published_gate(store);
    // A meeting of an excluded client, or a question naming one, stays on this computer.
    let filter = crate::exclusion::context_filter(store);
    if crate::exclusion::local_only_mode_refuses(routing) {
        return Err(crate::exclusion::refused().into());
    }
    let carries = request_carries(store, &filter, query, history, &RequestOrigin::Meeting { meeting_id });
    let hosted = crate::exclusion::streamed_is_hosted(routing);
    if carries && hosted {
        return Err(crate::exclusion::refused().into());
    }
    let provider = provider_for_streamed_answer(routing);
    let may_receive_google = crate::routing::streamed_may_receive_google_data(provider, None);
    let mut built = meeting_ask_prompt(store, meeting_id, query, history, routing, may_receive_google, provider, None)?;
    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("ask"), now_unix_ms()).map_err(ApiError::from)?;
    check_provider(store, provider, now_unix_ms()).map_err(ApiError::from)?;
    // The shell streams this prompt to `provider`: a hosted one that must not see it is refused
    // here, before any part is read.
    crate::exclusion::check_stream(store, provider, &built.prompt)?;
    // A whole-meeting question on a transcript the streamed model cannot take whole: read it
    // part by part first (blocking), then stream the final answer. When no link can be built
    // for the streamed provider the keyword-narrowed prompt stands. The parts go through the
    // same exclusion gate as the stream: an excluded client's meeting is read on this computer
    // only, and a hosted link is guarded.
    if let Some(parts) = built.map_parts.take() {
        if let Some(link) = link_for(provider, carries) {
            match gather_findings(store, &cap_config, &link, query, &parts) {
                Ok(findings) => {
                    built = meeting_ask_prompt(store, meeting_id, query, history, routing, may_receive_google, provider, Some(&findings))?;
                }
                // The gate refused the meeting on a hosted link: do not fall back to streaming a
                // keyword-narrowed slice of it.
                Err(llm::LlmError::Refused(r)) => return Err(crate::envelope::chain_failed(vec![r.failure_kind()]).into()),
                Err(_) => {}
            }
        }
    }
    let MeetingAskPrompt { context, prompt, narrowed, google, .. } = built;
    let exclusion = if hosted { ExclusionOutcome::Allowed } else { note_local_stream(&filter, carries, &[], &prompt) };
    // The prompt with the findings is the one streamed: checked again.
    crate::exclusion::check_stream(store, provider, &prompt)?;

    let prepare_id = llm::streaming::new_run_id();
    let meeting_ref = memory::topics::exclusion::input_ref("meeting", meeting_id);
    let clean_keys = clean_keys_for(store, &filter, exclusion, &context, [meeting_ref], &[query], history);
    prepared_asks().lock().unwrap().insert(
        prepare_id.clone(),
        PreparedAsk { context, provider, exclusion, clean_keys, history_google: Some(google) },
    );
    Ok(PreparedMeetingAskData { prepare_id, prompt, narrowed })
}

/// The link a streamed answer runs against, built the way the blocking chain builds it, for
/// the calls that read a long transcript part by part before the stream starts. `None` when
/// the provider has no link (no key, or an Ollama model the shell has not sent).
///
/// Exclusion: when the request carries an excluded client (`carries`) only an on-device link is
/// built ([`build_local_chain`], never an Ollama cloud model); otherwise hosted links are behind
/// the current exclusion gate, as in the blocking chain.
fn streamed_link(
    store: &memory::Store,
    routing: &RoutingConfig,
    provider: llm::Provider,
    carries: bool,
) -> Option<llm::ChainLink> {
    let ollama_model = crate::routing::limited_use_settings().streamed_ollama_model.unwrap_or_default();
    if provider == llm::Provider::Ollama && ollama_model.is_empty() {
        return None;
    }
    let chain = if carries {
        build_local_chain(&ollama_model, routing)
    } else {
        build_chain_for(llm::TaskKind::Chat, &ollama_model, routing, &crate::exclusion::current_gate(store))
    };
    chain.into_iter().find(|l| l.provider == provider)
}

/// Reads a transcript that does not fit one prompt part by part, one call per part, on
/// `link`: what each part holds that bears on `query`. Every call is spend-cap checked and
/// its usage logged, so a cap trip stops the reading with nothing further spent.
fn gather_findings(
    store: &memory::Store,
    cap_config: &llm::SpendCapConfig,
    link: &llm::ChainLink,
    query: &str,
    parts: &MapParts,
) -> Result<Vec<String>, llm::LlmError> {
    let prompts: Vec<String> = parts
        .parts
        .iter()
        .enumerate()
        .map(|(i, part)| llm::meeting_ask::build_map_prompt(query, &parts.title, i + 1, parts.parts.len(), part, &parts.instructions))
        .collect();
    // Exclusion: a hosted link gets the transcript whole or not at all. Every part is checked
    // against the gate before the first is sent, so a meeting naming an excluded client only in a
    // late part never has its earlier parts sent part by part (the guard alone would refuse only
    // the part that names it), as the summary checks the whole meeting first.
    if llm::is_hosted(link.provider, &link.model) {
        let gate = crate::exclusion::current_gate(store);
        for prompt in &prompts {
            gate.check(prompt).map_err(llm::LlmError::Refused)?;
        }
    }
    let mut findings = Vec::with_capacity(prompts.len());
    for prompt in &prompts {
        llm::check_spend_cap(store, cap_config, None, Some("ask"), now_unix_ms())
            .map_err(|_| llm::LlmError::NotImplemented("meeting ask spend cap"))?;
        let completion = link.client.complete(prompt)?;
        crate::util::log_link_usage(store, link, "ask", completion.token_usage);
        findings.push(completion.text);
    }
    Ok(findings)
}

/// [`answer_meeting`]'s result: the answer, its transcript citations, and whether the
/// transcript was narrowed.
#[derive(Debug, Clone, Serialize)]
pub struct MeetingAnswer {
    pub text: String,
    pub citations: Vec<llm::ContextItemRef>,
    pub narrowed: bool,
}

/// The blocking counterpart of [`prepare_meeting_ask`], through the governed chain (not
/// pinned): a provider over its monthly budget is skipped for the next one. The shells use
/// it when the streamed provider's budget is spent; `BudgetSpent` only when every provider is.
pub fn answer_meeting(
    store: &memory::Store,
    meeting_id: i64,
    query: &str,
    ollama_model: &str,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> Result<MeetingAnswer, MeetingAskError> {
    answer_meeting_with(
        store,
        meeting_id,
        query,
        history,
        routing,
        || build_chain_for(llm::TaskKind::Chat, ollama_model, routing, &crate::exclusion::current_gate(store)),
        || build_local_chain(ollama_model, routing),
    )
}

/// A meeting of an excluded client, or a question naming one, is answered by `local_chain`
/// (on this computer) or not at all; any other by `chain`.
pub(crate) fn answer_meeting_with(
    store: &memory::Store,
    meeting_id: i64,
    query: &str,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    chain: impl FnOnce() -> Vec<llm::ChainLink>,
    local_chain: impl FnOnce() -> Vec<llm::ChainLink>,
) -> Result<MeetingAnswer, MeetingAskError> {
    // One read of the filter for the decision and the stamp (fix round 3, T1): a client excluded
    // during the call makes the answer not clean, never clean for it.
    let filter = crate::exclusion::context_filter(store);
    let carries = request_carries(store, &filter, query, history, &RequestOrigin::Meeting { meeting_id });
    // Fails early (no transcript) before anything is spent.
    meeting_ask_prompt(store, meeting_id, query, history, routing, true, llm::Provider::Ollama, None)?;
    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("ask"), now_unix_ms()).map_err(ApiError::from)?;
    let chain = if carries { local_chain() } else { chain() };
    if carries && chain.is_empty() {
        return Err(crate::exclusion::refused().into());
    }
    let chain = apply_budgets(store, chain, false, now_unix_ms()).map_err(ApiError::from)?;
    let policy = routing.google_data_policy();
    // Limited Use: each link gets the prompt it may receive.
    let Some(((completion, built), entry)) = llm::try_chain(&chain, |entry| {
        let may_receive = llm::limited_use::link_may_receive_google_data(entry, policy);
        let prompt_for = |findings: Option<&[String]>| {
            meeting_ask_prompt(store, meeting_id, query, history, routing, may_receive, entry.provider, findings)
                .map_err(|_| llm::LlmError::NotImplemented("meeting ask prompt"))
        };
        let mut built = prompt_for(None)?;
        // Too long for this link's window and the question is about the whole meeting: read
        // every part first, then answer from what each held.
        if let Some(parts) = built.map_parts.take() {
            let findings = gather_findings(store, &cap_config, entry, query, &parts)?;
            built = prompt_for(Some(&findings))?;
        }
        let completion = entry.client.complete(&built.prompt)?;
        Ok((completion, built))
    }) else {
        return Err(ApiError::ProviderFailure.into());
    };
    crate::util::log_link_usage(store, entry, "ask", completion.token_usage);
    record_answer_provenance(store, &completion.text, built.google);
    if carries {
        mark_local_only(store, &completion.text);
    } else {
        let meeting_ref = memory::topics::exclusion::input_ref("meeting", meeting_id);
        mark_clean(store, clean_stamp(&filter, store, &built.context, [meeting_ref], &[query], history), &completion.text);
    }
    let citations = llm::extract_citations(&completion.text, &built.context);
    Ok(MeetingAnswer { text: completion.text, citations, narrowed: built.narrowed })
}

/// A meeting Ask prompt, its citable context, and whether it holds Google data.
struct MeetingAskPrompt {
    context: Vec<llm::ContextItem>,
    prompt: String,
    narrowed: bool,
    /// Whether the answer to this prompt inherits Google data: the meeting's title and
    /// attendee names (Google Calendar text), or earlier answers, were in it.
    google: bool,
    /// Set when the question is about the whole meeting and the transcript does not fit the
    /// target's window: the parts to read one by one (see [`gather_findings`]) before the
    /// prompt is built again with their findings.
    map_parts: Option<MapParts>,
}

/// A transcript split into parts that each fit one prompt, as the segments the prompt would
/// show (labels and title already as Limited Use allows), for reading part by part.
struct MapParts {
    parts: Vec<Vec<memory::MeetingTranscriptSegment>>,
    title: String,
    instructions: String,
}

/// The meeting Ask prompt and its citable context, shared by the streaming and blocking
/// paths. Limited Use: when `may_receive_google` is false and the meeting may hold Google
/// Calendar text, its title is left out and speakers keep their plain labels (their names
/// may come from the calendar's attendees); earlier answers built from Google data are
/// dropped from the history either way the link cannot receive them.
#[allow(clippy::too_many_arguments)]
fn meeting_ask_prompt(
    store: &memory::Store,
    meeting_id: i64,
    query: &str,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    may_receive_google: bool,
    target: llm::Provider,
    findings: Option<&[String]>,
) -> Result<MeetingAskPrompt, MeetingAskError> {
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
    // "Speaker N" becomes its name, a guess marked, as in the summary's transcript.
    let calendar_text = store.meeting_may_hold_google_calendar_data(meeting_id);
    let withhold_calendar_text = calendar_text && !may_receive_google;
    let speakers = if withhold_calendar_text { Vec::new() } else { store.meeting_speakers(meeting_id).unwrap_or_default() };
    let segments: Vec<_> = segments
        .into_iter()
        .map(|mut s| {
            s.speaker_label = memory::speaker_names::transcript_label(&s.speaker_label, &speakers);
            s
        })
        .collect();

    // The budget follows the target's window. A question about the whole meeting that does not
    // fit it is read part by part (`map_parts`); with the findings in hand, the numbered
    // items take what room the findings leave.
    let budget = llm::meeting_ask::context_budget_chars(target);
    let segments = llm::meeting_ask::redact_segments(segments);
    let whole_intent = llm::meeting_ask::is_whole_meeting_intent(query);
    let over_budget = !llm::meeting_ask::fits_budget(&segments, budget);
    let findings_len = findings.map_or(0, llm::meeting_ask::findings_chars);
    let item_budget = if findings_len > 0 { budget.saturating_sub(findings_len).max(budget / 4) } else { budget };
    let parts = (whole_intent && over_budget && findings.is_none())
        .then(|| llm::meeting_ask::chunk_segments(segments.clone(), budget))
        .filter(|p| p.len() > 1);
    let mut context = llm::meeting_ask::assemble_meeting_context(segments, query, item_budget);
    if findings_len > 0 {
        // Every part was read, so what the numbered items leave out is still in the answer.
        context.narrowed = false;
    }
    if withhold_calendar_text {
        for item in &mut context.items {
            if let llm::ContextItem::Transcript(t) = item {
                t.meeting_title = llm::limited_use::WITHHELD_TITLE.to_string();
            }
        }
    }
    // Every segment larger than the budget: nothing citable fits, same as no transcript.
    if context.items.is_empty() {
        return Err(MeetingAskError::NoTranscript);
    }
    let draft = store.get_meeting_note_draft(meeting_id).ok().flatten();
    // The summary was written with the title and invitees in view, so it is withheld with them.
    let summary = if withhold_calendar_text {
        String::new()
    } else {
        draft.as_ref().and_then(|d| d.summary_edited.clone()).or_else(|| meeting.summary_tldr.clone()).unwrap_or_default()
    };
    let notes = draft
        .as_ref()
        .and_then(|d| d.personal_notes.clone())
        .or_else(|| meeting.notes.clone())
        .unwrap_or_default();
    let (history, history_google) = history_for_link(store, history, may_receive_google);
    let title = if withhold_calendar_text { llm::limited_use::WITHHELD_TITLE } else { meeting.title.as_str() };
    let instructions = llm::profile::compose_instructions(&routing.custom_instructions, &routing.profile, &routing.output_language);
    let map_parts = parts.map(|mut parts| {
        if withhold_calendar_text {
            for s in parts.iter_mut().flatten() {
                s.meeting_title = llm::limited_use::WITHHELD_TITLE.to_string();
            }
        }
        MapParts { parts, title: title.to_string(), instructions: instructions.clone() }
    });
    let instructions = match findings {
        Some(findings) => llm::meeting_ask::instructions_with_findings(&instructions, findings),
        None => instructions,
    };
    let prompt = llm::meeting_ask::build_meeting_ask_prompt(&history, query, &context.items, &instructions, title, &summary, &notes);
    Ok(MeetingAskPrompt {
        context: context.items,
        prompt,
        narrowed: context.narrowed,
        google: history_google || (calendar_text && may_receive_google),
        map_parts,
    })
}

/// [`finalize_ask`]'s result: a plain object, not the `{"ok":…}` envelope — there is no
/// failure mode worth distinguishing from the caller's perspective.
#[derive(Debug, Clone, Serialize)]
pub struct FinalizedAsk {
    pub citations: Vec<llm::ContextItemRef>,
    /// What exclusion did to the streamed answer (the prepare's outcome, carried through).
    pub exclusion: ExclusionOutcome,
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
    let exclusion = prepared.as_ref().map_or(ExclusionOutcome::Allowed, |p| p.exclusion);
    let clean_keys = prepared.as_ref().and_then(|p| p.clean_keys.clone());
    if let Some(prepared) = prepared {
        if let Some(history_google) = prepared.history_google {
            record_answer_provenance(store, final_text, history_google || prepared.context.iter().any(|c| c.is_google_user_data()));
        }
        log_usage_for(store, prepared.provider, model, "ask", usage);
    }
    // A stream answered on this computer from an excluded client's material: the answer is
    // marked, so neither its title nor a later turn sends it to a hosted model.
    if exclusion == ExclusionOutcome::LocalOnly {
        mark_local_only(store, final_text);
    } else {
        mark_clean(store, clean_keys, final_text);
    }
    FinalizedAsk { citations, exclusion }
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

    generate_conversation_title_with(
        store,
        query,
        answer,
        routing,
        || build_chain_for(llm::TaskKind::ConversationTitle, ollama_model, routing, &crate::exclusion::current_gate(store)),
        || build_local_chain(ollama_model, routing),
    )
}

/// A first exchange that carries an excluded client is titled by `local_chain` (on this
/// computer), or keeps its placeholder; any other by `chain`.
pub(crate) fn generate_conversation_title_with(
    store: &memory::Store,
    query: &str,
    answer: &str,
    routing: &RoutingConfig,
    chain: impl FnOnce() -> Vec<llm::ChainLink>,
    local_chain: impl FnOnce() -> Vec<llm::ChainLink>,
) -> ApiResult<ConversationTitle> {
    // Also when the answer was made on this computer from an excluded client's material, whatever
    // its words; with no local model the placeholder stays (an error here keeps it).
    let filter = crate::exclusion::context_filter(store);
    // An answer not clean for every client excluded now (made before one was excluded, or on
    // this computer from one's material) is titled locally too (fix round 2, N7).
    let marked = filter.is_active()
        && (store.is_local_only_text(answer).unwrap_or(true)
            || store.is_local_only_text(query).unwrap_or(true)
            || !text_is_clean(store, &filter, answer));
    let carries = marked || crate::exclusion::carries_with(store, &filter, &format!("{query}\n{answer}"), &RequestOrigin::Other);
    let chain = if carries { local_chain() } else { chain() };
    if carries && chain.is_empty() {
        return Err(crate::exclusion::refused());
    }
    let chain = apply_budgets(store, chain, false, now_unix_ms())?;
    let policy = routing.google_data_policy();
    let answer_holds_google = store.answer_may_hold_google_data(answer);
    let ((title, token_usage), entry) = llm::try_chain_with_failures(&chain, |entry| {
        // Limited Use: a link that may not receive Google data titles from the question alone.
        let answer = if answer_holds_google && !llm::limited_use::link_may_receive_google_data(entry, policy) { "" } else { answer };
        llm::generate_conversation_title(entry.client.as_ref(), query, answer, &routing.output_language)
    })
    .map_err(crate::chain_failed)?;
    crate::util::log_link_usage(store, entry, "conversation_title", token_usage);
    Ok(ConversationTitle { title })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Main's Limited Use tests walk a given chain over a given context; here the chain goes
    /// through the exclusion-aware [`answer_with_filter`] (which assembles the same context
    /// itself), so both gates apply to what these tests see.
    fn answer_over_chain(
        store: &memory::Store,
        chain: Vec<llm::ChainLink>,
        query: &str,
        _context: &[llm::ContextItem],
        _label: Option<&str>,
        history: &[llm::ChatTurn],
        routing: &RoutingConfig,
    ) -> ApiResult<llm::ChatAnswer> {
        answer_with_filter(store, crate::exclusion::context_filter(store), query, &RequestOrigin::AskBar, 10, history, routing, || chain, Vec::new)
            .map(|(a, _)| a)
    }

    /// [`generate_conversation_title_with`] over a given chain (see [`answer_over_chain`]).
    fn title_over_chain(
        store: &memory::Store,
        chain: Vec<llm::ChainLink>,
        query: &str,
        answer: &str,
        routing: &RoutingConfig,
    ) -> ApiResult<ConversationTitle> {
        generate_conversation_title_with(store, query, answer, routing, || chain, Vec::new)
    }

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
        store.set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_OTHER).unwrap();
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
    fn a_meeting_ask_names_speakers_and_marks_guesses() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting_with_transcript(&store, &["we agreed on Friday"]);
        store
            .insert_transcript_segments(&[
                memory::NewTranscriptSegment { meeting_id: id, start_cs: 500, speaker_label: "Speaker 1".into(), text: "I send the deck".into() },
                memory::NewTranscriptSegment { meeting_id: id, start_cs: 600, speaker_label: "Speaker 2".into(), text: "I review it".into() },
            ])
            .unwrap();
        store.upsert_person("name:sam", "Sam", None).unwrap();
        store.upsert_person("ana@x.com", "Ana", Some("ana@x.com")).unwrap();
        store.set_meeting_speaker(id, 0, Some("name:sam"), Some(memory::speaker_names::NameStatus::Guess)).unwrap();
        store.set_meeting_speaker(id, 1, Some("ana@x.com"), Some(memory::speaker_names::NameStatus::Confirmed)).unwrap();

        let data = prepare_meeting_ask(&store, id, "who sends it?", &[], &RoutingConfig::default()).unwrap();
        assert!(data.prompt.contains("Sam (unconfirmed)) I send the deck"), "{}", data.prompt);
        assert!(data.prompt.contains("Ana) I review it"), "{}", data.prompt);
        assert!(data.prompt.contains("Ben) we agreed on Friday"), "other labels stay: {}", data.prompt);
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
        let huge = "x".repeat(llm::meeting_ask::context_budget_chars(llm::Provider::Ollama) + 1);
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
            exclusion: ExclusionOutcome::FilteredHosted { left_out: 2 },
        };
        assert_eq!(
            serde_json::to_string(&data).unwrap(),
            r#"{"prepare_id":"id","prompt":"p","exclusion":{"kind":"filtered_hosted","left_out":2}}"#
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
        for (name, provider) in [("anthropic", llm::Provider::Anthropic), ("openai", llm::Provider::OpenAI), ("whimbrel", llm::Provider::Whimbrel)] {
            routing.preferred_provider = name.into();
            assert_eq!(provider_for_streamed_answer(&routing), provider, "{name} spend is logged to {name}");
            // Whimbrel has no local budget: server credit is its limit.
            let store = memory::Store::open_in_memory().unwrap();
            assert!(check_provider(&store, provider, now_unix_ms()).is_ok(), "{name}");
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

    /// Streaming Ask is pinned to the one provider the shell streams against: a spent
    /// budget fails the prepare call, stashes nothing, and never falls through to Ollama.
    #[test]
    fn streaming_ask_provider_checked() {
        let _serial = serialize_prepared_asks();
        let store = store_with_capture_text("the quarterly roadmap review");
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                route: Some("ask".into()),
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: 2500,
                cache_hit: false,
                trace_id: "t".into(),
                ts_unix_ms: now_unix_ms(),
            })
            .unwrap();
        store.set_provider_budgets(&[("anthropic".into(), 2500)]).unwrap();
        let routing = RoutingConfig {
            anthropic_key: Some("k".into()),
            preferred_provider: "anthropic".into(),
            user_daily_cap_cents: Some(0),
            global_daily_cap_cents: Some(0),
            ..RoutingConfig::default()
        };
        let before = prepared_asks().lock().unwrap().len();
        let result = prepare_ask(&store, "roadmap", 5, &[], &routing);
        let Err(ApiError::BudgetSpent(spent)) = result else { panic!("{result:?}") };
        assert_eq!(spent.provider, llm::Provider::Anthropic);
        assert_eq!(prepared_asks().lock().unwrap().len(), before);

        // Streaming another provider is untouched by anthropic's spent budget (still set),
        // and by its own unspent one.
        store.set_provider_budgets(&[("anthropic".into(), 2500), ("openai".into(), 1)]).unwrap();
        let other = RoutingConfig { openai_key: Some("k".into()), preferred_provider: "openai".into(), ..routing.clone() };
        let prepared = prepare_ask(&store, "roadmap", 5, &[], &other).unwrap();
        discard_ask(&prepared.prepare_id);
    }

    /// A Gmail thread about the Falcon budget, and a screen capture about it.
    fn store_with_gmail_and_capture() -> memory::Store {
        let store = store_with_capture_text("Falcon budget spreadsheet on screen");
        store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "m-1".into(),
                ts_unix_ms: now_unix_ms() - 1_000,
                from: "Priya <priya@x.com>".into(),
                to: "me@x.com".into(),
                subject: "Falcon".into(),
                body_text: "GMAILSECRET the Falcon budget is 40k".into(),
                gmail_thread_id: "thread-1".into(),
                account_email: "me@x.com".into(),
                ..Default::default()
            }])
            .unwrap();
        while store.run_thread_state_sync_job(now_unix_ms()).unwrap() {}
        store
    }

    /// Records every prompt it is sent, then fails so the walk visits every link.
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl llm::LlmClient for Recorder {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            self.0.lock().unwrap().push(prompt.to_string());
            Err(llm::LlmError::Network("next".into()))
        }
    }

    /// The prompt each of `links` receives for one Ask over the Gmail store.
    fn prompts_per_link(links: Vec<llm::ChainLink>, routing: &RoutingConfig) -> Vec<String> {
        let store = store_with_gmail_and_capture();
        let (context, label) = assemble_ask_context(&store, "Falcon budget", 10, &llm::ContextFilter::none());
        assert!(context.iter().any(|c| c.is_google_user_data()), "the fixture reaches Ask through its Gmail thread");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let chain: Vec<llm::ChainLink> = links
            .into_iter()
            .map(|l| llm::ChainLink { client: Box::new(Recorder(seen.clone())), ..l })
            .collect();
        let _ = answer_over_chain(&store, chain, "Falcon budget", &context, label.as_deref(), &[], routing);
        let out = seen.lock().unwrap().clone();
        out
    }

    fn link(provider: llm::Provider, model: &str) -> llm::ChainLink {
        llm::ChainLink { provider, model: model.into(), client: Box::new(Fake) }
    }

    /// Limited Use, per link: the chain falls back between providers, so each link's prompt
    /// holds Gmail text only when that link may receive it; screen text reaches every link.
    #[test]
    fn each_link_gets_gmail_text_only_when_it_may_receive_it() {
        let links = || {
            vec![
                link(llm::Provider::Google, "gemini-2.5-flash"),
                link(llm::Provider::Anthropic, "claude"),
                link(llm::Provider::Ollama, "gpt-oss:120b-cloud"),
                link(llm::Provider::Ollama, "gemma4:12b"),
                link(llm::Provider::OpenRouter, "vendor/m"),
                link(llm::Provider::OpenAI, "gpt"),
                link(llm::Provider::Whimbrel, "claude-sonnet-5-5"),
            ]
        };
        let prompts = prompts_per_link(links(), &RoutingConfig::default());
        assert_eq!(prompts.len(), 7);
        let has_gmail: Vec<bool> = prompts.iter().map(|p| p.contains("GMAILSECRET")).collect();
        assert_eq!(has_gmail, [false, true, false, true, false, true, true]);
        assert!(prompts.iter().all(|p| p.contains("Falcon budget spreadsheet")), "other sources keep flowing");

        crate::routing::set_limited_use_settings(crate::routing::LimitedUseSettings {
            policy: llm::limited_use::GoogleDataPolicy { google_ai_billing_confirmed: true, remote_ollama_google_data: true },
            streamed_ollama_model: None,
            streamed_ollama_base_url: None,
        });
        let has_gmail: Vec<bool> = prompts_per_link(links(), &RoutingConfig::default()).iter().map(|p| p.contains("GMAILSECRET")).collect();
        assert_eq!(has_gmail, [true, true, true, true, false, true, true], "OpenRouter never gets it");
    }

    /// A streamed Ask is built for the one provider the shell streams against.
    #[test]
    fn a_streamed_ask_holds_gmail_text_only_for_a_provider_that_may_receive_it() {
        let _serial = serialize_prepared_asks();
        let store = store_with_gmail_and_capture();
        let prompt_for = |routing: &RoutingConfig| {
            let data = prepare_ask(&store, "Falcon budget", 10, &[], routing).unwrap();
            discard_ask(&data.prepare_id);
            data.prompt
        };
        let google = RoutingConfig { google_key: Some("k".into()), preferred_provider: "google".into(), ..RoutingConfig::default() };
        let p = prompt_for(&google);
        assert!(!p.contains("GMAILSECRET") && p.contains("Falcon budget spreadsheet"), "{p}");
        let settings = |billing: bool, model: Option<&str>| {
            crate::routing::set_limited_use_settings(crate::routing::LimitedUseSettings {
                policy: llm::limited_use::GoogleDataPolicy { google_ai_billing_confirmed: billing, remote_ollama_google_data: false },
                streamed_ollama_model: model.map(String::from),
                streamed_ollama_base_url: None,
            })
        };
        settings(true, None);
        assert!(prompt_for(&google).contains("GMAILSECRET"));
        settings(false, None);
        let anthropic = RoutingConfig { anthropic_key: Some("k".into()), preferred_provider: "anthropic".into(), ..RoutingConfig::default() };
        assert!(prompt_for(&anthropic).contains("GMAILSECRET"));
        let openrouter = RoutingConfig { openrouter_key: Some("k".into()), preferred_provider: "openrouter".into(), ..RoutingConfig::default() };
        assert!(!prompt_for(&openrouter).contains("GMAILSECRET"));
        // Ollama: the model the shell streams decides, not `preferred_model`; unknown is closed.
        let ollama = RoutingConfig { preferred_provider: "ollama".into(), preferred_model: "gemma4:12b".into(), ..RoutingConfig::default() };
        assert!(!prompt_for(&ollama).contains("GMAILSECRET"), "streamed model unknown");
        settings(false, Some("gpt-oss:120b-cloud"));
        assert!(!prompt_for(&ollama).contains("GMAILSECRET"));
        settings(false, Some("gemma4:12b"));
        assert!(prompt_for(&ollama).contains("GMAILSECRET"));
    }

    /// The race the per-call model closes: a background call's routing JSON (naming the
    /// local model) lands between two Asks streamed against a cloud model. Each Ask is
    /// decided by its own JSON, so the cloud ones never hold Gmail text.
    #[test]
    fn interleaved_calls_with_different_streamed_models_never_give_the_cloud_one_gmail() {
        let _serial = serialize_prepared_asks();
        let store = store_with_gmail_and_capture();
        let ollama = RoutingConfig { preferred_provider: "ollama".into(), ..RoutingConfig::default() };
        let cloud_json = r#"{"preferred_provider":"ollama","streamed_ollama_model":"gpt-oss:120b-cloud"}"#;
        let local_json = r#"{"preferred_provider":"ollama","streamed_ollama_model":"gemma4:12b"}"#;
        let prompt_for = |json: &str| {
            let routing = RoutingConfig::from_json(Some(json));
            let call = crate::routing::LimitedUseSettings::from_routing_json_str(Some(json));
            let data = prepare_ask_for(&store, "Falcon budget", 10, &[], &routing, &call).unwrap();
            discard_ask(&data.prepare_id);
            data.prompt
        };
        for _ in 0..3 {
            assert!(!prompt_for(cloud_json).contains("GMAILSECRET"));
            assert!(prompt_for(local_json).contains("GMAILSECRET"));
            RoutingConfig::from_json(Some(local_json)); // a background call
            assert!(!prompt_for(cloud_json).contains("GMAILSECRET"));
        }
        // A caller without per-call settings still fails closed: nothing names the model.
        let data = prepare_ask(&store, "Falcon budget", 10, &[], &ollama).unwrap();
        discard_ask(&data.prepare_id);
        assert!(!data.prompt.contains("GMAILSECRET"));
    }

    struct Fake;
    impl llm::LlmClient for Fake {
        fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            Ok(llm::Completion { text: "Friday [1].".into(), token_usage: llm::TokenUsage::default() })
        }
    }

    fn fake_chain(providers: &[llm::Provider]) -> Vec<llm::ChainLink> {
        providers.iter().map(|&provider| llm::ChainLink { provider, model: "m".into(), client: Box::new(Fake) }).collect()
    }

    /// The blocking meeting Ask is not pinned: a spent provider is skipped for the next one,
    /// and only an all-spent chain fails with `BudgetSpent`.
    #[test]
    fn a_blocking_meeting_ask_falls_back_past_a_spent_budget() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting_with_transcript(&store, &["we agreed on Friday"]);
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                route: Some("ask".into()),
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: 2500,
                cache_hit: false,
                trace_id: "t".into(),
                ts_unix_ms: now_unix_ms(),
            })
            .unwrap();
        store.set_provider_budgets(&[("anthropic".into(), 2500)]).unwrap();
        let routing = RoutingConfig {
            preferred_provider: "anthropic".into(),
            user_daily_cap_cents: Some(0),
            global_daily_cap_cents: Some(0),
            ..RoutingConfig::default()
        };

        let answer = answer_meeting_with(&store, id, "when?", &[], &routing, || {
            fake_chain(&[llm::Provider::Anthropic, llm::Provider::OpenAI])
        }, Vec::new)
        .unwrap();
        assert_eq!(answer.text, "Friday [1].");
        assert_eq!(answer.citations.len(), 1);

        let result = answer_meeting_with(&store, id, "when?", &[], &routing, || fake_chain(&[llm::Provider::Anthropic]), Vec::new);
        let Err(MeetingAskError::Api(ApiError::BudgetSpent(spent))) = result else { panic!("{result:?}") };
        assert_eq!(spent.provider, llm::Provider::Anthropic);
    }

    // --- Limited Use: answers built from Google data in later turns and titles ---

    const GMAIL_ANSWER: &str = "ANSWERFROMGMAIL the budget is 40k";
    const CLEAN_ANSWER: &str = "ANSWERFROMSCREEN the demo is on Friday";
    const UNKNOWN_ANSWER: &str = "ANSWERUNKNOWN written elsewhere";

    fn turn(role: llm::ChatRole, text: &str) -> llm::ChatTurn {
        llm::ChatTurn { role, text: text.into() }
    }

    /// Three earlier exchanges: one answered from Gmail, one from screen text, one whose
    /// provenance was never recorded.
    fn history_of_three(store: &memory::Store) -> Vec<llm::ChatTurn> {
        store.record_answer_provenance(GMAIL_ANSWER, true, 1).unwrap();
        store.record_answer_provenance(CLEAN_ANSWER, false, 1).unwrap();
        use llm::ChatRole::{Assistant, User};
        vec![
            turn(User, "QGMAIL"),
            turn(Assistant, GMAIL_ANSWER),
            turn(User, "QCLEAN"),
            turn(Assistant, CLEAN_ANSWER),
            turn(User, "QUNKNOWN"),
            turn(Assistant, UNKNOWN_ANSWER),
        ]
    }

    #[test]
    fn earlier_answers_built_from_google_data_reach_only_links_that_may_receive_it() {
        let store = store_with_gmail_and_capture();
        let history = history_of_three(&store);
        let (context, label) = assemble_ask_context(&store, "Falcon budget", 10, &llm::ContextFilter::none());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let chain: Vec<llm::ChainLink> = [(llm::Provider::OpenRouter, "vendor/m"), (llm::Provider::Anthropic, "claude")]
            .into_iter()
            .map(|(provider, model)| llm::ChainLink { provider, model: model.into(), client: Box::new(Recorder(seen.clone())) })
            .collect();
        let _ = answer_over_chain(&store, chain, "Falcon budget", &context, label.as_deref(), &history, &RoutingConfig::default());
        let prompts = seen.lock().unwrap().clone();
        let (restricted, allowed) = (&prompts[0], &prompts[1]);
        for text in [GMAIL_ANSWER, UNKNOWN_ANSWER, "QGMAIL", "QUNKNOWN"] {
            assert!(!restricted.contains(text), "{text} reached OpenRouter");
            assert!(allowed.contains(text), "{text} is held back from a link that may receive it");
        }
        assert!(restricted.contains(CLEAN_ANSWER) && restricted.contains("QCLEAN"), "a clean exchange stays");
    }

    #[test]
    fn a_streamed_ask_holds_earlier_google_answers_for_a_provider_that_may_not_receive_them() {
        let _serial = serialize_prepared_asks();
        let store = store_with_gmail_and_capture();
        let history = history_of_three(&store);
        let prompt_for = |routing: &RoutingConfig| {
            let data = prepare_ask(&store, "Falcon budget", 10, &history, routing).unwrap();
            discard_ask(&data.prepare_id);
            data.prompt
        };
        let openrouter = RoutingConfig { openrouter_key: Some("k".into()), preferred_provider: "openrouter".into(), ..RoutingConfig::default() };
        let p = prompt_for(&openrouter);
        assert!(!p.contains(GMAIL_ANSWER) && !p.contains(UNKNOWN_ANSWER), "{p}");
        assert!(p.contains(CLEAN_ANSWER));
        let anthropic = RoutingConfig { anthropic_key: Some("k".into()), preferred_provider: "anthropic".into(), ..RoutingConfig::default() };
        let p = prompt_for(&anthropic);
        assert!(p.contains(GMAIL_ANSWER) && p.contains(UNKNOWN_ANSWER) && p.contains(CLEAN_ANSWER));
    }

    #[test]
    fn an_answer_is_recorded_as_google_when_the_context_it_was_built_from_was() {
        // Blocking: Gmail in the context the answering link received.
        let store = store_with_gmail_and_capture();
        let (context, label) = assemble_ask_context(&store, "Falcon budget", 10, &llm::ContextFilter::none());
        let chain = fake_chain(&[llm::Provider::Anthropic]);
        answer_over_chain(&store, chain, "Falcon budget", &context, label.as_deref(), &[], &RoutingConfig::default()).unwrap();
        assert!(store.answer_may_hold_google_data("Friday [1]."));

        // Only screen text: recorded clean, so it keeps flowing into later turns.
        let store = store_with_capture_text("Falcon budget spreadsheet on screen");
        let (context, label) = assemble_ask_context(&store, "Falcon budget", 10, &llm::ContextFilter::none());
        assert!(context.iter().all(|c| !c.is_google_user_data()));
        answer_over_chain(&store, fake_chain(&[llm::Provider::Anthropic]), "Falcon budget", &context, label.as_deref(), &[], &RoutingConfig::default()).unwrap();
        assert!(!store.answer_may_hold_google_data("Friday [1]."));

        // A clean question that carries a Google answer in its history is Google too.
        store.record_answer_provenance(GMAIL_ANSWER, true, 1).unwrap();
        let history = [turn(llm::ChatRole::User, "q"), turn(llm::ChatRole::Assistant, GMAIL_ANSWER)];
        answer_over_chain(&store, fake_chain(&[llm::Provider::Anthropic]), "Falcon budget", &context, label.as_deref(), &history, &RoutingConfig::default()).unwrap();
        assert!(store.answer_may_hold_google_data("Friday [1]."));
    }

    #[test]
    fn a_streamed_answer_is_recorded_when_it_is_finalized() {
        let _serial = serialize_prepared_asks();
        let store = store_with_gmail_and_capture();
        let anthropic = RoutingConfig { anthropic_key: Some("k".into()), preferred_provider: "anthropic".into(), ..RoutingConfig::default() };
        let data = prepare_ask(&store, "Falcon budget", 10, &[], &anthropic).unwrap();
        assert!(data.prompt.contains("GMAILSECRET"));
        finalize_ask(&store, &data.prepare_id, "STREAMEDGMAIL answer", "claude", llm::TokenUsage::default());
        assert!(store.answer_may_hold_google_data("STREAMEDGMAIL answer"));

        // A prompt the provider could not hold Gmail text in produces a clean answer.
        let openrouter = RoutingConfig { openrouter_key: Some("k".into()), preferred_provider: "openrouter".into(), ..RoutingConfig::default() };
        let data = prepare_ask(&store, "Falcon budget", 10, &[], &openrouter).unwrap();
        finalize_ask(&store, &data.prepare_id, "STREAMEDCLEAN answer", "m", llm::TokenUsage::default());
        assert!(!store.answer_may_hold_google_data("STREAMEDCLEAN answer"));
    }

    #[test]
    fn a_title_link_that_may_not_receive_google_data_gets_the_question_only() {
        let store = memory::Store::open_in_memory().unwrap();
        store.record_answer_provenance(GMAIL_ANSWER, true, 1).unwrap();
        store.record_answer_provenance(CLEAN_ANSWER, false, 1).unwrap();
        let titles = |answer: &str| {
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let chain: Vec<llm::ChainLink> = [(llm::Provider::OpenRouter, "vendor/m"), (llm::Provider::Anthropic, "claude")]
                .into_iter()
                .map(|(provider, model)| llm::ChainLink { provider, model: model.into(), client: Box::new(Recorder(seen.clone())) })
                .collect();
            let _ = title_over_chain(&store, chain, "QTITLE", answer, &RoutingConfig::default());
            let prompts = seen.lock().unwrap().clone();
            prompts
        };
        for answer in [GMAIL_ANSWER, UNKNOWN_ANSWER] {
            let p = titles(answer);
            assert!(!p[0].contains(answer) && p[0].contains("QTITLE"), "{answer} reached OpenRouter");
            assert!(p[1].contains(answer));
        }
        let p = titles(CLEAN_ANSWER);
        assert!(p[0].contains(CLEAN_ANSWER) && p[1].contains(CLEAN_ANSWER));
    }

    // --- Limited Use: meeting Ask ---

    /// A calendar meeting whose source is unknown (Google Calendar text), titled
    /// "Zebra Review", with a speaker named from its attendees.
    fn google_meeting(store: &memory::Store) -> i64 {
        let id = meeting_with_transcript(store, &["we agreed on Friday"]);
        store.set_meeting_title(id, "Zebra Review").unwrap();
        store.set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_GOOGLE).unwrap();
        store.update_meeting_summary(id, "ZSUMMARY about the Zebra", &[], &[], &[], "p", "m").unwrap();
        store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id: id,
                start_cs: 500,
                speaker_label: "Speaker 1".into(),
                text: "I send the deck".into(),
            }])
            .unwrap();
        store.upsert_person("ana@x.com", "Ana Zebrowski", Some("ana@x.com")).unwrap();
        store.set_meeting_speaker(id, 0, Some("ana@x.com"), Some(memory::speaker_names::NameStatus::Confirmed)).unwrap();
        id
    }

    #[test]
    fn a_streamed_meeting_ask_leaves_out_google_calendar_text_for_a_restricted_provider() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let id = google_meeting(&store);
        let history = history_of_three(&store);
        let prompt_for = |routing: &RoutingConfig| {
            let data = prepare_meeting_ask(&store, id, "who sends it?", &history, routing).unwrap();
            discard_ask(&data.prepare_id);
            data.prompt
        };
        let openrouter = RoutingConfig { openrouter_key: Some("k".into()), preferred_provider: "openrouter".into(), ..RoutingConfig::default() };
        let p = prompt_for(&openrouter);
        for text in ["Zebra Review", "Zebrowski", "ZSUMMARY", GMAIL_ANSWER, UNKNOWN_ANSWER] {
            assert!(!p.contains(text), "{text} reached OpenRouter: {p}");
        }
        assert!(p.contains("I send the deck") && p.contains(CLEAN_ANSWER), "{p}");
        let anthropic = RoutingConfig { anthropic_key: Some("k".into()), preferred_provider: "anthropic".into(), ..RoutingConfig::default() };
        let p = prompt_for(&anthropic);
        for text in ["Zebra Review", "Ana Zebrowski", "ZSUMMARY", GMAIL_ANSWER] {
            assert!(p.contains(text), "{text} missing: {p}");
        }
    }

    #[test]
    fn a_blocking_meeting_ask_builds_each_links_prompt_for_that_link() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = google_meeting(&store);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let chain: Vec<llm::ChainLink> = [(llm::Provider::OpenRouter, "vendor/m"), (llm::Provider::Anthropic, "claude")]
            .into_iter()
            .map(|(provider, model)| llm::ChainLink { provider, model: model.into(), client: Box::new(Recorder(seen.clone())) })
            .collect();
        let _ = answer_meeting_with(&store, id, "who sends it?", &[], &RoutingConfig::default(), || chain, Vec::new);
        let prompts = seen.lock().unwrap().clone();
        assert_eq!(prompts.len(), 2);
        assert!(!prompts[0].contains("Zebra Review") && !prompts[0].contains("Zebrowski"));
        assert!(prompts[1].contains("Zebra Review") && prompts[1].contains("Ana Zebrowski"));
    }

    #[test]
    fn meeting_ask_answers_are_recorded() {
        let _serial = serialize_prepared_asks();
        let anthropic = RoutingConfig { anthropic_key: Some("k".into()), preferred_provider: "anthropic".into(), ..RoutingConfig::default() };
        // Google Calendar text in the prompt: the answer is Google.
        let store = memory::Store::open_in_memory().unwrap();
        let id = google_meeting(&store);
        let data = prepare_meeting_ask(&store, id, "q", &[], &anthropic).unwrap();
        finalize_ask(&store, &data.prepare_id, "MEETINGANSWER one", "claude", llm::TokenUsage::default());
        assert!(store.answer_may_hold_google_data("MEETINGANSWER one"));
        // A meeting known to come from another calendar: recorded clean.
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting_with_transcript(&store, &["we agreed on Friday"]);
        let data = prepare_meeting_ask(&store, id, "q", &[], &anthropic).unwrap();
        finalize_ask(&store, &data.prepare_id, "MEETINGANSWER two", "claude", llm::TokenUsage::default());
        assert!(!store.answer_may_hold_google_data("MEETINGANSWER two"));
        // Blocking.
        let a = answer_meeting_with(&store, id, "q", &[], &anthropic, || fake_chain(&[llm::Provider::Anthropic]), Vec::new).unwrap();
        assert!(!store.answer_may_hold_google_data(&a.text));
    }

    // --- Meeting Ask: budget follows the model, whole-meeting questions read it all ---

    struct Seen(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl llm::LlmClient for Seen {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            self.0.lock().unwrap().push(prompt.to_string());
            Ok(llm::Completion { text: "Friday [1].".into(), token_usage: llm::TokenUsage::default() })
        }
    }

    fn seen_chain(provider: llm::Provider) -> (Vec<llm::ChainLink>, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let link = llm::ChainLink { provider, model: "m".into(), client: Box::new(Seen(seen.clone())) };
        (vec![link], seen)
    }

    /// `n` lines of 1,000 characters each, line `i` carrying the marker `LINE{i}`.
    fn long_lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("LINE{i} {}", "x".repeat(990))).collect()
    }

    fn long_meeting(store: &memory::Store, n: usize) -> i64 {
        let lines = long_lines(n);
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        meeting_with_transcript(store, &refs)
    }

    #[test]
    fn a_hosted_model_gets_the_whole_transcript_for_a_whole_meeting_question_in_one_call() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = long_meeting(&store, 40); // 40k characters: over a local model's budget
        let (chain, seen) = seen_chain(llm::Provider::Anthropic);
        let answer =
            answer_meeting_with(&store, id, "list the action items", &[], &RoutingConfig::default(), || chain, Vec::new).unwrap();
        assert!(!answer.narrowed);
        let prompts = seen.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("LINE0 ") && prompts[0].contains("LINE39 "), "every segment is sent");
    }

    #[test]
    fn a_local_model_reads_a_long_transcript_part_by_part_for_a_whole_meeting_question() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = long_meeting(&store, 40);
        let (chain, seen) = seen_chain(llm::Provider::Ollama);
        let answer =
            answer_meeting_with(&store, id, "list the action items", &[], &RoutingConfig::default(), || chain, Vec::new).unwrap();
        let prompts = seen.lock().unwrap();
        // Two parts read, then the answer.
        assert_eq!(prompts.len(), 3, "{}", prompts.len());
        assert!(prompts[0].contains("part 1 of 2") && prompts[1].contains("part 2 of 2"));
        assert!(prompts[0].contains("LINE0 ") && !prompts[0].contains("LINE39 "));
        assert!(prompts[1].contains("LINE39 ") && !prompts[1].contains("LINE0 "));
        // Together the parts hold every segment.
        for i in 0..40 {
            let marker = format!("LINE{i} ");
            assert!(prompts[..2].iter().any(|p| p.contains(&marker)), "{marker} dropped");
        }
        assert!(prompts[2].contains("Part 1:\nFriday [1].") && prompts[2].contains("Question: list the action items"));
        assert!(!answer.narrowed, "every part was read");
    }

    #[test]
    fn a_narrow_question_on_a_long_transcript_keeps_the_keyword_selection_in_one_call() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = long_meeting(&store, 40);
        let (chain, seen) = seen_chain(llm::Provider::Ollama);
        let answer = answer_meeting_with(&store, id, "what about LINE7", &[], &RoutingConfig::default(), || chain, Vec::new).unwrap();
        assert!(answer.narrowed);
        let prompts = seen.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("LINE7 "));
    }

    #[test]
    fn a_long_transcript_that_fits_is_one_call_whatever_the_question() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = long_meeting(&store, 30); // 30k: within a local model's budget
        let (chain, seen) = seen_chain(llm::Provider::Ollama);
        answer_meeting_with(&store, id, "summarize", &[], &RoutingConfig::default(), || chain, Vec::new).unwrap();
        let prompts = seen.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("LINE0 ") && prompts[0].contains("LINE29 "));
    }

    #[test]
    fn the_parts_a_link_may_not_receive_google_text_in_carry_no_title() {
        let store = memory::Store::open_in_memory().unwrap();
        // 100k characters: over OpenRouter's budget, so a whole-meeting question reads it in parts.
        let id = long_meeting(&store, 100);
        store.set_meeting_title(id, "Zebra Review").unwrap();
        store.set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_GOOGLE).unwrap();
        let (chain, seen) = seen_chain(llm::Provider::OpenRouter);
        let _ = answer_meeting_with(&store, id, "list the action items", &[], &RoutingConfig::default(), || chain, Vec::new).unwrap();
        let prompts = seen.lock().unwrap();
        assert!(prompts.len() >= 3, "{}", prompts.len());
        assert!(prompts.iter().all(|p| !p.contains("Zebra")), "the title reached a link that may not receive it");
    }

    /// Merge 3: reading a transcript part by part on a hosted link checks every part against the
    /// exclusion gate before the first is sent, so a part naming an excluded client late in the
    /// transcript stops the reading before anything leaves (the guard alone would refuse only that
    /// part, after the earlier ones went). An on-device link is not held back.
    #[test]
    fn gather_findings_checks_every_part_before_the_first_hosted_call() {
        let store = memory::Store::open_in_memory().unwrap();
        let acme = store.create_seed_topic("Acme", &[], None).unwrap();
        store
            .set_client_control(&acme.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        let segment = |text: &str| memory::MeetingTranscriptSegment {
            id: 1,
            meeting_id: 1,
            meeting_title: "Planning".into(),
            start_cs: 0,
            speaker_label: "Ben".into(),
            text: text.into(),
        };
        let parts = MapParts {
            parts: vec![vec![segment("the budget plan")], vec![segment("and then Acme came up")]],
            title: "Planning".into(),
            instructions: String::new(),
        };
        let cap = llm::SpendCapConfig::default();
        let (link, seen) = seen_chain(llm::Provider::OpenRouter);
        let link = llm::guard_hosted(link.into_iter().next().unwrap(), crate::exclusion::current_gate(&store));
        let r = gather_findings(&store, &cap, &link, "list the action items", &parts);
        assert!(matches!(r, Err(llm::LlmError::Refused(_))), "{r:?}");
        assert_eq!(seen.lock().unwrap().len(), 0, "no part was sent");
        // On this computer every part is read.
        let (local, seen) = seen_chain(llm::Provider::Ollama);
        let r = gather_findings(&store, &cap, &local[0], "list the action items", &parts);
        assert_eq!(r.unwrap().len(), 2);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }
}
