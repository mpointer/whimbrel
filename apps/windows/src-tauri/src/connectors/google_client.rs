//! "Use your own Google Cloud project" (Settings → Connectors → Advanced, 2026-11-12).
//!
//! Public builds ask Google only for scopes that need no yearly CASA assessment: Calendar gets
//! `calendar.events`, Gmail gets `gmail.send`. Reading mail needs `gmail.readonly`, a restricted
//! scope, so it is offered only through a Desktop OAuth client from the user's own Google Cloud
//! project. While one is saved, Gmail and Calendar sign in with it and Gmail also asks to read.
//! Sync sign-in always keeps the app's client.
//!
//! The client ID and secret live in Credential Manager only (the Mac uses the same names in the
//! Keychain): never in settings.json, settings sync, logs or the UI, which only learns the ID.
//!
//! A refresh token only refreshes with the client that issued it, so each account records its
//! issuing client ID ([`record_issuer`]; none recorded means the app's client). An account whose
//! issuer is not the current client needs signing in again ([`client_changed`]), and is never
//! refreshed with a different client ([`refresh_client`]).

use std::fmt;

use crate::connectors::accounts::{account_name_for, Kind};
use crate::error::CommandError;
use crate::l10n::tr;
use crate::secrets::SecretStore;
use crate::syncing::config::SyncConfig;

/// The credential service for the custom client and the issuer records (the Mac's names).
pub const SERVICE: &str = "whimbrel-google-oauth";
pub const CLIENT_ID_ACCOUNT: &str = "custom-client-id";
pub const CLIENT_SECRET_ACCOUNT: &str = "custom-client-secret";

/// A Desktop OAuth client from the user's own Google Cloud project.
#[derive(Clone, PartialEq, Eq)]
pub struct CustomClient {
    pub id: String,
    secret: String,
}

impl CustomClient {
    pub fn new(id: &str, secret: &str) -> Self {
        CustomClient { id: id.trim().to_string(), secret: secret.trim().to_string() }
    }
}

/// Never prints the secret, so a stray `{:?}` cannot log it.
impl fmt::Debug for CustomClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomClient").field("id", &self.id).field("secret", &"<redacted>").finish()
    }
}

/// The saved client, or `None` when there is none (or only half of one).
pub fn load(secrets: &dyn SecretStore) -> Option<CustomClient> {
    let id = secrets.get_string(SERVICE, CLIENT_ID_ACCOUNT).ok().flatten()?;
    let secret = secrets.get_string(SERVICE, CLIENT_SECRET_ACCOUNT).ok().flatten()?;
    let client = CustomClient::new(&id, &secret);
    (!client.id.is_empty() && !client.secret.is_empty()).then_some(client)
}

/// Saves a client, replacing any saved one. Both fields are required.
pub fn save(secrets: &dyn SecretStore, client: &CustomClient) -> Result<(), CommandError> {
    if client.id.is_empty() || client.secret.is_empty() {
        return Err(CommandError::invalid_input(tr("Enter both the client ID and the client secret.", &[])));
    }
    let stored = |account: &str, value: &str| secrets.set_string(SERVICE, account, value).map_err(|e| CommandError::secrets(e.to_string()));
    stored(CLIENT_ID_ACCOUNT, &client.id)?;
    stored(CLIENT_SECRET_ACCOUNT, &client.secret)
}

/// Forgets the saved client. Accounts it signed in then need signing in again.
pub fn clear(secrets: &dyn SecretStore) -> Result<(), CommandError> {
    for account in [CLIENT_ID_ACCOUNT, CLIENT_SECRET_ACCOUNT] {
        secrets.delete(SERVICE, account).map_err(|e| CommandError::secrets(e.to_string()))?;
    }
    Ok(())
}

fn issuer_account(kind: Kind, email: &str) -> String {
    format!("issuer:{}", account_name_for(kind, email))
}

/// The client ID that issued `email`'s tokens, or `None` for the app's own client.
pub fn issuer(secrets: &dyn SecretStore, kind: Kind, email: &str) -> Option<String> {
    secrets.get_string(SERVICE, &issuer_account(kind, email)).ok().flatten()
}

/// Records which client signed `email` in: `Some(id)` for a custom client, `None` for the app's.
pub fn record_issuer(secrets: &dyn SecretStore, kind: Kind, email: &str, custom_id: Option<&str>) -> Result<(), CommandError> {
    let account = issuer_account(kind, email);
    match custom_id {
        Some(id) => secrets.set_string(SERVICE, &account, id),
        None => secrets.delete(SERVICE, &account),
    }
    .map_err(|e| CommandError::secrets(e.to_string()))
}

/// Forgets the issuer record when an account is disconnected.
pub fn forget_issuer(secrets: &dyn SecretStore, kind: Kind, email: &str) {
    let _ = secrets.delete(SERVICE, &issuer_account(kind, email));
}

/// Whether `email` was signed in with a client other than the current one, so only signing in
/// again lines it up (the custom client was saved, changed or cleared since).
pub fn client_changed(secrets: &dyn SecretStore, kind: Kind, email: &str) -> bool {
    issuer(secrets, kind, email) != load(secrets).map(|c| c.id)
}

/// The client to refresh `email`'s tokens with: the one that issued them. The app's client is
/// always at hand; a custom client only while it is still the saved one. Anything else cannot
/// refresh and needs signing in again.
pub fn refresh_client(secrets: &dyn SecretStore, config: &SyncConfig, kind: Kind, email: &str) -> Result<api::oauth::ProviderConfig, CommandError> {
    match issuer(secrets, kind, email) {
        None => Ok(config.google_provider()),
        Some(id) => match load(secrets) {
            Some(client) if client.id == id => Ok(provider(config, Some(&client))),
            _ => Err(CommandError::not_configured(tr(
                "{email} was connected with a Google Cloud project that is no longer saved. Sign in again.",
                &[("email", &email.to_string())],
            ))),
        },
    }
}

/// The app's Google client, or `custom` in its place. The scope is the caller's to set.
pub fn provider(config: &SyncConfig, custom: Option<&CustomClient>) -> api::oauth::ProviderConfig {
    let app = config.google_provider();
    match custom {
        Some(c) => api::oauth::ProviderConfig { client_id: c.id.clone(), client_secret: Some(c.secret.clone()), ..app },
        None => app,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn config() -> SyncConfig {
        crate::syncing::config::parse("sync_worker_base_url = \"u\"\ngoogle_oauth_client_id = \"app-id\"\ngoogle_oauth_client_secret = \"app-secret\"").unwrap()
    }

    #[test]
    fn a_saved_client_round_trips_and_clearing_forgets_it() {
        let secrets = MemorySecrets::default();
        assert_eq!(load(&secrets), None);
        save(&secrets, &CustomClient::new(" id.apps.googleusercontent.com ", " s3cret ")).unwrap();
        assert_eq!(load(&secrets), Some(CustomClient::new("id.apps.googleusercontent.com", "s3cret")));
        clear(&secrets).unwrap();
        assert_eq!(load(&secrets), None);
    }

    #[test]
    fn half_a_client_is_refused_and_is_not_a_client() {
        let secrets = MemorySecrets::default();
        assert!(save(&secrets, &CustomClient::new("id", "  ")).is_err());
        secrets.set_string(SERVICE, CLIENT_ID_ACCOUNT, "id").unwrap();
        assert_eq!(load(&secrets), None, "an ID without a secret is not usable");
    }

    #[test]
    fn the_secret_never_shows_in_debug_output() {
        let shown = format!("{:?}", CustomClient::new("my-id", "very-secret-value"));
        assert!(shown.contains("my-id"));
        assert!(!shown.contains("very-secret-value"), "{shown}");
    }

    #[test]
    fn saving_changing_or_clearing_the_client_flags_accounts_to_sign_in_again() {
        let secrets = MemorySecrets::default();
        // Signed in with the app's client, no custom client: nothing to do.
        assert!(!client_changed(&secrets, Kind::Gmail, "a@x.com"));
        // A custom client is saved: the app-client account must sign in again to use it.
        save(&secrets, &CustomClient::new("one", "s1")).unwrap();
        assert!(client_changed(&secrets, Kind::Gmail, "a@x.com"));
        // Signing in again with it lines it up.
        record_issuer(&secrets, Kind::Gmail, "a@x.com", Some("one")).unwrap();
        assert!(!client_changed(&secrets, Kind::Gmail, "a@x.com"));
        // Another client replaces it.
        save(&secrets, &CustomClient::new("two", "s2")).unwrap();
        assert!(client_changed(&secrets, Kind::Gmail, "a@x.com"));
        // Clearing it leaves an account the custom client signed in.
        clear(&secrets).unwrap();
        assert!(client_changed(&secrets, Kind::Gmail, "a@x.com"));
        // And recording the app's client again lines it back up.
        record_issuer(&secrets, Kind::Gmail, "a@x.com", None).unwrap();
        assert!(!client_changed(&secrets, Kind::Gmail, "a@x.com"));
    }

    #[test]
    fn issuers_are_kept_per_connector_and_account() {
        let secrets = MemorySecrets::default();
        record_issuer(&secrets, Kind::Gmail, "a@x.com", Some("one")).unwrap();
        assert_eq!(issuer(&secrets, Kind::Gmail, "a@x.com").as_deref(), Some("one"));
        assert_eq!(issuer(&secrets, Kind::Calendar, "a@x.com"), None);
        assert_eq!(issuer(&secrets, Kind::Gmail, "b@x.com"), None);
        forget_issuer(&secrets, Kind::Gmail, "a@x.com");
        assert_eq!(issuer(&secrets, Kind::Gmail, "a@x.com"), None);
    }

    #[test]
    fn tokens_refresh_only_with_the_client_that_issued_them() {
        let (secrets, config) = (MemorySecrets::default(), config());
        let app_id = config.google_provider().client_id;
        // App-issued tokens refresh with the app's client, even while a custom client is saved.
        save(&secrets, &CustomClient::new("one", "s1")).unwrap();
        assert_eq!(refresh_client(&secrets, &config, Kind::Gmail, "a@x.com").unwrap().client_id, app_id);
        // Custom-issued tokens refresh with that client and its secret.
        record_issuer(&secrets, Kind::Gmail, "a@x.com", Some("one")).unwrap();
        let p = refresh_client(&secrets, &config, Kind::Gmail, "a@x.com").unwrap();
        assert_eq!((p.client_id.as_str(), p.client_secret.as_deref()), ("one", Some("s1")));
        // Once that client is replaced or cleared, they cannot refresh at all.
        save(&secrets, &CustomClient::new("two", "s2")).unwrap();
        assert!(refresh_client(&secrets, &config, Kind::Gmail, "a@x.com").is_err());
        clear(&secrets).unwrap();
        assert!(refresh_client(&secrets, &config, Kind::Gmail, "a@x.com").is_err());
    }
}
