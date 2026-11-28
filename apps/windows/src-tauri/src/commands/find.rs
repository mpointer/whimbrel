//! Find and the Ask with numbered sources (screen 5, the redesign flag on; the old Library and Ask commands do not
//! call anything here).
//!
//! `library_find` is one search over what this PC remembers with date, app and person bounds that really filter.
//! `chat_route_preview` says where a question would be answered, from the same decision `chat_ask_sourced` makes, so
//! the Ask button's name is true. `chat_ask_sourced` is `chat_ask` over the core's sourced calls: the same streaming,
//! stopping, fallback, spend, title and excluded-client behaviour, with these differences: retrieval is bounded by the
//! filters, the numbered list the model was given reaches the page, and only the cited sources are saved (kind and id,
//! never their words). `chat_conversation_sourced` reads a saved conversation with each cited source drawn from its
//! live row now, so a trim, a forget or a client delete removes its text from old answers too.
//!
//! Nothing here logs a question, an answer or a source's words.

use std::collections::BTreeMap;

use memory::cards::{NewSourcedExchange, SavedSource, SourceCard};
use memory::find::{FindFilter, FindOrder, PersonFilter};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

use super::chat::{answered_model, blocking_started, fail, names_excluded_client, prepare_falls_back, WAITING_SHOWN_AFTER};
use super::library::{tag, DeviceTag};
use crate::chat::{self, ChatEvent, ContextEntry, StreamRoute, TerminalAction};
use crate::device_labels::DeviceLabeler;
use crate::error::CommandError;
use crate::l10n::tr;
use crate::settings::{ApiKeys, Instructions, PromptSurface, Provider, RoutingSettings};
use crate::state::AppState;
use crate::store_actor::StoreHandle;

// ---- what the page sends -----------------------------------------------------------------------------------------

/// The filters the page holds: dates (`[start, end)` in unix ms), one app, one person. Leave a filter out (null) to
/// leave it off. A filter that could widen a search is refused, not read as no filter: an unknown key, an app or
/// person that is present but blank, a person key with no person, or a range whose start is not before its end.
#[derive(Debug, Clone, Default, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct FindFilterInput {
    /// Inclusive.
    pub start_unix_ms: Option<i64>,
    /// Exclusive.
    pub end_unix_ms: Option<i64>,
    /// The app's id, as captures store it.
    pub app: Option<String>,
    /// The person's name, as the People filter offers it.
    pub person: Option<String>,
    pub person_key: Option<String>,
}

// The same fields, read by the derive. `FindFilterInput` reads them only from a JSON object: serde's derive would
// also accept a JSON array as the fields in order (`[]` is every field left out, which is no filter at all), and the
// page's shape is an object.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
struct FindFilterFields {
    start_unix_ms: Option<i64>,
    end_unix_ms: Option<i64>,
    app: Option<String>,
    person: Option<String>,
    person_key: Option<String>,
}

impl<'de> Deserialize<'de> for FindFilterInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Accepts a map and nothing else: a sequence, a string, a number or null is refused.
        struct ObjectOnly;
        impl<'de> serde::de::Visitor<'de> for ObjectOnly {
            type Value = FindFilterInput;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object of filters")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<FindFilterInput, A::Error> {
                let f = FindFilterFields::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(FindFilterInput { start_unix_ms: f.start_unix_ms, end_unix_ms: f.end_unix_ms, app: f.app, person: f.person, person_key: f.person_key })
            }
        }
        deserializer.deserialize_map(ObjectOnly)
    }
}

impl FindFilterInput {
    /// The core's filter, or why it is refused (see [`FindFilterInput`]): fail closed, never a wider search.
    pub fn to_core(&self) -> Result<FindFilter, CommandError> {
        // A person key names a person; on its own it would drop out and the search would run unbounded.
        if self.person.is_none() && self.person_key.as_deref().is_some_and(|k| !k.trim().is_empty()) {
            return Err(CommandError::invalid_input("The filters are not valid: a person key came with no person."));
        }
        FindFilter {
            start_unix_ms: self.start_unix_ms,
            end_unix_ms: self.end_unix_ms,
            app_bundle_id: self.app.clone(),
            person: self.person.clone().map(|name| PersonFilter { name, person_key: self.person_key.clone() }),
        }
        .validated()
        .map_err(|e| CommandError::invalid_input(format!("The filters are not valid: {e}.")))
    }
}

/// How a page of results is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindOrderInput {
    /// Newest first.
    Time,
    /// Best match first.
    Rank,
}

impl FindOrderInput {
    fn to_core(self) -> FindOrder {
        match self {
            FindOrderInput::Time => FindOrder::Time,
            FindOrderInput::Rank => FindOrder::Rank,
        }
    }
}

// ---- what the page gets ------------------------------------------------------------------------------------------

/// One result or source, read from its live row. Keys that do not apply are null.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FoundCard {
    /// `capture`, `transcript`, `snapshot` (a slide), `thread`, `episode`, `routine_run` or `meeting`.
    pub kind: String,
    pub id: i64,
    /// False when the row is gone from this PC: no title, no text.
    pub found: bool,
    pub ts_unix_ms: Option<i64>,
    pub title: String,
    /// The stored text, cut to length. Never a model's wording.
    pub text: Option<String>,
    /// Captures and episodes: the app, as stored.
    pub app: Option<String>,
    pub device: Option<DeviceTag>,
    /// Transcript lines, slides and meetings.
    pub meeting_id: Option<i64>,
    pub meeting_title: Option<String>,
    pub meeting_start_unix_ms: Option<i64>,
    /// A transcript line's speaker, as the meeting names it.
    pub speaker: Option<String>,
    pub speaker_is_guess: Option<bool>,
    /// A transcript line's offset into its meeting, in centiseconds.
    pub start_cs: Option<i64>,
    /// An email thread's last sender.
    pub sender: Option<String>,
    /// Why a transcript line matched a person: `speaker` or `attendee`.
    pub via: Option<String>,
}

pub fn found_card(card: SourceCard, labeler: &DeviceLabeler) -> FoundCard {
    FoundCard {
        device: card.device_id.as_deref().map(|d| tag(labeler, d)),
        kind: card.kind,
        id: card.id,
        found: card.found,
        ts_unix_ms: card.ts_unix_ms,
        title: card.title,
        text: card.text,
        app: card.app_bundle_id,
        meeting_id: card.meeting_id,
        meeting_title: card.meeting_title,
        meeting_start_unix_ms: card.meeting_start_unix_ms,
        speaker: card.speaker_label,
        speaker_is_guess: card.speaker_is_guess,
        start_cs: card.start_cs,
        sender: card.sender,
        via: card.via,
    }
}

/// One page of results.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FoundPage {
    pub results: Vec<FoundCard>,
    /// With a person: captures that mention them, listed apart.
    pub mentions: Vec<FoundCard>,
    /// How many candidates there are across pages.
    pub total: usize,
    pub mentions_total: usize,
    /// Candidates per kind, and `mentions`.
    pub counts: BTreeMap<String, usize>,
    /// The search model is installed and was used.
    pub semantic_used: bool,
    pub semantic_available: bool,
    /// More results follow this page.
    pub more: bool,
    /// A list hit its pool: more may exist beyond the candidates.
    pub capped: bool,
    /// Meaning search ran while new rows still wait for their embedding: the same words can find more soon. The page searches
    /// again then and replaces these results.
    pub meaning_catching_up: bool,
}

/// One item the model was given, by its number in the prompt.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NumberedSourceView {
    pub n: usize,
    pub card: FoundCard,
    /// The row could not be read just now (a database error). Not the same as gone: `card.found` is false either
    /// way, but an unreadable source may well exist, so the page must not say it is no longer on this PC.
    #[serde(default)]
    pub unreadable: bool,
}

/// One source a saved answer cited.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedSourceView {
    pub n: i64,
    pub card: FoundCard,
    /// The card could not be read just now (a database error): not a deleted row, so never "no longer on this PC".
    #[serde(default)]
    pub unreadable: bool,
}

/// The date range retrieval used: the filter's own, or a day question's window. A side is null when left open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsedRangeView {
    /// Inclusive.
    pub start_unix_ms: Option<i64>,
    /// Exclusive.
    pub end_unix_ms: Option<i64>,
}

pub fn range_view(range: Option<api::ask::UsedRange>) -> Option<UsedRangeView> {
    range.map(|r| UsedRangeView { start_unix_ms: r.start_unix_ms, end_unix_ms: r.end_unix_ms })
}

/// One saved exchange. `legacy` ones were saved before numbered sources: they carry `citations` (what the old Ask
/// saved) and no `sources`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourcedExchangeView {
    pub query: String,
    pub answer: String,
    pub created_at_unix_ms: i64,
    pub model: Option<String>,
    pub legacy: bool,
    pub citations: Vec<ContextEntry>,
    /// The provider that answered, an id such as `anthropic`.
    pub provider: Option<String>,
    pub answered_locally: Option<bool>,
    pub left_out: Option<i64>,
    /// The numbers the saved answer cited (the `n` of each source): the page draws a mark for `[n]` only for these.
    pub cited: Vec<usize>,
    pub sources: Vec<SavedSourceView>,
}

/// A saved conversation with its cited sources read from the live rows now.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourcedConversationView {
    pub id: i64,
    pub title: String,
    pub starred: bool,
    pub exchanges: Vec<SourcedExchangeView>,
}

/// Where a question would be answered. `destination` is `local`, `hosted`, `refused` or `unknown`; the button says
/// "on this PC" only for `local`. `require_local` says to pass it to the ask.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutePreviewView {
    pub destination: String,
    pub local: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reason: String,
    pub require_local: bool,
    pub may_fall_back: bool,
}

fn numbered_views(sources: Vec<api::ask::NumberedSource>, labeler: &DeviceLabeler) -> Vec<NumberedSourceView> {
    sources.into_iter().map(|s| NumberedSourceView { n: s.n, unreadable: s.unreadable, card: found_card(s.card, labeler) }).collect()
}

pub(super) fn conversation_view(c: memory::cards::SourcedConversation, labeler: &DeviceLabeler) -> SourcedConversationView {
    SourcedConversationView {
        id: c.id,
        title: c.title,
        starred: c.starred,
        exchanges: c
            .exchanges
            .into_iter()
            .map(|e| SourcedExchangeView {
                citations: e.citations_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default(),
                query: e.query,
                answer: e.answer,
                created_at_unix_ms: e.created_at_unix_ms,
                model: e.model,
                legacy: e.legacy,
                provider: e.provider,
                answered_locally: e.answered_locally,
                left_out: e.left_out,
                cited: e.cited,
                sources: e.sources.into_iter().map(|s| SavedSourceView { n: s.n, unreadable: s.unreadable, card: found_card(s.card, labeler) }).collect(),
            })
            .collect(),
    }
}

// ---- find, saved conversations -----------------------------------------------------------------------------------

/// One page of results for `query` under `filter`. An empty query with a filter lists what is in it by time; with no
/// filter it finds nothing. Read-only. Sends nothing anywhere.
///
/// It runs on the main store, as every Library read does (`library_search`, the episode list): the slow lane is for
/// calls that hold a store across a provider call (`store_actor.rs`), and a search that waited behind a blocking ask
/// would hang for as long as the ask. What keeps it short is the work core bounds: one person context per call, reads
/// narrowed in SQL, and an exact meaning search only when the filter selects a few thousand rows at most.
#[tauri::command]
pub async fn library_find(
    state: State<'_, AppState>,
    query: String,
    filter: FindFilterInput,
    order: FindOrderInput,
    offset: usize,
    limit: usize,
) -> Result<FoundPage, CommandError> {
    let labeler = state.device_labeler();
    let filter = filter.to_core()?;
    let order = order.to_core();
    let page = state
        .store()?
        .call(move |s| s.find(&query, &filter, order, offset, limit).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    Ok(FoundPage {
        results: page.results.into_iter().map(|c| found_card(c, &labeler)).collect(),
        mentions: page.mentions.into_iter().map(|c| found_card(c, &labeler)).collect(),
        total: page.total,
        mentions_total: page.mentions_total,
        counts: page.counts,
        semantic_used: page.semantic_used,
        semantic_available: page.semantic_available,
        more: page.more,
        capped: page.capped,
        meaning_catching_up: page.meaning_catching_up,
    })
}

/// The Dates and Apps filters a Find conversation was asked under, or `None` (none kept, or a store error read as none). Never a
/// person: only whether a People filter was on.
#[tauri::command]
pub async fn chat_conversation_find_filter(state: State<'_, AppState>, id: i64) -> Result<Option<memory::SavedFindFilter>, CommandError> {
    let kept = state.store()?.call(move |s| s.conversation_find_filter(id).ok().flatten()).await?;
    Ok(kept)
}

/// A saved conversation with each cited source read from its live row.
#[tauri::command]
pub async fn chat_conversation_sourced(state: State<'_, AppState>, id: i64) -> Result<Option<SourcedConversationView>, CommandError> {
    let labeler = state.device_labeler();
    let conversation = state.store()?.call(move |s| s.get_conversation_sourced(id).map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    Ok(conversation.map(|c| conversation_view(c, &labeler)))
}

// ---- routing -----------------------------------------------------------------------------------------------------

/// The routing a question gets and whether it streams: the decision `chat_ask`'s `run` makes, in one place for the
/// route preview and the sourced ask. `settings` already carries local-only when the ask requires it.
pub(super) fn route_for(settings: &RoutingSettings, keys: &ApiKeys, query: &str, ollama_up: bool) -> (RoutingSettings, Option<StreamRoute>) {
    let effective = chat::effective_routing(settings, query, ollama_up);
    let route = chat::local_instead(chat::stream_route(&effective, keys), ollama_up, api::local_model::is_configured());
    (effective, route)
}

/// Where `query` would be answered with these filters, from the same decision the ask makes. Sends nothing and
/// changes nothing: the routing is built without publishing the Limited Use switches to core, and the Ollama probe
/// runs off the store's thread and its answer is kept for a few seconds.
///
/// Which store it reads on depends on whether it can rescan. With no client excluded the decision is a handful of
/// cheap reads, made on the main store, so a question or a meeting summary holding the slow lane never keeps the
/// button's label waiting. With a client excluded the exclusion check may rebuild its cache (the real ask's
/// pre-check does, on the slow lane): that part runs on the slow lane, and when the lane is not free within
/// [`WAITING_SHOWN_AFTER`] (the moment the ask itself shows "waiting") the preview is `unknown`, so the button says
/// plain "Ask" and promises nothing.
#[tauri::command]
pub async fn chat_route_preview(
    state: State<'_, AppState>,
    query: String,
    conversation_id: Option<i64>,
    filter: FindFilterInput,
) -> Result<RoutePreviewView, CommandError> {
    let main = state.store()?.clone();
    let slow = state.slow_store()?.clone();
    let (settings, instructions) = {
        let s = state.settings.lock().unwrap();
        (s.routing(), s.instructions(PromptSurface::Ask))
    };
    let mut keys = ApiKeys::load(&*state.secrets)?;
    // The cached credentials only: a preview never refreshes a token.
    if !settings.local_only && settings.preferred_provider == Provider::Whimbrel {
        keys.whimbrel = crate::whimbrel_ai::cached();
    }
    let query = query.trim().to_string();
    let bound = api::ask::AskScope { filter: filter.to_core()?, require_local: false };
    // The probe of a local model server is a network call: not on a store's thread.
    let job = tokio::task::spawn_blocking(move || {
        let ollama_up = chat::ollama_serves_cached(&settings.ollama_model);
        let (effective, route) = route_for(&settings, &keys, &query, ollama_up);
        PreviewJob {
            mode: if route.is_some() { api::ask::RouteMode::Stream } else { api::ask::RouteMode::Blocking },
            routing: effective.routing_config_unpublished(&keys, &instructions),
            ollama_model: effective.ollama_model.clone(),
            query,
            conversation_id,
            scope: bound,
        }
    })
    .await
    .map_err(|e| CommandError::store(e.to_string()))?;
    let preview = preview_on_lanes(&main, &slow, WAITING_SHOWN_AFTER, job).await?;
    Ok(RoutePreviewView {
        destination: preview.destination.to_string(),
        local: preview.local,
        provider: preview.provider,
        model: preview.model,
        reason: preview.reason.to_string(),
        require_local: preview.require_local,
        may_fall_back: preview.may_fall_back,
    })
}

/// What one preview needs, captured when it is asked.
struct PreviewJob {
    query: String,
    conversation_id: Option<i64>,
    ollama_model: String,
    routing: api::RoutingConfig,
    scope: api::ask::AskScope,
    mode: api::ask::RouteMode,
}

impl PreviewJob {
    fn run(&self, store: &memory::Store) -> api::ask::RoutePreview {
        let history = self
            .conversation_id
            .and_then(|id| store.get_conversation(id).ok().flatten())
            .map(|c| chat::history_from(&c.exchanges))
            .unwrap_or_default();
        api::ask::ask_route_preview(store, &self.query, &history, &self.ollama_model, &self.routing, &self.scope, self.mode)
    }
}

/// Runs `job` where it belongs: on `main` when no client is excluded (nothing to rescan), else on `slow`, waiting at
/// most `wait` for that lane before answering [`api::ask::unchecked_preview`]. A job that the lane reaches after the
/// wait is over returns at once without reading anything.
async fn preview_on_lanes(main: &StoreHandle, slow: &StoreHandle, wait: std::time::Duration, job: PreviewJob) -> Result<api::ask::RoutePreview, CommandError> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let job = Arc::new(job);
    let may_rescan = main.call(|store| api::ask::route_preview_may_rescan(store)).await?;
    if !may_rescan {
        let cheap = job.clone();
        return main.call(move |store| cheap.run(store)).await;
    }
    let abandoned = Arc::new(AtomicBool::new(false));
    let (gave_up, rescan) = (abandoned.clone(), job.clone());
    let on_slow = slow.call(move |store| (!gave_up.load(Ordering::SeqCst)).then(|| rescan.run(store)));
    match tokio::time::timeout(wait, on_slow).await {
        Ok(done) => Ok(done?.unwrap_or_else(api::ask::unchecked_preview)),
        Err(_not_free_in_time) => {
            abandoned.store(true, Ordering::SeqCst);
            Ok(api::ask::unchecked_preview())
        }
    }
}

// ---- the sourced ask ---------------------------------------------------------------------------------------------

/// Everything one question needs, captured when it is asked.
struct SourcedAsk {
    query: String,
    /// `None` starts a new conversation with the first answered exchange.
    conversation_id: Option<i64>,
    /// With local-only already forced on when the page said the answer must stay on this PC: the stream, the blocking
    /// fallback and the title then all take the existing local-only path.
    settings: RoutingSettings,
    keys: ApiKeys,
    instructions: Instructions,
    slow: StoreHandle,
    scope: api::ask::AskScope,
    labeler: DeviceLabeler,
    /// False for an answer the page only shows (Today's): nothing is saved and no conversation is made.
    save: bool,
}

/// Starts answering `query` and returns at once; the events arrive on `on_event`, the same stream `chat_ask` sends,
/// plus `Sourced` (the numbered items) and `Cited` (what the answer cited), the last just before `Completed`.
/// `require_local` is the route preview's word that the answer comes from this PC: the call then never reaches a
/// hosted model and fails with the existing message instead. `save` false (screen 6: Today's answer) shows the answer and keeps nothing:
/// no conversation is made and no title is asked for; leave it out to save as before.
#[tauri::command]
pub fn chat_ask_sourced(
    state: State<'_, AppState>,
    query: String,
    conversation_id: Option<i64>,
    filter: FindFilterInput,
    require_local: bool,
    save: Option<bool>,
    on_event: Channel<ChatEvent>,
) -> Result<(), CommandError> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(CommandError::invalid_input(tr("Type a question first.", &[])));
    }
    let store = state.store()?.clone();
    let slow = state.slow_store()?.clone();
    let (mut settings, instructions) = {
        let s = state.settings.lock().unwrap();
        (s.routing(), s.instructions(PromptSurface::Ask))
    };
    if require_local {
        settings.local_only = true;
    }
    let scope = api::ask::AskScope { filter: filter.to_core()?, require_local };
    let mut ask = SourcedAsk {
        query,
        conversation_id,
        settings,
        keys: ApiKeys::load(&*state.secrets)?,
        instructions,
        slow,
        scope,
        labeler: state.device_labeler(),
        save: save.unwrap_or(true),
    };
    let sync = state.sync.clone();
    // Background model requests hold back from now until the answer ends (PR 874).
    let gate = crate::user_model_gate::enter();
    std::thread::Builder::new()
        .name("whimbrel-ask".into())
        .spawn(move || {
            let _gate = gate;
            crate::chat::freshen_whimbrel(sync.as_deref(), &ask.settings, &mut ask.keys);
            // A closed window drops the UI's end of the channel. The answer is still finished and persisted.
            let send = |event: ChatEvent| {
                let _ = on_event.send(event);
            };
            run(&store, &ask, &send);
        })
        .map_err(|e| CommandError::store(tr("Could not start the question: {error}", &[("error", &e.to_string())])))?;
    Ok(())
}

fn run(store: &StoreHandle, ask: &SourcedAsk, send: &(dyn Fn(ChatEvent) + Sync)) {
    let ollama_up = chat::ollama_serves(&ask.settings.ollama_model);
    let (effective, route) = route_for(&ask.settings, &ask.keys, &ask.query, ollama_up);
    let (provider, model) = chat::answering(&effective);
    // Never the question itself: only how it was routed.
    tracing::info!(
        provider,
        model = %model,
        streams = route.is_some(),
        ollama_up,
        require_local = ask.scope.require_local,
        "answering a question with sources"
    );
    let routing = effective.routing_config(&ask.keys, &ask.instructions);
    let call = effective.limited_use();
    let (query, conversation_id, streams, scope) = (ask.query.clone(), ask.conversation_id, route.is_some(), ask.scope.clone());
    // One store call: history and (to stream) the prompt with its numbered sources.
    let gathered = store.call_blocking(move |s| {
        let history = conversation_id
            .and_then(|id| s.get_conversation(id).ok().flatten())
            .map(|c| chat::history_from(&c.exchanges))
            .unwrap_or_default();
        // Converted here: core leaves a refusal's reason on this thread, take-once (`From<ApiError>`).
        let prepared = streams
            .then(|| api::ask::prepare_ask_sourced(s, &query, chat::CONTEXT_LIMIT, &history, &routing, &call, &scope).map_err(CommandError::from));
        (history, prepared)
    });
    let (history, prepared) = match gathered {
        Ok(g) => g,
        Err(error) => return fail(send, error, false),
    };
    match (route, prepared) {
        (Some(route), Some(Ok(prepared))) => stream(store, ask, &effective, route, prepared, history, send),
        // The spend cap, checked before any provider is tried: not the end only for a spent monthly budget on the
        // streaming provider, or a question that carries an excluded client, which the blocking path answers on this
        // PC or refuses (as `chat_ask` does).
        (_, Some(Err(error))) if !prepare_falls_back(&error) => fail(send, error, false),
        (_, Some(Err(error))) => {
            let local = names_excluded_client(&error) || question_names_excluded(ask, &history, send);
            send(blocking_started(&effective, &[]));
            if local {
                send(ChatEvent::RoutedLocally);
            } else {
                send(ChatEvent::FellBack);
            }
            blocking(store, ask, &effective, history, send);
        }
        _ => {
            // Until the pre-check says the question names no excluded client, the page names no model.
            let local = question_names_excluded(ask, &history, send);
            send(blocking_started(&effective, &[]));
            if local {
                send(ChatEvent::RoutedLocally);
            }
            blocking(store, ask, &effective, history, send);
        }
    }
}

/// Core's pre-check, first thing on the slow lane the answer will use. Sends `Waiting` when the lane is not free after
/// a moment. False when the store is gone: the answer then fails on its own.
fn question_names_excluded(ask: &SourcedAsk, history: &[llm::ChatTurn], send: &(dyn Fn(ChatEvent) + Sync)) -> bool {
    // The filter's person is part of what the question names (asking about them is asking about the client), as for
    // the routing the call itself makes.
    let (query, history, scope) = (ask.query.clone(), history.to_vec(), ask.scope.clone());
    let (reached, waiting) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|threads| {
        threads.spawn(move || {
            if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = waiting.recv_timeout(WAITING_SHOWN_AFTER) {
                send(ChatEvent::Waiting);
            }
        });
        ask.slow
            .call_blocking(move |s| {
                let _ = reached.send(());
                api::ask::question_names_excluded_client_scoped(s, &query, &history, &scope)
            })
            .unwrap_or(false)
    })
}

fn stream(
    store: &StoreHandle,
    ask: &SourcedAsk,
    effective: &RoutingSettings,
    route: StreamRoute,
    prepared: api::ask::PreparedAskSourced,
    history: Vec<llm::ChatTurn>,
    send: &dyn Fn(ChatEvent),
) {
    let api::ask::PreparedAskSourced { data, sources, answered_locally, date_range, .. } = prepared;
    let Some(run) = chat::start_stream(&data.prompt, &route) else {
        api::ask::discard_ask(&data.prepare_id);
        send(blocking_started(effective, &[]));
        return blocking(store, ask, effective, history, send);
    };
    send(ChatEvent::Started { run_id: Some(run.run_id.clone()), provider: route.provider.into(), model: route.model.clone(), context: Vec::new() });
    send(ChatEvent::Sourced { sources: numbered_views(sources, &ask.labeler), provider: route.provider.into(), answered_locally, date_range: range_view(date_range) });
    let end = chat::forward(&run.run_id, api::inference::poll, send, || std::thread::sleep(chat::POLL_INTERVAL));
    api::inference::release(&run.run_id);
    match chat::terminal_action_for(route.provider, &end.outcome, end.produced_output) {
        TerminalAction::Finalize => {
            // Usage goes under the model that answered; a stand-in for a model the provider no longer has is
            // recorded for Settings -> AI.
            let answered = end.model.clone().unwrap_or_else(|| route.model.clone());
            let (prepare_id, text, model, usage) = (data.prepare_id, end.text.clone(), answered.clone(), end.usage);
            let (provider, requested, substitute) = (llm::Provider::parse(route.provider), route.model.clone(), end.model.clone());
            let finalized = store.call_blocking(move |s| {
                if let Some(provider) = provider {
                    api::model_defaults::note_answer(s, provider, &requested, substitute.as_deref(), api::util::now_unix_ms());
                }
                api::ask::finalize_ask_sourced(s, &prepare_id, &text, &model, usage)
            });
            let (citations, cited, cited_sources, left_out) = match finalized {
                Ok(f) => (f.citations, f.cited, f.cited_sources, f.left_out),
                Err(_) => (Vec::new(), Vec::new(), Vec::new(), 0),
            };
            let answer = Answer { text: end.text, citations, cited, cited_sources, model: answered, provider: route.provider.to_string(), answered_locally, left_out };
            finish(store, ask, answer, send);
        }
        TerminalAction::Restore => {
            api::ask::discard_ask(&data.prepare_id);
            tracing::info!("question stopped");
            send(ChatEvent::Cancelled);
        }
        TerminalAction::FallBack => {
            api::ask::discard_ask(&data.prepare_id);
            tracing::info!(provider = route.provider, "stream failed before any output; answering on the blocking path");
            send(ChatEvent::FellBack);
            blocking(store, ask, effective, history, send);
        }
        TerminalAction::Fail => {
            api::ask::discard_ask(&data.prepare_id);
            fail(send, CommandError::provider_failure(), true);
        }
        TerminalAction::CreditExhausted => {
            api::ask::discard_ask(&data.prepare_id);
            tracing::info!("Whimbrel AI credit is used up");
            fail(send, CommandError::ProviderFailure { reason: Some(crate::error::ProviderFailureReason::credit_exhausted()) }, false);
        }
    }
}

fn blocking(store: &StoreHandle, ask: &SourcedAsk, effective: &RoutingSettings, history: Vec<llm::ChatTurn>, send: &dyn Fn(ChatEvent)) {
    let (_, model) = chat::answering(effective);
    let routing = effective.routing_config(&ask.keys, &ask.instructions);
    let (query, ollama_model, scope) = (ask.query.clone(), effective.ollama_model.clone(), ask.scope.clone());
    // The error is converted on the slow lane's thread, where core left the refusal's reason (take-once).
    let answered = ask.slow.call_blocking(move |s| {
        api::ask::answer_sourced(s, &query, &ollama_model, chat::CONTEXT_LIMIT, &history, &routing, &scope).map_err(CommandError::from)
    });
    match answered {
        Ok(Ok(done)) => {
            let detailed = done.detailed;
            let excluded_client = matches!(detailed.exclusion, api::exclusion::ExclusionOutcome::LocalOnly);
            let model = answered_model(&detailed.answered_by, &detailed.exclusion, &model);
            let provider = detailed.answered_by.provider.clone();
            send(ChatEvent::AnsweredBy { model: model.clone(), excluded_client });
            // The list is the one the answering link was given; after a fall back it replaces the stream's.
            send(ChatEvent::Sourced {
                sources: numbered_views(done.sources, &ask.labeler),
                provider: provider.clone(),
                answered_locally: done.answered_locally,
                date_range: range_view(done.date_range),
            });
            let left_out = chat::left_out(&detailed.exclusion).unwrap_or(0) as i64;
            let answer = Answer {
                text: detailed.answer.text,
                citations: detailed.answer.citations,
                cited: done.cited,
                cited_sources: done.cited_sources,
                model,
                provider,
                answered_locally: done.answered_locally,
                left_out,
            };
            finish(store, ask, answer, send)
        }
        Ok(Err(error)) | Err(error) => fail(send, error, false),
    }
}

/// An answer ready to save and show.
struct Answer {
    text: String,
    citations: Vec<llm::ContextItemRef>,
    cited: Vec<usize>,
    cited_sources: Vec<SavedSource>,
    model: String,
    provider: String,
    answered_locally: bool,
    left_out: i64,
}

/// Persists the exchange with its cited sources, shows the answer, then (for a new conversation) generates its title.
/// Title generation is best effort and never retried: any failure leaves the placeholder.
fn finish(store: &StoreHandle, ask: &SourcedAsk, answer: Answer, send: &dyn Fn(ChatEvent)) {
    tracing::info!(chars = answer.text.len(), cited = answer.cited.len(), "question answered with sources");
    let conversation_id = if ask.save { persist(store, ask, &answer) } else { None };
    send(ChatEvent::Cited { cited: answer.cited, answered_locally: answer.answered_locally, provider: answer.provider, left_out: answer.left_out });
    let left_out = (answer.left_out > 0).then_some(answer.left_out as usize);
    let text = answer.text;
    send(ChatEvent::Completed { answer: text.clone(), citations: answer.citations, conversation_id, narrowed: false, left_out });
    let (None, Some(id)) = (ask.conversation_id, conversation_id) else { return };
    // The title call sends the question and the answer to a model. An answer promised local, or made on this PC, is titled by a
    // model on this PC only (local-only routing: no hosted link is in that chain); with none, the placeholder stays.
    let mut title_settings = ask.settings.clone();
    if ask.scope.require_local || answer.answered_locally {
        title_settings.local_only = true;
    }
    let routing = chat::title_routing(&title_settings, chat::ollama_serves(&title_settings.ollama_model)).routing_config(&ask.keys, &Instructions::default());
    let (query, ollama_model) = (ask.query.clone(), title_settings.ollama_model.clone());
    let titled = ask.slow.call_blocking(move |s| {
        let title = api::ask::generate_conversation_title(s, &ollama_model, &query, &text, &routing).ok()?.title;
        s.set_conversation_title_if_placeholder(id, &title).ok()?.then_some(title)
    });
    if let Ok(Some(title)) = titled {
        send(ChatEvent::Titled { conversation_id: id, title });
    }
}

/// Creates the conversation on its first answered exchange, then appends it with its cited sources (kind and id only),
/// the provider, whether it was answered here and the left out count. `None` if the write failed (logged): the answer
/// is still shown.
fn persist(store: &StoreHandle, ask: &SourcedAsk, answer: &Answer) -> Option<i64> {
    let (conversation_id, query, text, model, provider) =
        (ask.conversation_id, ask.query.clone(), answer.text.clone(), answer.model.clone(), answer.provider.clone());
    let (sources, answered_locally, left_out, now) = (answer.cited_sources.clone(), answer.answered_locally, answer.left_out, api::util::now_unix_ms());
    // A new conversation keeps the Dates and Apps it was asked under, never a person: only that a People filter was on.
    let f = &ask.scope.filter;
    let kept = memory::SavedFindFilter {
        start_unix_ms: f.start_unix_ms,
        end_unix_ms: f.end_unix_ms,
        app_bundle_id: f.app_bundle_id.clone(),
        people_left_out: f.person.is_some(),
    };
    let result = store.call_blocking(move |s| -> Result<i64, String> {
        let id = match conversation_id {
            Some(id) => id,
            // Made with its kept filters in the same write: the conversation never exists without them.
            None => s.insert_conversation_with_find_filter(&query, now, &kept).map_err(|e| e.to_string())?,
        };
        s.append_conversation_exchange_sourced(&NewSourcedExchange {
            conversation_id: id,
            query: query.as_str(),
            answer: text.as_str(),
            sources: sources.as_slice(),
            model: Some(model.as_str()),
            provider: Some(provider.as_str()),
            answered_locally,
            left_out,
            now_unix_ms: now,
        })
        .map_err(|e| e.to_string())?;
        // After each stored exchange the conversation joins the topic most of its cited sources belong to (never over the
        // person's choice).
        let _ = api::conversations::auto_attach_topic(s, id);
        Ok(id)
    });
    match result {
        Ok(Ok(id)) => Some(id),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "could not save the exchange");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not save the exchange");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_input_becomes_the_cores_filter_with_blanks_dropped() {
        let input = FindFilterInput {
            start_unix_ms: Some(10),
            end_unix_ms: Some(20),
            app: Some("  ".into()),
            person: Some(" Sam Okafor ".into()),
            person_key: Some("sam@example.test".into()),
        };
        // A blank app is refused, not dropped: dropping it would search every app.
        assert!(input.to_core().is_err());
        let core = FindFilterInput { app: None, ..input }.to_core().unwrap();
        assert_eq!((core.start_unix_ms, core.end_unix_ms), (Some(10), Some(20)));
        assert_eq!(core.app_bundle_id, None);
        assert_eq!(core.person, Some(PersonFilter { name: "Sam Okafor".into(), person_key: Some("sam@example.test".into()) }));
        assert!(FindFilterInput::default().to_core().unwrap().is_empty());
    }

    #[test]
    fn a_filter_that_could_widen_a_search_is_refused() {
        let refused = |input: FindFilterInput| input.to_core().is_err();
        assert!(refused(FindFilterInput { person: Some("  ".into()), ..Default::default() }), "a blank person");
        assert!(refused(FindFilterInput { app: Some("".into()), ..Default::default() }), "a blank app");
        assert!(refused(FindFilterInput { person_key: Some("sam@example.test".into()), ..Default::default() }), "a person key with no person");
        assert!(refused(FindFilterInput { start_unix_ms: Some(20), end_unix_ms: Some(10), ..Default::default() }), "a reversed range");
        assert!(refused(FindFilterInput { start_unix_ms: Some(10), end_unix_ms: Some(10), ..Default::default() }), "an empty range");
        // A blank person key with no person is nothing to keep.
        assert!(FindFilterInput { person_key: Some(" ".into()), ..Default::default() }.to_core().is_ok());
        // An unknown key (a snake_case or misspelt one) is an error when the page's JSON is read.
        let unknown: Result<FindFilterInput, _> = serde_json::from_str(r#"{"start_unix_ms": 10}"#);
        assert!(unknown.is_err(), "the page's keys are camelCase; anything else is refused, not read as no filter");
        let known: FindFilterInput = serde_json::from_str(r#"{"startUnixMs": 10, "endUnixMs": 20, "personKey": null}"#).unwrap();
        assert_eq!((known.start_unix_ms, known.end_unix_ms), (Some(10), Some(20)));
    }

    /// serde's derive reads a JSON array as the struct's fields in order, and `[]` as every field left out: no filter at
    /// all. A filter is an object.
    #[test]
    fn a_filter_that_is_not_an_object_is_refused() {
        for raw in ["[]", "[null,null,null,null,null]", "[10,20,\"com.slack\",\"Sam\",null]", "null", "10", "\"com.slack\"", "true"] {
            let read: Result<FindFilterInput, _> = serde_json::from_str(raw);
            assert!(read.is_err(), "{raw} must not read as a filter");
        }
        // A value (the shape the IPC layer hands over) is held to the same rule.
        assert!(serde_json::from_value::<FindFilterInput>(serde_json::json!([])).is_err());
        let empty: FindFilterInput = serde_json::from_str("{}").unwrap();
        assert!(empty.to_core().unwrap().is_empty());
        let full: FindFilterInput = serde_json::from_str(r#"{"startUnixMs":1,"endUnixMs":2,"app":"a","person":"p","personKey":"k"}"#).unwrap();
        assert_eq!(full, FindFilterInput { start_unix_ms: Some(1), end_unix_ms: Some(2), app: Some("a".into()), person: Some("p".into()), person_key: Some("k".into()) });
        assert!(serde_json::from_str::<FindFilterInput>(r#"{"app":"a","extra":1}"#).is_err(), "an unknown key is still refused");
    }

    #[test]
    fn the_range_used_and_the_cited_numbers_reach_the_page() {
        let view = range_view(Some(api::ask::UsedRange { start_unix_ms: Some(5), end_unix_ms: None }));
        assert_eq!(serde_json::to_value(view).unwrap(), serde_json::json!({"startUnixMs": 5, "endUnixMs": null}));
        assert_eq!(range_view(None), None);
        let event = ChatEvent::Sourced { sources: Vec::new(), provider: "ollama".into(), answered_locally: true, date_range: view };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "sourced");
        assert_eq!(json["dateRange"], serde_json::json!({"startUnixMs": 5, "endUnixMs": null}));
        let none = serde_json::to_value(ChatEvent::Sourced { sources: Vec::new(), provider: "ollama".into(), answered_locally: false, date_range: None }).unwrap();
        assert!(none["dateRange"].is_null());
        // A saved conversation carries each exchange's cited numbers and whether a card could not be read.
        let card = SourceCard { kind: "capture".into(), id: 3, ..Default::default() };
        let convo = memory::cards::SourcedConversation {
            id: 1,
            title: "t".into(),
            starred: false,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            exchanges: vec![memory::cards::SourcedExchange {
                id: 1,
                query: "q".into(),
                answer: "a [2] b [1]".into(),
                created_at_unix_ms: 1,
                model: None,
                legacy: false,
                citations_json: None,
                provider: Some("ollama".into()),
                answered_locally: Some(true),
                left_out: Some(0),
                cited: vec![2, 1],
                sources: vec![memory::cards::SavedSourceCard { n: 2, card, unreadable: true }],
            }],
        };
        let view = serde_json::to_value(conversation_view(convo, &DeviceLabeler::default())).unwrap();
        assert_eq!(view["exchanges"][0]["cited"], serde_json::json!([2, 1]));
        assert_eq!(view["exchanges"][0]["sources"][0]["unreadable"], true);
    }

    fn memory_lane() -> StoreHandle {
        StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap()
    }

    fn a_job() -> PreviewJob {
        PreviewJob {
            query: "what about pricing?".into(),
            conversation_id: None,
            ollama_model: "gemma-test".into(),
            routing: api::RoutingConfig::default(),
            scope: api::ask::AskScope::default(),
            mode: api::ask::RouteMode::Blocking,
        }
    }

    /// Holds `lane` until the returned sender is used (or dropped), and returns only once the lane is held: its job
    /// has started, so anything sent to the lane after this waits behind it.
    fn hold(lane: &StoreHandle) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (started, running) = std::sync::mpsc::channel::<()>();
        let blocked = lane.clone();
        let thread = std::thread::spawn(move || {
            let _ = blocked.call_blocking(move |_| {
                let _ = started.send(());
                let _ = held.recv();
            });
        });
        running.recv().unwrap();
        (release, thread)
    }

    fn exclude_a_client(lane: &StoreHandle) {
        lane.call_blocking(|store| {
            let acme = store.create_seed_topic("Acme", &["Roadrunner Co".to_string()], None).unwrap();
            store.set_client_control(&acme.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        })
        .unwrap();
    }

    /// The preview is a handful of cheap reads when no client is excluded: a slow lane held by an ask or a summary
    /// does not keep the button's label waiting.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_preview_with_no_client_excluded_does_not_wait_for_the_slow_lane() {
        let (main, slow) = (memory_lane(), memory_lane());
        let (release, thread) = hold(&slow);
        let started = std::time::Instant::now();
        let preview = preview_on_lanes(&main, &slow, std::time::Duration::from_secs(60), a_job()).await.unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "it waited for a lane it does not need");
        assert_ne!(preview.reason, "excluded_client_check_pending");
        release.send(()).unwrap();
        thread.join().unwrap();
    }

    /// With a client excluded the check may rescan, so it runs on the slow lane and, when that lane is not free in
    /// time, the preview is `unknown`: the button promises nothing and the ask is made without `require_local`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_preview_that_must_rescan_says_unknown_when_the_slow_lane_is_busy_and_answers_when_it_is_free() {
        let (main, slow) = (memory_lane(), memory_lane());
        exclude_a_client(&main);
        let (release, thread) = hold(&slow);
        let started = std::time::Instant::now();
        let preview = preview_on_lanes(&main, &slow, std::time::Duration::from_millis(200), a_job()).await.unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "the busy lane was waited for");
        assert_eq!((preview.destination, preview.reason, preview.local, preview.require_local), ("unknown", "excluded_client_check_pending", false, false));
        // The lane comes free; the abandoned job then runs and reads nothing, and the next preview is answered.
        release.send(()).unwrap();
        thread.join().unwrap();
        let again = preview_on_lanes(&main, &slow, std::time::Duration::from_secs(30), a_job()).await.unwrap();
        assert_ne!(again.destination, "unknown", "a free lane answers: {again:?}");
    }

    #[test]
    fn a_card_keeps_what_the_core_gave_and_a_gone_row_shows_nothing() {
        let labeler = DeviceLabeler::default();
        let gone = found_card(SourceCard { kind: "capture".into(), id: 4, ..Default::default() }, &labeler);
        assert!(!gone.found);
        assert!(gone.text.is_none() && gone.device.is_none());
        let line = found_card(
            SourceCard { kind: "transcript".into(), id: 9, found: true, meeting_id: Some(3), start_cs: Some(1500), text: Some("Hello".into()), ..Default::default() },
            &labeler,
        );
        assert_eq!((line.meeting_id, line.start_cs, line.text.as_deref()), (Some(3), Some(1500), Some("Hello")));
    }
}
