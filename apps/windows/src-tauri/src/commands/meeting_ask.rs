//! The meeting Ask bar (issue 246, plan Decision 7): questions about one meeting, answered
//! from its transcript. During the call that is the live transcript so far; after it, the
//! full transcript.
//!
//! It runs like Ask (`commands/chat.rs`): its own thread, the same routing, streaming,
//! blocking fallback and `ChatEvent`s, and `chat_stop` stops it. The differences: the
//! citable context is the meeting's transcript (`api::ask::prepare_meeting_ask`), the
//! history comes from the page, and nothing is saved. Exchanges live in page state, as on
//! the Mac.

use llm::{ChatRole, ChatTurn};
use schemars::JsonSchema;
use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::State;

use super::chat::{blocking_started, fail, streams_past};
use crate::chat::{self, ChatEvent, StreamRoute, TerminalAction};
use crate::error::CommandError;
use crate::l10n::tr;
use crate::settings::{ApiKeys, Instructions, PromptSurface, RoutingSettings};
use crate::state::AppState;
use crate::store_actor::StoreHandle;

/// One answered exchange from the page, sent back as history for a follow-up.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeetingAskExchange {
    /// What the model was asked: a preset's prompt, not its label.
    pub query: String,
    pub answer: String,
}

/// The conversation so far, as the model sees it. Exchanges without an answer are skipped.
pub fn history_turns(exchanges: &[MeetingAskExchange]) -> Vec<ChatTurn> {
    exchanges
        .iter()
        .filter(|e| !e.answer.trim().is_empty())
        .flat_map(|e| [ChatTurn { role: ChatRole::User, text: e.query.clone() }, ChatTurn { role: ChatRole::Assistant, text: e.answer.clone() }])
        .collect()
}

/// What the UI shows for a question that could not be answered.
pub fn meeting_error(error: api::ask::MeetingAskError) -> CommandError {
    match error {
        api::ask::MeetingAskError::NoTranscript => CommandError::invalid_input(tr("There is no transcript for this meeting to ask about.", &[])),
        api::ask::MeetingAskError::Api(e) => e.into(),
    }
}

/// Whether a failed prepare still leaves the blocking answer to try (Ask's rule).
fn meeting_streams_past(error: &api::ask::MeetingAskError) -> bool {
    matches!(error, api::ask::MeetingAskError::Api(e) if streams_past(e))
}

struct MeetingAsk {
    meeting_id: i64,
    query: String,
    history: Vec<ChatTurn>,
    settings: RoutingSettings,
    keys: ApiKeys,
    instructions: Instructions,
    /// The slow lane, for the blocking answer, which holds a store across the provider call.
    slow: StoreHandle,
}

/// Starts answering `query` about the meeting and returns at once; the answer arrives on
/// `on_event`. `query` is what the model is asked (a preset's prompt).
#[tauri::command]
pub fn meeting_ask(
    state: State<'_, AppState>,
    meeting_id: i64,
    query: String,
    history: Vec<MeetingAskExchange>,
    on_event: Channel<ChatEvent>,
) -> Result<(), CommandError> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(CommandError::invalid_input(tr("Type a question first.", &[])));
    }
    let store = state.store()?.clone();
    let slow = state.slow_store()?.clone();
    let (settings, instructions) = {
        let s = state.settings.lock().unwrap();
        (s.routing(), s.instructions(PromptSurface::Ask))
    };
    let ask = MeetingAsk {
        meeting_id,
        query,
        history: history_turns(&history),
        settings,
        keys: ApiKeys::load(&*state.secrets)?,
        instructions,
        slow,
    };
    std::thread::Builder::new()
        .name("whimbrel-meeting-ask".into())
        .spawn(move || {
            // A page that went away drops the UI's end of the channel; the run still ends
            // cleanly (finalized or discarded), so a failed send is ignored.
            let send = |event: ChatEvent| {
                let _ = on_event.send(event);
            };
            run(&store, &ask, &send);
        })
        .map_err(|e| CommandError::store(tr("Could not start the question: {error}", &[("error", &e.to_string())])))?;
    Ok(())
}

fn run(store: &StoreHandle, ask: &MeetingAsk, send: &dyn Fn(ChatEvent)) {
    let ollama_up = chat::ollama_serves(&ask.settings.ollama_model);
    let effective = chat::effective_routing(&ask.settings, &ask.query, ollama_up);
    let route = chat::local_instead(chat::stream_route(&effective, &ask.keys), ollama_up, api::local_model::is_configured());
    let (provider, model) = chat::answering(&effective);
    // Never the question or the meeting's content: only how it was routed.
    tracing::info!(provider, model = %model, streams = route.is_some(), ollama_up, "answering a meeting question");
    let Some(route) = route else {
        send(blocking_started(&effective, &[]));
        return blocking(ask, &effective, send);
    };
    let routing = effective.routing_config(&ask.keys, &ask.instructions);
    let (meeting_id, query, history) = (ask.meeting_id, ask.query.clone(), ask.history.clone());
    let prepared = store.call_blocking(move |s| api::ask::prepare_meeting_ask(s, meeting_id, &query, &history, &routing));
    match prepared {
        Ok(Ok(prepared)) => stream(store, ask, &effective, route, prepared, send),
        Ok(Err(error)) if meeting_streams_past(&error) => {
            send(blocking_started(&effective, &[]));
            send(ChatEvent::FellBack);
            blocking(ask, &effective, send);
        }
        Ok(Err(error)) => fail(send, meeting_error(error), false),
        Err(error) => fail(send, error, false),
    }
}

fn stream(
    store: &StoreHandle,
    ask: &MeetingAsk,
    effective: &RoutingSettings,
    route: StreamRoute,
    prepared: api::ask::PreparedMeetingAskData,
    send: &dyn Fn(ChatEvent),
) {
    let Some(run) = api::inference::start(&prepared.prompt, &route.model, route.provider, route.key.as_deref(), None) else {
        api::ask::discard_ask(&prepared.prepare_id);
        send(blocking_started(effective, &[]));
        return blocking(ask, effective, send);
    };
    send(ChatEvent::Started { run_id: Some(run.run_id.clone()), provider: route.provider.into(), model: route.model.clone(), context: vec![] });
    let end = chat::forward(&run.run_id, api::inference::poll, send, || std::thread::sleep(chat::POLL_INTERVAL));
    api::inference::release(&run.run_id);
    match chat::terminal_action(&end.outcome, end.produced_output) {
        TerminalAction::Finalize => {
            let (prepare_id, text, model, usage) = (prepared.prepare_id, end.text.clone(), route.model.clone(), end.usage);
            let citations = store
                .call_blocking(move |s| api::ask::finalize_ask(s, &prepare_id, &text, &model, usage).citations)
                .unwrap_or_default();
            tracing::info!(chars = end.text.len(), citations = citations.len(), "meeting question answered");
            send(ChatEvent::Completed { answer: end.text, citations, conversation_id: None, narrowed: prepared.narrowed });
        }
        TerminalAction::Restore => {
            api::ask::discard_ask(&prepared.prepare_id);
            send(ChatEvent::Cancelled);
        }
        TerminalAction::FallBack => {
            api::ask::discard_ask(&prepared.prepare_id);
            tracing::info!(provider = route.provider, "meeting question stream failed before any output; answering on the blocking path");
            send(ChatEvent::FellBack);
            blocking(ask, effective, send);
        }
        TerminalAction::Fail => {
            api::ask::discard_ask(&prepared.prepare_id);
            fail(send, CommandError::provider_failure(), true);
        }
    }
}

fn blocking(ask: &MeetingAsk, effective: &RoutingSettings, send: &dyn Fn(ChatEvent)) {
    let routing = effective.routing_config(&ask.keys, &ask.instructions);
    let (meeting_id, query, history, ollama_model) = (ask.meeting_id, ask.query.clone(), ask.history.clone(), effective.ollama_model.clone());
    match ask.slow.call_blocking(move |s| api::ask::answer_meeting(s, meeting_id, &query, &ollama_model, &history, &routing)) {
        Ok(Ok(answer)) => {
            tracing::info!(chars = answer.text.len(), citations = answer.citations.len(), "meeting question answered");
            send(ChatEvent::Completed { answer: answer.text, citations: answer.citations, conversation_id: None, narrowed: answer.narrowed });
        }
        Ok(Err(error)) => fail(send, meeting_error(error), false),
        Err(error) => fail(send, error, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::ask::MeetingAskError;

    fn exchange(query: &str, answer: &str) -> MeetingAskExchange {
        MeetingAskExchange { query: query.into(), answer: answer.into() }
    }

    #[test]
    fn history_keeps_answered_exchanges_in_order() {
        let turns = history_turns(&[exchange("q1", "a1"), exchange("q2", " "), exchange("q3", "a3")]);
        let flat: Vec<(ChatRole, &str)> = turns.iter().map(|t| (t.role, t.text.as_str())).collect();
        assert_eq!(flat, vec![(ChatRole::User, "q1"), (ChatRole::Assistant, "a1"), (ChatRole::User, "q3"), (ChatRole::Assistant, "a3")]);
    }

    #[test]
    fn a_spent_budget_falls_back_and_nothing_else_does() {
        let spent = api::ApiError::BudgetSpent(api::budget::BudgetSpent { provider: llm::Provider::Anthropic, spent_cents: 2500, budget_cents: 2500 });
        assert!(meeting_streams_past(&MeetingAskError::Api(spent)));
        assert!(!meeting_streams_past(&MeetingAskError::NoTranscript));
        assert!(!meeting_streams_past(&MeetingAskError::Api(api::ApiError::ProviderFailure)));
    }

    #[test]
    fn no_transcript_is_an_input_error_and_api_errors_keep_their_kind() {
        let none = serde_json::to_value(meeting_error(MeetingAskError::NoTranscript)).unwrap();
        assert_eq!(none["kind"], "invalid_input");
        let failed = serde_json::to_value(meeting_error(MeetingAskError::Api(api::ApiError::ProviderFailure))).unwrap();
        assert_eq!(failed["kind"], "provider_failure");
    }
}
