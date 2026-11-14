//! What removing a Google account (the Gmail or Calendar connector) does, in one place so the
//! confirmation and the code agree (the Mac's `GoogleDisconnect`, #798): the stored tokens are
//! deleted, the account's Gmail data or Calendar events are deleted from this PC, and
//! Whimbrel's access is revoked with Google when nothing else on this PC still uses the same
//! grant. The revoke is best effort and runs in the background (it blocks for up to 30 s);
//! only its reason code is logged, never the token.

use api::oauth::{jwt_email_claim, StoredTokens};

use super::accounts::Kind;
use crate::secrets::{accounts as secret_accounts, SecretStore, GOOGLE_OAUTH_SERVICE};

/// Whether another Whimbrel connection on this PC signs in as `email` and so shares its grant
/// with Google: the other connector, or the sync sign-in. Revoking would cut that one off too,
/// so the revoke waits until the last one goes. Case-insensitive.
pub fn grant_is_shared(removing: Kind, email: &str, gmail: &[String], calendar: &[String], sync_email: Option<&str>) -> bool {
    let others = match removing {
        Kind::Gmail => calendar,
        Kind::Calendar => gmail,
    };
    others.iter().map(String::as_str).chain(sync_email).any(|o| o.eq_ignore_ascii_case(email))
}

/// The address the Whimbrel sync sign-in used, from its stored ID token, with no network call.
/// `None` when sync was never signed in on this PC.
pub fn sync_sign_in_email(secrets: &dyn SecretStore) -> Option<String> {
    let raw = secrets.get_string(GOOGLE_OAUTH_SERVICE, secret_accounts::GOOGLE_SYNC).ok().flatten()?;
    let tokens: StoredTokens = serde_json::from_str(&raw).ok()?;
    tokens.id_token.as_deref().and_then(jwt_email_claim)
}

/// Why no revoke is sent, or `None` when one is due for `refresh_token`.
pub fn skip_reason(revoke: bool, refresh_token: Option<&str>) -> Option<&'static str> {
    if !revoke {
        return Some("grant_shared");
    }
    match refresh_token {
        Some(t) if !t.is_empty() => None,
        _ => Some("nothing_to_revoke"),
    }
}

/// Revokes `refresh_token` with Google on a background thread and logs the outcome's reason
/// code (`what` is "Gmail" or "Calendar").
pub fn revoke_in_background(what: &'static str, refresh_token: String) {
    let spawned = std::thread::Builder::new().name("whimbrel-google-revoke".into()).spawn(move || {
        let outcome = api::oauth::revoke_google_token(&refresh_token);
        tracing::info!(connector = what, outcome = outcome.reason_code(), "Google revoke after disconnect");
    });
    if let Err(e) = spawned {
        tracing::warn!(connector = what, error = %e, "could not start the Google revoke");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_revoke_waits_while_another_connection_uses_the_same_address() {
        let (gmail, calendar) = (v(&["ana@x.com"]), v(&["Ana@X.com", "bo@x.com"]));
        assert!(grant_is_shared(Kind::Gmail, "ana@x.com", &gmail, &calendar, None), "calendar shares it, any case");
        assert!(grant_is_shared(Kind::Calendar, "ana@x.com", &gmail, &calendar, None), "gmail shares it");
        assert!(!grant_is_shared(Kind::Calendar, "bo@x.com", &gmail, &calendar, None), "only calendar has bo");
        assert!(grant_is_shared(Kind::Calendar, "bo@x.com", &gmail, &calendar, Some("BO@x.com")), "the sync sign-in shares it");
        assert!(!grant_is_shared(Kind::Gmail, "cy@x.com", &[], &[], None));
    }

    #[test]
    fn a_revoke_needs_an_unshared_grant_and_a_stored_refresh_token() {
        assert_eq!(skip_reason(false, Some("r")), Some("grant_shared"));
        assert_eq!(skip_reason(true, None), Some("nothing_to_revoke"));
        assert_eq!(skip_reason(true, Some("")), Some("nothing_to_revoke"));
        assert_eq!(skip_reason(true, Some("r")), None);
    }

    #[test]
    fn the_sync_sign_in_address_comes_from_its_stored_id_token() {
        let secrets = MemorySecrets::default();
        assert_eq!(sync_sign_in_email(&secrets), None);
        // header.payload.signature, payload {"email":"me@x.com"} (base64url, no padding).
        let id_token = "e30.eyJlbWFpbCI6Im1lQHguY29tIn0.sig";
        let tokens = serde_json::json!({ "accessToken": "a", "refreshToken": "r", "idToken": id_token, "expiresAtUnixMs": 0 });
        secrets.set_string(GOOGLE_OAUTH_SERVICE, secret_accounts::GOOGLE_SYNC, &tokens.to_string()).unwrap();
        assert_eq!(sync_sign_in_email(&secrets).as_deref(), Some("me@x.com"));
    }
}
