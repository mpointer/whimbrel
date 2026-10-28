//! The account's group key on this device, and the wrapped-key wire format.
//!
//! - **Storage:** base64 of the 32 key bytes in Credential Manager
//!   (`com.mpointer.kestrel.sync` / `group-key`), as the Mac keeps it in its Keychain.
//! - **Wire format:** the Worker holds a wrapped key as base64 of `nonce ‖ ciphertext`,
//!   exactly what `core/ffi`'s `whimbrel_sync_wrap_group_key` produces and
//!   `whimbrel_sync_unwrap_group_key` reads. A Mac and this PC hand each other keys, so
//!   the two sides must agree byte for byte.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use crate::secrets::{accounts, SecretError, SecretStore, SYNC_SERVICE};

const NONCE_LEN: usize = 12;

pub fn encode_wrapped(wrapped: &sync::WrappedGroupKey) -> String {
    let mut blob = Vec::with_capacity(NONCE_LEN + wrapped.ciphertext.len());
    blob.extend_from_slice(&wrapped.nonce);
    blob.extend_from_slice(&wrapped.ciphertext);
    STANDARD.encode(blob)
}

pub fn decode_wrapped(b64: &str) -> Option<sync::WrappedGroupKey> {
    let blob = STANDARD.decode(b64.trim()).ok()?;
    if blob.len() < NONCE_LEN {
        return None;
    }
    let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
    Some(sync::WrappedGroupKey { nonce: nonce.try_into().ok()?, ciphertext: ciphertext.to_vec() })
}

pub fn decode_public_key(b64: &str) -> Option<[u8; 32]> {
    STANDARD.decode(b64.trim()).ok()?.try_into().ok()
}

/// Wraps the group key for `recipient`, in the wire format.
pub fn wrap_for(me: &sync::DeviceKeypair, recipient_public_key_b64: &str, key: &sync::GroupKey) -> Option<String> {
    let recipient = decode_public_key(recipient_public_key_b64)?;
    sync::wrap_group_key(me, &recipient, key).ok().map(|w| encode_wrapped(&w))
}

/// Unwraps a handed-off key. The Worker doesn't say which device approved us, so each
/// other approved device's public key is tried as the sender, as the Mac does.
pub fn unwrap_from_candidates(me: &sync::DeviceKeypair, wrapped_b64: &str, candidate_public_keys_b64: &[String]) -> Option<sync::GroupKey> {
    let wrapped = decode_wrapped(wrapped_b64)?;
    candidate_public_keys_b64
        .iter()
        .filter_map(|pk| decode_public_key(pk))
        .find_map(|sender| sync::unwrap_group_key(me, &sender, &wrapped).ok())
}

/// The stored group key. A malformed stored value reads as absent (and is logged); the
/// enrollment flow then waits for a fresh handoff rather than inventing a key.
pub fn load_group_key(secrets: &dyn SecretStore) -> Result<Option<sync::GroupKey>, SecretError> {
    let Some(b64) = secrets.get_string(SYNC_SERVICE, accounts::GROUP_KEY)? else {
        return Ok(None);
    };
    match STANDARD.decode(b64.trim()).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()) {
        Some(bytes) => Ok(Some(sync::GroupKey::from_bytes(bytes))),
        None => {
            tracing::warn!("the stored group key is malformed; ignoring it");
            Ok(None)
        }
    }
}

pub fn save_group_key(secrets: &dyn SecretStore, key: &sync::GroupKey) -> Result<(), SecretError> {
    secrets.set_string(SYNC_SERVICE, accounts::GROUP_KEY, &STANDARD.encode(key.to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn pk_b64(k: &sync::DeviceKeypair) -> String {
        STANDARD.encode(k.public_key_bytes())
    }

    #[test]
    fn a_key_wrapped_by_a_peer_unwraps_here_against_the_right_candidate() {
        let mac = sync::DeviceKeypair::generate();
        let pc = sync::DeviceKeypair::generate();
        let other = sync::DeviceKeypair::generate();
        let key = sync::GroupKey::generate();

        // The Mac wraps the account key for this PC, as whimbrel_sync_wrap_group_key would.
        let wrapped = wrap_for(&mac, &pk_b64(&pc), &key).unwrap();
        let got = unwrap_from_candidates(&pc, &wrapped, &[pk_b64(&other), pk_b64(&mac)]).expect("the Mac is a candidate");
        assert_eq!(got.to_bytes(), key.to_bytes());

        assert!(unwrap_from_candidates(&pc, &wrapped, &[pk_b64(&other)]).is_none(), "wrong sender");
        assert!(unwrap_from_candidates(&pc, "not base64", &[pk_b64(&mac)]).is_none());
    }

    #[test]
    fn the_wire_format_is_nonce_then_ciphertext() {
        let w = sync::WrappedGroupKey { nonce: [7; 12], ciphertext: vec![1, 2, 3] };
        let encoded = encode_wrapped(&w);
        assert_eq!(STANDARD.decode(&encoded).unwrap(), [vec![7u8; 12], vec![1, 2, 3]].concat());
        let back = decode_wrapped(&encoded).unwrap();
        assert_eq!((back.nonce, back.ciphertext), (w.nonce, w.ciphertext));
        assert!(decode_wrapped(&STANDARD.encode([0u8; 5])).is_none(), "shorter than a nonce");
    }

    #[test]
    fn the_group_key_round_trips_through_the_secret_store() {
        let secrets = MemorySecrets::default();
        assert!(load_group_key(&secrets).unwrap().is_none());
        let key = sync::GroupKey::generate();
        save_group_key(&secrets, &key).unwrap();
        assert_eq!(load_group_key(&secrets).unwrap().unwrap().to_bytes(), key.to_bytes());

        secrets.set_string(SYNC_SERVICE, accounts::GROUP_KEY, "garbage").unwrap();
        assert!(load_group_key(&secrets).unwrap().is_none(), "malformed reads as absent");
    }
}
