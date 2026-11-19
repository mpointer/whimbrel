//! sealed-fields wire format, Rust port.
//!
//! Token: `<version>:<base64url(nonce(12) | ciphertext | tag(16))>`, AES-256-GCM,
//! random nonce per value, unpadded base64url on write (up to two trailing `=`
//! are tolerated on read; the standard alphabet is not accepted). The GCM AAD is the ASCII version label,
//! followed by `0x00 || part` for every context part, so a token moved to another
//! version, table, column or row fails authentication. With no context the AAD
//! is the version alone. Byte-compatible with sealed-fields 0.6.1 (TypeScript and
//! Python); see `VECTORS.md`.

pub mod recovery;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use zeroize::Zeroize;

const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;

/// A 256-bit AES-GCM key. Zeroed on drop; `Debug` never prints the bytes.
pub struct SealedKey([u8; 32]);

impl SealedKey {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        SealedKey(b)
    }
}

impl Drop for SealedKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for SealedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealedKey(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealedError {
    /// Not a structurally valid token (prefix, base64, or too short).
    Malformed,
    /// Structurally valid but failed GCM authentication: wrong key, version,
    /// context, or tampered data.
    Authentication,
}

impl std::fmt::Display for SealedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SealedError::Malformed => f.write_str("malformed sealed token"),
            SealedError::Authentication => f.write_str("sealed token failed authentication"),
        }
    }
}

impl std::error::Error for SealedError {}

/// sealed-fields rejects empty context parts and parts containing NUL.
fn context_valid(context: &[&str]) -> bool {
    context.iter().all(|p| !p.is_empty() && !p.contains('\0'))
}

fn build_aad(version: &str, context: &[&str]) -> Vec<u8> {
    let mut aad = version.as_bytes().to_vec();
    for part in context {
        aad.push(0);
        aad.extend_from_slice(part.as_bytes());
    }
    aad
}

fn cipher(key: &SealedKey) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(&key.0).expect("32-byte key")
}

/// Seal `plaintext`. Panics if the OS random source fails or a context part is
/// empty or contains NUL, or `version` is not `v<digits>` (a programming error, as in sealed-fields).
pub fn seal(key: &SealedKey, version: &str, context: &[&str], plaintext: &[u8]) -> String {
    assert!(context_valid(context), "context parts must be non-empty and contain no NUL");
    assert!(version_valid(version), "version must match v\\d+");
    let mut nonce = [0u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).expect("OS random source unavailable");
    let aad = build_aad(version, context);
    // aes-gcm returns ciphertext || tag.
    let ct_and_tag = cipher(key)
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: &aad })
        .expect("AES-GCM encrypt");
    let mut blob = Vec::with_capacity(NONCE_BYTES + ct_and_tag.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ct_and_tag);
    format!("{version}:{}", URL_SAFE_NO_PAD.encode(blob))
}

fn decode_blob(s: &str) -> Option<Vec<u8>> {
    let body = s.strip_suffix("==").or_else(|| s.strip_suffix('=')).unwrap_or(s);
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return None;
    }
    URL_SAFE_NO_PAD.decode(body).ok()
}

fn version_valid(v: &str) -> bool {
    v.strip_prefix('v')
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

pub fn unseal(
    key: &SealedKey,
    version: &str,
    context: &[&str],
    token: &str,
) -> Result<Vec<u8>, SealedError> {
    if !context_valid(context) {
        return Err(SealedError::Malformed);
    }
    let (prefix, body) = token.split_once(':').ok_or(SealedError::Malformed)?;
    if !version_valid(prefix) {
        return Err(SealedError::Malformed);
    }
    let raw = decode_blob(body).ok_or(SealedError::Malformed)?;
    if raw.len() < NONCE_BYTES + TAG_BYTES {
        return Err(SealedError::Malformed);
    }
    // The token's own prefix is bound by the AAD: a re-labelled token can
    // never authenticate under the caller's expected version.
    if prefix != version {
        return Err(SealedError::Authentication);
    }
    let (nonce, ct_and_tag) = raw.split_at(NONCE_BYTES);
    let aad = build_aad(version, context);
    cipher(key)
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ct_and_tag, aad: &aad })
        .map_err(|_| SealedError::Authentication)
}
