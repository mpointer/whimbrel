//! The routine write-action round trip (M3.0 part 2, Task 7), moved from the
//! claim → build tool call → finalize/release logic in apps/macos
//! `RoutineScheduler.executeWriteAction`. Three phases, per ADR 0001: [`plan`] claims
//! the run and builds the connector call (Store only), the shell makes the MCP call
//! (network, no Store), then [`settle`] finalizes or releases (Store only). The 60 s
//! timer, token resolution, the `McpHost` call and notifications stay in the shell.

use agent::WriteActionPayload;
use memory::{RunStatus, Store};

/// Everything the shell needs to make one connector call, plus the strings the
/// outcome is recorded with (kept here so both shells finalize identically).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlannedAction {
    pub run_id: i64,
    /// `"google-calendar"` or `"gmail"`: which connector script to spawn.
    pub connector: String,
    pub tool: String,
    pub args_json: String,
    /// The env var the connector process reads its OAuth access token from.
    pub token_env: String,
    /// Surfaced when the shell has no usable token for `connector`.
    pub no_token_message: String,
    pub success_summary: String,
    pub terminal_failure_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PlanResult {
    /// Already claimed by another approval, finalized, or rejected: a benign race.
    NotClaimable,
    /// The draft was unreadable; the run has been finalized as failed.
    Undecodable,
    Planned { plan: PlannedAction },
}

/// The real outcome of the shell's connector round trip. `Success` and
/// `TerminalFailure` mean the request reached Google (finalize); `RetryableFailure`
/// (Node.js missing, spawn/handshake failure) and `NoToken` mean nothing did (release,
/// so the draft survives).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolOutcome {
    Success { result_json: String },
    TerminalFailure,
    RetryableFailure { reason: String },
    NoToken,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Settled {
    pub finalized: bool,
    /// What the shell shows when the run stays pending; `None` on success, on a
    /// terminal failure (already recorded), or when nothing needs surfacing.
    pub user_message: Option<String>,
}

/// Atomically claims `run_id`'s drafted action (only one caller wins, so a
/// double-tapped Approve or an overlapping auto-approve can never send twice) and
/// builds the connector call from the payload.
pub fn plan(store: &Store, run_id: i64) -> rusqlite::Result<PlanResult> {
    let Some(payload_json) = store.claim_pending_action(run_id)? else {
        return Ok(PlanResult::NotClaimable);
    };
    let Ok(payload) = serde_json::from_str::<WriteActionPayload>(&payload_json) else {
        store.finalize_routine_run(run_id, RunStatus::Failed, "Couldn't read the drafted action.".into(), "{}".into())?;
        return Ok(PlanResult::Undecodable);
    };
    Ok(PlanResult::Planned { plan: build_plan(run_id, &payload) })
}

fn build_plan(run_id: i64, payload: &WriteActionPayload) -> PlannedAction {
    match payload {
        WriteActionPayload::CreateCalendarEvent { title, start_unix_ms, end_unix_ms } => PlannedAction {
            run_id,
            connector: "google-calendar".into(),
            tool: "create_event".into(),
            // The connector's create_event schema takes full ISO 8601 datetimes with a
            // timezone (mcp-servers/google-calendar/src/index.ts).
            args_json: serde_json::json!({ "title": title, "start_iso": iso8601(*start_unix_ms), "end_iso": iso8601(*end_unix_ms) }).to_string(),
            token_env: "GOOGLE_CALENDAR_ACCESS_TOKEN".into(),
            no_token_message: "Calendar isn't connected, or needs to be reconnected. Check Settings → Connectors, then try Approve again.".into(),
            success_summary: format!("Added \"{title}\" to your calendar."),
            terminal_failure_summary: "Google rejected the calendar event request.".into(),
        },
        WriteActionPayload::SendEmail { to, subject, body } => PlannedAction {
            run_id,
            connector: "gmail".into(),
            tool: "send_email".into(),
            args_json: serde_json::json!({ "to": to, "subject": subject, "body": body }).to_string(),
            token_env: "GMAIL_ACCESS_TOKEN".into(),
            no_token_message: "Gmail isn't connected, or needs to be reconnected. Check Settings → Connectors, then try Approve again.".into(),
            success_summary: format!("Sent \"{subject}\" to {to}."),
            terminal_failure_summary: "Google rejected the email send request.".into(),
        },
    }
}

fn iso8601(unix_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(unix_ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Records the outcome: finalize on success / terminal failure, release back to
/// `PendingApproval` otherwise (claiming never clears the draft, so nothing is lost).
pub fn settle(store: &Store, plan: &PlannedAction, outcome: &ToolOutcome) -> rusqlite::Result<Settled> {
    match outcome {
        ToolOutcome::Success { result_json } => {
            store.finalize_routine_run(plan.run_id, RunStatus::Success, plan.success_summary.clone(), result_json.clone())?;
            Ok(Settled { finalized: true, user_message: None })
        }
        ToolOutcome::TerminalFailure => {
            store.finalize_routine_run(plan.run_id, RunStatus::Failed, plan.terminal_failure_summary.clone(), "{}".into())?;
            Ok(Settled { finalized: true, user_message: None })
        }
        ToolOutcome::RetryableFailure { reason } => {
            store.release_claimed_routine_run(plan.run_id)?;
            Ok(Settled {
                finalized: false,
                user_message: Some(format!("Couldn't complete this action just now ({reason}). It's still pending -- try Approve again in a bit.")),
            })
        }
        ToolOutcome::NoToken => {
            store.release_claimed_routine_run(plan.run_id)?;
            Ok(Settled { finalized: false, user_message: Some(plan.no_token_message.clone()) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::{NewPendingRoutineRun, NewRoutine, RoutineTemplate};

    fn seed_pending_run(store: &Store, payload_json: &str) -> i64 {
        let routine_id = store
            .insert_routine(&NewRoutine { name: "EOD".into(), template: RoutineTemplate::EodRecap, schedule_hhmm: "17:00".into(), enabled: true })
            .unwrap();
        store
            .insert_pending_routine_run(&NewPendingRoutineRun { routine_id, ran_at_unix_ms: 1, output_summary: "draft".into(), pending_action_json: payload_json.into() })
            .unwrap()
    }

    fn fetch_run(store: &Store, run_id: i64) -> memory::RoutineRun {
        store.search_routine_runs("", 100).unwrap().into_iter().find(|r| r.id == run_id).unwrap()
    }

    fn status(store: &Store, run_id: i64) -> RunStatus {
        fetch_run(store, run_id).status
    }

    fn planned(result: PlanResult) -> PlannedAction {
        match result {
            PlanResult::Planned { plan } => plan,
            other => panic!("expected planned, got {other:?}"),
        }
    }

    #[test]
    fn plan_claims_and_maps_a_calendar_payload_to_create_event() {
        let store = Store::open_in_memory().unwrap();
        let run_id = seed_pending_run(&store, r#"{"kind":"create_calendar_event","title":"Sync \"quotes\"","start_unix_ms":1780000000000,"end_unix_ms":1780003600000}"#);
        let plan = planned(plan(&store, run_id).unwrap());
        assert_eq!(status(&store, run_id), RunStatus::Executing, "claimed");
        assert_eq!(plan.connector, "google-calendar");
        assert_eq!(plan.tool, "create_event");
        assert_eq!(plan.token_env, "GOOGLE_CALENDAR_ACCESS_TOKEN");
        let args: serde_json::Value = serde_json::from_str(&plan.args_json).unwrap();
        assert_eq!(args["title"], "Sync \"quotes\"");
        assert_eq!(args["start_iso"], "2026-05-28T20:26:40Z");
        assert_eq!(args["end_iso"], "2026-05-28T21:26:40Z");
        assert_eq!(plan.success_summary, "Added \"Sync \"quotes\"\" to your calendar.");
        assert!(plan.no_token_message.starts_with("Calendar isn't connected"));
        assert!(matches!(super::plan(&store, run_id).unwrap(), PlanResult::NotClaimable), "second claim loses");
    }

    #[test]
    fn plan_maps_an_email_payload_to_send_email() {
        let store = Store::open_in_memory().unwrap();
        let run_id = seed_pending_run(&store, r#"{"kind":"send_email","to":"a@x.com","subject":"Hi\nthere","body":"line1\nline2"}"#);
        let plan = planned(plan(&store, run_id).unwrap());
        assert_eq!((plan.connector.as_str(), plan.tool.as_str(), plan.token_env.as_str()), ("gmail", "send_email", "GMAIL_ACCESS_TOKEN"));
        let args: serde_json::Value = serde_json::from_str(&plan.args_json).unwrap();
        assert_eq!(args, serde_json::json!({"to":"a@x.com","subject":"Hi\nthere","body":"line1\nline2"}));
        assert_eq!(plan.success_summary, "Sent \"Hi\nthere\" to a@x.com.");
        assert_eq!(plan.terminal_failure_summary, "Google rejected the email send request.");
    }

    #[test]
    fn plan_finalizes_an_undecodable_draft_as_failed() {
        let store = Store::open_in_memory().unwrap();
        let run_id = seed_pending_run(&store, r#"{"kind":"teleport"}"#);
        assert!(matches!(plan(&store, run_id).unwrap(), PlanResult::Undecodable));
        let run = fetch_run(&store, run_id);
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.output_summary, "Couldn't read the drafted action.");
        assert!(store.pending_routine_runs().unwrap().is_empty());
    }

    #[test]
    fn settle_success_and_terminal_failure_finalize() {
        let store = Store::open_in_memory().unwrap();
        let a = seed_pending_run(&store, r#"{"kind":"send_email","to":"a@x.com","subject":"S","body":"B"}"#);
        let plan_a = planned(plan(&store, a).unwrap());
        let settled = settle(&store, &plan_a, &ToolOutcome::Success { result_json: r#"{"id":"m1"}"#.into() }).unwrap();
        assert_eq!(settled, Settled { finalized: true, user_message: None });
        let run = fetch_run(&store, a);
        assert_eq!((run.status, run.output_summary.as_str(), run.output_json.as_str()), (RunStatus::Success, "Sent \"S\" to a@x.com.", r#"{"id":"m1"}"#));

        let b = seed_pending_run(&store, r#"{"kind":"send_email","to":"a@x.com","subject":"S","body":"B"}"#);
        let plan_b = planned(plan(&store, b).unwrap());
        assert!(settle(&store, &plan_b, &ToolOutcome::TerminalFailure).unwrap().finalized);
        let run_b = fetch_run(&store, b);
        assert_eq!((run_b.status, run_b.output_summary.as_str()), (RunStatus::Failed, "Google rejected the email send request."));
    }

    #[test]
    fn settle_retryable_failure_and_no_token_release_the_draft_with_a_message() {
        let store = Store::open_in_memory().unwrap();
        let run_id = seed_pending_run(&store, r#"{"kind":"create_calendar_event","title":"T","start_unix_ms":0,"end_unix_ms":1000}"#);
        let plan = planned(plan(&store, run_id).unwrap());
        let settled = settle(&store, &plan, &ToolOutcome::RetryableFailure { reason: "Node.js isn't installed on this Mac.".into() }).unwrap();
        assert!(!settled.finalized);
        assert_eq!(settled.user_message.as_deref(), Some("Couldn't complete this action just now (Node.js isn't installed on this Mac.). It's still pending -- try Approve again in a bit."));
        assert_eq!(status(&store, run_id), RunStatus::PendingApproval);
        // Still approvable: claim again, then a missing token releases it again.
        let plan = planned(super::plan(&store, run_id).unwrap());
        let settled = settle(&store, &plan, &ToolOutcome::NoToken).unwrap();
        assert_eq!(settled.user_message.as_deref(), Some(plan.no_token_message.as_str()));
        assert_eq!(status(&store, run_id), RunStatus::PendingApproval);
        assert_eq!(store.pending_routine_runs().unwrap().len(), 1);
    }

    #[test]
    fn outcome_json_shapes_match_the_header() {
        assert_eq!(serde_json::from_str::<ToolOutcome>(r#"{"kind":"success","result_json":"{}"}"#).unwrap(), ToolOutcome::Success { result_json: "{}".into() });
        assert_eq!(serde_json::from_str::<ToolOutcome>(r#"{"kind":"terminal_failure"}"#).unwrap(), ToolOutcome::TerminalFailure);
        assert_eq!(serde_json::from_str::<ToolOutcome>(r#"{"kind":"retryable_failure","reason":"r"}"#).unwrap(), ToolOutcome::RetryableFailure { reason: "r".into() });
        assert_eq!(serde_json::from_str::<ToolOutcome>(r#"{"kind":"no_token"}"#).unwrap(), ToolOutcome::NoToken);
        let json = serde_json::to_value(PlanResult::NotClaimable).unwrap();
        assert_eq!(json["status"], "not_claimable");
    }
}
