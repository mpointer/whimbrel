//! The LLM-backed background batches `core/memory` can't run itself (it has no `llm`
//! dependency): episode summaries (docs/superpowers/specs/2026-08-12-l2-episodes-design.md)
//! and incremental thread summaries (docs/superpowers/specs/2026-08-14-thread-distillation-design.md).
//! Moved from `core/ffi::whimbrel_episodes_run_summarize_batch` /
//! `whimbrel_threads_run_summarize_batch` (M3.0). Both run against the Ollama `model` only —
//! these are unattended jobs, never routed to a hosted provider. An Ollama cloud model is
//! hosted, though (`llm::is_ollama_cloud_model`): with one, the client is behind the exclusion
//! gate, and an excluded client's episode or thread is not summarised at all (spec 7.1: made on
//! this computer, or not made). The TypeSafe priority scorer is hosted: an excluded client's
//! thread is never scored.

use crate::util::now_unix_ms;

/// Runs up to `max_jobs` pending `episode_summarize` jobs. `custom_instructions` is
/// trimmed; whitespace-only means "no instructions". `output_language` is a BCP 47 tag
/// for the summary's language; "" adds no language line. Returns the count actually
/// processed (`0` means nothing was eligible); `Err` only if the claim itself failed.
pub fn run_episode_summarize_batch(
    store: &memory::Store,
    ollama_model: &str,
    max_jobs: usize,
    custom_instructions: &str,
    output_language: &str,
) -> rusqlite::Result<usize> {
    let client = ollama_client(store, ollama_model);
    let hosted = llm::is_ollama_cloud_model(ollama_model);
    run_episode_summarize_batch_with(store, client.as_ref(), hosted, max_jobs, custom_instructions, output_language)
}

/// [`run_episode_summarize_batch`] through `client`; `hosted`: it runs in Ollama's cloud.
pub(crate) fn run_episode_summarize_batch_with(
    store: &memory::Store,
    client: &dyn llm::LlmClient,
    hosted: bool,
    max_jobs: usize,
    custom_instructions: &str,
    output_language: &str,
) -> rusqlite::Result<usize> {
    if memory::background_gate::recording_pauses_background_work() {
        return Ok(0);
    }
    let custom_instructions = custom_instructions.trim();
    let jobs = store.claim_episode_summarize_jobs(max_jobs, now_unix_ms())?;
    let mut processed = 0;
    for job in jobs {
        // Read again for each job (cached until anything changes): a client excluded during the
        // batch is never sent for the rest of it (fix round 4, M1).
        let filter = filter_if_hosted(store, hosted);
        // An unreadable setting: nothing is sent, and the job waits (never ends on it).
        if filter.is_closed() {
            let _ = store.retry_episode_summarize_job_after_transport_error(job.job_id, now_unix_ms());
            continue;
        }
        if excluded_episode(&filter, &job) {
            // Not made: a hosted model may not see it, and there is none here to make it.
            let _ = store.fail_episode_summarize_job(job.job_id, NOT_MADE, job.episode_id, now_unix_ms());
            processed += 1;
            continue;
        }
        // No model for episodes with nothing to summarize: their captures are gone
        // (retention, dedupe), or it was one glance.
        match skip_reason(&job.items, output_language) {
            Some(Skip::Empty) => {
                let _ = store.forget_episode_activity(job.episode_id);
                processed += 1;
                continue;
            }
            Some(Skip::Glance(summary)) => {
                let _ = store.complete_episode_summarize_job(job.job_id, job.episode_id, &summary, "[]", now_unix_ms());
                processed += 1;
                continue;
            }
            None => {}
        }
        // Re-read the clock per job: an Ollama call takes seconds to tens of seconds, so
        // the batch-start time (fine for the one-shot eligibility check) would shorten
        // the intended backoff for later jobs in the same batch.
        let now = now_unix_ms();
        match llm::summarize_episode_in(client, &job.items, custom_instructions, output_language) {
            Ok(summary) => {
                let refs_json =
                    serde_json::to_string(&summary.refs).unwrap_or_else(|_| "[]".to_string());
                let _ = store.complete_episode_summarize_job(
                    job.job_id,
                    job.episode_id,
                    &summary.summary,
                    &refs_json,
                    now,
                );
            }
            Err(e) => {
                store.record_summarize_error("episode_summarize", &e.to_string(), now);
                if e.is_environmental() {
                    // Ollama isn't reachable, or the model isn't pulled: that says nothing
                    // about whether THIS job can ever succeed -- retry forever rather than
                    // burning all five attempts to a terminal 'failed', the same
                    // philosophy `episode_segment`/`reindex_sweep` already use.
                    let _ = store.retry_episode_summarize_job_after_transport_error(job.job_id, now);
                } else {
                    // Ollama IS serving and something else went wrong (bad output, a
                    // server error), which should count against the attempt ceiling.
                    let _ = store.fail_episode_summarize_job(
                        job.job_id,
                        job.attempts,
                        job.episode_id,
                        now,
                    );
                }
            }
        }
        processed += 1;
    }
    Ok(processed)
}

/// Attempts that end a summarize or priority job at once: the item is not made, not retried.
const NOT_MADE: i64 = i64::MAX - 1;

/// The Ollama client for an unattended job; an Ollama cloud model is behind the exclusion gate.
fn ollama_client(store: &memory::Store, ollama_model: &str) -> Box<dyn llm::LlmClient> {
    let client = Box::new(llm::OllamaClient::new(ollama_model));
    if llm::is_ollama_cloud_model(ollama_model) {
        Box::new(llm::GuardedClient::new(client, crate::exclusion::current_gate(store)))
    } else {
        client
    }
}

/// The exclusion filter when `ollama_model` runs in Ollama's cloud; open on this computer.
#[cfg(test)]
fn hosted_filter(store: &memory::Store, ollama_model: &str) -> llm::ContextFilter {
    filter_if_hosted(store, llm::is_ollama_cloud_model(ollama_model))
}

/// The exclusion filter for a model that is `hosted`; open on this computer.
fn filter_if_hosted(store: &memory::Store, hosted: bool) -> llm::ContextFilter {
    if hosted {
        crate::exclusion::context_filter(store)
    } else {
        llm::ContextFilter::none()
    }
}

/// Whether an episode job is an excluded client's: filed under one, or a capture names one.
fn excluded_episode(filter: &llm::ContextFilter, job: &memory::EpisodeSummarizeJob) -> bool {
    if !filter.is_active() {
        return false;
    }
    filter.material().is_none_or(|m| m.episodes.contains(&job.episode_id))
        || job.items.iter().any(|c| !filter.keeps(&llm::ContextItem::Capture(c.clone())))
}

/// Whether a thread is an excluded client's: filed under one, or its text or senders name one
/// or carry a client contact's address.
fn excluded_thread(filter: &llm::ContextFilter, thread_id: i64, prior: Option<&str>, messages: &[memory::ThreadEmailMessage]) -> bool {
    if !filter.is_active() {
        return false;
    }
    let Some(m) = filter.material() else { return true };
    m.threads.contains(&thread_id)
        || messages.iter().any(|msg| m.items.contains(&msg.item_id))
        || prior.is_some_and(|p| filter.names_client(p))
        || messages.iter().any(|msg| {
            filter.names_client(&msg.body_text)
                || filter.names_client(&msg.from)
                || llm::exclusion::attendee_is_client(m, &msg.from)
        })
}

/// A glance: at most one capture, spanning under this long.
const GLANCE_MAX_MS: i64 = 30_000;

enum Skip {
    Empty,
    Glance(String),
}

/// Why an episode needs no model call, if it doesn't. On real data, 1,015 of 1,501
/// failed summaries had no captures left, and nearly every "no meaningful activity"
/// summary came from an empty or one-capture episode.
fn skip_reason(items: &[memory::CaptureItem], lang: &str) -> Option<Skip> {
    let (Some(first), Some(last)) = (items.first(), items.last()) else {
        return Some(Skip::Empty);
    };
    if items.len() <= 1 && last.ts_unix_ms - first.ts_unix_ms < GLANCE_MAX_MS {
        let app = first.app_bundle_id.rsplit('.').next().unwrap_or(&first.app_bundle_id);
        let title = first.window_title.trim();
        use crate::l10n::{keys, text};
        return Some(Skip::Glance(if title.is_empty() {
            text(lang, keys::BRIEFLY_VIEWED_APP, &[("app", app)])
        } else {
            text(lang, keys::BRIEFLY_VIEWED_TITLE_IN_APP, &[("title", title), ("app", app)])
        }));
    }
    None
}

/// Runs up to `max_jobs` pending `thread_summarize` jobs, mirroring
/// [`run_episode_summarize_batch`] exactly. Gmail text goes to an Ollama cloud model only
/// with [`run_thread_summarize_batch_with`]'s switch on; here it never does.
pub fn run_thread_summarize_batch(
    store: &memory::Store,
    ollama_model: &str,
    max_jobs: usize,
    custom_instructions: &str,
    output_language: &str,
) -> rusqlite::Result<usize> {
    run_thread_summarize_batch_with(store, ollama_model, max_jobs, custom_instructions, output_language, false)
}

/// [`run_thread_summarize_batch`] with the Limited Use switch `remote_ollama_google_data`
/// (`RoutingConfig::remote_ollama_google_data`). Thread summaries read Gmail messages, so
/// with an Ollama cloud model (`:cloud` / `-cloud`, run on Ollama's servers) and the
/// switch off, no job is claimed: they stay pending, unspent, for a local model.
pub fn run_thread_summarize_batch_with(
    store: &memory::Store,
    ollama_model: &str,
    max_jobs: usize,
    custom_instructions: &str,
    output_language: &str,
    remote_ollama_google_data: bool,
) -> rusqlite::Result<usize> {
    if memory::background_gate::recording_pauses_background_work() {
        return Ok(0);
    }
    let policy = llm::limited_use::GoogleDataPolicy { remote_ollama_google_data, ..Default::default() };
    if !llm::limited_use::provider_may_receive_google_data(llm::Provider::Ollama, ollama_model, None, policy) {
        return Ok(0);
    }
    let client = ollama_client(store, ollama_model);
    let hosted = llm::is_ollama_cloud_model(ollama_model);
    run_thread_summarize_batch_through(store, client.as_ref(), hosted, max_jobs, custom_instructions, output_language)
}

/// [`run_thread_summarize_batch_with`] through `client` (Limited Use already decided);
/// `hosted`: it runs in Ollama's cloud, so each job also passes the exclusion filter.
pub(crate) fn run_thread_summarize_batch_through(
    store: &memory::Store,
    client: &dyn llm::LlmClient,
    hosted: bool,
    max_jobs: usize,
    custom_instructions: &str,
    output_language: &str,
) -> rusqlite::Result<usize> {
    let custom_instructions = custom_instructions.trim();
    let jobs = store.claim_thread_summarize_jobs(max_jobs, now_unix_ms())?;
    let mut processed = 0;
    for job in jobs {
        let now = now_unix_ms();
        // Read again for each job (fix round 4, M1): a client excluded during the batch is never
        // sent for the rest of it.
        let filter = filter_if_hosted(store, hosted);
        if filter.is_closed() {
            let _ = store.retry_thread_summarize_job_after_transport_error(job.job_id, now);
            continue;
        }
        if excluded_thread(&filter, job.thread_id, job.prior_summary.as_deref(), &job.new_messages) {
            let _ = store.fail_thread_summarize_job(job.job_id, NOT_MADE, job.thread_id, now);
            processed += 1;
            continue;
        }
        match llm::summarize_thread_incrementally_in(
            client,
            job.prior_summary.as_deref(),
            &job.new_messages,
            custom_instructions,
            output_language,
        ) {
            Ok(summary) => {
                // Cursor is compared against items.id, not timestamp order -- new_messages
                // is sorted chronologically (ts_unix_ms ASC), which need not match id order
                // for peer-synced items, so take the max id actually covered rather than the
                // id of the chronologically-last message.
                let last_item_id = job
                    .new_messages
                    .iter()
                    .map(|m| m.item_id)
                    .max()
                    .unwrap_or(0);
                let _ = store.complete_thread_summarize_job(
                    job.job_id,
                    job.thread_id,
                    &summary.summary,
                    last_item_id,
                    now,
                );
            }
            Err(e) => {
                store.record_summarize_error("thread_summarize", &e.to_string(), now);
                if e.is_environmental() {
                    let _ = store.retry_thread_summarize_job_after_transport_error(job.job_id, now);
                } else {
                    let _ = store.fail_thread_summarize_job(
                        job.job_id,
                        job.attempts,
                        job.thread_id,
                        now,
                    );
                }
            }
        }
        processed += 1;
    }
    Ok(processed)
}

/// What one `run_thread_priority_batch` call did.
pub struct PriorityBatchOutcome {
    pub processed: usize,
    /// TypeSafe rejected the API key (401/403): the app should show "key rejected".
    pub auth_rejected: bool,
}

/// How a TypeSafe failure is treated. Only an error that says something about THIS
/// job's input counts against its attempts.
#[derive(Debug, PartialEq)]
enum PriorityFailure {
    AuthRejected,
    RetryAfterMs(i64),
    CountAttempt,
}

fn classify_priority_error(e: &llm::LlmError) -> PriorityFailure {
    match e {
        llm::LlmError::Http { status: 401 | 403, .. } => PriorityFailure::AuthRejected,
        llm::LlmError::Http { status: 429, .. } => PriorityFailure::RetryAfterMs(5 * 60_000),
        llm::LlmError::Http { status, .. } if *status >= 500 => PriorityFailure::RetryAfterMs(30_000),
        llm::LlmError::Network(_) => PriorityFailure::RetryAfterMs(30_000),
        // The exclusion setting couldn't be read: nothing was sent; try again later. A
        // thread naming an excluded client never will be sent, so it may fail.
        llm::LlmError::Refused(r) if r.reason == llm::guard::REASON_UNREADABLE => PriorityFailure::RetryAfterMs(5 * 60_000),
        _ => PriorityFailure::CountAttempt,
    }
}

/// The latest message is cut to this many characters before it leaves the machine.
const PRIORITY_MESSAGE_CHARS: usize = 1000;

/// "Name <addr@host>" or a bare address → (display name, lowercase domain).
fn split_sender(raw: &str) -> (String, String) {
    let (name, addr) = match (raw.rfind('<'), raw.rfind('>')) {
        (Some(l), Some(r)) if l < r => (raw[..l].trim(), &raw[l + 1..r]),
        _ => ("", raw.trim()),
    };
    let domain = if addr.contains('@') { addr.rsplit('@').next().unwrap_or("").trim().to_lowercase() } else { String::new() };
    (name.trim_matches('"').trim().to_string(), domain)
}

/// Builds the only data that leaves the machine. Everything is redacted FIRST and the
/// message is cut afterwards, so a secret that straddles the cut can't leak half-redacted.
fn build_priority_input(job: &memory::ThreadPriorityJob) -> llm::typesafe::PriorityInput {
    let (sender_name, sender_domain) = split_sender(&job.sender);
    let redacted_message = memory::redact::redact_secrets(&job.latest_message);
    llm::typesafe::PriorityInput {
        subject: memory::redact::redact_secrets(&job.subject).into_owned(),
        sender_name: memory::redact::redact_secrets(&sender_name).into_owned(),
        sender_domain,
        latest_message: redacted_message.chars().take(PRIORITY_MESSAGE_CHARS).collect(),
    }
}

/// Scores one input; a panic inside the client (ureq has panicked on socket-return
/// failures) must not unwind through the `extern "C"` caller, so it becomes an error.
fn score_catching_panics(
    scorer: &dyn llm::typesafe::PriorityScorer,
    input: &llm::typesafe::PriorityInput,
) -> Result<memory::priority::PriorityJudgments, llm::LlmError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scorer.score(input)))
        .unwrap_or_else(|_| Err(llm::LlmError::Provider("TypeSafe client panicked".to_string())))
}

/// Runs up to `max_jobs` pending `thread_priority` jobs through `scorer`. After a
/// rejected key the remaining claimed jobs are released and the batch stops.
///
/// Limited Use: every thread is a Gmail thread and the scorer is TypeSafe, so until
/// TypeSafe confirms in writing that it does not train on API data
/// (`memory::limited_use::TYPESAFE_GOOGLE_DATA_CONFIRMED`) nothing is claimed or sent:
/// the jobs stay pending (no attempt counted) and the batch reports 0 processed.
pub fn run_thread_priority_batch(
    store: &memory::Store,
    scorer: &dyn llm::typesafe::PriorityScorer,
    max_jobs: usize,
) -> rusqlite::Result<PriorityBatchOutcome> {
    run_thread_priority_batch_if(store, scorer, max_jobs, memory::limited_use::TYPESAFE_GOOGLE_DATA_CONFIRMED)
}

fn run_thread_priority_batch_if(
    store: &memory::Store,
    scorer: &dyn llm::typesafe::PriorityScorer,
    max_jobs: usize,
    typesafe_may_read_gmail: bool,
) -> rusqlite::Result<PriorityBatchOutcome> {
    let mut outcome = PriorityBatchOutcome { processed: 0, auth_rejected: false };
    if !typesafe_may_read_gmail || memory::background_gate::recording_pauses_background_work() {
        return Ok(outcome);
    }
    let mut jobs = store.claim_thread_priority_jobs(max_jobs, now_unix_ms())?.into_iter();
    while let Some(job) = jobs.next() {
        let now = now_unix_ms();
        // TypeSafe is hosted: an excluded client's thread is never scored (classify first). Read
        // again for each job (fix round 4, M1): a client excluded during the batch is never sent.
        let filter = crate::exclusion::context_filter(store);
        // An unreadable setting: nothing is sent, and the job waits (never ends on it).
        if filter.is_closed() {
            store.retry_thread_priority_job_later(job.job_id, 5 * 60_000, now)?;
            continue;
        }
        let message = memory::ThreadEmailMessage { item_id: -1, from: job.sender.clone(), ts_unix_ms: 0, body_text: format!("{}\n{}", job.subject, job.latest_message) };
        if excluded_thread(&filter, job.thread_id, None, std::slice::from_ref(&message)) {
            store.fail_thread_priority_job(job.job_id, NOT_MADE, now)?;
            outcome.processed += 1;
            continue;
        }
        match score_catching_panics(scorer, &build_priority_input(&job)) {
            Ok(judgments) => {
                store.complete_thread_priority_job(job.job_id, job.thread_id, job.message_count, &judgments, now)?;
                outcome.processed += 1;
            }
            Err(e) => {
                // Same kv_meta slot family as the summarize jobs: core's stderr never
                // reaches the app log, so the reason has to live in the database.
                store.record_summarize_error("thread_priority", &e.to_string(), now);
                match classify_priority_error(&e) {
                    PriorityFailure::AuthRejected => {
                        store.retry_thread_priority_job_later(job.job_id, 30_000, now)?;
                        for rest in jobs.by_ref() {
                            store.retry_thread_priority_job_later(rest.job_id, 30_000, now)?;
                        }
                        outcome.auth_rejected = true;
                        break;
                    }
                    PriorityFailure::RetryAfterMs(delay) => {
                        store.retry_thread_priority_job_later(job.job_id, delay, now)?;
                    }
                    PriorityFailure::CountAttempt => {
                        store.fail_thread_priority_job(job.job_id, job.attempts, now)?;
                    }
                }
            }
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_errors_are_classified() {
        let http = |status| llm::LlmError::Http { status, body: String::new() };
        assert_eq!(classify_priority_error(&http(429)), PriorityFailure::RetryAfterMs(300_000));
        assert_eq!(classify_priority_error(&http(503)), PriorityFailure::RetryAfterMs(30_000));
        assert_eq!(classify_priority_error(&llm::LlmError::Network("x".into())), PriorityFailure::RetryAfterMs(30_000));
        assert_eq!(classify_priority_error(&http(401)), PriorityFailure::AuthRejected);
        assert_eq!(classify_priority_error(&http(403)), PriorityFailure::AuthRejected);
        assert_eq!(classify_priority_error(&llm::LlmError::Provider("x".into())), PriorityFailure::CountAttempt);
    }

    fn capture(ts: i64, title: &str) -> memory::CaptureItem {
        memory::CaptureItem {
            id: ts,
            device_id: "d".into(),
            ts_unix_ms: ts,
            app_bundle_id: "com.apple.Terminal".into(),
            window_title: title.into(),
            url: None,
            text: "x".into(),
            hash: 0,
            source: None,
            local_seq: None,
        }
    }

    #[test]
    fn persisted_strings_render_english_for_empty_lang() {
        match skip_reason(&[capture(1_000, "  ")], "") {
            Some(Skip::Glance(s)) => assert_eq!(s, "Briefly viewed Terminal."),
            _ => panic!("one capture is a glance"),
        }
        match skip_reason(&[capture(1_000, "{app}")], "") {
            Some(Skip::Glance(s)) => assert_eq!(s, "Briefly viewed \"{app}\" in Terminal."),
            _ => panic!("one capture is a glance"),
        }
    }

    #[test]
    fn empty_and_glance_episodes_skip_the_model_and_real_ones_do_not() {
        assert!(matches!(skip_reason(&[], ""), Some(Skip::Empty)));
        match skip_reason(&[capture(1_000, "zsh")], "") {
            Some(Skip::Glance(s)) => assert_eq!(s, "Briefly viewed \"zsh\" in Terminal."),
            _ => panic!("one capture is a glance"),
        }
        assert!(skip_reason(&[capture(1_000, "a"), capture(2_000, "b")], "").is_none(), "two captures get a real summary");
    }

    /// Mirrors `core/ffi`'s `*_returns_zero_when_nothing_pending` tests: no jobs means
    /// no Ollama call, so this is deterministic without a server.
    #[test]
    fn both_batches_return_zero_when_nothing_is_pending() {
        let store = memory::Store::open_in_memory().unwrap();
        assert_eq!(
            run_episode_summarize_batch(&store, "gemma4:12b", 5, "   ", "").unwrap(),
            0
        );
        assert_eq!(
            run_thread_summarize_batch(&store, "gemma4:12b", 5, "   ", "").unwrap(),
            0
        );
    }

    use llm::typesafe::{PriorityInput, PriorityScorer};
    use memory::priority::PriorityJudgments;
    use std::cell::RefCell;

    struct FakeScorer {
        inputs: RefCell<Vec<PriorityInput>>,
        results: RefCell<Vec<Result<PriorityJudgments, llm::LlmError>>>,
    }

    impl FakeScorer {
        fn new(results: Vec<Result<PriorityJudgments, llm::LlmError>>) -> Self {
            Self { inputs: RefCell::new(Vec::new()), results: RefCell::new(results) }
        }
    }

    impl PriorityScorer for FakeScorer {
        fn score(&self, input: &PriorityInput) -> Result<PriorityJudgments, llm::LlmError> {
            self.inputs.borrow_mut().push(input.clone());
            self.results.borrow_mut().remove(0)
        }
    }

    struct PanickingScorer;

    impl PriorityScorer for PanickingScorer {
        fn score(&self, _input: &PriorityInput) -> Result<PriorityJudgments, llm::LlmError> {
            panic!("boom")
        }
    }

    fn judgments() -> PriorityJudgments {
        PriorityJudgments { asks_reply: 0.9, urgency: 2.0, person: 0.7, bulk: 0.1, automated: 0.05 }
    }

    /// A store with `n` waiting-on-me Gmail threads and their pending `thread_priority` jobs.
    fn store_with_waiting_threads(n: usize, body: &str) -> memory::Store {
        let store = memory::Store::open_in_memory().unwrap();
        let items: Vec<memory::NewEmailItem> = (0..n)
            .map(|i| memory::NewEmailItem {
                device_id: "dev-1".into(),
                external_id: format!("msg-{i}"),
                ts_unix_ms: 1000 + i as i64,
                from: "\"Ben Lawrick\" <ben@alphasights.com>".into(),
                to: "me@example.com".into(),
                subject: format!("Subject {i}"),
                body_text: body.into(),
                gmail_thread_id: format!("thread-{i}"),
                account_email: "me@example.com".into(),
                ..Default::default()
            })
            .collect();
        store.upsert_email_items(&items).unwrap();
        store.run_jobs_batch(10, 5_000).unwrap(); // drives thread_state_sync
        store
    }

    fn job_statuses(store: &memory::Store) -> Vec<String> {
        // Only through public API: a second claim returns what is still pending.
        store.claim_thread_priority_jobs(100, i64::MAX / 2).unwrap().iter().map(|j| format!("pending:{}", j.thread_id)).collect()
    }

    /// TypeSafe is hosted: a thread naming an excluded client is never scored, and its job ends
    /// without a call; an Ollama cloud model's episode and thread summaries leave the client out.
    #[test]
    fn unattended_jobs_never_send_excluded_text() {
        let store = store_with_waiting_threads(2, "Please confirm by Friday.");
        let acme = store.create_seed_topic("Ben Lawrick", &[], None).unwrap();
        store
            .set_client_control(&acme.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        let scorer = FakeScorer::new(vec![]);
        // Limited Use holds every Gmail thread from TypeSafe today (nothing sent, nothing ended).
        assert_eq!(run_thread_priority_batch(&store, &scorer, 5).unwrap().processed, 0);
        // Once TypeSafe may read Gmail, the exclusion gate still keeps the client's threads.
        let outcome = run_thread_priority_batch_if(&store, &scorer, 5, true).unwrap();
        assert!(scorer.inputs.borrow().is_empty(), "no thread from the client was sent");
        assert_eq!(outcome.processed, 2);
        assert!(job_statuses(&store).is_empty(), "the jobs are finished, not left to stall");

        // An Ollama cloud model is hosted: its client is behind the gate, and a thread or
        // episode of the client is not summarised at all.
        assert!(!hosted_filter(&store, "gemma4:12b").is_active());
        let filter = hosted_filter(&store, "gpt-oss:120b-cloud");
        assert!(filter.is_active());
        let msg = |from: &str, body: &str| memory::ThreadEmailMessage { item_id: 1, from: from.into(), ts_unix_ms: 1, body_text: body.into() };
        assert!(excluded_thread(&filter, 99, None, &[msg("x@y.com", "lunch with Ben Lawrick")]));
        assert!(!excluded_thread(&filter, 99, None, &[msg("x@y.com", "lunch on Friday")]));
        assert!(matches!(
            ollama_client(&store, "gpt-oss:120b-cloud").complete("notes from Ben Lawrick"),
            Err(llm::LlmError::Refused(_))
        ));
    }

    /// Runs `hook` once, at its first score: a client excluded while the batch runs.
    struct ExcludingScorer<'a> {
        inner: FakeScorer,
        hook: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
    }

    impl PriorityScorer for ExcludingScorer<'_> {
        fn score(&self, input: &PriorityInput) -> Result<PriorityJudgments, llm::LlmError> {
            if let Some(hook) = self.hook.borrow_mut().take() {
                hook();
            }
            self.inner.score(input)
        }
    }

    fn exclude(store: &memory::Store, key: &str) {
        store
            .set_client_control(key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
    }

    /// Fix round 4, M1: the TypeSafe batch reads the setting for each thread. A client excluded
    /// while the first thread is scored is never sent for the rest of the batch. Control: the
    /// first thread, scored before, was sent.
    #[test]
    fn a_client_excluded_during_a_priority_batch_is_not_sent_for_the_rest() {
        let store = store_with_waiting_threads(2, "Please confirm by Friday.");
        let ben = store.create_seed_topic("Ben Lawrick", &[], None).unwrap();
        let scorer = ExcludingScorer {
            inner: FakeScorer::new(vec![Ok(judgments()), Ok(judgments())]),
            hook: RefCell::new(Some(Box::new(|| exclude(&store, &ben.key)))),
        };
        let outcome = run_thread_priority_batch_if(&store, &scorer, 5, true).unwrap();
        assert_eq!(scorer.inner.inputs.borrow().len(), 1, "only the thread scored before the exclusion");
        assert_eq!(outcome.processed, 2, "the other job ends, not left to stall");
    }

    /// Runs `hook` once, at its first call, and records every prompt.
    struct HookedClient<'a> {
        prompts: RefCell<Vec<String>>,
        hook: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
    }

    impl llm::LlmClient for HookedClient<'_> {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            if let Some(hook) = self.hook.borrow_mut().take() {
                hook();
            }
            self.prompts.borrow_mut().push(prompt.to_string());
            Ok(llm::Completion { text: "A summary.".into(), token_usage: llm::TokenUsage::default() })
        }
    }

    /// X6 final review, Major 7: an Ollama cloud model never gets an excluded client's episode, even one whose
    /// captures never name the client (filed under it), and the job ends instead of stalling. Control: the same
    /// episode with nothing excluded is sent. Fails if the batch's `excluded_episode` check is dropped.
    #[test]
    fn an_excluded_clients_episode_is_never_sent_to_a_cloud_model() {
        for excluded in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let store = memory::Store::open(&dir.path().join("m.db"), dir.path()).unwrap();
            // Two captures in one app (not a glance), then another app, which closes the episode.
            for (ts, app, text) in [
                (1_000, "com.apple.Numbers", "zebracanary pricing sheet"),
                (61_000, "com.apple.Numbers", "zebracanary pricing totals"),
                (121_000, "com.apple.Notes", "groceries"),
            ] {
                store
                    .insert_capture_item(&memory::NewCaptureItem {
                        device_id: "d".into(),
                        ts_unix_ms: ts,
                        app_bundle_id: app.into(),
                        window_title: "Sheet".into(),
                        url: None,
                        text: text.into(),
                        hash: ts,
                        source: "ax".into(),
                    })
                    .unwrap();
            }
            store.run_jobs_batch(10, 121_000 + 600_001).unwrap(); // segments the captures into episodes
            let c = rusqlite::Connection::open(dir.path().join("m.db")).unwrap();
            let episode: i64 =
                c.query_row("SELECT id FROM episodes WHERE app_bundle_id = 'com.apple.Numbers'", [], |r| r.get(0)).unwrap();
            let acme = store.create_seed_topic("Acme", &[], None).unwrap();
            c.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES ('episode', ?1, 1, 1)", [episode])
                .unwrap();
            c.execute(
                "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')",
                [acme.topic_id, c.last_insert_rowid()],
            )
            .unwrap();
            if excluded {
                exclude(&store, &acme.key);
            }
            let client = HookedClient { prompts: RefCell::new(Vec::new()), hook: RefCell::new(None) };
            run_episode_summarize_batch_with(&store, &client, true, 5, "", "").unwrap();
            if excluded {
                assert!(client.prompts.borrow().is_empty(), "the excluded client's episode was sent");
                let left = store.claim_episode_summarize_jobs(5, i64::MAX / 2).unwrap();
                assert!(!left.iter().any(|j| j.episode_id == episode), "the job ended");
            } else {
                assert_eq!(client.prompts.borrow().len(), 1, "control: sent with nothing excluded");
            }
        }
    }

    /// Fix round 4, M1: a thread summary batch on an Ollama cloud model reads the setting for each
    /// thread. Control: the first thread, summarised before the exclusion, was sent.
    #[test]
    fn a_client_excluded_during_a_thread_summary_batch_is_not_sent_for_the_rest() {
        let store = store_with_waiting_threads(2, "Please confirm by Friday.");
        let ben = store.create_seed_topic("Ben Lawrick", &[], None).unwrap();
        let client = HookedClient { prompts: RefCell::new(Vec::new()), hook: RefCell::new(Some(Box::new(|| exclude(&store, &ben.key)))) };
        let processed = run_thread_summarize_batch_through(&store, &client, true, 5, "", "").unwrap();
        assert_eq!(processed, 2, "both jobs end");
        assert_eq!(client.prompts.borrow().len(), 1, "only the thread summarised before the exclusion");
    }

    /// Fix round 1, M1: an unreadable setting never ends a job: nothing is sent, the job waits.
    #[test]
    fn an_unreadable_setting_never_ends_a_job() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let store = memory::Store::open(&path, dir.path()).unwrap();
        store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "msg-0".into(),
                ts_unix_ms: 1000,
                from: "\"Ben\" <ben@example.com>".into(),
                to: "me@example.com".into(),
                subject: "Subject".into(),
                body_text: "Please confirm.".into(),
                gmail_thread_id: "thread-0".into(),
                account_email: "me@example.com".into(),
                ..Default::default()
            }])
            .unwrap();
        store.run_jobs_batch(10, 5_000).unwrap();
        rusqlite::Connection::open(&path).unwrap().execute("DROP TABLE client_control", []).unwrap();
        let scorer = FakeScorer::new(vec![]);
        run_thread_priority_batch_if(&store, &scorer, 5, true).unwrap();
        assert!(scorer.inputs.borrow().is_empty());
        assert_eq!(job_statuses(&store).len(), 1, "still pending, not failed");
    }

    #[test]
    fn a_successful_score_is_stored_and_the_job_is_done() {
        let store = store_with_waiting_threads(1, "Please confirm by Friday.");
        let scorer = FakeScorer::new(vec![Ok(judgments())]);

        let outcome = run_thread_priority_batch_if(&store, &scorer, 5, true).unwrap();

        assert_eq!((outcome.processed, outcome.auth_rejected), (1, false));
        let input = scorer.inputs.borrow()[0].clone();
        assert_eq!(input.subject, "Subject 0");
        assert_eq!(input.sender_name, "Ben Lawrick");
        assert_eq!(input.sender_domain, "alphasights.com");
        assert_eq!(input.latest_message, "Please confirm by Friday.");
        assert!(job_statuses(&store).is_empty(), "job should be done");
    }

    #[test]
    fn a_secret_straddling_the_1000_character_cut_is_still_redacted() {
        let key = format!("apikey_{}", "a1b2_c3".repeat(10)); // matches the TypeSafe key pattern
        let body = format!("{} {}", "x".repeat(989), key); // the space gives \b; the key starts at char 990
        let store = store_with_waiting_threads(1, &body);
        let scorer = FakeScorer::new(vec![Ok(judgments())]);

        run_thread_priority_batch_if(&store, &scorer, 1, true).unwrap();

        let sent = scorer.inputs.borrow()[0].latest_message.clone();
        assert!(sent.contains("[REDACTED"), "expected a redaction marker, got {sent:?}");
        assert!(!sent.contains("apikey_"), "raw key prefix leaked: {sent:?}");
        assert!(sent.chars().count() <= 1000);
    }

    #[test]
    fn a_key_glued_to_text_at_the_cut_is_redacted_too() {
        // The same case without the separating space: before the redactor dropped its
        // leading word boundary for distinctive prefixes, this key went out unredacted.
        let key = format!("apikey_{}", "a1b2_c3".repeat(10));
        let body = format!("{}{}", "x".repeat(990), key);
        let store = store_with_waiting_threads(1, &body);
        let scorer = FakeScorer::new(vec![Ok(judgments())]);

        run_thread_priority_batch_if(&store, &scorer, 1, true).unwrap();

        let sent = scorer.inputs.borrow()[0].latest_message.clone();
        assert!(sent.contains("[REDACTED"), "expected a redaction marker, got {sent:?}");
        assert!(!sent.contains("apikey_"), "raw key prefix leaked: {sent:?}");
    }

    #[test]
    fn network_and_server_errors_retry_without_using_attempts() {
        let store = store_with_waiting_threads(1, "hi");
        let scorer = FakeScorer::new(vec![Err(llm::LlmError::Network("down".into()))]);

        let outcome = run_thread_priority_batch_if(&store, &scorer, 1, true).unwrap();

        assert_eq!((outcome.processed, outcome.auth_rejected), (0, false));
        // Not claimable now (30 s backoff), and still pending with zero attempts afterwards.
        assert!(store.claim_thread_priority_jobs(1, 5_000).unwrap().is_empty());
        let later = store.claim_thread_priority_jobs(1, i64::MAX / 2).unwrap();
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].attempts, 0);
    }

    /// The exclusion setting couldn't be read: the job was never sent, so it waits and
    /// keeps its attempts. One naming an excluded client counts an attempt (it may fail).
    #[test]
    fn a_refusal_for_an_unreadable_setting_retries_without_using_attempts() {
        let refused = |reason| llm::LlmError::Refused(llm::GateRefusal { reason });
        assert_eq!(classify_priority_error(&refused(llm::guard::REASON_UNREADABLE)), PriorityFailure::RetryAfterMs(300_000));
        assert_eq!(classify_priority_error(&refused(llm::guard::REASON_EXCLUDED)), PriorityFailure::CountAttempt);

        let store = store_with_waiting_threads(1, "hi");
        let scorer = FakeScorer::new(vec![Err(refused(llm::guard::REASON_UNREADABLE))]);
        let outcome = run_thread_priority_batch_if(&store, &scorer, 1, true).unwrap();
        assert_eq!((outcome.processed, outcome.auth_rejected), (0, false));
        assert!(store.claim_thread_priority_jobs(1, 5_000).unwrap().is_empty());
        let later = store.claim_thread_priority_jobs(1, i64::MAX / 2).unwrap();
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].attempts, 0);
    }

    #[test]
    fn a_rejected_key_releases_every_claimed_job_and_flags_the_batch() {
        let store = store_with_waiting_threads(3, "hi");
        let scorer = FakeScorer::new(vec![Err(llm::LlmError::Http { status: 401, body: "bad key".into() })]);

        let outcome = run_thread_priority_batch_if(&store, &scorer, 3, true).unwrap();

        assert!(outcome.auth_rejected);
        assert_eq!(scorer.inputs.borrow().len(), 1, "no further calls after a rejected key");
        // All three jobs are pending again (none left 'running'), attempts untouched.
        let later = store.claim_thread_priority_jobs(10, i64::MAX / 2).unwrap();
        assert_eq!(later.len(), 3);
        assert!(later.iter().all(|j| j.attempts == 0));
    }

    #[test]
    fn rate_limits_back_off_five_minutes_and_other_errors_count_an_attempt() {
        let store = store_with_waiting_threads(2, "hi");
        let scorer = FakeScorer::new(vec![
            Err(llm::LlmError::Http { status: 429, body: String::new() }),
            Err(llm::LlmError::Provider("garbled".into())),
        ]);

        run_thread_priority_batch_if(&store, &scorer, 2, true).unwrap();

        let soon = store.claim_thread_priority_jobs(10, 5_000 + 60_000).unwrap();
        assert!(soon.is_empty(), "both are backing off");
        let later = store.claim_thread_priority_jobs(10, i64::MAX / 2).unwrap();
        let mut attempts: Vec<i64> = later.iter().map(|j| j.attempts).collect();
        attempts.sort();
        assert_eq!(attempts, [0, 1]); // 429 kept 0; the garbled answer cost one
    }

    #[test]
    fn a_panicking_scorer_counts_as_a_failed_attempt_instead_of_unwinding() {
        let store = store_with_waiting_threads(1, "hi");

        let outcome = run_thread_priority_batch_if(&store, &PanickingScorer, 1, true).unwrap();

        assert_eq!((outcome.processed, outcome.auth_rejected), (0, false));
        let later = store.claim_thread_priority_jobs(10, i64::MAX / 2).unwrap();
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].attempts, 1);
    }

    /// Limited Use: Gmail text is not sent to TypeSafe until TypeSafe confirms its terms;
    /// the jobs stay pending with no attempt counted.
    #[test]
    fn gmail_threads_are_not_sent_to_typesafe_until_it_confirms_its_terms() {
        assert!(!memory::limited_use::TYPESAFE_GOOGLE_DATA_CONFIRMED);
        let store = store_with_waiting_threads(2, "please reply");
        let outcome = run_thread_priority_batch(&store, &PanickingScorer, 5).unwrap();
        assert_eq!((outcome.processed, outcome.auth_rejected), (0, false));
        let later = store.claim_thread_priority_jobs(10, i64::MAX / 2).unwrap();
        assert_eq!(later.len(), 2);
        assert!(later.iter().all(|j| j.attempts == 0));
    }

    /// Thread summaries read Gmail: an Ollama cloud model gets none without the switch.
    #[test]
    fn thread_summaries_skip_an_ollama_cloud_model_without_the_switch() {
        let store = store_with_waiting_threads(1, "hi");
        store.run_jobs_batch(10, 5_000).unwrap();
        assert_eq!(run_thread_summarize_batch(&store, "gpt-oss:120b-cloud", 5, "", "").unwrap(), 0);
        assert_eq!(run_thread_summarize_batch_with(&store, "kimi-k2:1t-cloud", 5, "", "", false).unwrap(), 0);
        let pending = store.claim_thread_summarize_jobs(10, i64::MAX / 2).unwrap();
        assert!(!pending.is_empty(), "the jobs stay pending for a local model");
    }

    #[test]
    fn senders_split_into_a_name_and_a_lowercase_domain() {
        assert_eq!(split_sender("\"Future Me, Answered\" <support@FutureMeAnswered.com>"), ("Future Me, Answered".to_string(), "futuremeanswered.com".to_string()));
        assert_eq!(split_sender("Peter van de Put <vandeputp@gmail.com>"), ("Peter van de Put".to_string(), "gmail.com".to_string()));
        assert_eq!(split_sender("bob@x.org"), (String::new(), "x.org".to_string()));
        assert_eq!(split_sender("not an address"), (String::new(), String::new()));
    }
}
