//! Ask (plan Task 8): the decisions behind grounded chat, kept free of Tauri and the
//! store actor so they are unit-tested. Which provider a question goes to and whether
//! it streams, what a stream's terminal event means, conversation history, the context
//! citations resolve against, and the loop that drains a run's events.
//! `commands/chat.rs` wires them to `api::ask`, `api::inference` and a Tauri `Channel`.

use std::time::Duration;

use llm::streaming::{InferenceEvent, InferenceFailureClass, TerminalOutcome};
use llm::{ChatRole, ChatTurn, ContextItemRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CommandError;
use crate::settings::{ApiKeys, Provider, RoutingSettings};
use crate::l10n::tr;

/// Items per search when gathering context, and the answer's retrieval limit (the
/// Mac's `contextLimit: 5`).
pub const CONTEXT_LIMIT: usize = 5;
/// How often a streaming run is drained. Its buffer holds 256 events and cancels the
/// run when full; at 100 ms no provider comes close.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The settings one question is answered with. The Mac's `effectiveAskRoutingProvider`
/// with its picker untouched (Windows has no picker): a question that tolerates latency
/// (`api::prompts::is_time_critical` is false: "summarize", "trend", …) goes to Ollama,
/// anything else to the preferred provider.
///
/// Windows adds one condition: Ollama must be able to answer (`ollama_up`: running,
/// with the configured model pulled; see [`ollama_serves`]).
/// Without it the substitution buys only a failed attempt, then an answer from
/// `build_chain`'s fallback model instead of the one the user chose. Seen on a PC with
/// Ollama installed but not running, where a synthesis question with Opus 5 selected
/// went to Ollama and then Haiku.
pub fn effective_routing(routing: &RoutingSettings, query: &str, ollama_up: bool) -> RoutingSettings {
    if ollama_up && !api::prompts::is_time_critical(query) {
        RoutingSettings { preferred_provider: Provider::Ollama, ..routing.clone() }
    } else {
        routing.clone()
    }
}

/// Title generation is never time-critical (the Mac routes it with a sentinel
/// non-time-critical query), so it prefers Ollama whenever Ollama is running.
pub fn title_routing(routing: &RoutingSettings, ollama_up: bool) -> RoutingSettings {
    if ollama_up {
        RoutingSettings { preferred_provider: Provider::Ollama, ..routing.clone() }
    } else {
        routing.clone()
    }
}

/// Whether local Ollama is accepting connections, at the address `build_chain` and
/// `api::inference` use. A refused connection returns at once; 300 ms caps a filtered one.
pub fn ollama_reachable() -> bool {
    let addr: std::net::SocketAddr = "127.0.0.1:11434".parse().unwrap();
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Whether local Ollama can answer with `model`: running, and the model pulled.
/// Running alone isn't enough: with the model missing (the default `gemma4:12b` on a
/// PC that has `gemma4:latest`), Ollama answers 404, and a synthesis question sent
/// there only gains a failed attempt.
pub fn ollama_serves(model: &str) -> bool {
    ollama_reachable()
        && api::hosted_models::fetch_models(api::hosted_models::HostedProvider::Ollama, "", None)
            .map(|pulled| pulled.iter().any(|name| same_ollama_model(name, model)))
            .unwrap_or(false)
}

/// Ollama names a model `name:tag`, and a bare `name` means `name:latest`.
pub fn same_ollama_model(a: &str, b: &str) -> bool {
    let full = |m: &str| if m.contains(':') { m.to_string() } else { format!("{m}:latest") };
    full(a.trim()) == full(b.trim())
}

/// The provider and model the blocking path will try first, for the UI's label. The
/// chain can still fall through to another provider.
pub fn answering(routing: &RoutingSettings) -> (&'static str, String) {
    if routing.local_only {
        ("ollama", routing.ollama_model.clone())
    } else {
        (routing.preferred_provider.as_str(), routing.preferred_model().to_string())
    }
}

/// A question that streams through `api::inference`.
#[derive(Clone, PartialEq, Eq)]
pub struct StreamRoute {
    /// `api::inference::start`'s provider name.
    pub provider: &'static str,
    pub model: String,
    /// The API key; for Whimbrel AI, the sync ID token.
    pub key: Option<String>,
    /// Whimbrel AI only: this PC's device key, sent beside the ID token.
    pub device_key: Option<String>,
}

impl std::fmt::Debug for StreamRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamRoute").field("provider", &self.provider).field("model", &self.model).field("key", &self.key.is_some()).field("device_key", &self.device_key.is_some()).finish()
    }
}

/// Whether a question streams, and how. The Mac's `streamRoute`: Ollama always streams;
/// a hosted provider streams with a key, and without one answers on the blocking path
/// (its chain falls back). Local-only mode answers from Ollama whatever is preferred, so
/// it streams Ollama.
pub fn stream_route(routing: &RoutingSettings, keys: &ApiKeys) -> Option<StreamRoute> {
    let ollama = || StreamRoute { provider: "ollama", model: routing.ollama_model.clone(), key: None, device_key: None };
    if routing.local_only {
        return Some(ollama());
    }
    let hosted = |provider: &'static str, key: &Option<String>, model: &str| {
        key.clone().filter(|k| !k.is_empty()).map(|key| StreamRoute { provider, model: model.to_string(), key: Some(key), device_key: None })
    };
    match routing.preferred_provider {
        Provider::Ollama => Some(ollama()),
        Provider::Google => hosted("google", &keys.google, &routing.google_model),
        Provider::Anthropic => hosted("anthropic", &keys.anthropic, &routing.anthropic_model),
        Provider::Openai => hosted("openai", &keys.openai, &routing.openai_model),
        // No model picked yet: the blocking path's chain leaves OpenRouter out.
        Provider::Openrouter if routing.openrouter_model.trim().is_empty() => None,
        Provider::Openrouter => hosted("openrouter", &keys.openrouter, &routing.openrouter_model),
        // Signed in to sync: the ID token rides `key` and the device key beside it. Not signed in:
        // the blocking chain, which leaves Whimbrel AI out.
        Provider::Whimbrel => keys.whimbrel.as_ref().map(|c| StreamRoute {
            provider: "whimbrel",
            model: routing.whimbrel_model.clone(),
            key: Some(c.id_token.clone()),
            device_key: Some(c.device_key.clone()),
        }),
    }
}

/// Before a question Whimbrel AI may answer (preferred, and not local-only): refreshes the sync ID
/// token when 5 minutes or less are left, then takes the cached credentials into `keys`. Runs on
/// the question's own thread: the refresh can call Google.
pub fn freshen_whimbrel(sync: Option<&crate::syncing::controller::SyncController>, routing: &RoutingSettings, keys: &mut ApiKeys) {
    if routing.local_only || routing.preferred_provider != Provider::Whimbrel {
        return;
    }
    if let Some(sync) = sync {
        sync.whimbrel_before_ask();
    }
    keys.whimbrel = crate::whimbrel_ai::cached();
}

/// Starts `route`'s stream: Whimbrel AI through `api::inference::start_whimbrel`, every other
/// provider through `api::inference::start`. `None` when it couldn't start.
pub fn start_stream(prompt: &str, route: &StreamRoute) -> Option<api::inference::StartedRun> {
    match (route.provider, route.key.as_deref(), route.device_key.as_deref()) {
        ("whimbrel", Some(id_token), Some(device_key)) => api::inference::start_whimbrel(prompt, &route.model, id_token, device_key),
        ("whimbrel", ..) => None,
        _ => api::inference::start(prompt, &route.model, route.provider, route.key.as_deref(), None),
    }
}

/// With no Ollama serving the model, a question bound for Ollama streams from the small local
/// model instead, once it's installed (the Mac's `replacesOllama`, W-1.3). `api::inference`
/// starts the helper and, if it can't, starts nothing, so the question takes the blocking path.
pub fn local_instead(route: Option<StreamRoute>, ollama_up: bool, local_installed: bool) -> Option<StreamRoute> {
    match route {
        Some(r) if r.provider == "ollama" && !ollama_up && local_installed => {
            Some(StreamRoute { provider: "local", model: api::local_model::MODEL_ID.to_string(), key: None, device_key: None })
        }
        other => other,
    }
}

/// What to do once a stream ends (the Mac's `handleStreamingAskPhaseChange`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalAction {
    /// Extract citations, persist, show the answer.
    Finalize,
    /// Stopped by the user: no bubble, and the question goes back into the composer.
    Restore,
    /// Failed before any output: answer on the blocking path instead.
    FallBack,
    /// Failed after output had streamed: never silently retry; show an error bubble.
    Fail,
    /// Whimbrel AI answered 402: the credit is used up. Say so, and don't retry through the chain,
    /// which would only meet the same 402 (the Mac's `pinned_whimbrel_stops_on_402`).
    CreditExhausted,
}

pub fn terminal_action(outcome: &TerminalOutcome, produced_output: bool) -> TerminalAction {
    match outcome {
        TerminalOutcome::Completed => TerminalAction::Finalize,
        TerminalOutcome::Cancelled => TerminalAction::Restore,
        TerminalOutcome::Failed { .. } if !produced_output => TerminalAction::FallBack,
        TerminalOutcome::Failed { .. } => TerminalAction::Fail,
    }
}

/// `terminal_action` for a stream from `provider` (`StreamRoute::provider`): a Whimbrel AI 402
/// (core's failure text starts `HTTP 402`) stops with the credit message.
pub fn terminal_action_for(provider: &str, outcome: &TerminalOutcome, produced_output: bool) -> TerminalAction {
    match outcome {
        TerminalOutcome::Failed { message, .. } if provider == "whimbrel" && message.starts_with("HTTP 402") => TerminalAction::CreditExhausted,
        _ => terminal_action(outcome, produced_output),
    }
}

/// The conversation so far, as the model sees it. Only answered exchanges are ever
/// persisted, so every question has its answer.
pub fn history_from(exchanges: &[memory::ConversationExchange]) -> Vec<ChatTurn> {
    exchanges
        .iter()
        .flat_map(|e| {
            [ChatTurn { role: ChatRole::User, text: e.query.clone() }, ChatTurn { role: ChatRole::Assistant, text: e.answer.clone() }]
        })
        .collect()
}

/// A Library detail view a citation can open (the UI's `Selection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibraryTarget {
    Capture { id: i64 },
    Meeting { id: i64 },
    Episode { id: i64 },
}

/// One item the answer may cite. The UI matches a citation to it by `item`. A saved
/// exchange stores the entries it cited in this shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextEntry {
    /// `{kind, id}`, exactly as a citation (`llm::ContextItemRef`) serializes.
    pub item: ContextItemRef,
    pub label: String,
    pub ts_unix_ms: Option<i64>,
    /// Where clicking the citation goes; `None` for routine runs and threads, which have
    /// no Library view yet.
    pub open: Option<LibraryTarget>,
}

fn capture_entry(c: &memory::CaptureItem) -> ContextEntry {
    let label = if c.window_title.trim().is_empty() { c.app_bundle_id.clone() } else { format!("{} · {}", c.window_title, c.app_bundle_id) };
    ContextEntry { item: ContextItemRef::Capture { id: c.id }, label, ts_unix_ms: Some(c.ts_unix_ms), open: Some(LibraryTarget::Capture { id: c.id }) }
}

fn transcript_entry(t: &memory::MeetingTranscriptSegment) -> ContextEntry {
    ContextEntry {
        item: ContextItemRef::Transcript { id: t.id },
        label: tr("Meeting \"{title}\", {speaker}", &[("title", &t.meeting_title), ("speaker", &t.speaker_label)]),
        ts_unix_ms: None,
        open: Some(LibraryTarget::Meeting { id: t.meeting_id }),
    }
}

fn snapshot_entry(h: &memory::MeetingSnapshotHit) -> ContextEntry {
    ContextEntry {
        item: ContextItemRef::Snapshot { id: h.id },
        label: tr("Slide in meeting \"{title}\"", &[("title", &h.meeting_title)]),
        ts_unix_ms: None,
        open: Some(LibraryTarget::Meeting { id: h.meeting_id }),
    }
}

fn thread_entry(t: memory::Thread) -> ContextEntry {
    ContextEntry {
        item: ContextItemRef::Thread { id: t.id },
        label: t.subject.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| tr("(no subject)", &[])),
        ts_unix_ms: Some(t.last_activity_ts_unix_ms),
        open: None,
    }
}

/// Appends the entries whose item isn't in `out` yet (nor earlier in `more`), in order.
fn add_unseen(out: &mut Vec<ContextEntry>, more: impl IntoIterator<Item = ContextEntry>) {
    let mut seen: std::collections::HashSet<ContextItemRef> = out.iter().map(|e| e.item).collect();
    out.extend(more.into_iter().filter(|e| seen.insert(e.item)));
}

/// The candidates a citation can resolve against: the Mac's `fetchContextItems`
/// (`TimelineView+Ask.swift`). Keyword captures, transcripts and slides (5 each), then
/// semantic ones not already listed, 5 routine runs, 10 episodes, the 5 email threads
/// whose messages match (PR 628), and the 10 threads waiting on the user. A display
/// resolver only; retrieval for the answer itself is `api::ask`'s own. Errors read as
/// empty: this never fails a question.
pub fn fetch_context(store: &memory::Store, query: &str) -> Vec<ContextEntry> {
    let captures = store.search_keyword(query, CONTEXT_LIMIT).unwrap_or_default();
    let transcripts = store.search_transcript_keyword(query, CONTEXT_LIMIT).unwrap_or_default();
    let slides = store.search_snapshot_keyword_with_meeting_ts(query, CONTEXT_LIMIT).unwrap_or_default();
    let mut out: Vec<ContextEntry> = captures
        .iter()
        .map(capture_entry)
        .chain(transcripts.iter().map(transcript_entry))
        .chain(slides.iter().map(|(h, _)| snapshot_entry(h)))
        .collect();
    // Ask's own retrieval always fuses keyword and semantic hits, so a citation can point at
    // either; keyword search ORs its words (PR 622) and is rarely empty now. The semantic hits
    // follow the keyword ones, minus any already listed (as on the Mac).
    let captures = store.search_semantic(query, CONTEXT_LIMIT).unwrap_or_default();
    let transcripts = store.search_transcript_semantic(query, CONTEXT_LIMIT).unwrap_or_default();
    let slides = store.search_snapshot_semantic_with_meeting_ts(query, CONTEXT_LIMIT).unwrap_or_default();
    let semantic = captures.iter().map(capture_entry).chain(transcripts.iter().map(transcript_entry)).chain(slides.iter().map(|(h, _)| snapshot_entry(h)));
    add_unseen(&mut out, semantic);
    out.extend(store.recent_routine_runs_across_all(5).unwrap_or_default().into_iter().map(|r| ContextEntry {
        item: ContextItemRef::RoutineRun { id: r.id },
        label: r.routine_name,
        ts_unix_ms: Some(r.ran_at_unix_ms),
        open: None,
    }));
    out.extend(store.recent_episodes(10).unwrap_or_default().into_iter().map(|e| ContextEntry {
        item: ContextItemRef::Episode { id: e.id },
        label: e.window_title.filter(|t| !t.trim().is_empty()).unwrap_or(e.app_bundle_id),
        ts_unix_ms: Some(e.start_ts_unix_ms),
        open: Some(LibraryTarget::Episode { id: e.id }),
    }));
    // Ask finds email through its thread (PR 628); those come first, then the waiting
    // threads not already listed.
    let email = store.search_email_thread_keyword(query, CONTEXT_LIMIT).unwrap_or_default();
    out.extend(email.into_iter().map(|hit| thread_entry(hit.thread)));
    add_unseen(&mut out, store.threads_needing_attention(10).unwrap_or_default().into_iter().map(thread_entry));
    out
}

/// What the UI hears about one question, over a Tauri `Channel`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatEvent {
    /// The question is being answered. `run_id` is set while it streams (Stop cancels
    /// it); `None` on the blocking path, where the UI shows a spinner.
    #[serde(rename_all = "camelCase")]
    Started { run_id: Option<String>, provider: String, model: String, context: Vec<ContextEntry> },
    Delta { text: String },
    Progress { label: String },
    /// The stream failed before any output; the blocking path is answering instead.
    FellBack,
    #[serde(rename_all = "camelCase")]
    Completed { answer: String, citations: Vec<ContextItemRef>, conversation_id: Option<i64>, narrowed: bool },
    /// Stopped: the question goes back into the composer.
    Cancelled,
    /// `partial`: some of the answer had streamed before it failed.
    Failed { error: CommandError, partial: bool },
    /// A new conversation got its generated title.
    #[serde(rename_all = "camelCase")]
    Titled { conversation_id: i64, title: String },
}

/// How a stream ended.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamEnd {
    pub outcome: TerminalOutcome,
    /// The full answer: the terminal event's text, or what streamed if it has none.
    pub text: String,
    pub usage: llm::TokenUsage,
    /// Whether any text streamed before the end (the Mac's `streamedText.isEmpty`).
    pub produced_output: bool,
}

/// Drains `run_id` until its terminal event, forwarding text and progress to `send`,
/// and calling `wait` between polls. `send` never stops the loop: if the window closed,
/// the run must still finish (and be persisted), and an undrained buffer would cancel it.
/// `poll` returning `None` (the run vanished) ends as a failure.
pub fn forward(
    run_id: &str,
    mut poll: impl FnMut(&str) -> Option<Vec<InferenceEvent>>,
    send: &dyn Fn(ChatEvent),
    mut wait: impl FnMut(),
) -> StreamEnd {
    let mut streamed = String::new();
    loop {
        let Some(events) = poll(run_id) else {
            return StreamEnd {
                outcome: TerminalOutcome::Failed { class: InferenceFailureClass::Provider, message: "the run ended unexpectedly".into() },
                produced_output: !streamed.is_empty(),
                text: streamed,
                usage: llm::TokenUsage::default(),
            };
        };
        for event in events {
            match event {
                InferenceEvent::TextDelta { text, .. } if !text.is_empty() => {
                    streamed.push_str(&text);
                    send(ChatEvent::Delta { text });
                }
                InferenceEvent::Progress { label, .. } => send(ChatEvent::Progress { label }),
                InferenceEvent::Terminal { outcome, final_text, usage, .. } => {
                    let produced_output = !streamed.is_empty();
                    let text = if final_text.is_empty() { streamed } else { final_text };
                    return StreamEnd { outcome, text, usage, produced_output };
                }
                _ => {}
            }
        }
        wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::streaming::InferenceRunState;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    fn routing(provider: Provider) -> RoutingSettings {
        RoutingSettings { preferred_provider: provider, ..RoutingSettings::default() }
    }

    fn keys(google: Option<&str>) -> ApiKeys {
        ApiKeys { google: google.map(String::from), anthropic: Some("a".into()), ..ApiKeys::default() }
    }

    #[test]
    fn ollama_and_keyed_hosted_providers_stream_the_rest_block() {
        let local = local_instead(stream_route(&routing(Provider::Ollama), &keys(None)), false, true).unwrap();
        assert_eq!((local.provider, local.model.as_str(), local.key), ("local", "local", None), "no Ollama: the local model streams");
        assert_eq!(local_instead(stream_route(&routing(Provider::Ollama), &keys(None)), true, true).unwrap().provider, "ollama", "Ollama up wins");
        assert_eq!(local_instead(stream_route(&routing(Provider::Ollama), &keys(None)), false, false).unwrap().provider, "ollama", "not installed");
        assert_eq!(local_instead(stream_route(&routing(Provider::Google), &keys(Some("g"))), false, true).unwrap().provider, "google", "a key wins");
        let ollama = stream_route(&routing(Provider::Ollama), &keys(None)).unwrap();
        assert_eq!((ollama.provider, ollama.model.as_str(), ollama.key.is_none()), ("ollama", "gemma4:12b", true));
        let google = stream_route(&routing(Provider::Google), &keys(Some("g"))).unwrap();
        assert_eq!((google.provider, google.model.as_str(), google.key.as_deref()), ("google", "gemini-2.5-flash", Some("g")));
        assert_eq!(stream_route(&routing(Provider::Google), &keys(None)), None, "no key: the blocking chain falls back");
        assert_eq!(stream_route(&routing(Provider::Google), &keys(Some(""))), None);
        let anthropic = stream_route(&routing(Provider::Anthropic), &keys(None)).unwrap();
        assert_eq!((anthropic.provider, anthropic.key.as_deref()), ("anthropic", Some("a")));
        assert_eq!(stream_route(&routing(Provider::Openai), &keys(Some("g"))), None, "no OpenAI key: blocking");
    }

    #[test]
    fn whimbrel_ai_streams_with_the_sync_credentials_or_not_at_all() {
        let creds = crate::whimbrel_ai::Credentials { id_token: "jwt".into(), device_key: "pk".into() };
        let signed_in = ApiKeys { whimbrel: Some(creds), ..keys(None) };
        let route = stream_route(&routing(Provider::Whimbrel), &signed_in).unwrap();
        assert_eq!(
            (route.provider, route.model.as_str(), route.key.as_deref(), route.device_key.as_deref()),
            ("whimbrel", "claude-haiku-4-5", Some("jwt"), Some("pk"))
        );
        assert!(!format!("{route:?}").contains("jwt"), "the token is never printed");
        assert_eq!(stream_route(&routing(Provider::Whimbrel), &keys(None)), None, "not signed in to sync: the blocking chain");
        let local = RoutingSettings { local_only: true, ..routing(Provider::Whimbrel) };
        assert_eq!(stream_route(&local, &signed_in).unwrap().provider, "ollama");
        // A route missing the device key never starts a stream (it would go out unauthenticated).
        let half = StreamRoute { device_key: None, ..route };
        assert!(start_stream("hi", &half).is_none());
    }

    #[test]
    fn local_only_streams_ollama_and_never_a_hosted_provider() {
        for provider in [Provider::Google, Provider::Anthropic, Provider::Openai] {
            let r = RoutingSettings { local_only: true, ..routing(provider) };
            assert_eq!(stream_route(&r, &keys(Some("g"))).unwrap().provider, "ollama");
            assert_eq!(answering(&r).0, "ollama");
        }
    }

    #[test]
    fn a_synthesis_question_goes_to_ollama_only_while_ollama_is_running() {
        let preferred = RoutingSettings { anthropic_model: "claude-opus-5".into(), ..routing(Provider::Anthropic) };
        let synthesis = "Summarize what happened today";
        assert_eq!(effective_routing(&preferred, synthesis, true).preferred_provider, Provider::Ollama);
        assert_eq!(effective_routing(&preferred, "what time is my next meeting?", true).preferred_provider, Provider::Anthropic);

        // Ollama down: the user's own choice answers, model and all.
        let down = effective_routing(&preferred, synthesis, false);
        assert_eq!((down.preferred_provider, down.preferred_model()), (Provider::Anthropic, "claude-opus-5"));
        assert_eq!(answering(&down), ("anthropic", "claude-opus-5".to_string()));
    }

    #[test]
    fn ollama_model_names_match_with_or_without_the_latest_tag() {
        assert!(same_ollama_model("gemma4:latest", "gemma4"));
        assert!(same_ollama_model("gemma4:12b", " gemma4:12b "));
        assert!(!same_ollama_model("gemma4:latest", "gemma4:12b"), "a different tag is a different model");
    }

    #[test]
    fn titles_prefer_a_running_ollama_and_otherwise_the_chosen_provider() {
        let preferred = routing(Provider::Anthropic);
        let title = title_routing(&RoutingSettings { local_only: true, ..preferred.clone() }, true);
        assert_eq!((title.preferred_provider, title.local_only), (Provider::Ollama, true));
        assert_eq!(title_routing(&preferred, false).preferred_provider, Provider::Anthropic);
    }

    #[test]
    fn terminal_events_map_to_the_macs_actions() {
        let failed = TerminalOutcome::Failed { class: InferenceFailureClass::Network, message: "reset".into() };
        assert_eq!(terminal_action(&TerminalOutcome::Completed, true), TerminalAction::Finalize);
        assert_eq!(terminal_action(&TerminalOutcome::Completed, false), TerminalAction::Finalize);
        assert_eq!(terminal_action(&TerminalOutcome::Cancelled, true), TerminalAction::Restore);
        assert_eq!(terminal_action(&failed, false), TerminalAction::FallBack);
        assert_eq!(terminal_action(&failed, true), TerminalAction::Fail);
        // Whimbrel AI's 402 stops with the credit line; anyone else's 402, or Whimbrel's 500, falls back.
        let credit = TerminalOutcome::Failed { class: InferenceFailureClass::Provider, message: "HTTP 402: {\"error\":\"insufficient_credit\"}".into() };
        assert_eq!(terminal_action_for("whimbrel", &credit, false), TerminalAction::CreditExhausted);
        assert_eq!(terminal_action_for("anthropic", &credit, false), TerminalAction::FallBack);
        let server = TerminalOutcome::Failed { class: InferenceFailureClass::Provider, message: "HTTP 500: oops".into() };
        assert_eq!(terminal_action_for("whimbrel", &server, false), TerminalAction::FallBack);
        assert_eq!(terminal_action_for("whimbrel", &TerminalOutcome::Completed, true), TerminalAction::Finalize);
    }

    #[test]
    fn history_alternates_question_and_answer() {
        let ex = |q: &str, a: &str| memory::ConversationExchange { id: 0, query: q.into(), answer: a.into(), citations_json: "[]".into(), created_at_unix_ms: 0, model: None };
        let turns = history_from(&[ex("q1", "a1"), ex("q2", "a2")]);
        let shape: Vec<(bool, &str)> = turns.iter().map(|t| (matches!(t.role, ChatRole::User), t.text.as_str())).collect();
        assert_eq!(shape, [(true, "q1"), (false, "a1"), (true, "q2"), (false, "a2")]);
    }

    fn delta(text: &str) -> InferenceEvent {
        InferenceEvent::TextDelta { run_id: "r".into(), sequence: 0, text: text.into() }
    }

    fn terminal(outcome: TerminalOutcome, final_text: &str) -> InferenceEvent {
        InferenceEvent::Terminal {
            run_id: "r".into(),
            outcome,
            at_unix_ms: 0,
            final_text: final_text.into(),
            usage: llm::TokenUsage { input_tokens: Some(12), output_tokens: Some(3) },
        }
    }

    /// Runs `forward` over scripted polls, returning what it sent and how it ended.
    fn run_fake(script: Vec<Option<Vec<InferenceEvent>>>) -> (Vec<ChatEvent>, StreamEnd, usize) {
        let mut polls: VecDeque<_> = script.into();
        let sent = RefCell::new(Vec::new());
        let mut waits = 0;
        let end = forward("r", |_| polls.pop_front().expect("polled past the terminal event"), &|e| sent.borrow_mut().push(e), || waits += 1);
        (sent.into_inner(), end, waits)
    }

    #[test]
    fn forwarder_sends_deltas_and_progress_and_stops_at_the_terminal_event() {
        let state = InferenceEvent::State { run_id: "r".into(), state: InferenceRunState::Streaming, at_unix_ms: 0, provider: None, model: None, attempt: 0 };
        let (sent, end, waits) = run_fake(vec![
            Some(vec![state, delta("Hel")]),
            Some(vec![]),
            Some(vec![InferenceEvent::Progress { run_id: "r".into(), label: "Warming up".into() }, delta("lo"), delta("")]),
            Some(vec![terminal(TerminalOutcome::Completed, "Hello [1]")]),
        ]);
        assert_eq!(
            sent,
            [ChatEvent::Delta { text: "Hel".into() }, ChatEvent::Progress { label: "Warming up".into() }, ChatEvent::Delta { text: "lo".into() }]
        );
        assert_eq!((end.outcome, end.text.as_str(), end.produced_output), (TerminalOutcome::Completed, "Hello [1]", true));
        assert_eq!(end.usage.input_tokens, Some(12), "usage feeds finalize_ask's spend ledger");
        assert_eq!(waits, 3, "waits between polls, not after the terminal event");
    }

    #[test]
    fn forwarder_reports_whether_anything_streamed_before_a_failure() {
        let failed = || TerminalOutcome::Failed { class: InferenceFailureClass::Network, message: "refused".into() };
        let (_, end, _) = run_fake(vec![Some(vec![terminal(failed(), "")])]);
        assert_eq!((end.produced_output, terminal_action(&end.outcome, end.produced_output)), (false, TerminalAction::FallBack));
        let (_, end, _) = run_fake(vec![Some(vec![delta("partial")]), Some(vec![terminal(failed(), "")])]);
        assert_eq!((end.text.as_str(), terminal_action(&end.outcome, end.produced_output)), ("partial", TerminalAction::Fail));
    }

    #[test]
    fn a_vanished_run_ends_as_a_failure() {
        let (_, end, _) = run_fake(vec![Some(vec![delta("so far")]), None]);
        assert!(matches!(end.outcome, TerminalOutcome::Failed { .. }));
        assert_eq!((end.text.as_str(), end.produced_output), ("so far", true));
    }

    #[test]
    fn context_resolves_a_capture_to_its_library_view() {
        let store = memory::Store::open_in_memory().unwrap();
        let record = ingest::CaptureRecord {
            ts_unix_ms: 1_757_520_000_000,
            bundle_id: "notepad.exe".into(),
            window_title: "Plan".into(),
            url: None,
            text: "the falcon migration plan".into(),
            source: "uia".into(),
        };
        ingest::ingest_record(&store, "dev", &record);
        let context = fetch_context(&store, "falcon");
        let capture = context.iter().find(|e| matches!(e.item, ContextItemRef::Capture { .. })).expect("the keyword hit");
        let ContextItemRef::Capture { id } = capture.item else { unreachable!() };
        assert_eq!(capture.open, Some(LibraryTarget::Capture { id }));
        assert_eq!(capture.label, "Plan · notepad.exe");
        // No keyword hit and no embedding model: no captures, and no error.
        assert!(fetch_context(&store, "zebra").iter().all(|e| !matches!(e.item, ContextItemRef::Capture { .. })));
    }

    #[test]
    fn semantic_hits_follow_keyword_hits_without_repeats() {
        let entry = |item| ContextEntry { item, label: String::new(), ts_unix_ms: None, open: None };
        let mut out = vec![entry(ContextItemRef::Capture { id: 1 }), entry(ContextItemRef::Transcript { id: 2 })];
        add_unseen(
            &mut out,
            [ContextItemRef::Capture { id: 1 }, ContextItemRef::Capture { id: 3 }, ContextItemRef::Capture { id: 3 }, ContextItemRef::Transcript { id: 2 }].map(entry),
        );
        let items: Vec<ContextItemRef> = out.iter().map(|e| e.item).collect();
        assert_eq!(items, [ContextItemRef::Capture { id: 1 }, ContextItemRef::Transcript { id: 2 }, ContextItemRef::Capture { id: 3 }]);
    }

    #[test]
    fn email_threads_that_match_come_first_without_repeats() {
        let store = memory::Store::open_in_memory().unwrap();
        let email = |id: &str, thread: &str, subject: &str, body: &str, ts: i64| memory::NewEmailItem {
            device_id: "dev".into(),
            external_id: id.into(),
            ts_unix_ms: ts,
            from: "priya@example.com".into(),
            to: "me@example.com".into(),
            subject: subject.into(),
            body_text: body.into(),
            gmail_thread_id: thread.into(),
            account_email: "me@example.com".into(),
            gmail_labels: vec!["INBOX".into()],
            list_unsubscribe: false,
        };
        store
            .upsert_email_items(&[
                email("m-1", "t-1", "Budget", "The Falcon budget is 40k for Q3", 1_000),
                email("m-2", "t-2", "Lunch", "Lunch on Friday?", 2_000),
            ])
            .unwrap();
        while store.run_thread_state_sync_job(5_000).unwrap() {}
        let waiting = store.threads_needing_attention(10).unwrap();
        let budget = waiting.iter().find(|t| t.external_thread_id == "t-1").expect("the budget thread waits on me").id;

        let context = fetch_context(&store, "What is the Falcon budget?");
        let threads: Vec<&ContextEntry> = context.iter().filter(|e| matches!(e.item, ContextItemRef::Thread { .. })).collect();
        assert_eq!(threads[0].item, ContextItemRef::Thread { id: budget }, "the matching thread leads");
        assert_eq!(threads[0].label, "Budget");
        assert_eq!(threads.iter().filter(|e| e.item == ContextItemRef::Thread { id: budget }).count(), 1);
        assert_eq!(threads.len(), 2, "the other waiting thread still follows");
    }

    #[test]
    fn context_resolves_a_slide_to_its_meeting() {
        let store = memory::Store::open_in_memory().unwrap();
        let meeting = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Acme sync".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        store
            .insert_meeting_snapshot(&memory::NewMeetingSnapshot {
                meeting_id: meeting,
                captured_unix_ms: 1_500,
                file_path: r"C:\snaps\10.00.00.png".into(),
                width: 1280,
                height: 720,
                ocr_text: "Falcon launch timeline".into(),
            })
            .unwrap();
        let context = fetch_context(&store, "timeline");
        let slide = context.iter().find(|e| matches!(e.item, ContextItemRef::Snapshot { .. })).expect("the slide hit");
        assert_eq!(slide.open, Some(LibraryTarget::Meeting { id: meeting }));
        assert_eq!(slide.label, "Slide in meeting \"Acme sync\"");
    }

    #[test]
    fn events_serialize_with_a_kind_tag_and_camel_case_fields() {
        let started = ChatEvent::Started { run_id: Some("r".into()), provider: "ollama".into(), model: "m".into(), context: vec![] };
        let v = serde_json::to_value(started).unwrap();
        assert_eq!((v["kind"].as_str(), v["runId"].as_str()), (Some("started"), Some("r")));
        assert_eq!(serde_json::to_value(ChatEvent::FellBack).unwrap(), serde_json::json!({"kind": "fell_back"}));
        let done = ChatEvent::Completed { answer: "a".into(), citations: vec![ContextItemRef::Episode { id: 7 }], conversation_id: Some(3), narrowed: false };
        let v = serde_json::to_value(done).unwrap();
        assert_eq!(v["citations"][0], serde_json::json!({"kind": "episode", "id": 7}));
        assert_eq!(v["conversationId"], 3);
        let failed = serde_json::to_value(ChatEvent::Failed { error: CommandError::provider_failure(), partial: true }).unwrap();
        assert_eq!(failed["error"]["kind"], "provider_failure");
    }
}
