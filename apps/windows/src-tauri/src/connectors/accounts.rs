//! Connecting a Google account to a connector (M3.3 Task 7).
//!
//! The same OAuth machinery sync sign-in uses (`api::oauth`), with the connector's own
//! scopes and its own Credential Manager account name. Tokens land under a temporary
//! `…:pending-<uuid>` name first, because the account's address only arrives with the ID
//! token; once it does they move to `google-calendar-connector:<email>`, the Mac's naming,
//! so a later settings sync lines up field for field.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::oauth::{self, StoredTokens};

use crate::error::CommandError;
use crate::secrets::{SecretStore, GOOGLE_OAUTH_SERVICE};
use crate::syncing::config::SyncConfig;
use crate::l10n::tr;

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
        .ok_or_else(|| CommandError::not_configured(tr("{email} isn't connected any more.", &[("email", &email.to_string())])))?;
    serde_json::from_str(&raw).map_err(|e| CommandError::secrets(tr("Stored tokens for {email} are unreadable: {error}", &[("email", &email.to_string()), ("error", &e.to_string())])))
}

pub fn save_tokens_for(kind: Kind, secrets: &dyn SecretStore, email: &str, tokens: &StoredTokens) -> Result<(), CommandError> {
    let json = serde_json::to_string(tokens).map_err(|e| CommandError::secrets(e.to_string()))?;
    secrets
        .set_string(GOOGLE_OAUTH_SERVICE, &account_name_for(kind, email), &json)
        .map_err(|e| CommandError::secrets(e.to_string()))
}

/// The accounts whose sign-in Google revoked or let expire (`invalid_grant` on refresh), by
/// Credential Manager account name. Only signing in again fixes these, so Settings offers it on
/// the account's row. A refresh that fails for any other reason (no network, Google down) is not
/// recorded: the next round retries it. Kept in memory: after a restart the first round finds it again.
static EXPIRED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn set_expired(kind: Kind, email: &str, expired: bool) {
    let mut set = EXPIRED.lock().unwrap_or_else(|e| e.into_inner());
    let set = set.get_or_insert_with(HashSet::new);
    let name = account_name_for(kind, email);
    let changed = if expired { set.insert(name) } else { set.remove(&name) };
    if changed && expired {
        tracing::warn!(email, connector = kind.id(), "the Google sign-in expired; Settings offers Sign in again");
    }
}

/// Whether `email`'s sign-in for `kind` expired and needs signing in again.
pub fn sign_in_expired(kind: Kind, email: &str) -> bool {
    EXPIRED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|s| s.contains(&account_name_for(kind, email)))
}

/// An unexpired access token for `email`, refreshing and saving when it has aged out.
pub fn fresh_access_token_for(kind: Kind, secrets: &dyn SecretStore, config: &SyncConfig, email: &str) -> Result<String, CommandError> {
    let stored = tokens_for(kind, secrets, email)?;
    let now = chrono::Utc::now().timestamp_millis();
    if stored.is_fresh(now) {
        return Ok(stored.access_token);
    }
    let refreshed = match oauth::refresh_stored(&provider_for(kind, config), &stored, now) {
        Ok(refreshed) => refreshed,
        Err(e) => {
            if e == oauth::OAuthError::RefreshTokenRevoked {
                set_expired(kind, email, true);
            }
            return Err(CommandError::network(crate::syncing::controller::oauth_message(&e)));
        }
    };
    save_tokens_for(kind, secrets, email, &refreshed)?;
    set_expired(kind, email, false);
    Ok(refreshed.access_token)
}

/// `auth_url` with Google's `login_hint`, so the consent page starts at `email`'s account.
fn with_login_hint(auth_url: &str, email: &str) -> String {
    let encoded: String = email
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let sep = if auth_url.contains('?') { '&' } else { '?' };
    format!("{auth_url}{sep}login_hint={encoded}")
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
    connect_hinted(kind, secrets, config, open, None)
}

/// [`connect_for`], with the consent page started at `login_hint`'s account when given.
fn connect_hinted(
    kind: Kind,
    secrets: &Arc<dyn SecretStore>,
    config: &SyncConfig,
    open: impl Fn(&str) -> Result<(), String>,
    login_hint: Option<&str>,
) -> Result<String, CommandError> {
    let provider = provider_for(kind, config);
    let begun = oauth::begin(&provider).map_err(|e| CommandError::network(crate::syncing::controller::oauth_message(&e)))?;
    let url = login_hint.map_or_else(|| begun.auth_url.clone(), |email| with_login_hint(&begun.auth_url, email));
    open(&url).map_err(CommandError::network)?;

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
        .ok_or_else(|| CommandError::network(tr("Google didn't say which account signed in.", &[])))?;
    secrets
        .set_string(GOOGLE_OAUTH_SERVICE, &account_name_for(kind, &email), &json)
        .map_err(|e| CommandError::secrets(e.to_string()))?;
    let _ = secrets.delete(GOOGLE_OAUTH_SERVICE, &pending);
    set_expired(kind, &email, false);
    Ok(email)
}

/// Signs `email` in again after its sign-in expired: the consent page starts at that account, and
/// the new tokens replace the old ones. Signing in as another address fixes nothing, so it is an
/// error, and that address's new tokens are removed unless it is itself connected (`is_connected`).
pub fn reconnect_for(
    kind: Kind,
    secrets: &Arc<dyn SecretStore>,
    config: &SyncConfig,
    open: impl Fn(&str) -> Result<(), String>,
    email: &str,
    is_connected: impl Fn(&str) -> bool,
) -> Result<(), CommandError> {
    let signed_in = connect_hinted(kind, secrets, config, open, Some(email))?;
    check_reconnected(kind, &**secrets, email, &signed_in, is_connected)
}

fn check_reconnected(
    kind: Kind,
    secrets: &dyn SecretStore,
    email: &str,
    signed_in: &str,
    is_connected: impl Fn(&str) -> bool,
) -> Result<(), CommandError> {
    if signed_in.eq_ignore_ascii_case(email) {
        return Ok(());
    }
    if !is_connected(signed_in) {
        let _ = disconnect_for(kind, secrets, signed_in);
    }
    Err(CommandError::invalid_input(tr(
        "You signed in as {signed_in}. Sign in again and choose {email}.",
        &[("signed_in", &signed_in.to_string()), ("email", &email.to_string())],
    )))
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

    #[test]
    fn the_consent_page_starts_at_the_account_being_fixed() {
        assert_eq!(
            with_login_hint("https://accounts.google.com/o/oauth2/v2/auth?client_id=x", "mike+test@example.com"),
            "https://accounts.google.com/o/oauth2/v2/auth?client_id=x&login_hint=mike%2Btest%40example.com"
        );
        assert_eq!(with_login_hint("https://a/auth", "a@b.c"), "https://a/auth?login_hint=a%40b.c");
    }

    #[test]
    fn an_expired_sign_in_is_remembered_per_connector_until_fixed() {
        let email = "expired-test@example.com";
        assert!(!sign_in_expired(Kind::Calendar, email));
        set_expired(Kind::Calendar, email, true);
        assert!(sign_in_expired(Kind::Calendar, email));
        assert!(!sign_in_expired(Kind::Gmail, email), "Gmail's grant is separate");
        set_expired(Kind::Calendar, email, false);
        assert!(!sign_in_expired(Kind::Calendar, email));
    }

    #[test]
    fn signing_in_as_another_address_fixes_nothing_and_leaves_no_tokens_behind() {
        let secrets = MemorySecrets::default();
        let other = StoredTokens { access_token: "at".into(), refresh_token: "rt".into(), id_token: None, expires_at_unix_ms: 9_000_000_000_000, granted_scope: None };
        save_tokens_for(Kind::Calendar, &secrets, "other@example.com", &other).unwrap();
        save_tokens_for(Kind::Calendar, &secrets, "kept@example.com", &other).unwrap();

        assert!(check_reconnected(Kind::Calendar, &secrets, "Me@Example.com", "me@example.com", |_| false).is_ok(), "case doesn't matter");

        let err = check_reconnected(Kind::Calendar, &secrets, "me@example.com", "other@example.com", |_| false).unwrap_err();
        assert!(err.to_string().contains("other@example.com") && err.to_string().contains("me@example.com"));
        assert!(tokens_for(Kind::Calendar, &secrets, "other@example.com").is_err(), "a stray sign-in is removed");

        check_reconnected(Kind::Calendar, &secrets, "me@example.com", "kept@example.com", |e| e == "kept@example.com").unwrap_err();
        assert!(tokens_for(Kind::Calendar, &secrets, "kept@example.com").is_ok(), "a connected account keeps its tokens");
    }
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
