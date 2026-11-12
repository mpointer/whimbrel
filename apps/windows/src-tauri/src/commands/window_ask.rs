//! The hotkey panel's "Ask about this window" (spec §5, §6): the question is answered from
//! the window the panel read (`hotkey_panel::hotkey_panel_read`) plus memory background,
//! through `api::window_ask::prepare_window_ask` and the same streaming path as Ask.
//!
//! Differences from Ask (`commands/chat.rs`), all so the panel's destination line stays
//! true: the route is the default provider, never Ask's per-question Ollama substitution
//! (the line is shown before a question exists); there is no blocking fallback chain, so
//! a provider that can't stream, or a stream that fails, is reported instead of being
//! answered by another provider; and nothing is saved. The question, the window text and
//! the answer are never logged.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use super::chat::fail;
use crate::chat::{self, ChatEvent, OllamaState, StreamRoute, TerminalAction};
use crate::error::{CommandError, ProviderFailureReason};
use crate::hotkey_panel::{DestinationView, HotkeyPanel};
use crate::l10n::tr;
use crate::settings::{ApiKeys, PromptSurface, RoutingSettings};
use crate::state::AppState;
use crate::store_actor::StoreHandle;

/// `window_ask`'s answer. The answer itself arrives on the channel as `ChatEvent`s.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowAskReply {
    /// Prepared and streaming.
    #[serde(rename_all = "camelCase")]
    Started { request_id: u64, destination: DestinationView, truncated: bool },
    /// The request is not the panel's current one (a newer press, or the panel closed):
    /// nothing was sent.
    #[serde(rename_all = "camelCase")]
    Stale { request_id: u64 },
    /// The AI settings changed since the panel showed where the text would go: nothing was
    /// sent, and the panel shows the new destination before asking again.
    #[serde(rename_all = "camelCase")]
    DestinationChanged { request_id: u64, destination: DestinationView },
}

/// The stream that answers for the panel, or why its destination can't answer from here.
/// Ask's `stream_route` on the default provider, with the small local model standing in
/// for an Ollama that isn't serving. Never a different provider.
pub fn route_or_reason(settings: &RoutingSettings, keys: &ApiKeys) -> Result<StreamRoute, ProviderFailureReason> {
    let route = chat::stream_route(settings, keys).ok_or(ProviderFailureReason::NoProviderConfigured)?;
    if route.provider != "ollama" {
        return Ok(route);
    }
    let ollama = chat::ollama_state(&settings.ollama_model);
    pick_local(route, ollama, api::local_model::is_configured(), settings.local_only)
}

/// The Ollama route when Ollama serves the model, else the local model if installed, else
/// why not, as something the user can act on: start an installed Ollama, pull the model
/// into a running one; "no local model" only when there is no local model at all (no
/// Ollama, no bundled model) in local-only mode.
pub fn pick_local(route: StreamRoute, ollama: OllamaState, local_installed: bool, local_only: bool) -> Result<StreamRoute, ProviderFailureReason> {
    let ollama_up = ollama == OllamaState::Serves;
    let model = route.model.clone();
    match chat::local_instead(Some(route), ollama_up, local_installed) {
        Some(r) if r.provider == "ollama" && !ollama_up => Err(match ollama {
            OllamaState::ModelMissing => ProviderFailureReason::OllamaModelMissing { model },
            OllamaState::Absent if local_only => ProviderFailureReason::LocalModelMissing,
            _ => ProviderFailureReason::OllamaNotRunning,
        }),
        Some(r) => Ok(r),
        None => Err(ProviderFailureReason::NoProviderConfigured),
    }
}

/// What the UI shows for a prepare that failed.
pub fn window_ask_error(error: api::window_ask::WindowAskError) -> CommandError {
    use api::window_ask::WindowAskError;
    match error {
        // `hotkey_panel_read` keeps only reads with text, so this means the panel asked
        // without one; the panel already says it couldn't read the window.
        WindowAskError::NoText => CommandError::invalid_input(String::new()),
        WindowAskError::LocalModelMissing => CommandError::ProviderFailure { reason: Some(ProviderFailureReason::LocalModelMissing) },
        WindowAskError::Api(e) => e.into(),
    }
}

/// Asks `query` about the window read for `request_id`. Returns once the ask is prepared;
/// the answer streams on `on_event`. Stop it with `chat_stop`.
#[tauri::command]
pub async fn window_ask(app: AppHandle, request_id: u64, query: String, on_event: Channel<ChatEvent>) -> Result<WindowAskReply, CommandError> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(CommandError::invalid_input(tr("Type a question first.", &[])));
    }
    tauri::async_runtime::spawn_blocking(move || prepare_and_stream(&app, request_id, query, on_event))
        .await
        .map_err(|e| CommandError::store(tr("Could not start the question: {error}", &[("error", &e.to_string())])))?
}

fn prepare_and_stream(app: &AppHandle, request_id: u64, query: String, on_event: Channel<ChatEvent>) -> Result<WindowAskReply, CommandError> {
    let state = app.state::<AppState>();
    let panel = app.state::<HotkeyPanel>();
    let (Some(input), Some(shown)) = (panel.requests.input(request_id), panel.requests.shown(request_id)) else {
        return Ok(WindowAskReply::Stale { request_id });
    };
    let store = state.store()?.clone();
    // Routing exactly as `chat_ask` builds it, from the default provider.
    let (settings, instructions) = {
        let s = state.settings.lock().unwrap();
        (s.routing(), s.instructions(PromptSurface::Ask))
    };
    let mut keys = ApiKeys::load(&*state.secrets)?;
    // On ask only, never on open (the Mac's `beforeAsk`).
    chat::freshen_whimbrel(state.sync.as_deref(), &settings, &mut keys);
    let routing = settings.routing_config(&keys, &instructions);
    let destination = api::window_ask::destination(&routing);
    if destination != shown {
        // The page shows the new line; asking again then goes ahead without a new read.
        panel.requests.set_shown(request_id, destination.clone());
        return Ok(WindowAskReply::DestinationChanged { request_id, destination: DestinationView::from(&destination) });
    }
    let route = route_or_reason(&settings, &keys).map_err(|reason| CommandError::ProviderFailure { reason: Some(reason) })?;
    // Never the question or the window: only how it was routed.
    tracing::info!(provider = route.provider, model = %route.model, "answering a question about a window");
    let prepared = store
        .call_blocking(move |s| api::window_ask::prepare_window_ask(s, &input, &query, &[], &routing))?
        .map_err(window_ask_error)?;
    let reply = WindowAskReply::Started { request_id, destination: DestinationView::from(&prepared.destination), truncated: prepared.truncated };
    let app = app.clone();
    std::thread::Builder::new()
        .name("whimbrel-window-ask".into())
        .spawn(move || {
            // A closed panel drops the UI's end of the channel; the run still ends cleanly.
            let send = |event: ChatEvent| {
                let _ = on_event.send(event);
            };
            stream(&app, request_id, &store, route, prepared, &send);
        })
        .map_err(|e| CommandError::store(tr("Could not start the question: {error}", &[("error", &e.to_string())])))?;
    Ok(reply)
}

fn stream(app: &AppHandle, request_id: u64, store: &StoreHandle, route: StreamRoute, prepared: api::window_ask::PreparedWindowAsk, send: &dyn Fn(ChatEvent)) {
    let panel = app.state::<HotkeyPanel>();
    // Closed while preparing: nothing is sent.
    if !panel.requests.is_current(request_id) {
        api::ask::discard_ask(&prepared.prepare_id);
        return send(ChatEvent::Cancelled);
    }
    let Some(run) = chat::start_stream(&prepared.prompt, &route) else {
        api::ask::discard_ask(&prepared.prepare_id);
        return fail(send, CommandError::provider_failure(), false);
    };
    // Recorded under the lock a close takes, so a close in between still stops it: if the
    // panel closed since the check above, stop at once (the Restore arm discards the ask).
    if !panel.requests.start_run(request_id, &run.run_id) {
        api::inference::cancel(&run.run_id);
    }
    send(ChatEvent::Started { run_id: Some(run.run_id.clone()), provider: route.provider.into(), model: route.model.clone(), context: vec![] });
    let end = chat::forward(&run.run_id, api::inference::poll, send, || std::thread::sleep(chat::POLL_INTERVAL));
    api::inference::release(&run.run_id);
    panel.requests.end_run(&run.run_id);
    match chat::terminal_action_for(route.provider, &end.outcome, end.produced_output) {
        TerminalAction::Finalize => {
            let (prepare_id, text, model, usage) = (prepared.prepare_id, end.text.clone(), route.model.clone(), end.usage);
            let citations = store
                .call_blocking(move |s| api::ask::finalize_ask(s, &prepare_id, &text, &model, usage).citations)
                .unwrap_or_default();
            send(ChatEvent::Completed { answer: end.text, citations, conversation_id: None, narrowed: false });
        }
        TerminalAction::Restore => {
            api::ask::discard_ask(&prepared.prepare_id);
            send(ChatEvent::Cancelled);
        }
        // No fallback: another provider would answer where the destination line didn't say.
        TerminalAction::FallBack | TerminalAction::Fail => {
            api::ask::discard_ask(&prepared.prepare_id);
            fail(send, CommandError::provider_failure(), end.produced_output);
        }
        TerminalAction::CreditExhausted => {
            api::ask::discard_ask(&prepared.prepare_id);
            fail(send, CommandError::ProviderFailure { reason: Some(ProviderFailureReason::CreditExhausted) }, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(provider: &'static str) -> StreamRoute {
        StreamRoute { provider, model: "m".into(), key: None, device_key: None }
    }

    #[test]
    fn ollama_answers_when_serving_else_the_local_model_else_why_not() {
        use OllamaState::*;
        assert_eq!(pick_local(route("ollama"), Serves, true, false).unwrap().provider, "ollama");
        assert_eq!(pick_local(route("ollama"), NotRunning, true, true).unwrap().provider, "local");
        assert_eq!(pick_local(route("ollama"), Absent, false, true).unwrap_err(), ProviderFailureReason::LocalModelMissing);
        assert_eq!(pick_local(route("ollama"), Absent, false, false).unwrap_err(), ProviderFailureReason::OllamaNotRunning);
    }

    #[test]
    fn an_installed_ollama_that_is_stopped_says_start_it_not_no_local_model() {
        use OllamaState::*;
        assert_eq!(pick_local(route("ollama"), NotRunning, false, true).unwrap_err(), ProviderFailureReason::OllamaNotRunning);
        assert_eq!(pick_local(route("ollama"), NotRunning, false, false).unwrap_err(), ProviderFailureReason::OllamaNotRunning);
    }

    #[test]
    fn a_running_ollama_without_the_model_names_it() {
        let missing = ProviderFailureReason::OllamaModelMissing { model: "m".into() };
        assert_eq!(pick_local(route("ollama"), OllamaState::ModelMissing, false, true).unwrap_err(), missing);
        assert_eq!(pick_local(route("ollama"), OllamaState::ModelMissing, false, false).unwrap_err(), missing);
        assert_eq!(pick_local(route("ollama"), OllamaState::ModelMissing, true, true).unwrap().provider, "local");
    }

    #[test]
    fn a_hosted_provider_without_a_key_has_no_route() {
        let settings = RoutingSettings { preferred_provider: crate::settings::Provider::Anthropic, local_only: false, ..RoutingSettings::default() };
        assert_eq!(route_or_reason(&settings, &ApiKeys::default()).unwrap_err(), ProviderFailureReason::NoProviderConfigured);
    }

    #[test]
    fn a_missing_local_model_has_its_own_kind_and_no_ollama_advice() {
        use api::window_ask::WindowAskError;
        let json = serde_json::to_value(window_ask_error(WindowAskError::LocalModelMissing)).unwrap();
        assert_eq!(json["reason"]["kind"], "local_model_missing");
        assert_eq!(ProviderFailureReason::LocalModelMissing.message(), None);
    }

    #[test]
    fn prepare_errors_keep_their_kind() {
        use api::window_ask::WindowAskError;
        assert_eq!(serde_json::to_value(window_ask_error(WindowAskError::NoText)).unwrap()["kind"], "invalid_input");
        let failed = serde_json::to_value(window_ask_error(WindowAskError::Api(api::ApiError::ProviderFailure))).unwrap();
        assert_eq!(failed["kind"], "provider_failure");
    }

    #[test]
    fn replies_are_tagged_by_kind() {
        let stale = serde_json::to_value(WindowAskReply::Stale { request_id: 3 }).unwrap();
        assert_eq!(stale, serde_json::json!({"kind": "stale", "requestId": 3}));
    }
}
