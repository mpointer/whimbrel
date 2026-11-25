//! The one-time history backfill (`whimbrel-cloud-runner backfill`): the runner signs in as `run_once` does, then
//! walks the whole relay once, tagging every `items` row with its `kind_tag` (`sync_engine::runner_kind_backfill`)
//! so later pulls can skip the kinds a routine does not read, then walks it again for `slice_tag`
//! (`sync_engine::runner_slice_backfill_observed`) so they can skip Google Calendar events and old months too. Both
//! passes share one time budget. It decrypts item payloads in memory to compute a tag and writes only the tag back;
//! nothing is written to disk and nothing from a payload is logged or reported.

use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use sync_engine::{BackfillStop, KindBackfillProgress};

use crate::{RunnerConfig, RunnerError};

/// How long one backfill step runs before the progress line: about a minute, so the line shows once a minute and
/// the runner refreshes its claim (which holds the Mac and Windows apps off) that often.
const STEP_BUDGET: Duration = Duration::from_secs(60);

/// What one backfill run did: where the kind pass stands (`done` and `total` count `local_seq` positions over every
/// device's rows, not rows), whether it reached the end, the rows left untagged because they would not open or
/// decode, its steps, how long the whole run took, and where the slice pass stands (`slice_done`, `slice_total`,
/// `slice_stopped`). Counts only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillReport {
    pub done: u64,
    pub total: u64,
    pub finished: bool,
    pub undecryptable: u64,
    pub steps: usize,
    pub duration_ms: i64,
    /// Why the kind pass stopped: `finished`, `budget` or `claim_held`, or `skipped` (`backfill --slice-only`, its
    /// counts then zero).
    pub stopped: &'static str,
    /// The slice pass's walk, in the same positions as `done` and `total`.
    pub slice_done: u64,
    pub slice_total: u64,
    /// Why the slice pass stopped: `finished`, `budget` (also when too little time was left to start it) or
    /// `claim_held`.
    pub slice_stopped: &'static str,
}

impl BackfillReport {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "done": self.done,
            "total": self.total,
            "finished": self.finished,
            "undecryptable": self.undecryptable,
            "steps": self.steps,
            "duration_ms": self.duration_ms,
            "stopped": self.stopped,
            "slice_done": self.slice_done,
            "slice_total": self.slice_total,
            "slice_stopped": self.slice_stopped,
        })
        .to_string()
    }

    fn from_progress(p: &KindBackfillProgress, stop: BackfillStop, steps: usize, duration_ms: i64) -> Self {
        Self {
            done: p.done,
            total: p.total,
            finished: p.finished,
            undecryptable: p.undecryptable,
            steps,
            duration_ms,
            stopped: stop_name(stop),
            slice_done: 0,
            slice_total: 0,
            slice_stopped: stop_name(BackfillStop::Budget),
        }
    }

    /// The report with the slice pass's result.
    fn with_slice(self, p: &KindBackfillProgress, stop: BackfillStop) -> Self {
        Self { slice_done: p.done, slice_total: p.total, slice_stopped: stop_name(stop), ..self }
    }
}

fn stop_name(stop: BackfillStop) -> &'static str {
    match stop {
        BackfillStop::Finished => "finished",
        BackfillStop::Budget => "budget",
        BackfillStop::ClaimHeld => "claim_held",
        BackfillStop::Skipped => "skipped",
    }
}

/// What is left of the run's `total_budget` for the slice pass after `elapsed` (sign-in and the kind pass), or
/// `None` when too little is left to start a step (`sync_engine::BACKFILL_MIN_STEP_REMAINING`).
fn slice_budget(total_budget: Duration, elapsed: Duration) -> Option<Duration> {
    let left = total_budget.saturating_sub(elapsed);
    (left >= sync_engine::BACKFILL_MIN_STEP_REMAINING).then_some(left)
}

/// One backfill with the host's secrets, for at most about `total_budget` over both passes: opens the provision
/// bundle, signs in to the Worker as the runner, then runs the passes ([`run_passes`]) into a scratch store in
/// memory: the kind pass and then the slice pass with what is left of the budget, or with `slice_only` the slice
/// pass alone with all of it (the store is new each run, so a second run would otherwise walk the whole kind pass
/// again first). Prints one stderr line (`backfill: <done>/<total> positions`, then `backfill slice: ...`) after
/// each step of about a minute. Safe to run again: a new run walks from the start and skips the rows already tagged.
pub fn backfill_once(cfg: &RunnerConfig, total_budget: Duration, slice_only: bool) -> Result<BackfillReport, RunnerError> {
    let started = Instant::now();
    let runner = sync::DeviceKeypair::from_bytes(cfg.device_secret_key);
    let device_id = STANDARD.encode(runner.public_key_bytes());
    let (group_key, token) =
        api::sync_ops::runner::open_provision_bundle(&cfg.provision, &runner, &cfg.approver_public_key).map_err(|_| RunnerError::Provision)?;
    let session = api::sync_ops::runner::runner_authenticate(&cfg.worker_url, &device_id, &token).map_err(|e| match e {
        api::sync_ops::enrollment::WorkerError::Http { status: 401 | 403, .. } => RunnerError::Unauthorized,
        _ => RunnerError::Auth,
    })?;
    drop(token);
    let store = memory::Store::open_in_memory().map_err(|_| RunnerError::Store)?;
    run_passes(
        total_budget,
        slice_only,
        started,
        |budget| {
            // `steps` is only set inside `on_step`; at least one step always runs before an `Ok`, so it is never stale.
            let mut steps = 0;
            sync_engine::runner_kind_backfill_observed(
                &session.turso_url,
                &session.turso_token,
                &group_key,
                &store,
                &device_id,
                budget,
                STEP_BUDGET,
                &mut |p, n| {
                    steps = n;
                    eprintln!("backfill: {}/{} positions", p.done, p.total);
                },
            )
            .map(|(p, stop)| (p, stop, steps))
            .map_err(|_| RunnerError::Backfill)
        },
        |budget| {
            sync_engine::runner_slice_backfill_observed(
                &session.turso_url,
                &session.turso_token,
                &group_key,
                &store,
                &device_id,
                budget,
                STEP_BUDGET,
                &mut |p, _| eprintln!("backfill slice: {}/{} positions", p.done, p.total),
            )
            .map_err(|_| RunnerError::Backfill)
        },
    )
}

/// The passes of one backfill run that `started` then, under one `total_budget`: `kind` (unless `slice_only`, which
/// reports it as `skipped` with zeros) with the whole budget and returning its steps, then `slice` with what is left
/// (not run with under `BACKFILL_MIN_STEP_REMAINING` left: reported as `budget`, nothing walked). A kind pass that
/// fails ends the run with its error and the slice pass never runs; one that stops on its budget or another
/// device's claim does not.
fn run_passes(
    total_budget: Duration,
    slice_only: bool,
    started: Instant,
    kind: impl FnOnce(Duration) -> Result<(KindBackfillProgress, BackfillStop, usize), RunnerError>,
    slice: impl FnOnce(Duration) -> Result<(KindBackfillProgress, BackfillStop), RunnerError>,
) -> Result<BackfillReport, RunnerError> {
    let report = if slice_only {
        BackfillReport::from_progress(&KindBackfillProgress::default(), BackfillStop::Skipped, 0, 0)
    } else {
        let (progress, stop, steps) = kind(total_budget)?;
        BackfillReport::from_progress(&progress, stop, steps, 0)
    };
    let report = match slice_budget(total_budget, started.elapsed()) {
        Some(left) => {
            let (p, stop) = slice(left)?;
            report.with_slice(&p, stop)
        }
        // No time left to start it: reported as stopped by the budget, nothing walked.
        None => report,
    };
    Ok(BackfillReport { duration_ms: started.elapsed().as_millis() as i64, ..report })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_json_has_the_expected_keys_and_only_counts() {
        let report = BackfillReport {
            done: 18,
            total: 98,
            finished: false,
            undecryptable: 3,
            steps: 7,
            duration_ms: 4200,
            stopped: "claim_held",
            slice_done: 40,
            slice_total: 98,
            slice_stopped: "budget",
        };
        let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
        let object = value.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["done", "duration_ms", "finished", "slice_done", "slice_stopped", "slice_total", "steps", "stopped", "total", "undecryptable"]
        );
        assert_eq!(object["done"], 18);
        assert_eq!(object["total"], 98);
        assert_eq!(object["finished"], false);
        assert_eq!(object["undecryptable"], 3);
        assert_eq!(object["steps"], 7);
        assert_eq!(object["duration_ms"], 4200);
        assert_eq!(object["stopped"], "claim_held");
        assert_eq!(object["slice_done"], 40);
        assert_eq!(object["slice_total"], 98);
        assert_eq!(object["slice_stopped"], "budget");
        let stop_key = |k: &str| k == "stopped" || k == "slice_stopped";
        assert!(object.iter().all(|(k, v)| v.is_number() || v.is_boolean() || (stop_key(k) && matches!(v.as_str(), Some("finished" | "budget" | "claim_held" | "skipped")))), "no text, so no key, token, URL or payload");
    }

    #[test]
    fn a_finished_progress_becomes_a_finished_report() {
        let p = KindBackfillProgress { done: 10, total: 10, finished: true, undecryptable: 1 };
        let report = BackfillReport::from_progress(&p, BackfillStop::Finished, 2, 5);
        assert_eq!(
            (report.done, report.total, report.finished, report.undecryptable, report.steps, report.duration_ms, report.stopped),
            (10, 10, true, 1, 2, 5, "finished")
        );
        // Until the slice pass reports, it stands at nothing walked, stopped by the budget.
        assert_eq!((report.slice_done, report.slice_total, report.slice_stopped), (0, 0, "budget"));
    }

    /// The slice pass adds its own result and leaves the kind pass's fields as they were.
    #[test]
    fn the_slice_result_is_added_without_touching_the_kind_fields() {
        let kind = KindBackfillProgress { done: 7, total: 10, finished: false, undecryptable: 2 };
        let before = BackfillReport::from_progress(&kind, BackfillStop::ClaimHeld, 3, 9);
        let slice = KindBackfillProgress { done: 10, total: 10, finished: true, undecryptable: 0 };
        let after = before.with_slice(&slice, BackfillStop::Finished);
        assert_eq!((after.slice_done, after.slice_total, after.slice_stopped), (10, 10, "finished"));
        assert_eq!(BackfillReport { slice_done: 0, slice_total: 0, slice_stopped: "budget", ..after }, before);
        let held = before.with_slice(&KindBackfillProgress { done: 0, total: 10, ..Default::default() }, BackfillStop::ClaimHeld);
        assert_eq!(held.slice_stopped, "claim_held");
    }

    /// One budget covers both passes: the slice pass gets what the kind pass left, and none under 10 s.
    #[test]
    fn the_slice_pass_gets_what_is_left_of_the_budget() {
        let min = Duration::from_secs(60);
        assert_eq!(slice_budget(50 * min, 20 * min), Some(30 * min));
        assert_eq!(slice_budget(50 * min, 50 * min - Duration::from_secs(10)), Some(Duration::from_secs(10)));
        assert_eq!(slice_budget(50 * min, 50 * min - Duration::from_secs(9)), None);
        assert_eq!(slice_budget(50 * min, 60 * min), None, "an overrun kind pass leaves nothing");
    }

    #[test]
    fn the_backfill_error_is_a_fixed_sentence() {
        assert_eq!(RunnerError::Backfill.to_string(), "the history backfill failed");
    }

    fn progress(done: u64, total: u64, finished: bool) -> KindBackfillProgress {
        KindBackfillProgress { done, total, finished, undecryptable: 0 }
    }

    /// The kind pass runs first with the whole budget, then the slice pass with what is left.
    #[test]
    fn the_kind_pass_runs_before_the_slice_pass() {
        let order = std::cell::RefCell::new(Vec::new());
        let total = Duration::from_secs(3000);
        let report = run_passes(
            total,
            false,
            Instant::now(),
            |budget| {
                order.borrow_mut().push(("kind", budget));
                Ok((progress(5, 10, false), BackfillStop::ClaimHeld, 2))
            },
            |budget| {
                order.borrow_mut().push(("slice", budget));
                Ok((progress(10, 10, true), BackfillStop::Finished))
            },
        )
        .unwrap();
        let order = order.into_inner();
        assert_eq!(order.iter().map(|(p, _)| *p).collect::<Vec<_>>(), ["kind", "slice"]);
        assert_eq!(order[0].1, total);
        assert!(order[1].1 <= total && order[1].1 > total - Duration::from_secs(5), "what is left: {:?}", order[1].1);
        assert_eq!((report.done, report.total, report.steps, report.stopped), (5, 10, 2, "claim_held"));
        assert_eq!((report.slice_done, report.slice_total, report.slice_stopped), (10, 10, "finished"));
    }

    /// A kind pass that fails ends the run with an error (exit 1) and the slice pass never runs.
    #[test]
    fn a_failing_kind_pass_stops_the_run_before_the_slice_pass() {
        let mut slice_ran = false;
        let result = run_passes(
            Duration::from_secs(3000),
            false,
            Instant::now(),
            |_| Err(RunnerError::Backfill),
            |_| {
                slice_ran = true;
                Ok((progress(1, 1, true), BackfillStop::Finished))
            },
        );
        assert!(matches!(result, Err(RunnerError::Backfill)));
        assert!(!slice_ran);
    }

    /// `--slice-only`: the kind pass never runs and reports `skipped` with zeros; the slice pass gets the budget.
    #[test]
    fn slice_only_skips_the_kind_pass_and_gives_the_slice_pass_the_budget() {
        let total = Duration::from_secs(3000);
        let mut slice_budget_seen = None;
        let report = run_passes(
            total,
            true,
            Instant::now(),
            |_| panic!("the kind pass must not run"),
            |budget| {
                slice_budget_seen = Some(budget);
                Ok((progress(4, 9, false), BackfillStop::Budget))
            },
        )
        .unwrap();
        let seen = slice_budget_seen.unwrap();
        assert!(seen <= total && seen > total - Duration::from_secs(5), "{seen:?}");
        assert_eq!(
            (report.done, report.total, report.finished, report.undecryptable, report.steps, report.stopped),
            (0, 0, false, 0, 0, "skipped")
        );
        assert_eq!((report.slice_done, report.slice_total, report.slice_stopped), (4, 9, "budget"));
        let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
        assert_eq!(value["stopped"], "skipped");
    }

    /// A kind pass that spent the budget leaves no slice pass: it reports `budget`, nothing walked.
    #[test]
    fn no_slice_pass_starts_when_the_kind_pass_spent_the_budget() {
        let total = Duration::from_secs(60);
        let started = Instant::now().checked_sub(Duration::from_secs(55)).unwrap();
        let report = run_passes(
            total,
            false,
            started,
            |_| Ok((progress(3, 10, false), BackfillStop::Budget, 1)),
            |_| panic!("no time left for the slice pass"),
        )
        .unwrap();
        assert_eq!(report.stopped, "budget");
        assert_eq!((report.slice_done, report.slice_total, report.slice_stopped), (0, 0, "budget"));
    }
}
