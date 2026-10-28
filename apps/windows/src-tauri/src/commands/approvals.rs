//! Automations → Approvals (M3.4 Task 2): what a routine wants to add or send, waiting for a yes.
//! The round trip itself is `connectors::write`; this is the list the user sees and the two
//! buttons.

use agent::WriteActionPayload;
use memory::Store;
use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::connectors::write::{self, Connectors, Executed};
use crate::error::CommandError;
use crate::state::AppState;

/// What a pending action would do, in the user's terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActionView {
    CreateCalendarEvent {
        title: String,
        #[serde(rename = "startUnixMs")]
        start_unix_ms: i64,
        #[serde(rename = "endUnixMs")]
        end_unix_ms: i64,
    },
    SendEmail {
        to: String,
        subject: String,
        body: String,
    },
}

impl From<&WriteActionPayload> for ActionView {
    fn from(p: &WriteActionPayload) -> Self {
        match p {
            WriteActionPayload::CreateCalendarEvent { title, start_unix_ms, end_unix_ms } => {
                ActionView::CreateCalendarEvent { title: title.clone(), start_unix_ms: *start_unix_ms, end_unix_ms: *end_unix_ms }
            }
            WriteActionPayload::SendEmail { to, subject, body } => ActionView::SendEmail { to: to.clone(), subject: subject.clone(), body: body.clone() },
        }
    }
}

/// The connector directory a payload is carried out by.
fn connector_of(p: &WriteActionPayload) -> &'static str {
    match p {
        WriteActionPayload::CreateCalendarEvent { .. } => "google-calendar",
        WriteActionPayload::SendEmail { .. } => "gmail",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalView {
    pub run_id: i64,
    pub routine_name: String,
    pub ran_at_unix_ms: i64,
    /// The routine's own one-line description of the draft.
    pub summary: String,
    /// `None` when the draft could not be read; it can still be rejected.
    pub action: Option<ActionView>,
    /// The accounts this could be done with, and the one used unless the user picks another.
    pub accounts: Vec<String>,
    pub account: Option<String>,
    /// Why Approve cannot work yet (no account connected), or `None`.
    pub blocked: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApproveResult {
    /// The list as it is now.
    pub approvals: Vec<ApprovalView>,
    /// What to tell the user: why it is still waiting, or that someone else already handled it.
    pub message: Option<String>,
}

fn store_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::store(e.to_string())
}

fn blocked_reason(connector: &str) -> &'static str {
    match connector {
        "google-calendar" => "Connect a Google Calendar account in Settings to approve this.",
        _ => "Connect Gmail in Settings to approve this.",
    }
}

/// Everything waiting for approval, oldest first.
pub fn pending(store: &Store) -> Result<Vec<ApprovalView>, CommandError> {
    let mut runs = store.pending_routine_runs().map_err(store_error)?;
    runs.sort_by_key(|r| r.ran_at_unix_ms);
    runs.into_iter()
        .map(|run| {
            let payload = store
                .pending_action_for_run(run.id)
                .map_err(store_error)?
                .and_then(|json| serde_json::from_str::<WriteActionPayload>(&json).ok());
            let connector = payload.as_ref().map(connector_of);
            let (accounts, account) = match connector {
                Some(c) => {
                    let list = api::connectors::accounts::list(store, write::registry_id(c)).map_err(store_error)?;
                    let chosen = list.default_email.clone().or_else(|| list.emails.first().cloned());
                    (list.emails, chosen)
                }
                None => (Vec::new(), None),
            };
            let blocked = match connector {
                Some(c) if account.is_none() => Some(blocked_reason(c).to_string()),
                _ => None,
            };
            Ok(ApprovalView {
                run_id: run.id,
                routine_name: run.routine_name,
                ran_at_unix_ms: run.ran_at_unix_ms,
                summary: run.output_summary,
                action: payload.as_ref().map(ActionView::from),
                accounts,
                account,
                blocked,
            })
        })
        .collect()
}

pub fn count(store: &Store) -> Result<usize, CommandError> {
    Ok(store.pending_routine_runs().map_err(store_error)?.len())
}

/// Rejecting is final for that draft; one that is already claimed or finished simply is not there.
pub fn reject(store: &Store, run_id: i64) -> Result<(), CommandError> {
    store.reject_routine_run(run_id).map(|_| ()).map_err(store_error)
}

/// Turns what the round trip reported into what the user is told.
pub fn message_for(executed: &Executed) -> Option<String> {
    match executed {
        Executed::NotClaimable => Some("That was already handled.".to_string()),
        Executed::Unreadable => Some("Couldn't read the drafted action, so it was marked as failed.".to_string()),
        Executed::Done { message, .. } => message.clone(),
    }
}

#[tauri::command]
pub async fn approvals_list(state: tauri::State<'_, AppState>) -> Result<Vec<ApprovalView>, CommandError> {
    state.store()?.call(|s| pending(s)).await?
}

#[tauri::command]
pub async fn approvals_count(state: tauri::State<'_, AppState>) -> Result<usize, CommandError> {
    state.store()?.call(|s| count(s)).await?
}

#[tauri::command]
pub async fn approval_reject(state: tauri::State<'_, AppState>, run_id: i64) -> Result<Vec<ApprovalView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            reject(s, run_id)?;
            pending(s)
        })
        .await?
}

/// Approve: carries the action out now. Off the main thread: it spawns a process and waits on the
/// network.
#[tauri::command]
pub async fn approval_approve(app: AppHandle, run_id: i64, account: Option<String>) -> Result<ApproveResult, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let store = state.store()?.clone();
        let connectors = write::Real::new(state.secrets.clone(), state.paths.root.clone());
        let executed = write::execute(&store, run_id, account.as_deref(), &connectors as &dyn Connectors)?;
        let approvals = store.call_blocking(|s| pending(s))??;
        Ok(ApproveResult { approvals, message: message_for(&executed) })
    })
    .await
    .map_err(|e| CommandError::store(format!("the approval failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::{NewPendingRoutineRun, NewRoutine, RoutineTemplate};

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn draft(store: &Store, template: RoutineTemplate, name: &str, at: i64, payload: &str) -> i64 {
        let routine = store.insert_routine(&NewRoutine { name: name.into(), template, schedule_hhmm: "09:00".into(), enabled: true }).unwrap();
        store
            .insert_pending_routine_run(&NewPendingRoutineRun { routine_id: routine, ran_at_unix_ms: at, output_summary: format!("{name} draft"), pending_action_json: payload.into() })
            .unwrap()
    }

    fn focus() -> String {
        serde_json::to_string(&WriteActionPayload::CreateCalendarEvent { title: "Focus".into(), start_unix_ms: 1_000, end_unix_ms: 2_000 }).unwrap()
    }

    fn email() -> String {
        serde_json::to_string(&WriteActionPayload::SendEmail { to: "me@example.com".into(), subject: "Your day".into(), body: "You shipped.".into() }).unwrap()
    }

    #[test]
    fn nothing_waiting_is_an_empty_list_and_a_zero_count() {
        let s = store();
        assert!(pending(&s).unwrap().is_empty());
        assert_eq!(count(&s).unwrap(), 0);
    }

    #[test]
    fn a_draft_is_shown_as_what_it_would_do_with_the_account_it_would_use() {
        let s = store();
        api::connectors::accounts::add(&s, "calendar", "me@example.com", 1).unwrap();
        api::connectors::accounts::add(&s, "calendar", "other@example.com", 2).unwrap();
        let run = draft(&s, RoutineTemplate::FocusTime, "Focus Time", 10, &focus());

        let shown = pending(&s).unwrap();
        assert_eq!(shown.len(), 1);
        let a = &shown[0];
        assert_eq!((a.run_id, a.routine_name.as_str(), a.summary.as_str()), (run, "Focus Time", "Focus Time draft"));
        assert_eq!(a.action, Some(ActionView::CreateCalendarEvent { title: "Focus".into(), start_unix_ms: 1_000, end_unix_ms: 2_000 }));
        assert_eq!(a.accounts, ["me@example.com", "other@example.com"]);
        assert_eq!(a.account.as_deref(), Some("me@example.com"), "the first account added is the default");
        assert_eq!(a.blocked, None);
    }

    #[test]
    fn a_draft_with_no_account_to_do_it_with_says_so_instead_of_offering_a_dead_button() {
        let s = store();
        draft(&s, RoutineTemplate::FocusTime, "Focus Time", 10, &focus());
        draft(&s, RoutineTemplate::EodRecap, "EOD Recap", 20, &email());
        let shown = pending(&s).unwrap();
        assert_eq!(shown[0].blocked.as_deref(), Some("Connect a Google Calendar account in Settings to approve this."));
        assert_eq!(shown[1].blocked.as_deref(), Some("Connect Gmail in Settings to approve this."));
        assert!(shown.iter().all(|a| a.account.is_none() && a.accounts.is_empty()));
    }

    #[test]
    fn an_email_draft_shows_its_recipient_subject_and_full_body() {
        let s = store();
        draft(&s, RoutineTemplate::EodRecap, "EOD Recap", 10, &email());
        assert_eq!(
            pending(&s).unwrap()[0].action,
            Some(ActionView::SendEmail { to: "me@example.com".into(), subject: "Your day".into(), body: "You shipped.".into() })
        );
    }

    #[test]
    fn a_draft_that_cannot_be_read_is_still_listed_so_it_can_be_rejected() {
        let s = store();
        let run = draft(&s, RoutineTemplate::FocusTime, "Focus Time", 10, "not json");
        let shown = pending(&s).unwrap();
        assert_eq!((shown[0].run_id, shown[0].action.is_none(), shown[0].blocked.is_none()), (run, true, true));
        reject(&s, run).unwrap();
        assert!(pending(&s).unwrap().is_empty());
    }

    #[test]
    fn the_oldest_waits_first_and_the_count_matches_the_list() {
        let s = store();
        draft(&s, RoutineTemplate::EodRecap, "Newer", 300, &email());
        draft(&s, RoutineTemplate::FocusTime, "Older", 100, &focus());
        let names: Vec<String> = pending(&s).unwrap().into_iter().map(|a| a.routine_name).collect();
        assert_eq!(names, ["Older", "Newer"]);
        assert_eq!(count(&s).unwrap(), 2);
    }

    #[test]
    fn rejecting_removes_it_from_the_list_and_rejecting_what_is_gone_is_not_an_error() {
        let s = store();
        let run = draft(&s, RoutineTemplate::FocusTime, "Focus Time", 10, &focus());
        reject(&s, run).unwrap();
        assert_eq!(count(&s).unwrap(), 0);
        reject(&s, run).expect("a double click is fine");
        reject(&s, 9999).expect("so is an unknown run");
    }

    #[test]
    fn what_the_user_is_told_after_an_approval() {
        assert_eq!(message_for(&Executed::NotClaimable).as_deref(), Some("That was already handled."));
        assert!(message_for(&Executed::Unreadable).unwrap().contains("marked as failed"));
        assert_eq!(message_for(&Executed::Done { finalized: true, message: None }), None, "success says nothing; the item just leaves the list");
        assert_eq!(message_for(&Executed::Done { finalized: false, message: Some("still pending".into()) }).as_deref(), Some("still pending"));
    }
}
