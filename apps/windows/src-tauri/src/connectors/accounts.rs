//! Connecting a Google account to a connector (M3.3 Task 7).
//!
//! The same OAuth machinery sync sign-in uses (`api::oauth`), with the connector's own
//! scopes and its own Credential Manager account name. Tokens land under a temporary
//! `…:pending-<uuid>` name first, because the account's address only arrives with the ID
//! token; once it does they move to `google-calendar-connector:<email>`, the Mac's naming,
//! so a later settings sync lines up field for field.

use std::sync::Arc;
use std::time::Duration;

use api::oauth::{self, StoredTokens};

use crate::error::CommandError;
use crate::secrets::{SecretStore, GOOGLE_OAUTH_SERVICE};
use crate::syncing::config::SyncConfig;

/// The connector id core's account registry uses.
pub const CALENDAR: &str = "calendar";
/// Credential Manager account names, as on the Mac.
pub const CALENDAR_ACCOUNT_PREFIX: &str = "google-calendar-connector:";
/// FR-MTG-1 needs to read events; `calendar.events` is there for M3.4's write actions.
pub const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events openid email";
/// How long a consent page may sit unanswered.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The connector id core's account registry uses for Gmail.
pub const GMAIL: &str = "gmail";
/// Gmail's Credential Manager account names, as on the Mac (`gmail-connector:<email>`).
pub const GMAIL_ACCOUNT_PREFIX: &str = "gmail-connector:";
/// The Mac's Gmail scopes (`GmailConnectorState`): `gmail.compose` is Google's superset of
/// `gmail.send`, so one grant covers sending the approved recap; `gmail.readonly` is for reading mail.
/// Both are Google "restricted" scopes, fine for a personal Google Cloud project and a blocker for
/// public distribution without Google's verification.
pub const GMAIL_SCOPE: &str =
    "openid email https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose";

/// Which Google connector an account belongs to. Everything below is the same for both except the
/// names it is stored under and the scopes it asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Calendar,
    Gmail,
}

impl Kind {
    /// The id core's account registry uses.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Calendar => CALENDAR,
            Kind::Gmail => GMAIL,
        }
    }

    fn account_prefix(self) -> &'static str {
        match self {
            Kind::Calendar => CALENDAR_ACCOUNT_PREFIX,
            Kind::Gmail => GMAIL_ACCOUNT_PREFIX,
        }
    }

    pub fn scope(self) -> &'static str {
        match self {
            Kind::Calendar => CALENDAR_SCOPE,
            Kind::Gmail => GMAIL_SCOPE,
        }
    }
}

pub fn account_name_for(kind: Kind, email: &str) -> String {
    format!("{}{email}", kind.account_prefix())
}

pub fn account_name(email: &str) -> String {
    account_name_for(Kind::Calendar, email)
}

/// The provider config for a connector sign-in: the sync client, the connector's scopes.
pub fn provider_for(kind: Kind, config: &SyncConfig) -> oauth::ProviderConfig {
    oauth::ProviderConfig { scope: kind.scope().into(), ..config.google_provider() }
}

pub fn provider(config: &SyncConfig) -> oauth::ProviderConfig {
    provider_for(Kind::Calendar, config)
}

/// Reads an account's stored tokens.
pub fn tokens_for(kind: Kind, secrets: &dyn SecretStore, email: &str) -> Result<StoredTokens, CommandError> {
    let raw = secrets
        .get_string(GOOGLE_OAUTH_SERVICE, &account_name_for(kind, email))
        .map_err(|e| CommandError::secrets(e.to_string()))?
        .ok_or_else(|| CommandError::not_configured(format!("{email} isn't connected any more.")))?;
    serde_json::from_str(&raw).map_err(|e| CommandError::secrets(format!("stored tokens for {email} are unreadable: {e}")))
}

pub fn save_tokens_for(kind: Kind, secrets: &dyn SecretStore, email: &str, tokens: &StoredTokens) -> Result<(), CommandError> {
    let json = serde_json::to_string(tokens).map_err(|e| CommandError::secrets(e.to_string()))?;
    secrets
        .set_string(GOOGLE_OAUTH_SERVICE, &account_name_for(kind, email), &json)
        .map_err(|e| CommandError::secrets(e.to_string()))
}

/// An unexpired access token for `email`, refreshing and saving when it has aged out.
pub fn fresh_access_token_for(kind: Kind, secrets: &dyn SecretStore, config: &SyncConfig, email: &str) -> Result<String, CommandError> {
    let stored = tokens_for(kind, secrets, email)?;
    let now = chrono::Utc::now().timestamp_millis();
    if stored.is_fresh(now) {
        return Ok(stored.access_token);
    }
    let refreshed = oauth::refresh_stored(&provider_for(kind, config), &stored, now)
        .map_err(|e| CommandError::network(crate::syncing::controller::oauth_message(&e)))?;
    save_tokens_for(kind, secrets, email, &refreshed)?;
    Ok(refreshed.access_token)
}

/// Runs the consent flow and returns the address that signed in. Blocking: the caller
/// runs it off the main thread. The tokens are stored under the pending name first, so a
/// flow abandoned midway leaves nothing claiming to be a connected account.
pub fn connect_for(
    kind: Kind,
    secrets: &Arc<dyn SecretStore>,
    config: &SyncConfig,
    open: impl Fn(&str) -> Result<(), String>,
) -> Result<String, CommandError> {
    let provider = provider_for(kind, config);
    let begun = oauth::begin(&provider).map_err(|e| CommandError::network(crate::syncing::controller::oauth_message(&e)))?;
    open(&begun.auth_url).map_err(CommandError::network)?;

    let pending = format!("{}pending-{}", kind.account_prefix(), begun.session_id);
    let now = chrono::Utc::now().timestamp_millis();
    let tokens = oauth::wait(&begun.session_id, CONNECT_TIMEOUT.as_millis() as u64, now)
        .map_err(|e| CommandError::network(crate::syncing::controller::oauth_message(&e)))?;
    let json = serde_json::to_string(&tokens).map_err(|e| CommandError::secrets(e.to_string()))?;
    secrets.set_string(GOOGLE_OAUTH_SERVICE, &pending, &json).map_err(|e| CommandError::secrets(e.to_string()))?;

    let email = tokens
        .id_token
        .as_deref()
        .and_then(oauth::jwt_email_claim)
        .ok_or_else(|| CommandError::network("Google didn't say which account signed in."))?;
    secrets
        .set_string(GOOGLE_OAUTH_SERVICE, &account_name_for(kind, &email), &json)
        .map_err(|e| CommandError::secrets(e.to_string()))?;
    let _ = secrets.delete(GOOGLE_OAUTH_SERVICE, &pending);
    Ok(email)
}

/// Forgets an account's tokens. Its calendar rows stay: they are already in the store.
pub fn disconnect_for(kind: Kind, secrets: &dyn SecretStore, email: &str) -> Result<(), CommandError> {
    secrets
        .delete(GOOGLE_OAUTH_SERVICE, &account_name_for(kind, email))
        .map(|_| ())
        .map_err(|e| CommandError::secrets(e.to_string()))
}

/// The Calendar connector's versions of the functions above, under the names M3.3 gave them.
pub fn tokens(secrets: &dyn SecretStore, email: &str) -> Result<StoredTokens, CommandError> {
    tokens_for(Kind::Calendar, secrets, email)
}

pub fn save_tokens(secrets: &dyn SecretStore, email: &str, tokens: &StoredTokens) -> Result<(), CommandError> {
    save_tokens_for(Kind::Calendar, secrets, email, tokens)
}

pub fn fresh_access_token(secrets: &dyn SecretStore, config: &SyncConfig, email: &str) -> Result<String, CommandError> {
    fresh_access_token_for(Kind::Calendar, secrets, config, email)
}

pub fn connect(secrets: &Arc<dyn SecretStore>, config: &SyncConfig, open: impl Fn(&str) -> Result<(), String>) -> Result<String, CommandError> {
    connect_for(Kind::Calendar, secrets, config, open)
}

pub fn disconnect(secrets: &dyn SecretStore, email: &str) -> Result<(), CommandError> {
    disconnect_for(Kind::Calendar, secrets, email)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn config() -> SyncConfig {
        SyncConfig {
            worker_base_url: "https://worker.example".into(),
            google_client_id: "client".into(),
            google_client_secret: "secret".into(),
        }
    }

    #[test]
    fn the_account_name_is_the_macs() {
        assert_eq!(account_name("a@example.com"), "google-calendar-connector:a@example.com");
    }

    #[test]
    fn the_connector_asks_for_calendar_scopes_on_the_sync_client() {
        let provider = provider(&config());
        assert!(provider.scope.contains("calendar.readonly"));
        assert!(provider.scope.contains("openid email"), "the email claim names the account");
        assert_eq!(provider.client_id, "client", "the same desktop client as sync");
    }

    #[test]
    fn tokens_round_trip_and_a_missing_account_says_so() {
        let secrets = MemorySecrets::default();
        let stored = StoredTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            id_token: None,
            expires_at_unix_ms: 9_000_000_000_000,
            granted_scope: None,
        };
        save_tokens(&secrets, "a@example.com", &stored).unwrap();
        assert_eq!(tokens(&secrets, "a@example.com").unwrap().access_token, "at");
        assert_eq!(fresh_access_token(&secrets, &config(), "a@example.com").unwrap(), "at", "a fresh token is used as is");

        let error = tokens(&secrets, "b@example.com").unwrap_err();
        assert!(format!("{error}").contains("isn't connected"), "{error}");
    }

    #[test]
    fn disconnecting_forgets_the_tokens() {
        let secrets = MemorySecrets::default();
        let stored = StoredTokens {
            access_token: "at".into(),
            refresh_token: String::new(),
            id_token: None,
            expires_at_unix_ms: 9_000_000_000_000,
            granted_scope: None,
        };
        save_tokens(&secrets, "a@example.com", &stored).unwrap();
        disconnect(&secrets, "a@example.com").unwrap();
        assert!(tokens(&secrets, "a@example.com").is_err());
    }

    fn stored(scope: Option<&str>) -> StoredTokens {
        StoredTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            id_token: None,
            expires_at_unix_ms: 9_000_000_000_000,
            granted_scope: scope.map(str::to_string),
        }
    }

    #[test]
    fn gmail_accounts_are_named_the_way_the_mac_names_them_and_apart_from_calendar() {
        assert_eq!(account_name_for(Kind::Gmail, "a@example.com"), "gmail-connector:a@example.com");
        assert_eq!(account_name_for(Kind::Calendar, "a@example.com"), account_name("a@example.com"));
        assert_ne!(account_name_for(Kind::Gmail, "a@example.com"), account_name_for(Kind::Calendar, "a@example.com"));
    }

    #[test]
    fn gmail_asks_for_the_macs_scopes_on_the_same_client_and_calendars_scopes_are_unchanged() {
        let gmail = provider_for(Kind::Gmail, &config());
        for needed in ["openid email", "gmail.readonly", "gmail.compose"] {
            assert!(gmail.scope.contains(needed), "{needed}");
        }
        assert!(!gmail.scope.contains("calendar"), "Gmail does not ask for Calendar");
        assert_eq!(gmail.client_id, "client", "the same desktop client as sync");
        assert_eq!(provider(&config()).scope, CALENDAR_SCOPE);
        assert!(!provider(&config()).scope.contains("gmail"), "and Calendar does not ask for Gmail");
    }

    #[test]
    fn each_kind_has_its_own_registry_id() {
        assert_eq!((Kind::Calendar.id(), Kind::Gmail.id()), ("calendar", "gmail"));
    }

    #[test]
    fn a_gmail_account_and_a_calendar_account_with_one_address_do_not_share_tokens() {
        let secrets = MemorySecrets::default();
        save_tokens_for(Kind::Gmail, &secrets, "a@example.com", &stored(Some("gmail"))).unwrap();
        assert_eq!(tokens_for(Kind::Gmail, &secrets, "a@example.com").unwrap().granted_scope.as_deref(), Some("gmail"));
        assert!(tokens(&secrets, "a@example.com").is_err(), "connecting Gmail does not connect Calendar");

        save_tokens(&secrets, "a@example.com", &stored(Some("calendar"))).unwrap();
        assert_eq!(tokens_for(Kind::Gmail, &secrets, "a@example.com").unwrap().granted_scope.as_deref(), Some("gmail"), "and the reverse");
    }

    #[test]
    fn disconnecting_gmail_forgets_only_gmails_tokens() {
        let secrets = MemorySecrets::default();
        save_tokens_for(Kind::Gmail, &secrets, "a@example.com", &stored(None)).unwrap();
        save_tokens(&secrets, "a@example.com", &stored(None)).unwrap();
        disconnect_for(Kind::Gmail, &secrets, "a@example.com").unwrap();
        assert!(tokens_for(Kind::Gmail, &secrets, "a@example.com").is_err());
        assert!(tokens(&secrets, "a@example.com").is_ok(), "the Calendar account is still connected");
    }

    #[test]
    fn a_fresh_gmail_token_is_used_as_it_is() {
        let secrets = MemorySecrets::default();
        save_tokens_for(Kind::Gmail, &secrets, "a@example.com", &stored(None)).unwrap();
        assert_eq!(fresh_access_token_for(Kind::Gmail, &secrets, &config(), "a@example.com").unwrap(), "at");
    }
}
