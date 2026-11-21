//! The hosted-model gate as the store says it is now (X6 client controls, spec 7.3-7.4).
//!
//! [`current_gate`] reads the excluded clients' names, aliases and known contacts' addresses
//! (`llm::exclusion::hosted_terms`, the term set retrieval matches too) and builds the gate every
//! hosted request passes (`llm::guard`): a no-op when nothing is excluded, a refuse-all when
//! the setting can't be read (fail closed), otherwise the exclusion set.
//!
//! The streaming entry points (`crate::inference`, `whimbrel_inference_start`) take no store
//! by design (ADR 0001), so they use [`published_gate`]: the gate the newest
//! [`refresh_published_gate`] built in this process, refusing every hosted prompt before the
//! first. It is the second line, not the first: every prepare that precedes a stream
//! (`ask::prepare_*`, `window_ask::prepare_window_ask`) checks the final prompt itself with
//! [`check_stream`], so no stream depends on this global. Whatever changes the exclusion
//! setting (the settings write command, the sync merge) must call [`refresh_published_gate`].
//!
//! Task 8 (spec 7.1-7.4) adds what the gate alone cannot see, material filed under an excluded
//! client that never names it:
//! - [`context_filter`]: retrieval for a hosted prompt leaves excluded material out
//!   (`llm::ContextFilter`), and says how many items it left out ([`ExclusionOutcome`]).
//! - [`request_carries_client`]: a request that names the client or holds a known contact's
//!   address (as written, encoded or spoken), is asked from its page or about its meeting, or whose
//!   window text does either goes local-only, or is refused. In material, an address keeps out only
//!   the lines holding it; a name keeps out the whole meeting.
//! - [`classify_before_hosted`]: new material is matched locally (names, aliases, attendee
//!   emails, no model) before any hosted call about it.
//! - [`hosted_summaries_to_redo`]: the local re-summary offer when exclusion is turned on.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;

use llm::guard::{ExclusionSet, HostedGate, NoExclusions, RefuseAllHosted};

use crate::ApiResult;

/// The gate for hosted requests, read from `store` now.
pub fn current_gate(store: &memory::Store) -> Arc<dyn HostedGate> {
    gate_for(store.exclusion_unreadable(), llm::exclusion::hosted_terms(store).ok_or(rusqlite::Error::InvalidQuery))
}

/// Reads the gate from `store` and publishes it for the store-less streaming entry points.
/// The sequence number is taken before the read, so a slower, older read can never replace
/// a newer one.
pub fn refresh_published_gate(store: &memory::Store) -> Arc<dyn HostedGate> {
    let seq = NEXT_SEQ.fetch_add(1, Ordering::SeqCst);
    let gate = current_gate(store);
    PUBLISHED.offer(seq, gate.clone());
    #[cfg(test)]
    LAST_PUBLISHED_HERE.with(|l| *l.borrow_mut() = Some(gate.clone()));
    gate
}

#[cfg(test)]
thread_local! {
    /// The gate this thread last published: tests read their own publish without racing other tests' threads.
    pub(crate) static LAST_PUBLISHED_HERE: std::cell::RefCell<Option<Arc<dyn HostedGate>>> = const { std::cell::RefCell::new(None) };
}

/// The gate the newest [`refresh_published_gate`] built in this process; refuses every
/// hosted prompt before the first.
pub fn published_gate() -> Arc<dyn HostedGate> {
    PUBLISHED.get()
}

/// The prompt a shell is about to stream to `provider`, checked now against the store: a
/// hosted provider that must not see it is refused here, recorded for
/// [`crate::last_provider_failure`], never naming the client. Local providers always pass.
pub fn check_stream(store: &memory::Store, provider: llm::Provider, prompt: &str) -> ApiResult<()> {
    if provider.is_local() {
        return Ok(());
    }
    current_gate(store).check(prompt).map_err(|r| crate::envelope::chain_failed(vec![r.failure_kind()]))
}

/// The sentence turning exclusion on shows (spec 7.4).
pub const ALREADY_LEFT_NOTE: &str = "Material that already left cannot be recalled.";

/// Where a request was made from: the page or window can carry the client as much as the words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestOrigin {
    /// The Ask bar or Ask tab: only the words (and the conversation so far) say who it is about.
    AskBar,
    /// Asked from topic `key`'s page.
    TopicPage { key: String },
    /// The hotkey panel's "Ask about this window": the window's title and text.
    HotkeyWindow { window_text: String },
    /// Asked about one meeting (meeting Ask, its summary, its brief).
    Meeting { meeting_id: i64 },
    /// Where it was asked from could not be read (fix round 2, N8): treated as carrying a client.
    Unreadable,
    Other,
}

/// What exclusion did to one answer. The UI shows it locally ("left out material from excluded
/// clients", or the refusal reason); it never goes into a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExclusionOutcome {
    /// Nothing was excluded from this answer.
    Allowed,
    /// A hosted model answered without `left_out` retrieved items (distinct items, not clients:
    /// an item that only names a client cannot always be told apart by client).
    FilteredHosted { left_out: usize },
    /// The request carries an excluded client and was answered on this computer.
    LocalOnly,
    /// The request carries an excluded client and no local model could answer; nothing was sent.
    Refused,
}

impl ExclusionOutcome {
    /// `FilteredHosted` when a hosted link answered and `filter` left something out, else `Allowed`.
    pub fn after_hosted(hosted: bool, filter: &llm::ContextFilter) -> Self {
        match filter.left_out() {
            n if hosted && n > 0 => ExclusionOutcome::FilteredHosted { left_out: n },
            _ => ExclusionOutcome::Allowed,
        }
    }
}

/// The retrieval filter for a prompt that may reach a hosted model, read from `store` now.
pub fn context_filter(store: &memory::Store) -> llm::ContextFilter {
    llm::ContextFilter::load(store)
}

/// Whether any link of `chain` leaves this computer ([`llm::is_hosted`]: Ollama cloud models too).
pub fn chain_is_hosted(chain: &[llm::ChainLink]) -> bool {
    chain.iter().any(|l| llm::is_hosted(l.provider, &l.model))
}

/// The retrieval filter for a prompt that may go down `chain`: open when every link is on this
/// computer, [`context_filter`] otherwise.
pub fn filter_for_chain(store: &memory::Store, chain: &[llm::ChainLink]) -> llm::ContextFilter {
    if chain_is_hosted(chain) {
        context_filter(store)
    } else {
        llm::ContextFilter::none()
    }
}

/// `topic_source` ids of every source a hosted prompt must not carry, by the one shared
/// predicate (fix round 2, N1): filed under an excluded client (an item in two topics is
/// excluded if either is), excluded through its meeting, thread, slide, episode, address or
/// event link (`llm::ContextFilter::excludes_source`, the function retrieval uses too), or its
/// own indexed text naming a client; every source when the setting cannot be read.
pub fn excluded_source_ids(store: &memory::Store) -> rusqlite::Result<HashSet<i64>> {
    let filter = context_filter(store);
    let sources = store.topic_source_refs()?;
    if filter.is_closed() {
        return Ok(sources.into_iter().map(|(id, ..)| id).collect());
    }
    let Some(m) = filter.material() else { return Ok(HashSet::new()) };
    Ok(sources
        .into_iter()
        .filter(|(id, kind, r, text)| {
            m.source_ids.contains(id) || r.is_some_and(|r| filter.excludes_source(kind, r)) || filter.names_client(text)
        })
        .map(|(id, ..)| id)
        .collect())
}

/// Whether a request carries an excluded client: `query` (with the conversation so far) names
/// one or holds a known client contact's address, it is asked from an excluded client's page or
/// about an excluded client's meeting, or the window text does either. True when the setting cannot be read. Such a request is answered on
/// this computer or refused; it never falls back to a hosted model (spec 7.2).
pub fn request_carries_client(store: &memory::Store, query: &str, origin: RequestOrigin) -> bool {
    let filter = context_filter(store);
    carries_with(store, &filter, query, &origin)
}

pub(crate) fn carries_with(store: &memory::Store, filter: &llm::ContextFilter, query: &str, origin: &RequestOrigin) -> bool {
    if filter.is_closed() || matches!(origin, RequestOrigin::Unreadable) {
        return true;
    }
    let Some(material) = filter.material() else { return false };
    // By name or alias, or by a known client contact's address written anywhere in the request
    // ("what did wile@coyote.example's team say?"): the filter's one term set (`llm::exclusion::hosted_terms`).
    if filter.names_client(query) {
        return true;
    }
    match origin {
        RequestOrigin::AskBar | RequestOrigin::Other | RequestOrigin::Unreadable => false,
        // Topic pages are seed pages; a merged-away key is in the excluded chain too.
        RequestOrigin::TopicPage { key } => material.topic_keys.contains(key),
        // By name, or by a known client contact's address (a mail thread from them that never names the
        // client), as for meetings and threads (X6 final review, Major 6).
        RequestOrigin::HotkeyWindow { window_text } => {
            filter.names_client(window_text) || filter.addresses_client(&addresses_in(window_text))
        }
        RequestOrigin::Meeting { meeting_id } => match store.get_meeting(*meeting_id) {
            Ok(Some(m)) => filter.excludes_meeting(m.id, &m.title, &m.attendees),
            Ok(None) => material.meetings.contains(meeting_id),
            Err(_) => true,
        },
    }
}

/// The email addresses written anywhere in free text (`Name <a@b.c>`, `mailto:a@b.c`, a bare `a@b.c.`),
/// lowercased. A token counts when it has one `@` with a local part before it and a dotted domain after it.
fn addresses_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split(|c: char| c.is_whitespace() || "<>()[]{},;:\"'`|".contains(c))
        .filter_map(|t| {
            let t = t.trim_matches(|c: char| !c.is_alphanumeric());
            let (local, domain) = t.split_once('@')?;
            let ok = !local.is_empty()
                && !domain.contains('@')
                && domain.split('.').count() >= 2
                && domain.split('.').all(|p| !p.is_empty());
            ok.then(|| t.to_lowercase())
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Classify first (spec 7.1): whether new material about to go to a hosted model belongs to an
/// excluded client, by plain matching (no model) of each excluded client's name and aliases
/// against `material_text` and of its known attendee addresses against `attendee_emails`. True
/// when the setting cannot be read. A true answer keeps the item's summary and extraction on
/// this computer ([`crate::routing::build_local_chain`]) or skips them.
pub fn classify_before_hosted(store: &memory::Store, material_text: &str, attendee_emails: &[String]) -> bool {
    let filter = context_filter(store);
    classify_with(&filter, material_text, attendee_emails)
}

pub(crate) fn classify_with(filter: &llm::ContextFilter, material_text: &str, attendee_emails: &[String]) -> bool {
    if filter.is_closed() {
        return true;
    }
    if filter.material().is_none() {
        return false;
    }
    filter.names_client(material_text)
        || attendee_emails.iter().any(|a| filter.names_client(a))
        || filter.addresses_client(attendee_emails)
}

/// `events` without an excluded client's (filed under one, or titled with its name). Uses the
/// hosted filter even for an all-local chain: the titles are stored in the run, which later
/// rides hosted prompts unless it is clean.
pub fn events_without_excluded(
    store: &memory::Store,
    events: &[routines::CalendarEventSummary],
    used: &llm::ContextFilter,
) -> Vec<routines::CalendarEventSummary> {
    let current;
    let filter = if used.is_active() {
        used
    } else {
        current = context_filter(store);
        &current
    };
    events.iter().filter(|e| !filter.excludes_event(e.id.as_deref().unwrap_or(""), &e.title, e.start_unix_ms)).cloned().collect()
}

/// Provenance for a routine run or brief built with `used` from `inputs`
/// ([`memory::topics::exclusion::input_ref`]): the inputs, clean while none of them is excluded
/// (fix round 4, S2). A closed filter, or more inputs than the cap, gives
/// [`memory::topics::exclusion::UNFILTERED_PROVENANCE`]. Fix round 1, B3: a run that is not clean
/// is kept out of hosted prompts and MCP, never deleted. A run built unfiltered (on this
/// computer) while some client is excluded is not clean either: its inputs were never checked
/// for words that name the client.
pub fn provenance_for(store: &memory::Store, used: &llm::ContextFilter, inputs: impl IntoIterator<Item = String>) -> String {
    if used.is_closed() || (!used.is_active() && context_filter(store).is_active()) {
        return memory::topics::exclusion::UNFILTERED_PROVENANCE.to_string();
    }
    memory::topics::exclusion::input_refs_stamp(inputs)
        .unwrap_or_else(|| memory::topics::exclusion::UNFILTERED_PROVENANCE.to_string())
}

/// The local re-summary offer (spec 7.4): the meetings of client `key` whose summary a hosted
/// model wrote (or whose summary records no provider). Summarising one again with
/// [`crate::summary::summarize_meeting`] now runs on this computer, since the meeting is the
/// excluded client's. Shown with [`ALREADY_LEFT_NOTE`].
pub fn hosted_summaries_to_redo(store: &memory::Store, key: &str) -> rusqlite::Result<Vec<i64>> {
    store.hosted_summarised_meetings(key)
}

/// Prompts a prepare built for a stream on this computer that hold material a hosted model must
/// not see (by hash). A stream to an Ollama cloud model refuses them ([`stream_gate`]): the
/// prepares are told only "Ollama", not which Ollama model the shell will stream.
static LOCAL_ONLY_PROMPTS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
const LOCAL_ONLY_PROMPTS_MAX: usize = 256;

fn prompt_hash(prompt: &str) -> u64 {
    prompt.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// Records that `prompt` must stay on this computer.
pub(crate) fn mark_local_only(prompt: &str) {
    let mut v = LOCAL_ONLY_PROMPTS.lock().unwrap_or_else(|e| e.into_inner());
    if v.len() >= LOCAL_ONLY_PROMPTS_MAX {
        v.remove(0);
    }
    v.push(prompt_hash(prompt));
}

fn is_local_only(prompt: &str) -> bool {
    LOCAL_ONLY_PROMPTS.lock().unwrap_or_else(|e| e.into_inner()).contains(&prompt_hash(prompt))
}

/// `gate` plus the prompts marked local-only: the gate for a stream to an Ollama cloud model.
struct StreamGate(Arc<dyn llm::HostedGate>);

impl llm::HostedGate for StreamGate {
    fn check(&self, prompt: &str) -> Result<(), llm::GateRefusal> {
        if is_local_only(prompt) {
            return Err(llm::GateRefusal { reason: llm::guard::REASON_EXCLUDED });
        }
        self.0.check(prompt)
    }
}

/// The gate a stream to Ollama's cloud passes: `gate`, and no prompt a prepare built from
/// material that must stay on this computer.
pub fn stream_gate(gate: Arc<dyn llm::HostedGate>) -> Arc<dyn llm::HostedGate> {
    Arc::new(StreamGate(gate))
}

/// Whether the Ollama model the shell streams with runs in Ollama's cloud: the routing's Ollama
/// model, or the picked model when Ollama is preferred.
pub fn ollama_is_cloud(routing: &crate::RoutingConfig) -> bool {
    llm::is_ollama_cloud_model(&routing.ollama_model)
        || (routing.preferred_provider == "ollama" && llm::is_ollama_cloud_model(&routing.preferred_model))
}

/// Whether the stream a shell starts from a prepare leaves this computer: a hosted provider, or
/// an Ollama cloud model. In local-only mode it never does: a prepare refuses instead
/// ([`local_only_mode_refuses`]).
pub fn streamed_is_hosted(routing: &crate::RoutingConfig) -> bool {
    let provider = crate::ask::provider_for_streamed_answer(routing);
    match provider {
        llm::Provider::Ollama => !routing.sensitive && ollama_is_cloud(routing),
        p => llm::is_hosted(p, ""),
    }
}

/// Local-only mode with an Ollama cloud model: nothing may stream (the model is not local), so a
/// prepare refuses instead of handing the shell a prompt it would send to Ollama's servers.
pub fn local_only_mode_refuses(routing: &crate::RoutingConfig) -> bool {
    // A shell that does not say which Ollama model it streams: core cannot know it is on this
    // computer, so local-only mode fails closed (fix round 2, M4).
    routing.sensitive && (routing.ollama_model.trim().is_empty() || ollama_is_cloud(routing))
}

/// The refusal for a request that carries an excluded client and has no local model to go to.
pub(crate) fn refused() -> crate::ApiError {
    crate::envelope::chain_failed(vec![llm::ProviderFailureKind::ExcludedClient])
}

static NEXT_SEQ: AtomicU64 = AtomicU64::new(1);
static PUBLISHED: Published = Published::new();

/// The published gate and the sequence number of the read that built it.
struct Published(RwLock<Option<(u64, Arc<dyn HostedGate>)>>);

impl Published {
    const fn new() -> Self {
        Self(RwLock::new(None))
    }

    /// Replaces the gate only when `seq` is newer than the one published.
    fn offer(&self, seq: u64, gate: Arc<dyn HostedGate>) {
        let mut slot = self.0.write().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_none_or(|(published, _)| seq > *published) {
            *slot = Some((seq, gate));
        }
    }

    fn get(&self) -> Arc<dyn HostedGate> {
        match &*self.0.read().unwrap_or_else(|e| e.into_inner()) {
            Some((_, gate)) => gate.clone(),
            None => Arc::new(RefuseAllHosted),
        }
    }
}

fn gate_for(unreadable: bool, terms: rusqlite::Result<Vec<String>>) -> Arc<dyn HostedGate> {
    match terms {
        _ if unreadable => Arc::new(RefuseAllHosted),
        Err(_) => Arc::new(RefuseAllHosted),
        Ok(terms) => {
            let set = ExclusionSet::new(terms);
            if set.is_empty() {
                Arc::new(NoExclusions)
            } else {
                Arc::new(set)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::topics::client_controls::ClientControl;

    fn store() -> (tempfile::TempDir, memory::Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::test_store::open(&dir.path().join("t.db"), dir.path());
        (dir, store)
    }

    #[test]
    fn nothing_excluded_passes_everything() {
        let (_d, s) = store();
        s.create_seed_topic("Acme", &[], None).unwrap();
        assert!(current_gate(&s).check("the Acme call").is_ok());
    }

    #[test]
    fn an_excluded_client_and_its_aliases_are_refused() {
        let (_d, s) = store();
        let acme = s.create_seed_topic("Acme", &["Roadrunner Co".to_string()], None).unwrap();
        s.set_client_control(&acme.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        let gate = current_gate(&s);
        assert_eq!(gate.check("the ACME call").unwrap_err().reason, llm::guard::REASON_EXCLUDED);
        assert!(gate.check("from roadrunner co").is_err());
        assert!(gate.check("the Globex call").is_ok());
    }

    #[test]
    fn a_gate_never_published_refuses_every_hosted_prompt() {
        let never = Published::new();
        assert_eq!(never.get().check("hello").unwrap_err().reason, llm::guard::REASON_UNREADABLE);
    }

    /// Two reads race: the older one (smaller sequence) finishes last and must not replace
    /// the newer gate.
    #[test]
    fn an_older_read_cannot_replace_a_newer_gate() {
        let slot = Published::new();
        slot.offer(5, Arc::new(ExclusionSet::new(vec!["Acme".into()])));
        slot.offer(3, Arc::new(NoExclusions));
        assert!(slot.get().check("the Acme call").is_err(), "the stale no-op gate was dropped");
        slot.offer(6, Arc::new(NoExclusions));
        assert!(slot.get().check("the Acme call").is_ok(), "a newer read replaces it");
    }

    #[test]
    fn check_stream_refuses_hosted_only() {
        let (_d, s) = store();
        let acme = s.create_seed_topic("Acme", &[], None).unwrap();
        s.set_client_control(&acme.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        assert!(check_stream(&s, llm::Provider::Ollama, "the Acme call").is_ok());
        assert!(check_stream(&s, llm::Provider::Local, "the Acme call").is_ok());
        assert!(matches!(check_stream(&s, llm::Provider::Anthropic, "the Acme call"), Err(crate::ApiError::ProviderFailure)));
        assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
        assert!(check_stream(&s, llm::Provider::Anthropic, "the Globex call").is_ok());
    }

    /// The prepares that precede a stream refuse a hosted one themselves; a local stream
    /// still gets its prompt.
    #[test]
    fn a_prepare_for_a_hosted_stream_refuses_a_prompt_naming_an_excluded_client() {
        let (_d, s) = store();
        let acme = s.create_seed_topic("Acme", &[], None).unwrap();
        s.set_client_control(&acme.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        let hosted = crate::RoutingConfig {
            preferred_provider: "anthropic".into(),
            anthropic_key: Some("k".into()),
            ..Default::default()
        };
        assert!(crate::ask::prepare_ask(&s, "what did Acme decide?", 5, &[], &hosted).is_err());
        assert_eq!(crate::last_provider_failure(), Some(llm::ProviderFailureKind::ExcludedClient));
        let input = crate::window_ask::WindowInput { app_name: "Mail".into(), title: "Inbox".into(), text: "From Acme".into(), is_selection: false };
        assert!(crate::window_ask::prepare_window_ask(&s, &input, "summarise", &[], &hosted).is_err());
        assert!(crate::ask::prepare_ask(&s, "what did Acme decide?", 5, &[], &crate::RoutingConfig::default()).is_ok());
    }

    #[test]
    fn an_unreadable_setting_refuses_every_hosted_prompt() {
        assert_eq!(gate_for(true, Ok(vec![])).check("hello").unwrap_err().reason, llm::guard::REASON_UNREADABLE);
        let err = rusqlite::Error::InvalidQuery;
        assert!(gate_for(false, Err(err)).check("hello").is_err());
        assert!(gate_for(false, Ok(vec![])).check("hello").is_ok());
    }
}
