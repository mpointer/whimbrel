//! Secrets in Windows Credential Manager (plan Decision 5). The only module that touches
//! it; core never does.
//!
//! Service and account names are the Mac's Keychain strings, so every secret has the
//! same name on both platforms. Values are stored as raw bytes (`keyring`'s
//! `set_secret`), not through its password API, so the full 2,560-byte credential blob
//! is usable — a Google token set with an ID token is about 1.5–2 KB.

use std::collections::HashMap;
use std::sync::Mutex;

/// API keys (Mac: the app bundle id).
pub const APP_SERVICE: &str = "com.mpointer.kestrel";
/// The sync device keypair and group key (Mac: `<bundle id>.sync`).
pub const SYNC_SERVICE: &str = "com.mpointer.kestrel.sync";
/// Google OAuth tokens for sync sign-in (Mac: `<bundle id>.google-oauth`).
pub const GOOGLE_OAUTH_SERVICE: &str = "com.mpointer.kestrel.google-oauth";

pub mod accounts {
    pub const ANTHROPIC_API_KEY: &str = "anthropic-api-key";
    pub const OPENAI_API_KEY: &str = "openai-api-key";
    pub const GOOGLE_API_KEY: &str = "google-api-key";
    pub const DEVICE_SECRET_KEY: &str = "device-secret-key";
    pub const GROUP_KEY: &str = "group-key";
    pub const GOOGLE_SYNC: &str = "google-sync";
}

/// `CRED_MAX_CREDENTIAL_BLOB_SIZE` for a generic credential.
pub const MAX_SECRET_BYTES: usize = 5 * 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// Refused before calling Credential Manager, which would fail opaquely.
    TooLarge { len: usize, max: usize },
    /// Credential Manager (or the platform layer) failed.
    Backend(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretError::TooLarge { len, max } => write!(f, "secret is {len} bytes; Credential Manager holds at most {max}"),
            SecretError::Backend(msg) => write!(f, "Credential Manager: {msg}"),
        }
    }
}

impl std::error::Error for SecretError {}

/// Where secrets live. The app uses [`CredentialManager`]; tests use [`MemorySecrets`].
pub trait SecretStore: Send + Sync {
    /// `Ok(None)` when nothing is stored under the name.
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, SecretError>;
    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), SecretError>;
    /// Deleting a missing secret is not an error.
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError>;

    /// A UTF-8 secret (API keys, base64 keys). An empty value reads as absent.
    fn get_string(&self, service: &str, account: &str) -> Result<Option<String>, SecretError> {
        match self.get(service, account)? {
            Some(bytes) => {
                let s = String::from_utf8(bytes).map_err(|_| SecretError::Backend(format!("{service}/{account} is not UTF-8")))?;
                Ok(Some(s).filter(|s| !s.is_empty()))
            }
            None => Ok(None),
        }
    }

    fn set_string(&self, service: &str, account: &str, value: &str) -> Result<(), SecretError> {
        self.set(service, account, value.as_bytes())
    }
}

fn check_size(value: &[u8]) -> Result<(), SecretError> {
    if value.len() > MAX_SECRET_BYTES {
        return Err(SecretError::TooLarge { len: value.len(), max: MAX_SECRET_BYTES });
    }
    Ok(())
}

/// Windows Credential Manager (generic credentials, per user, DPAPI-protected).
pub struct CredentialManager;

impl CredentialManager {
    fn entry(service: &str, account: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(service, account).map_err(|e| SecretError::Backend(e.to_string()))
    }
}

impl SecretStore for CredentialManager {
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, SecretError> {
        match Self::entry(service, account)?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Backend(e.to_string())),
        }
    }

    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), SecretError> {
        check_size(value)?;
        Self::entry(service, account)?.set_secret(value).map_err(|e| SecretError::Backend(e.to_string()))
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        match Self::entry(service, account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Backend(e.to_string())),
        }
    }
}

/// In-memory secrets for tests. Unlike `keyring`'s mock backend, values persist across
/// lookups, which the identity and settings tests depend on.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<(String, String), Vec<u8>>>);

impl SecretStore for MemorySecrets {
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, SecretError> {
        Ok(self.0.lock().unwrap().get(&(service.to_string(), account.to_string())).cloned())
    }

    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), SecretError> {
        check_size(value)?;
        self.0.lock().unwrap().insert((service.to_string(), account.to_string()), value.to_vec());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_secrets_round_trip_and_delete() {
        let s = MemorySecrets::default();
        assert_eq!(s.get_string(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap(), None);
        s.set_string(APP_SERVICE, accounts::ANTHROPIC_API_KEY, "sk-test").unwrap();
        assert_eq!(s.get_string(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap().as_deref(), Some("sk-test"));
        s.delete(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap();
        s.delete(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap();
        assert_eq!(s.get(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap(), None);
    }

    #[test]
    fn empty_string_reads_as_absent() {
        let s = MemorySecrets::default();
        s.set_string(APP_SERVICE, accounts::OPENAI_API_KEY, "").unwrap();
        assert_eq!(s.get_string(APP_SERVICE, accounts::OPENAI_API_KEY).unwrap(), None);
    }

    #[test]
    fn oversized_secrets_are_refused_before_the_backend() {
        let s = MemorySecrets::default();
        assert!(s.set(SYNC_SERVICE, "x", &vec![0u8; MAX_SECRET_BYTES]).is_ok());
        assert_eq!(
            s.set(SYNC_SERVICE, "x", &vec![0u8; MAX_SECRET_BYTES + 1]),
            Err(SecretError::TooLarge { len: MAX_SECRET_BYTES + 1, max: MAX_SECRET_BYTES })
        );
    }

    /// Real Credential Manager, under a throwaway service name it deletes afterwards.
    /// `cargo test --release --lib -- --ignored credential_manager` (plan Task 2's
    /// blob-size check). Asserts the full 2,560 bytes round-trip, i.e. raw bytes and not
    /// UTF-16, and that a realistic Google token set fits.
    #[test]
    #[ignore = "writes to the real Windows Credential Manager"]
    fn credential_manager_holds_a_full_blob_and_a_google_token_set() {
        let service = "com.mpointer.kestrel.test";
        let cm = CredentialManager;
        let full: Vec<u8> = (0..MAX_SECRET_BYTES).map(|i| (i % 251) as u8).collect();
        cm.set(service, "full-blob", &full).unwrap();
        assert_eq!(cm.get(service, "full-blob").unwrap(), Some(full));
        cm.delete(service, "full-blob").unwrap();
        assert_eq!(cm.get(service, "full-blob").unwrap(), None);

        // Shaped like api::oauth::StoredTokens for Google sign-in: a ~1,150-char RS256 ID
        // token JWT, a ~220-char access token, a ~100-char refresh token.
        let tokens = serde_json::json!({
            "access_token": "a".repeat(220),
            "refresh_token": "1//".to_string() + &"r".repeat(100),
            "id_token": "e".repeat(1150),
            "expires_at_unix_ms": 1_757_520_000_000i64,
            "granted_scope": "openid https://www.googleapis.com/auth/userinfo.email",
        })
        .to_string();
        println!("google token set: {} bytes of {MAX_SECRET_BYTES}", tokens.len());
        cm.set_string(service, "google-sync", &tokens).unwrap();
        assert_eq!(cm.get_string(service, "google-sync").unwrap().as_deref(), Some(tokens.as_str()));
        cm.delete(service, "google-sync").unwrap();
    }
}
