//! Making a routine's write action real (M3.4 Task 2): the three-phase round trip of
//! `api::write_action` (ADR 0001). `plan` claims the run and builds the connector call under the
//! store, the connector call runs with **no** store held, then `settle` records the outcome.
//!
//! The claim is what makes this safe to call twice for one run (a double-clicked Approve, or the
//! scheduler's auto-approve racing a click): only the call that wins it gets a plan, so an email
//! can never be sent twice. Once a run is claimed, every path below reaches `settle`, so a run is
//! never left stranded mid-flight; a failure that did not reach Google releases the draft so it
//! survives to be retried.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;

use api::oauth;
use api::write_action::{self, PlanResult, PlannedAction, ToolOutcome};
use mcp_host::McpHost;

use crate::connectors::accounts::{self, Kind};
use crate::connectors::node;
use crate::error::CommandError;
use crate::secrets::SecretStore;
use crate::store_actor::StoreHandle;
use crate::syncing::config::SyncConfig;

/// What a calendar write needs from the grant. Google lets a user untick a scope on the consent
/// page, so a connected account can lack it even though Whimbrel asked.
pub const CALENDAR_WRITE_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events";
/// What sending needs. `gmail.compose`, which is what Whimbrel asks for, satisfies it: core's scope
/// helper knows compose is Google's documented superset of send.
pub const GMAIL_SEND_SCOPE: &str = "https://www.googleapis.com/auth/gmail.send";

/// The id core's account registry uses for a connector directory name.
pub fn registry_id(connector: &str) -> &'static str {
    match connector {
        "google-calendar" => accounts::CALENDAR,
        _ => "gmail",
    }
}

/// The two things the round trip needs from outside the store, so tests can stand in for them.
pub trait Connectors: Send + Sync {
    /// An access token for `account` that carries the scope this connector's write needs, or
    /// `None`: not connected, the grant lacks the scope, or the token could not be refreshed.
    /// The user is told to reconnect, never shown a half-working Approve.
    fn token(&self, connector: &str, account: &str) -> Option<String>;

    /// Makes the call. Everything before the call itself (Node, the script, the handshake) is a
    /// retryable failure because nothing reached Google; only the call failing is terminal.
    fn call(&self, plan: &PlannedAction, token: &str) -> ToolOutcome;
}

/// The real thing: Credential Manager tokens, `node`, and the staged connector scripts.
pub struct Real {
    secrets: Arc<dyn SecretStore>,
    /// `None` when the build has no Google client, in which case nothing can be refreshed.
    config: Option<SyncConfig>,
    host: McpHost,
    root: PathBuf,
}

impl Real {
    pub fn new(secrets: Arc<dyn SecretStore>, root: PathBuf) -> Self {
        Real { secrets, config: SyncConfig::embedded().ok(), host: McpHost::new(), root }
    }

    fn run(&self, plan: &PlannedAction, token: &str) -> ToolOutcome {
        let label = if plan.connector == "google-calendar" { "Calendar" } else { "Gmail" };
        let retry = |reason: String| ToolOutcome::RetryableFailure { reason };
        let Some(node_path) = node::node_path() else {
            return retry("Node.js wasn't found. Reinstall Whimbrel.".into());
        };
        let script = match node::script(&self.root, &plan.connector) {
            Ok(script) => script,
            Err(_) => return retry(format!("The {label} connector's files aren't installed. Reinstall Whimbrel, or if you build it from source run apps\\windows\\scripts\\prepare-connectors.ps1.")),
        };
        let id = match self.host.connect(
            &node_path.to_string_lossy(),
            &[script.to_string_lossy().into_owned()],
            &[(plan.token_env.clone(), token.to_string())],
        ) {
            Ok(id) => id,
            Err(e) => return retry(format!("The {label} connector didn't start: {e}")),
        };
        let outcome = match self.host.list_tools(id) {
            Ok(tools) if tools.iter().any(|t| t.name == plan.tool) => match self.host.call_tool(id, &plan.tool, &plan.args_json) {
                Ok(result_json) => ToolOutcome::Success { result_json },
                Err(e) => {
                    // The one step that reaches Google, so its failure is the terminal one.
                    tracing::warn!(tool = %plan.tool, error = %e, "the connector call failed");
                    ToolOutcome::TerminalFailure
                }
            },
            Ok(_) => retry(format!("The {label} connector doesn't offer {}.", plan.tool)),
            Err(e) => retry(format!("The {label} connector didn't list its tools: {e}")),
        };
        let _ = self.host.disconnect(id);
        outcome
    }
}

impl Connectors for Real {
    fn token(&self, connector: &str, account: &str) -> Option<String> {
        let (kind, required) = match connector {
            "google-calendar" => (Kind::Calendar, CALENDAR_WRITE_SCOPE),
            "gmail" => (Kind::Gmail, GMAIL_SEND_SCOPE),
            _ => return None,
        };
        let config = self.config.as_ref()?;
        let mut stored = match accounts::tokens_for(kind, &*self.secrets, account) {
            Ok(stored) => stored,
            Err(e) => {
                tracing::warn!(?kind, error = %e, "no stored tokens for the account");
                return None;
            }
        };
        // A grant whose scope was never recorded is not a refusal: one refresh tells us what Google
        // actually granted, which is what core says a write action does.
        if stored.granted_scope.is_none() {
            let now = chrono::Utc::now().timestamp_millis();
            match oauth::refresh_stored(&accounts::provider_for(kind, config), &stored, now) {
                Ok(refreshed) => {
                    if let Err(e) = accounts::save_tokens_for(kind, &*self.secrets, account, &refreshed) {
                        tracing::warn!(?kind, error = %e, "could not save the refreshed tokens");
                    }
                    stored = refreshed;
                }
                Err(e) => {
                    tracing::warn!(?kind, error = %e, "could not learn which scopes the grant has");
                    return None;
                }
            }
        }
        if !oauth::granted_scope_satisfies(stored.granted_scope.as_deref(), required) {
            tracing::warn!(?kind, "the grant is missing the write scope; reconnect the account in Settings");
            return None;
        }
        match accounts::fresh_access_token_for(kind, &*self.secrets, config, account) {
            Ok(token) => Some(token),
            Err(e) => {
                tracing::warn!(?kind, error = %e, "could not get a fresh access token");
                None
            }
        }
    }

    fn call(&self, plan: &PlannedAction, token: &str) -> ToolOutcome {
        self.run(plan, token)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Executed {
    /// Another approval already has it, or it is finished or rejected: a benign race.
    NotClaimable,
    /// The draft could not be read; the run has been recorded as failed.
    Unreadable,
    Done {
        /// The run is over (sent, or Google refused). `false` means it is still waiting.
        finalized: bool,
        /// What to tell the user when it is still waiting.
        message: Option<String>,
    },
}

fn store_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::store(e.to_string())
}

/// The account to use when the caller does not name one: the default, else the first connected.
fn default_account(store: &StoreHandle, connector: &str) -> Option<String> {
    let registry = registry_id(connector);
    store
        .call_blocking(move |s| api::connectors::accounts::list(s, registry).ok())
        .ok()
        .flatten()
        .and_then(|a| a.default_email.or_else(|| a.emails.first().cloned()))
}

/// Runs one pending run's write action against the real connector. Blocking: it spawns a
/// process and waits on the network, so callers are off the async runtime.
pub fn execute(store: &StoreHandle, run_id: i64, account: Option<&str>, connectors: &dyn Connectors) -> Result<Executed, CommandError> {
    let planned = store.call_blocking(move |s| write_action::plan(s, run_id))?.map_err(store_error)?;
    let plan = match planned {
        PlanResult::NotClaimable => return Ok(Executed::NotClaimable),
        PlanResult::Undecodable => return Ok(Executed::Unreadable),
        PlanResult::Planned { plan } => plan,
    };

    // The run is claimed. Whatever happens next must reach `settle`.
    let chosen = account.map(str::to_string).or_else(|| default_account(store, &plan.connector));
    let attempt = catch_unwind(AssertUnwindSafe(|| match chosen.and_then(|a| connectors.token(&plan.connector, &a)) {
        Some(token) => connectors.call(&plan, &token),
        None => ToolOutcome::NoToken,
    }));
    let outcome = attempt.unwrap_or_else(|_| ToolOutcome::RetryableFailure { reason: "an unexpected error".into() });

    let for_settle = plan.clone();
    let settled = store.call_blocking(move |s| write_action::settle(s, &for_settle, &outcome))?.map_err(store_error)?;
    Ok(Executed::Done { finalized: settled.finalized, message: settled.user_message })
}

/// After a tick: runs every pending write action whose routine is switched on **and** has
/// auto-approve on. A routine the user has since switched off does not act unattended, even if it
/// left a draft behind. Returns how many were attempted.
pub fn auto_approve(store: &StoreHandle, connectors: &dyn Connectors) -> usize {
    let due: Vec<i64> = match store.call_blocking(|s| -> Result<Vec<i64>, String> {
        let routines = s.list_routines().map_err(|e| e.to_string())?;
        let pending = s.pending_routine_runs().map_err(|e| e.to_string())?;
        Ok(pending
            .into_iter()
            .filter(|run| routines.iter().any(|r| r.id == run.routine_id && r.enabled && r.auto_approve_write_actions))
            .map(|run| run.id)
            .collect())
    }) {
        Ok(Ok(ids)) => ids,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "auto-approve: could not read the pending approvals");
            return 0;
        }
        Err(e) => {
            tracing::warn!(error = %e, "auto-approve: the store is unavailable");
            return 0;
        }
    };
    for id in &due {
        match execute(store, *id, None, connectors) {
            Ok(Executed::Done { finalized: true, .. }) => tracing::info!(run = id, "auto-approve: done"),
            Ok(Executed::Done { message, .. }) => tracing::info!(run = id, message = message.as_deref().unwrap_or(""), "auto-approve: left pending"),
            Ok(other) => tracing::info!(run = id, outcome = ?other, "auto-approve: nothing to do"),
            Err(e) => tracing::warn!(run = id, error = %e, "auto-approve: failed"),
        }
    }
    due.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent::WriteActionPayload;
    use memory::{NewPendingRoutineRun, NewRoutine, RoutineTemplate, RunStatus, Store};
    use std::sync::Mutex;

    /// Records what it was asked and answers as scripted.
    struct Fake {
        token: Option<String>,
        outcome: ToolOutcome,
        panics: bool,
        asked: Mutex<Vec<String>>,
        called: Mutex<Vec<(String, String, String)>>,
    }

    impl Fake {
        fn answering(outcome: ToolOutcome) -> Self {
            Fake { token: Some("tok".into()), outcome, panics: false, asked: Mutex::new(vec![]), called: Mutex::new(vec![]) }
        }
        fn success() -> Self {
            Self::answering(ToolOutcome::Success { result_json: "{\"ok\":true}".into() })
        }
    }

    impl Connectors for Fake {
        fn token(&self, _connector: &str, account: &str) -> Option<String> {
            self.asked.lock().unwrap().push(account.to_string());
            self.token.clone()
        }
        fn call(&self, plan: &PlannedAction, token: &str) -> ToolOutcome {
            assert!(!self.panics, "the connector blew up");
            self.called.lock().unwrap().push((plan.tool.clone(), plan.args_json.clone(), token.to_string()));
            self.outcome.clone()
        }
    }

    fn store() -> StoreHandle {
        StoreHandle::spawn(|| Store::open_in_memory().map_err(|e| e.to_string())).unwrap()
    }

    fn focus_event() -> WriteActionPayload {
        WriteActionPayload::CreateCalendarEvent { title: "Focus".into(), start_unix_ms: 1_791_450_000_000, end_unix_ms: 1_791_457_200_000 }
    }

    /// A routine with one pending run holding `payload`; returns (routine id, run id).
    fn pending(store: &StoreHandle, payload: &str, auto: bool, enabled: bool) -> (i64, i64) {
        let payload = payload.to_string();
        store
            .call_blocking(move |s| {
                let routine = s
                    .insert_routine(&NewRoutine { name: "Focus Time".into(), template: RoutineTemplate::FocusTime, schedule_hhmm: "09:00".into(), enabled })
                    .unwrap();
                s.set_routine_auto_approve(routine, auto).unwrap();
                let run = s
                    .insert_pending_routine_run(&NewPendingRoutineRun { routine_id: routine, ran_at_unix_ms: 1, output_summary: "draft".into(), pending_action_json: payload })
                    .unwrap();
                (routine, run)
            })
            .unwrap()
    }

    fn connect(store: &StoreHandle, email: &str) {
        let email = email.to_string();
        store.call_blocking(move |s| api::connectors::accounts::add(s, "calendar", &email, 1).unwrap()).unwrap();
    }

    fn run_state(store: &StoreHandle, run: i64) -> (RunStatus, String) {
        store
            .call_blocking(move |s| {
                let found = s.search_routine_runs("", 100).unwrap().into_iter().find(|r| r.id == run).unwrap();
                (found.status, found.output_summary)
            })
            .unwrap()
    }

    fn payload_json(p: &WriteActionPayload) -> String {
        serde_json::to_string(p).unwrap()
    }

    #[test]
    fn an_approved_calendar_event_is_created_with_the_accounts_token_and_recorded() {
        let store = store();
        connect(&store, "me@example.com");
        let (_, run) = pending(&store, &payload_json(&focus_event()), false, true);
        let fake = Fake::success();

        let done = execute(&store, run, None, &fake).unwrap();
        assert_eq!(done, Executed::Done { finalized: true, message: None });
        let called = fake.called.lock().unwrap();
        assert_eq!(called.len(), 1);
        let (tool, args, token) = &called[0];
        assert_eq!((tool.as_str(), token.as_str()), ("create_event", "tok"));
        assert!(args.contains("\"title\":\"Focus\"") && args.contains("start_iso") && args.contains("end_iso"), "{args}");
        assert_eq!(run_state(&store, run), (RunStatus::Success, "Added \"Focus\" to your calendar.".to_string()));
    }

    #[test]
    fn a_second_approval_of_the_same_run_does_nothing() {
        let store = store();
        connect(&store, "me@example.com");
        let (_, run) = pending(&store, &payload_json(&focus_event()), false, true);
        let fake = Fake::success();
        execute(&store, run, None, &fake).unwrap();
        assert_eq!(execute(&store, run, None, &fake).unwrap(), Executed::NotClaimable);
        assert_eq!(fake.called.lock().unwrap().len(), 1, "it was only ever created once");
    }

    #[test]
    fn an_account_the_user_names_wins_over_the_default_and_with_none_set_the_first_connected_is_used() {
        let store = store();
        connect(&store, "first@example.com");
        connect(&store, "second@example.com");
        let (_, a) = pending(&store, &payload_json(&focus_event()), false, true);
        let (_, b) = pending(&store, &payload_json(&focus_event()), false, true);
        let fake = Fake::success();
        execute(&store, a, Some("second@example.com"), &fake).unwrap();
        execute(&store, b, None, &fake).unwrap();
        assert_eq!(*fake.asked.lock().unwrap(), ["second@example.com", "first@example.com"], "the first account added is the default");
    }

    #[test]
    fn no_account_or_no_token_leaves_the_draft_waiting_and_says_to_reconnect() {
        let store = store();
        let (_, no_account) = pending(&store, &payload_json(&focus_event()), false, true);
        let done = execute(&store, no_account, None, &Fake::success()).unwrap();
        let Executed::Done { finalized: false, message: Some(message) } = done else { panic!("{done:?}") };
        assert!(message.contains("Calendar isn't connected"), "{message}");
        assert_eq!(run_state(&store, no_account).0, RunStatus::PendingApproval, "the draft survives");

        connect(&store, "me@example.com");
        let (_, expired) = pending(&store, &payload_json(&focus_event()), false, true);
        let fake = Fake { token: None, ..Fake::success() };
        assert!(matches!(execute(&store, expired, None, &fake).unwrap(), Executed::Done { finalized: false, .. }));
        assert_eq!(run_state(&store, expired).0, RunStatus::PendingApproval);
        assert!(fake.called.lock().unwrap().is_empty(), "no call is made without a usable token");
    }

    #[test]
    fn a_failure_before_google_is_retryable_and_a_refusal_by_google_is_final() {
        let store = store();
        connect(&store, "me@example.com");
        let (_, retry) = pending(&store, &payload_json(&focus_event()), false, true);
        let fake = Fake::answering(ToolOutcome::RetryableFailure { reason: "Node.js isn't installed.".into() });
        let Executed::Done { finalized: false, message: Some(message) } = execute(&store, retry, None, &fake).unwrap() else { panic!() };
        assert!(message.contains("Node.js isn't installed.") && message.contains("still pending"), "{message}");
        assert_eq!(run_state(&store, retry).0, RunStatus::PendingApproval);

        let (_, refused) = pending(&store, &payload_json(&focus_event()), false, true);
        let done = execute(&store, refused, None, &Fake::answering(ToolOutcome::TerminalFailure)).unwrap();
        assert_eq!(done, Executed::Done { finalized: true, message: None });
        assert_eq!(run_state(&store, refused), (RunStatus::Failed, "Google rejected the calendar event request.".to_string()));
    }

    #[test]
    fn a_draft_that_cannot_be_read_is_recorded_as_failed_without_any_call() {
        let store = store();
        connect(&store, "me@example.com");
        let (_, run) = pending(&store, "not json", false, true);
        let fake = Fake::success();
        assert_eq!(execute(&store, run, None, &fake).unwrap(), Executed::Unreadable);
        assert_eq!(run_state(&store, run).0, RunStatus::Failed);
        assert!(fake.called.lock().unwrap().is_empty());
    }

    #[test]
    fn a_connector_that_panics_still_settles_the_run_instead_of_stranding_it() {
        let store = store();
        connect(&store, "me@example.com");
        let (_, run) = pending(&store, &payload_json(&focus_event()), false, true);
        let fake = Fake { panics: true, ..Fake::success() };
        let Executed::Done { finalized: false, .. } = execute(&store, run, None, &fake).unwrap() else { panic!() };
        assert_eq!(run_state(&store, run).0, RunStatus::PendingApproval, "released, not left executing");
    }

    #[test]
    fn auto_approve_runs_only_routines_that_are_on_and_opted_in() {
        let store = store();
        connect(&store, "me@example.com");
        let (_, opted_in) = pending(&store, &payload_json(&focus_event()), true, true);
        let (_, asks_first) = pending(&store, &payload_json(&focus_event()), false, true);
        let (_, switched_off) = pending(&store, &payload_json(&focus_event()), true, false);
        let fake = Fake::success();

        assert_eq!(auto_approve(&store, &fake), 1);
        assert_eq!(run_state(&store, opted_in).0, RunStatus::Success);
        assert_eq!(run_state(&store, asks_first).0, RunStatus::PendingApproval, "no opt-in, no action");
        assert_eq!(run_state(&store, switched_off).0, RunStatus::PendingApproval, "a routine switched off does not act unattended");
        assert_eq!(fake.called.lock().unwrap().len(), 1);
    }

    #[test]
    fn auto_approve_with_nothing_pending_does_nothing() {
        let store = store();
        let fake = Fake::success();
        assert_eq!(auto_approve(&store, &fake), 0);
        assert!(fake.called.lock().unwrap().is_empty());
    }

    /// A `Real` over in-memory secrets, with a Google client, for the token rules (no network is
    /// touched: a fresh, sufficient grant is returned as it is).
    fn real_with(account: &str, tokens: api::oauth::StoredTokens) -> Real {
        real_with_kind(Kind::Calendar, account, tokens)
    }

    fn real_with_kind(kind: Kind, account: &str, tokens: api::oauth::StoredTokens) -> Real {
        let secrets: Arc<dyn SecretStore> = Arc::new(crate::secrets::MemorySecrets::default());
        accounts::save_tokens_for(kind, &*secrets, account, &tokens).unwrap();
        Real {
            secrets,
            config: Some(SyncConfig { worker_base_url: "https://worker.example".into(), google_client_id: "client".into(), google_client_secret: "secret".into() }),
            host: McpHost::new(),
            root: std::env::temp_dir(),
        }
    }

    fn grant(scope: Option<&str>) -> api::oauth::StoredTokens {
        api::oauth::StoredTokens {
            access_token: "access-1".into(),
            refresh_token: "refresh-1".into(),
            id_token: None,
            expires_at_unix_ms: chrono::Utc::now().timestamp_millis() + 3_600_000,
            granted_scope: scope.map(str::to_string),
        }
    }

    #[test]
    fn a_calendar_grant_that_includes_the_write_scope_yields_its_access_token() {
        for scope in [
            "openid email https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events",
            "https://www.googleapis.com/auth/calendar",
        ] {
            let real = real_with("me@example.com", grant(Some(scope)));
            assert_eq!(real.token("google-calendar", "me@example.com").as_deref(), Some("access-1"), "{scope}");
        }
    }

    #[test]
    fn a_grant_the_user_narrowed_to_read_only_is_not_used_to_write() {
        let real = real_with("me@example.com", grant(Some("openid email https://www.googleapis.com/auth/calendar.readonly")));
        assert_eq!(real.token("google-calendar", "me@example.com"), None, "the user is told to reconnect, not shown a button that fails");
    }

    #[test]
    fn an_account_with_no_stored_tokens_or_an_unknown_connector_gives_no_token() {
        let real = real_with("me@example.com", grant(Some(CALENDAR_WRITE_SCOPE)));
        assert_eq!(real.token("google-calendar", "someone-else@example.com"), None);
        assert_eq!(real.token("outlook365", "me@example.com"), None, "Outlook is deferred");
        assert_eq!(real.token("gmail", "me@example.com"), None, "a Calendar grant is not a Gmail grant");
    }

    #[test]
    fn a_gmail_grant_that_includes_compose_or_send_yields_its_access_token() {
        for scope in [
            "openid email https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose",
            "https://www.googleapis.com/auth/gmail.send",
            "https://mail.google.com/",
        ] {
            let real = real_with_kind(Kind::Gmail, "me@example.com", grant(Some(scope)));
            assert_eq!(real.token("gmail", "me@example.com").as_deref(), Some("access-1"), "{scope}");
        }
    }

    #[test]
    fn a_gmail_grant_the_user_narrowed_to_read_only_is_not_used_to_send() {
        let real = real_with_kind(Kind::Gmail, "me@example.com", grant(Some("openid email https://www.googleapis.com/auth/gmail.readonly")));
        assert_eq!(real.token("gmail", "me@example.com"), None, "the user is told to reconnect Gmail");
    }

    #[test]
    fn the_calendar_account_of_the_same_address_is_not_used_to_send() {
        let real = real_with_kind(Kind::Calendar, "me@example.com", grant(Some(GMAIL_SEND_SCOPE)));
        assert_eq!(real.token("gmail", "me@example.com"), None);
    }

    #[test]
    fn without_a_google_client_nothing_can_be_refreshed_so_there_is_no_token() {
        let mut real = real_with("me@example.com", grant(Some(CALENDAR_WRITE_SCOPE)));
        real.config = None;
        assert_eq!(real.token("google-calendar", "me@example.com"), None);
    }

    #[test]
    fn the_registry_ids_are_the_ones_core_uses() {
        assert_eq!(registry_id("google-calendar"), "calendar");
        assert_eq!(registry_id("gmail"), "gmail");
    }
}
