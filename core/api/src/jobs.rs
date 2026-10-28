//! The LLM-backed background batches `core/memory` can't run itself (it has no `llm`
//! dependency): episode summaries (docs/superpowers/specs/2026-08-12-l2-episodes-design.md)
//! and incremental thread summaries (docs/superpowers/specs/2026-08-14-thread-distillation-design.md).
//! Moved from `core/ffi::whimbrel_episodes_run_summarize_batch` /
//! `whimbrel_threads_run_summarize_batch` (M3.0). Both run against a local Ollama
//! `model` only — these are unattended jobs, never routed to a hosted provider.

use crate::util::now_unix_ms;

/// Runs up to `max_jobs` pending `episode_summarize` jobs. `custom_instructions` is
/// trimmed; whitespace-only means "no instructions". Returns the count actually
/// processed (`0` means nothing was eligible); `Err` only if the claim itself failed.
pub fn run_episode_summarize_batch(
    store: &memory::Store,
    ollama_model: &str,
    max_jobs: usize,
    custom_instructions: &str,
) -> rusqlite::Result<usize> {
    let custom_instructions = custom_instructions.trim();
    let jobs = store.claim_episode_summarize_jobs(max_jobs, now_unix_ms())?;
    let client = llm::OllamaClient::new(ollama_model);
    let mut processed = 0;
    for job in jobs {
        // No model for episodes with nothing to summarize: their captures are gone
        // (retention, dedupe), or it was one glance.
        match skip_reason(&job.items) {
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
        match llm::summarize_episode(&client, &job.items, custom_instructions) {
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

/// A glance: at most one capture, spanning under this long.
const GLANCE_MAX_MS: i64 = 30_000;

enum Skip {
    Empty,
    Glance(String),
}

/// Why an episode needs no model call, if it doesn't. On real data, 1,015 of 1,501
/// failed summaries had no captures left, and nearly every "no meaningful activity"
/// summary came from an empty or one-capture episode.
fn skip_reason(items: &[memory::CaptureItem]) -> Option<Skip> {
    let (Some(first), Some(last)) = (items.first(), items.last()) else {
        return Some(Skip::Empty);
    };
    if items.len() <= 1 && last.ts_unix_ms - first.ts_unix_ms < GLANCE_MAX_MS {
        let app = first.app_bundle_id.rsplit('.').next().unwrap_or(&first.app_bundle_id);
        let title = first.window_title.trim();
        return Some(Skip::Glance(if title.is_empty() {
            format!("Briefly viewed {app}.")
        } else {
            format!("Briefly viewed \"{title}\" in {app}.")
        }));
    }
    None
}

/// Runs up to `max_jobs` pending `thread_summarize` jobs, mirroring
/// [`run_episode_summarize_batch`] exactly.
pub fn run_thread_summarize_batch(
    store: &memory::Store,
    ollama_model: &str,
    max_jobs: usize,
    custom_instructions: &str,
) -> rusqlite::Result<usize> {
    let custom_instructions = custom_instructions.trim();
    let jobs = store.claim_thread_summarize_jobs(max_jobs, now_unix_ms())?;
    let client = llm::OllamaClient::new(ollama_model);
    let mut processed = 0;
    for job in jobs {
        let now = now_unix_ms();
        match llm::summarize_thread_incrementally(
            &client,
            job.prior_summary.as_deref(),
            &job.new_messages,
            custom_instructions,
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
pub fn run_thread_priority_batch(
    store: &memory::Store,
    scorer: &dyn llm::typesafe::PriorityScorer,
    max_jobs: usize,
) -> rusqlite::Result<PriorityBatchOutcome> {
    let mut outcome = PriorityBatchOutcome { processed: 0, auth_rejected: false };
    let mut jobs = store.claim_thread_priority_jobs(max_jobs, now_unix_ms())?.into_iter();
    while let Some(job) = jobs.next() {
        let now = now_unix_ms();
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
    fn empty_and_glance_episodes_skip_the_model_and_real_ones_do_not() {
        assert!(matches!(skip_reason(&[]), Some(Skip::Empty)));
        match skip_reason(&[capture(1_000, "zsh")]) {
            Some(Skip::Glance(s)) => assert_eq!(s, "Briefly viewed \"zsh\" in Terminal."),
            _ => panic!("one capture is a glance"),
        }
        assert!(skip_reason(&[capture(1_000, "a"), capture(2_000, "b")]).is_none(), "two captures get a real summary");
    }

    /// Mirrors `core/ffi`'s `*_returns_zero_when_nothing_pending` tests: no jobs means
    /// no Ollama call, so this is deterministic without a server.
    #[test]
    fn both_batches_return_zero_when_nothing_is_pending() {
        let store = memory::Store::open_in_memory().unwrap();
        assert_eq!(
            run_episode_summarize_batch(&store, "gemma4:12b", 5, "   ").unwrap(),
            0
        );
        assert_eq!(
            run_thread_summarize_batch(&store, "gemma4:12b", 5, "   ").unwrap(),
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

    #[test]
    fn a_successful_score_is_stored_and_the_job_is_done() {
        let store = store_with_waiting_threads(1, "Please confirm by Friday.");
        let scorer = FakeScorer::new(vec![Ok(judgments())]);

        let outcome = run_thread_priority_batch(&store, &scorer, 5).unwrap();

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

        run_thread_priority_batch(&store, &scorer, 1).unwrap();

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

        run_thread_priority_batch(&store, &scorer, 1).unwrap();

        let sent = scorer.inputs.borrow()[0].latest_message.clone();
        assert!(sent.contains("[REDACTED"), "expected a redaction marker, got {sent:?}");
        assert!(!sent.contains("apikey_"), "raw key prefix leaked: {sent:?}");
    }

    #[test]
    fn network_and_server_errors_retry_without_using_attempts() {
        let store = store_with_waiting_threads(1, "hi");
        let scorer = FakeScorer::new(vec![Err(llm::LlmError::Network("down".into()))]);

        let outcome = run_thread_priority_batch(&store, &scorer, 1).unwrap();

        assert_eq!((outcome.processed, outcome.auth_rejected), (0, false));
        // Not claimable now (30 s backoff), and still pending with zero attempts afterwards.
        assert!(store.claim_thread_priority_jobs(1, 5_000).unwrap().is_empty());
        let later = store.claim_thread_priority_jobs(1, i64::MAX / 2).unwrap();
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].attempts, 0);
    }

    #[test]
    fn a_rejected_key_releases_every_claimed_job_and_flags_the_batch() {
        let store = store_with_waiting_threads(3, "hi");
        let scorer = FakeScorer::new(vec![Err(llm::LlmError::Http { status: 401, body: "bad key".into() })]);

        let outcome = run_thread_priority_batch(&store, &scorer, 3).unwrap();

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

        run_thread_priority_batch(&store, &scorer, 2).unwrap();

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

        let outcome = run_thread_priority_batch(&store, &PanickingScorer, 1).unwrap();

        assert_eq!((outcome.processed, outcome.auth_rejected), (0, false));
        let later = store.claim_thread_priority_jobs(10, i64::MAX / 2).unwrap();
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].attempts, 1);
    }

    #[test]
    fn senders_split_into_a_name_and_a_lowercase_domain() {
        assert_eq!(split_sender("\"Future Me, Answered\" <support@FutureMeAnswered.com>"), ("Future Me, Answered".to_string(), "futuremeanswered.com".to_string()));
        assert_eq!(split_sender("Peter van de Put <vandeputp@gmail.com>"), ("Peter van de Put".to_string(), "gmail.com".to_string()));
        assert_eq!(split_sender("bob@x.org"), (String::new(), "x.org".to_string()));
        assert_eq!(split_sender("not an address"), (String::new(), String::new()));
    }
}
