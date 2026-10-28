//! FR-RTN-1..4: routine scheduling and execution. `is_due` is timezone-correct
//! (`chrono::Local`, not `chrono::Utc`) — a routine's `schedule_hhmm` is meant as the
//! user's own local wall-clock time ("8:30 daily brief"), and this crate is the one
//! place that timezone-sensitive comparison happens; every other layer (Store,
//! whimbrel_routines_tick) deals only in already-computed UTC millisecond bounds, the
//! same convention `Store::timeline_range` already uses.

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, Timelike, Weekday};
use memory::Routine;

/// `WeeklyDigest` fires on this local weekday only -- no per-routine day-of-week config
/// exists (or is planned; see this crate's existing FocusTime day-of-week deferral), so
/// this is the one fixed day, matching a Sunday "week in review" cadence.
const WEEKLY_DIGEST_WEEKDAY: Weekday = Weekday::Sun;

/// True if `routine` is enabled, its `schedule_hhmm` wall-clock time has already passed
/// for `now` (in `now`'s own local timezone), it hasn't already run on `now`'s local
/// calendar date, and -- for `WeeklyDigest` only -- `now` falls on
/// `WEEKLY_DIGEST_WEEKDAY`. `last_run_local_date` is the caller's job to compute (from
/// the routine's most recent `RoutineRun.ran_at_unix_ms`, converted to a local date) —
/// keeping this function pure and trivially testable across timezone/date-boundary
/// cases without a live clock or a `Store`.
///
/// Every check except `enabled` is skipped when the `WHIMBREL_DEBUG_FORCE_ROUTINES_DUE`
/// env var is set -- a manual-test escape hatch for firing an enabled routine on demand
/// (e.g. to see a real WeeklyDigest/EodRecap email without waiting for its real
/// schedule/weekday) that has zero effect on a normal Finder/Dock launch, which doesn't
/// inherit a Terminal's env vars.
pub fn is_due(
    routine: &Routine,
    now: DateTime<Local>,
    last_run_local_date: Option<NaiveDate>,
) -> bool {
    is_due_inner(
        routine,
        now,
        last_run_local_date,
        std::env::var_os("WHIMBREL_DEBUG_FORCE_ROUTINES_DUE").is_some(),
    )
}

fn is_due_inner(
    routine: &Routine,
    now: DateTime<Local>,
    last_run_local_date: Option<NaiveDate>,
    force_due: bool,
) -> bool {
    if !routine.enabled {
        return false;
    }
    if force_due {
        return true;
    }
    if last_run_local_date == Some(now.date_naive()) {
        return false;
    }
    if routine.template == memory::RoutineTemplate::WeeklyDigest
        && now.weekday() != WEEKLY_DIGEST_WEEKDAY
    {
        return false;
    }
    let Some((hour, minute)) = parse_hhmm(&routine.schedule_hhmm) else {
        return false;
    };
    now.hour() > hour || (now.hour() == hour && now.minute() >= minute)
}

/// The UTC-millisecond instant of `now`'s own local calendar date at `schedule_hhmm`'s
/// wall-clock time -- i.e. "when this routine was scheduled to run today", which is not
/// the same as when the tick that noticed it actually fired.
///
/// Focus Time needs this because its drafted calendar block must describe the block the
/// user actually asked for ("09:00 Focus Time" -> a 09:00 block), not wherever the 60s
/// heartbeat happened to land, and certainly not wherever the clock has moved to by the
/// time the user gets around to approving the draft. Returns `None` for a malformed
/// `schedule_hhmm` (the same input `is_due` rejects) or a wall-clock time that doesn't
/// exist locally on this date, e.g. inside a DST spring-forward gap -- the caller
/// decides the fallback.
pub fn scheduled_start_unix_ms(schedule_hhmm: &str, now: DateTime<Local>) -> Option<i64> {
    use chrono::TimeZone;
    let (hour, minute) = parse_hhmm(schedule_hhmm)?;
    now.date_naive()
        .and_hms_opt(hour, minute, 0)
        .and_then(|naive| now.timezone().from_local_datetime(&naive).single())
        .map(|dt| dt.timestamp_millis())
}

/// How long past its scheduled time a routine must still be undone before a non-primary device
/// may stand in for the primary (docs/superpowers/specs/2026-10-12-routines-safety-net-design.md).
pub const FAILOVER_GRACE_MS: i64 = 3 * 60 * 60 * 1000;

/// Whether a device that is not the chosen primary may run `routine` now. Only the read-only
/// templates qualify, and only once `FAILOVER_GRACE_MS` has passed since today's scheduled time.
/// Callers have already checked `is_due`, which also keeps a routine the primary ran today
/// (a synced run) from being run again. A routine scheduled so late that scheduled time plus the
/// grace falls after midnight never qualifies: the next day is a new day.
pub fn failover_eligible(routine: &Routine, now_unix_ms: i64, now_local: DateTime<Local>) -> bool {
    matches!(
        routine.template,
        memory::RoutineTemplate::DailyBrief | memory::RoutineTemplate::ReplyPriorities
    ) && scheduled_start_unix_ms(&routine.schedule_hhmm, now_local)
        .is_some_and(|start| now_unix_ms >= start + FAILOVER_GRACE_MS)
}

fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let t = NaiveTime::parse_from_str(s, "%H:%M").ok()?;
    Some((t.hour(), t.minute()))
}

use serde::Serialize;

/// One calendar event, minimal — just enough for "what's on today" framing in the
/// brief. Swift supplies these (EventKit isn't reachable from Rust) via
/// `whimbrel_routines_tick`'s `today_events_json` parameter.
#[derive(Debug, Clone, Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct CalendarEventSummary {
    pub title: String,
    pub start_unix_ms: i64,
}

/// The full structured payload persisted as `RoutineRun.output_json` — richer than
/// `output_summary` (which is just the short notification/inbox-preview text).
#[derive(Debug, Clone, Serialize)]
struct DailyBriefOutput {
    digest_summary: String,
    open_loops: Vec<String>,
    today_events: Vec<CalendarEventSummary>,
}

/// A `run_daily_brief` call's outcome: the row ready to persist (routine_id left as `0`
/// — see this task's test comment) plus the real token usage for the caller to log via
/// `core/ffi`'s existing `log_usage_for` (mirroring every other governed LLM call).
pub struct RoutineRunOutcome {
    pub new_run: memory::NewRoutineRun,
    pub token_usage: llm::TokenUsage,
}

/// FR-RTN-2's own example: yesterday's timeline digest + today's calendar → one
/// synthesized brief. Reuses `llm::generate_daily_digest_from_context` verbatim
/// (already short-circuits to a no-LLM-call placeholder on an empty day) rather than
/// duplicating its prompt-building logic.
pub fn run_daily_brief(
    store: &memory::Store,
    llm_client: &dyn llm::LlmClient,
    yesterday_start_unix_ms: i64,
    yesterday_end_unix_ms: i64,
    today_events: &[CalendarEventSummary],
    ran_at_unix_ms: i64,
    custom_instructions: &str,
) -> Result<RoutineRunOutcome, llm::LlmError> {
    let items = llm::gather_and_assemble_context_for_window(
        store,
        "",
        ran_at_unix_ms,
        yesterday_start_unix_ms,
        yesterday_end_unix_ms,
        20,
        true,
    );
    let digest = llm::generate_daily_digest_from_context(
        llm_client,
        &items,
        custom_instructions,
        yesterday_start_unix_ms,
    )?;

    let events_line = if today_events.is_empty() {
        "No events on today's calendar.".to_string()
    } else {
        let titles: Vec<&str> = today_events.iter().map(|e| e.title.as_str()).collect();
        format!("Today: {}.", titles.join(", "))
    };
    let output_summary = format!("{} {}", digest.summary, events_line);

    let output = DailyBriefOutput {
        digest_summary: digest.summary,
        open_loops: digest.open_loops,
        today_events: today_events.to_vec(),
    };
    let output_json = serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_string());

    Ok(RoutineRunOutcome {
        new_run: memory::NewRoutineRun {
            routine_id: 0,
            ran_at_unix_ms,
            status: memory::RunStatus::Success,
            output_summary,
            output_json,
        },
        token_usage: digest.token_usage,
    })
}

/// Deterministic — no LLM call, no `Store` access. Focus Time exists specifically to
/// exercise the Calendar write path with the simplest possible real draft. Scoped to
/// a plain daily recurring block for this pass (see this plan's Global Constraints
/// for why day-of-week recurrence is deferred).
pub fn build_focus_time_draft(
    block_start_unix_ms: i64,
    block_end_unix_ms: i64,
    ran_at_unix_ms: i64,
) -> memory::NewPendingRoutineRun {
    let action = agent::PendingWriteAction::create_calendar_event(
        "Focus Time",
        block_start_unix_ms,
        block_end_unix_ms,
    );
    memory::NewPendingRoutineRun {
        routine_id: 0, // caller (core/ffi) fills in the real routine_id before persisting -- same convention as run_daily_brief's NewRoutineRun
        ran_at_unix_ms,
        output_summary: action.description,
        pending_action_json: action.payload_json,
    }
}

/// FR-RTN-4's end-of-day recap. Reuses `llm::generate_daily_digest_from_context`
/// exactly like `run_daily_brief` does, but over *today's* activity (not
/// yesterday's) -- the caller (core/ffi) computes today's own UTC-ms bounds the same way it already
/// computes yesterday's for the daily brief. When `gmail_connected` is true and a
/// `recipient_email` is supplied, additionally drafts an email delivery of the same
/// recap -- this is the write action Phase B's approval gate exists to protect;
/// notification/inbox delivery of the recap itself is unconditional and ungated,
/// matching every other routine's delivery in this app.
pub struct EodRecapOutcome {
    pub new_run: memory::NewRoutineRun,
    pub token_usage: llm::TokenUsage,
    pub email_draft: Option<memory::NewPendingRoutineRun>,
}

pub fn run_eod_recap(
    store: &memory::Store,
    llm_client: &dyn llm::LlmClient,
    today_start_unix_ms: i64,
    today_end_unix_ms: i64,
    ran_at_unix_ms: i64,
    gmail_connected: bool,
    recipient_email: Option<&str>,
    custom_instructions: &str,
) -> Result<EodRecapOutcome, llm::LlmError> {
    let items = llm::gather_and_assemble_context_for_window(
        store,
        "",
        ran_at_unix_ms,
        today_start_unix_ms,
        today_end_unix_ms,
        20,
        true,
    );
    let digest = llm::generate_daily_digest_from_context(
        llm_client,
        &items,
        custom_instructions,
        today_start_unix_ms,
    )?;

    let output_json = serde_json::to_string(&digest).unwrap_or_else(|_| "{}".to_string());
    let new_run = memory::NewRoutineRun {
        routine_id: 0, // caller fills in the real routine_id before persisting
        ran_at_unix_ms,
        status: memory::RunStatus::Success,
        output_summary: digest.summary.clone(),
        output_json,
    };

    let email_draft = match (gmail_connected, recipient_email) {
        (true, Some(email)) => {
            let action = agent::PendingWriteAction::send_email(
                email,
                "EOD Recap",
                &recap_email_body(&digest),
            );
            Some(memory::NewPendingRoutineRun {
                routine_id: 0, // caller fills in
                ran_at_unix_ms,
                output_summary: action.description,
                pending_action_json: action.payload_json,
            })
        }
        _ => None,
    };

    Ok(EodRecapOutcome {
        new_run,
        token_usage: digest.token_usage,
        email_draft,
    })
}

/// Appends `digest.open_loops` (dropped otherwise -- `digest.summary` alone was the
/// original, too-thin email body) as a plain-text list under the recap's own narrative.
/// Shared by `run_eod_recap` and `run_weekly_digest`.
fn recap_email_body(digest: &llm::DailyDigest) -> String {
    if digest.open_loops.is_empty() {
        digest.summary.clone()
    } else {
        let open_loops = digest
            .open_loops
            .iter()
            .map(|loop_item| format!("- {loop_item}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("{}\n\nOpen loops:\n{open_loops}", digest.summary)
    }
}

/// FR-RTN-4's weekly counterpart to `run_eod_recap` -- same shape (unconditional digest
/// persistence, gated email delivery), but over a 7-day window via
/// `llm::generate_weekly_digest_from_context`'s richer weekly-reflection prompt. Reuses
/// `EodRecapOutcome` verbatim (identical fields; naming it "Eod"-specific would only cost
/// a rename, not add anything) rather than defining a duplicate struct. `routines::is_due`
/// is what actually restricts this to firing once a week -- this function itself just
/// summarizes whatever window it's given.
pub fn run_weekly_digest(
    store: &memory::Store,
    llm_client: &dyn llm::LlmClient,
    week_start_unix_ms: i64,
    week_end_unix_ms: i64,
    ran_at_unix_ms: i64,
    gmail_connected: bool,
    recipient_email: Option<&str>,
    custom_instructions: &str,
) -> Result<EodRecapOutcome, llm::LlmError> {
    let items = llm::gather_and_assemble_context_for_window(
        store,
        "",
        ran_at_unix_ms,
        week_start_unix_ms,
        week_end_unix_ms,
        60,
        true,
    );
    let digest = llm::generate_weekly_digest_from_context(
        llm_client,
        &items,
        custom_instructions,
        week_start_unix_ms,
        week_end_unix_ms,
    )?;

    let output_json = serde_json::to_string(&digest).unwrap_or_else(|_| "{}".to_string());
    let new_run = memory::NewRoutineRun {
        routine_id: 0, // caller fills in the real routine_id before persisting
        ran_at_unix_ms,
        status: memory::RunStatus::Success,
        output_summary: digest.summary.clone(),
        output_json,
    };

    let email_draft = match (gmail_connected, recipient_email) {
        (true, Some(email)) => {
            let action = agent::PendingWriteAction::send_email(
                email,
                "Weekly Digest",
                &recap_email_body(&digest),
            );
            Some(memory::NewPendingRoutineRun {
                routine_id: 0, // caller fills in
                ran_at_unix_ms,
                output_summary: action.description,
                pending_action_json: action.payload_json,
            })
        }
        _ => None,
    };

    Ok(EodRecapOutcome {
        new_run,
        token_usage: digest.token_usage,
        email_draft,
    })
}

/// How many ranked threads a Reply Priorities run lists.
const REPLY_PRIORITIES_LIMIT: usize = 10;

/// Deterministic, no model call: lists the scored threads waiting on the user, highest
/// priority first. Core cannot see the app's on/off toggle, so "off" and "nothing scored
/// yet" share one message.
pub fn run_reply_priorities(store: &memory::Store, ran_at_unix_ms: i64) -> memory::NewRoutineRun {
    let scored = store.scored_waiting_threads(REPLY_PRIORITIES_LIMIT).unwrap_or_default();
    let (output_summary, threads_json) = if scored.is_empty() {
        (
            "Nothing to rank yet. Turn on Rank email by reply priority in AI & Routing settings.".to_string(),
            Vec::new(),
        )
    } else {
        let names: Vec<String> = scored
            .iter()
            .map(|(t, _)| {
                let subject = t.subject.clone().unwrap_or_else(|| "(no subject)".to_string());
                format!("{subject} ({})", t.last_message_from)
            })
            .collect();
        let json = scored
            .iter()
            .map(|(t, p)| {
                serde_json::json!({
                    "thread_id": t.id,
                    "subject": t.subject,
                    "sender": t.last_message_from,
                    "priority": p,
                })
            })
            .collect();
        (
            format!("{} threads look like they need a reply, most likely first: {}.", scored.len(), names.join("; ")),
            json,
        )
    };
    memory::NewRoutineRun {
        routine_id: 0,
        ran_at_unix_ms,
        status: memory::RunStatus::Success,
        output_summary,
        output_json: serde_json::json!({ "threads": threads_json }).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn routine_for(template: memory::RoutineTemplate, hhmm: &str) -> Routine {
        Routine { template, ..routine(hhmm, true) }
    }

    fn local_at(h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 8, 3, h, mi, 0).unwrap()
    }

    #[test]
    fn failover_waits_three_hours_past_the_scheduled_time() {
        let r = routine_for(memory::RoutineTemplate::DailyBrief, "08:30");
        let before = local_at(11, 29);
        let at = local_at(11, 30);
        assert!(!failover_eligible(&r, before.timestamp_millis(), before));
        assert!(failover_eligible(&r, at.timestamp_millis(), at));
    }

    #[test]
    fn only_the_read_only_templates_may_fail_over() {
        let now = local_at(23, 0);
        for t in [
            memory::RoutineTemplate::FocusTime,
            memory::RoutineTemplate::EodRecap,
            memory::RoutineTemplate::WeeklyDigest,
            memory::RoutineTemplate::MeetingPrep,
        ] {
            assert!(!failover_eligible(&routine_for(t.clone(), "08:30"), now.timestamp_millis(), now), "{t:?}");
        }
        assert!(failover_eligible(&routine_for(memory::RoutineTemplate::ReplyPriorities, "08:30"), now.timestamp_millis(), now));
    }

    #[test]
    fn a_malformed_schedule_never_fails_over() {
        let now = local_at(23, 0);
        assert!(!failover_eligible(&routine_for(memory::RoutineTemplate::DailyBrief, "nope"), now.timestamp_millis(), now));
    }

    #[test]
    fn a_schedule_so_late_that_the_grace_passes_midnight_never_fails_over() {
        let r = routine_for(memory::RoutineTemplate::DailyBrief, "22:30");
        let late = local_at(23, 59);
        assert!(!failover_eligible(&r, late.timestamp_millis(), late));
    }

    fn routine(schedule_hhmm: &str, enabled: bool) -> Routine {
        Routine {
            id: 1,
            name: "Daily Brief".to_string(),
            template: memory::RoutineTemplate::DailyBrief,
            schedule_hhmm: schedule_hhmm.to_string(),
            enabled,
            auto_approve_write_actions: false,
            created_at_unix_ms: 0,
        }
    }

    fn weekly_digest_routine(schedule_hhmm: &str, enabled: bool) -> Routine {
        Routine {
            id: 2,
            name: "Weekly Digest".to_string(),
            template: memory::RoutineTemplate::WeeklyDigest,
            schedule_hhmm: schedule_hhmm.to_string(),
            enabled,
            auto_approve_write_actions: false,
            created_at_unix_ms: 0,
        }
    }

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    struct StubLlmClient {
        response: &'static str,
    }

    impl llm::LlmClient for StubLlmClient {
        fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            Ok(llm::Completion {
                text: self.response.to_string(),
                token_usage: llm::TokenUsage {
                    input_tokens: Some(10),
                    output_tokens: Some(5),
                },
            })
        }
    }

    struct PromptCapturingStubLlmClient {
        response: &'static str,
        last_prompt: std::cell::RefCell<String>,
    }

    impl llm::LlmClient for PromptCapturingStubLlmClient {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            *self.last_prompt.borrow_mut() = prompt.to_string();
            Ok(llm::Completion {
                text: self.response.to_string(),
                token_usage: llm::TokenUsage {
                    input_tokens: Some(10),
                    output_tokens: Some(5),
                },
            })
        }
    }

    #[test]
    fn run_daily_brief_includes_a_routine_run_from_inside_the_window() {
        let store = memory::Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&memory::NewRoutine {
                name: "Build Watcher".to_string(),
                template: memory::RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        store
            .insert_routine_run(&memory::NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                status: memory::RunStatus::Success,
                output_summary: "cargo build succeeded with zero errors".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        let client = PromptCapturingStubLlmClient {
            response: r#"{"summary":"Build succeeded.","open_loops":[]}"#,
            last_prompt: std::cell::RefCell::new(String::new()),
        };

        run_daily_brief(&store, &client, 0, 2_000, &[], 50_000, "").unwrap();

        let prompt = client.last_prompt.borrow();
        assert!(
            prompt.contains("cargo build succeeded with zero errors"),
            "expected the routine run seeded inside the window to reach the digest prompt, got: {prompt}"
        );
    }

    #[test]
    fn run_daily_brief_folds_yesterdays_digest_and_todays_events_into_one_summary() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "Some Window".to_string(),
                url: None,
                text: "shipped the routines scheduler".to_string(),
                hash: 1,
                source: "ax".to_string(),
            })
            .unwrap();
        let client = StubLlmClient {
            response: r#"{"summary":"Shipped the routines scheduler.","open_loops":["Follow up on Task 5"]}"#,
        };
        let today_events = vec![CalendarEventSummary {
            title: "Standup".to_string(),
            start_unix_ms: 100_000,
        }];

        let outcome =
            run_daily_brief(&store, &client, 0, 2_000, &today_events, 50_000, "").unwrap();

        assert_eq!(outcome.new_run.routine_id, 0); // caller fills in the real routine_id before persisting -- see Task 4
        assert_eq!(outcome.new_run.ran_at_unix_ms, 50_000);
        assert_eq!(outcome.new_run.status, memory::RunStatus::Success);
        assert!(outcome
            .new_run
            .output_summary
            .contains("Shipped the routines scheduler"));
        assert!(outcome.new_run.output_summary.contains("Standup"));
        assert!(outcome.new_run.output_json.contains("Follow up on Task 5"));
        assert!(outcome.new_run.output_json.contains("Standup"));
        assert_eq!(outcome.token_usage.input_tokens, Some(10));
    }

    #[test]
    fn run_daily_brief_handles_no_events_today_and_no_activity_yesterday() {
        let store = memory::Store::open_in_memory().unwrap();
        let client = StubLlmClient { response: "unused" }; // generate_daily_digest_from_context short-circuits on empty items, never calls this
        let outcome = run_daily_brief(&store, &client, 0, 2_000, &[], 50_000, "").unwrap();
        assert_eq!(outcome.new_run.status, memory::RunStatus::Success);
        assert!(outcome
            .new_run
            .output_summary
            .contains("No activity captured today."));
    }

    #[test]
    fn run_daily_brief_threads_custom_instructions_into_the_digest_prompt() {
        let store = memory::Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&memory::NewRoutine {
                name: "Build Watcher".to_string(),
                template: memory::RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        store
            .insert_routine_run(&memory::NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                status: memory::RunStatus::Success,
                output_summary: "cargo build succeeded with zero errors".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        let client = PromptCapturingStubLlmClient {
            response: r#"{"summary":"ok","open_loops":[]}"#,
            last_prompt: std::cell::RefCell::new(String::new()),
        };

        run_daily_brief(
            &store,
            &client,
            0,
            2_000,
            &[],
            50_000,
            "Always answer in bullet points.",
        )
        .unwrap();

        let prompt = client.last_prompt.borrow();
        assert!(prompt.contains("Additional instructions: Always answer in bullet points.\n\n"));
    }

    /// #48: the digest prompt must name the actual day being summarized -- yesterday's
    /// window for Daily Brief -- so the model has an explicit basis for ignoring content
    /// that describes a different day (a rolling thread summary, another routine's own
    /// digest of a different window).
    #[test]
    fn run_daily_brief_names_the_summarized_day_in_the_prompt() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "dev-a".to_string(),
                ts_unix_ms: 1_500,
                app_bundle_id: "com.apple.Terminal".to_string(),
                window_title: "Terminal".to_string(),
                url: None,
                text: "cargo build".to_string(),
                hash: 1,
                source: "screen".to_string(),
            })
            .unwrap();
        let client = PromptCapturingStubLlmClient {
            response: r#"{"summary":"ok","open_loops":[]}"#,
            last_prompt: std::cell::RefCell::new(String::new()),
        };

        run_daily_brief(&store, &client, 0, 2_000, &[], 50_000, "").unwrap();

        let prompt = client.last_prompt.borrow();
        assert!(
            prompt.contains("The day being summarized is"),
            "expected the digest prompt to name the day it's actually about, got: {prompt}"
        );
    }

    #[test]
    fn not_due_before_the_scheduled_time() {
        let r = routine("08:30", true);
        assert!(!is_due(&r, local(2026, 8, 3, 8, 29), None));
    }

    #[test]
    fn due_exactly_at_the_scheduled_time() {
        let r = routine("08:30", true);
        assert!(is_due(&r, local(2026, 8, 3, 8, 30), None));
    }

    #[test]
    fn due_after_the_scheduled_time_if_not_yet_run_today() {
        let r = routine("08:30", true);
        assert!(is_due(&r, local(2026, 8, 3, 14, 0), None));
    }

    #[test]
    fn not_due_again_the_same_local_date_it_already_ran() {
        let r = routine("08:30", true);
        let today = local(2026, 8, 3, 9, 0).date_naive();
        assert!(!is_due(&r, local(2026, 8, 3, 20, 0), Some(today)));
    }

    #[test]
    fn due_again_on_a_new_local_date_even_if_it_ran_yesterday() {
        let r = routine("08:30", true);
        let yesterday = local(2026, 8, 2, 9, 0).date_naive();
        assert!(is_due(&r, local(2026, 8, 3, 8, 30), Some(yesterday)));
    }

    #[test]
    fn never_due_when_disabled() {
        let r = routine("08:30", false);
        assert!(!is_due(&r, local(2026, 8, 3, 20, 0), None));
    }

    #[test]
    fn never_due_with_a_malformed_schedule() {
        let r = routine("not-a-time", true);
        assert!(!is_due(&r, local(2026, 8, 3, 20, 0), None));
    }

    #[test]
    fn weekly_digest_is_due_on_sunday_at_or_after_its_scheduled_time() {
        let r = weekly_digest_routine("17:30", true);
        // 2026-08-02 is a Sunday.
        assert!(is_due(&r, local(2026, 8, 2, 17, 30), None));
    }

    #[test]
    fn weekly_digest_is_not_due_on_any_other_day_even_past_its_scheduled_time() {
        let r = weekly_digest_routine("17:30", true);
        // 2026-08-03 is a Monday.
        assert!(!is_due(&r, local(2026, 8, 3, 20, 0), None));
    }

    #[test]
    fn force_due_bypasses_weekday_time_and_same_day_dedup_checks() {
        // 2026-08-03 is a Monday, well before 17:30, and it "already ran today" --
        // every ordinary is_due check below would say false.
        let r = weekly_digest_routine("17:30", true);
        let today = local(2026, 8, 3, 9, 0).date_naive();
        assert!(is_due_inner(&r, local(2026, 8, 3, 9, 0), Some(today), true));
    }

    #[test]
    fn force_due_still_respects_enabled() {
        let r = weekly_digest_routine("17:30", false);
        assert!(!is_due_inner(&r, local(2026, 8, 2, 17, 30), None, true));
    }

    #[test]
    fn scheduled_start_is_the_schedules_own_wall_clock_time_not_the_tick_time() {
        // A tick that fires at 09:00:47 (or a user who approves the draft at 15:00) must
        // still produce a block that starts at the scheduled 09:00.
        let late_tick = local(2026, 8, 3, 15, 0);
        let start = scheduled_start_unix_ms("09:00", late_tick).unwrap();
        assert_eq!(start, local(2026, 8, 3, 9, 0).timestamp_millis());
    }

    #[test]
    fn scheduled_start_uses_the_local_date_of_the_tick() {
        let start = scheduled_start_unix_ms("09:30", local(2026, 12, 25, 23, 59)).unwrap();
        assert_eq!(start, local(2026, 12, 25, 9, 30).timestamp_millis());
    }

    #[test]
    fn scheduled_start_rejects_a_malformed_schedule() {
        assert!(scheduled_start_unix_ms("not-a-time", local(2026, 8, 3, 9, 0)).is_none());
        assert!(scheduled_start_unix_ms("25:00", local(2026, 8, 3, 9, 0)).is_none());
    }

    #[test]
    fn build_focus_time_draft_produces_a_pending_calendar_event_action() {
        let new_run = build_focus_time_draft(1_000, 2_000, 500);
        assert_eq!(new_run.ran_at_unix_ms, 500);
        assert!(new_run.output_summary.contains("Focus Time"));
        let payload: agent::WriteActionPayload =
            serde_json::from_str(&new_run.pending_action_json).unwrap();
        match payload {
            agent::WriteActionPayload::CreateCalendarEvent {
                title,
                start_unix_ms,
                end_unix_ms,
            } => {
                assert_eq!(title, "Focus Time");
                assert_eq!(start_unix_ms, 1_000);
                assert_eq!(end_unix_ms, 2_000);
            }
            _ => panic!("expected CreateCalendarEvent"),
        }
    }

    #[test]
    fn run_eod_recap_without_gmail_produces_no_email_draft() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "Some Window".to_string(),
                url: None,
                text: "wrapped up the eod recap template".to_string(),
                hash: 1,
                source: "ax".to_string(),
            })
            .unwrap();
        let client = StubLlmClient {
            response: r#"{"summary":"Wrapped up the EOD recap template.","open_loops":[]}"#,
        };

        let outcome = run_eod_recap(&store, &client, 0, 2_000, 50_000, false, None, "").unwrap();

        assert_eq!(outcome.new_run.status, memory::RunStatus::Success);
        assert!(outcome
            .new_run
            .output_summary
            .contains("Wrapped up the EOD recap template"));
        assert!(outcome.email_draft.is_none());
    }

    #[test]
    fn run_eod_recap_with_gmail_connected_also_drafts_an_email() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Mail".to_string(),
                window_title: "Some Window".to_string(),
                url: None,
                text: "quiet day wrapping up".to_string(),
                hash: 2,
                source: "ax".to_string(),
            })
            .unwrap();
        let client = StubLlmClient {
            response: r#"{"summary":"Quiet day.","open_loops":[]}"#,
        };

        let outcome = run_eod_recap(
            &store,
            &client,
            0,
            2_000,
            50_000,
            true,
            Some("me@example.com"),
            "",
        )
        .unwrap();

        assert_eq!(outcome.new_run.status, memory::RunStatus::Success);
        let email_draft = outcome
            .email_draft
            .expect("gmail_connected=true with a recipient must produce a draft");
        assert!(email_draft.output_summary.contains("me@example.com"));
        let payload: agent::WriteActionPayload =
            serde_json::from_str(&email_draft.pending_action_json).unwrap();
        match payload {
            agent::WriteActionPayload::SendEmail { to, body, .. } => {
                assert_eq!(to, "me@example.com");
                assert!(body.contains("Quiet day."));
            }
            _ => panic!("expected SendEmail"),
        }
    }

    #[test]
    fn run_eod_recap_email_includes_open_loops() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Mail".to_string(),
                window_title: "Some Window".to_string(),
                url: None,
                text: "shipped the release".to_string(),
                hash: 4,
                source: "ax".to_string(),
            })
            .unwrap();
        let client = StubLlmClient {
            response: r#"{"summary":"Shipped 0.5.5.","open_loops":["waiting on notarization","decide Stripe account"]}"#,
        };

        let outcome = run_eod_recap(
            &store,
            &client,
            0,
            2_000,
            50_000,
            true,
            Some("me@example.com"),
            "",
        )
        .unwrap();

        let email_draft = outcome.email_draft.expect("expected an email draft");
        let payload: agent::WriteActionPayload =
            serde_json::from_str(&email_draft.pending_action_json).unwrap();
        match payload {
            agent::WriteActionPayload::SendEmail { body, .. } => {
                assert!(body.contains("Shipped 0.5.5."));
                assert!(body.contains("Open loops:"));
                assert!(body.contains("- waiting on notarization"));
                assert!(body.contains("- decide Stripe account"));
            }
            _ => panic!("expected SendEmail"),
        }
    }

    #[test]
    fn run_eod_recap_threads_custom_instructions_into_the_digest_prompt() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "Some Window".to_string(),
                url: None,
                text: "wrapped up the eod recap template".to_string(),
                hash: 3,
                source: "ax".to_string(),
            })
            .unwrap();
        let client = PromptCapturingStubLlmClient {
            response: r#"{"summary":"ok","open_loops":[]}"#,
            last_prompt: std::cell::RefCell::new(String::new()),
        };

        run_eod_recap(
            &store,
            &client,
            0,
            2_000,
            50_000,
            false,
            None,
            "Always answer in bullet points.",
        )
        .unwrap();

        let prompt = client.last_prompt.borrow();
        assert!(prompt.contains("Additional instructions: Always answer in bullet points.\n\n"));
    }

    #[test]
    fn run_weekly_digest_without_gmail_produces_no_email_draft() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "Some Window".to_string(),
                url: None,
                text: "shipped several PRs this week".to_string(),
                hash: 5,
                source: "ax".to_string(),
            })
            .unwrap();
        let client = StubLlmClient {
            response: r#"{"summary":"Busy week.","open_loops":[]}"#,
        };

        let outcome = run_weekly_digest(
            &store,
            &client,
            0,
            7 * 24 * 60 * 60 * 1000,
            50_000,
            false,
            None,
            "",
        )
        .unwrap();

        assert_eq!(outcome.new_run.status, memory::RunStatus::Success);
        assert!(outcome.new_run.output_summary.contains("Busy week."));
        assert!(outcome.email_draft.is_none());
    }

    #[test]
    fn run_weekly_digest_with_gmail_connected_drafts_an_email_with_open_loops() {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "test-device".to_string(),
                ts_unix_ms: 1_000,
                app_bundle_id: "com.apple.Mail".to_string(),
                window_title: "Some Window".to_string(),
                url: None,
                text: "shipped several PRs this week".to_string(),
                hash: 6,
                source: "ax".to_string(),
            })
            .unwrap();
        let client = StubLlmClient {
            response: r#"{"summary":"Busy week.","open_loops":["lock the demo date"]}"#,
        };

        let outcome = run_weekly_digest(
            &store,
            &client,
            0,
            7 * 24 * 60 * 60 * 1000,
            50_000,
            true,
            Some("me@example.com"),
            "",
        )
        .unwrap();

        let email_draft = outcome.email_draft.expect("expected an email draft");
        let payload: agent::WriteActionPayload =
            serde_json::from_str(&email_draft.pending_action_json).unwrap();
        match payload {
            agent::WriteActionPayload::SendEmail { to, subject, body } => {
                assert_eq!(to, "me@example.com");
                assert_eq!(subject, "Weekly Digest");
                assert!(body.contains("Busy week."));
                assert!(body.contains("- lock the demo date"));
            }
            _ => panic!("expected SendEmail"),
        }
    }

    fn store_with_scored_threads(scores: &[(&str, f64)]) -> memory::Store {
        let store = memory::Store::open_in_memory().unwrap();
        let items: Vec<memory::NewEmailItem> = scores
            .iter()
            .enumerate()
            .map(|(i, (subject, _))| memory::NewEmailItem {
                device_id: "dev-1".into(),
                external_id: format!("m{i}"),
                ts_unix_ms: 1000 + i as i64,
                from: "Ben <ben@alphasights.com>".into(),
                to: "me@example.com".into(),
                subject: subject.to_string(),
                body_text: "hi".into(),
                gmail_thread_id: format!("t{i}"),
                account_email: "me@example.com".into(),
                ..Default::default()
            })
            .collect();
        store.upsert_email_items(&items).unwrap();
        store.run_jobs_batch(10, 5_000).unwrap();
        for (i, (_, asks_reply)) in scores.iter().enumerate() {
            let thread = store
                .threads_needing_attention(50)
                .unwrap()
                .into_iter()
                .find(|t| t.external_thread_id == format!("t{i}"))
                .unwrap();
            store
                .save_thread_priority(
                    thread.id,
                    thread.message_count,
                    &memory::priority::PriorityJudgments { asks_reply: *asks_reply, urgency: 3.0, person: 0.0, bulk: 0.0, automated: 0.0 },
                    1,
                )
                .unwrap();
        }
        store
    }

    #[test]
    fn reply_priorities_lists_the_highest_scored_threads_first() {
        let store = store_with_scored_threads(&[("Low", 0.2), ("High", 0.9), ("Mid", 0.5)]);
        let run = run_reply_priorities(&store, 9_000);

        assert_eq!(run.status, memory::RunStatus::Success);
        assert!(run.output_summary.starts_with("3 threads"), "{}", run.output_summary);
        let high = run.output_summary.find("High").unwrap();
        let low = run.output_summary.find("Low").unwrap();
        assert!(high < low, "{}", run.output_summary);
        let json: serde_json::Value = serde_json::from_str(&run.output_json).unwrap();
        assert_eq!(json["threads"][0]["subject"], "High");
        assert!(json["threads"][0]["priority"].as_f64().unwrap() > json["threads"][2]["priority"].as_f64().unwrap());
    }

    #[test]
    fn reply_priorities_with_nothing_scored_says_how_to_turn_it_on() {
        let store = memory::Store::open_in_memory().unwrap();
        let run = run_reply_priorities(&store, 9_000);
        assert_eq!(run.status, memory::RunStatus::Success);
        assert!(run.output_summary.contains("Nothing to rank yet"), "{}", run.output_summary);
        assert!(run.output_summary.contains("Rank email by reply priority"));
        assert_eq!(run.output_json, "{\"threads\":[]}");
    }
}
