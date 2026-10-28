//! FR-RTN-1..4: the routine scheduler's tick. Moved from `core/ffi::whimbrel_routines_tick`
//! (M3.0). The other `whimbrel_routines_*` C functions are single-`Store`-call
//! pass-throughs and have no twin here — a Rust shell calls `memory::Store` directly.

use std::collections::HashSet;

use chrono::TimeZone;

use crate::routing::{build_chain, RoutingConfig};
use crate::util::log_usage_for;

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
/// considered. A spend-cap rejection or exhausted provider chain for one due routine does NOT fail
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
    let gate = gate.into();
    store.seed_builtin_routines()?;
    // The primary-device gate (docs/superpowers/specs/2026-10-08-primary-device-routines-design.md):
    // a device that is not the one chosen to run routines seeds them, so pulled runs have a routine to
    // attach to and a later promotion works, but runs nothing -- not even a due or write-capable one.
    if gate == RunGate::Off {
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
        let last_run_local_date = match store.most_recent_routine_run(routine.id) {
            Ok(Some(run)) => chrono::Local
                .timestamp_millis_opt(run.ran_at_unix_ms)
                .single()
                .map(|dt| dt.date_naive()),
            Ok(None) => None,
            Err(_) => continue,
        };
        if !routines::is_due(routine, now_local, last_run_local_date) {
            continue;
        }
        if gate == RunGate::Failover && !routines::failover_eligible(routine, now_unix_ms, now_local) {
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

                let chain = build_chain(ollama_model, routing);
                let Some((outcome, entry)) = llm::try_chain(&chain, |entry| {
                    routines::run_daily_brief(
                        store,
                        entry.client.as_ref(),
                        yesterday_start,
                        yesterday_end,
                        today_events,
                        now_unix_ms,
                        &llm::profile::compose_instructions(&routing.daily_digest_instructions, &routing.profile),
                    )
                }) else {
                    continue;
                };
                log_usage_for(
                    store,
                    entry.provider,
                    &entry.model,
                    "routine",
                    outcome.token_usage,
                );
                let mut new_run = outcome.new_run;
                new_run.routine_id = routine.id;
                let Ok(run_id) = store.insert_routine_run(&new_run) else {
                    continue;
                };
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

                let chain = build_chain(ollama_model, routing);
                let Some((outcome, entry)) = llm::try_chain(&chain, |entry| {
                    routines::run_eod_recap(
                        store,
                        entry.client.as_ref(),
                        today_start,
                        today_end,
                        now_unix_ms,
                        gmail_connected,
                        recipient_email,
                        &llm::profile::compose_instructions(&routing.daily_digest_instructions, &routing.profile),
                    )
                }) else {
                    continue;
                };
                log_usage_for(
                    store,
                    entry.provider,
                    &entry.model,
                    "routine",
                    outcome.token_usage,
                );
                let mut new_run = outcome.new_run;
                new_run.routine_id = routine.id;
                let Ok(run_id) = store.insert_routine_run(&new_run) else {
                    continue;
                };
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

                let chain = build_chain(ollama_model, routing);
                let Some((outcome, entry)) = llm::try_chain(&chain, |entry| {
                    routines::run_weekly_digest(
                        store,
                        entry.client.as_ref(),
                        week_start,
                        week_end,
                        now_unix_ms,
                        gmail_connected,
                        recipient_email,
                        &llm::profile::compose_instructions(&routing.weekly_digest_instructions, &routing.profile),
                    )
                }) else {
                    continue;
                };
                log_usage_for(
                    store,
                    entry.provider,
                    &entry.model,
                    "routine",
                    outcome.token_usage,
                );
                let mut new_run = outcome.new_run;
                new_run.routine_id = routine.id;
                let Ok(run_id) = store.insert_routine_run(&new_run) else {
                    continue;
                };
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
                let mut new_run = routines::run_reply_priorities(store, now_unix_ms);
                new_run.routine_id = routine.id;
                let Ok(run_id) = store.insert_routine_run(&new_run) else {
                    continue;
                };
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
}
