//! Settings → Connected accounts (M3.3 Task 7): connecting a Google Calendar, and what
//! the card shows about it.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::connectors::accounts::Kind;
use crate::connectors::write::GMAIL_SEND_SCOPE;
use crate::connectors::{accounts, google_client, google_disconnect, node};
use crate::error::CommandError;
use crate::secrets::SecretStore;
use crate::state::AppState;
use crate::l10n::tr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CalendarAccounts {
    pub emails: Vec<String>,
    /// Meetings still to come that a prompt could fire for.
    pub upcoming: usize,
    /// The next one's title and start, for the card's line.
    pub next_meeting: Option<String>,
    pub next_start_unix_ms: Option<i64>,
    /// When the last round finished, and why it failed if it did.
    pub last_synced_unix_ms: Option<i64>,
    pub sync_error: Option<String>,
    /// Why calendar syncing can't run right now (no Node, connector not staged, sync not
    /// configured), or `None` when it can.
    pub blocked: Option<String>,
    /// The accounts that need signing in again: Google's sign-in expired, or the Google client
    /// they were signed in with changed (Advanced → your own Google Cloud project).
    pub sign_in_expired: Vec<String>,
    /// Accounts Google keeps from the Calendar API: why, per account. An expired sign-in wins, so an
    /// account in `sign_in_expired` is never here.
    pub access_issues: Vec<CalendarAccessIssue>,
}

/// One account Google keeps from the Calendar API (core::connectors::google_error).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CalendarAccessIssue {
    pub email: String,
    /// `calendar_access_not_granted` (the calendar box was left unticked) or `calendar_access_blocked`.
    pub code: String,
}

/// The access issues to show: only connected accounts, and none for one whose sign-in expired.
pub fn access_issues_for(emails: &[String], expired: &[String], issues: &std::collections::HashMap<String, &'static str>) -> Vec<CalendarAccessIssue> {
    emails
        .iter()
        .filter(|e| !expired.contains(e))
        .filter_map(|e| issues.get(e).map(|code| CalendarAccessIssue { email: e.clone(), code: code.to_string() }))
        .collect()
}

fn blocked_reason(state: &AppState) -> Option<String> {
    if state.calendar.is_none() {
        return Some(tr("Calendar syncing needs Secrets.toml's Google client. See the app README.", &[]));
    }
    let (connector, _) = node::GOOGLE_CALENDAR;
    node::ready(&state.paths.root, connector).err().map(|e| e.to_string())
}

#[tauri::command]
pub fn calendar_accounts(state: State<'_, AppState>) -> Result<CalendarAccounts, CommandError> {
    let emails = state
        .store()?
        .call_blocking(|s| api::connectors::accounts::list(s, accounts::CALENDAR).map_err(|e| e.to_string()))?
        .map(|a| a.emails)
        .map_err(CommandError::store)?;
    let now = chrono::Utc::now().timestamp_millis();
    let upcoming = state.calendar.as_ref().map(|c| c.upcoming(now)).unwrap_or_default();
    let status = state.calendar.as_ref().map(|c| c.status()).unwrap_or_default();
    let sign_in_expired: Vec<String> = emails.iter().filter(|e| accounts::needs_sign_in(Kind::Calendar, &*state.secrets, e)).cloned().collect();
    let issues = state.calendar.as_ref().map(|c| c.access_issues()).unwrap_or_default();
    let access_issues = access_issues_for(&emails, &sign_in_expired, &issues);
    Ok(CalendarAccounts {
        sign_in_expired,
        access_issues,
        emails,
        upcoming: upcoming.len(),
        next_meeting: upcoming.first().map(|m| m.title.clone()),
        next_start_unix_ms: upcoming.first().map(|m| m.start_unix_ms),
        last_synced_unix_ms: status.last_synced_unix_ms,
        sync_error: status.error,
        blocked: blocked_reason(&state),
    })
}

/// Opens Google's consent page and waits for it, then records the account and polls.
#[tauri::command]
pub async fn calendar_connect(app: AppHandle) -> Result<CalendarAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let config = crate::syncing::config::SyncConfig::embedded().map_err(CommandError::not_configured)?;
        let email = accounts::connect(&state.secrets, &config, crate::syncing::controller::open_in_browser)?;
        let stored = state
            .store()?
            .call_blocking({
                let email = email.clone();
                move |s| api::connectors::accounts::add(s, accounts::CALENDAR, &email, chrono::Utc::now().timestamp_millis()).map_err(|e| e.to_string())
            })?
            .map_err(CommandError::store)?;
        tracing::info!(email, added = stored, "connected a Google Calendar account");
        if let Some(calendar) = &state.calendar {
            calendar.poll_now();
        }
        calendar_accounts(state)
    })
    .await
    .map_err(|e| CommandError::network(tr("Connecting the account failed: {error}", &[("error", &e.to_string())])))?
}

/// The connected Gmail and Calendar addresses and the sync sign-in's: who else shares a grant.
fn google_grant_is_shared(state: &AppState, removing: Kind, email: &str) -> bool {
    let listed = |kind: &'static str| {
        state
            .store()
            .ok()
            .and_then(|s| s.call_blocking(move |s| api::connectors::accounts::list(s, kind).ok()).ok().flatten())
            .map(|a| a.emails)
            .unwrap_or_default()
    };
    let sync = google_disconnect::sync_sign_in_email(&*state.secrets);
    google_disconnect::grant_is_shared(removing, email, &listed(accounts::GMAIL), &listed(accounts::CALENDAR), sync.as_deref())
}

/// Whether removing this Google account also revokes Whimbrel's access with Google (what the
/// confirmation says). `service` is "gmail" or "calendar".
#[tauri::command]
pub async fn google_removal_revokes(app: AppHandle, service: String, email: String) -> Result<bool, CommandError> {
    let kind = if service == "gmail" { Kind::Gmail } else { Kind::Calendar };
    tauri::async_runtime::spawn_blocking(move || !google_grant_is_shared(&app.state::<AppState>(), kind, &email))
        .await
        .map_err(|e| CommandError::store(e.to_string()))
}

/// Removes a Calendar account (#798): its tokens are deleted, its events leave this PC (future
/// ones it owned are marked removed so other devices drop them too; recorded meetings stay),
/// then Whimbrel's access is revoked with Google in the background unless another connection
/// on this PC shares the address.
#[tauri::command]
pub async fn calendar_disconnect(app: AppHandle, email: String) -> Result<CalendarAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let revoke = !google_grant_is_shared(&state, Kind::Calendar, &email);
        let refresh = accounts::tokens_for(Kind::Calendar, &*state.secrets, &email).ok().map(|t| t.refresh_token);
        accounts::disconnect(&*state.secrets, &email)?;
        if let Some(calendar) = &state.calendar {
            calendar.forget(&email);
        }
        state
            .store()?
            .call_blocking({
                let email = email.clone();
                move |s| api::connectors::accounts::remove(s, accounts::CALENDAR, &email).map_err(|e| e.to_string())
            })?
            .map_err(CommandError::store)?;
        // Its events leave this PC (#798); its future meetings leave every device, phones
        // included (calendar removals, 2026-11-11). The same email `calendar::apply` was given.
        // A failure leaves them until they pass: not fatal.
        let deleted = state.store()?.call_blocking({
            let email = email.clone();
            move |s| {
                let sole = api::connectors::accounts::list(s, accounts::CALENDAR).map(|a| a.emails.is_empty()).unwrap_or(false);
                s.delete_calendar_data_for_account("google_calendar", &email, sole, chrono::Utc::now().timestamp_millis()).map_err(|e| e.to_string())
            }
        });
        match deleted {
            Ok(Ok(outcome)) => tracing::info!(email, ?outcome, "deleted the account's calendar events"),
            Ok(Err(e)) => tracing::warn!(email, error = %e, "could not delete the account's calendar events"),
            Err(e) => tracing::warn!(email, error = %e, "could not delete the account's calendar events"),
        }
        match google_disconnect::skip_reason(revoke, refresh.as_deref()) {
            None => google_disconnect::revoke_in_background("Calendar", refresh.unwrap_or_default()),
            Some(reason) => tracing::info!(reason, "Calendar disconnect: no Google revoke sent"),
        }
        tracing::info!(email, "disconnected a Google Calendar account");
        calendar_accounts(state)
    })
    .await
    .map_err(|e| CommandError::network(tr("Disconnecting the account failed: {error}", &[("error", &e.to_string())])))?
}

/// Syncs now rather than waiting for the five-minute timer.
#[tauri::command]
pub async fn calendar_sync_now(app: AppHandle) -> Result<CalendarAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if let Some(calendar) = &state.calendar {
            calendar.poll_now();
        }
        calendar_accounts(state)
    })
    .await
    .map_err(|e| CommandError::network(tr("Syncing failed: {error}", &[("error", &e.to_string())])))?
}

// --- Gmail (M3.4 Task 3) ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GmailAccount {
    pub email: String,
    /// The stored grant can no longer send: its tokens are gone, or Google did not grant the
    /// permission to send. Connecting the same address again fixes it.
    pub needs_reconnect: bool,
    /// When Whimbrel last read this account's mail, and why the last round failed if it did.
    pub last_read_unix_ms: Option<i64>,
    pub read_error: Option<String>,
    /// Google's sign-in for this account expired, or the Google client it was signed in with
    /// changed: only "Sign in again" fixes it.
    pub sign_in_expired: bool,
    /// Google granted sending but not reading (the app's own client since 2026-11-12), so
    /// Whimbrel does not read this account's mail. Reading needs the user's own Google project.
    pub send_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GmailAccounts {
    pub accounts: Vec<GmailAccount>,
    /// Why Gmail can't be connected right now (no Google client, no Node, connector not staged), or `None`.
    pub blocked: Option<String>,
}

/// Whether an account's stored grant can still send. A grant whose scope was never recorded is not
/// held against it: sending learns the scope with one refresh, so only a **known** lack counts.
pub fn needs_reconnect(secrets: &dyn SecretStore, email: &str) -> bool {
    match accounts::tokens_for(Kind::Gmail, secrets, email) {
        Err(_) => true,
        Ok(stored) => match stored.granted_scope.as_deref() {
            None => false,
            Some(granted) => !api::oauth::granted_scope_satisfies(Some(granted), GMAIL_SEND_SCOPE),
        },
    }
}

pub fn gmail_view(
    store: &memory::Store,
    secrets: &dyn SecretStore,
    blocked: Option<String>,
    status: &dyn Fn(&str) -> crate::connectors::gmail::AccountStatus,
) -> Result<GmailAccounts, CommandError> {
    let connected = api::connectors::accounts::list(store, accounts::GMAIL).map_err(|e| CommandError::store(e.to_string()))?;
    Ok(GmailAccounts {
        accounts: connected
            .emails
            .into_iter()
            .map(|email| {
                let read = status(&email);
                GmailAccount {
                    needs_reconnect: needs_reconnect(secrets, &email),
                    sign_in_expired: accounts::needs_sign_in(Kind::Gmail, secrets, &email),
                    send_only: accounts::tokens_for(Kind::Gmail, secrets, &email).is_ok_and(|t| {
                        let granted = t.granted_scope.as_deref();
                        // A grant that can't send either needs reconnecting, which its row already says.
                        !accounts::can_read_mail(granted) && api::oauth::granted_scope_satisfies(granted, GMAIL_SEND_SCOPE)
                    }),
                    last_read_unix_ms: read.last_read_unix_ms,
                    read_error: read.error,
                    email,
                }
            })
            .collect(),
        blocked,
    })
}

/// Gmail can send from this build: the Google client is embedded and the connector is staged.
pub(crate) fn gmail_ready(state: &AppState) -> bool {
    gmail_blocked(state).is_none()
}

fn gmail_blocked(state: &AppState) -> Option<String> {
    if crate::syncing::config::SyncConfig::embedded().is_err() {
        return Some(tr("Connecting Gmail needs Secrets.toml's Google client. See the app README.", &[]));
    }
    let (connector, _) = node::GMAIL;
    node::ready(&state.paths.root, connector).err().map(|e| e.to_string())
}

fn gmail_state(state: &AppState) -> Result<GmailAccounts, CommandError> {
    let secrets = state.secrets.clone();
    let blocked = gmail_blocked(state);
    let gmail = state.gmail.clone();
    state.store()?.call_blocking(move |s| {
        gmail_view(s, &*secrets, blocked, &|email| gmail.as_ref().map(|g| g.status(email)).unwrap_or_default())
    })?
}

#[tauri::command]
pub async fn gmail_accounts(app: AppHandle) -> Result<GmailAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || gmail_state(&app.state::<AppState>()))
        .await
        .map_err(|e| CommandError::store(tr("Reading the Gmail accounts failed: {error}", &[("error", &e.to_string())])))?
}

/// Signs a connected calendar account in again after its sign-in expired, then syncs it.
#[tauri::command]
pub async fn calendar_reconnect(app: AppHandle, email: String) -> Result<CalendarAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let config = crate::syncing::config::SyncConfig::embedded().map_err(CommandError::not_configured)?;
        let connected = state
            .store()?
            .call_blocking(|s| api::connectors::accounts::list(s, accounts::CALENDAR).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)?
            .emails;
        accounts::reconnect_for(Kind::Calendar, &state.secrets, &config, crate::syncing::controller::open_in_browser, &email, |e| {
            connected.iter().any(|c| c.eq_ignore_ascii_case(e))
        })?;
        tracing::info!(email, "signed a Google Calendar account in again");
        if let Some(calendar) = &state.calendar {
            calendar.poll_now();
        }
        calendar_accounts(state)
    })
    .await
    .map_err(|e| CommandError::network(tr("Signing in again failed: {error}", &[("error", &e.to_string())])))?
}

/// Signs a connected Gmail account in again after its sign-in expired, then reads it.
#[tauri::command]
pub async fn gmail_reconnect(app: AppHandle, email: String) -> Result<GmailAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let config = crate::syncing::config::SyncConfig::embedded().map_err(CommandError::not_configured)?;
        let connected = state
            .store()?
            .call_blocking(|s| api::connectors::accounts::list(s, accounts::GMAIL).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)?
            .emails;
        accounts::reconnect_for(Kind::Gmail, &state.secrets, &config, crate::syncing::controller::open_in_browser, &email, |e| {
            connected.iter().any(|c| c.eq_ignore_ascii_case(e))
        })?;
        tracing::info!(email, "signed a Gmail account in again");
        if let Some(gmail) = &state.gmail {
            gmail.poll_now();
        }
        gmail_state(&state)
    })
    .await
    .map_err(|e| CommandError::network(tr("Signing in again failed: {error}", &[("error", &e.to_string())])))?
}

/// Opens Google's consent page and waits for it. Connecting an address that is already there
/// replaces its tokens, which is how an account that needs reconnecting is fixed.
#[tauri::command]
pub async fn gmail_connect(app: AppHandle) -> Result<GmailAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let config = crate::syncing::config::SyncConfig::embedded().map_err(CommandError::not_configured)?;
        let email = accounts::connect_for(Kind::Gmail, &state.secrets, &config, crate::syncing::controller::open_in_browser)?;
        let added = state
            .store()?
            .call_blocking({
                let email = email.clone();
                move |s| api::connectors::accounts::add(s, accounts::GMAIL, &email, chrono::Utc::now().timestamp_millis()).map_err(|e| e.to_string())
            })?
            .map_err(CommandError::store)?;
        tracing::info!(email, added, "connected a Gmail account");
        // Records the mailbox position at once, so only mail arriving from now on is read.
        if let Some(gmail) = &state.gmail {
            gmail.poll_now();
        }
        gmail_state(&state)
    })
    .await
    .map_err(|e| CommandError::network(tr("Connecting Gmail failed: {error}", &[("error", &e.to_string())])))?
}

/// Removes a Gmail account (#798): its tokens are deleted, the mail Whimbrel stored from it (and
/// what was built from that mail) leaves this PC, then Whimbrel's access is revoked with Google
/// in the background unless another connection on this PC shares the address. Routines are left
/// as they are: with no Gmail connected the recap still runs, it just drafts no email.
#[tauri::command]
pub async fn gmail_disconnect(app: AppHandle, email: String) -> Result<GmailAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let revoke = !google_grant_is_shared(&state, Kind::Gmail, &email);
        let refresh = accounts::tokens_for(Kind::Gmail, &*state.secrets, &email).ok().map(|t| t.refresh_token);
        accounts::disconnect_for(Kind::Gmail, &*state.secrets, &email)?;
        state
            .store()?
            .call_blocking({
                let email = email.clone();
                move |s| api::connectors::accounts::remove(s, accounts::GMAIL, &email).map_err(|e| e.to_string())
            })?
            .map_err(CommandError::store)?;
        if let Some(gmail) = &state.gmail {
            gmail.forget(&email);
        }
        // After the account left the list, so a poll that passed its still-connected check has
        // queued its write first and this delete runs after it.
        let deleted = state.store()?.call_blocking({
            let email = email.clone();
            move |s| s.delete_email_data_for_account(&email).map_err(|e| e.to_string())
        });
        match deleted {
            Ok(Ok(removed)) => tracing::info!(email, removed, "deleted the account's stored mail"),
            Ok(Err(e)) => tracing::warn!(email, error = %e, "could not delete the account's stored mail"),
            Err(e) => tracing::warn!(email, error = %e, "could not delete the account's stored mail"),
        }
        match google_disconnect::skip_reason(revoke, refresh.as_deref()) {
            None => google_disconnect::revoke_in_background("Gmail", refresh.unwrap_or_default()),
            Some(reason) => tracing::info!(reason, "Gmail disconnect: no Google revoke sent"),
        }
        tracing::info!(email, "disconnected a Gmail account");
        gmail_state(&state)
    })
    .await
    .map_err(|e| CommandError::network(tr("Disconnecting Gmail failed: {error}", &[("error", &e.to_string())])))?
}

/// Deletes the mail Whimbrel stored from one account, with the summaries and scores built from
/// it. The account stays connected (or stays disconnected); a connected account re-baselines
/// on its next round. Works with no Gmail service too, since stored mail outlives the account.
#[tauri::command]
pub async fn gmail_delete_stored_mail(app: AppHandle, email: String) -> Result<GmailAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        match &state.gmail {
            Some(gmail) => {
                gmail.delete_stored_mail(&email)?;
            }
            None => {
                let account = email.clone();
                state
                    .store()?
                    .call_blocking(move |s| s.delete_email_data_for_account(&account).map_err(|e| e.to_string()))?
                    .map_err(CommandError::store)?;
            }
        }
        gmail_state(&state)
    })
    .await
    .map_err(|e| CommandError::store(tr("Deleting the stored mail failed: {error}", &[("error", &e.to_string())])))?
}

// --- Your own Google Cloud project (2026-11-12) -------------------------------------------------------

/// What Settings shows about the custom Google client. The secret never leaves Credential Manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GoogleClientStatus {
    /// The saved client's ID, or `None` when Gmail and Calendar use the app's own client.
    pub client_id: Option<String>,
}

pub fn google_client_view(secrets: &dyn SecretStore) -> GoogleClientStatus {
    GoogleClientStatus { client_id: google_client::load(secrets).map(|c| c.id) }
}

#[tauri::command]
pub fn google_client_status(state: State<'_, AppState>) -> GoogleClientStatus {
    google_client_view(&*state.secrets)
}

/// Saves the user's own Desktop OAuth client. Connected Google accounts then show "Sign in again".
#[tauri::command]
pub fn google_client_save(state: State<'_, AppState>, client_id: String, client_secret: String) -> Result<GoogleClientStatus, CommandError> {
    let client = google_client::CustomClient::new(&client_id, &client_secret);
    google_client::save(&*state.secrets, &client)?;
    tracing::info!("saved a custom Google client; connected Google accounts need signing in again");
    Ok(google_client_view(&*state.secrets))
}

/// Forgets the user's own client: Gmail and Calendar go back to the app's, sending only for Gmail.
#[tauri::command]
pub fn google_client_clear(state: State<'_, AppState>) -> Result<GoogleClientStatus, CommandError> {
    google_client::clear(&*state.secrets)?;
    tracing::info!("cleared the custom Google client; connected Google accounts need signing in again");
    Ok(google_client_view(&*state.secrets))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_access_issues_show_for_connected_accounts_unless_the_sign_in_expired() {
        let emails = vec!["a@x.com".to_string(), "b@x.com".to_string(), "c@x.com".to_string()];
        let issues: std::collections::HashMap<String, &'static str> = [
            ("a@x.com".to_string(), "calendar_access_not_granted"),
            ("b@x.com".to_string(), "calendar_access_blocked"),
            ("gone@x.com".to_string(), "calendar_access_blocked"),
        ]
        .into_iter()
        .collect();
        let shown = access_issues_for(&emails, &["b@x.com".to_string()], &issues);
        assert_eq!(shown, vec![CalendarAccessIssue { email: "a@x.com".into(), code: "calendar_access_not_granted".into() }]);
    }
    use crate::secrets::MemorySecrets;
    use api::oauth::StoredTokens;
    use memory::Store;

    fn none(_: &str) -> crate::connectors::gmail::AccountStatus {
        Default::default()
    }

    fn grant(scope: Option<&str>) -> StoredTokens {
        StoredTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            id_token: None,
            expires_at_unix_ms: 9_000_000_000_000,
            granted_scope: scope.map(str::to_string),
        }
    }

    fn connected(secrets: &MemorySecrets, store: &Store, email: &str, scope: Option<&str>) {
        accounts::save_tokens_for(Kind::Gmail, secrets, email, &grant(scope)).unwrap();
        api::connectors::accounts::add(store, accounts::GMAIL, email, 1).unwrap();
    }

    #[test]
    fn nothing_connected_is_an_empty_list() {
        let view = gmail_view(&Store::open_in_memory().unwrap(), &MemorySecrets::default(), None, &none).unwrap();
        assert_eq!(view, GmailAccounts { accounts: vec![], blocked: None });
    }

    #[test]
    fn an_account_that_can_send_does_not_need_reconnecting() {
        let (store, secrets) = (Store::open_in_memory().unwrap(), MemorySecrets::default());
        connected(&secrets, &store, "a@example.com", Some("openid email https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose"));
        connected(&secrets, &store, "b@example.com", Some(GMAIL_SEND_SCOPE));
        let view = gmail_view(&store, &secrets, None, &none).unwrap();
        assert_eq!(view.accounts.iter().map(|a| (a.email.as_str(), a.needs_reconnect)).collect::<Vec<_>>(), [("a@example.com", false), ("b@example.com", false)]);
    }

    #[test]
    fn each_account_shows_its_own_last_read_and_error() {
        let (store, secrets) = (Store::open_in_memory().unwrap(), MemorySecrets::default());
        connected(&secrets, &store, "a@example.com", None);
        connected(&secrets, &store, "b@example.com", None);
        let status = |email: &str| match email {
            "a@example.com" => crate::connectors::gmail::AccountStatus { last_read_unix_ms: Some(5), error: None },
            _ => crate::connectors::gmail::AccountStatus { last_read_unix_ms: None, error: Some("offline".into()) },
        };
        let view = gmail_view(&store, &secrets, None, &status).unwrap();
        let shown: Vec<_> = view.accounts.iter().map(|a| (a.email.as_str(), a.last_read_unix_ms, a.read_error.as_deref())).collect();
        assert_eq!(shown, [("a@example.com", Some(5), None), ("b@example.com", None, Some("offline"))]);
    }

    #[test]
    fn an_account_whose_grant_lacks_permission_to_send_says_to_reconnect() {
        let (store, secrets) = (Store::open_in_memory().unwrap(), MemorySecrets::default());
        connected(&secrets, &store, "a@example.com", Some("openid email https://www.googleapis.com/auth/gmail.readonly"));
        assert!(gmail_view(&store, &secrets, None, &none).unwrap().accounts[0].needs_reconnect);
    }

    #[test]
    fn an_account_whose_tokens_are_gone_says_to_reconnect() {
        let (store, secrets) = (Store::open_in_memory().unwrap(), MemorySecrets::default());
        api::connectors::accounts::add(&store, accounts::GMAIL, "a@example.com", 1).unwrap();
        assert!(gmail_view(&store, &secrets, None, &none).unwrap().accounts[0].needs_reconnect, "listed but nothing to send with");
    }

    #[test]
    fn a_send_only_grant_is_marked_and_a_reading_or_older_grant_is_not() {
        let (store, secrets) = (Store::open_in_memory().unwrap(), MemorySecrets::default());
        connected(&secrets, &store, "send@example.com", Some(accounts::GMAIL_SCOPE));
        connected(&secrets, &store, "read@example.com", Some(accounts::GMAIL_READ_SCOPE));
        connected(&secrets, &store, "old@example.com", None);
        let view = gmail_view(&store, &secrets, None, &none).unwrap();
        let shown: Vec<_> = view.accounts.iter().map(|a| (a.email.as_str(), a.send_only, a.needs_reconnect)).collect();
        assert_eq!(shown, [("send@example.com", true, false), ("read@example.com", false, false), ("old@example.com", false, false)]);
    }

    #[test]
    fn a_grant_that_can_neither_read_nor_send_needs_reconnecting_and_is_not_called_send_only() {
        let (store, secrets) = (Store::open_in_memory().unwrap(), MemorySecrets::default());
        connected(&secrets, &store, "a@example.com", Some("openid email"));
        let account = &gmail_view(&store, &secrets, None, &none).unwrap().accounts[0];
        assert_eq!((account.send_only, account.needs_reconnect), (false, true));
    }

    #[test]
    fn saving_or_clearing_the_own_client_asks_connected_accounts_to_sign_in_again() {
        let (store, secrets) = (Store::open_in_memory().unwrap(), MemorySecrets::default());
        connected(&secrets, &store, "a@example.com", Some(accounts::GMAIL_SCOPE));
        assert!(!gmail_view(&store, &secrets, None, &none).unwrap().accounts[0].sign_in_expired);
        google_client::save(&secrets, &google_client::CustomClient::new("own", "s")).unwrap();
        assert!(gmail_view(&store, &secrets, None, &none).unwrap().accounts[0].sign_in_expired);
        google_client::record_issuer(&secrets, Kind::Gmail, "a@example.com", Some("own")).unwrap();
        assert!(!gmail_view(&store, &secrets, None, &none).unwrap().accounts[0].sign_in_expired, "signed in again with it");
        google_client::clear(&secrets).unwrap();
        assert!(gmail_view(&store, &secrets, None, &none).unwrap().accounts[0].sign_in_expired);
    }

    #[test]
    fn the_client_status_shows_the_id_and_never_the_secret() {
        let secrets = MemorySecrets::default();
        assert_eq!(google_client_view(&secrets), GoogleClientStatus { client_id: None });
        google_client::save(&secrets, &google_client::CustomClient::new("own-id", "hidden-secret")).unwrap();
        let view = google_client_view(&secrets);
        assert_eq!(view.client_id.as_deref(), Some("own-id"));
        assert!(!serde_json::to_string(&view).unwrap().contains("hidden-secret"));
    }

    #[test]
    fn a_grant_whose_scope_was_never_recorded_is_not_held_against_the_account() {
        let (store, secrets) = (Store::open_in_memory().unwrap(), MemorySecrets::default());
        connected(&secrets, &store, "a@example.com", None);
        assert!(!gmail_view(&store, &secrets, None, &none).unwrap().accounts[0].needs_reconnect, "sending learns it with one refresh");
    }

    #[test]
    fn a_reason_gmail_cannot_be_connected_is_passed_through() {
        let view = gmail_view(&Store::open_in_memory().unwrap(), &MemorySecrets::default(), Some("Node.js isn't installed.".into()), &none).unwrap();
        assert_eq!(view.blocked.as_deref(), Some("Node.js isn't installed."));
    }

    #[test]
    fn a_calendar_account_of_the_same_address_does_not_make_gmail_look_connected() {
        let (store, secrets) = (Store::open_in_memory().unwrap(), MemorySecrets::default());
        accounts::save_tokens(&secrets, "a@example.com", &grant(Some(GMAIL_SEND_SCOPE))).unwrap();
        api::connectors::accounts::add(&store, accounts::CALENDAR, "a@example.com", 1).unwrap();
        assert!(gmail_view(&store, &secrets, None, &none).unwrap().accounts.is_empty());
    }
}
