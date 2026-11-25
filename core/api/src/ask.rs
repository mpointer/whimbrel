//! FR-CHAT: grounded chat — the blocking `answer`, the streaming `prepare_ask` /
//! `finalize_ask` / `discard_ask` split (#74), and conversation titles (FR-CHAT-6).
//! Moved from `core/ffi`'s `whimbrel_llm_answer` family (M3.0); the C functions are thin
//! wrappers. Argument decoding (`history_json`, the `-1` token sentinels) stays in `ffi`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

use memory::cards::{SavedSource, SourceCard};
use memory::find::FindFilter;

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

/// [`question_names_excluded_client`] for a question asked with `scope`: the filter's person is part of what the
/// question names (asking about a person is asking about them), as the Ask's own routing counts it.
pub fn question_names_excluded_client_scoped(store: &memory::Store, query: &str, history: &[llm::ChatTurn], scope: &AskScope) -> bool {
    let filter = crate::exclusion::context_filter(store);
    request_carries(store, &filter, &scope.carry_text(query), history, &RequestOrigin::AskBar)
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
    answer_scoped_by(store, filter, query, origin, context_limit, history, routing, chain, local_chain, &AskScope::default())
        .map(|(answer, outcome, by, _, _)| (answer, outcome, by))
}

/// How a sourced Ask is bounded and where it may run.
#[derive(Debug, Clone, Default)]
pub struct AskScope {
    /// Date, app and person bounds on every retrieval source. Empty means no bound: retrieval is then
    /// exactly the unbounded call's.
    pub filter: FindFilter,
    /// Answer on this computer or fail: the call never reaches a hosted model, whatever the routing
    /// says. A question that names an excluded client is answered locally either way.
    pub require_local: bool,
}

impl AskScope {
    /// The words that decide whether a request carries an excluded client: the question, and the person
    /// the filter names (asking about one is asking about them). The question alone when no person is set.
    fn carry_text<'a>(&self, query: &'a str) -> std::borrow::Cow<'a, str> {
        match self.filter.clone().normalized().person {
            Some(p) => std::borrow::Cow::Owned(format!("{query}\n{}", p.name)),
            None => std::borrow::Cow::Borrowed(query),
        }
    }
}

/// The chain a blocking Ask walks, and whether it was chosen because the request carries an excluded
/// client. The one place the blocking route is decided: [`answer_scoped_by`] and the route preview both call it.
pub(crate) struct BlockingPlan {
    pub(crate) chain: Vec<llm::ChainLink>,
    /// The request carries an excluded client, so only on-device links were considered.
    pub(crate) rerouted: bool,
}

/// The spend cap, then the links this request may use: on-device only for a request that carries an
/// excluded client (refused when none), or when `require_local`; routing's own chain otherwise. Links over
/// their monthly budget are dropped; `Err` when none is left.
pub(crate) fn plan_blocking(
    store: &memory::Store,
    routing: &RoutingConfig,
    carries: bool,
    require_local: bool,
    chain: impl FnOnce() -> Vec<llm::ChainLink>,
    local_chain: impl FnOnce() -> Vec<llm::ChainLink>,
) -> ApiResult<BlockingPlan> {
    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("ask"), now_unix_ms())?;
    if carries {
        let chain = apply_budgets(store, local_chain(), false, now_unix_ms())?;
        if chain.is_empty() {
            return Err(crate::exclusion::refused());
        }
        return Ok(BlockingPlan { chain, rerouted: true });
    }
    // In local-only mode the routing's own chain is already on-device links only (and ignores the "use the
    // local model" switch, as every Ask does in that mode), so `require_local` changes nothing there.
    let links = if require_local && !routing.sensitive { local_chain() } else { chain() };
    Ok(BlockingPlan { chain: apply_budgets(store, links, false, now_unix_ms())?, rerouted: false })
}

/// [`answer_with_filter_by`] bounded by `scope`, and it also returns the references of the items in the
/// context of the link that answered, in prompt order: number `n` is `refs[n - 1]`, and the date range
/// the retrieval used.
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_scoped_by(
    store: &memory::Store,
    filter: llm::ContextFilter,
    query: &str,
    origin: &RequestOrigin,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    chain: impl FnOnce() -> Vec<llm::ChainLink>,
    local_chain: impl FnOnce() -> Vec<llm::ChainLink>,
    scope: &AskScope,
) -> ApiResult<(llm::ChatAnswer, ExclusionOutcome, AnsweredBy, Vec<llm::ContextItemRef>, Option<UsedRange>)> {
    let carries = request_carries(store, &filter, &scope.carry_text(query), history, origin);

    let plan = plan_blocking(store, routing, carries, scope.require_local, chain, local_chain)?;
    // The person the filter names is resolved once for this Ask, whichever contexts it reads.
    let bound = store.scope_ctx(&scope.filter).map_err(read_failed)?;

    let instructions = llm::profile::compose_instructions(&routing.custom_instructions, &routing.profile, &routing.output_language);
    let policy = routing.google_data_policy();
    if plan.rerouted {
        let chain = plan.chain;
        let open = llm::ContextFilter::none();
        let (context, label, range) = assemble_ask_context_scoped(store, query, context_limit, &open, &bound)?;
        let ((answer, refs), entry) = llm::try_chain_with_failures(&chain, |entry| {
            // Limited Use applies on this computer too (an Ollama host off loopback).
            let may_receive = llm::limited_use::link_may_receive_google_data(entry, policy);
            let context = llm::limited_use::filter_context_in(store, &context, may_receive);
            let (history, history_google) = history_for_link(store, history, may_receive);
            let answer = llm::answer_with_citations(entry.client.as_ref(), &history, query, &context, &instructions, label.as_deref())?;
            record_answer_provenance(store, &answer.text, history_google || context.iter().any(|c| c.is_google_user_data()));
            Ok((answer, context.iter().map(llm::ContextItem::citation_ref).collect::<Vec<_>>()))
        })
        .map_err(crate::chain_failed)?;
        crate::util::log_link_usage(store, entry, "ask", answer.token_usage);
        mark_local_only(store, &answer.text);
        return Ok((answer, ExclusionOutcome::LocalOnly, AnsweredBy::of(entry), refs, range));
    }

    let chain = plan.chain;
    let hosted = crate::exclusion::chain_is_hosted(&chain);
    // Hosted links answer from memory without excluded material; on-device links from all of it.
    let open = llm::ContextFilter::none();
    let (hosted_context, label, range) = assemble_ask_context_scoped(store, query, context_limit, if hosted { &filter } else { &open }, &bound)?;
    let local_context = if hosted && filter.is_active() && chain.iter().any(|l| !llm::is_hosted(l.provider, &l.model)) {
        Some(assemble_ask_context_scoped(store, query, context_limit, &open, &bound)?.0)
    } else {
        None
    };
    let ((answer, refs), entry) = llm::try_chain_with_failures(&chain, |entry| {
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
        Ok((answer, context.iter().map(llm::ContextItem::citation_ref).collect::<Vec<_>>()))
    })
    .map_err(crate::chain_failed)?;
    crate::util::log_link_usage(store, entry, "ask", answer.token_usage);
    let answered_hosted = llm::is_hosted(entry.provider, &entry.model);
    // An on-device link answered from a context holding excluded material: the answer is
    // the client's as far as any later hosted call goes, and the outcome says so.
    let used = local_context.as_ref().filter(|_| !answered_hosted).unwrap_or(&hosted_context);
    if !answered_hosted && filter.is_active() && (filter.is_closed() || used.iter().any(|i| !filter.keeps(i))) {
        mark_local_only(store, &answer.text);
        return Ok((answer, ExclusionOutcome::LocalOnly, AnsweredBy::of(entry), refs, range));
    }
    mark_clean(store, clean_stamp(&filter, store, used, [], &[query], history), &answer.text);
    let outcome = ExclusionOutcome::after_hosted(answered_hosted, &filter);
    Ok((answer, outcome, AnsweredBy::of(entry), refs, range))
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

/// [`assemble_ask_context_scoped`] with no bound (the tests' shorthand).
#[cfg(test)]
fn assemble_ask_context(
    store: &memory::Store,
    query: &str,
    context_limit: usize,
    filter: &llm::ContextFilter,
) -> (Vec<llm::ContextItem>, Option<String>) {
    let bound = store.scope_ctx(&FindFilter::default()).unwrap();
    let (items, label, _) = assemble_ask_context_scoped(store, query, context_limit, filter, &bound).unwrap();
    (items, label)
}

/// The date range an Ask's retrieval was bounded by: the filter's own, or the window of a "today" or
/// "yesterday" question. `None` for no date bound. A side is `None` when the filter leaves it open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct UsedRange {
    /// Inclusive.
    pub start_unix_ms: Option<i64>,
    /// Exclusive.
    pub end_unix_ms: Option<i64>,
}

/// Shared phase-A logic for both the blocking [`answer`] and the streaming
/// [`prepare_ask`]: date-scoped ("today"/"yesterday") or generic context retrieval, through
/// `filter` (open for a prompt that stays on this computer). With a non-empty `scope` every source is
/// bounded by it. A date filter wins over the question's own day: with one set, retrieval is the normal
/// query path over the filter's range (the question's words still search), and the prompt's date label,
/// when the question names a day, describes that range, not the day; without one, a "today" or
/// "yesterday" question reads its window, narrowed by the app and person bounds. With no scope this is
/// the unbounded retrieval, call for call. The last element is the range actually used.
fn assemble_ask_context_scoped(
    store: &memory::Store,
    query: &str,
    context_limit: usize,
    filter: &llm::ContextFilter,
    scope: &memory::find::ScopeCtx<'_>,
) -> ApiResult<(Vec<llm::ContextItem>, Option<String>, Option<UsedRange>)> {
    let now = now_unix_ms();
    let day = llm::detect_date_scope(query);
    if !scope.is_empty() {
        let bound = scope.filter();
        let dated = bound.start_unix_ms.is_some() || bound.end_unix_ms.is_some();
        if let (Some(date_scope), false) = (day, dated) {
            let (start, end) = llm::date_scope_bounds(date_scope, now);
            let items =
                llm::scoped::gather_and_assemble_context_for_window_scoped(store, now, start, end, context_limit, true, filter, scope).map_err(read_failed)?;
            return Ok((items, Some(llm::format_date_scope_label(date_scope, start)), Some(UsedRange { start_unix_ms: Some(start), end_unix_ms: Some(end) })));
        }
        // The normal query path: over the filter's range when it has dates (which win over the question's day).
        let items = llm::scoped::gather_and_assemble_context_scoped(store, query, now, context_limit, true, true, filter, scope).map_err(read_failed)?;
        let label = day.filter(|_| dated).map(|_| llm::format_date_range_label(bound.start_unix_ms, bound.end_unix_ms));
        let range = dated.then_some(UsedRange { start_unix_ms: bound.start_unix_ms, end_unix_ms: bound.end_unix_ms });
        return Ok((items, label, range));
    }
    match day {
        Some(scope) => {
            let (start, end) = llm::date_scope_bounds(scope, now);
            // Ask's dated window reads added items too; the routines' window does not.
            let items = llm::gather_and_assemble_context_for_window_with_added(
                store,
                "",
                now,
                start,
                end,
                context_limit,
                true,
                filter,
            );
            Ok((items, Some(llm::format_date_scope_label(scope, start)), Some(UsedRange { start_unix_ms: Some(start), end_unix_ms: Some(end) })))
        }
        None => Ok((
            llm::gather_and_assemble_context(store, query, now, context_limit, true, true, filter),
            None,
            None,
        )),
    }
}

/// A scoped read that failed: a filter that does not validate is [`ApiError::InvalidFilter`]; any other
/// error is a database failure ([`ApiError::StoreFailure`]). Never an answer from what could still be read.
fn read_failed(e: rusqlite::Error) -> ApiError {
    match e {
        rusqlite::Error::InvalidParameterName(_) => ApiError::InvalidFilter,
        _ => ApiError::StoreFailure,
    }
}

/// A topic Ask's search limit starts at this many times its context limit...
const SCOPE_POOL_START: usize = 4;
/// ...doubles while too few of the topic's items are found, and stops at this many times: a
/// bound on the work one topic Ask does (five retrievals at most) when memory is large and
/// semantic search, which always fills its limit, never runs short.
const SCOPE_POOL_MAX: usize = 64;

/// [`assemble_ask_context`] over the items `scope` keeps (a topic Ask). Items outside the scope
/// are dropped before packing, so they take no budget. The search limit widens from
/// `SCOPE_POOL_START` to `SCOPE_POOL_MAX` times `context_limit` until `context_limit` items in
/// scope are found or retrieval is exhausted (every search leg came back short); the result
/// holds at most `context_limit` items, in retrieval order.
fn assemble_ask_context_in_scope(
    store: &memory::Store,
    query: &str,
    context_limit: usize,
    filter: &llm::ContextFilter,
    scope: llm::Scope<'_>,
) -> (Vec<llm::ContextItem>, Option<String>) {
    let window = llm::detect_date_scope(query).map(|scope| (scope, llm::date_scope_bounds(scope, now_unix_ms())));
    let mut factor = SCOPE_POOL_START;
    loop {
        let limit = context_limit.saturating_mul(factor);
        let found = match window {
            Some((_, (start, end))) => llm::gather_and_assemble_context_for_window_in_scope(
                store,
                "",
                now_unix_ms(),
                start,
                end,
                limit,
                true,
                filter,
                scope,
            ),
            None => llm::gather_and_assemble_context_in_scope(store, query, now_unix_ms(), limit, true, true, filter, scope),
        };
        if found.items.len() >= context_limit || found.exhausted || factor >= SCOPE_POOL_MAX {
            let mut items = found.items;
            items.truncate(context_limit);
            return (items, window.map(|(scope, (start, _))| llm::format_date_scope_label(scope, start)));
        }
        factor *= 2;
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

#[cfg(test)]
thread_local! {
    /// How many asks THIS thread has stashed. Tests run in parallel against the process-global
    /// map, so its length is not a safe way to ask "did my call stash anything?"; this is.
    static STASHED_ON_THIS_THREAD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Stashes a prepared ask for `finalize_ask` to pick up. The one place a prepare writes the map.
pub(crate) fn stash_prepared(prepare_id: String, prepared: PreparedAsk) {
    #[cfg(test)]
    STASHED_ON_THIS_THREAD.with(|n| n.set(n.get() + 1));
    prepared_asks().lock().unwrap().insert(prepare_id, prepared);
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
/// `scope_empty` is true only from [`prepare_ask_in_topic`] when nothing in the topic matched
/// (or Limited Use left nothing for the streamed provider):
/// then nothing was stashed or sent, and `prepare_id` and `prompt` are empty.
#[derive(Debug, Clone, Serialize)]
pub struct PreparedAskData {
    pub prepare_id: String,
    pub prompt: String,
    pub exclusion: ExclusionOutcome,
    pub scope_empty: bool,
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
    prepare_ask_with(store, query, RequestOrigin::AskBar, context_limit, history, routing, None, None)
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
    prepare_ask_with(store, query, RequestOrigin::AskBar, context_limit, history, routing, Some(call), None)
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
    prepare_ask_with(store, query, origin, context_limit, history, routing, None, None)
}

/// [`prepare_ask`] over one topic's items (screen 5, call 1): asked from that topic's page
/// (`RequestOrigin::TopicPage`), so an excluded client's topic is refused for a hosted stream
/// and answered on this computer (call 4), with every exclusion, spend-cap and Limited Use step
/// of [`prepare_ask`] in the same order. A topic with no live key (unknown, gone) is asked as
/// from the Ask bar; a key that cannot be read is `Unreadable` (refused for a hosted stream).
/// Retrieval keeps only items whose topic item is a non-removed member of `topic_id` (any
/// member, not only items that prefer this topic; merges followed), dropped before packing, and
/// widens its search until `context_limit` such items are found, retrieval is exhausted, or the
/// bound is reached ([`assemble_ask_context_in_scope`]). When none are found, or Limited Use
/// leaves none for the streamed provider (a topic of only Gmail threads), nothing is stashed or
/// sent: `scope_empty` is true and `prepare_id` and `prompt` are empty (the shell says so and
/// offers to ask everything). That return comes after the refusals and the Limited Use filter
/// and before the spend-cap check, which records an event.
pub fn prepare_ask_in_topic(
    store: &memory::Store,
    topic_id: i64,
    query: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
) -> ApiResult<PreparedAskData> {
    prepare_ask_in_topic_with(store, topic_id, query, context_limit, history, routing, None)
}

/// [`prepare_ask_in_topic`] with this call's own Limited Use settings (`call`, read from this
/// call's routing JSON), as [`prepare_ask_for`] is to [`prepare_ask`].
pub fn prepare_ask_in_topic_for(
    store: &memory::Store,
    topic_id: i64,
    query: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    call: &crate::routing::LimitedUseSettings,
) -> ApiResult<PreparedAskData> {
    prepare_ask_in_topic_with(store, topic_id, query, context_limit, history, routing, Some(call))
}

fn prepare_ask_in_topic_with(
    store: &memory::Store,
    topic_id: i64,
    query: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    call: Option<&crate::routing::LimitedUseSettings>,
) -> ApiResult<PreparedAskData> {
    let origin = match store.topic_key(topic_id) {
        Ok(Some(key)) => RequestOrigin::TopicPage { key },
        Ok(None) => RequestOrigin::AskBar,
        Err(_) => RequestOrigin::Unreadable,
    };
    let in_topic = |item: &llm::ContextItem| item_in_topic(store, topic_id, item);
    prepare_ask_with(store, query, origin, context_limit, history, routing, call, Some(&in_topic))
}

/// Whether `item` is filed under live topic `topic_id`: its citation's topic item (a transcript
/// line is its meeting, a capture its episode) is a non-removed member of that topic, whichever
/// topic it prefers. An item with no topic item, or a lookup that fails, is not in the topic.
fn item_in_topic(store: &memory::Store, topic_id: i64, item: &llm::ContextItem) -> bool {
    let (kind, id) = crate::conversations::kind_and_id(&item.citation_ref());
    match store.topic_item_for_citation(kind, id) {
        Ok(Some((item_kind, ref_id))) => store.item_in_topic(&item_kind, ref_id, topic_id).unwrap_or(false),
        _ => false,
    }
}

/// The one streamed Ask prepare. `keep`, when given, scopes retrieval to the items it keeps
/// ([`prepare_ask_in_topic`]); every other step is the same with or without it.
#[allow(clippy::too_many_arguments)]
fn prepare_ask_with(
    store: &memory::Store,
    query: &str,
    origin: RequestOrigin,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    call: Option<&crate::routing::LimitedUseSettings>,
    keep: Option<&dyn Fn(&llm::ContextItem) -> bool>,
) -> ApiResult<PreparedAskData> {
    prepare_ask_scoped_in(store, query, origin, context_limit, history, routing, call, &AskScope::default(), keep).map(|p| p.data)
}

/// What a streamed Ask does with this routing before anything is read, for a request that
/// `carries` an excluded client or not. The one decision [`prepare_ask_scoped`] and the route preview share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamDecision {
    /// Nothing streams. `reason` is `local_only_cloud_or_unknown_model` (local-only mode, and the Ollama
    /// model is a cloud model or not stated) or `names_excluded_client_hosted_stream` (the request
    /// carries an excluded client and the stream would leave this computer: the shell asks again on this computer).
    Refuse { reason: &'static str },
    /// The shell streams against `provider`; `hosted` says that stream leaves this computer.
    Stream { provider: llm::Provider, hosted: bool },
}

pub(crate) fn stream_decision(routing: &RoutingConfig, carries: bool) -> StreamDecision {
    if crate::exclusion::local_only_mode_refuses(routing) {
        return StreamDecision::Refuse { reason: "local_only_cloud_or_unknown_model" };
    }
    let hosted = crate::exclusion::streamed_is_hosted(routing);
    if carries && hosted {
        return StreamDecision::Refuse { reason: "names_excluded_client_hosted_stream" };
    }
    StreamDecision::Stream { provider: provider_for_streamed_answer(routing), hosted }
}

/// What [`prepare_ask_scoped`] hands back: the plain [`PreparedAskData`], the references of the prompt's
/// items in order (number `n` is `refs[n - 1]`), the provider streamed against, and whether that stream
/// stays on this computer.
pub(crate) struct PreparedScoped {
    pub(crate) data: PreparedAskData,
    pub(crate) refs: Vec<llm::ContextItemRef>,
    pub(crate) provider: llm::Provider,
    pub(crate) local: bool,
    pub(crate) range: Option<UsedRange>,
}

/// [`prepare_ask_with`] bounded by `scope`: its filter bounds retrieval, and `require_local` makes the
/// routing local-only for this call, so a hosted provider or an Ollama cloud model is refused (the
/// existing local-only refusal) rather than streamed to.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_ask_scoped(
    store: &memory::Store,
    query: &str,
    origin: RequestOrigin,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    call: Option<&crate::routing::LimitedUseSettings>,
    scope: &AskScope,
) -> ApiResult<PreparedScoped> {
    prepare_ask_scoped_in(store, query, origin, context_limit, history, routing, call, scope, None)
}

/// [`prepare_ask_scoped`] that can also keep only the items `keep` keeps (a topic Ask,
/// [`prepare_ask_in_topic`]); every other step is the same with or without it. `keep` and a
/// non-empty `scope.filter` are not combined: the topic Ask passes the default scope.
#[allow(clippy::too_many_arguments)]
fn prepare_ask_scoped_in(
    store: &memory::Store,
    query: &str,
    origin: RequestOrigin,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    call: Option<&crate::routing::LimitedUseSettings>,
    scope: &AskScope,
    keep: Option<&dyn Fn(&llm::ContextItem) -> bool>,
) -> ApiResult<PreparedScoped> {
    crate::exclusion::refresh_published_gate(store);
    let filter = crate::exclusion::context_filter(store);
    let forced;
    let routing = if scope.require_local {
        forced = RoutingConfig { sensitive: true, ..routing.clone() };
        &forced
    } else {
        routing
    };
    let carries = request_carries(store, &filter, &scope.carry_text(query), history, &origin);
    let hosted = match stream_decision(routing, carries) {
        StreamDecision::Refuse { .. } => return Err(crate::exclusion::refused()),
        StreamDecision::Stream { hosted, .. } => hosted,
    };
    let open = llm::ContextFilter::none();
    let retrieval_filter = if hosted { &filter } else { &open };
    let (context, date_scope_label, range) = match keep {
        None => {
            let bound = store.scope_ctx(&scope.filter).map_err(read_failed)?;
            assemble_ask_context_scoped(store, query, context_limit, retrieval_filter, &bound)?
        }
        Some(keep) => {
            let (items, label) = assemble_ask_context_in_scope(store, query, context_limit, retrieval_filter, keep);
            (items, label, None)
        }
    };
    // The shell streams against this one provider: pinned, no fallthrough.
    let provider = provider_for_streamed_answer(routing);
    // Limited Use: the prompt (and the citable context) holds Gmail threads only when the
    // provider streamed against may receive Google user data.
    let may_receive_google = crate::routing::streamed_may_receive_google_data(provider, call);
    let context = llm::limited_use::filter_context_in(store, &context, may_receive_google);
    if keep.is_some() && context.is_empty() {
        // Nothing in scope to send (none matched, or Limited Use left none): no prompt is
        // built, stashed or marked. A hosted stream still reports what the filter left out.
        let exclusion = if hosted { ExclusionOutcome::after_hosted(true, &filter) } else { ExclusionOutcome::Allowed };
        let data = PreparedAskData { prepare_id: String::new(), prompt: String::new(), exclusion, scope_empty: true };
        return Ok(PreparedScoped { data, refs: Vec::new(), provider, local: !hosted, range });
    }

    let cap_config = routing.spend_cap_config();
    llm::check_spend_cap(store, &cap_config, None, Some("ask"), now_unix_ms())?;
    check_provider(store, provider, now_unix_ms())?;
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
    let refs: Vec<llm::ContextItemRef> = context.iter().map(llm::ContextItem::citation_ref).collect();
    stash_prepared(
        prepare_id.clone(),
        PreparedAsk { context, provider, exclusion, clean_keys, history_google: Some(history_google) },
    );
    Ok(PreparedScoped { data: PreparedAskData { prepare_id, prompt, exclusion, scope_empty: false }, refs, provider, local: !hosted, range })
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
    stash_prepared(
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
    finalize_prepared(store, prepared, final_text, model, usage)
}

/// [`finalize_ask`] over a prepare already taken out of the stash.
fn finalize_prepared(
    store: &memory::Store,
    prepared: Option<PreparedAsk>,
    final_text: &str,
    model: &str,
    usage: llm::TokenUsage,
) -> FinalizedAsk {
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

// ---- numbered sources (screen 5) -------------------------------------------------------------

/// One item the model saw, by its number in the prompt (`[n]`), with its card read from the live row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NumberedSource {
    pub n: usize,
    pub card: SourceCard,
    /// The row could not be read just now (a database error), which is not the same as gone: `card.found`
    /// is false either way, but an unreadable source may well exist, and a page must not say it is "no
    /// longer on this device".
    pub unreadable: bool,
}

fn ref_key(r: &llm::ContextItemRef) -> (String, i64) {
    match *r {
        llm::ContextItemRef::Capture { id } => ("capture".into(), id),
        llm::ContextItemRef::Transcript { id } => ("transcript".into(), id),
        llm::ContextItemRef::RoutineRun { id } => ("routine_run".into(), id),
        llm::ContextItemRef::Episode { id } => ("episode".into(), id),
        llm::ContextItemRef::Thread { id } => ("thread".into(), id),
        llm::ContextItemRef::Snapshot { id } => ("snapshot".into(), id),
        llm::ContextItemRef::McpItem { id } => ("mcp_item".into(), id),
    }
}

/// The cards for the prompt's items, numbered from 1 in prompt order, read from the live rows now. A row
/// gone since the prompt was built comes back with `found: false` and no text. A read that FAILS marks
/// every card `unreadable` (and `found: false`, no text), so it is never mistaken for a deleted row.
pub fn numbered_sources(store: &memory::Store, refs: &[llm::ContextItemRef]) -> Vec<NumberedSource> {
    let keys: Vec<(String, i64)> = refs.iter().map(ref_key).collect();
    let (cards, unreadable) = match store.resolve_cards(&keys) {
        Ok(cards) => (cards, false),
        Err(_) => (keys.iter().map(|(k, id)| SourceCard { kind: k.clone(), id: *id, ..Default::default() }).collect(), true),
    };
    cards.into_iter().enumerate().map(|(i, card)| NumberedSource { n: i + 1, card, unreadable }).collect()
}

/// The sources an answer cited, as they are saved: number, kind and id only.
pub fn saved_sources(refs: &[llm::ContextItemRef], cited: &[usize]) -> Vec<SavedSource> {
    cited
        .iter()
        .filter_map(|n| refs.get(n.checked_sub(1)?).map(|r| (*n, ref_key(r))))
        .map(|(n, (kind, id))| SavedSource { n: n as i64, kind, id })
        .collect()
}

/// [`DetailedAnswer`] with the numbered list the model saw and which numbers the answer cited.
#[derive(Debug, Clone, Serialize)]
pub struct SourcedAnswer {
    #[serde(flatten)]
    pub detailed: DetailedAnswer,
    /// Every item in the prompt of the link that answered, by number.
    pub sources: Vec<NumberedSource>,
    /// The numbers the answer cites, in first-cited order. A number outside the list is dropped.
    pub cited: Vec<usize>,
    /// The cited ones as the conversation saves them (`{n, kind, id}`).
    pub cited_sources: Vec<SavedSource>,
    /// The answer was made on this computer: an on-device link, or a question rerouted for an excluded
    /// client. False for an Ollama cloud model, which `answered_by.local` counts as local. Save this one.
    pub answered_locally: bool,
    /// The date range the retrieval used: the filter's own (a date filter wins over a "yesterday" in the
    /// question), or the window of a "today" or "yesterday" question. Null for no date bound.
    pub date_range: Option<UsedRange>,
}

/// [`answer_detailed`] bounded by `scope`, returning the numbered context. `scope.filter` bounds every
/// retrieval source; `scope.require_local` keeps the call on this computer (it fails with the existing
/// provider failure when no on-device link can answer). Exclusion runs first, as for every Ask.
pub fn answer_sourced(
    store: &memory::Store,
    query: &str,
    ollama_model: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    scope: &AskScope,
) -> ApiResult<SourcedAnswer> {
    let scope = &checked_scope(scope)?;
    let filter = crate::exclusion::context_filter(store);
    let (answer, exclusion, answered_by, refs, date_range) = answer_scoped_by(
        store,
        filter,
        query,
        &RequestOrigin::AskBar,
        context_limit,
        history,
        routing,
        || build_chain_for(llm::TaskKind::Chat, ollama_model, routing, &crate::exclusion::current_gate(store)),
        || build_local_chain(ollama_model, routing),
        scope,
    )?;
    let cited = llm::cited_numbers(&answer.text, refs.len());
    let answered_locally =
        exclusion == ExclusionOutcome::LocalOnly || llm::Provider::parse(&answered_by.provider).map_or(answered_by.local, |p| !llm::is_hosted(p, &answered_by.model));
    Ok(SourcedAnswer {
        answered_locally,
        sources: numbered_sources(store, &refs),
        cited_sources: saved_sources(&refs, &cited),
        cited,
        date_range,
        detailed: DetailedAnswer { answer, exclusion, answered_by },
    })
}

/// [`PreparedAskData`] with the numbered list the prompt holds.
#[derive(Debug, Clone, Serialize)]
pub struct PreparedAskSourced {
    #[serde(flatten)]
    pub data: PreparedAskData,
    pub sources: Vec<NumberedSource>,
    /// The provider the shell streams against.
    pub provider: String,
    /// The stream stays on this computer. Saved with the answer.
    pub answered_locally: bool,
    /// The date range the retrieval used (see [`SourcedAnswer::date_range`]).
    pub date_range: Option<UsedRange>,
}

/// `scope` with its filter validated and normalized, or the refusal for one that could widen the Ask
/// ([`FindFilter::validated`]). A filter that does not validate is an error, never an unbounded search:
/// it is [`ApiError::InvalidFilter`] (nothing is read or sent).
fn checked_scope(scope: &AskScope) -> ApiResult<AskScope> {
    let filter = scope.filter.clone().validated().map_err(|_| ApiError::InvalidFilter)?;
    Ok(AskScope { filter, require_local: scope.require_local })
}

/// [`prepare_ask_for`] bounded by `scope`. With `scope.require_local` the routing is local-only for this
/// call: a hosted provider, an Ollama cloud model, or an Ollama model the routing does not name is refused
/// with the existing local-only refusal. Core cannot stop a shell from streaming the prompt somewhere else;
/// a shell that said "on this computer" must stream it to its on-device model only.
pub fn prepare_ask_sourced(
    store: &memory::Store,
    query: &str,
    context_limit: usize,
    history: &[llm::ChatTurn],
    routing: &RoutingConfig,
    call: &crate::routing::LimitedUseSettings,
    scope: &AskScope,
) -> ApiResult<PreparedAskSourced> {
    let scope = &checked_scope(scope)?;
    let p = prepare_ask_scoped(store, query, RequestOrigin::AskBar, context_limit, history, routing, Some(call), scope)?;
    Ok(PreparedAskSourced {
        sources: numbered_sources(store, &p.refs),
        provider: p.provider.as_str().to_string(),
        answered_locally: p.local,
        date_range: p.range,
        data: p.data,
    })
}

/// [`FinalizedAsk`] with the numbers the answer cited.
#[derive(Debug, Clone, Serialize)]
pub struct FinalizedAskSourced {
    pub citations: Vec<llm::ContextItemRef>,
    pub exclusion: ExclusionOutcome,
    /// The cited numbers, first-cited order, within the list the prepare returned.
    pub cited: Vec<usize>,
    /// The cited items as the conversation saves them (`{n, kind, id}`).
    pub cited_sources: Vec<SavedSource>,
    /// How many retrieved items exclusion left out of the prompt (0 unless a hosted model answered).
    pub left_out: i64,
}

/// [`finalize_ask`] that also returns the cited numbers, taken against the list the same prepare
/// returned (kept with the prepared state). An unknown or already finalized id cites nothing.
pub fn finalize_ask_sourced(
    store: &memory::Store,
    prepare_id: &str,
    final_text: &str,
    model: &str,
    usage: llm::TokenUsage,
) -> FinalizedAskSourced {
    let prepared = prepared_asks().lock().unwrap().remove(prepare_id);
    let refs: Vec<llm::ContextItemRef> = prepared.as_ref().map(|p| p.context.iter().map(llm::ContextItem::citation_ref).collect()).unwrap_or_default();
    let finalized = finalize_prepared(store, prepared, final_text, model, usage);
    let cited = llm::cited_numbers(final_text, refs.len());
    let left_out = match finalized.exclusion {
        ExclusionOutcome::FilteredHosted { left_out } => left_out as i64,
        _ => 0,
    };
    FinalizedAskSourced { cited_sources: saved_sources(&refs, &cited), cited, citations: finalized.citations, exclusion: finalized.exclusion, left_out }
}

// ---- where an Ask would be answered ("the button's name is a promise") ------------------------

/// Which call a preview is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    /// `prepare_ask_sourced` and a stream by the shell: pinned to the routing's provider.
    Stream,
    /// `answer_sourced`: the provider chain, which falls through on a failure.
    Blocking,
}

/// Where an Ask would be answered, from the same decisions the call makes.
///
/// `destination`: `local` (this computer), `hosted` (it leaves this computer), `refused` (the call
/// would fail: see `reason`) or `unknown` (it cannot be known before sending). `local` is `true` only for
/// `local`. `provider` and `model` are the first link, or the provider the shell streams against.
///
/// `require_local` says to pass `require_local` to the Ask call (and to any blocking fallback of a stream):
/// set for every `local` answer, so the call never falls through to a hosted model.
/// `may_fall_back` (hosted only): a failure can move the answer to another provider in the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoutePreview {
    pub destination: &'static str,
    pub local: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reason: &'static str,
    pub require_local: bool,
    pub may_fall_back: bool,
}

impl RoutePreview {
    fn new(destination: &'static str, reason: &'static str) -> Self {
        RoutePreview { destination, local: destination == "local", provider: None, model: None, reason, require_local: destination == "local", may_fall_back: false }
    }
    fn with(mut self, provider: &str, model: &str) -> Self {
        self.provider = Some(provider.to_string());
        self.model = Some(model.to_string()).filter(|m| !m.is_empty());
        self
    }
}

/// The preview of a call whose filter does not validate: `refused`, reason `invalid_filter`. What
/// [`ask_route_preview`] says for such a filter, for a caller that could not even build the filter.
pub fn invalid_filter_preview() -> RoutePreview {
    RoutePreview::new("refused", "invalid_filter")
}

/// Whether previewing a call could rescan the store for excluded clients' material: some client is
/// excluded and the setting reads. With none excluded (or an unreadable setting, which fails closed with no
/// scan) a preview is a handful of cheap reads, safe on the store a page is waiting on; otherwise it may
/// take as long as the exclusion cache needs to rebuild, so a shell runs it where a slow read belongs and
/// shows [`unchecked_preview`] if that lane is not free in time.
pub fn route_preview_may_rescan(store: &memory::Store) -> bool {
    if store.exclusion_unreadable() {
        return false;
    }
    !matches!(store.excluded_terms(), Ok(terms) if terms.is_empty())
}

/// What a shell shows when the preview needed a rescan it could not start in time: `unknown`, so the button
/// promises nothing (plain "Ask") and the call is made without `require_local`, as the preview of a call
/// that cannot be known in advance always is. Reason `excluded_client_check_pending`.
pub fn unchecked_preview() -> RoutePreview {
    RoutePreview::new("unknown", "excluded_client_check_pending")
}

/// Where `query` (with `history`) would be answered under `routing`, for a call of `mode`, without
/// sending anything or changing any state. See [`RoutePreview`], and the truth table in the report
/// of this feature: a question that names an excluded client reroutes to this computer; local-only
/// mode, a missing key or model, a spent spend cap or monthly budget are known now; whether Ollama is
/// running is the shell's to know (a local answer then falls to the bundled model, still local).
pub fn ask_route_preview(
    store: &memory::Store,
    query: &str,
    history: &[llm::ChatTurn],
    ollama_model: &str,
    routing: &RoutingConfig,
    scope: &AskScope,
    mode: RouteMode,
) -> RoutePreview {
    // The Ask itself refuses a filter that does not validate: say so rather than name a destination.
    let Ok(scope) = checked_scope(scope) else { return RoutePreview::new("refused", "invalid_filter") };
    let scope = &scope;
    let filter = crate::exclusion::context_filter(store);
    let carries = request_carries(store, &filter, &scope.carry_text(query), history, &RequestOrigin::AskBar);
    let preview = match mode {
        RouteMode::Blocking => preview_blocking(store, ollama_model, routing, scope, carries),
        RouteMode::Stream => preview_stream(store, ollama_model, routing, scope, carries),
    };
    // Planning records a refusal's reason for the failing call to take; a preview fails nothing.
    llm::take_last_chain_failures();
    preview
}

/// A blocking call's route. A `local` answer is asked for with `require_local`, and the call then walks the
/// on-device chain only (which can be empty where the routing's own local-only chain is not), so a local
/// result is judged again as that call, and the preview says `refused` when it would fail.
fn preview_blocking(store: &memory::Store, ollama_model: &str, routing: &RoutingConfig, scope: &AskScope, carries: bool) -> RoutePreview {
    let first = blocking_outcome(store, ollama_model, routing, scope.require_local, carries);
    if first.destination != "local" || scope.require_local {
        return first;
    }
    let forced = blocking_outcome(store, ollama_model, routing, true, carries);
    match forced.destination {
        "local" => RoutePreview { provider: forced.provider, model: forced.model, ..first },
        _ => forced,
    }
}

/// [`preview_blocking`]'s decision for a call with this `require_local`.
fn blocking_outcome(store: &memory::Store, ollama_model: &str, routing: &RoutingConfig, require_local: bool, carries: bool) -> RoutePreview {
    let scope = &AskScope { require_local, ..Default::default() };
    let plan = plan_blocking(
        store,
        routing,
        carries,
        scope.require_local,
        || build_chain_for(llm::TaskKind::Chat, ollama_model, routing, &crate::exclusion::current_gate(store)),
        || build_local_chain(ollama_model, routing),
    );
    let plan = match plan {
        Ok(plan) => plan,
        Err(ApiError::SpendCap(_)) => return RoutePreview::new("refused", "spend_cap"),
        Err(ApiError::BudgetSpent(b)) => return RoutePreview::new("refused", "budget_spent").with(b.provider.as_str(), ""),
        Err(_) if carries => return RoutePreview::new("refused", "excluded_client_no_local_model"),
        Err(_) => return RoutePreview::new("refused", "no_provider"),
    };
    let Some(first) = plan.chain.first() else {
        return RoutePreview::new("refused", if scope.require_local || routing.sensitive { "local_only_no_local_model" } else { "no_provider" });
    };
    if !llm::is_hosted(first.provider, &first.model) {
        let reason = if plan.rerouted {
            "names_excluded_client"
        } else if scope.require_local {
            "required_local"
        } else if routing.sensitive {
            "local_only_mode"
        } else {
            "first_link_is_local"
        };
        return RoutePreview::new("local", reason).with(first.provider.as_str(), &first.model);
    }
    let mut preview = RoutePreview::new("hosted", "first_link_is_hosted").with(first.provider.as_str(), &first.model);
    preview.may_fall_back = plan.chain.len() > 1;
    preview
}

/// A streamed call's route. A `local` answer is asked for with `require_local` (see [`RoutePreview`]), and
/// the call then forces local-only routing, which can refuse what the routing as given would stream (an
/// Ollama model the routing does not name, or a cloud one). So a local result is judged again on exactly the
/// routing the real call will run, and the preview says `refused` when that call would.
fn preview_stream(store: &memory::Store, ollama_model: &str, routing: &RoutingConfig, scope: &AskScope, carries: bool) -> RoutePreview {
    let first = stream_outcome(store, ollama_model, routing, scope.require_local, carries);
    if first.destination != "local" {
        return first;
    }
    let local_routing = RoutingConfig { sensitive: true, ..routing.clone() };
    let forced = stream_outcome(store, ollama_model, &local_routing, true, carries);
    match forced.destination {
        // What the call will do: the same on-device stream. Keep the reason that explains why it is local.
        "local" => RoutePreview { provider: forced.provider, model: forced.model, ..first },
        // Never `hosted` or `unknown` for a call that is told to stay local: that call refuses or fails.
        "refused" => forced,
        _ => RoutePreview::new("refused", "local_only_cloud_or_unknown_model"),
    }
}

/// [`preview_stream`]'s decision on `routing` as given; `forced` says the call is already told to stay local.
fn stream_outcome(store: &memory::Store, ollama_model: &str, routing: &RoutingConfig, forced: bool, carries: bool) -> RoutePreview {
    let local_routing = RoutingConfig { sensitive: true, ..routing.clone() };
    let eff = if forced { &local_routing } else { routing };
    let (provider, hosted, rerouted) = match stream_decision(eff, carries) {
        StreamDecision::Stream { provider, hosted } => (provider, hosted, false),
        StreamDecision::Refuse { reason: "names_excluded_client_hosted_stream" } => match stream_decision(&local_routing, carries) {
            StreamDecision::Stream { provider, hosted } => (provider, hosted, true),
            StreamDecision::Refuse { .. } => return RoutePreview::new("refused", "excluded_client_no_local_model"),
        },
        StreamDecision::Refuse { reason } => return RoutePreview::new("refused", reason),
    };
    let routing = if forced || rerouted { &local_routing } else { routing };
    if llm::check_spend_cap(store, &routing.spend_cap_config(), None, Some("ask"), now_unix_ms()).is_err() {
        return RoutePreview::new("refused", "spend_cap");
    }
    let model = streamed_model(routing, ollama_model, provider);
    if check_provider(store, provider, now_unix_ms()).is_err() {
        // The shell then answers on the blocking chain, which picks the next provider and does not say which.
        return RoutePreview::new("unknown", "budget_spent_falls_back_to_blocking").with(provider.as_str(), &model);
    }
    if hosted {
        let mut preview = RoutePreview::new("hosted", "preferred_provider").with(provider.as_str(), &model);
        preview.may_fall_back = true;
        return preview;
    }
    let reason = if rerouted {
        "names_excluded_client"
    } else if forced {
        "required_local"
    } else if routing.sensitive {
        "local_only_mode"
    } else {
        "preferred_is_local"
    };
    RoutePreview::new("local", reason).with(provider.as_str(), &model)
}

/// The model a stream against `provider` runs, as far as the routing says.
fn streamed_model(routing: &RoutingConfig, ollama_model: &str, provider: llm::Provider) -> String {
    let preferred = (routing.preferred_provider == provider.as_str()).then_some(routing.preferred_model.as_str()).filter(|m| !m.is_empty());
    match provider {
        llm::Provider::Ollama => [routing.ollama_model.as_str(), ollama_model, preferred.unwrap_or("")].into_iter().find(|m| !m.is_empty()).unwrap_or("").to_string(),
        llm::Provider::Whimbrel => preferred.unwrap_or("").to_string(),
        llm::Provider::OpenRouter => preferred.unwrap_or(&routing.openrouter_model).to_string(),
        llm::Provider::Local => crate::local_model::MODEL_ID.to_string(),
        p => crate::model_defaults::resolve(p, Some(&routing.preferred_model), &crate::model_defaults::cached()),
    }
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

    /// A topic Ask keeps only its topic's items: an added item is never a topic item in v1, so a
    /// topic scope drops it, even one whose text names the topic.
    #[test]
    fn a_topic_scope_drops_added_items() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::test_store::open(&dir.path().join("m.db"), dir.path());
        let topic = store.create_seed_topic("Birch", &[], None).unwrap();
        let id = crate::test_store::add_mcp_item(&store, "a", "Birch roadmap", "roadmap for Birch", None);
        let hit = store.search_mcp_items("roadmap", false, 5).unwrap().into_iter().find(|h| h.id == id).unwrap();
        assert!(!item_in_topic(&store, topic.topic_id, &llm::ContextItem::McpItem(hit)));
        let keep = |i: &llm::ContextItem| item_in_topic(&store, topic.topic_id, i);
        let (items, _) = assemble_ask_context_in_scope(&store, "roadmap", 10, &llm::ContextFilter::none(), &keep);
        assert!(items.iter().all(|i| !matches!(i, llm::ContextItem::McpItem(_))));
        let (open, _, _) = assemble_ask_context_scoped(&store, "roadmap", 10, &llm::ContextFilter::none(), &store.scope_ctx(&Default::default()).unwrap()).unwrap();
        assert!(open.iter().any(|i| matches!(i, llm::ContextItem::McpItem(_))), "unscoped, the item is found");
    }

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
        let before = stashed_on_this_thread();
        assert!(matches!(prepare_meeting_ask(&store, id, "q", &[], &RoutingConfig::default()), Err(MeetingAskError::NoTranscript)));
        assert!(matches!(prepare_meeting_ask(&store, 9_999, "q", &[], &RoutingConfig::default()), Err(MeetingAskError::NoTranscript)));
        assert_eq!(stashed_on_this_thread(), before, "a refused ask must stash nothing");
    }

    #[test]
    fn a_transcript_whose_every_segment_exceeds_the_budget_is_treated_as_no_transcript() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let huge = "x".repeat(llm::meeting_ask::context_budget_chars(llm::Provider::Ollama) + 1);
        let id = meeting_with_transcript(&store, &[huge.as_str()]);
        let before = stashed_on_this_thread();
        assert!(matches!(prepare_meeting_ask(&store, id, "q", &[], &RoutingConfig::default()), Err(MeetingAskError::NoTranscript)));
        assert_eq!(stashed_on_this_thread(), before, "a refused ask must stash nothing");
    }

    #[test]
    fn a_meeting_ask_obeys_the_spend_cap_and_stashes_nothing() {
        let _serial = serialize_prepared_asks();
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting_with_transcript(&store, &["hello"]);
        let routing = tripped_cap_routing(&store);
        let before = stashed_on_this_thread();
        let result = prepare_meeting_ask(&store, id, "q", &[], &routing);
        assert!(matches!(result, Err(MeetingAskError::Api(ApiError::SpendCap(_)))));
        assert_eq!(stashed_on_this_thread(), before, "a refused ask must stash nothing");
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
    fn stashed_on_this_thread() -> usize {
        STASHED_ON_THIS_THREAD.with(|n| n.get())
    }

    fn serialize_prepared_asks() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The control for the "stashes nothing" tests: the per-thread counter they read does move
    /// when an ask is stashed, so a flat counter means something.
    #[test]
    fn a_stashed_ask_moves_the_per_thread_counter() {
        let store = store_with_capture_text("the quarterly roadmap review");
        let before = stashed_on_this_thread();
        let prepared = prepare_ask(&store, "roadmap", 5, &[], &RoutingConfig::default()).unwrap();
        assert_eq!(stashed_on_this_thread(), before + 1);
        discard_ask(&prepared.prepare_id);
    }

    #[test]
    fn prepare_ask_reports_spend_cap_without_stashing_anything() {
        let _serial = serialize_prepared_asks();
        let store = store_with_capture_text("the quarterly roadmap review");
        let routing = tripped_cap_routing(&store);
        let before = stashed_on_this_thread();
        let result = prepare_ask(&store, "roadmap", 5, &[], &routing);
        assert!(matches!(result, Err(ApiError::SpendCap(_))));
        assert_eq!(stashed_on_this_thread(), before, "a refused ask must stash nothing");
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
            scope_empty: false,
        };
        assert_eq!(
            serde_json::to_string(&data).unwrap(),
            r#"{"prepare_id":"id","prompt":"p","exclusion":{"kind":"filtered_hosted","left_out":2},"scope_empty":false}"#
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
        let before = stashed_on_this_thread();
        let result = prepare_ask(&store, "roadmap", 5, &[], &routing);
        let Err(ApiError::BudgetSpent(spent)) = result else { panic!("{result:?}") };
        assert_eq!(spent.provider, llm::Provider::Anthropic);
        assert_eq!(stashed_on_this_thread(), before, "a refused ask must stash nothing");

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

    // ---- Topic-scoped Ask (screen 5, calls 1 and 4) -------------------------------------

    /// A file-backed store (topic filings are written through a raw connection) holding two
    /// meetings that both mention the roadmap; only `filed` is filed under topic Birch.
    struct TopicFx {
        _dir: tempfile::TempDir,
        store: memory::Store,
        c: rusqlite::Connection,
        birch: i64,
        filed: i64,
        unfiled: i64,
    }

    fn topic_fx() -> TopicFx {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let store = crate::test_store::open(&path, dir.path());
        let c = crate::test_store::raw(&path);
        let filed = titled_meeting(&store, "Planning", 3_600_000, "the birchtree roadmap is due in March");
        let unfiled = titled_meeting(&store, "Review", 7_200_000, "the oakleaf roadmap slipped a quarter");
        let birch = store.create_seed_topic("Birch", &[], None).unwrap().topic_id;
        file_under(&c, birch, "meeting", filed);
        TopicFx { _dir: dir, store, c, birch, filed, unfiled }
    }

    /// `prepare_ask_in_topic_for` decides Gmail by this call's own Limited Use settings, as
    /// `prepare_ask_for` does: an Ollama call whose streamed model is local keeps a topic's Gmail
    /// thread; one streaming a cloud model (or naming none) has it filtered.
    #[test]
    fn a_topic_ask_for_honours_the_calls_limited_use_settings() {
        let _serial = serialize_prepared_asks();
        let fx = topic_fx();
        fx.store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "m-1".into(),
                ts_unix_ms: now_unix_ms() - 1_000,
                from: "Priya <priya@x.com>".into(),
                to: "me@x.com".into(),
                subject: "Roadmap".into(),
                body_text: "GMAILSECRET the roadmap budget is 40k".into(),
                gmail_thread_id: "thread-1".into(),
                account_email: "me@x.com".into(),
                ..Default::default()
            }])
            .unwrap();
        while fx.store.run_thread_state_sync_job(now_unix_ms()).unwrap() {}
        let email: i64 = fx.c.query_row("SELECT id FROM items WHERE kind = 'email'", [], |r| r.get(0)).unwrap();
        file_under(&fx.c, fx.birch, "email", email);
        let prompt_for = |json: &str| {
            let routing = RoutingConfig::from_json(Some(json));
            let call = crate::routing::LimitedUseSettings::from_routing_json_str(Some(json));
            let data = prepare_ask_in_topic_for(&fx.store, fx.birch, "roadmap", 10, &[], &routing, &call).unwrap();
            discard_ask(&data.prepare_id);
            data.prompt
        };
        let local = r#"{"preferred_provider":"ollama","streamed_ollama_model":"gemma4:12b"}"#;
        let cloud = r#"{"preferred_provider":"ollama","streamed_ollama_model":"gpt-oss:120b-cloud"}"#;
        let p = prompt_for(local);
        assert!(p.contains("GMAILSECRET") && p.contains("birchtree"), "{p}");
        let p = prompt_for(cloud);
        assert!(!p.contains("GMAILSECRET") && p.contains("birchtree"), "{p}");
        assert!(!prompt_for(r#"{"preferred_provider":"ollama"}"#).contains("GMAILSECRET"), "unknown model fails closed");
    }

    /// A completed meeting titled `title` with one transcript line (not from Google Calendar).
    fn titled_meeting(store: &memory::Store, title: &str, ago_ms: i64, line: &str) -> i64 {
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: now_unix_ms() - ago_ms,
                end_unix_ms: Some(now_unix_ms() - ago_ms + 60_000),
                title: title.into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[memory::NewTranscriptSegment { meeting_id: id, start_cs: 0, speaker_label: "Ben".into(), text: line.into() }])
            .unwrap();
        store.set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_OTHER).unwrap();
        id
    }

    fn file_under(c: &rusqlite::Connection, topic: i64, kind: &str, ref_id: i64) {
        c.execute(
            "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, 1, 1)",
            rusqlite::params![kind, ref_id],
        )
        .unwrap();
        let sid = c.last_insert_rowid();
        c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", rusqlite::params![topic, sid])
            .unwrap();
    }

    /// The meeting each stashed transcript line of `prepare_id` belongs to.
    fn stashed_meetings(c: &rusqlite::Connection, prepare_id: &str) -> Vec<i64> {
        let asks = prepared_asks().lock().unwrap();
        let context = &asks.get(prepare_id).expect("stashed").context;
        context
            .iter()
            .map(|item| match item.citation_ref() {
                llm::ContextItemRef::Transcript { id } => c
                    .query_row("SELECT thread_id FROM items WHERE id = ?1", [id], |r| r.get(0))
                    .unwrap(),
                other => panic!("only transcript lines here: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_topic_ask_cites_only_that_topics_items() {
        let _serial = serialize_prepared_asks();
        let fx = topic_fx();
        let routing = RoutingConfig::default();
        // Control: the whole of memory answers from both meetings.
        let all = prepare_ask(&fx.store, "roadmap", 5, &[], &routing).unwrap();
        assert!(all.prompt.contains("birchtree") && all.prompt.contains("oakleaf"), "{}", all.prompt);
        let mut both = stashed_meetings(&fx.c, &all.prepare_id);
        both.sort();
        assert_eq!(both, vec![fx.filed, fx.unfiled]);
        discard_ask(&all.prepare_id);

        let scoped = prepare_ask_in_topic(&fx.store, fx.birch, "roadmap", 5, &[], &routing).unwrap();
        assert!(!scoped.scope_empty);
        assert!(scoped.prompt.contains("birchtree"), "{}", scoped.prompt);
        assert!(!scoped.prompt.contains("oakleaf"), "the other meeting is not in the topic:\n{}", scoped.prompt);
        assert_eq!(stashed_meetings(&fx.c, &scoped.prepare_id), vec![fx.filed]);
        assert_eq!(scoped.exclusion, ExclusionOutcome::Allowed);
        let finalized = finalize_ask(&fx.store, &scoped.prepare_id, "March [1].", "m", llm::TokenUsage::default());
        assert_eq!(finalized.citations.len(), 1);
    }

    #[test]
    fn a_topic_with_nothing_matching_sends_nothing() {
        let _serial = serialize_prepared_asks();
        let fx = topic_fx();
        let empty = fx.store.create_seed_topic("Cedar", &[], None).unwrap().topic_id;
        // Over the spend cap too: an empty scope returns before the cap check (which records an
        // event), so the answer is "nothing in this topic", not a cap error.
        let routing = tripped_cap_routing(&fx.store);
        let events = |c: &rusqlite::Connection| -> i64 { c.query_row("SELECT COUNT(*) FROM spend_cap_event", [], |r| r.get(0)).unwrap() };
        let before = events(&fx.c);
        for topic in [empty, 999_999] {
            let data = prepare_ask_in_topic(&fx.store, topic, "roadmap", 5, &[], &routing).unwrap();
            assert!(data.scope_empty, "topic {topic}");
            assert_eq!((data.prepare_id.as_str(), data.prompt.as_str()), ("", ""));
            assert_eq!(data.exclusion, ExclusionOutcome::Allowed);
            assert!(!prepared_asks().lock().unwrap().contains_key(""), "nothing is stashed");
        }
        assert_eq!(events(&fx.c), before, "no spend-cap event is recorded for an ask that sends nothing");
        // A topic with members that do match still meets the cap.
        assert!(matches!(prepare_ask_in_topic(&fx.store, fx.birch, "roadmap", 5, &[], &routing), Err(ApiError::SpendCap(_))));
    }

    /// A topic whose only matches are Gmail threads, asked against a provider that may not
    /// receive Google data: Limited Use empties the context, so the topic is reported empty
    /// (nothing stashed, no prompt) and no spend-cap event is recorded, even over the cap.
    #[test]
    fn a_gmail_only_topic_for_a_provider_without_google_data_is_scope_empty() {
        let _serial = serialize_prepared_asks();
        let fx = topic_fx();
        fx.store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "m-1".into(),
                ts_unix_ms: now_unix_ms() - 1_000,
                from: "Priya <priya@x.com>".into(),
                to: "me@x.com".into(),
                subject: "Pricing".into(),
                body_text: "GMAILSECRET the pricing budget is 40k".into(),
                gmail_thread_id: "thread-1".into(),
                account_email: "me@x.com".into(),
                ..Default::default()
            }])
            .unwrap();
        while fx.store.run_thread_state_sync_job(now_unix_ms()).unwrap() {}
        let email: i64 = fx.c.query_row("SELECT id FROM items WHERE kind = 'email'", [], |r| r.get(0)).unwrap();
        let mail = fx.store.create_seed_topic("Mailonly", &[], None).unwrap().topic_id;
        file_under(&fx.c, mail, "email", email);
        let tripped = tripped_cap_routing(&fx.store);
        let events = |c: &rusqlite::Connection| -> i64 { c.query_row("SELECT COUNT(*) FROM spend_cap_event", [], |r| r.get(0)).unwrap() };
        let before = events(&fx.c);
        let ask = |json: &str| {
            let mut routing = RoutingConfig::from_json(Some(json));
            routing.global_daily_cap_cents = tripped.global_daily_cap_cents;
            let call = crate::routing::LimitedUseSettings::from_routing_json_str(Some(json));
            prepare_ask_in_topic_for(&fx.store, mail, "pricing budget", 10, &[], &routing, &call)
        };
        let cloud = r#"{"preferred_provider":"ollama","streamed_ollama_model":"gpt-oss:120b-cloud"}"#;
        let data = ask(cloud).unwrap();
        assert!(data.scope_empty, "Limited Use left nothing in the topic");
        assert_eq!((data.prepare_id.as_str(), data.prompt.as_str()), ("", ""));
        assert!(!prepared_asks().lock().unwrap().contains_key(""), "nothing is stashed");
        assert_eq!(events(&fx.c), before, "no spend-cap event for an ask that sends nothing");
        // Control: a local model may read the thread, so the same topic meets the cap.
        let local = r#"{"preferred_provider":"ollama","streamed_ollama_model":"gemma4:12b"}"#;
        assert!(matches!(ask(local), Err(ApiError::SpendCap(_))));
    }

    #[test]
    fn a_follow_up_after_a_local_answer_stays_local() {
        let _serial = serialize_prepared_asks();
        let fx = topic_fx();
        let acme = fx.store.create_seed_topic("Acme", &[], None).unwrap();
        fx.store
            .set_client_control(&acme.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        let hosted = RoutingConfig { preferred_provider: "anthropic".into(), anthropic_key: Some("k".into()), ..RoutingConfig::default() };

        // Today: a first question naming the client is refused for a hosted stream.
        assert!(prepare_ask(&fx.store, "What did Acme decide about the roadmap?", 5, &[], &hosted).is_err());
        assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));

        // The earlier exchange was answered on this computer from the client's material.
        let local_answer = "They moved the launch to May.";
        mark_local_only(&fx.store, local_answer);
        let history = [
            llm::ChatTurn { role: llm::ChatRole::User, text: "what was decided?".into() },
            llm::ChatTurn { role: llm::ChatRole::Assistant, text: local_answer.into() },
        ];
        // Control: without that history the follow-up goes hosted in either scope.
        let open = prepare_ask_in_topic(&fx.store, fx.birch, "and the roadmap?", 5, &[], &hosted).unwrap();
        assert!(!open.scope_empty && !open.prompt.is_empty());
        discard_ask(&open.prepare_id);

        let everything = prepare_ask(&fx.store, "and the roadmap?", 5, &history, &hosted);
        assert!(everything.is_err(), "Everything: the follow-up is refused for a hosted stream");
        assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
        let topic = prepare_ask_in_topic(&fx.store, fx.birch, "and the roadmap?", 5, &history, &hosted);
        assert!(topic.is_err(), "a topic: the follow-up is refused for a hosted stream");
        assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
        // Both refusals are the exclusion refusal the shell answers locally on.
        let (Err(a), Err(b)) = (everything, topic) else { unreachable!() };
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        assert_eq!(format!("{a:?}"), format!("{:?}", crate::exclusion::refused()));

        // On this computer the topic follow-up is answered, and marked local-only.
        let local = prepare_ask_in_topic(&fx.store, fx.birch, "and the roadmap?", 5, &history, &RoutingConfig::default()).unwrap();
        assert!(local.prompt.contains("birchtree"));
        assert_eq!(local.exclusion, ExclusionOutcome::LocalOnly);
        discard_ask(&local.prepare_id);
    }

    #[test]
    fn existing_prepares_report_scope_not_empty() {
        let _serial = serialize_prepared_asks();
        let store = store_with_capture_text("the quarterly roadmap review");
        let routing = RoutingConfig::default();
        let data = prepare_ask(&store, "roadmap", 5, &[], &routing).unwrap();
        assert!(!data.scope_empty);
        discard_ask(&data.prepare_id);
        // Nothing retrieved at all is still not an empty scope: the whole of memory was asked.
        let none = prepare_ask(&store, "zzzunmatched", 5, &[], &routing).unwrap();
        assert!(!none.scope_empty && !none.prepare_id.is_empty());
        discard_ask(&none.prepare_id);
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["scope_empty"], serde_json::json!(false));
    }

    /// Spec call 4: a topic page of an excluded client is asked from that page, so a hosted
    /// stream is refused with the exclusion refusal (the shell then answers on this computer),
    /// never reported as an empty topic.
    #[test]
    fn a_hosted_topic_ask_on_an_excluded_clients_topic_is_refused() {
        let _serial = serialize_prepared_asks();
        let fx = topic_fx();
        let acme = fx.store.create_seed_topic("Acme", &[], None).unwrap();
        let acme_meeting = titled_meeting(&fx.store, "Sync", 5_400_000, "the pinecone roadmap moved");
        file_under(&fx.c, acme.topic_id, "meeting", acme_meeting);
        fx.store
            .set_client_control(&acme.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        let hosted = RoutingConfig { preferred_provider: "anthropic".into(), anthropic_key: Some("k".into()), ..RoutingConfig::default() };

        let refused = prepare_ask_in_topic(&fx.store, acme.topic_id, "and the roadmap?", 5, &[], &hosted);
        let Err(e) = refused else { panic!("not refused: {refused:?}") };
        assert_eq!(format!("{e:?}"), format!("{:?}", crate::exclusion::refused()));
        assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
        // Another topic still goes hosted.
        let other = prepare_ask_in_topic(&fx.store, fx.birch, "and the roadmap?", 5, &[], &hosted).unwrap();
        assert!(!other.scope_empty && other.prompt.contains("birchtree"));
        discard_ask(&other.prepare_id);
        // On this computer the client's topic is answered, marked local-only.
        let local = prepare_ask_in_topic(&fx.store, acme.topic_id, "and the roadmap?", 5, &[], &RoutingConfig::default()).unwrap();
        assert!(local.prompt.contains("pinecone"));
        assert_eq!(local.exclusion, ExclusionOutcome::LocalOnly);
        discard_ask(&local.prepare_id);
    }

    /// An item filed under two topics is in both: a topic ask keeps it even where that topic is
    /// not the item's preferred one.
    #[test]
    fn a_topic_ask_keeps_an_item_filed_under_another_preferred_topic_too() {
        let _serial = serialize_prepared_asks();
        let fx = topic_fx();
        let umber = fx.store.create_seed_topic("Umber", &[], None).unwrap().topic_id;
        let sid: i64 = fx
            .c
            .query_row("SELECT id FROM topic_source WHERE kind = 'meeting' AND ref_id = ?1", [fx.filed], |r| r.get(0))
            .unwrap();
        fx.c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'user')", rusqlite::params![umber, sid])
            .unwrap();
        assert_eq!(fx.store.live_topic_of_item("meeting", fx.filed).unwrap(), Some(umber), "Umber is preferred");
        for topic in [fx.birch, umber] {
            let data = prepare_ask_in_topic(&fx.store, topic, "roadmap", 5, &[], &RoutingConfig::default()).unwrap();
            assert!(!data.scope_empty, "topic {topic}");
            assert_eq!(stashed_meetings(&fx.c, &data.prepare_id), vec![fx.filed], "topic {topic}");
            discard_ask(&data.prepare_id);
        }
    }

    /// Review finding: one long meeting outside the topic whose lines all outrank the topic's
    /// (more of them than 4x the limit, together over the context budget) must not starve the
    /// topic: its own matching lines still reach the context.
    #[test]
    fn a_topic_ask_is_not_starved_by_a_long_meeting_outside_the_topic() {
        let _serial = serialize_prepared_asks();
        let fx = topic_fx();
        let filler = "x".repeat(300);
        let long = store_meeting_lines(&fx.store, "All hands", 600_000, 40, |i| format!("roadmap point {i} {filler}"));
        assert!(40 * (filler.len() + 20) > llm::CONTEXT_BUDGET_CHARS, "the long meeting alone fills the budget");
        let cedar = fx.store.create_seed_topic("Cedar", &[], None).unwrap().topic_id;
        let old = store_meeting_lines(&fx.store, "Cedar sync", 20 * 86_400_000, 2, |i| format!("cedar roadmap item {i} {filler}"));
        file_under(&fx.c, cedar, "meeting", old);

        let all = prepare_ask(&fx.store, "roadmap", 5, &[], &RoutingConfig::default()).unwrap();
        let reached = stashed_meetings(&fx.c, &all.prepare_id);
        assert!(reached.contains(&long) && !reached.contains(&old), "control: the long meeting fills the whole of memory's context");
        discard_ask(&all.prepare_id);

        let data = prepare_ask_in_topic(&fx.store, cedar, "roadmap", 5, &[], &RoutingConfig::default()).unwrap();
        assert!(!data.scope_empty, "the topic has matching lines");
        let meetings = stashed_meetings(&fx.c, &data.prepare_id);
        assert_eq!(meetings, vec![old, old], "both of the topic's lines, and only them");
        assert!(data.prompt.contains("cedar roadmap item 0") && data.prompt.contains("cedar roadmap item 1"));
        discard_ask(&data.prepare_id);
    }

    /// A completed meeting `ago_ms` ago with `n` transcript lines, each a second apart.
    fn store_meeting_lines(store: &memory::Store, title: &str, ago_ms: i64, n: i64, line: impl Fn(i64) -> String) -> i64 {
        let id = titled_meeting(store, title, ago_ms, &line(0));
        let rest: Vec<memory::NewTranscriptSegment> = (1..n)
            .map(|i| memory::NewTranscriptSegment { meeting_id: id, start_cs: i * 100, speaker_label: "Ben".into(), text: line(i) })
            .collect();
        store.insert_transcript_segments(&rest).unwrap();
        id
    }
}

#[cfg(test)]
mod sourced_tests {
    // The sourced Ask calls: the numbers the model saw, retrieval bounded by date, app and person, the
    // route preview, and "on this computer or fail". Fixtures use a recording transport behind the real
    // routing, so what a link is sent is what the test reads.

    use std::sync::{Arc, Mutex};

    use memory::find::{FindFilter, PersonFilter};
    use memory::topics::client_controls::ClientControl;

    use crate::ask::{self, AskScope, RouteMode};
    use crate::routing::{with_test_transport, RoutingConfig};

    const R0: i64 = 1_000_000;
    const R1: i64 = 2_000_000;
    const CANARY: &str = "zebracanary";

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<String>>>);

    impl Log {
        fn prompts(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    struct Recorder {
        log: Log,
        reply: String,
    }

    impl llm::LlmClient for Recorder {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            self.log.0.lock().unwrap().push(prompt.to_string());
            Ok(llm::Completion { text: self.reply.clone(), token_usage: llm::TokenUsage::default() })
        }
    }

    /// Runs `f` with every link routing builds made of a recorder: hosted links record into `hosted`, on-device
    /// links into `local`, each replying `reply`.
    fn recording<R>(hosted: &Log, local: &Log, reply: &str, f: impl FnOnce() -> R) -> R {
        let (h, l, reply) = (hosted.clone(), local.clone(), reply.to_string());
        with_test_transport(
            move |provider, model| -> Box<dyn llm::LlmClient> {
                let log = if llm::is_hosted(provider, model) { h.clone() } else { l.clone() };
                Box::new(Recorder { log, reply: reply.clone() })
            },
            f,
        )
    }

    struct Fx {
        _dir: tempfile::TempDir,
        s: memory::Store,
        c: rusqlite::Connection,
    }

    fn capture(s: &memory::Store, n: i64, ts: i64, app: &str, title: &str, text: &str) -> i64 {
        s.insert_capture_item(&memory::NewCaptureItem {
            device_id: "mac-1".into(),
            ts_unix_ms: ts,
            app_bundle_id: app.into(),
            window_title: title.into(),
            url: None,
            text: text.into(),
            hash: n,
            source: "ax".into(),
        })
        .unwrap()
        .unwrap()
    }

    fn meeting(s: &memory::Store, start: i64, title: &str, attendees: &[&str], lines: &[&str]) -> i64 {
        let id = s
            .insert_meeting(&memory::NewMeeting {
                device_id: "mac-1".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 60_000),
                title: title.into(),
                attendees: attendees.iter().map(|a| a.to_string()).collect(),
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        let segs: Vec<memory::NewTranscriptSegment> = lines
            .iter()
            .enumerate()
            .map(|(i, t)| memory::NewTranscriptSegment { meeting_id: id, start_cs: i as i64 * 100, speaker_label: "Speaker 1".into(), text: t.to_string() })
            .collect();
        s.insert_transcript_segments(&segs).unwrap();
        // Not from Google Calendar: Limited Use has its own tests.
        s.set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_OTHER).unwrap();
        id
    }

    fn email(c: &rusqlite::Connection, ext: &str, thread: &str, from: &str, ts: i64, body: &str) {
        c.execute(
            "INSERT OR IGNORE INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, status, summarize_status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('gmail', ?1, 'me@x.com', ?2, 1, ?3, ?4, 'waiting_on_me', 'pending_summary', 1, 1)",
            rusqlite::params![thread, format!("Subject {thread}"), ts, from],
        )
        .unwrap();
        c.execute(
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) VALUES ('d', 'email', 'gmail', ?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                ts,
                serde_json::to_string(&[from, "me@x.com"]).unwrap(),
                format!("Subject {thread}"),
                body,
                serde_json::json!({"gmail_thread_id": thread, "account_email": "me@x.com"}).to_string(),
                ext
            ],
        )
        .unwrap();
    }

    /// One of every kind inside `[R0, R1)` and one outside, each holding "pricing".
    fn world() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.db");
        let s = crate::test_store::open(&path, dir.path());
        let c = crate::test_store::raw(&path);
        capture(&s, 1, 1_500_000, "com.slack", "in slack", "pricing review alpha");
        capture(&s, 2, 500_000, "com.slack", "before", "pricing review before");
        capture(&s, 3, 2_500_000, "com.slack", "after", "pricing review after");
        capture(&s, 4, 1_600_000, "com.notes", "in notes", "pricing review notes");
        capture(&s, 5, 1_700_000, "com.slack", "sam", "lunch with Sam Okafor about pricing");
        let m_in = meeting(&s, 1_200_000, "Planning", &["Sam Okafor"], &["pricing starts here"]);
        meeting(&s, 5_000_000, "Later", &["Sam Okafor"], &["pricing later"]);
        meeting(&s, 1_300_000, "Standup", &["Ana"], &["pricing in the standup"]);
        for (m, ts, text) in [(m_in, 1_200_500, "pricing slide in"), (m_in + 1, 5_000_500, "pricing slide out")] {
            s.insert_meeting_snapshot(&memory::NewMeetingSnapshot { meeting_id: m, captured_unix_ms: ts, file_path: format!("/s/{ts}.png"), width: 1, height: 1, ocr_text: text.into() })
                .unwrap();
        }
        s.upsert_person("name:sam", "Sam Okafor", Some("sam@x.com")).unwrap();
        email(&c, "e1", "t-in", "sam@x.com", 1_700_000, "pricing mail in");
        email(&c, "e2", "t-out", "sam@x.com", 9_000_000, "pricing mail out");
        email(&c, "e3", "t-other", "zed@x.com", 1_750_000, "pricing mail other");
        for (app, end, text) in [("com.slack", 1_800_000, "pricing episode in"), ("com.slack", 9_800_000, "pricing episode out"), ("com.notes", 1_810_000, "pricing episode notes")] {
            c.execute(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('d', ?1, 'Window', ?2, ?3, 1, 2, 2, ?4, 'summarized', 1, 1)",
                rusqlite::params![app, end - 1_000, end, text],
            )
            .unwrap();
        }
        c.execute("INSERT INTO routine (id, name, template, schedule_hhmm, created_at_unix_ms) VALUES (1, 'Focus', 'focus_time', '09:00', 1)", []).unwrap();
        for (ts, text) in [(1_900_000, "pricing run in"), (9_900_000, "pricing run out")] {
            c.execute("INSERT INTO routine_run (routine_id, ran_at_unix_ms, status, output_summary, output_json) VALUES (1, ?1, 'success', ?2, '{}')", rusqlite::params![ts, text]).unwrap();
        }
        Fx { _dir: dir, s, c }
    }

    fn range() -> FindFilter {
        FindFilter { start_unix_ms: Some(R0), end_unix_ms: Some(R1), ..Default::default() }
    }

    /// The Ollama model streamed against is on this computer's own host, so Gmail text may ride (Limited Use).
    fn loopback() -> crate::routing::LimitedUseSettings {
        crate::routing::LimitedUseSettings { streamed_ollama_model: Some("gemma-test".into()), ..Default::default() }
    }

    fn scope(filter: FindFilter) -> AskScope {
        AskScope { filter, require_local: false }
    }

    fn local_routing() -> RoutingConfig {
        RoutingConfig::default()
    }

    fn hosted_routing() -> RoutingConfig {
        RoutingConfig { preferred_provider: "anthropic".into(), anthropic_key: Some("k".into()), ollama_model: "gemma-test".into(), ..RoutingConfig::default() }
    }

    fn prepare(fx: &Fx, q: &str, routing: &RoutingConfig, scope: &AskScope) -> ask::PreparedAskSourced {
        let r = ask::prepare_ask_sourced(&fx.s, q, 20, &[], routing, &loopback(), scope).unwrap();
        ask::discard_ask(&r.data.prepare_id);
        r
    }

    /// The `[n] (label) text` lines of a prompt, as the model read them.
    fn numbered_lines(prompt: &str) -> Vec<(usize, String)> {
        prompt
            .lines()
            .filter_map(|l| {
                let rest = l.strip_prefix('[')?;
                let (n, text) = rest.split_once("] ")?;
                Some((n.parse().ok()?, text.to_string()))
            })
            .collect()
    }

    #[test]
    fn the_numbers_in_the_prompt_are_the_numbers_of_the_sources_returned_with_it() {
        let fx = world();
        let p = ask::prepare_ask_sourced(&fx.s, "pricing review", 20, &[], &local_routing(), &loopback(), &AskScope::default()).unwrap();
        let lines = numbered_lines(&p.data.prompt);
        assert!(lines.len() >= 5, "{}", p.data.prompt);
        assert_eq!(lines.len(), p.sources.len());
        for (line, src) in lines.iter().zip(&p.sources) {
            assert_eq!(line.0, src.n, "the number the model saw");
            assert!(src.card.found);
            if src.card.kind == "thread" {
                // The prompt shows the thread's summary and its matching message; the card shows the stored summary, when there is one.
                assert!(line.1.contains(&src.card.title), "{} / {:?}", line.1, src.card);
                continue;
            }
            let words = src.card.text.as_deref().expect("a found row has its text");
            let first_words: String = words.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
            assert!(line.1.contains(&first_words), "[{}] {} is not the card {:?}", line.0, line.1, src.card);
            assert!(src.card.found);
        }
        // Citing: first-cited order, repeats once, out of range dropped, only the numbers of this list.
        let f = ask::finalize_ask_sourced(&fx.s, &p.data.prepare_id, &format!("A [2] and [1], again [2]. Bad [{}] [0] [999].", lines.len() + 1), "m", llm::TokenUsage::default());
        assert_eq!(f.cited, vec![2, 1]);
        assert_eq!(f.cited_sources.iter().map(|s| s.n).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!((f.cited_sources[0].kind.as_str(), f.cited_sources[0].id), (p.sources[1].card.kind.as_str(), p.sources[1].card.id));
        assert_eq!(f.citations.len(), 2);
        // Once finalized the id is spent: nothing is cited.
        let again = ask::finalize_ask_sourced(&fx.s, &p.data.prepare_id, "[1]", "m", llm::TokenUsage::default());
        assert!(again.cited.is_empty() && again.cited_sources.is_empty());
    }

    #[test]
    fn the_blocking_answer_numbers_the_context_of_the_link_that_answered() {
        let fx = world();
        let (hosted, local) = (Log::default(), Log::default());
        let a = recording(&hosted, &local, "It is [2] and also [1][2]. Not [77].", || {
            ask::answer_sourced(&fx.s, "pricing review", "gemma-test", 20, &[], &local_routing(), &AskScope::default()).unwrap()
        });
        assert_eq!(local.prompts().len(), 1);
        let lines = numbered_lines(&local.prompts()[0]);
        assert_eq!(lines.len(), a.sources.len());
        assert_eq!(a.cited, vec![2, 1]);
        assert_eq!(a.cited_sources.len(), 2);
        assert_eq!(a.detailed.answer.citations.len(), 2, "the old citations field is unchanged");
        assert!(a.answered_locally);
        let json = serde_json::to_value(&a).unwrap();
        for key in ["text", "citations", "token_usage", "exclusion", "answered_by", "sources", "cited", "cited_sources", "answered_locally"] {
            assert!(json.get(key).is_some(), "{key} in {json}");
        }
    }

    /// Pin (1): with no filter, the sourced calls retrieve what the old calls retrieve, into the same prompt bytes.
    #[test]
    fn no_filter_retrieves_exactly_what_the_old_calls_do() {
        let fx = world();
        let (h1, l1, h2, l2) = (Log::default(), Log::default(), Log::default(), Log::default());
        for q in ["pricing review", "what did Sam say about pricing today"] {
            let old = recording(&h1, &l1, "ok", || ask::answer_detailed(&fx.s, q, "gemma-test", 20, &[], &local_routing()).unwrap());
            let new = recording(&h2, &l2, "ok", || ask::answer_sourced(&fx.s, q, "gemma-test", 20, &[], &local_routing(), &AskScope::default()).unwrap());
            assert_eq!(l1.prompts().last(), l2.prompts().last(), "the blocking prompt for {q:?}");
            assert_eq!(old.answer.text, new.detailed.answer.text);
            // The streamed pair.
            let routing = local_routing();
            let a = ask::prepare_ask_for(&fx.s, q, 20, &[], &routing, &crate::routing::LimitedUseSettings::default()).unwrap();
            let b = ask::prepare_ask_sourced(&fx.s, q, 20, &[], &routing, &crate::routing::LimitedUseSettings::default(), &AskScope::default()).unwrap();
            assert_eq!(a.prompt, b.data.prompt, "the streamed prompt for {q:?}");
            ask::discard_ask(&a.prepare_id);
            ask::discard_ask(&b.data.prepare_id);
        }
    }

    /// Pin (2): with a date filter nothing outside the range reaches the prompt, for every kind.
    #[test]
    fn a_date_filter_keeps_every_kind_inside_the_range() {
        let fx = world();
        let p = prepare(&fx, "pricing review", &local_routing(), &scope(range()));
        let kinds: std::collections::HashSet<&str> = p.sources.iter().map(|s| s.card.kind.as_str()).collect();
        for k in ["capture", "transcript", "snapshot", "thread", "episode", "routine_run"] {
            assert!(kinds.contains(k), "{k} missing: {kinds:?}");
        }
        for s in &p.sources {
            let ts = s.card.ts_unix_ms.unwrap();
            assert!((R0..R1).contains(&ts), "[{}] {} at {ts} is outside the range", s.n, s.card.kind);
        }
        for line in numbered_lines(&p.data.prompt) {
            for outside in ["before", "after", "later", "out"] {
                assert!(!line.1.split_whitespace().any(|w| w == outside), "an outside item reached the prompt: {}", line.1);
            }
        }
        // The question's own window ("today") is narrowed by the filter, which wins: a range from long ago holds nothing from today.
        let old = FindFilter { start_unix_ms: Some(1), end_unix_ms: Some(10), ..Default::default() };
        let p = prepare(&fx, "what happened today", &local_routing(), &scope(old));
        assert!(p.sources.is_empty(), "{:?}", p.sources);
    }

    /// Pin (3): with an app filter only captures of that app.
    #[test]
    fn an_app_filter_keeps_only_that_apps_captures() {
        let fx = world();
        let f = FindFilter { app_bundle_id: Some("com.notes".into()), ..Default::default() };
        let p = prepare(&fx, "pricing review", &local_routing(), &scope(f));
        assert!(!p.sources.is_empty());
        for s in &p.sources {
            assert_eq!(s.card.kind, "capture", "{:?}", s.card);
            assert_eq!(s.card.app_bundle_id.as_deref(), Some("com.notes"));
        }
    }

    /// Pin (4): with a person filter, per the table.
    #[test]
    fn a_person_filter_follows_the_table() {
        let fx = world();
        let f = FindFilter { person: Some(PersonFilter { name: "Sam Okafor".into(), person_key: Some("name:sam".into()) }), ..range() };
        let p = prepare(&fx, "pricing", &local_routing(), &scope(f));
        let by_kind = |k: &str| p.sources.iter().filter(|s| s.card.kind == k).collect::<Vec<_>>();
        // Lines of the meeting Sam attended, not the standup or the later meeting.
        let lines: Vec<&str> = by_kind("transcript").iter().map(|s| s.card.text.as_deref().unwrap()).collect();
        assert_eq!(lines, vec!["pricing starts here"]);
        assert_eq!(by_kind("snapshot").len(), 1);
        // Mail from the address on record, not Zed's.
        assert_eq!(by_kind("thread").iter().map(|s| s.card.title.as_str()).collect::<Vec<_>>(), vec!["Subject t-in"]);
        // Episodes and runs are hidden; captures are the ones that mention the name as a phrase.
        assert!(by_kind("episode").is_empty() && by_kind("routine_run").is_empty());
        let caps: Vec<&str> = by_kind("capture").iter().map(|s| s.card.text.as_deref().unwrap()).collect();
        assert_eq!(caps, vec!["lunch with Sam Okafor about pricing"]);
    }

    /// The excluded client's material, filed under it and never naming it.
    fn excluded_fixture() -> Fx {
        let fx = world();
        let acme = fx.s.create_seed_topic("Acme", &["Roadrunner Co".to_string()], None).unwrap();
        // A capture that names the client by its alias, and a meeting filed under it that never names it.
        capture(&fx.s, 99, 1_450_000, "com.slack", "client", &format!("pricing {CANARY} secret numbers for Roadrunner Co"));
        let m = meeting(&fx.s, 1_460_000, "Quarterly sync", &["Sam Okafor"], &[&format!("pricing {CANARY} in the client meeting")]);
        fx.c.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES ('meeting', ?1, 1, 1)", rusqlite::params![m]).unwrap();
        let sid = fx.c.last_insert_rowid();
        fx.c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", rusqlite::params![acme.topic_id, sid]).unwrap();
        fx.s.set_client_control(&acme.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        fx
    }

    /// Pin (5): an excluded client's rows never reach a hosted model, whatever the filters.
    #[test]
    fn an_excluded_clients_rows_never_reach_a_hosted_model_whatever_the_filters() {
        let fx = excluded_fixture();
        let filters = [
            FindFilter::default(),
            range(),
            FindFilter { app_bundle_id: Some("com.slack".into()), ..Default::default() },
            FindFilter { person: Some(PersonFilter { name: "Sam Okafor".into(), person_key: None }), ..range() },
            FindFilter { start_unix_ms: Some(0), end_unix_ms: Some(i64::MAX), ..Default::default() },
        ];
        for f in filters {
            let (hosted, local) = (Log::default(), Log::default());
            let a = recording(&hosted, &local, "An answer.", || ask::answer_sourced(&fx.s, "pricing", "gemma-test", 20, &[], &hosted_routing(), &scope(f.clone())).unwrap());
            assert_eq!(hosted.prompts().len(), 1, "{f:?}");
            assert!(!hosted.prompts()[0].contains(CANARY), "{f:?}: {}", hosted.prompts()[0]);
            assert!(a.sources.iter().all(|s| s.card.text.as_deref().is_none_or(|t| !t.contains(CANARY))), "{f:?}");
            // The streamed prompt too.
            let p = ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &hosted_routing(), &crate::routing::LimitedUseSettings::default(), &scope(f.clone())).unwrap();
            assert!(!p.data.prompt.contains(CANARY), "{f:?}");
            assert!(p.sources.iter().all(|s| s.card.text.as_deref().is_none_or(|t| !t.contains(CANARY))));
            ask::discard_ask(&p.data.prepare_id);
        }
        // On this computer the same filters read all of memory: the filter never loosens, and never tightens, what stays local.
        let p = prepare(&fx, "pricing", &local_routing(), &scope(range()));
        assert!(p.data.prompt.contains(CANARY));
    }

    #[test]
    fn require_local_never_reaches_a_hosted_model_and_fails_when_nothing_local_can_answer() {
        let fx = world();
        let local_only = AskScope { require_local: true, ..Default::default() };
        // Blocking: a hosted preferred provider, but the call is held to on-device links.
        let (hosted, local) = (Log::default(), Log::default());
        recording(&hosted, &local, "on device", || {
            let a = ask::answer_sourced(&fx.s, "pricing", "gemma-test", 20, &[], &hosted_routing(), &local_only).unwrap();
            assert!(a.detailed.answered_by.local);
        });
        assert_eq!((hosted.prompts().len(), local.prompts().len()), (0, 1));
        // No on-device link (Ollama turned off, no bundled model): the existing provider failure, nothing sent.
        let (hosted, local) = (Log::default(), Log::default());
        let no_local = RoutingConfig { use_local_model: false, ..hosted_routing() };
        let r = recording(&hosted, &local, "x", || ask::answer_sourced(&fx.s, "pricing", "gemma-test", 20, &[], &no_local, &local_only));
        assert!(matches!(r, Err(crate::ApiError::ProviderFailure)), "{r:?}");
        assert_eq!((hosted.prompts().len(), local.prompts().len()), (0, 0));
        // Streaming: routing is local-only for this call, so the stream goes to Ollama.
        let p = ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &hosted_routing(), &crate::routing::LimitedUseSettings::default(), &local_only).unwrap();
        assert_eq!((p.provider.as_str(), p.answered_locally), ("ollama", true));
        ask::discard_ask(&p.data.prepare_id);
        // ... and is refused, as local-only mode is, when the routing names no Ollama model (core cannot know it is local).
        let nameless = RoutingConfig { ollama_model: String::new(), ..hosted_routing() };
        assert!(ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &nameless, &crate::routing::LimitedUseSettings::default(), &local_only).is_err());
        let cloud = RoutingConfig { ollama_model: "gpt-oss:120b-cloud".into(), ..hosted_routing() };
        assert!(ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &cloud, &crate::routing::LimitedUseSettings::default(), &local_only).is_err());
        // Without require_local the same routing streams to the hosted provider.
        let p = ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &hosted_routing(), &crate::routing::LimitedUseSettings::default(), &AskScope::default()).unwrap();
        assert_eq!((p.provider.as_str(), p.answered_locally), ("anthropic", false));
        ask::discard_ask(&p.data.prepare_id);
    }

    fn preview(fx: &Fx, q: &str, routing: &RoutingConfig, scope: &AskScope, mode: RouteMode) -> ask::RoutePreview {
        ask::ask_route_preview(&fx.s, q, &[], "gemma-test", routing, scope, mode)
    }

    /// Every branch of the route preview, for both calls.
    #[test]
    fn the_route_preview_says_where_the_call_would_answer() {
        let fx = world();
        let none = AskScope::default();
        let local = AskScope { require_local: true, ..Default::default() };

        // Preferred hosted, a key held: hosted, naming the provider; the call goes there.
        for mode in [RouteMode::Blocking, RouteMode::Stream] {
            let p = preview(&fx, "pricing", &hosted_routing(), &none, mode);
            assert_eq!((p.destination, p.local, p.provider.as_deref(), p.reason, p.require_local), ("hosted", false, Some("anthropic"), if mode == RouteMode::Blocking { "first_link_is_hosted" } else { "preferred_provider" }, false), "{mode:?}");
            assert!(p.may_fall_back || mode == RouteMode::Blocking && false || mode == RouteMode::Stream);
        }
        // Preferred local (the default): local, and the call is told to stay there. A stream that is told
        // to stay local forces local-only routing, which refuses an Ollama model the routing does not name, so
        // the preview of a stream says so rather than promise a local answer the call then refuses.
        let p = preview(&fx, "pricing", &local_routing(), &none, RouteMode::Blocking);
        assert_eq!((p.destination, p.local, p.provider.as_deref(), p.require_local), ("local", true, Some("ollama"), true));
        let named = RoutingConfig { ollama_model: "gemma-test".into(), ..local_routing() };
        let p = preview(&fx, "pricing", &named, &none, RouteMode::Stream);
        assert_eq!((p.destination, p.local, p.provider.as_deref(), p.require_local), ("local", true, Some("ollama"), true));
        let p = preview(&fx, "pricing", &local_routing(), &none, RouteMode::Stream);
        assert_eq!((p.destination, p.reason, p.require_local), ("refused", "local_only_cloud_or_unknown_model", false));
        // Local-only mode: local (blocking) or, with no Ollama model named, refused (streaming fails closed).
        let sensitive = RoutingConfig { sensitive: true, ..hosted_routing() };
        assert_eq!(preview(&fx, "pricing", &sensitive, &none, RouteMode::Blocking).reason, "local_only_mode");
        assert_eq!(preview(&fx, "pricing", &sensitive, &none, RouteMode::Stream).destination, "local");
        let unnamed = RoutingConfig { ollama_model: String::new(), ..sensitive.clone() };
        let p = preview(&fx, "pricing", &unnamed, &none, RouteMode::Stream);
        assert_eq!((p.destination, p.reason), ("refused", "local_only_cloud_or_unknown_model"));
        // Required local overrides a hosted preference.
        for mode in [RouteMode::Blocking, RouteMode::Stream] {
            let p = preview(&fx, "pricing", &hosted_routing(), &local, mode);
            assert_eq!((p.destination, p.reason, p.provider.as_deref()), ("local", "required_local", Some("ollama")), "{mode:?}");
        }
        // Nothing configured at all: no key, no local model.
        let nothing = RoutingConfig { use_local_model: false, preferred_provider: "anthropic".into(), ..RoutingConfig::default() };
        let p = preview(&fx, "pricing", &nothing, &none, RouteMode::Blocking);
        assert_eq!((p.destination, p.reason), ("refused", "no_provider"));
        // A spent monthly budget: blocking skips to the next provider (here none is left); a stream falls back to blocking.
        fx.s.log_usage(&memory::NewUsageLogEntry {
            user_id: None,
            provider: "anthropic".into(),
            model: "m".into(),
            route: Some("ask".into()),
            input_tokens: None,
            output_tokens: None,
            estimated_cost_cents: 500,
            cache_hit: false,
            trace_id: "t".into(),
            ts_unix_ms: crate::util::now_unix_ms(),
        })
        .unwrap();
        fx.s.set_provider_budgets(&[("anthropic".to_string(), 500)]).unwrap();
        let rich = RoutingConfig { user_daily_cap_cents: Some(100_000), global_daily_cap_cents: Some(100_000), ..hosted_routing() };
        let stream = preview(&fx, "pricing", &rich, &none, RouteMode::Stream);
        assert_eq!((stream.destination, stream.reason, stream.provider.as_deref()), ("unknown", "budget_spent_falls_back_to_blocking", Some("anthropic")));
        let blocking = preview(&fx, "pricing", &rich, &none, RouteMode::Blocking);
        assert_eq!((blocking.destination, blocking.provider.as_deref()), ("local", Some("ollama")), "the chain skips the spent provider: {blocking:?}");
        assert_eq!(preview(&fx, "pricing", &RoutingConfig { use_local_model: false, ..rich.clone() }, &none, RouteMode::Blocking).reason, "budget_spent");
        fx.s.set_provider_budgets(&[]).unwrap();
        // A spent spend cap.
        let tripped = RoutingConfig { user_daily_cap_cents: Some(1), ..hosted_routing() };
        fx.s.log_usage(&memory::NewUsageLogEntry {
            user_id: None,
            provider: "openai".into(),
            model: "m".into(),
            route: Some("ask".into()),
            input_tokens: None,
            output_tokens: None,
            estimated_cost_cents: 50,
            cache_hit: false,
            trace_id: "t2".into(),
            ts_unix_ms: crate::util::now_unix_ms(),
        })
        .unwrap();
        for mode in [RouteMode::Blocking, RouteMode::Stream] {
            assert_eq!(preview(&fx, "pricing", &tripped, &none, mode).reason, "spend_cap", "{mode:?}");
        }
    }

    #[test]
    fn a_question_naming_an_excluded_client_is_previewed_local_and_answered_local() {
        let fx = excluded_fixture();
        let none = AskScope::default();
        for mode in [RouteMode::Blocking, RouteMode::Stream] {
            let p = preview(&fx, "what did Acme say about pricing?", &hosted_routing(), &none, mode);
            assert_eq!((p.destination, p.reason, p.require_local, p.provider.as_deref()), ("local", "names_excluded_client", true, Some("ollama")), "{mode:?}");
            // The same question without the client's name is previewed hosted.
            assert_eq!(preview(&fx, "what about pricing?", &hosted_routing(), &none, mode).destination, "hosted");
            // No on-device model: refused, never hosted.
            let no_local = RoutingConfig { use_local_model: false, ollama_model: String::new(), ..hosted_routing() };
            assert_eq!(preview(&fx, "what did Acme say?", &no_local, &none, mode).reason, "excluded_client_no_local_model", "{mode:?}");
        }
        // Previewing recorded no refusal for a later failure to take.
        assert!(crate::last_provider_failure().is_none());
        // The filter's person names the client: asking about them is asking about the client.
        let direct = ask::ask_route_preview(&fx.s, "what did Acme say about pricing?", &[], "gemma-test", &hosted_routing(), &none, RouteMode::Blocking);
        assert_eq!((direct.destination, direct.reason), ("local", "names_excluded_client"));
        let about_acme = AskScope { filter: FindFilter { person: Some(PersonFilter { name: "Acme".into(), person_key: None }), ..Default::default() }, require_local: false };
        assert_eq!(preview(&fx, "what about pricing?", &hosted_routing(), &about_acme, RouteMode::Blocking).reason, "names_excluded_client");
        // The call does what the preview said.
        let (hosted, local) = (Log::default(), Log::default());
        let a = recording(&hosted, &local, "An answer.", || ask::answer_sourced(&fx.s, "what did Acme say about pricing?", "gemma-test", 20, &[], &hosted_routing(), &none).unwrap());
        assert_eq!((hosted.prompts().len(), local.prompts().len()), (0, 1));
        assert!(a.detailed.answered_by.local);
        // A stream of it is refused (the shell asks again on this computer), as for every Ask.
        assert!(ask::prepare_ask_sourced(&fx.s, "what did Acme say?", 20, &[], &hosted_routing(), &crate::routing::LimitedUseSettings::default(), &none).is_err());
    }

    /// Finding: the date filter wins over a "yesterday" in the question, and the label and the range returned say so.
    #[test]
    fn a_date_filter_wins_over_a_day_word_and_the_range_used_is_returned() {
        use chrono::TimeZone;
        let fx = world();
        // The filter is a range long ago; the question says "yesterday". The filter's range is read, by the
        // question's words, not an empty window.
        let p = ask::prepare_ask_sourced(&fx.s, "what happened yesterday with pricing", 20, &[], &local_routing(), &loopback(), &scope(range())).unwrap();
        ask::discard_ask(&p.data.prepare_id);
        assert!(!p.sources.is_empty(), "the filter's range holds matches: {}", p.data.prompt);
        for s in &p.sources {
            assert!((R0..R1).contains(&s.card.ts_unix_ms.unwrap()), "[{}] {} is outside the range", s.n, s.card.kind);
        }
        assert_eq!(p.date_range, Some(ask::UsedRange { start_unix_ms: Some(R0), end_unix_ms: Some(R1) }));
        let day = |ms: i64| chrono::Local.timestamp_millis_opt(ms).single().unwrap().format("%B %-d, %Y").to_string();
        let want = if day(R0) == day(R1 - 1) { day(R0) } else { format!("the period from {} to {}", day(R0), day(R1 - 1)) };
        assert!(p.data.prompt.contains(&format!("The question is about {want}.")), "the label names the range used: {}", p.data.prompt);
        assert!(!p.data.prompt.contains("yesterday,"), "no stale 'yesterday' label: {}", p.data.prompt);
        // The blocking call says the same.
        let (hosted, local) = (Log::default(), Log::default());
        let a = recording(&hosted, &local, "Fine.", || ask::answer_sourced(&fx.s, "what happened yesterday with pricing", "gemma-test", 20, &[], &local_routing(), &scope(range())).unwrap());
        assert_eq!(a.date_range, p.date_range);
        assert!(local.prompts()[0].contains(&want));
        // A date filter and no day word: no label, the same range.
        let p = prepare(&fx, "pricing", &local_routing(), &scope(range()));
        assert!(!p.data.prompt.contains("The question is about"));
        assert_eq!(p.date_range, Some(ask::UsedRange { start_unix_ms: Some(R0), end_unix_ms: Some(R1) }));
        // No date filter: the question's window, labelled as before, and reported.
        let app = FindFilter { app_bundle_id: Some("com.slack".into()), ..Default::default() };
        let p = prepare(&fx, "what happened yesterday", &local_routing(), &scope(app));
        let r = p.date_range.expect("a yesterday question reads a window");
        assert!(r.start_unix_ms.unwrap() < r.end_unix_ms.unwrap() && p.data.prompt.contains("The question is about yesterday, "));
        // No scope at all: no range, unless the question names a day.
        assert_eq!(prepare(&fx, "pricing", &local_routing(), &AskScope::default()).date_range, None);
    }

    /// Finding: a failed read of the source cards is not "no longer on this device".
    #[test]
    fn a_failed_card_read_is_not_a_deleted_row() {
        let fx = world();
        let refs = vec![llm::ContextItemRef::Capture { id: 1 }, llm::ContextItemRef::Capture { id: 2 }];
        let ok = ask::numbered_sources(&fx.s, &refs);
        assert!(ok.iter().all(|s| s.card.found && !s.unreadable));
        // A row that is gone: not found, but readable.
        let gone = ask::numbered_sources(&fx.s, &[llm::ContextItemRef::Capture { id: 9_999 }]);
        assert!(!gone[0].card.found && !gone[0].unreadable);
        // A database error: every card is unreadable, none is dressed as deleted.
        fx.c.execute("DROP TABLE items", []).unwrap();
        let failed = ask::numbered_sources(&fx.s, &refs);
        assert_eq!(failed.len(), 2);
        assert!(failed.iter().all(|s| s.unreadable && !s.card.found && s.card.text.is_none()), "{failed:?}");
        assert_eq!(failed[1].n, 2);
        assert_eq!(serde_json::to_value(&failed[0]).unwrap()["unreadable"], true);
    }

    /// Finding: a filter that does not validate is an error from every sourced call, never a wider Ask.
    #[test]
    fn a_filter_that_does_not_validate_is_refused_by_every_sourced_call() {
        let fx = world();
        let bad = [
            FindFilter { app_bundle_id: Some(" ".into()), ..Default::default() },
            FindFilter { person: Some(PersonFilter { name: " ".into(), person_key: None }), ..Default::default() },
            FindFilter { start_unix_ms: Some(5), end_unix_ms: Some(5), ..Default::default() },
        ];
        for f in bad {
            let sc = scope(f);
            assert!(ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &local_routing(), &loopback(), &sc).is_err());
            assert!(ask::answer_sourced(&fx.s, "pricing", "gemma-test", 20, &[], &local_routing(), &sc).is_err());
            for mode in [RouteMode::Blocking, RouteMode::Stream] {
                let p = preview(&fx, "pricing", &local_routing(), &sc, mode);
                assert_eq!((p.destination, p.reason), ("refused", "invalid_filter"));
            }
        }
    }

    /// The rule the Ask button's name rests on, over a generated matrix of routings: a preview of `local` is a
    /// call told to stay local that does not refuse for a reason knowable in advance and never reaches a hosted
    /// model; a preview of `hosted` is a call that goes to the first link it names; a `refused` one fails.
    #[test]
    fn the_preview_and_the_real_call_agree_over_every_routing() {
        let fx = excluded_fixture();
        fx.s.log_usage(&memory::NewUsageLogEntry {
            user_id: None,
            provider: "anthropic".into(),
            model: "m".into(),
            route: Some("ask".into()),
            input_tokens: None,
            output_tokens: None,
            estimated_cost_cents: 500,
            cache_hit: false,
            trace_id: "t".into(),
            ts_unix_ms: crate::util::now_unix_ms(),
        })
        .unwrap();
        let (mut checked, mut seen) = (0, std::collections::BTreeSet::new());
        for preferred in ["anthropic", "openai", "ollama"] {
            for anthropic_key in [false, true] {
                for openai_key in [false, true] {
                    for local_only in [false, true] {
                        for ollama_model in ["", "gemma-test", "gpt-oss:120b-cloud"] {
                            for use_local_model in [false, true] {
                                for budget_spent in [false, true] {
                                    for named in [false, true] {
                                        fx.s.set_provider_budgets(&if budget_spent { vec![("anthropic".to_string(), 500)] } else { vec![] }).unwrap();
                                        let routing = RoutingConfig {
                                            preferred_provider: preferred.into(),
                                            anthropic_key: anthropic_key.then(|| "k".to_string()),
                                            openai_key: openai_key.then(|| "k".to_string()),
                                            sensitive: local_only,
                                            ollama_model: ollama_model.into(),
                                            use_local_model,
                                            user_daily_cap_cents: Some(100_000),
                                            global_daily_cap_cents: Some(100_000),
                                            ..RoutingConfig::default()
                                        };
                                        let q = if named { "what did Acme say about pricing?" } else { "what about pricing?" };
                                        let label = format!("{preferred} ak={anthropic_key} ok={openai_key} local_only={local_only} model={ollama_model:?} bundled={use_local_model} spent={budget_spent} named={named}");
                                        // Streaming: prepare_ask_sourced is the real call.
                                        let p = preview(&fx, q, &routing, &AskScope::default(), RouteMode::Stream);
                                        let told = AskScope { require_local: p.require_local, ..Default::default() };
                                        let real = ask::prepare_ask_sourced(&fx.s, q, 20, &[], &routing, &loopback(), &told);
                                        if let Ok(r) = &real {
                                            ask::discard_ask(&r.data.prepare_id);
                                        }
                                        seen.insert(("stream", p.destination));
                                        match p.destination {
                                            "local" => {
                                                let r = real.as_ref().unwrap_or_else(|e| panic!("stream previewed local but refused ({e:?}): {label}"));
                                                assert!(r.answered_locally && r.provider == "ollama", "stream previewed local but would stream to {} (locally: {}): {label}", r.provider, r.answered_locally);
                                            }
                                            "hosted" => {
                                                let r = real.as_ref().unwrap_or_else(|e| panic!("stream previewed hosted but failed ({e:?}): {label}"));
                                                assert!(!r.answered_locally && Some(r.provider.as_str()) == p.provider.as_deref(), "stream previewed {:?} but streams to {}: {label}", p.provider, r.provider);
                                            }
                                            "refused" => {
                                                // Nothing can be promised on this computer: a call told to stay local fails.
                                                let must_stay = AskScope { require_local: true, ..Default::default() };
                                                let r = ask::prepare_ask_sourced(&fx.s, q, 20, &[], &routing, &loopback(), &must_stay);
                                                if let Ok(r) = &r {
                                                    ask::discard_ask(&r.data.prepare_id);
                                                }
                                                assert!(r.is_err(), "stream previewed refused ({}) but a call told to stay local proceeds: {label}", p.reason);
                                            }
                                            "unknown" => {
                                                // The button promised nothing, and the shell asks with `require_local` false. The call may do
                                                // only what an unknown preview allows: hand back to the blocking path (the spent budget that
                                                // made it unknown), or stream to the provider the preview names; and a question that names an
                                                // excluded client never streams to a hosted model, whatever the label said.
                                                assert!(!p.require_local && !p.local, "an unknown preview asks for nothing and promises nothing: {label}");
                                                match &real {
                                                    Err(crate::ApiError::BudgetSpent(_)) => {}
                                                    Err(e) => panic!("stream previewed unknown ({}) but failed another way ({e:?}): {label}", p.reason),
                                                    Ok(r) => {
                                                        assert!(Some(r.provider.as_str()) == p.provider.as_deref(), "stream previewed unknown for {:?} but streams to {}: {label}", p.provider, r.provider);
                                                        assert!(!named || r.answered_locally, "a question naming an excluded client streams to a hosted model after an unknown preview: {label}");
                                                    }
                                                }
                                            }
                                            other => panic!("a stream preview is local, hosted, refused or unknown, not {other:?}: {label}"),
                                        }
                                        // Blocking: answer_sourced, through recording links, is the real call.
                                        let p = preview(&fx, q, &routing, &AskScope::default(), RouteMode::Blocking);
                                        let told = AskScope { require_local: p.require_local, ..Default::default() };
                                        let (hosted, local) = (Log::default(), Log::default());
                                        let real = recording(&hosted, &local, "Fine.", || ask::answer_sourced(&fx.s, q, "gemma-test", 20, &[], &routing, &told));
                                        seen.insert(("blocking", p.destination));
                                        match p.destination {
                                            "local" => {
                                                let a = real.as_ref().unwrap_or_else(|e| panic!("blocking previewed local but failed ({e:?}): {label}"));
                                                assert!(hosted.prompts().is_empty() && a.answered_locally, "blocking previewed local but reached a hosted link: {label}");
                                            }
                                            "hosted" => {
                                                let a = real.as_ref().unwrap_or_else(|e| panic!("blocking previewed hosted but failed ({e:?}): {label}"));
                                                assert_eq!(Some(a.detailed.answered_by.provider.as_str()), p.provider.as_deref(), "blocking previewed hosted but another link answered: {label}");
                                                assert!(local.prompts().is_empty() || p.may_fall_back, "{label}");
                                            }
                                            "refused" => {
                                                let must_stay = AskScope { require_local: true, ..Default::default() };
                                                let r = recording(&hosted, &local, "Fine.", || ask::answer_sourced(&fx.s, q, "gemma-test", 20, &[], &routing, &must_stay));
                                                assert!(r.is_err(), "blocking previewed refused ({}) but a call told to stay local answers: {label}", p.reason);
                                            }
                                            other => panic!("a blocking preview is local, hosted or refused, not {other:?}: {label}"),
                                        }
                                        checked += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        fx.s.set_provider_budgets(&[]).unwrap();
        assert_eq!(checked, 3 * 2 * 2 * 2 * 3 * 2 * 2 * 2);
        for want in [("stream", "local"), ("stream", "hosted"), ("stream", "refused"), ("stream", "unknown"), ("blocking", "local"), ("blocking", "hosted"), ("blocking", "refused")] {
            assert!(seen.contains(&want), "the matrix never reached {want:?}: {seen:?}");
        }
    }

    /// Finding: a scoped read that fails is a failed Ask, not a thinner prompt that says nothing was found.
    #[test]
    fn a_failed_scoped_read_fails_the_sourced_ask_and_leaves_the_unfiltered_ask_alone() {
        let fx = world();
        let f = range();
        // Before the failure both read fine.
        let ok = ask::prepare_ask_sourced(&fx.s, "pricing review", 20, &[], &local_routing(), &loopback(), &scope(f.clone())).unwrap();
        ask::discard_ask(&ok.data.prepare_id);
        // Only the scoped episodes read touches this table: its error must not be swallowed.
        fx.c.execute("DROP TABLE episodes", []).unwrap();
        let r = ask::prepare_ask_sourced(&fx.s, "pricing review", 20, &[], &local_routing(), &loopback(), &scope(f.clone()));
        assert!(matches!(r, Err(crate::ApiError::StoreFailure)), "{:?}", r.map(|p| p.data.prompt));
        // The blocking call fails the same way and sends nothing to any link.
        let (hosted, local) = (Log::default(), Log::default());
        let r = recording(&hosted, &local, "Nothing found.", || ask::answer_sourced(&fx.s, "pricing review", "gemma-test", 20, &[], &local_routing(), &scope(f.clone())));
        assert!(matches!(r, Err(crate::ApiError::StoreFailure)), "{:?}", r.map(|a| a.detailed.answer.text));
        assert_eq!((hosted.prompts().len(), local.prompts().len()), (0, 0), "no prompt was built from a partial read");
        // A person filter reads through the same door (episodes are hidden under a person, so it reads the
        // threads), and so does a question about a day, which reads a window.
        fx.c.execute("DROP TABLE threads", []).unwrap();
        let sam = FindFilter { person: Some(PersonFilter { name: "Sam Okafor".into(), person_key: None }), ..range() };
        assert!(matches!(ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &local_routing(), &loopback(), &scope(sam)), Err(crate::ApiError::StoreFailure)));
        let sam_today = FindFilter { person: Some(PersonFilter { name: "Sam Okafor".into(), person_key: None }), ..Default::default() };
        assert!(matches!(ask::prepare_ask_sourced(&fx.s, "what happened yesterday", 20, &[], &local_routing(), &loopback(), &scope(sam_today)), Err(crate::ApiError::StoreFailure)));
        // The unfiltered call is the old call: it reads what it did and answers.
        let (hosted, local) = (Log::default(), Log::default());
        let a = recording(&hosted, &local, "An answer.", || ask::answer_sourced(&fx.s, "pricing review", "gemma-test", 20, &[], &local_routing(), &AskScope::default()));
        assert!(a.is_ok() && local.prompts().len() == 1);
        let p = ask::prepare_ask_sourced(&fx.s, "pricing review", 20, &[], &local_routing(), &loopback(), &AskScope::default()).unwrap();
        ask::discard_ask(&p.data.prepare_id);
        // The envelopes: an invalid filter has its own kind; a store failure is the C layer's bare NULL.
        let invalid: crate::ApiResult<()> = Err(crate::ApiError::InvalidFilter);
        assert_eq!(crate::envelope::envelope_json(&invalid).unwrap(), r#"{"ok":false,"error":{"kind":"invalid_filter"}}"#);
        let failed: crate::ApiResult<()> = Err(crate::ApiError::StoreFailure);
        assert!(crate::envelope::envelope_json(&failed).is_none());
    }

    /// Finding: an invalid filter is `InvalidFilter` from every sourced call, not "not found".
    #[test]
    fn an_invalid_filter_is_its_own_error() {
        let fx = world();
        let sc = scope(FindFilter { app_bundle_id: Some(" ".into()), ..Default::default() });
        assert!(matches!(ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &local_routing(), &loopback(), &sc), Err(crate::ApiError::InvalidFilter)));
        assert!(matches!(ask::answer_sourced(&fx.s, "pricing", "gemma-test", 20, &[], &local_routing(), &sc), Err(crate::ApiError::InvalidFilter)));
        let p = ask::invalid_filter_preview();
        assert_eq!((p.destination, p.reason, p.local, p.require_local), ("refused", "invalid_filter", false, false));
    }

    /// Finding: the person the filter names is resolved once for an Ask, however many sources and contexts read it,
    /// including the hosted and the on-device contexts of one blocking call.
    #[test]
    fn an_ask_resolves_its_person_once() {
        let fx = excluded_fixture();
        let sam = FindFilter { person: Some(PersonFilter { name: "Sam Okafor".into(), person_key: Some("name:sam".into()) }), ..range() };
        let builds = || memory::find::person_context_builds(&fx.s);
        let before = builds();
        // A hosted link first with an on-device link behind it, and a client excluded: the call reads a context
        // for each kind of link.
        let (hosted, local) = (Log::default(), Log::default());
        let routing = hosted_routing();
        recording(&hosted, &local, "Fine.", || ask::answer_sourced(&fx.s, "pricing", "gemma-test", 20, &[], &routing, &scope(sam.clone())).unwrap());
        assert_eq!(hosted.prompts().len(), 1);
        assert_eq!(builds() - before, 1, "one person context for the blocking Ask");
        let p = ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &routing, &crate::routing::LimitedUseSettings::default(), &scope(sam.clone())).unwrap();
        ask::discard_ask(&p.data.prepare_id);
        assert_eq!(builds() - before, 2, "and one for the streamed one");
        // A question about a day reads through the window path, with the same one context.
        let day = FindFilter { person: sam.person.clone(), ..Default::default() };
        let p = ask::prepare_ask_sourced(&fx.s, "what happened yesterday", 20, &[], &routing, &crate::routing::LimitedUseSettings::default(), &scope(day)).unwrap();
        ask::discard_ask(&p.data.prepare_id);
        assert_eq!(builds() - before, 3);
        // No person, no context.
        let p = ask::prepare_ask_sourced(&fx.s, "pricing", 20, &[], &routing, &crate::routing::LimitedUseSettings::default(), &scope(range())).unwrap();
        ask::discard_ask(&p.data.prepare_id);
        assert_eq!(builds() - before, 3);
    }

    /// Finding: the preview rescans only when some client is excluded, which is when a shell must keep it off the
    /// store a page is waiting on.
    #[test]
    fn a_preview_may_rescan_only_when_a_client_is_excluded() {
        assert!(!ask::route_preview_may_rescan(&world().s), "nothing excluded: a few cheap reads");
        assert!(ask::route_preview_may_rescan(&excluded_fixture().s), "a client is excluded: the exclusion cache may rebuild");
        let p = ask::unchecked_preview();
        assert_eq!((p.destination, p.reason, p.local, p.require_local, p.provider, p.may_fall_back), ("unknown", "excluded_client_check_pending", false, false, None, false));
    }

    #[test]
    fn a_preview_for_a_streamed_call_that_is_pinned_local_by_a_cloud_model_is_hosted() {
        let fx = world();
        let cloud = RoutingConfig { ollama_model: "gpt-oss:120b-cloud".into(), ..RoutingConfig::default() };
        let p = preview(&fx, "pricing", &cloud, &AskScope::default(), RouteMode::Stream);
        assert_eq!((p.destination, p.provider.as_deref()), ("hosted", Some("ollama")));
    }
}
