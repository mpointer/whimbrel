//! Cryptographic primitives for multi-device sync
//! (docs/superpowers/specs/2026-07-29-multi-device-sync-accounts-design.md).
//!
//! This crate implements only the cryptographic core: device keypairs, the
//! shared group encryption key, key-wrapping between devices, and payload
//! encryption. Networking, Turso, and the Cloudflare Worker are separate,
//! later work — nothing here talks to the network or touches disk.

mod comparison_code;
pub use comparison_code::device_comparison_code;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use hkdf::Hkdf;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use std::fmt;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

/// A device's long-term X25519 keypair, used to wrap/unwrap the shared
/// group encryption key during device enrollment. Not used to encrypt
/// payloads directly — see `GroupKey` for that.
pub struct DeviceKeypair {
    secret: StaticSecret,
    public: PublicKey,
}

impl DeviceKeypair {
    /// Generates a fresh random keypair using the OS CSPRNG.
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        DeviceKeypair { secret, public }
    }

    /// The public key, safe to share with other devices and the server.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.public.to_bytes()
    }

    /// The private key bytes, for the caller to persist in secure storage
    /// (e.g. the macOS Keychain). Never send these anywhere.
    pub fn to_secret_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    /// Reconstructs a keypair from previously-persisted private key bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        DeviceKeypair { secret, public }
    }

    fn diffie_hellman(
        &self,
        their_public: &[u8; 32],
    ) -> Result<Zeroizing<[u8; 32]>, SyncCryptoError> {
        let their_public = PublicKey::from(*their_public);
        let shared = self.secret.diffie_hellman(&their_public);
        if !shared.was_contributory() {
            return Err(SyncCryptoError);
        }
        Ok(Zeroizing::new(shared.to_bytes()))
    }
}

/// The symmetric key shared by every device in a user's account, used to
/// encrypt/decrypt synced payloads. Generated once by the first device
/// enrolled into an account and distributed to later devices via
/// `wrap_group_key`/`unwrap_group_key`.
pub struct GroupKey([u8; 32]);

impl GroupKey {
    /// Generates a fresh random group key using the OS CSPRNG.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        GroupKey(bytes)
    }

    /// The raw key bytes, for the caller to persist in secure storage.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0
    }

    /// Reconstructs a group key from previously-persisted bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        GroupKey(bytes)
    }
}

impl Drop for GroupKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

fn derive_wrap_key(shared_secret: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, shared_secret);
    let mut okm = Zeroizing::new([0u8; 32]);
    hk.expand(b"kestrel-sync-group-key-wrap", &mut *okm)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    okm
}

/// A group key, encrypted for one specific recipient device's public key.
/// Deliberately carries no claim about who sent it — the caller must supply
/// the expected sender's public key from an already-trusted source (e.g. the
/// enrollment flow's authenticated approval channel), never trust a field
/// embedded in this untrusted blob about its own provenance.
pub struct WrappedGroupKey {
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

/// Returned when decryption, unwrapping, or a key-agreement check fails —
/// deliberately carries no detail (wrong key, tampered ciphertext, wrong
/// AAD, wrong format version, and a rejected low-order public key all look
/// identical to a caller, which is correct: don't leak which part failed).
#[derive(Debug)]
pub struct SyncCryptoError;

impl fmt::Display for SyncCryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sync crypto operation failed")
    }
}

impl std::error::Error for SyncCryptoError {}

/// Wraps `group_key` so only the device holding `recipient_public_key`'s
/// matching private key can recover it, given the correct sender public key
/// separately. Fails if `recipient_public_key` is a low-order point (a
/// degenerate Diffie-Hellman result an attacker could predict).
pub fn wrap_group_key(
    sender: &DeviceKeypair,
    recipient_public_key: &[u8; 32],
    group_key: &GroupKey,
) -> Result<WrappedGroupKey, SyncCryptoError> {
    let shared = sender.diffie_hellman(recipient_public_key)?;
    let wrap_key = derive_wrap_key(&shared);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&*wrap_key));
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, group_key.to_bytes().as_ref())
        .expect("ChaCha20Poly1305 encryption of a 32-byte plaintext cannot fail");
    Ok(WrappedGroupKey {
        nonce: nonce_bytes,
        ciphertext,
    })
}

/// Recovers the group key from a `WrappedGroupKey`. `sender_public_key` must
/// come from an already-trusted source the caller has independently
/// verified (e.g. "this is the device I requested approval from") — never
/// from a field inside the wrapped blob itself, which an attacker controls.
/// Fails if `recipient` is not the device the key was wrapped for, if
/// `sender_public_key` doesn't match who actually wrapped it, if the
/// ciphertext was tampered with, or if `sender_public_key` is a low-order
/// point.
pub fn unwrap_group_key(
    recipient: &DeviceKeypair,
    sender_public_key: &[u8; 32],
    wrapped: &WrappedGroupKey,
) -> Result<GroupKey, SyncCryptoError> {
    let shared = recipient.diffie_hellman(sender_public_key)?;
    let wrap_key = derive_wrap_key(&shared);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&*wrap_key));
    let nonce = Nonce::from_slice(&wrapped.nonce);
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(nonce, wrapped.ciphertext.as_ref())
            .map_err(|_| SyncCryptoError)?,
    );
    let bytes: [u8; 32] = plaintext
        .as_slice()
        .try_into()
        .map_err(|_| SyncCryptoError)?;
    Ok(GroupKey::from_bytes(bytes))
}

const PAYLOAD_FORMAT_VERSION: u8 = 1;

/// Encrypts arbitrary payload bytes (a serialized `capture_item`, `meeting`,
/// `transcript_segment`, or `summary` row) under the group key. `aad`
/// (additional authenticated data — e.g. `account_id || device_id ||
/// sequence_number`) is authenticated but not encrypted: it binds this
/// ciphertext to the context it belongs to, so a party with write access to
/// the sync store cannot move, duplicate, or replay it into a different
/// row without decryption failing. The caller is responsible for
/// serializing the payload and choosing `aad` — this function only handles
/// bytes. Returns `version (1 byte) || nonce (12 bytes) || ciphertext`,
/// ready to upload as one blob.
pub fn encrypt_payload(group_key: &GroupKey, plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let key_bytes = Zeroizing::new(group_key.to_bytes());
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&*key_bytes));
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("ChaCha20Poly1305 encryption cannot fail for well-formed input");
    let mut out = Vec::with_capacity(1 + nonce_bytes.len() + ciphertext.len());
    out.push(PAYLOAD_FORMAT_VERSION);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    out
}

/// Decrypts a blob produced by `encrypt_payload`. `aad` must exactly match
/// what was passed to `encrypt_payload` for this blob. Fails if the blob is
/// too short, the format version is unrecognized, the group key is wrong,
/// the AAD doesn't match, or the ciphertext was tampered with.
pub fn decrypt_payload(
    group_key: &GroupKey,
    versioned_nonce_and_ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, SyncCryptoError> {
    if versioned_nonce_and_ciphertext.len() < 1 + 12 {
        return Err(SyncCryptoError);
    }
    let (version, rest) = versioned_nonce_and_ciphertext.split_at(1);
    if version[0] != PAYLOAD_FORMAT_VERSION {
        return Err(SyncCryptoError);
    }
    let (nonce_bytes, ciphertext) = rest.split_at(12);
    let key_bytes = Zeroizing::new(group_key.to_bytes());
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&*key_bytes));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| SyncCryptoError)
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_keypair_round_trips_through_bytes() {
        let original = crate::DeviceKeypair::generate();
        let original_public = original.public_key_bytes();
        let original_private = original.to_secret_bytes();

        let restored = crate::DeviceKeypair::from_bytes(original_private);

        assert_eq!(restored.public_key_bytes(), original_public);
        assert_eq!(restored.to_secret_bytes(), original_private);
    }

    #[test]
    fn two_generated_keypairs_are_different() {
        let a = crate::DeviceKeypair::generate();
        let b = crate::DeviceKeypair::generate();
        assert_ne!(a.public_key_bytes(), b.public_key_bytes());
    }

    #[test]
    fn group_key_round_trips_through_bytes() {
        let original = crate::GroupKey::generate();
        let original_bytes = original.to_bytes();

        let restored = crate::GroupKey::from_bytes(original_bytes);

        assert_eq!(restored.to_bytes(), original_bytes);
    }

    #[test]
    fn two_generated_group_keys_are_different() {
        let a = crate::GroupKey::generate();
        let b = crate::GroupKey::generate();
        assert_ne!(a.to_bytes(), b.to_bytes());
    }

    #[test]
    fn round_trips_group_key_through_wrap_unwrap() {
        let sender = crate::DeviceKeypair::generate();
        let recipient = crate::DeviceKeypair::generate();
        let group_key = crate::GroupKey::generate();
        let original_bytes = group_key.to_bytes();

        let wrapped =
            crate::wrap_group_key(&sender, &recipient.public_key_bytes(), &group_key).unwrap();
        let unwrapped =
            crate::unwrap_group_key(&recipient, &sender.public_key_bytes(), &wrapped).unwrap();

        assert_eq!(unwrapped.to_bytes(), original_bytes);
    }

    #[test]
    fn wrong_recipient_cannot_unwrap() {
        let sender = crate::DeviceKeypair::generate();
        let recipient = crate::DeviceKeypair::generate();
        let wrong_device = crate::DeviceKeypair::generate();
        let group_key = crate::GroupKey::generate();

        let wrapped =
            crate::wrap_group_key(&sender, &recipient.public_key_bytes(), &group_key).unwrap();
        let result = crate::unwrap_group_key(&wrong_device, &sender.public_key_bytes(), &wrapped);

        assert!(result.is_err());
    }

    #[test]
    fn wrong_claimed_sender_cannot_unwrap() {
        let sender = crate::DeviceKeypair::generate();
        let impostor = crate::DeviceKeypair::generate();
        let recipient = crate::DeviceKeypair::generate();
        let group_key = crate::GroupKey::generate();

        let wrapped =
            crate::wrap_group_key(&sender, &recipient.public_key_bytes(), &group_key).unwrap();
        let result = crate::unwrap_group_key(&recipient, &impostor.public_key_bytes(), &wrapped);

        assert!(result.is_err());
    }

    #[test]
    fn wrap_group_key_rejects_low_order_recipient_public_key() {
        let sender = crate::DeviceKeypair::generate();
        let group_key = crate::GroupKey::generate();
        let low_order_public_key = [0u8; 32];

        let result = crate::wrap_group_key(&sender, &low_order_public_key, &group_key);

        assert!(result.is_err());
    }

    #[test]
    fn tampered_wrapped_ciphertext_fails_to_unwrap() {
        let sender = crate::DeviceKeypair::generate();
        let recipient = crate::DeviceKeypair::generate();
        let group_key = crate::GroupKey::generate();

        let mut wrapped =
            crate::wrap_group_key(&sender, &recipient.public_key_bytes(), &group_key).unwrap();
        let last = wrapped.ciphertext.len() - 1;
        wrapped.ciphertext[last] ^= 0xFF;

        let result = crate::unwrap_group_key(&recipient, &sender.public_key_bytes(), &wrapped);
        assert!(result.is_err());
    }

    #[test]
    fn tampered_wrapped_nonce_fails_to_unwrap() {
        let sender = crate::DeviceKeypair::generate();
        let recipient = crate::DeviceKeypair::generate();
        let group_key = crate::GroupKey::generate();

        let mut wrapped =
            crate::wrap_group_key(&sender, &recipient.public_key_bytes(), &group_key).unwrap();
        wrapped.nonce[0] ^= 0xFF;

        let result = crate::unwrap_group_key(&recipient, &sender.public_key_bytes(), &wrapped);
        assert!(result.is_err());
    }

    #[test]
    fn round_trips_payload_through_encrypt_decrypt() {
        let group_key = crate::GroupKey::generate();
        let plaintext = b"hello from a meeting transcript";

        let ciphertext = crate::encrypt_payload(&group_key, plaintext, b"meeting-1");
        let decrypted = crate::decrypt_payload(&group_key, &ciphertext, b"meeting-1").unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn tampered_ciphertext_fails_to_decrypt() {
        let group_key = crate::GroupKey::generate();
        let plaintext = b"hello from a meeting transcript";

        let mut ciphertext = crate::encrypt_payload(&group_key, plaintext, b"");
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 0xFF;

        let result = crate::decrypt_payload(&group_key, &ciphertext, b"");
        assert!(result.is_err());
    }

    #[test]
    fn wrong_group_key_fails_to_decrypt() {
        let group_key = crate::GroupKey::generate();
        let other_key = crate::GroupKey::generate();
        let plaintext = b"hello from a meeting transcript";

        let ciphertext = crate::encrypt_payload(&group_key, plaintext, b"");
        let result = crate::decrypt_payload(&other_key, &ciphertext, b"");

        assert!(result.is_err());
    }

    #[test]
    fn truncated_ciphertext_fails_to_decrypt_instead_of_panicking() {
        let group_key = crate::GroupKey::generate();
        let too_short = vec![0u8; 5];

        let result = crate::decrypt_payload(&group_key, &too_short, b"");
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_payload_fails_with_wrong_aad() {
        let group_key = crate::GroupKey::generate();
        let plaintext = b"hello from a meeting transcript";

        let ciphertext = crate::encrypt_payload(&group_key, plaintext, b"meeting-1");
        let result = crate::decrypt_payload(&group_key, &ciphertext, b"meeting-2");

        assert!(result.is_err());
    }

    #[test]
    fn decrypt_payload_fails_with_unknown_version_byte() {
        let group_key = crate::GroupKey::generate();
        let plaintext = b"hello from a meeting transcript";

        let mut ciphertext = crate::encrypt_payload(&group_key, plaintext, b"");
        ciphertext[0] = 99;

        let result = crate::decrypt_payload(&group_key, &ciphertext, b"");
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_payload_uses_a_fresh_nonce_each_call() {
        let group_key = crate::GroupKey::generate();
        let plaintext = b"same plaintext both times";

        let first = crate::encrypt_payload(&group_key, plaintext, b"");
        let second = crate::encrypt_payload(&group_key, plaintext, b"");

        assert_ne!(first, second);
        assert_ne!(first[1..13], second[1..13]);
    }
}
