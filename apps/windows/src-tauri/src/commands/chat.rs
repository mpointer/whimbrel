//! Ask (plan Task 8): grounded chat over the memory store, with persisted conversations.
//!
//! Each question runs on its own thread, which talks to the store actor with
//! `call_blocking` and reports to the UI through a Tauri `Channel`. Every provider with
//! a key streams through `api::inference`. A hosted provider with no key, and every
//! fallback, use the blocking `api::ask::answer`, which holds the store for the provider
//! call (Decision 9). The decisions live in `crate::chat`.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::chat::{self, ChatEvent, ContextEntry, StreamRoute, TerminalAction};
use crate::error::CommandError;
use crate::settings::{ApiKeys, Instructions, PromptSurface, RoutingSettings};
use crate::state::AppState;
use crate::store_actor::StoreHandle;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRow {
    pub id: i64,
    pub title: String,
    pub starred: bool,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeView {
    pub query: String,
    pub answer: String,
    pub created_at_unix_ms: i64,
    /// The entries the answer cited, as saved. Empty for exchanges saved before
    /// citations were stored.
    pub citations: Vec<ContextEntry>,
    /// The model that answered; `None` for exchanges saved before it was stored.
    pub model: Option<String>,
}

/// A saved conversation.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationView {
    pub id: i64,
    pub title: String,
    pub starred: bool,
    pub exchanges: Vec<ExchangeView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StarterPromptView {
    pub id: String,
    pub title: String,
    pub query: String,
}

/// The Ask composer's "X¢ / cap¢ today". Spend is the anonymous identity's, which is
/// what the user cap gates.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatSpend {
    pub user_cents: i64,
    /// `None`: no cap (the setting is 0).
    pub cap_cents: Option<i64>,
}

/// Unset means the core's default cap; 0 means none.
pub fn user_cap_cents(routing: &RoutingSettings) -> Option<i64> {
    let cap = routing.user_daily_cap_cents.unwrap_or(llm::DEFAULT_USER_DAILY_CAP_CENTS);
    (cap != 0).then_some(cap)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Everything one question needs, captured when it is asked.
struct Ask {
    query: String,
    /// `None` starts a new conversation with the first answered exchange.
    conversation_id: Option<i64>,
    settings: RoutingSettings,
    keys: ApiKeys,
    instructions: Instructions,
    /// The slow lane, for the calls that hold a store across a provider call (the
    /// blocking answer and the title). Everything else uses the main store.
    slow: StoreHandle,
}

/// Starts answering `query` and returns at once; the answer arrives on `on_event`.
#[tauri::command]
pub fn chat_ask(state: State<'_, AppState>, query: String, conversation_id: Option<i64>, on_event: Channel<ChatEvent>) -> Result<(), CommandError> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(CommandError::invalid_input("Type a question first."));
    }
    let store = state.store()?.clone();
    let slow = state.slow_store()?.clone();
    let (settings, instructions) = {
        let s = state.settings.lock().unwrap();
        (s.routing(), s.instructions(PromptSurface::Ask))
    };
    let ask = Ask { query, conversation_id, settings, keys: ApiKeys::load(&*state.secrets)?, instructions, slow };
    std::thread::Builder::new()
        .name("whimbrel-ask".into())
        .spawn(move || {
            // A closed window drops the UI's end of the channel. The answer is still
            // finished and persisted, so a failed send is ignored.
            let send = |event: ChatEvent| {
                let _ = on_event.send(event);
            };
            run(&store, &ask, &send);
        })
        .map_err(|e| CommandError::store(format!("could not start the question: {e}")))?;
    Ok(())
}

/// Stops a streaming answer; it ends with `Cancelled`.
#[tauri::command]
pub fn chat_stop(run_id: String) -> bool {
    api::inference::cancel(&run_id)
}

fn run(store: &StoreHandle, ask: &Ask, send: &dyn Fn(ChatEvent)) {
    let ollama_up = chat::ollama_serves(&ask.settings.ollama_model);
    let effective = chat::effective_routing(&ask.settings, &ask.query, ollama_up);
    let route = chat::stream_route(&effective, &ask.keys);
    let (provider, model) = chat::answering(&effective);
    // Never the question itself: only how it was routed.
    tracing::info!(
        provider,
        model = %model,
        streams = route.is_some(),
        ollama_up,
        time_critical = api::prompts::is_time_critical(&ask.query),
        "answering a question"
    );
    let routing = effective.routing_config(&ask.keys, &ask.instructions);
    let (query, conversation_id, streams) = (ask.query.clone(), ask.conversation_id, route.is_some());
    // One store call: history, the citation context, and (to stream) the prompt.
    let gathered = store.call_blocking(move |s| {
        let history = conversation_id
            .and_then(|id| s.get_conversation(id).ok().flatten())
            .map(|c| chat::history_from(&c.exchanges))
            .unwrap_or_default();
        let context = chat::fetch_context(s, &query);
        let prepared = streams.then(|| api::ask::prepare_ask(s, &query, chat::CONTEXT_LIMIT, &history, &routing));
        (history, context, prepared)
    });
    let (history, context, prepared) = match gathered {
        Ok(g) => g,
        Err(error) => return fail(send, error, false),
    };
    match (route, prepared) {
        (Some(route), Some(Ok(prepared))) => stream(store, ask, &effective, route, prepared, history, context, send),
        // The spend cap, checked before any provider is tried.
        (_, Some(Err(error))) => fail(send, error.into(), false),
        _ => {
            send(blocking_started(&effective, &context));
            blocking(store, ask, &effective, history, &context, send);
        }
    }
}

/// Reports a failed question to the UI and the log (the error, never the question).
fn fail(send: &dyn Fn(ChatEvent), error: CommandError, partial: bool) {
    tracing::warn!(error = %error, partial, "question failed");
    send(ChatEvent::Failed { error, partial });
}

fn blocking_started(effective: &RoutingSettings, context: &[ContextEntry]) -> ChatEvent {
    let (provider, model) = chat::answering(effective);
    ChatEvent::Started { run_id: None, provider: provider.into(), model, context: context.to_vec() }
}

#[allow(clippy::too_many_arguments)]
fn stream(
    store: &StoreHandle,
    ask: &Ask,
    effective: &RoutingSettings,
    route: StreamRoute,
    prepared: api::ask::PreparedAskData,
    history: Vec<llm::ChatTurn>,
    context: Vec<ContextEntry>,
    send: &dyn Fn(ChatEvent),
) {
    let Some(run) = api::inference::start(&prepared.prompt, &route.model, route.provider, route.key.as_deref(), None) else {
        api::ask::discard_ask(&prepared.prepare_id);
        send(blocking_started(effective, &context));
        return blocking(store, ask, effective, history, &context, send);
    };
    send(ChatEvent::Started { run_id: Some(run.run_id.clone()), provider: route.provider.into(), model: route.model.clone(), context: context.clone() });
    let end = chat::forward(&run.run_id, api::inference::poll, send, || std::thread::sleep(chat::POLL_INTERVAL));
    api::inference::release(&run.run_id);
    match chat::terminal_action(&end.outcome, end.produced_output) {
        TerminalAction::Finalize => {
            let (prepare_id, text, model, usage) = (prepared.prepare_id, end.text.clone(), route.model.clone(), end.usage);
            let citations = store
                .call_blocking(move |s| api::ask::finalize_ask(s, &prepare_id, &text, &model, usage).citations)
                .unwrap_or_default();
            finish(store, ask, end.text, citations, &context, &route.model, send);
        }
        TerminalAction::Restore => {
            api::ask::discard_ask(&prepared.prepare_id);
            tracing::info!("question stopped");
            send(ChatEvent::Cancelled);
        }
        TerminalAction::FallBack => {
            api::ask::discard_ask(&prepared.prepare_id);
            tracing::info!(provider = route.provider, "stream failed before any output; answering on the blocking path");
            send(ChatEvent::FellBack);
            blocking(store, ask, effective, history, &context, send);
        }
        TerminalAction::Fail => {
            api::ask::discard_ask(&prepared.prepare_id);
            fail(send, CommandError::ProviderFailure, true);
        }
    }
}

fn blocking(store: &StoreHandle, ask: &Ask, effective: &RoutingSettings, history: Vec<llm::ChatTurn>, context: &[ContextEntry], send: &dyn Fn(ChatEvent)) {
    let (_, model) = chat::answering(effective);
    let routing = effective.routing_config(&ask.keys, &ask.instructions);
    let (query, ollama_model) = (ask.query.clone(), effective.ollama_model.clone());
    match ask.slow.call_blocking(move |s| api::ask::answer(s, &query, &ollama_model, chat::CONTEXT_LIMIT, &history, &routing)) {
        Ok(Ok(answer)) => finish(store, ask, answer.text, answer.citations, context, &model, send),
        Ok(Err(error)) => fail(send, error.into(), false),
        Err(error) => fail(send, error, false),
    }
}

/// Persists the exchange, shows the answer, then (for a new conversation) generates its
/// title. Title generation is best effort and never retried: any failure leaves the
/// placeholder.
fn finish(
    store: &StoreHandle,
    ask: &Ask,
    answer: String,
    citations: Vec<llm::ContextItemRef>,
    context: &[ContextEntry],
    model: &str,
    send: &dyn Fn(ChatEvent),
) {
    tracing::info!(chars = answer.len(), citations = citations.len(), "question answered");
    let cited: Vec<&ContextEntry> = context.iter().filter(|e| citations.contains(&e.item)).collect();
    let citations_json = serde_json::to_string(&cited).unwrap_or_else(|_| "[]".into());
    let conversation_id = persist(store, ask.conversation_id, &ask.query, &answer, citations_json, model);
    send(ChatEvent::Completed { answer: answer.clone(), citations, conversation_id });
    let (None, Some(id)) = (ask.conversation_id, conversation_id) else { return };
    let routing = chat::title_routing(&ask.settings, chat::ollama_serves(&ask.settings.ollama_model))
        .routing_config(&ask.keys, &Instructions::default());
    let (query, ollama_model) = (ask.query.clone(), ask.settings.ollama_model.clone());
    let titled = ask.slow.call_blocking(move |s| {
        let title = api::ask::generate_conversation_title(s, &ollama_model, &query, &answer, &routing).ok()?.title;
        s.set_conversation_title_if_placeholder(id, &title).ok()?.then_some(title)
    });
    if let Ok(Some(title)) = titled {
        send(ChatEvent::Titled { conversation_id: id, title });
    }
}

/// Creates the conversation on its first answered exchange, then appends. `None` if the
/// write failed (logged): the answer is still shown.
fn persist(store: &StoreHandle, conversation_id: Option<i64>, query: &str, answer: &str, citations_json: String, model: &str) -> Option<i64> {
    let (query, answer, model, now) = (query.to_string(), answer.to_string(), model.to_string(), now_ms());
    let result = store.call_blocking(move |s| -> Result<i64, String> {
        let id = match conversation_id {
            Some(id) => id,
            None => s.insert_conversation(&query, now).map_err(|e| e.to_string())?,
        };
        s.append_conversation_exchange(id, &query, &answer, &citations_json, Some(&model), now).map_err(|e| e.to_string())?;
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

async fn conversation_rows(store: &StoreHandle) -> Result<Vec<ConversationRow>, CommandError> {
    let rows = store.call(|s| s.list_conversations().map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    Ok(rows
        .into_iter()
        .map(|c| ConversationRow { id: c.id, title: c.title, starred: c.starred, updated_at_unix_ms: c.updated_at_unix_ms })
        .collect())
}

/// Starred first, then most recently updated.
#[tauri::command]
pub async fn chat_conversations(state: State<'_, AppState>) -> Result<Vec<ConversationRow>, CommandError> {
    conversation_rows(state.store()?).await
}

#[tauri::command]
pub async fn chat_conversation(state: State<'_, AppState>, id: i64) -> Result<Option<ConversationView>, CommandError> {
    let conversation = state.store()?.call(move |s| s.get_conversation(id).map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    Ok(conversation.map(|c| ConversationView {
        id: c.id,
        title: c.title,
        starred: c.starred,
        exchanges: c
            .exchanges
            .into_iter()
            .map(|e| ExchangeView {
                citations: serde_json::from_str(&e.citations_json).unwrap_or_default(),
                query: e.query,
                answer: e.answer,
                created_at_unix_ms: e.created_at_unix_ms,
                model: e.model,
            })
            .collect(),
    }))
}

/// A manual rename always wins over a title still being generated.
#[tauri::command]
pub async fn chat_rename_conversation(state: State<'_, AppState>, id: i64, title: String) -> Result<Vec<ConversationRow>, CommandError> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(CommandError::invalid_input("A conversation needs a title."));
    }
    let store = state.store()?;
    store.call(move |s| s.rename_conversation(id, &title).map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    conversation_rows(store).await
}

#[tauri::command]
pub async fn chat_star_conversation(state: State<'_, AppState>, id: i64, starred: bool) -> Result<Vec<ConversationRow>, CommandError> {
    let store = state.store()?;
    store.call(move |s| s.set_conversation_starred(id, starred).map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    conversation_rows(store).await
}

#[tauri::command]
pub async fn chat_delete_conversation(state: State<'_, AppState>, id: i64) -> Result<Vec<ConversationRow>, CommandError> {
    let store = state.store()?;
    store.call(move |s| s.delete_conversation(id).map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    conversation_rows(store).await
}

#[tauri::command]
pub fn chat_starter_prompts() -> Vec<StarterPromptView> {
    api::prompts::STARTER_PROMPTS
        .iter()
        .map(|p| StarterPromptView { id: p.id.into(), title: p.title.into(), query: p.query.into() })
        .collect()
}

#[tauri::command]
pub async fn chat_spend(state: State<'_, AppState>) -> Result<ChatSpend, CommandError> {
    let cap_cents = user_cap_cents(&state.settings.lock().unwrap().routing());
    let user_cents = state.store()?.call(|s| api::spend::spend_today(s, None, now_ms()).user_cents).await?;
    Ok(ChatSpend { user_cents, cap_cents })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> StoreHandle {
        StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap()
    }

    #[test]
    fn the_user_cap_is_the_core_default_unless_set_and_zero_means_none() {
        let with = |cap| RoutingSettings { user_daily_cap_cents: cap, ..RoutingSettings::default() };
        assert_eq!(user_cap_cents(&with(None)), Some(llm::DEFAULT_USER_DAILY_CAP_CENTS));
        assert_eq!(user_cap_cents(&with(Some(500))), Some(500));
        assert_eq!(user_cap_cents(&with(Some(0))), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_first_answer_creates_a_conversation_and_later_ones_append() {
        let store = store();
        let (s1, s2) = (store.clone(), store.clone());
        let cited = ContextEntry { item: llm::ContextItemRef::Episode { id: 7 }, label: "Figma".into(), ts_unix_ms: Some(1), open: Some(chat::LibraryTarget::Episode { id: 7 }) };
        let json = serde_json::to_string(&[&cited]).unwrap();
        let id = tokio::task::spawn_blocking(move || persist(&s1, None, "what did I plan?", "The falcon migration.", json, "gemini-2.5-flash"))
            .await
            .unwrap()
            .expect("created");
        let again = tokio::task::spawn_blocking(move || persist(&s2, Some(id), "and after that?", "Nothing yet.", "[]".into(), "llama3")).await.unwrap();
        assert_eq!(again, Some(id));

        let rows = conversation_rows(&store).await.unwrap();
        assert_eq!(rows.iter().map(|r| (r.id, r.title.as_str())).collect::<Vec<_>>(), [(id, "what did I plan?")], "the placeholder is the first question");
        let history = store
            .call(move |s| chat::history_from(&s.get_conversation(id).unwrap().unwrap().exchanges))
            .await
            .unwrap();
        assert_eq!(history.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["what did I plan?", "The falcon migration.", "and after that?", "Nothing yet."]);

        let saved = store.call(move |s| s.get_conversation(id).unwrap().unwrap().exchanges).await.unwrap();
        let first: Vec<ContextEntry> = serde_json::from_str(&saved[0].citations_json).unwrap();
        assert_eq!(first, [cited], "a reopened answer gets back the entries it cited");
        assert_eq!(saved.iter().map(|e| e.model.as_deref()).collect::<Vec<_>>(), [Some("gemini-2.5-flash"), Some("llama3")]);
    }

    /// `cargo test --lib -- --ignored ask_chain_links_answer --nocapture`. Sends a tiny
    /// prompt through every link `build_chain` makes from the saved settings and keys,
    /// the way a blocking Ask does: once routed Ollama-first (a synthesis question), once
    /// as preferred. Prints each link's reply or error, with every key redacted. Costs a
    /// few tokens per hosted provider.
    #[test]
    #[ignore = "uses the saved settings and keys and calls every provider"]
    fn ask_chain_links_answer() {
        let paths = crate::paths::Paths::from_env().unwrap();
        let settings = crate::settings::SettingsFile::load(&paths.settings).routing();
        let keys = ApiKeys::load(&crate::secrets::CredentialManager).unwrap();
        let secrets: Vec<String> = [&keys.anthropic, &keys.openai, &keys.google].into_iter().flatten().cloned().collect();
        let redact = |s: String| secrets.iter().fold(s, |s, k| s.replace(k.as_str(), "<key>"));
        for (label, routing) in [("Ollama first", chat::title_routing(&settings, true)), ("as preferred", settings.clone())] {
            let config = routing.routing_config(&keys, &Instructions::default());
            println!("== {label}: preferred {} / {}", config.preferred_provider, config.preferred_model);
            for link in api::routing::build_chain(&routing.ollama_model, &config) {
                let outcome = match link.client.complete("Reply with the single word OK.") {
                    Ok(c) => format!("ok: {:?}", c.text.chars().take(40).collect::<String>()),
                    Err(e) => redact(format!("error: {e:?}")),
                };
                println!("  {:?} {}: {}", link.provider, link.model, outcome);
            }
        }
    }

    #[test]
    fn starter_prompts_are_the_macs_six() {
        let prompts = chat_starter_prompts();
        assert_eq!(prompts.len(), 6);
        assert_eq!(prompts[0].title, "What's quietly slipping?");
    }
}
