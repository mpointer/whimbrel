//! FR-RTN-1..4: the routine scheduler's tick. Moved from `core/ffi::whimbrel_routines_tick`
//! (M3.0). The other `whimbrel_routines_*` C functions are single-`Store`-call
//! pass-throughs and have no twin here — a Rust shell calls `memory::Store` directly.

use std::collections::{HashMap, HashSet};

use chrono::TimeZone;

use crate::routing::{build_chain_for_routine, RoutineModelPref, RoutingConfig, Transport};

/// Tests: the chain every routine of this thread's [`tick`] runs down (a recording transport), in place of the
/// one built from routing.
#[cfg(test)]
pub(crate) type TestChain = Box<dyn Fn(&memory::Store) -> Vec<llm::ChainLink>>;

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_CHAIN: std::cell::RefCell<Option<TestChain>> = const { std::cell::RefCell::new(None) };
}

/// The routine's chain without over-budget providers; `None` when a spent budget stops the
/// run. A pinned model that made it to the front of the chain does not fall through.
/// `transport`, when set, makes each link's client ([`crate::routing::with_transport`]).
fn routine_chain(
    store: &memory::Store,
    model_pref: &str,
    ollama_model: &str,
    routing: &RoutingConfig,
    now_unix_ms: i64,
    transport: Option<&Transport>,
) -> Option<Vec<llm::ChainLink>> {
    #[cfg(test)]
    if let Some(chain) = TEST_CHAIN.with(|c| c.borrow().as_ref().map(|f| f(store))) {
        return Some(chain);
    }
    let pref = RoutineModelPref::parse(model_pref);
    let chain = build_chain_for_routine(&pref, ollama_model, routing, &crate::exclusion::current_gate(store));
    let chain = match transport {
        Some(t) => crate::routing::with_transport(store, chain, t),
        None => chain,
    };
    let pinned = matches!(&pref, RoutineModelPref::Pinned { provider, .. } if chain.first().is_some_and(|l| l.provider == *provider));
    crate::budget::apply_budgets(store, chain, pinned, now_unix_ms).ok()
}

/// Local midnight of `now_local`'s calendar day in UTC milliseconds, falling back to
/// `now_unix_ms` when the local date can't be resolved (DST gap).
pub(crate) fn local_midnight_ms(now_local: chrono::DateTime<chrono::Local>, now_unix_ms: i64) -> i64 {
    now_local
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|dt| now_local.timezone().from_local_datetime(&dt).single())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(now_unix_ms)
}

/// Who may run routines on this device (docs/superpowers/specs/2026-10-12-routines-safety-net-design.md).
/// `Full`: everything due. `Failover`: another device is the primary, so only the read-only
/// routines that are 3 hours overdue (`routines::failover_eligible`). `Off`: seed only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunGate {
    Off,
    Full,
    Failover,
}

impl RunGate {
    /// The FFI encoding: 0 = Off, 1 = Full, 2 = Failover. Anything else is Off, so a bad value
    /// can never run routines.
    pub fn from_byte(b: u8) -> Self {
        match b {
            1 => RunGate::Full,
            2 => RunGate::Failover,
            _ => RunGate::Off,
        }
    }
}

/// `true` means every due routine runs; `false` means none. Keeps callers that predate the
/// failover state (Windows, older tests) unchanged.
impl From<bool> for RunGate {
    fn from(may_run: bool) -> Self {
        if may_run { RunGate::Full } else { RunGate::Off }
    }
}

/// One scheduler tick, called every ~60 seconds by the shell.
///
/// Seeds any missing built-in routine (Daily Brief enabled at `08:30`; Focus Time, EOD
/// Recap and Weekly Digest disabled — write-capable routines are never silently on),
/// then checks every enabled routine against `routines::is_due` using `now_unix_ms`
/// converted to this machine's local timezone, both for the due-check and for the
/// day/week window bounds. Write steps (calendar event, email draft) are never executed
/// directly: they're persisted `PendingApproval` and gated behind the approval-gate
/// `Store` methods. Returns the runs that actually completed this tick (usually empty).
///
/// `Err` only if the `routine` table itself could not be read, before any routine was
/// considered. A spend-cap rejection, a spent budget or exhausted provider chain for one due routine does NOT fail
/// the tick — that routine is skipped (no `routine_run` row) and is due again next tick.
/// `routing` is only consulted when a routine is actually due; `gmail_connected` /
/// `recipient_email` only by the EOD Recap and Weekly Digest arms. `gate` Off makes this a
/// seed-only call: nothing runs; Failover runs only the read-only routines that are 3 hours overdue.
#[allow(clippy::too_many_arguments)]
pub fn tick(
    store: &memory::Store,
    now_unix_ms: i64,
    ollama_model: &str,
    routing: &RoutingConfig,
    today_events: &[routines::CalendarEventSummary],
    gmail_connected: bool,
    recipient_email: Option<&str>,
    gate: impl Into<RunGate>,
) -> rusqlite::Result<Vec<memory::RoutineRun>> {
    run_due(store, now_unix_ms, ollama_model, routing, today_events, gmail_connected, recipient_email, gate.into(), Scope::Every, None)
}

/// Which routines the cloud runner owns (spec 2026-11-15-cloud-routine-runner-design.md, Amendment 8),
/// read from synced settings by the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloudPolicy {
    /// The templates flagged to run in the cloud, by their stored name (`daily_brief`, `eod_recap`, ...).
    /// A template that is not [`routines::cloud_eligible`] is never treated as flagged.
    pub cloud_templates: Vec<String>,
    /// Whether the runner's heartbeat is under its grace period old.
    pub runner_fresh: bool,
    /// Each template's ready-by time ("HH:MM", from `routine.<template>.readyBy`). Empty means every
    /// routine keeps its schedule.
    pub ready_by: HashMap<String, String>,
}

/// The run time the runner allows for when it starts a routine before its ready-by time. A placeholder
/// until the author-only phase measures the p95 (plan Task 9).
pub const READY_BY_P95_MS: i64 = 180_000;

/// The safety margin added to [`READY_BY_P95_MS`]. A placeholder until Task 9 measures.
pub const READY_BY_MARGIN_MS: i64 = 120_000;

/// The latest local time a device waits until before it stands in for a cloud routine with a ready-by
/// time: a ready-by whose grace would end after it (or tomorrow) stands in here instead, unless the
/// ready-by itself is later, when it stands in at the ready-by.
pub const DEVICE_STAND_IN_CAP_HHMM: &str = "23:30";

impl CloudPolicy {
    pub(crate) fn flags(&self, t: &memory::RoutineTemplate) -> bool {
        routines::cloud_eligible(t) && self.cloud_templates.iter().any(|c| *c == template_name(t))
    }

    /// The ready-by time set for `t`, if any.
    fn ready_by_for(&self, t: &memory::RoutineTemplate) -> Option<&str> {
        self.ready_by.get(&template_name(t)).map(String::as_str)
    }
}

/// Who is calling [`tick_cloud`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerRole {
    /// The cloud runner: no local model, no connectors.
    Runner,
    /// A person's own device.
    Device,
}

/// The stored name of `t`, as its serde form.
fn template_name(t: &memory::RoutineTemplate) -> String {
    serde_json::to_value(t).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The note on a run the runner could not make because the client settings keep it off hosted models
/// (Amendment 7). Not translated yet: `memory::l10n::text` falls back to this English key.
pub const LEFT_FOR_YOUR_COMPUTER: &str = "Left for your computer: your client settings keep this routine off hosted AI.";

/// The `output_json` field marking a run as [`LEFT_FOR_YOUR_COMPUTER`]; its value is why.
pub const LEFT_FOR_COMPUTER_FIELD: &str = "left_for_computer";

/// Whether `run` is the runner's "left for your computer" row: the computer does not count it as
/// that day's run.
pub fn is_left_for_computer(run: &memory::RoutineRun) -> bool {
    run.status == memory::RunStatus::Failed
        && serde_json::from_str::<serde_json::Value>(&run.output_json).is_ok_and(|j| j.get(LEFT_FOR_COMPUTER_FIELD).is_some())
}

/// [`tick`] for an account with the cloud runner.
///
/// On a `Device`, a routine flagged for the cloud is skipped until `routines::FAILOVER_GRACE_MS`
/// past its ready-by time (its scheduled time when it has none; with a ready-by, no later than
/// [`DEVICE_STAND_IN_CAP_HHMM`] and never before the ready-by), fresh runner or stale; after that it
/// runs here if no run is dated today
/// (the runner made nothing: spend cap, no provider, provider down, or no runner at all). A run the
/// runner left for this computer today is not counted as that day's run, so the device takes it back
/// at once. `gate` applies throughout. Unflagged routines behave as in [`tick`]. `runner_fresh` does
/// not change the device's rule today; it is kept for the caller's status display and later use.
///
/// On the `Runner`, only flagged and eligible templates run. One with a ready-by time is due once
/// [`routines::ready_by_start_ms`] (ready-by minus [`READY_BY_P95_MS`] and [`READY_BY_MARGIN_MS`]) has
/// passed today, else at its schedule; the once-a-day check is the same. `gate` is treated as `Full`, and no
/// email draft is made (`gmail_connected` and `recipient_email` are ignored). The chain is the
/// runner's own and passes the same exclusion filter and guard as on a device; when the guard
/// refuses (the request carries an excluded client, or the setting cannot be read) a `Failed` run
/// noted [`LEFT_FOR_YOUR_COMPUTER`] is stored instead, so the person's computer runs it. Meeting
/// prep is event-driven and is never run by either tick.
#[allow(clippy::too_many_arguments)]
pub fn tick_cloud(
    store: &memory::Store,
    now_unix_ms: i64,
    ollama_model: &str,
    routing: &RoutingConfig,
    today_events: &[routines::CalendarEventSummary],
    gmail_connected: bool,
    recipient_email: Option<&str>,
    gate: impl Into<RunGate>,
    policy: &CloudPolicy,
    role: RunnerRole,
) -> rusqlite::Result<Vec<memory::RoutineRun>> {
    tick_cloud_with_transport(store, now_unix_ms, ollama_model, routing, today_events, gmail_connected, recipient_email, gate, policy, role, None)
}

/// [`tick_cloud`] with each chain link's client made by `transport` when it is set
/// ([`crate::routing::with_transport`]): the chain, its budgets and its exclusion gate are the
/// same. The cloud runner passes its own (`None` in production) so its tests need no network.
#[allow(clippy::too_many_arguments)]
pub fn tick_cloud_with_transport(
    store: &memory::Store,
    now_unix_ms: i64,
    ollama_model: &str,
    routing: &RoutingConfig,
    today_events: &[routines::CalendarEventSummary],
    gmail_connected: bool,
    recipient_email: Option<&str>,
    gate: impl Into<RunGate>,
    policy: &CloudPolicy,
    role: RunnerRole,
    transport: Option<&Transport>,
) -> rusqlite::Result<Vec<memory::RoutineRun>> {
    let (gate, gmail_connected, recipient_email) = match role {
        RunnerRole::Runner => (RunGate::Full, false, None),
        RunnerRole::Device => (gate.into(), gmail_connected, recipient_email),
    };
    run_due(store, now_unix_ms, ollama_model, routing, today_events, gmail_connected, recipient_email, gate, Scope::Cloud(policy, role), transport)
}

/// Which routines one tick considers.
#[derive(Clone, Copy)]
enum Scope<'a> {
    /// Every enabled routine ([`tick`]).
    Every,
    /// [`tick_cloud`]'s split between the runner and this device.
    Cloud(&'a CloudPolicy, RunnerRole),
}

/// On the runner, a chain the exclusion guard refused leaves a "left for your computer" row. Other
/// failures, and every failure on a device, skip the routine as before (due again next tick).
fn note_refusal(store: &memory::Store, scope: Scope, routine_id: i64, now_unix_ms: i64, lang: &str, failures: &[llm::ProviderFailureKind]) {
    if !matches!(scope, Scope::Cloud(_, RunnerRole::Runner)) {
        return;
    }
    let why = if failures.contains(&llm::ProviderFailureKind::ExclusionUnreadable) {
        "exclusion_unreadable"
    } else if failures.contains(&llm::ProviderFailureKind::ExcludedClient) {
        "excluded_client"
    } else {
        return;
    };
    let _ = store.insert_routine_run(&memory::NewRoutineRun {
        routine_id,
        ran_at_unix_ms: now_unix_ms,
        status: memory::RunStatus::Failed,
        output_summary: memory::l10n::text(lang, LEFT_FOR_YOUR_COMPUTER, &[]),
        output_json: serde_json::json!({ LEFT_FOR_COMPUTER_FIELD: why }).to_string(),
    });
}

#[allow(clippy::too_many_arguments)]
fn run_due(
    store: &memory::Store,
    now_unix_ms: i64,
    ollama_model: &str,
    routing: &RoutingConfig,
    today_events: &[routines::CalendarEventSummary],
    gmail_connected: bool,
    recipient_email: Option<&str>,
    gate: RunGate,
    scope: Scope,
    transport: Option<&Transport>,
) -> rusqlite::Result<Vec<memory::RoutineRun>> {
    store.seed_builtin_routines()?;
    // The primary-device gate (docs/superpowers/specs/2026-10-08-primary-device-routines-design.md):
    // a device that is not the one chosen to run routines seeds them, so pulled runs have a routine to
    // attach to and a later promotion works, but runs nothing -- not even a due or write-capable one.
    if gate == RunGate::Off {
        return Ok(Vec::new());
    }
    // No routine starts while a meeting records (Meeting Prep is a separate entry point and is exempt).
    if memory::background_gate::recording_pauses_background_work() {
        return Ok(Vec::new());
    }
    let all_routines = store.list_routines()?;

    let now_local: chrono::DateTime<chrono::Local> = chrono::Local
        .timestamp_millis_opt(now_unix_ms)
        .single()
        .unwrap_or_else(chrono::Local::now);

    let mut completed_runs: Vec<memory::RoutineRun> = Vec::new();

    // A routine with an outstanding pending_approval row never gets a second WRITE
    // ACTION draft stacked on top of it -- without this, a draft that's failing to
    // approve (silently, or because the user just hasn't gotten to it) piled up a
    // near-identical new one every day it stayed unresolved. Checked per-template below
    // (not as a blanket skip of the whole routine): Focus Time's pending draft IS its
    // entire output, so the routine itself shouldn't re-run while unresolved -- but EOD
    // Recap's pending row is only the auxiliary EMAIL delivery; the recap itself is
    // unconditional, delivered every due day regardless of whether yesterday's email
    // draft was ever acted on.
    let routines_with_pending_approval: HashSet<i64> = store
        .pending_routine_runs()
        .map(|runs| runs.into_iter().map(|r| r.routine_id).collect())
        .unwrap_or_default();

    for routine in all_routines.iter().filter(|r| r.enabled) {
        let Ok(last_run) = store.most_recent_routine_run(routine.id) else {
            continue;
        };
        let failover = gate == RunGate::Failover;
        let mut last_run_counts = true;
        // On the runner, a ready-by time moves the start earlier than the schedule.
        let mut early_start: Option<i64> = None;
        match scope {
            Scope::Every => {}
            Scope::Cloud(policy, RunnerRole::Runner) => {
                if !policy.flags(&routine.template) {
                    continue;
                }
                early_start = policy.ready_by_for(&routine.template).and_then(|hhmm| {
                    routines::ready_by_start_ms(hhmm, now_local, READY_BY_P95_MS, READY_BY_MARGIN_MS)
                });
            }
            Scope::Cloud(policy, RunnerRole::Device) => {
                if policy.flags(&routine.template) {
                    let left_today = last_run.as_ref().is_some_and(|run| {
                        is_left_for_computer(run)
                            && chrono::Local.timestamp_millis_opt(run.ran_at_unix_ms).single().map(|dt| dt.date_naive())
                                == Some(now_local.date_naive())
                    });
                    if left_today {
                        // The runner handed it back today: not today's run here, and no wait.
                        last_run_counts = false;
                    } else {
                        // Fresh or stale, the runner has until the grace has passed, counted from the
                        // ready-by time when one is set; after it, a routine with no run today (the runner
                        // made nothing) runs here. A ready-by whose grace would end after
                        // DEVICE_STAND_IN_CAP_HHMM is capped there, so a late ready-by still gets a stand-in
                        // the same day, but never before the ready-by itself.
                        let ready_by = policy
                            .ready_by_for(&routine.template)
                            .and_then(|hhmm| routines::scheduled_start_unix_ms(hhmm, now_local));
                        let stand_in = match ready_by {
                            Some(ready) => {
                                let after_grace = ready + routines::FAILOVER_GRACE_MS;
                                let capped = routines::scheduled_start_unix_ms(DEVICE_STAND_IN_CAP_HHMM, now_local)
                                    .map_or(after_grace, |cap| after_grace.min(cap));
                                Some(ready.max(capped))
                            }
                            None => routines::scheduled_start_unix_ms(&routine.schedule_hhmm, now_local)
                                .map(|start| start + routines::FAILOVER_GRACE_MS),
                        };
                        let grace_passed = stand_in.is_some_and(|at| now_unix_ms >= at);
                        if !grace_passed {
                            continue;
                        }
                    }
                }
            }
        }
        let last_run_local_date = last_run.filter(|_| last_run_counts).and_then(|run| {
            chrono::Local
                .timestamp_millis_opt(run.ran_at_unix_ms)
                .single()
                .map(|dt| dt.date_naive())
        });
        let due = match early_start {
            // The schedule's time of day is replaced by the early start; the on/off, once-a-day and
            // weekday checks are `is_due`'s.
            Some(start) => {
                let from_midnight = memory::Routine { schedule_hhmm: "00:00".to_string(), ..routine.clone() };
                now_unix_ms >= start && routines::is_due(&from_midnight, now_local, last_run_local_date)
            }
            None => routines::is_due(routine, now_local, last_run_local_date),
        };
        if !due {
            continue;
        }
        if failover && !routines::failover_eligible(routine, now_unix_ms, now_local) {
            continue;
        }

        match routine.template {
            memory::RoutineTemplate::DailyBrief => {
                let yesterday_end = local_midnight_ms(now_local, now_unix_ms);
                let yesterday_start = yesterday_end - 24 * 60 * 60 * 1000;

                let cap_config = routing.spend_cap_config();
                if llm::check_spend_cap(store, &cap_config, None, Some("routine"), now_unix_ms)
                    .is_err()
                {
                    continue;
                }

                let Some(chain) = routine_chain(store, &routine.model_pref, ollama_model, routing, now_unix_ms, transport) else {
                    continue;
                };
                // A chain that can reach a hosted model gets no excluded client's material.
                let filter = crate::exclusion::filter_for_chain(store, &chain);
                // Today's events are stored in the run: an excluded client's are left out.
                let events = crate::exclusion::events_without_excluded(store, today_events, &filter);
                let tried = llm::try_chain_with_failures(&chain, |entry| {
                    routines::run_daily_brief_with(
                        store,
                        entry.client.as_ref(),
                        &filter,
                        yesterday_start,
                        yesterday_end,
                        &events,
                        now_unix_ms,
                        &llm::profile::compose_instructions(&routing.daily_digest_instructions, &routing.profile, &routing.output_language),
                        &routing.output_language,
                        llm::limited_use::link_may_receive_google_data(entry, routing.google_data_policy()),
                    )
                });
                let (outcome, entry) = match tried {
                    Ok(done) => done,
                    Err(failures) => {
                        note_refusal(store, scope, routine.id, now_unix_ms, &routing.output_language, &failures);
                        continue;
                    }
                };
                crate::util::log_link_usage(store, entry, "routine", outcome.token_usage);
                let input_refs = outcome.input_refs;
                let mut new_run = outcome.new_run;
                new_run.routine_id = routine.id;
                let Ok(run_id) = store.insert_routine_run(&new_run) else {
                    continue;
                };
                // Fix round 1, B3: clean only when built from filtered material.
                let _ = store.set_routine_run_provenance(run_id, &crate::exclusion::provenance_for(store, &filter, input_refs));
                completed_runs.push(memory::RoutineRun {
                    id: run_id,
                    routine_id: new_run.routine_id,
                    ran_at_unix_ms: new_run.ran_at_unix_ms,
                    status: new_run.status,
                    output_summary: new_run.output_summary,
                    output_json: new_run.output_json,
                });
            }
            memory::RoutineTemplate::FocusTime => {
                // The pending draft IS this routine's entire output -- skip re-drafting
                // while a prior one is still unresolved.
                if routines_with_pending_approval.contains(&routine.id) {
                    continue;
                }
                // Anchored to the routine's own scheduled wall-clock time today, NOT to
                // when this tick fired. A tick can land up to 60s late, and the draft
                // then sits in Pending Approvals until the user acts -- possibly hours.
                // The schedule is what the user actually asked for, so that's what the
                // drafted block describes. Falls back to the tick time only for a
                // schedule string `scheduled_start_unix_ms` can't resolve (malformed, or
                // inside a DST spring-forward gap).
                let block_start =
                    routines::scheduled_start_unix_ms(&routine.schedule_hhmm, now_local)
                        .unwrap_or(now_unix_ms);
                let block_end = block_start + 60 * 60 * 1000; // 1-hour block, deterministic
                let mut new_run =
                    routines::build_focus_time_draft(block_start, block_end, now_unix_ms);
                new_run.routine_id = routine.id;
                let Ok(run_id) = store.insert_pending_routine_run(&new_run) else {
                    continue;
                };
                completed_runs.push(memory::RoutineRun {
                    id: run_id,
                    routine_id: routine.id,
                    ran_at_unix_ms: now_unix_ms,
                    status: memory::RunStatus::PendingApproval,
                    output_summary: new_run.output_summary,
                    output_json: "{}".to_string(),
                });
            }
            memory::RoutineTemplate::EodRecap => {
                let today_end = now_unix_ms;
                let today_start = local_midnight_ms(now_local, now_unix_ms);

                let cap_config = routing.spend_cap_config();
                if llm::check_spend_cap(store, &cap_config, None, Some("routine"), now_unix_ms)
                    .is_err()
                {
                    continue;
                }

                let Some(chain) = routine_chain(store, &routine.model_pref, ollama_model, routing, now_unix_ms, transport) else {
                    continue;
                };
                // A chain that can reach a hosted model gets no excluded client's material.
                let filter = crate::exclusion::filter_for_chain(store, &chain);
                let tried = llm::try_chain_with_failures(&chain, |entry| {
                    routines::run_eod_recap_with(
                        store,
                        entry.client.as_ref(),
                        &filter,
                        today_start,
                        today_end,
                        now_unix_ms,
                        gmail_connected,
                        recipient_email,
                        &llm::profile::compose_instructions(&routing.daily_digest_instructions, &routing.profile, &routing.output_language),
                        &routing.output_language,
                        llm::limited_use::link_may_receive_google_data(entry, routing.google_data_policy()),
                    )
                });
                let (outcome, entry) = match tried {
                    Ok(done) => done,
                    Err(failures) => {
                        note_refusal(store, scope, routine.id, now_unix_ms, &routing.output_language, &failures);
                        continue;
                    }
                };
                crate::util::log_link_usage(store, entry, "routine", outcome.token_usage);
                let input_refs = outcome.input_refs;
                let mut new_run = outcome.new_run;
                new_run.routine_id = routine.id;
                let Ok(run_id) = store.insert_routine_run(&new_run) else {
                    continue;
                };
                // Fix round 1, B3: clean only when built from filtered material.
                let _ = store.set_routine_run_provenance(run_id, &crate::exclusion::provenance_for(store, &filter, input_refs));
                completed_runs.push(memory::RoutineRun {
                    id: run_id,
                    routine_id: routine.id,
                    ran_at_unix_ms: new_run.ran_at_unix_ms,
                    status: new_run.status,
                    output_summary: new_run.output_summary,
                    output_json: new_run.output_json,
                });

                // Only the email delivery is skipped while a prior draft is still
                // unresolved -- the recap itself (inserted unconditionally above) is
                // never gated on it.
                if let Some(mut email_draft) = outcome.email_draft {
                    if routines_with_pending_approval.contains(&routine.id) {
                        continue;
                    }
                    email_draft.routine_id = routine.id;
                    if let Ok(email_run_id) = store.insert_pending_routine_run(&email_draft) {
                        completed_runs.push(memory::RoutineRun {
                            id: email_run_id,
                            routine_id: routine.id,
                            ran_at_unix_ms: email_draft.ran_at_unix_ms,
                            status: memory::RunStatus::PendingApproval,
                            output_summary: email_draft.output_summary,
                            output_json: "{}".to_string(),
                        });
                    }
                }
            }
            memory::RoutineTemplate::WeeklyDigest => {
                // `routines::is_due` only lets this arm run on WEEKLY_DIGEST_WEEKDAY, so
                // "today" here IS that fixed weekday -- the window is the 7 full days
                // ending at today's local midnight, mirroring DailyBrief's own
                // midnight-anchored "yesterday" window.
                let week_end = local_midnight_ms(now_local, now_unix_ms);
                let week_start = week_end - 7 * 24 * 60 * 60 * 1000;

                let cap_config = routing.spend_cap_config();
                if llm::check_spend_cap(store, &cap_config, None, Some("routine"), now_unix_ms)
                    .is_err()
                {
                    continue;
                }

                let Some(chain) = routine_chain(store, &routine.model_pref, ollama_model, routing, now_unix_ms, transport) else {
                    continue;
                };
                // A chain that can reach a hosted model gets no excluded client's material.
                let filter = crate::exclusion::filter_for_chain(store, &chain);
                let tried = llm::try_chain_with_failures(&chain, |entry| {
                    routines::run_weekly_digest_with(
                        store,
                        entry.client.as_ref(),
                        &filter,
                        week_start,
                        week_end,
                        now_unix_ms,
                        gmail_connected,
                        recipient_email,
                        &llm::profile::compose_instructions(&routing.weekly_digest_instructions, &routing.profile, &routing.output_language),
                        llm::limited_use::link_may_receive_google_data(entry, routing.google_data_policy()),
                    )
                });
                let (outcome, entry) = match tried {
                    Ok(done) => done,
                    Err(failures) => {
                        note_refusal(store, scope, routine.id, now_unix_ms, &routing.output_language, &failures);
                        continue;
                    }
                };
                crate::util::log_link_usage(store, entry, "routine", outcome.token_usage);
                let input_refs = outcome.input_refs;
                let mut new_run = outcome.new_run;
                new_run.routine_id = routine.id;
                let Ok(run_id) = store.insert_routine_run(&new_run) else {
                    continue;
                };
                // Fix round 1, B3: clean only when built from filtered material.
                let _ = store.set_routine_run_provenance(run_id, &crate::exclusion::provenance_for(store, &filter, input_refs));
                completed_runs.push(memory::RoutineRun {
                    id: run_id,
                    routine_id: routine.id,
                    ran_at_unix_ms: new_run.ran_at_unix_ms,
                    status: new_run.status,
                    output_summary: new_run.output_summary,
                    output_json: new_run.output_json,
                });

                if let Some(mut email_draft) = outcome.email_draft {
                    if routines_with_pending_approval.contains(&routine.id) {
                        continue;
                    }
                    email_draft.routine_id = routine.id;
                    if let Ok(email_run_id) = store.insert_pending_routine_run(&email_draft) {
                        completed_runs.push(memory::RoutineRun {
                            id: email_run_id,
                            routine_id: routine.id,
                            ran_at_unix_ms: email_draft.ran_at_unix_ms,
                            status: memory::RunStatus::PendingApproval,
                            output_summary: email_draft.output_summary,
                            output_json: "{}".to_string(),
                        });
                    }
                }
            }
            // Event-driven: `api::meeting_prep::run` prepares briefs shortly before each
            // calendar meeting. The daily schedule has nothing to do for it.
            // Event-driven too: `api::away_followup` files its drafts when Away Mode skips a meeting.
            memory::RoutineTemplate::MeetingPrep | memory::RoutineTemplate::AwayFollowUp => continue,
            memory::RoutineTemplate::ReplyPriorities => {
                // Deterministic: no spend cap, no provider chain, nothing to approve.
                let mut new_run = routines::run_reply_priorities(store, now_unix_ms, &routing.output_language);
                new_run.routine_id = routine.id;
                let Ok(run_id) = store.insert_routine_run(&new_run) else {
                    continue;
                };
                // Built from every waiting thread, unfiltered: clean only while nothing is excluded.
                // No recorded inputs: never clean while a client is excluded.
                let _ = store.set_routine_run_provenance(run_id, memory::topics::exclusion::UNFILTERED_PROVENANCE);
                completed_runs.push(memory::RoutineRun {
                    id: run_id,
                    routine_id: routine.id,
                    ran_at_unix_ms: new_run.ran_at_unix_ms,
                    status: new_run.status,
                    output_summary: new_run.output_summary,
                    output_json: new_run.output_json,
                });
            }
        }
    }

    Ok(completed_runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_millis_at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        chrono::Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn run_tick(store: &memory::Store, ms: i64, gate: RunGate) -> Vec<memory::RoutineRun> {
        tick(store, ms, "gemma4:12b", &RoutingConfig::default(), &[], false, None, gate).unwrap()
    }

    #[test]
    fn failover_runs_the_daily_brief_only_after_the_grace_period() {
        let store = memory::Store::open_in_memory().unwrap();
        assert!(run_tick(&store, local_millis_at(2026, 8, 3, 10, 0), RunGate::Failover).is_empty());
        assert!(run_tick(&store, local_millis_at(2026, 8, 3, 11, 29), RunGate::Failover).is_empty());
        let ran = run_tick(&store, local_millis_at(2026, 8, 3, 11, 30), RunGate::Failover);
        assert_eq!(ran.len(), 1);
        assert!(ran[0].output_summary.contains("No activity captured today"));
    }

    #[test]
    fn failover_never_runs_a_write_capable_routine() {
        let store = memory::Store::open_in_memory().unwrap();
        run_tick(&store, 0, RunGate::Full); // seed
        for r in store.list_routines().unwrap() {
            store.set_routine_enabled(r.id, true).unwrap();
        }
        let ran = run_tick(&store, local_millis_at(2026, 8, 3, 23, 0), RunGate::Failover);
        let routines = store.list_routines().unwrap();
        assert!(!ran.is_empty(), "the read-only routines still stand in");
        for run in ran {
            let t = routines.iter().find(|r| r.id == run.routine_id).unwrap().template.clone();
            assert!(
                matches!(t, memory::RoutineTemplate::DailyBrief | memory::RoutineTemplate::ReplyPriorities),
                "{t:?} must not fail over"
            );
        }
        assert!(store.pending_routine_runs().unwrap().is_empty(), "no draft is parked on a stand-in device");
    }

    #[test]
    fn a_run_dated_today_from_the_primary_suppresses_failover() {
        let store = memory::Store::open_in_memory().unwrap();
        // The primary's run arrives by sync and counts as a run today; a local Full run is the same row shape.
        assert_eq!(run_tick(&store, local_millis_at(2026, 8, 3, 9, 0), RunGate::Full).len(), 1);
        assert!(run_tick(&store, local_millis_at(2026, 8, 3, 12, 0), RunGate::Failover).is_empty());
    }

    #[test]
    fn a_failover_run_syncs_as_exactly_one_outbox_row() {
        let store = memory::Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-secondary");
        let before = store.list_outbox_operations().unwrap().len();
        assert_eq!(run_tick(&store, local_millis_at(2026, 8, 3, 12, 0), RunGate::Failover).len(), 1);
        assert_eq!(store.list_outbox_operations().unwrap().len(), before + 1);
    }

    #[test]
    fn full_and_off_still_behave_like_true_and_false() {
        let store = memory::Store::open_in_memory().unwrap();
        assert!(run_tick(&store, local_millis_at(2026, 8, 3, 9, 0), RunGate::Off).is_empty());
        assert_eq!(store.list_routines().unwrap().len(), 6, "Off still seeds");
        assert_eq!(run_tick(&store, local_millis_at(2026, 8, 3, 9, 0), RunGate::Full).len(), 1);
        assert_eq!(RunGate::from(true), RunGate::Full);
        assert_eq!(RunGate::from(false), RunGate::Off);
        assert_eq!(RunGate::from_byte(2), RunGate::Failover);
        assert_eq!(RunGate::from_byte(7), RunGate::Off);
    }

    /// Mirrors `core/ffi`'s `routines_tick_seeds_the_daily_brief_then_runs_it_once_due_
    /// then_not_again_same_day` at the typed layer (an empty store means the daily brief
    /// short-circuits to its "No activity" placeholder without a provider call).
    #[test]
    fn tick_seeds_then_runs_daily_brief_once_when_due_and_not_again_same_day() {
        let store = memory::Store::open_in_memory().unwrap();
        let routing = RoutingConfig::default();

        let early = tick(
            &store,
            local_millis_at(2026, 8, 3, 7, 0),
            "gemma4:12b",
            &routing,
            &[],
            false,
            None, true,
        )
        .unwrap();
        assert!(early.is_empty());
        assert_eq!(store.list_routines().unwrap().len(), 6);

        let due = tick(
            &store,
            local_millis_at(2026, 8, 3, 9, 0),
            "gemma4:12b",
            &routing,
            &[],
            false,
            None, true,
        )
        .unwrap();
        assert_eq!(due.len(), 1);
        assert!(due[0].output_summary.contains("No activity captured today"));

        let later = tick(
            &store,
            local_millis_at(2026, 8, 3, 20, 0),
            "gemma4:12b",
            &routing,
            &[],
            false,
            None, true,
        )
        .unwrap();
        assert!(later.is_empty());
    }

    /// Mirrors `tick_seeds_missing_templates_into_a_database_that_already_has_daily_brief`.
    #[test]
    fn tick_seeds_only_missing_templates_and_never_enables_write_capable_ones() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_routine(&memory::NewRoutine {
                name: "Daily Brief".to_string(),
                template: memory::RoutineTemplate::DailyBrief,
                schedule_hhmm: "07:15".to_string(),
                enabled: true,
            })
            .unwrap();
        tick(
            &store,
            0,
            "gemma4:12b",
            &RoutingConfig::default(),
            &[],
            false,
            None, true,
        )
        .unwrap();
        tick(
            &store,
            0,
            "gemma4:12b",
            &RoutingConfig::default(),
            &[],
            false,
            None, true,
        )
        .unwrap();

        let routines = store.list_routines().unwrap();
        assert_eq!(routines.len(), 6);
        let daily: Vec<_> = routines
            .iter()
            .filter(|r| r.template == memory::RoutineTemplate::DailyBrief)
            .collect();
        assert_eq!(daily.len(), 1);
        assert_eq!(daily[0].schedule_hhmm, "07:15");
        for r in routines
            .iter()
            .filter(|r| r.template != memory::RoutineTemplate::DailyBrief)
        {
            assert!(!r.enabled, "{} must seed disabled", r.name);
            assert!(!r.auto_approve_write_actions);
        }
    }

    #[test]
    fn tick_seeds_meeting_prep_disabled_and_never_runs_it_itself() {
        let store = memory::Store::open_in_memory().unwrap();
        let routing = RoutingConfig::default();
        tick(&store, local_millis_at(2026, 8, 3, 7, 0), "gemma4:12b", &routing, &[], false, None, true).unwrap();

        let routine = store
            .list_routines()
            .unwrap()
            .into_iter()
            .find(|r| r.template == memory::RoutineTemplate::MeetingPrep)
            .expect("seeded");
        assert!(!routine.enabled, "never silently on");

        store.set_routine_enabled(routine.id, true).unwrap();
        let runs = tick(&store, local_millis_at(2026, 8, 3, 8, 30), "gemma4:12b", &routing, &[], false, None, true).unwrap();
        assert!(runs.iter().all(|r| r.routine_id != routine.id), "event-driven: the daily tick never runs it");
        assert!(store.most_recent_routine_run(routine.id).unwrap().is_none());
    }

    #[test]
    fn tick_skips_away_followup() {
        let store = memory::Store::open_in_memory().unwrap();
        let routing = RoutingConfig::default();
        tick(&store, local_millis_at(2026, 8, 3, 7, 0), "gemma4:12b", &routing, &[], false, None, true).unwrap();
        let routine_id = store.ensure_away_routine().unwrap();
        let routine = store.list_routines().unwrap().into_iter().find(|r| r.id == routine_id).unwrap();
        store.set_routine_enabled(routine.id, true).unwrap();
        let runs = tick(&store, local_millis_at(2026, 8, 3, 8, 30), "gemma4:12b", &routing, &[], false, None, true).unwrap();
        assert!(runs.iter().all(|r| r.routine_id != routine.id));
        assert!(store.most_recent_routine_run(routine.id).unwrap().is_none());
    }

    #[test]
    fn a_device_that_may_not_run_routines_seeds_them_but_runs_nothing() {
        let store = memory::Store::open_in_memory().unwrap();
        let routing = RoutingConfig::default();
        tick(&store, local_millis_at(2026, 8, 3, 7, 0), "gemma4:12b", &routing, &[], false, None, false).unwrap();
        let routines = store.list_routines().unwrap();
        assert_eq!(routines.len(), 6, "the routines are still seeded so a later promotion works");
        for template in [memory::RoutineTemplate::ReplyPriorities, memory::RoutineTemplate::FocusTime] {
            let id = routines.iter().find(|r| r.template == template).unwrap().id;
            store.set_routine_enabled(id, true).unwrap();
        }
        let due = local_millis_at(2026, 8, 3, 9, 30);
        let ran = tick(&store, due, "gemma4:12b", &routing, &[], false, None, false).unwrap();
        assert!(ran.is_empty(), "nothing runs when this device may not");
        assert!(store.pending_routine_runs().unwrap().is_empty(), "no draft was parked");
        assert!(store.recent_routine_runs_across_all(10).unwrap().is_empty());

        // The same state on a device that may run does run them.
        let ran = tick(&store, due, "gemma4:12b", &routing, &[], false, None, true).unwrap();
        assert!(ran.iter().any(|r| r.status == memory::RunStatus::PendingApproval), "Focus Time parks its draft");
        assert!(ran.iter().any(|r| r.output_summary.contains("Nothing to rank yet")), "Reply Priorities ran");
    }

    #[test]
    fn tick_seeds_reply_priorities_disabled_and_runs_it_when_enabled() {
        let store = memory::Store::open_in_memory().unwrap();
        let routing = RoutingConfig::default();
        tick(&store, local_millis_at(2026, 8, 3, 7, 0), "gemma4:12b", &routing, &[], false, None, true).unwrap();

        let routine = store.list_routines().unwrap().into_iter()
            .find(|r| r.template == memory::RoutineTemplate::ReplyPriorities)
            .expect("seeded");
        assert!(!routine.enabled, "never silently on");
        assert_eq!(routine.schedule_hhmm, "08:00");

        store.set_routine_enabled(routine.id, true).unwrap();
        let runs = tick(&store, local_millis_at(2026, 8, 3, 8, 30), "gemma4:12b", &routing, &[], false, None, true).unwrap();
        let run = runs.iter().find(|r| r.routine_id == routine.id).expect("ran once due");
        assert!(run.output_summary.contains("Nothing to rank yet"));

        let again = tick(&store, local_millis_at(2026, 8, 3, 9, 30), "gemma4:12b", &routing, &[], false, None, true).unwrap();
        assert!(again.iter().all(|r| r.routine_id != routine.id), "not again the same day");
    }

    fn policy(templates: &[&str], runner_fresh: bool) -> CloudPolicy {
        CloudPolicy { cloud_templates: templates.iter().map(|t| t.to_string()).collect(), runner_fresh, ..CloudPolicy::default() }
    }

    fn with_ready_by(mut policy: CloudPolicy, template: &str, hhmm: &str) -> CloudPolicy {
        policy.ready_by.insert(template.to_string(), hhmm.to_string());
        policy
    }

    #[test]
    fn the_runner_starts_a_flagged_routine_early_enough_to_be_ready_by_the_ready_by_time() {
        let store = memory::Store::open_in_memory().unwrap();
        store.seed_builtin_routines().unwrap();
        // The brief is scheduled at 08:30 but must be ready by 07:00.
        let cloud = with_ready_by(policy(&["daily_brief"], true), "daily_brief", "07:00");
        let ready = local_millis_at(2026, 8, 3, 7, 0);
        let start = ready - READY_BY_P95_MS - READY_BY_MARGIN_MS;
        assert!(run_cloud(&store, start - 60_000, RunGate::Full, &cloud, RunnerRole::Runner).is_empty(), "not before its start");
        let ran = run_cloud(&store, start, RunGate::Full, &cloud, RunnerRole::Runner);
        assert_eq!(ran.len(), 1, "runs at ready-by minus p95 and margin, before its 08:30 schedule");
        assert_eq!(template_of(&store, &ran[0]), memory::RoutineTemplate::DailyBrief);
        // The once-a-day check holds: not again at its schedule, nor later that day.
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 8, 30), RunGate::Full, &cloud, RunnerRole::Runner).is_empty());
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 23, 0), RunGate::Full, &cloud, RunnerRole::Runner).is_empty());
        // A malformed ready-by falls back to the schedule.
        let store = memory::Store::open_in_memory().unwrap();
        let bad = with_ready_by(policy(&["daily_brief"], true), "daily_brief", "7am");
        assert!(run_cloud(&store, start, RunGate::Full, &bad, RunnerRole::Runner).is_empty());
        assert_eq!(run_cloud(&store, local_millis_at(2026, 8, 3, 8, 30), RunGate::Full, &bad, RunnerRole::Runner).len(), 1);
    }

    #[test]
    fn without_ready_by_the_runner_uses_the_schedule() {
        let store = memory::Store::open_in_memory().unwrap();
        // A ready-by for another template does not move the brief.
        let cloud = with_ready_by(policy(&["daily_brief"], true), "eod_recap", "07:00");
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 8, 29), RunGate::Full, &cloud, RunnerRole::Runner).is_empty());
        assert_eq!(run_cloud(&store, local_millis_at(2026, 8, 3, 8, 30), RunGate::Full, &cloud, RunnerRole::Runner).len(), 1);
    }

    #[test]
    fn a_late_ready_by_still_lets_the_device_stand_in_at_23_30_when_nothing_synced() {
        // Ready by 21:30: ready-by plus the grace is 00:30 tomorrow, so the device stands in at 23:30 today.
        let cloud = with_ready_by(policy(&["daily_brief"], true), "daily_brief", "21:30");
        let store = memory::Store::open_in_memory().unwrap();
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 23, 29), RunGate::Full, &cloud, RunnerRole::Device).is_empty());
        assert_eq!(run_cloud(&store, local_millis_at(2026, 8, 3, 23, 30), RunGate::Full, &cloud, RunnerRole::Device).len(), 1);
        // A run synced in for today: the device does not stand in.
        let synced = memory::Store::open_in_memory().unwrap();
        assert_eq!(run_cloud(&synced, local_millis_at(2026, 8, 3, 21, 0), RunGate::Full, &policy(&["daily_brief"], true), RunnerRole::Runner).len(), 1);
        assert!(run_cloud(&synced, local_millis_at(2026, 8, 3, 23, 45), RunGate::Full, &cloud, RunnerRole::Device).is_empty());
    }

    #[test]
    fn a_ready_by_whose_grace_ends_before_23_30_stands_in_at_ready_by_plus_grace() {
        let cloud = with_ready_by(policy(&["daily_brief"], true), "daily_brief", "20:00");
        let store = memory::Store::open_in_memory().unwrap();
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 22, 59), RunGate::Full, &cloud, RunnerRole::Device).is_empty());
        assert_eq!(run_cloud(&store, local_millis_at(2026, 8, 3, 23, 0), RunGate::Full, &cloud, RunnerRole::Device).len(), 1);
    }

    #[test]
    fn the_cap_never_fires_before_ready_by_itself() {
        // Ready by 23:45 (after the 23:30 cap): the device stands in at 23:45, never before.
        let cloud = with_ready_by(policy(&["daily_brief"], true), "daily_brief", "23:45");
        let store = memory::Store::open_in_memory().unwrap();
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 23, 30), RunGate::Full, &cloud, RunnerRole::Device).is_empty());
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 23, 44), RunGate::Full, &cloud, RunnerRole::Device).is_empty());
        assert_eq!(run_cloud(&store, local_millis_at(2026, 8, 3, 23, 45), RunGate::Full, &cloud, RunnerRole::Device).len(), 1);
    }

    #[test]
    fn the_device_grace_counts_from_ready_by() {
        // Brief at 08:30, ready by 10:00: the device waits until 13:00, not 11:30.
        let cloud = with_ready_by(policy(&["daily_brief"], true), "daily_brief", "10:00");
        let store = memory::Store::open_in_memory().unwrap();
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 11, 30), RunGate::Full, &cloud, RunnerRole::Device).is_empty());
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 12, 59), RunGate::Full, &cloud, RunnerRole::Device).is_empty());
        assert_eq!(run_cloud(&store, local_millis_at(2026, 8, 3, 13, 0), RunGate::Full, &cloud, RunnerRole::Device).len(), 1);
        // Ready by 07:00 (before the 08:30 schedule): the device may stand in from 10:00.
        let early = with_ready_by(policy(&["daily_brief"], true), "daily_brief", "07:00");
        let store = memory::Store::open_in_memory().unwrap();
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 9, 59), RunGate::Full, &early, RunnerRole::Device).is_empty());
        assert_eq!(run_cloud(&store, local_millis_at(2026, 8, 3, 10, 0), RunGate::Full, &early, RunnerRole::Device).len(), 1);
    }

    fn run_cloud(store: &memory::Store, ms: i64, gate: RunGate, policy: &CloudPolicy, role: RunnerRole) -> Vec<memory::RoutineRun> {
        tick_cloud(store, ms, "gemma4:12b", &RoutingConfig::default(), &[], false, None, gate, policy, role).unwrap()
    }

    fn template_of(store: &memory::Store, run: &memory::RoutineRun) -> memory::RoutineTemplate {
        store.list_routines().unwrap().into_iter().find(|r| r.id == run.routine_id).unwrap().template
    }

    #[test]
    fn device_skips_a_cloud_routine_while_the_runner_is_fresh() {
        let store = memory::Store::open_in_memory().unwrap();
        let at = local_millis_at(2026, 8, 3, 9, 0);
        assert!(run_cloud(&store, at, RunGate::Full, &policy(&["daily_brief"], true), RunnerRole::Device).is_empty());
        assert!(store.recent_routine_runs_across_all(10).unwrap().is_empty());
        // Control: the same device runs it when it is not flagged for the cloud.
        assert_eq!(run_cloud(&store, at, RunGate::Full, &policy(&[], true), RunnerRole::Device).len(), 1);
    }

    #[test]
    fn device_runs_it_under_the_failover_rule_when_the_runner_is_stale() {
        let store = memory::Store::open_in_memory().unwrap();
        let stale = policy(&["daily_brief"], false);
        // 08:30 brief: not before the grace period has passed, even on the primary.
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 9, 0), RunGate::Full, &stale, RunnerRole::Device).is_empty());
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 11, 29), RunGate::Full, &stale, RunnerRole::Device).is_empty());
        let ran = run_cloud(&store, local_millis_at(2026, 8, 3, 11, 30), RunGate::Full, &stale, RunnerRole::Device);
        assert_eq!(ran.len(), 1);
        assert_eq!(template_of(&store, &ran[0]), memory::RoutineTemplate::DailyBrief);
        // Off still runs nothing.
        let store = memory::Store::open_in_memory().unwrap();
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 12, 0), RunGate::Off, &stale, RunnerRole::Device).is_empty());
    }

    #[test]
    fn runner_runs_only_flagged_eligible_templates() {
        let store = memory::Store::open_in_memory().unwrap();
        store.seed_builtin_routines().unwrap();
        for r in store.list_routines().unwrap() {
            store.set_routine_enabled(r.id, true).unwrap();
        }
        // Write routines and reply priorities are flagged too: the runner still never runs them.
        let flagged = policy(&["daily_brief", "reply_priorities", "focus_time", "away_follow_up"], true);
        let late = local_millis_at(2026, 8, 3, 23, 0);
        // The gate is the device's own setting; the runner treats it as Full.
        let ran = tick_cloud(&store, late, "gemma4:12b", &RoutingConfig::default(), &[], true, Some("me@example.com"), RunGate::Off, &flagged, RunnerRole::Runner)
            .unwrap();
        assert_eq!(ran.len(), 1, "{ran:?}");
        assert_eq!(template_of(&store, &ran[0]), memory::RoutineTemplate::DailyBrief);
        assert!(store.pending_routine_runs().unwrap().is_empty(), "no draft is parked on the runner");
        // An eligible template that is not flagged does not run there either.
        let store = memory::Store::open_in_memory().unwrap();
        assert!(run_cloud(&store, late, RunGate::Full, &policy(&["eod_recap"], true), RunnerRole::Runner).is_empty());
        assert!(store.recent_routine_runs_across_all(10).unwrap().is_empty());
    }

    /// A model transport that keeps every prompt it is sent.
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    impl llm::LlmClient for Recorder {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            self.0.lock().unwrap().push(prompt.to_string());
            Ok(llm::Completion { text: r#"{"summary": "A day.", "open_loops": []}"#.into(), token_usage: llm::TokenUsage::default() })
        }
    }

    /// A store with Acme excluded from hosted models and one meeting yesterday that does not name it.
    fn excluded_client_store() -> (tempfile::TempDir, memory::Store) {
        use memory::topics::client_controls::ClientControl;
        let dir = tempfile::tempdir().unwrap();
        let store = memory::Store::open(&dir.path().join("t.db"), dir.path()).unwrap();
        let acme = store.create_seed_topic("Acme", &[], None).unwrap();
        store.set_client_control(&acme.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        let start = local_millis_at(2026, 8, 2, 14, 0);
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 60_000),
                title: "Team call".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        assert!(store.set_meeting_calendar_source(id, memory::limited_use::CALENDAR_SOURCE_OTHER).unwrap());
        store
            .insert_transcript_segments(&[memory::NewTranscriptSegment { meeting_id: id, start_cs: 0, speaker_label: "Speaker 1".into(), text: "The Globex review is next week.".into() }])
            .unwrap();
        store.update_meeting_summary(id, "Globex review next week.", &[], &[], &[], "anthropic", "m").unwrap();
        (dir, store)
    }

    #[test]
    fn an_excluded_client_request_is_refused_on_the_runner_chain_and_the_run_row_notes_it() {
        let (_dir, store) = excluded_client_store();
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = sent.clone();
        // The runner's chain: hosted only, behind the gate production uses.
        TEST_CHAIN.with(|c| {
            *c.borrow_mut() = Some(Box::new(move |s: &memory::Store| {
                vec![llm::guard_hosted(
                    llm::ChainLink { provider: llm::Provider::Anthropic, model: "claude-test".into(), client: Box::new(Recorder(recorder.clone())) },
                    crate::exclusion::current_gate(s),
                )]
            }))
        });
        // The routine's own request names the client.
        let routing = RoutingConfig { daily_digest_instructions: "Always lead with Acme.".into(), ..RoutingConfig::default() };
        let cloud = policy(&["daily_brief"], true);
        let at = local_millis_at(2026, 8, 3, 9, 0);
        let ran = tick_cloud(&store, at, "gemma4:12b", &routing, &[], false, None, RunGate::Full, &cloud, RunnerRole::Runner);
        TEST_CHAIN.with(|c| *c.borrow_mut() = None);
        assert!(ran.unwrap().is_empty(), "nothing was produced");
        assert!(sent.lock().unwrap().is_empty(), "nothing was sent to the hosted model");
        let brief = store.list_routines().unwrap().into_iter().find(|r| r.template == memory::RoutineTemplate::DailyBrief).unwrap();
        let row = store.most_recent_routine_run(brief.id).unwrap().expect("the run row says why");
        assert_eq!(row.status, memory::RunStatus::Failed);
        assert_eq!(row.output_summary, LEFT_FOR_YOUR_COMPUTER);
        assert!(is_left_for_computer(&row));
        let stored = format!("{} {}", row.output_summary, row.output_json).to_lowercase();
        assert!(!stored.contains("acme"), "the note never names the client: {stored}");
        // The runner does not try again the same day.
        assert!(tick_cloud(&store, at + 60_000, "gemma4:12b", &routing, &[], false, None, RunGate::Full, &cloud, RunnerRole::Runner).unwrap().is_empty());
        assert_eq!(store.recent_routine_runs(brief.id, 10).unwrap().len(), 1);
    }

    /// The cloud runner's seam: a substituted transport changes only the client a link talks
    /// through. The chain still comes from routing and every hosted link stays behind the gate.
    #[test]
    fn a_substituted_transport_stays_behind_the_exclusion_gate() {
        let (_dir, store) = excluded_client_store();
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = sent.clone();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_by = seen.clone();
        let transport = move |provider: llm::Provider, model: &str| -> Box<dyn llm::LlmClient> {
            seen_by.lock().unwrap().push((provider, model.to_string()));
            Box::new(Recorder(recorder.clone()))
        };
        let routing = RoutingConfig {
            preferred_provider: "anthropic".into(),
            anthropic_key: Some("k".into()),
            use_local_model: false,
            daily_digest_instructions: "Always lead with Acme.".into(),
            ..RoutingConfig::default()
        };
        let cloud = policy(&["daily_brief"], true);
        let at = local_millis_at(2026, 8, 3, 9, 0);
        let ran = tick_cloud_with_transport(&store, at, "", &routing, &[], false, None, RunGate::Full, &cloud, RunnerRole::Runner, Some(&transport))
            .unwrap();
        assert!(ran.is_empty());
        assert!(sent.lock().unwrap().is_empty(), "the gate refused before the transport");
        assert_eq!(seen.lock().unwrap().iter().map(|(p, _)| *p).collect::<Vec<_>>(), vec![llm::Provider::Anthropic], "the chain is routing's");
        let brief = store.list_routines().unwrap().into_iter().find(|r| r.template == memory::RoutineTemplate::DailyBrief).unwrap();
        assert!(is_left_for_computer(&store.most_recent_routine_run(brief.id).unwrap().unwrap()));

        // The same request without the client's name goes through the substituted transport.
        let (_dir, store) = excluded_client_store();
        let routing = RoutingConfig { daily_digest_instructions: String::new(), ..routing };
        let ran = tick_cloud_with_transport(&store, at, "", &routing, &[], false, None, RunGate::Full, &cloud, RunnerRole::Runner, Some(&transport))
            .unwrap();
        assert_eq!(ran.len(), 1);
        assert_eq!(sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_computer_picks_up_a_routine_the_runner_left_for_it() {
        let (_dir, store) = excluded_client_store();
        store.seed_builtin_routines().unwrap();
        let brief = store.list_routines().unwrap().into_iter().find(|r| r.template == memory::RoutineTemplate::DailyBrief).unwrap();
        let at = local_millis_at(2026, 8, 3, 9, 0);
        // The runner's row, as it arrives by sync.
        store
            .insert_routine_run(&memory::NewRoutineRun {
                routine_id: brief.id,
                ran_at_unix_ms: at,
                status: memory::RunStatus::Failed,
                output_summary: LEFT_FOR_YOUR_COMPUTER.into(),
                output_json: serde_json::json!({ LEFT_FOR_COMPUTER_FIELD: "excluded_client" }).to_string(),
            })
            .unwrap();
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = sent.clone();
        // The computer's chain: a model on this computer.
        TEST_CHAIN.with(|c| {
            *c.borrow_mut() = Some(Box::new(move |_: &memory::Store| {
                vec![llm::ChainLink { provider: llm::Provider::Ollama, model: "gemma-test".into(), client: Box::new(Recorder(recorder.clone())) }]
            }))
        });
        let ran = run_cloud(&store, at + 5 * 60_000, RunGate::Full, &policy(&["daily_brief"], true), RunnerRole::Device);
        let again = run_cloud(&store, at + 10 * 60_000, RunGate::Full, &policy(&["daily_brief"], true), RunnerRole::Device);
        TEST_CHAIN.with(|c| *c.borrow_mut() = None);
        assert_eq!(ran.len(), 1, "a fresh runner does not stop the computer from taking it back");
        assert_eq!(ran[0].status, memory::RunStatus::Success);
        assert_eq!(sent.lock().unwrap().len(), 1);
        assert!(again.is_empty(), "once a day");
    }

    /// Fix round 1, I1: only a row left today hands the routine back.
    #[test]
    fn an_earlier_days_left_row_does_not_override_a_fresh_runner() {
        let store = memory::Store::open_in_memory().unwrap();
        store.seed_builtin_routines().unwrap();
        let brief = store.list_routines().unwrap().into_iter().find(|r| r.template == memory::RoutineTemplate::DailyBrief).unwrap();
        store
            .insert_routine_run(&memory::NewRoutineRun {
                routine_id: brief.id,
                ran_at_unix_ms: local_millis_at(2026, 8, 3, 9, 0),
                status: memory::RunStatus::Failed,
                output_summary: LEFT_FOR_YOUR_COMPUTER.into(),
                output_json: serde_json::json!({ LEFT_FOR_COMPUTER_FIELD: "excluded_client" }).to_string(),
            })
            .unwrap();
        let fresh = policy(&["daily_brief"], true);
        assert!(run_cloud(&store, local_millis_at(2026, 8, 4, 9, 0), RunGate::Full, &fresh, RunnerRole::Device).is_empty());
        assert_eq!(store.recent_routine_runs(brief.id, 10).unwrap().len(), 1);
    }

    /// Fix round 1, I2: the runner made nothing (spend cap, no provider, provider down): the computer runs
    /// it once the grace has passed.
    #[test]
    fn device_runs_a_cloud_routine_after_the_grace_when_a_fresh_runner_produced_nothing() {
        let store = memory::Store::open_in_memory().unwrap();
        let ran = run_cloud(&store, local_millis_at(2026, 8, 3, 11, 30), RunGate::Full, &policy(&["daily_brief"], true), RunnerRole::Device);
        assert_eq!(ran.len(), 1);
        assert_eq!(template_of(&store, &ran[0]), memory::RoutineTemplate::DailyBrief);
    }

    #[test]
    fn device_still_skips_a_cloud_routine_before_the_grace_while_the_runner_is_fresh() {
        let store = memory::Store::open_in_memory().unwrap();
        let fresh = policy(&["daily_brief"], true);
        for (h, m) in [(8, 30), (9, 0), (11, 29)] {
            assert!(run_cloud(&store, local_millis_at(2026, 8, 3, h, m), RunGate::Full, &fresh, RunnerRole::Device).is_empty());
        }
        assert!(store.recent_routine_runs_across_all(10).unwrap().is_empty());
    }

    #[test]
    fn a_late_runner_run_after_the_device_already_ran_does_not_run_twice() {
        let store = memory::Store::open_in_memory().unwrap();
        let cloud = policy(&["daily_brief"], true);
        assert_eq!(run_cloud(&store, local_millis_at(2026, 8, 3, 11, 30), RunGate::Full, &cloud, RunnerRole::Device).len(), 1);
        // The runner comes back late the same day: the device's run (synced) counts as today's.
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 12, 0), RunGate::Full, &cloud, RunnerRole::Runner).is_empty());
        assert!(run_cloud(&store, local_millis_at(2026, 8, 3, 13, 0), RunGate::Full, &cloud, RunnerRole::Device).is_empty());
        let brief = store.list_routines().unwrap().into_iter().find(|r| r.template == memory::RoutineTemplate::DailyBrief).unwrap();
        assert_eq!(store.recent_routine_runs(brief.id, 10).unwrap().len(), 1);
    }

    #[test]
    fn stale_runner_cloud_flagged_eod_recap_runs_on_the_device_after_the_grace() {
        let store = memory::Store::open_in_memory().unwrap();
        store.seed_builtin_routines().unwrap();
        for r in store.list_routines().unwrap() {
            store.set_routine_enabled(r.id, r.template == memory::RoutineTemplate::EodRecap).unwrap();
        }
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = sent.clone();
        TEST_CHAIN.with(|c| {
            *c.borrow_mut() = Some(Box::new(move |_: &memory::Store| {
                vec![llm::ChainLink { provider: llm::Provider::Ollama, model: "gemma-test".into(), client: Box::new(Recorder(recorder.clone())) }]
            }))
        });
        let stale = policy(&["eod_recap"], false);
        // 17:30 recap: the grace ends at 20:30.
        let before = run_cloud(&store, local_millis_at(2026, 8, 3, 20, 29), RunGate::Full, &stale, RunnerRole::Device);
        let after = run_cloud(&store, local_millis_at(2026, 8, 3, 20, 30), RunGate::Full, &stale, RunnerRole::Device);
        TEST_CHAIN.with(|c| *c.borrow_mut() = None);
        assert!(before.is_empty());
        assert_eq!(after.len(), 1, "{after:?}");
        assert_eq!(template_of(&store, &after[0]), memory::RoutineTemplate::EodRecap);
    }

    /// A routine pinned to a provider with a spent budget stops; one whose pinned provider
    /// has no key (so the pin isn't the chain's first link) or that isn't pinned falls through.
    #[test]
    fn routine_pinned_to_spent_provider_stops_unpinned_falls_through() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = crate::util::now_unix_ms();
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: "anthropic".into(),
                model: "m".into(),
                route: Some("routine".into()),
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: 2500,
                cache_hit: false,
                trace_id: "t".into(),
                ts_unix_ms: now,
            })
            .unwrap();
        store.set_provider_budgets(&[("anthropic".into(), 2500)]).unwrap();
        let routing = RoutingConfig { anthropic_key: Some("k".into()), openai_key: Some("k".into()), ..RoutingConfig::default() };
        let providers = |pref: &str, routing: &RoutingConfig| {
            routine_chain(&store, pref, "gemma4:12b", routing, now, None).map(|c| c.iter().map(|l| l.provider).collect::<Vec<_>>())
        };
        assert_eq!(providers("anthropic/claude-haiku-4-5", &routing), None, "pinned: no fallthrough");
        let unpinned = providers("default", &routing).unwrap();
        assert!(!unpinned.is_empty() && !unpinned.contains(&llm::Provider::Anthropic), "{unpinned:?}");
        // The pinned provider has no key: the routine runs on the default chain, minus it.
        let no_key = RoutingConfig { anthropic_key: None, ..routing.clone() };
        let split = providers("anthropic/claude-haiku-4-5", &no_key).unwrap();
        assert!(!split.is_empty() && !split.contains(&llm::Provider::Anthropic), "{split:?}");
        // A pin to a provider with budget left keeps it first.
        assert_eq!(providers("openai/gpt-5-mini", &routing).unwrap()[0], llm::Provider::OpenAI);
    }
}
