//! This device's sync identity (plan Decision 4).
//!
//! As on the Mac (`AppState.swift`), the store's `device_id` is the base64 (standard,
//! padded) X25519 public key of the device keypair. The keypair's secret lives in
//! Credential Manager as base64 text under `com.mpointer.kestrel.sync` /
//! `device-secret-key`. It is created on first launch and never regenerated silently:
//! a new keypair is a new device to the sync account and to every peer's timeline.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use crate::secrets::{accounts, SecretError, SecretStore, SYNC_SERVICE};

pub struct DeviceIdentity {
    secret: [u8; 32],
    /// The store's `device_id` and the Worker's `devicePublicKey`.
    pub device_id: String,
}

impl std::fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceIdentity").field("device_id", &self.device_id).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    Secrets(SecretError),
    /// The stored secret is not 32 bytes of base64. Refused rather than replaced: the
    /// user has to decide to become a new device.
    Malformed,
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::Secrets(e) => write!(f, "{e}"),
            IdentityError::Malformed => write!(
                f,
                "the stored device key ({SYNC_SERVICE} / {}) is malformed; delete it to enroll as a new device",
                accounts::DEVICE_SECRET_KEY
            ),
        }
    }
}

impl std::error::Error for IdentityError {}

impl From<SecretError> for IdentityError {
    fn from(e: SecretError) -> Self {
        IdentityError::Secrets(e)
    }
}

impl DeviceIdentity {
    /// Loads the device keypair, creating and storing one on first launch. The flag is
    /// `true` when a new identity was created.
    pub fn load_or_create(secrets: &dyn SecretStore) -> Result<(Self, bool), IdentityError> {
        if let Some(stored) = secrets.get_string(SYNC_SERVICE, accounts::DEVICE_SECRET_KEY)? {
            let bytes: [u8; 32] = STANDARD
                .decode(stored.trim())
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or(IdentityError::Malformed)?;
            return Ok((Self::from_secret(bytes), false));
        }
        let keypair = sync::DeviceKeypair::generate();
        let secret = keypair.to_secret_bytes();
        secrets.set_string(SYNC_SERVICE, accounts::DEVICE_SECRET_KEY, &STANDARD.encode(secret))?;
        Ok((Self::from_secret(secret), true))
    }

    fn from_secret(secret: [u8; 32]) -> Self {
        let keypair = sync::DeviceKeypair::from_bytes(secret);
        DeviceIdentity { secret, device_id: STANDARD.encode(keypair.public_key_bytes()) }
    }

    /// The keypair, for wrapping and unwrapping the group key (Task 5).
    pub fn keypair(&self) -> sync::DeviceKeypair {
        sync::DeviceKeypair::from_bytes(self.secret)
    }

    /// A log-safe prefix of the device id.
    pub fn short_id(&self) -> &str {
        &self.device_id[..8]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    #[test]
    fn first_launch_creates_and_stores_an_identity() {
        let secrets = MemorySecrets::default();
        let (id, created) = DeviceIdentity::load_or_create(&secrets).unwrap();
        assert!(created);
        assert_eq!(STANDARD.decode(&id.device_id).unwrap().len(), 32);
        assert!(secrets.get_string(SYNC_SERVICE, accounts::DEVICE_SECRET_KEY).unwrap().is_some());
    }

    #[test]
    fn identity_is_stable_across_launches() {
        let secrets = MemorySecrets::default();
        let (first, _) = DeviceIdentity::load_or_create(&secrets).unwrap();
        let (second, created) = DeviceIdentity::load_or_create(&secrets).unwrap();
        assert!(!created);
        assert_eq!(first.device_id, second.device_id);
    }

    #[test]
    fn deleting_the_secret_makes_a_new_device() {
        let secrets = MemorySecrets::default();
        let (first, _) = DeviceIdentity::load_or_create(&secrets).unwrap();
        secrets.delete(SYNC_SERVICE, accounts::DEVICE_SECRET_KEY).unwrap();
        let (second, created) = DeviceIdentity::load_or_create(&secrets).unwrap();
        assert!(created);
        assert_ne!(first.device_id, second.device_id);
    }

    #[test]
    fn device_id_is_the_public_key_the_mac_would_compute() {
        // Same derivation as core/ffi's whimbrel_sync_device_public_key: STANDARD base64
        // of DeviceKeypair::from_bytes(secret).public_key_bytes().
        let secret = [7u8; 32];
        let secrets = MemorySecrets::default();
        secrets.set_string(SYNC_SERVICE, accounts::DEVICE_SECRET_KEY, &STANDARD.encode(secret)).unwrap();
        let (id, _) = DeviceIdentity::load_or_create(&secrets).unwrap();
        let expected = STANDARD.encode(sync::DeviceKeypair::from_bytes(secret).public_key_bytes());
        assert_eq!(id.device_id, expected);
        assert_eq!(id.keypair().public_key_bytes(), sync::DeviceKeypair::from_bytes(secret).public_key_bytes());
    }

    #[test]
    fn a_malformed_stored_secret_is_refused_not_replaced() {
        let secrets = MemorySecrets::default();
        secrets.set_string(SYNC_SERVICE, accounts::DEVICE_SECRET_KEY, "not base64!").unwrap();
        assert_eq!(DeviceIdentity::load_or_create(&secrets).unwrap_err(), IdentityError::Malformed);
        assert_eq!(
            secrets.get_string(SYNC_SERVICE, accounts::DEVICE_SECRET_KEY).unwrap().as_deref(),
            Some("not base64!"),
            "the stored value must be left for the user to decide about"
        );
    }

    #[test]
    fn debug_output_never_contains_the_secret() {
        let secrets = MemorySecrets::default();
        let (id, _) = DeviceIdentity::load_or_create(&secrets).unwrap();
        let stored = secrets.get_string(SYNC_SERVICE, accounts::DEVICE_SECRET_KEY).unwrap().unwrap();
        assert!(!format!("{id:?}").contains(&stored));
    }
}
