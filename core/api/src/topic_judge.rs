//! Jev judging: sends queued ambiguous sources to TypeSafe as choice questions over the
//! seed topics, records the verdicts, and stops at the token budget.

use llm::typesafe::{TopicJudge, TopicJudgeInput, TopicJudgment};

use crate::util::{now_unix_ms, uuid_like_trace_id};

/// A choice below this probability records "none of these".
const ASSIGN_AT: f64 = 0.6;
/// Subject topics (ML, Rust) are broad, so Jev must be surer before filing under one.
const SUBJECT_ASSIGN_AT: f64 = 0.8;
/// Failed calls in a row (other than auth or transient) that end a batch.
const MAX_FAILURES_IN_A_ROW: usize = 5;

/// What one `run_topic_judge_batch` call did.
#[derive(Debug, Default)]
pub struct TopicJudgeOutcome {
    pub processed: usize,
    /// TypeSafe rejected the API key (401/403): the app should show "key rejected".
    pub auth_rejected: bool,
    /// The first pass's tokens reached the budget set by `start_topic_judging`.
    pub over_budget: bool,
    /// After the first pass: today's `DAILY_ALLOWANCE_TOKENS` are used up; judging resumes
    /// on its own tomorrow (UTC).
    pub paused_for_today: bool,
    /// `start_topic_judging` was never called on this database (or it was replaced).
    pub not_started: bool,
}

/// A panic inside the client must not unwind through the `extern "C"` caller.
fn judge_catching_panics(
    judge: &dyn TopicJudge,
    input: &TopicJudgeInput,
) -> Result<TopicJudgment, llm::LlmError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| judge.judge(input))).unwrap_or_else(
        |_| {
            Err(llm::LlmError::Provider(
                "TypeSafe client panicked".to_string(),
            ))
        },
    )
}

/// The topic `t{n}` names when it is the likeliest label at `ASSIGN_AT` or above.
fn chosen_topic(
    j: &TopicJudgment,
    topics: &[(i64, String, String)],
    subjects: &[i64],
) -> Option<i64> {
    let (label, p) = j.probabilities.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    let n: usize = label.strip_prefix('t')?.parse().ok()?;
    let id = topics.get(n.checked_sub(1)?)?.0;
    let bar = if subjects.contains(&id) {
        SUBJECT_ASSIGN_AT
    } else {
        ASSIGN_AT
    };
    (*p >= bar).then_some(id)
}

/// Judges up to `max` queued sources. Does nothing before `start_topic_judging`. Until the
/// queue first drains it stops once the tokens used reach the budget set there (the caller
/// set 1.5 × the estimate); after that it stops for the day at `DAILY_ALLOWANCE_TOKENS`.
/// A rejected key or a transient failure stops the batch and leaves the source queued; any
/// other failure is charged at the estimate and counts against the source, which leaves
/// the queue unjudged on its third. Five such failures in a row end the batch.
pub fn run_topic_judge_batch(
    store: &memory::Store,
    judge: &dyn TopicJudge,
    max: usize,
    usd_per_million: f64,
) -> rusqlite::Result<TopicJudgeOutcome> {
    use memory::topics::judge::{judge_request_tokens, JudgeGate};
    let mut out = TopicJudgeOutcome::default();
    if store.topic_judge_gate()? == JudgeGate::NotStarted {
        out.not_started = true;
        return Ok(out);
    }
    let mut failures = 0;
    for c in store.next_judge_candidates(max)? {
        match store.topic_judge_gate()? {
            JudgeGate::Open => {}
            JudgeGate::OverBudget => {
                out.over_budget = true;
                break;
            }
            JudgeGate::PausedForToday => {
                out.paused_for_today = true;
                break;
            }
            JudgeGate::NotStarted => break,
        }
        let input = TopicJudgeInput {
            ask_work: c.kind == "episode",
            kind: c.kind,
            text: memory::redact::redact_secrets(&c.text).into_owned(),
            labels: c
                .topics
                .iter()
                .enumerate()
                .map(|(i, (_, name, also))| {
                    let d = if also.is_empty() {
                        name.clone()
                    } else {
                        format!("{name} (also: {also})")
                    };
                    (format!("t{}", i + 1), d)
                })
                .collect(),
        };
        let now = now_unix_ms();
        match judge_catching_panics(judge, &input) {
            Ok(j) => {
                if j.usage.is_none() {
                    // No usage block: count the estimate so the budget stop still holds.
                    store
                        .add_topic_judge_used_tokens(judge_request_tokens(&input.kind, &c.text))?;
                }
                if let Some(u) = j.usage {
                    let (i, o) = (
                        u.input_tokens.map(i64::from),
                        u.output_tokens.map(i64::from),
                    );
                    let tokens = i.unwrap_or(0) + o.unwrap_or(0);
                    store.add_topic_judge_used_tokens(tokens)?;
                    let _ = store.log_usage(&memory::NewUsageLogEntry {
                        user_id: None,
                        provider: "typesafe".into(),
                        model: j.model.clone(),
                        route: Some("topic_judge".into()),
                        input_tokens: i,
                        output_tokens: o,
                        estimated_cost_cents: store
                            .topic_judge_cost_cents(tokens, usd_per_million)?,
                        cache_hit: false,
                        trace_id: uuid_like_trace_id(now),
                        ts_unix_ms: now,
                    });
                }
                let work = if input.ask_work { j.work } else { None };
                store.record_topic_verdict(
                    c.source_id,
                    chosen_topic(&j, &c.topics, &c.subjects),
                    work,
                    &j.model,
                    now,
                )?;
                out.processed += 1;
                failures = 0;
            }
            Err(e) => {
                store.record_summarize_error("topic_judge", &e.to_string(), now);
                match e {
                    llm::LlmError::Http {
                        status: 401 | 403, ..
                    } => {
                        out.auth_rejected = true;
                        break;
                    }
                    llm::LlmError::Http { status, .. } if status == 429 || status >= 500 => break,
                    llm::LlmError::Network(_) => break,
                    // Says something about this source: the third such failure drops it.
                    // The call may still have been billed, so it counts at the estimate.
                    _ => {
                        store.add_topic_judge_used_tokens(judge_request_tokens(
                            &input.kind,
                            &c.text,
                        ))?;
                        store.record_topic_judge_failure(c.source_id)?;
                        failures += 1;
                        if failures >= MAX_FAILURES_IN_A_ROW {
                            break;
                        }
                    }
                }
            }
        }
    }
    store.end_topic_judge_first_pass_if_drained()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        seen: RefCell<Vec<TopicJudgeInput>>,
        reply: Box<dyn Fn(&TopicJudgeInput) -> Result<TopicJudgment, llm::LlmError>>,
    }

    impl TopicJudge for Fake {
        fn judge(&self, input: &TopicJudgeInput) -> Result<TopicJudgment, llm::LlmError> {
            self.seen.borrow_mut().push(input.clone());
            (self.reply)(input)
        }
    }

    #[test]
    fn subject_topics_need_a_surer_pick() {
        let j = |p: f64| TopicJudgment {
            probabilities: vec![("t1".into(), p), ("none".into(), 1.0 - p)],
            work: None,
            model: "m".into(),
            usage: None,
        };
        let topics = vec![(7, "ML".to_string(), String::new())];
        assert_eq!(chosen_topic(&j(0.7), &topics, &[7]), None);
        assert_eq!(chosen_topic(&j(0.85), &topics, &[7]), Some(7));
        assert_eq!(chosen_topic(&j(0.7), &topics, &[]), Some(7));
    }

    fn fake(probs: &[(&str, f64)], work: Option<f64>, tokens: u32) -> Fake {
        let probs: Vec<(String, f64)> = probs.iter().map(|(l, p)| (l.to_string(), *p)).collect();
        Fake {
            seen: RefCell::new(Vec::new()),
            reply: Box::new(move |_| {
                Ok(TopicJudgment {
                    probabilities: probs.clone(),
                    work,
                    model: "jev-test".into(),
                    usage: Some(llm::TokenUsage {
                        input_tokens: Some(tokens - tokens / 3),
                        output_tokens: Some(tokens / 3),
                    }),
                })
            }),
        }
    }

    fn failing(e: fn() -> llm::LlmError) -> Fake {
        Fake {
            seen: RefCell::new(Vec::new()),
            reply: Box::new(move |_| Err(e())),
        }
    }

    /// A file-backed store with one seed per name and one email per subject, indexed and
    /// filed, plus a second connection for assertions.
    fn setup(
        seeds: &[&str],
        subjects: &[&str],
    ) -> (
        tempfile::TempDir,
        memory::Store,
        rusqlite::Connection,
        Vec<i64>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let store = memory::Store::open(&path, dir.path()).unwrap();
        let c = rusqlite::Connection::open(&path).unwrap();
        let ids = seeds
            .iter()
            .map(|n| store.create_seed_topic(n, &[], None).unwrap().topic_id)
            .collect();
        for (i, subject) in subjects.iter().enumerate() {
            c.execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                 VALUES ('d','email','gmail',?1,json_array('a@x.com'),?2,?3,'{}',?4)",
                rusqlite::params![1000 + i as i64, subject, format!("{subject} body"), format!("gm-{i}")],
            )
            .unwrap();
        }
        while memory::topics::index::run_topic_index_job(&store, 200).unwrap() {}
        while memory::topics::cluster::run_topic_cluster_job(&store, 200).unwrap() {}
        (dir, store, c, ids)
    }

    fn n(c: &rusqlite::Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn nothing_runs_before_start() {
        let (_d, s, c, _) = setup(&["Maximus"], &["Lunch plans"]);
        let j = fake(&[("t1", 0.9), ("none", 0.1)], None, 100);
        let out = run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        assert_eq!(out.processed, 0);
        assert!(
            out.not_started,
            "no budget in the database: the app shows Start again"
        );
        assert!(j.seen.borrow().is_empty());
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 1);
    }

    #[test]
    fn confident_choice_assigns_a_jev_member() {
        let (_d, s, c, ids) = setup(&["Maximus"], &["Lunch plans"]);
        s.start_topic_judging(10_000).unwrap();
        let j = fake(&[("t1", 0.7), ("none", 0.3)], None, 90);
        let out = run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        assert_eq!(out.processed, 1);
        let seen = j.seen.borrow();
        assert_eq!(
            seen[0].labels,
            vec![("t1".to_string(), "Maximus".to_string())]
        );
        assert!(!seen[0].ask_work, "work is only asked for episodes");
        assert_eq!(
            n(
                &c,
                &format!(
                    "SELECT COUNT(*) FROM topic_member WHERE topic_id = {} AND origin = 'jev'",
                    ids[0]
                )
            ),
            1
        );
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 0);
        let (key, model): (Option<String>, String) = c
            .query_row(
                "SELECT topic_key, model FROM topic_verdict WHERE source_key = 'email:gm-0'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(key.is_some());
        assert_eq!(model, "jev-test");
        assert_eq!(s.topic_judge_budget().unwrap(), Some((10_000, 90)));
        assert_eq!(
            n(&c, "SELECT COUNT(*) FROM usage_log WHERE provider = 'typesafe' AND route = 'topic_judge' AND model = 'jev-test' \
                   AND input_tokens = 60 AND output_tokens = 30 AND estimated_cost_cents = 0"),
            1
        );
    }

    #[test]
    fn low_confidence_records_a_none_verdict_and_no_member() {
        let (_d, s, c, _) = setup(&["Maximus"], &["Lunch plans"]);
        s.start_topic_judging(10_000).unwrap();
        let j = fake(&[("t1", 0.5), ("none", 0.3)], None, 90);
        assert_eq!(run_topic_judge_batch(&s, &j, 10, 1.0).unwrap().processed, 1);
        assert_eq!(
            n(&c, "SELECT COUNT(*) FROM topic_member WHERE origin = 'jev'"),
            0
        );
        assert_eq!(
            n(
                &c,
                "SELECT COUNT(*) FROM topic_verdict WHERE topic_key IS NULL"
            ),
            1
        );
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 0);
    }

    #[test]
    fn rejected_key_stops_and_leaves_todo_and_alias_members_intact() {
        let (_d, s, c, _) = setup(
            &["Maximus", "CMS"],
            &["Maximus CMS handoff", "Lunch plans", "Maximus kickoff"],
        );
        s.start_topic_judging(10_000).unwrap();
        assert_eq!(
            n(
                &c,
                "SELECT COUNT(*) FROM topic_member WHERE origin = 'seed'"
            ),
            1
        );
        let j = failing(|| llm::LlmError::Http {
            status: 401,
            body: String::new(),
        });
        let out = run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        assert!(out.auth_rejected);
        assert_eq!(out.processed, 0);
        assert_eq!(j.seen.borrow().len(), 1, "stops at the first rejection");
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 2);
        assert_eq!(
            n(
                &c,
                "SELECT COUNT(*) FROM topic_member WHERE origin = 'seed'"
            ),
            1
        );
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_verdict"), 0);
    }

    #[test]
    fn transient_errors_leave_the_source_queued_and_stop_the_batch() {
        let (_d, s, c, _) = setup(&["Maximus"], &["Lunch plans", "Dinner plans"]);
        s.start_topic_judging(10_000).unwrap();
        let j = failing(|| llm::LlmError::Http {
            status: 503,
            body: String::new(),
        });
        let out = run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        assert!(!out.auth_rejected);
        assert_eq!(j.seen.borrow().len(), 1);
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 2);
        let j = failing(|| llm::LlmError::Provider("bad answer".into()));
        run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        assert_eq!(
            j.seen.borrow().len(),
            2,
            "other errors move on to the next source"
        );
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 2);
        assert_eq!(
            n(
                &c,
                "SELECT COUNT(*) FROM kv_meta WHERE key = 'last_error:topic_judge'"
            ),
            1
        );
    }

    #[test]
    fn repeated_failures_leave_the_queue_on_the_third() {
        let (_d, s, c, _) = setup(&["Maximus"], &["Lunch plans"]);
        s.start_topic_judging(10_000).unwrap();
        let j = failing(|| llm::LlmError::Provider("bad answer".into()));
        for left in [1, 1, 0] {
            run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
            assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), left);
        }
        assert_eq!(j.seen.borrow().len(), 3);
        run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        assert_eq!(j.seen.borrow().len(), 3, "a 4th batch makes no call");
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_verdict"), 0);
    }

    #[test]
    fn stops_when_usage_passes_the_budget() {
        let (_d, s, c, _) = setup(
            &["Maximus"],
            &["Lunch plans", "Dinner plans", "Breakfast plans"],
        );
        s.start_topic_judging(400).unwrap();
        let j = fake(&[("t1", 0.9), ("none", 0.1)], None, 300);
        let first = run_topic_judge_batch(&s, &j, 2, 1.0).unwrap();
        assert_eq!(first.processed, 2);
        assert!(!first.over_budget);
        let second = run_topic_judge_batch(&s, &j, 2, 1.0).unwrap();
        assert!(second.over_budget);
        assert_eq!(second.processed, 0);
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 1);
    }

    /// I5: failed calls are charged, and five failures in a row end the batch.
    #[test]
    fn failed_calls_are_charged_and_five_in_a_row_stop_the_batch() {
        let subjects: Vec<String> = (0..7).map(|i| format!("Plans {i}")).collect();
        let subjects: Vec<&str> = subjects.iter().map(String::as_str).collect();
        let (_d, s, c, _) = setup(&["Maximus"], &subjects);
        s.start_topic_judging(10_000_000).unwrap();
        let j = failing(|| llm::LlmError::Provider("bad answer".into()));
        let out = run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        assert_eq!(out.processed, 0);
        assert_eq!(j.seen.borrow().len(), 5, "stops after 5 failures in a row");
        let (_, used) = s.topic_judge_budget().unwrap().unwrap();
        assert!(used >= 5 * 150, "failed calls count: {used}");
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 7);
    }

    /// I4: once the queue has drained after Start, new sources are judged on a daily
    /// allowance (not the first pass budget) until it runs out, then again the next day.
    #[test]
    fn after_the_first_pass_judging_runs_on_a_daily_allowance() {
        let (_d, s, c, _) = setup(&["Maximus"], &["Lunch plans"]);
        s.start_topic_judging(10_000).unwrap();
        let j = fake(&[("t1", 0.9), ("none", 0.1)], None, 90);
        assert_eq!(run_topic_judge_batch(&s, &j, 10, 1.0).unwrap().processed, 1);
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 0);

        for i in 0..4 {
            c.execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                 VALUES ('d','email','gmail',?1,json_array('a@x.com'),?2,'later','{}',?3)",
                rusqlite::params![5000 + i, format!("Later {i}"), format!("later-{i}")],
            )
            .unwrap();
        }
        while memory::topics::run_topic_batch(&s, 200).unwrap() {}
        let j = fake(&[("t1", 0.9), ("none", 0.1)], None, 200_000);
        let out = run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        assert_eq!(out.processed, 2, "250,000 a day: the second call passes it");
        assert!(out.paused_for_today && !out.over_budget, "{out:?}");
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 2);

        // The next day.
        c.execute(
            "UPDATE kv_meta SET value = CAST(value AS INTEGER) - 1 WHERE key = 'topic_judge_day'",
            [],
        )
        .unwrap();
        let out = run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        assert_eq!(out.processed, 2);
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 0);
    }

    #[test]
    fn missing_usage_counts_the_estimate_against_the_budget() {
        let (_d, s, c, _) = setup(&["Maximus"], &["Lunch plans", "Dinner plans"]);
        s.start_topic_judging(100).unwrap();
        let mut j = fake(&[("t1", 0.9), ("none", 0.1)], None, 0);
        j.reply = Box::new(|_| {
            Ok(TopicJudgment {
                probabilities: vec![("none".into(), 1.0)],
                work: None,
                model: "jev-test".into(),
                usage: None,
            })
        });
        assert_eq!(run_topic_judge_batch(&s, &j, 1, 1.0).unwrap().processed, 1);
        let (_, used) = s.topic_judge_budget().unwrap().unwrap();
        assert!(used >= 150, "estimated tokens are counted: {used}");
        let second = run_topic_judge_batch(&s, &j, 1, 1.0).unwrap();
        assert!(second.over_budget);
        assert_eq!(n(&c, "SELECT COUNT(*) FROM topic_judge_todo"), 1);
    }

    #[test]
    fn noise_episode_verdict_sets_noise() {
        let (_d, s, c, _) = setup(&["Maximus"], &[]);
        c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, start_ts_unix_ms, end_ts_unix_ms, start_item_id, \
             end_item_id, item_count, created_at_unix_ms, updated_at_unix_ms) VALUES ('d','a',0,0,0,0,0,0,0)",
            [],
        )
        .unwrap();
        let ep = c.last_insert_rowid();
        c.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('episode', ?1, 0, 'Scrolling', 0)", [ep]).unwrap();
        let sid = c.last_insert_rowid();
        c.execute(
            "INSERT INTO topic_judge_todo (source_id) VALUES (?1)",
            [sid],
        )
        .unwrap();
        s.start_topic_judging(10_000).unwrap();
        let j = fake(&[("t1", 0.1), ("none", 0.9)], Some(0.2), 90);
        assert_eq!(run_topic_judge_batch(&s, &j, 10, 1.0).unwrap().processed, 1);
        assert!(j.seen.borrow()[0].ask_work);
        assert_eq!(j.seen.borrow()[0].kind, "episode");
        assert_eq!(
            n(
                &c,
                &format!("SELECT noise FROM topic_source WHERE id = {sid}")
            ),
            1
        );
        assert_eq!(
            n(&c, "SELECT COUNT(*) FROM topic_verdict WHERE work = 0.2"),
            1
        );
    }

    #[test]
    fn text_sent_is_redacted() {
        let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH";
        let subject = format!("Lunch plans {secret}");
        let (_d, s, _c, _) = setup(&["Maximus"], &[&subject]);
        s.start_topic_judging(10_000).unwrap();
        let j = fake(&[("t1", 0.1), ("none", 0.9)], None, 90);
        run_topic_judge_batch(&s, &j, 10, 1.0).unwrap();
        let seen = j.seen.borrow();
        assert!(seen[0].text.contains("Lunch plans"));
        assert!(!seen[0].text.contains(secret));
    }
}
