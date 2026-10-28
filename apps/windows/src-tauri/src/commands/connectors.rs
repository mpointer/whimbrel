//! Settings → Connected accounts (M3.3 Task 7): connecting a Google Calendar, and what
//! the card shows about it.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::connectors::accounts::Kind;
use crate::connectors::write::GMAIL_SEND_SCOPE;
use crate::connectors::{accounts, node};
use crate::error::CommandError;
use crate::secrets::SecretStore;
use crate::state::AppState;

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
}

fn blocked_reason(state: &AppState) -> Option<String> {
    if state.calendar.is_none() {
        return Some("Calendar syncing needs Secrets.toml's Google client. See the app README.".into());
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
    Ok(CalendarAccounts {
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
    .map_err(|e| CommandError::network(format!("connecting the account failed: {e}")))?
}

#[tauri::command]
pub async fn calendar_disconnect(app: AppHandle, email: String) -> Result<CalendarAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
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
        tracing::info!(email, "disconnected a Google Calendar account");
        calendar_accounts(state)
    })
    .await
    .map_err(|e| CommandError::network(format!("disconnecting the account failed: {e}")))?
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
    .map_err(|e| CommandError::network(format!("syncing failed: {e}")))?
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
                GmailAccount { needs_reconnect: needs_reconnect(secrets, &email), last_read_unix_ms: read.last_read_unix_ms, read_error: read.error, email }
            })
            .collect(),
        blocked,
    })
}

fn gmail_blocked(state: &AppState) -> Option<String> {
    if crate::syncing::config::SyncConfig::embedded().is_err() {
        return Some("Connecting Gmail needs Secrets.toml's Google client. See the app README.".into());
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
        .map_err(|e| CommandError::store(format!("reading the Gmail accounts failed: {e}")))?
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
    .map_err(|e| CommandError::network(format!("connecting Gmail failed: {e}")))?
}

/// Forgets the account's tokens and removes it. Routines are left as they are: with no Gmail
/// connected the recap still runs, it just drafts no email.
#[tauri::command]
pub async fn gmail_disconnect(app: AppHandle, email: String) -> Result<GmailAccounts, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
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
        tracing::info!(email, "disconnected a Gmail account");
        gmail_state(&state)
    })
    .await
    .map_err(|e| CommandError::network(format!("disconnecting Gmail failed: {e}")))?
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
    .map_err(|e| CommandError::store(format!("deleting the stored mail failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
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
