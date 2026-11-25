//! Conversations (screen 6, the redesign flag on; nothing in the old Ask or Library calls anything here).
//!
//! What lives here: the Conversations list (with each row's topic and the first line of its latest answer), a conversation's
//! page (scope, topic link, the topics its sources belong to), choosing or removing its topic, keeping an answer the page holds
//! as a conversation (Today's Continue, a first answer, a follow-up in a topic or meeting conversation: no second model call), and
//! the streamed Ask inside one topic.
//!
//! A topic Ask is core's `prepare_ask_in_topic_for` with `finalize_ask_sourced`: core has no sourced topic entry point, so the
//! numbered list of the prompt is not returned. The cited numbers and pointers come from the stashed prepare (`finalize_ask_sourced`),
//! and the cards for those are read from the live rows (`resolve_cards`), so the page draws the same numbered marks and cards as the Find
//! page. A topic has no blocking path (as on the Mac): a question that names an excluded client is prepared again for a model on this PC.
//!
//! Nothing here logs a question, an answer or a source's words.

use memory::cards::{NewSourcedExchange, SavedSource, SourceCard};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

use super::chat::{fail, names_excluded_client, ConversationRow};
use super::find::{conversation_view, found_card, route_for, NumberedSourceView, SourcedConversationView};
use super::topic_page::TopicRefView;
use crate::chat::{self, ChatEvent, StreamRoute, TerminalAction};
use crate::device_labels::DeviceLabeler;
use crate::error::{CommandError, ProviderFailureReason};
use crate::l10n::tr;
use crate::settings::{ApiKeys, Instructions, PromptSurface, RoutingSettings};
use crate::state::AppState;
use crate::store_actor::StoreHandle;

// ---- what the page sends and gets --------------------------------------------------------------------------------

/// What a conversation asks about: everything, one topic, or one meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversationScopeView {
    Everything,
    Topic { id: i64 },
    Meeting { id: i64 },
}

impl From<memory::ConversationScope> for ConversationScopeView {
    fn from(scope: memory::ConversationScope) -> Self {
        match scope {
            memory::ConversationScope::Everything => Self::Everything,
            memory::ConversationScope::Topic(id) => Self::Topic { id },
            memory::ConversationScope::Meeting(id) => Self::Meeting { id },
        }
    }
}

impl From<ConversationScopeView> for memory::ConversationScope {
    fn from(scope: ConversationScopeView) -> Self {
        match scope {
            ConversationScopeView::Everything => Self::Everything,
            ConversationScopeView::Topic { id } => Self::Topic(id),
            ConversationScopeView::Meeting { id } => Self::Meeting(id),
        }
    }
}

// A list row: the summary fields, the live topic and the first line of the latest answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationListRow {
    pub id: i64,
    pub title: String,
    pub starred: bool,
    pub updated_at_unix_ms: i64,
    pub topic: Option<TopicRefView>,
    pub latest_answer_line: Option<String>,
}

// One source an answer cited, as the page sends it back to be kept: its number, kind and id, nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CitedPointerInput {
    pub n: i64,
    pub kind: String,
    pub id: i64,
}

// An answer the page holds, to be kept. Core saves only the pointers (it drops an unknown kind or a number below 1); the words
// of a source are read from its live row each time. `answered_locally`, `routed_locally`, `provider`, `model`, `left_out` and
// `require_local` are what the page heard on the answer's own events: the page supplies them, and the shell never trusts them to
// make anything LESS private. Each one can only keep the exchange on this PC (its title, and the history of later questions);
// whether it counts as local is also decided here from the stored rows (`must_stay_local`), and the excluded-material check reads the
// STORED scope of an existing conversation (the `scope` argument of `conversation_save_exchange` makes a new one and nothing else).
// What the exchange DISPLAYS as made here is computed from its provider and model alone (`displayed_local`). An empty `model` or `provider` is
// none, and an unknown or missing provider counts as local (fail closed).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedExchangeInput {
    pub query: String,
    pub answer: String,
    pub cited: Vec<CitedPointerInput>,
    pub model: String,
    pub provider: String,
    pub answered_locally: bool,
    pub left_out: i64,
    pub require_local: bool,
    pub routed_locally: bool,
}

// What keeping an answer gave back: the conversation, and the topic it joined (none when its sources do not point at one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedExchangeResult {
    pub conversation_id: i64,
    pub topic: Option<TopicRefView>,
    pub cited_topics: Vec<TopicRefView>,
}

// A conversation's topic link. `set_by_user` is true after the person chose a topic or removed it: it is then never moved again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTopicView {
    pub topic: Option<TopicRefView>,
    pub set_by_user: bool,
}

// A conversation as its page draws it. `local_only[i]` is true when exchange `i` was kept on this PC for an excluded client (core's
// own marker). `scope` is the one the next question is asked with (a merged topic followed; everything once its topic or meeting is
// gone). The scope's name comes with it: a topic's name, or a meeting's title and start for the page to word.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPageView {
    pub conversation: SourcedConversationView,
    pub local_only: Vec<bool>,
    pub scope: ConversationScopeView,
    pub scope_topic_name: Option<String>,
    pub scope_meeting_title: Option<String>,
    pub scope_meeting_start_unix_ms: Option<i64>,
    pub topic: Option<TopicRefView>,
    pub topic_set_by_user: bool,
    pub cited_topics: Vec<TopicRefView>,
    pub topics: Vec<TopicRefView>,
}

fn topic_views(topics: Vec<memory::topics::page::TopicRef>) -> Vec<TopicRefView> {
    topics.into_iter().map(TopicRefView::from).collect()
}

// ---- lists, the page, the topic link -----------------------------------------------------------------------------

/// Every conversation, starred first and then most recently updated, with its live topic and the first line of its latest answer.
#[tauri::command]
pub async fn conversations_list(state: State<'_, AppState>) -> Result<Vec<ConversationListRow>, CommandError> {
    let rows = state.store()?.call(|s| s.conversation_rows().map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    Ok(rows
        .into_iter()
        .map(|r| ConversationListRow {
            id: r.id,
            title: r.title,
            starred: r.starred,
            updated_at_unix_ms: r.updated_at_unix_ms,
            topic: r.topic.map(TopicRefView::from),
            latest_answer_line: r.latest_answer_line,
        })
        .collect())
}

/// The conversations linked to a topic (or to one merged into it), newest updated first: the topic page's list.
#[tauri::command]
pub async fn topic_conversations_list(state: State<'_, AppState>, topic_id: i64) -> Result<Vec<ConversationRow>, CommandError> {
    let rows = state.store()?.call(move |s| s.topic_conversations(topic_id).map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    Ok(rows.into_iter().map(|c| ConversationRow { id: c.id, title: c.title, starred: c.starred, updated_at_unix_ms: c.updated_at_unix_ms }).collect())
}

/// What a stored conversation was started about, as it was stored (not resolved). Find opens an everything conversation itself and sends a
/// topic or meeting one to the Conversations page.
#[tauri::command]
pub async fn conversation_scope(state: State<'_, AppState>, id: i64) -> Result<ConversationScopeView, CommandError> {
    let scope = state.store()?.call(move |s| s.conversation_scope(id).map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    Ok(scope.into())
}

/// The page of one conversation: its exchanges with each cited source read from its live row now, its scope, its topic link and the
/// topics the chip can offer. `None` for a conversation that is not there.
#[tauri::command]
pub async fn conversation_page(state: State<'_, AppState>, id: i64) -> Result<Option<ConversationPageView>, CommandError> {
    let labeler = state.device_labeler();
    let read = state.store()?.call(move |s| read_page(s, id)).await?.map_err(CommandError::store)?;
    Ok(read.map(|p| {
        let (convo, local_only) = (p.conversation, p.local_only);
        ConversationPageView {
            conversation: conversation_view(convo, &labeler),
            local_only,
            scope: p.scope.into(),
            scope_topic_name: p.scope_topic_name,
            scope_meeting_title: p.scope_meeting_title,
            scope_meeting_start_unix_ms: p.scope_meeting_start_unix_ms,
            topic: p.topic.map(TopicRefView::from),
            topic_set_by_user: p.topic_set_by_user,
            cited_topics: topic_views(p.cited_topics),
            topics: topic_views(p.topics),
        }
    }))
}

/// What one store call reads for a page (core's own types; the command turns them into views).
struct PageRead {
    conversation: memory::cards::SourcedConversation,
    local_only: Vec<bool>,
    scope: memory::ConversationScope,
    scope_topic_name: Option<String>,
    scope_meeting_title: Option<String>,
    scope_meeting_start_unix_ms: Option<i64>,
    topic: Option<memory::topics::page::TopicRef>,
    topic_set_by_user: bool,
    cited_topics: Vec<memory::topics::page::TopicRef>,
    topics: Vec<memory::topics::page::TopicRef>,
}

fn read_page(s: &memory::Store, id: i64) -> Result<Option<PageRead>, String> {
    let Some(conversation) = s.get_conversation_sourced(id).map_err(|e| e.to_string())? else { return Ok(None) };
    let kept_local = s.local_only_exchange_ids(id).map_err(|e| e.to_string())?;
    let local_only = conversation.exchanges.iter().map(|e| kept_local.contains(&e.id)).collect();
    // A read that fails is an error, never a wider scope.
    let scope = s.resolved_conversation_scope(id).map_err(|e| e.to_string())?;
    let mut scope_topic_name = None;
    let mut meeting = None;
    match scope {
        memory::ConversationScope::Everything => {}
        memory::ConversationScope::Topic(topic_id) => scope_topic_name = s.live_topic_ref(topic_id).map_err(|e| e.to_string())?.map(|t| t.name),
        memory::ConversationScope::Meeting(meeting_id) => meeting = s.get_meeting(meeting_id).map_err(|e| e.to_string())?,
    }
    let link = s.conversation_topic(id).map_err(|e| e.to_string())?;
    // The chip's menus are best effort: a read that fails leaves them empty rather than the page unreadable.
    let cited_topics = api::conversations::cited_topics(s, id).unwrap_or_default();
    let topics: Vec<memory::topics::page::TopicRef> = s.topic_page_list(api::util::now_unix_ms()).map(|list| list.into_iter().map(|e| e.topic).collect()).unwrap_or_default();
    Ok(Some(PageRead {
        conversation,
        local_only,
        scope,
        scope_topic_name,
        scope_meeting_title: meeting.as_ref().map(|m| m.title.clone()),
        scope_meeting_start_unix_ms: meeting.as_ref().map(|m| m.start_unix_ms),
        topic: link.topic,
        topic_set_by_user: link.set_by_user,
        cited_topics,
        topics,
    }))
}

/// The person's choice of topic, or none to take the conversation out of its topic. Final: it is never moved again on its own, and a
/// removed topic is not joined again after the next answer (core remembers the removal as `set_by_user`).
#[tauri::command]
pub async fn conversation_set_topic(state: State<'_, AppState>, id: i64, topic_id: Option<i64>) -> Result<ConversationTopicView, CommandError> {
    let link = state.store()?.call(move |s| set_topic_link(s, id, topic_id)).await?.map_err(CommandError::store)?;
    Ok(ConversationTopicView { topic: link.topic.map(TopicRefView::from), set_by_user: link.set_by_user })
}

/// The one store call behind `conversation_set_topic`. The conversation is checked here, in the same call as the write (the store runs one call at a time,
/// so a delete cannot land between the two): a conversation that is gone gets an error, never an orphan `topic_conversation` row.
fn set_topic_link(s: &memory::Store, id: i64, topic_id: Option<i64>) -> Result<memory::ConversationTopic, String> {
    if s.get_conversation(id).map_err(|e| e.to_string())?.is_none() {
        return Err(tr("That conversation is gone.", &[]));
    }
    if let Some(topic) = topic_id {
        if s.live_topic_ref(topic).map_err(|e| e.to_string())?.is_none() {
            return Err(tr("That topic is gone.", &[]));
        }
    }
    s.set_conversation_topic(id, topic_id, true).map_err(|e| e.to_string())?;
    s.conversation_topic(id).map_err(|e| e.to_string())
}

// ---- keeping an answer as a conversation -------------------------------------------------------------------------

/// The one write behind keeping an answer: makes the conversation when `conversation_id` is none (with its scope, and for a topic
/// scope in that topic from the start, not as the person's choice), appends the exchange as pointers, then lets the conversation join
/// the topic most of its sources belong to. A row made here that could not take its exchange is removed again. Returns the id, the
/// topic after the call, the topics its sources belong to, and whether the exchange must stay on this PC (`must_stay_local`: the title
/// is then made here or not at all). An exchange that must stay local is marked with core's own local-only mark BEFORE it is appended,
/// so core stores it as local only and every later question keeps it off a hosted model.
fn save_exchange(
    s: &memory::Store,
    conversation_id: Option<i64>,
    scope: memory::ConversationScope,
    e: &SavedExchangeInput,
    now_unix_ms: i64,
) -> Result<SavedKeep, String> {
    let made = conversation_id.is_none();
    // The scope the excluded-material check reads is the one STORED for an existing conversation, never the one the page sent: the page's
    // scope is used only to make a new conversation. (A page that sent "everything" for an excluded client's topic conversation would
    // otherwise skip the topic or meeting check.) A scope that cannot be read fails the save, so nothing is kept open.
    let checked_scope = match conversation_id {
        Some(id) => s.conversation_scope(id).map_err(|e| e.to_string())?,
        None => scope,
    };
    // Decided first, from the stored rows and the page's words, so a failed read keeps the exchange local rather than open. Core's rule
    // (`exchange_touches_excluded_client`, the one the Mac uses) fails closed; the cited pointers go in as the page sent them, so a kind
    // core cannot read counts as touching a client.
    let cited: Vec<(String, i64)> = e.cited.iter().map(|c| (c.kind.clone(), c.id)).collect();
    let material = api::conversations::exchange_touches_excluded_client(s, &checked_scope, &e.query, &e.answer, &cited);
    let local = must_stay_local(e, material);
    if local {
        // Fail closed: an exchange that cannot be marked is not kept.
        s.mark_local_only_text(&e.answer).map_err(|e| e.to_string())?;
    }
    let id = match conversation_id {
        Some(id) => id,
        None => s.insert_conversation(&e.query, now_unix_ms).map_err(|e| e.to_string())?,
    };
    let sources: Vec<SavedSource> = e.cited.iter().map(|c| SavedSource { n: c.n, kind: c.kind.clone(), id: c.id }).collect();
    let model = Some(e.model.as_str()).filter(|m| !m.trim().is_empty());
    let provider = Some(e.provider.as_str()).filter(|p| !p.trim().is_empty());
    let appended = s.append_conversation_exchange_sourced(&NewSourcedExchange {
        conversation_id: id,
        query: e.query.as_str(),
        answer: e.answer.as_str(),
        sources: sources.as_slice(),
        model,
        provider,
        // Display only: it says the answer was made here. The shell decides it from the provider and model it was given (`displayed_local`),
        // never from the page's flag and never from `local`, which may hold for a hosted answer.
        answered_locally: displayed_local(e),
        left_out: e.left_out,
        now_unix_ms,
    });
    if let Err(error) = appended {
        if made {
            let _ = s.delete_conversation(id);
        }
        return Err(error.to_string());
    }
    if made {
        s.set_conversation_scope(id, &scope).map_err(|e| e.to_string())?;
        if let memory::ConversationScope::Topic(topic) = scope {
            s.set_conversation_topic(id, Some(topic), false).map_err(|e| e.to_string())?;
        }
    }
    // After each stored exchange: join the topic most of its cited sources belong to (never over the person's choice).
    let topic = api::conversations::auto_attach_topic(s, id).ok().flatten();
    let cited_topics = api::conversations::cited_topics(s, id).unwrap_or_default();
    Ok(SavedKeep { id, topic, cited_topics, local })
}

/// What `save_exchange` gives back.
struct SavedKeep {
    id: i64,
    topic: Option<memory::topics::page::TopicRef>,
    cited_topics: Vec<memory::topics::page::TopicRef>,
    local: bool,
}

/// Whether the stored exchange shows "Answered on this PC". The page's `answered_locally` counts only where this side's own `made_here`
/// (core's `is_hosted` rule, which any `cloud` word in an Ollama model name trips) agrees, and `made_here` alone is enough: so the
/// result is `made_here(provider, model)`, and a hosted provider or model can never be stored as local whatever the page said. The page's
/// flag still keeps an exchange off hosted models (`must_stay_local`); it just never changes this display.
fn displayed_local(e: &SavedExchangeInput) -> bool {
    // (page flag AND made_here) OR made_here is made_here.
    made_here(e.provider.trim(), e.model.trim())
}

/// Whether a held answer's exchange must stay on this PC, so its title is made on a model here (or not at all) and it never rides to
/// a hosted model later. Pure. Every field the page supplied can only turn this ON: `answered_locally`, `require_local`, `routed_locally`,
/// a provider that is a model on this PC, and a provider that is empty or unknown (fail closed). `material` is what this side found in
/// the stored rows (core's `exchange_touches_excluded_client`). Nothing the page says turns it off.
fn must_stay_local(e: &SavedExchangeInput, material: bool) -> bool {
    let provider_unknown = e.provider.trim() != "local" && llm::Provider::parse(e.provider.trim()).is_none();
    material || e.answered_locally || e.require_local || e.routed_locally || provider_unknown || made_here(e.provider.trim(), e.model.trim())
}

/// Keeps an answer the page holds: a new conversation (`conversation_id` none: Today's Continue, a meeting answer's Continue, the first
/// answer of a new conversation) or a follow-up in a topic or meeting conversation. No model is asked for the answer. A new
/// conversation then gets its generated title on `on_event` (`Titled`), made on this PC only when the answer was made there or the
/// question was held to it; with no such model the placeholder stays.
#[tauri::command]
pub async fn conversation_save_exchange(
    state: State<'_, AppState>,
    conversation_id: Option<i64>,
    scope: ConversationScopeView,
    exchange: SavedExchangeInput,
    on_event: Channel<ChatEvent>,
) -> Result<SavedExchangeResult, CommandError> {
    if exchange.query.trim().is_empty() || exchange.answer.trim().is_empty() {
        return Err(CommandError::invalid_input(tr("There is no answer to keep.", &[])));
    }
    let store = state.store()?.clone();
    let slow = state.slow_store()?.clone();
    let settings = state.settings.lock().unwrap().routing();
    let keys = ApiKeys::load(&*state.secrets)?;
    let core_scope: memory::ConversationScope = scope.into();
    let (kept, now) = (exchange.clone(), api::util::now_unix_ms());
    let kept_exchange = store
        .call(move |s| save_exchange(s, conversation_id, core_scope, &kept, now))
        .await?
        .map_err(CommandError::store)?;
    if conversation_id.is_none() {
        start_title(slow, settings, keys, kept_exchange.id, exchange, kept_exchange.local, on_event);
    }
    Ok(SavedExchangeResult {
        conversation_id: kept_exchange.id,
        topic: kept_exchange.topic.map(TopicRefView::from),
        cited_topics: topic_views(kept_exchange.cited_topics),
    })
}

/// Whether the title is asked of a model on this PC only: the person's own local-only setting, or an exchange that must stay local. The second can
/// only turn it on.
fn title_held_local(settings_local_only: bool, must_stay_local: bool) -> bool {
    settings_local_only || must_stay_local
}

/// Makes the generated title in the background and sends `Titled`. Best effort, never retried: any failure leaves the placeholder.
/// `local` (from `must_stay_local`, which the page's words can only turn on) means the title is asked of a model on this PC only; core's
/// own title function then also reroutes any text that carries an excluded client, so the two checks both hold.
fn start_title(slow: StoreHandle, settings: RoutingSettings, keys: ApiKeys, id: i64, exchange: SavedExchangeInput, local: bool, on_event: Channel<ChatEvent>) {
    let gate = crate::user_model_gate::enter();
    let started = std::thread::Builder::new().name("whimbrel-title".into()).spawn(move || {
        let _gate = gate;
        let mut title_settings = settings;
        title_settings.local_only = title_held_local(title_settings.local_only, local);
        let routing = chat::title_routing(&title_settings, chat::ollama_serves(&title_settings.ollama_model)).routing_config(&keys, &Instructions::default());
        let (query, answer, ollama_model) = (exchange.query, exchange.answer, title_settings.ollama_model.clone());
        let titled = slow.call_blocking(move |s| {
            let title = api::ask::generate_conversation_title(s, &ollama_model, &query, &answer, &routing).ok()?.title;
            s.set_conversation_title_if_placeholder(id, &title).ok()?.then_some(title)
        });
        if let Ok(Some(title)) = titled {
            let _ = on_event.send(ChatEvent::Titled { conversation_id: id, title });
        }
    });
    if let Err(error) = started {
        tracing::warn!(error = %error, "could not start the title");
    }
}

// ---- the streamed Ask inside one topic ---------------------------------------------------------------------------

/// Everything one topic question needs, captured when it is asked.
struct TopicAsk {
    topic_id: i64,
    query: String,
    /// The conversation whose earlier exchanges go with the question as context; none for a first question.
    conversation_id: Option<i64>,
    settings: RoutingSettings,
    keys: ApiKeys,
    instructions: Instructions,
    labeler: DeviceLabeler,
}

/// Starts answering `query` from one topic's items and returns at once; the events arrive on `on_event`: the stream's, then `Sourced`
/// (the cited sources only) and `Cited` just before `Completed`. When nothing in the topic matched, the only event is `ScopeEmpty` (nothing was
/// sent). `require_local` is the route preview's word that the answer comes from this PC (the Ask control said so): the call is then held to
/// this PC exactly as `chat_ask_sourced` holds it (local-only routing, so it never reaches a hosted model and fails with the existing message
/// instead). Nothing is saved: the page keeps the answer with `conversation_save_exchange`. There is no blocking path in a topic: a
/// question that names an excluded client is prepared again for a model on this PC, and with none the existing refusal shows.
#[tauri::command]
pub fn chat_ask_topic(
    state: State<'_, AppState>,
    topic_id: i64,
    query: String,
    conversation_id: Option<i64>,
    require_local: Option<bool>,
    on_event: Channel<ChatEvent>,
) -> Result<(), CommandError> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(CommandError::invalid_input(tr("Type a question first.", &[])));
    }
    let store = state.store()?.clone();
    let (mut settings, instructions) = {
        let s = state.settings.lock().unwrap();
        (s.routing(), s.instructions(PromptSurface::Ask))
    };
    if require_local.unwrap_or(false) {
        settings.local_only = true;
    }
    let mut ask = TopicAsk { topic_id, query, conversation_id, settings, keys: ApiKeys::load(&*state.secrets)?, instructions, labeler: state.device_labeler() };
    let sync = state.sync.clone();
    // Background model requests hold back from now until the answer ends (PR 874).
    let gate = crate::user_model_gate::enter();
    std::thread::Builder::new()
        .name("whimbrel-topic-ask".into())
        .spawn(move || {
            let _gate = gate;
            crate::chat::freshen_whimbrel(sync.as_deref(), &ask.settings, &mut ask.keys);
            // A closed window drops the UI's end of the channel. The run still ends cleanly (finalized or discarded).
            let send = |event: ChatEvent| {
                let _ = on_event.send(event);
            };
            run(&store, &ask, &send);
        })
        .map_err(|e| CommandError::store(tr("Could not start the question: {error}", &[("error", &e.to_string())])))?;
    Ok(())
}

fn run(store: &StoreHandle, ask: &TopicAsk, send: &(dyn Fn(ChatEvent) + Sync)) {
    let ollama_up = chat::ollama_serves(&ask.settings.ollama_model);
    let (effective, route) = route_for(&ask.settings, &ask.keys, &ask.query, ollama_up);
    let (provider, model) = chat::answering(&effective);
    // Never the question itself: only how it was routed.
    tracing::info!(provider, model = %model, streams = route.is_some(), ollama_up, "answering a question in a topic");
    match route {
        Some(route) => attempt(store, ask, &effective, route, false, send),
        // Nothing to stream to (no provider set up, or local-only mode with a cloud model): a topic has no blocking answer.
        None => fail(send, CommandError::ProviderFailure { reason: Some(ProviderFailureReason::NoProviderConfigured) }, false),
    }
}

/// Prepares the topic's prompt under `effective` and streams it. `local_retry`: this is the second go, for a model on this PC.
fn attempt(store: &StoreHandle, ask: &TopicAsk, effective: &RoutingSettings, route: StreamRoute, local_retry: bool, send: &(dyn Fn(ChatEvent) + Sync)) {
    let routing = effective.routing_config(&ask.keys, &ask.instructions);
    let call = effective.limited_use();
    let (topic_id, query, conversation_id) = (ask.topic_id, ask.query.clone(), ask.conversation_id);
    // One store call: history and the prompt, retrieval narrowed to the topic's members.
    let gathered = store.call_blocking(move |s| {
        let history = conversation_id
            .and_then(|id| s.get_conversation(id).ok().flatten())
            .map(|c| chat::history_from(&c.exchanges))
            .unwrap_or_default();
        // Converted here: core leaves a refusal's reason on this thread, take-once (`From<ApiError>`).
        let prepared = api::ask::prepare_ask_in_topic_for(s, topic_id, &query, chat::CONTEXT_LIMIT, &history, &routing, &call).map_err(CommandError::from);
        (history, prepared)
    });
    let prepared = match gathered {
        Ok((_, prepared)) => prepared,
        Err(error) => return fail(send, error, false),
    };
    match prepared {
        // Nothing in the topic matched: nothing was stashed or sent.
        Ok(data) if data.scope_empty => send(ChatEvent::ScopeEmpty),
        Ok(data) => stream(store, ask, effective, route, data, local_retry, send),
        Err(error) if !local_retry && names_excluded_client(&error) => retry_on_this_pc(store, ask, effective, send),
        Err(error) => fail(send, error, false),
    }
}

/// The question names a client excluded from hosted models: prepared again for a model on this PC (a topic has no blocking answer).
fn retry_on_this_pc(store: &StoreHandle, ask: &TopicAsk, effective: &RoutingSettings, send: &(dyn Fn(ChatEvent) + Sync)) {
    let local = RoutingSettings { local_only: true, ..effective.clone() };
    let ollama_up = chat::ollama_serves(&local.ollama_model);
    match chat::local_instead(chat::stream_route(&local, &ask.keys), ollama_up, api::local_model::is_configured()) {
        Some(route) => attempt(store, ask, &local, route, true, send),
        None => fail(send, CommandError::ProviderFailure { reason: Some(ProviderFailureReason::ExcludedClient) }, false),
    }
}

/// Whether a stream to `provider` with `model` runs on this computer: the bundled model, or a model that is not hosted.
fn made_here(provider: &str, model: &str) -> bool {
    provider == "local" || llm::Provider::parse(provider).is_some_and(|p| !llm::is_hosted(p, model))
}

fn stream(
    store: &StoreHandle,
    ask: &TopicAsk,
    effective: &RoutingSettings,
    route: StreamRoute,
    prepared: api::ask::PreparedAskData,
    local_retry: bool,
    send: &(dyn Fn(ChatEvent) + Sync),
) {
    let Some(run) = chat::start_stream(&prepared.prompt, &route) else {
        api::ask::discard_ask(&prepared.prepare_id);
        return fail(send, CommandError::provider_failure(), false);
    };
    send(ChatEvent::Started { run_id: Some(run.run_id.clone()), provider: route.provider.into(), model: route.model.clone(), context: Vec::new() });
    if local_retry {
        send(ChatEvent::RoutedLocally);
    }
    let end = chat::forward(&run.run_id, api::inference::poll, send, || std::thread::sleep(chat::POLL_INTERVAL));
    api::inference::release(&run.run_id);
    match chat::terminal_action_for(route.provider, &end.outcome, end.produced_output) {
        TerminalAction::Finalize => {
            // Usage goes under the model that answered; a stand-in for a model the provider no longer has is recorded for Settings -> AI.
            let answered = end.model.clone().unwrap_or_else(|| route.model.clone());
            let (prepare_id, text, model, usage) = (prepared.prepare_id, end.text.clone(), answered.clone(), end.usage);
            let (provider, requested, substitute) = (llm::Provider::parse(route.provider), route.model.clone(), end.model.clone());
            let finalized = store.call_blocking(move |s| {
                if let Some(provider) = provider {
                    api::model_defaults::note_answer(s, provider, &requested, substitute.as_deref(), api::util::now_unix_ms());
                }
                api::ask::finalize_ask_sourced(s, &prepare_id, &text, &model, usage)
            });
            let (citations, cited, cited_sources, left_out, kept_local) = match finalized {
                Ok(f) => (f.citations, f.cited, f.cited_sources, f.left_out, matches!(f.exclusion, api::exclusion::ExclusionOutcome::LocalOnly)),
                Err(_) => (Vec::new(), Vec::new(), Vec::new(), 0, false),
            };
            let answered_locally = kept_local || made_here(route.provider, &answered);
            tracing::info!(chars = end.text.len(), cited = cited.len(), "question answered in a topic");
            send(ChatEvent::Sourced {
                sources: cited_cards(store, &cited_sources, &ask.labeler),
                provider: route.provider.into(),
                answered_locally,
                date_range: None,
            });
            send(ChatEvent::Cited { cited, answered_locally, provider: route.provider.to_string(), left_out });
            let shown_left_out = (left_out > 0).then_some(left_out as usize);
            send(ChatEvent::Completed { answer: end.text, citations, conversation_id: None, narrowed: false, left_out: shown_left_out });
        }
        TerminalAction::Restore => {
            api::ask::discard_ask(&prepared.prepare_id);
            tracing::info!("question stopped");
            send(ChatEvent::Cancelled);
        }
        TerminalAction::FallBack => {
            api::ask::discard_ask(&prepared.prepare_id);
            // Held back by the exclusion gate with nothing sent: a model on this PC answers instead, once.
            match chat::gate_refusal(&end.outcome) {
                Some(refusal) if !local_retry && names_excluded_client(&refusal) => retry_on_this_pc(store, ask, effective, send),
                Some(refusal) => fail(send, refusal, false),
                None => fail(send, CommandError::provider_failure(), false),
            }
        }
        TerminalAction::Fail => {
            api::ask::discard_ask(&prepared.prepare_id);
            fail(send, CommandError::provider_failure(), true);
        }
        TerminalAction::CreditExhausted => {
            api::ask::discard_ask(&prepared.prepare_id);
            tracing::info!("Whimbrel AI credit is used up");
            fail(send, CommandError::ProviderFailure { reason: Some(ProviderFailureReason::credit_exhausted()) }, false);
        }
    }
}

/// The cards for the cited sources, each under its own number, read from the live rows now. A read that fails marks every card
/// `unreadable` (and `found: false`, no text), so it is never mistaken for a deleted row.
fn cited_cards(store: &StoreHandle, cited: &[SavedSource], labeler: &DeviceLabeler) -> Vec<NumberedSourceView> {
    let keys: Vec<(String, i64)> = cited.iter().map(|c| (c.kind.clone(), c.id)).collect();
    let read = store.call_blocking(move |s| s.resolve_cards(&keys).map_err(|e| e.to_string()));
    let (cards, unreadable) = match read {
        Ok(Ok(cards)) if cards.len() == cited.len() => (cards, false),
        _ => (cited.iter().map(|c| SourceCard { kind: c.kind.clone(), id: c.id, ..Default::default() }).collect::<Vec<_>>(), true),
    };
    cited.iter().zip(cards).map(|(c, card)| NumberedSourceView { n: c.n.max(0) as usize, card: found_card(card, labeler), unreadable }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> memory::Store {
        memory::Store::open_in_memory().unwrap()
    }

    fn input(query: &str, answer: &str) -> SavedExchangeInput {
        SavedExchangeInput {
            query: query.into(),
            answer: answer.into(),
            cited: Vec::new(),
            model: "a-model".into(),
            provider: "ollama".into(),
            answered_locally: true,
            left_out: 0,
            require_local: false,
            routed_locally: false,
        }
    }

    #[test]
    fn a_scope_reads_and_writes_the_shape_the_page_uses() {
        let topic: ConversationScopeView = serde_json::from_str(r#"{"kind":"topic","id":7}"#).unwrap();
        assert_eq!(topic, ConversationScopeView::Topic { id: 7 });
        assert_eq!(memory::ConversationScope::from(topic), memory::ConversationScope::Topic(7));
        assert_eq!(serde_json::to_value(ConversationScopeView::Everything).unwrap(), serde_json::json!({"kind":"everything"}));
        assert_eq!(serde_json::to_value(ConversationScopeView::from(memory::ConversationScope::Meeting(3))).unwrap(), serde_json::json!({"kind":"meeting","id":3}));
        assert!(serde_json::from_str::<ConversationScopeView>(r#"{"kind":"topic"}"#).is_err(), "a topic scope names its topic");
        assert!(serde_json::from_str::<ConversationScopeView>(r#"{"kind":"elsewhere"}"#).is_err());
    }

    #[test]
    fn a_stream_is_made_here_for_the_bundled_model_and_for_a_model_that_is_not_hosted() {
        assert!(made_here("local", "any"));
        assert!(!made_here("anthropic", "claude-test"));
        assert!(!made_here("not-a-provider", "x"));
    }

    #[test]
    fn keeping_an_answer_makes_a_conversation_with_its_scope_and_one_exchange() {
        let s = store();
        let kept = save_exchange(&s, None, memory::ConversationScope::Everything, &input("What did Alex say?", "Alex said so."), 10).unwrap();
        let id = kept.id;
        assert!(kept.topic.is_none() && kept.cited_topics.is_empty());
        let convo = s.get_conversation_sourced(id).unwrap().unwrap();
        assert_eq!(convo.exchanges.len(), 1);
        assert_eq!((convo.exchanges[0].query.as_str(), convo.exchanges[0].answer.as_str()), ("What did Alex say?", "Alex said so."));
        assert_eq!(convo.exchanges[0].provider.as_deref(), Some("ollama"));
        assert_eq!(convo.exchanges[0].answered_locally, Some(true));
        assert_eq!(s.conversation_scope(id).unwrap(), memory::ConversationScope::Everything);
        // A follow-up goes into the same conversation.
        save_exchange(&s, Some(id), memory::ConversationScope::Everything, &input("And then?", "Then lunch."), 20).unwrap();
        assert_eq!(s.get_conversation_sourced(id).unwrap().unwrap().exchanges.len(), 2);
    }

    #[test]
    fn a_topic_conversation_is_in_its_topic_from_the_start_but_not_as_the_persons_choice() {
        let s = store();
        let topic = s.create_seed_topic("Acme", &[], None).unwrap().topic_id;
        let id = save_exchange(&s, None, memory::ConversationScope::Topic(topic), &input("q", "a"), 10).unwrap().id;
        assert_eq!(s.conversation_scope(id).unwrap(), memory::ConversationScope::Topic(topic));
        let link = s.conversation_topic(id).unwrap();
        assert_eq!(link.topic.map(|t| t.topic_id), Some(topic));
        assert!(!link.set_by_user, "the person can still change it");
    }

    #[test]
    fn a_topic_the_person_removed_is_not_joined_again_by_the_next_answer() {
        let s = store();
        let topic = s.create_seed_topic("Acme", &[], None).unwrap().topic_id;
        let id = save_exchange(&s, None, memory::ConversationScope::Topic(topic), &input("q", "a"), 10).unwrap().id;
        s.set_conversation_topic(id, None, true).unwrap();
        let after = save_exchange(&s, Some(id), memory::ConversationScope::Topic(topic), &input("q2", "a2"), 20).unwrap().topic;
        assert!(after.is_none());
        let link = s.conversation_topic(id).unwrap();
        assert!(link.topic.is_none() && link.set_by_user);
    }

    #[test]
    fn only_pointers_with_a_known_kind_and_a_number_are_kept() {
        let s = store();
        let mut e = input("q", "a [1] [2] [3]");
        e.cited = vec![
            CitedPointerInput { n: 1, kind: "capture".into(), id: 5 },
            CitedPointerInput { n: 0, kind: "capture".into(), id: 6 },
            CitedPointerInput { n: 2, kind: "not-a-kind".into(), id: 7 },
            CitedPointerInput { n: 1, kind: "episode".into(), id: 8 },
        ];
        let id = save_exchange(&s, None, memory::ConversationScope::Everything, &e, 10).unwrap().id;
        let saved = s.get_conversation_sourced(id).unwrap().unwrap();
        let kept: Vec<i64> = saved.exchanges[0].sources.iter().map(|c| c.n).collect();
        assert_eq!(kept, vec![1], "an unknown kind, a number below 1 and a repeated number are dropped");
    }


    // ---- locality is decided here, from the rows and the page's words, and the page can only make it stricter ----

    /// A held answer that, on its own words, is a hosted answer with nothing held back.
    fn hosted_input() -> SavedExchangeInput {
        SavedExchangeInput { provider: "anthropic".into(), model: "claude-test".into(), answered_locally: false, ..input("q", "a") }
    }

    #[test]
    fn a_hosted_answer_with_no_excluded_material_may_be_titled_by_a_hosted_model() {
        assert!(!must_stay_local(&hosted_input(), false));
    }

    #[test]
    fn what_the_page_says_can_only_keep_an_exchange_on_this_pc() {
        let base = hosted_input();
        assert!(must_stay_local(&base, true), "material this side found");
        assert!(must_stay_local(&SavedExchangeInput { answered_locally: true, ..base.clone() }, false));
        assert!(must_stay_local(&SavedExchangeInput { require_local: true, ..base.clone() }, false));
        assert!(must_stay_local(&SavedExchangeInput { routed_locally: true, ..base.clone() }, false));
        assert!(must_stay_local(&SavedExchangeInput { provider: "ollama".into(), model: "gemma3:4b".into(), ..base.clone() }, false), "a model on this PC");
        // A page that says "hosted" and "not local" cannot undo what was found.
        assert!(must_stay_local(&base, true));
    }

    #[test]
    fn a_missing_or_unknown_provider_fails_closed() {
        for provider in ["", "  ", "not-a-provider"] {
            assert!(must_stay_local(&SavedExchangeInput { provider: provider.into(), ..hosted_input() }, false), "{provider:?}");
        }
        assert!(!must_stay_local(&SavedExchangeInput { provider: "whimbrel".into(), model: "m".into(), ..hosted_input() }, false));
    }

    #[test]
    fn nothing_excluded_means_a_hosted_answer_is_kept_open() {
        let s = store();
        let kept = save_exchange(&s, None, memory::ConversationScope::Everything, &hosted_input(), 10).unwrap();
        assert!(!kept.local);
    }

    #[test]
    fn an_exchange_that_must_stay_local_is_marked_before_it_is_stored_so_core_keeps_it_local_only() {
        let s = store();
        // The page said hosted and not local; the provider is unknown, so it is kept local (fail closed).
        let e = SavedExchangeInput { provider: String::new(), ..input("What did Wile say?", "Wile said the price.") };
        let e = SavedExchangeInput { answered_locally: false, ..e };
        let kept = save_exchange(&s, None, memory::ConversationScope::Everything, &e, 10).unwrap();
        assert!(kept.local);
        assert!(s.is_local_only_text("Wile said the price.").unwrap(), "core's own mark, written before the append");
        assert_eq!(s.local_only_exchange_ids(kept.id).unwrap().len(), 1, "core stored the exchange as local only");
        // A hosted answer with nothing to hold back is not marked.
        let open = save_exchange(&s, None, memory::ConversationScope::Everything, &SavedExchangeInput { answer: "Plain.".into(), ..hosted_input() }, 20).unwrap();
        assert!(!open.local && !s.is_local_only_text("Plain.").unwrap());
        assert!(s.local_only_exchange_ids(open.id).unwrap().is_empty());
    }

    #[test]
    fn the_stored_local_flag_never_claims_more_than_was_true() {
        let s = store();
        let e = SavedExchangeInput { routed_locally: true, ..hosted_input() };
        let kept = save_exchange(&s, None, memory::ConversationScope::Everything, &e, 10).unwrap();
        assert!(kept.local);
        let convo = s.get_conversation_sourced(kept.id).unwrap().unwrap();
        assert_eq!(convo.exchanges[0].answered_locally, Some(false), "kept local for its title and history, not displayed as made here");
    }

    #[test]
    fn a_title_is_made_on_this_pc_when_the_setting_or_the_exchange_says_so_and_never_loosened() {
        assert!(!title_held_local(false, false));
        assert!(title_held_local(true, false), "the person's own setting holds");
        assert!(title_held_local(false, true));
        assert!(title_held_local(true, true));
        // The decision the save makes feeds it: a hosted answer with nothing held back is titled by the model the settings choose; the same
        // answer under any one of the page's local words is not.
        let base = hosted_input();
        assert!(!title_held_local(false, must_stay_local(&base, false)));
        assert!(title_held_local(false, must_stay_local(&SavedExchangeInput { routed_locally: true, ..base.clone() }, false)));
        assert!(title_held_local(false, must_stay_local(&base, true)), "excluded material found on this side");
    }

    #[test]
    fn a_hosted_looking_answer_is_never_stored_as_made_here_whatever_the_page_said() {
        // The page says "answered locally" for each of these; the shell's own rule (core's `is_hosted`) says they ran off this PC.
        for (provider, model) in [("anthropic", "claude-test"), ("ollama", "gpt-oss:120b-cloud"), ("ollama", "glm-4.6:cloud"), ("ollama", "kimi-k2:cloud-1t"), ("ollama", "Qwen3:Cloud"), ("not-a-provider", "x"), ("", "")] {
            let e = SavedExchangeInput { provider: provider.into(), model: model.into(), answered_locally: true, ..input("q", "a") };
            assert!(!displayed_local(&e), "{provider} {model}");
            let s = store();
            let id = save_exchange(&s, None, memory::ConversationScope::Everything, &e, 10).unwrap().id;
            assert_eq!(s.get_conversation_sourced(id).unwrap().unwrap().exchanges[0].answered_locally, Some(false), "{provider} {model}");
        }
        // A model on this PC (or the bundled one) is shown as made here even when the page did not say so.
        for (provider, model) in [("local", "any"), ("ollama", "gemma3:4b"), ("ollama", "cloudy-name:7b")] {
            let e = SavedExchangeInput { provider: provider.into(), model: model.into(), answered_locally: false, ..input("q", "a") };
            assert!(displayed_local(&e), "{provider} {model}");
        }
    }

    /// An excluded client, as core's own tests set one up: a seed topic with `exclude_hosted`. Returns the topic's id.
    fn exclude(s: &memory::Store, name: &str) -> i64 {
        let topic = s.create_seed_topic(name, &[], None).unwrap();
        s.set_client_control(&topic.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        topic.topic_id
    }

    #[test]
    fn a_hosted_answer_that_names_an_excluded_client_is_kept_on_this_pc() {
        let s = store();
        exclude(&s, "Acme");
        let e = SavedExchangeInput { query: "What did Acme decide?".into(), answer: "Acme agreed to the plan.".into(), ..hosted_input() };
        let kept = save_exchange(&s, None, memory::ConversationScope::Everything, &e, 10).unwrap();
        assert!(kept.local);
        assert!(s.is_local_only_text("Acme agreed to the plan.").unwrap());
        assert_eq!(s.local_only_exchange_ids(kept.id).unwrap().len(), 1);
        // Hosted, so it is not displayed as made here.
        assert_eq!(s.get_conversation_sourced(kept.id).unwrap().unwrap().exchanges[0].answered_locally, Some(false));
        // Another client's words do not trip it.
        let other = SavedExchangeInput { query: "What did Globex say?".into(), answer: "Globex was fine.".into(), ..hosted_input() };
        assert!(!save_exchange(&s, None, memory::ConversationScope::Everything, &other, 20).unwrap().local);
    }

    #[test]
    fn an_excluded_clients_topic_is_checked_by_its_scope() {
        let s = store();
        let acme = exclude(&s, "Acme");
        let plain = hosted_input();
        assert!(!save_exchange(&s, None, memory::ConversationScope::Everything, &plain, 10).unwrap().local);
        assert!(save_exchange(&s, None, memory::ConversationScope::Topic(acme), &plain, 20).unwrap().local);
    }

    #[test]
    fn a_follow_up_is_checked_under_the_stored_scope_not_the_one_the_page_sent() {
        let s = store();
        let acme = exclude(&s, "Acme");
        let first = save_exchange(&s, None, memory::ConversationScope::Topic(acme), &hosted_input(), 10).unwrap();
        assert!(first.local, "a conversation in an excluded client's topic stays on this PC");
        // The page sends "everything" for the same conversation, with words that name no client: the stored scope still decides.
        let follow_up = SavedExchangeInput { query: "and then?".into(), answer: "Then lunch.".into(), ..hosted_input() };
        let kept = save_exchange(&s, Some(first.id), memory::ConversationScope::Everything, &follow_up, 20).unwrap();
        assert!(kept.local);
        assert!(s.is_local_only_text("Then lunch.").unwrap());
        // The same words in a conversation that is not an excluded client's are not held back.
        let open = save_exchange(&s, None, memory::ConversationScope::Everything, &hosted_input(), 30).unwrap();
        let again = save_exchange(&s, Some(open.id), memory::ConversationScope::Topic(acme), &SavedExchangeInput { answer: "Fine.".into(), ..follow_up }, 40).unwrap();
        assert!(!again.local, "the page's scope is used only to make a new conversation");
    }

    #[test]
    fn a_cited_row_that_cannot_be_read_keeps_the_exchange_on_this_pc_once_a_client_is_excluded() {
        let s = store();
        let mut e = SavedExchangeInput { cited: vec![CitedPointerInput { n: 1, kind: "capture".into(), id: 404 }], ..hosted_input() };
        // With no client excluded, nothing is checked and the exchange stays open.
        assert!(!save_exchange(&s, None, memory::ConversationScope::Everything, &e, 10).unwrap().local);
        exclude(&s, "Acme");
        // Core's rule fails closed on a gone row and on a kind it cannot read in full.
        assert!(save_exchange(&s, None, memory::ConversationScope::Everything, &e, 20).unwrap().local);
        e.cited[0].kind = "not-a-kind".into();
        assert!(save_exchange(&s, None, memory::ConversationScope::Everything, &e, 30).unwrap().local);
    }

    #[test]
    fn choosing_a_topic_for_a_conversation_that_is_gone_writes_nothing() {
        let s = store();
        let topic = s.create_seed_topic("Acme", &[], None).unwrap().topic_id;
        let id = save_exchange(&s, None, memory::ConversationScope::Everything, &input("q", "a"), 10).unwrap().id;
        assert_eq!(set_topic_link(&s, id, Some(topic)).unwrap().topic.map(|t| t.topic_id), Some(topic));
        s.delete_conversation(id).unwrap();
        assert!(set_topic_link(&s, id, Some(topic)).is_err(), "gone");
        assert!(set_topic_link(&s, id, None).is_err(), "gone, whatever is asked");
        assert!(s.conversation_topic(id).unwrap().topic.is_none(), "no orphan row");
        assert!(set_topic_link(&s, 9999, Some(topic)).is_err());
    }

    #[test]
    fn the_cards_for_cited_sources_keep_their_numbers_and_a_gone_row_shows_nothing() {
        let lane = StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap();
        let cited = [SavedSource { n: 2, kind: "capture".into(), id: 404 }];
        let cards = cited_cards(&lane, &cited, &DeviceLabeler::default());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].n, 2);
        assert!(!cards[0].card.found && !cards[0].unreadable && cards[0].card.text.is_none());
    }
}
