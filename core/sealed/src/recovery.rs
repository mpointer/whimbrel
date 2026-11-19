//! Recovery key: a passphrase-sealed text file and 24 BIP-39 words.
//!
//! File format (plain text, one item per line):
//!
//! ```text
//! whimbrel-recovery 1
//! kdf argon2id m=65536 t=3 p=1
//! salt <base64url, 16 bytes>
//! v1:<sealed token>
//! ```
//!
//! The 32-byte Argon2id output is the sealed-fields key; the plaintext is the
//! raw database key, sealed with version `v1` and context
//! `["whimbrel", "recovery-key"]`. The KDF parameters are read from the header
//! on open, but only within a floor (m >= 19456 KiB, t >= 2, p >= 1, so a hostile
//! file cannot downgrade them) and a ceiling (m <= 256 MiB, t <= 16, p <= 16, so
//! it cannot exhaust memory or time).
//!
//! The passphrase is normalised to Unicode NFC once, in [`normalise`], before
//! both the length check and the KDF, so the same passphrase typed on another
//! keyboard or OS (composed vs decomposed accents) opens the file. The KDF input
//! is the UTF-8 bytes of the NFC form.

use crate::{seal, unseal, SealedKey};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, Zeroizing};

/// Minimum passphrase length, counted in characters (not bytes).
pub const MIN_PASSPHRASE_CHARS: usize = 12;

const FORMAT_LINE: &str = "whimbrel-recovery 1";
const VERSION: &str = "v1";
const CONTEXT: [&str; 2] = ["whimbrel", "recovery-key"];
const SALT_BYTES: usize = 16;

// Parameters written by this version.
const WRITE_M_KIB: u32 = 65536;
const WRITE_T: u32 = 3;
const WRITE_P: u32 = 1;

// Floor enforced on read (downgrade protection) ...
const MIN_M_KIB: u32 = 19456;
const MIN_T: u32 = 2;
const MIN_P: u32 = 1;
// ... and a ceiling, so a hostile file cannot make us allocate without limit.
const MAX_M_KIB: u32 = 1 << 18; // 256 MiB
const MAX_T: u32 = 16;
const MAX_P: u32 = 16;

const _: () = {
    assert!(MIN_M_KIB <= WRITE_M_KIB && WRITE_M_KIB <= MAX_M_KIB);
    assert!(MIN_T <= WRITE_T && WRITE_T <= MAX_T);
    assert!(MIN_P <= WRITE_P && WRITE_P <= MAX_P);
};

/// The one place a passphrase is normalised (NFC).
fn normalise(passphrase: &str) -> Zeroizing<String> {
    Zeroizing::new(passphrase.nfc().collect::<String>())
}

/// The passphrase length the [`MIN_PASSPHRASE_CHARS`] check counts: characters (Unicode
/// scalar values) of the NFC form. For a live count in the setup screen.
pub fn passphrase_length(passphrase: &str) -> usize {
    normalise(passphrase).chars().count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryError {
    PassphraseTooShort,
    /// Authentication failed: the passphrase is wrong or the file was altered.
    WrongPassphraseOrDamaged,
    /// Not a recovery file we understand (format, header, parameters).
    Malformed,
    /// Wrong word count, unknown word, or checksum failure.
    BadWords,
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RecoveryError::PassphraseTooShort => "passphrase is too short",
            RecoveryError::WrongPassphraseOrDamaged => "wrong passphrase, or the file is damaged",
            RecoveryError::Malformed => "not a valid recovery file",
            RecoveryError::BadWords => "the recovery words are not valid",
        })
    }
}

impl std::error::Error for RecoveryError {}

/// Argon2id (v1.3) over the passphrase bytes; 32-byte output.
pub(crate) fn derive_key(
    passphrase: &str,
    salt: &[u8],
    m_kib: u32,
    t: u32,
    p: u32,
) -> Result<[u8; 32], RecoveryError> {
    let params = Params::new(m_kib, t, p, Some(32)).map_err(|_| RecoveryError::Malformed)?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|_| RecoveryError::Malformed)?;
    Ok(out)
}

fn file_key(
    passphrase: &str,
    salt: &[u8],
    m: u32,
    t: u32,
    p: u32,
) -> Result<SealedKey, RecoveryError> {
    let mut raw = derive_key(passphrase, salt, m, t, p)?;
    let key = SealedKey::from_bytes(raw);
    raw.zeroize();
    Ok(key)
}

/// Seal `db_key` under a key derived from `passphrase`. Fails with
/// `PassphraseTooShort` below [`MIN_PASSPHRASE_CHARS`] characters.
pub fn make_recovery_file(db_key: &[u8; 32], passphrase: &str) -> Result<String, RecoveryError> {
    let passphrase = normalise(passphrase);
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(RecoveryError::PassphraseTooShort);
    }
    let mut salt = [0u8; SALT_BYTES];
    getrandom::fill(&mut salt).expect("OS random source unavailable");
    let key = file_key(&passphrase, &salt, WRITE_M_KIB, WRITE_T, WRITE_P)?;
    let token = seal(&key, VERSION, &CONTEXT, db_key);
    Ok(format!(
        "{FORMAT_LINE}\nkdf argon2id m={WRITE_M_KIB} t={WRITE_T} p={WRITE_P}\nsalt {}\n{token}\n",
        URL_SAFE_NO_PAD.encode(salt)
    ))
}

fn parse_kdf_line(line: &str) -> Option<(u32, u32, u32)> {
    let rest = line.strip_prefix("kdf argon2id ")?;
    let mut it = rest.split(' ');
    let m = it.next()?.strip_prefix("m=")?.parse::<u32>().ok()?;
    let t = it.next()?.strip_prefix("t=")?.parse::<u32>().ok()?;
    let p = it.next()?.strip_prefix("p=")?.parse::<u32>().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((m, t, p))
}

/// Open a file made by [`make_recovery_file`]. The passphrase length is not
/// checked here; a wrong passphrase and a damaged token are indistinguishable
/// and both return `WrongPassphraseOrDamaged`.
pub fn open_recovery_file(file: &str, passphrase: &str) -> Result<[u8; 32], RecoveryError> {
    let lines: Vec<&str> = file.lines().map(|l| l.trim_end_matches('\r')).collect();
    // Exactly four lines; trailing blank lines are tolerated.
    let (header, rest) = lines.split_at(lines.len().min(4));
    if header.len() != 4 || rest.iter().any(|l| !l.trim().is_empty()) {
        return Err(RecoveryError::Malformed);
    }
    if header[0] != FORMAT_LINE {
        return Err(RecoveryError::Malformed);
    }
    let (m, t, p) = parse_kdf_line(header[1]).ok_or(RecoveryError::Malformed)?;
    if !(MIN_M_KIB..=MAX_M_KIB).contains(&m)
        || !(MIN_T..=MAX_T).contains(&t)
        || !(MIN_P..=MAX_P).contains(&p)
    {
        return Err(RecoveryError::Malformed);
    }
    let salt_b64 = header[2].strip_prefix("salt ").ok_or(RecoveryError::Malformed)?;
    let salt = URL_SAFE_NO_PAD.decode(salt_b64).map_err(|_| RecoveryError::Malformed)?;
    if salt.len() != SALT_BYTES {
        return Err(RecoveryError::Malformed);
    }
    let passphrase = normalise(passphrase);
    let key = file_key(&passphrase, &salt, m, t, p)?;
    let mut plain = unseal(&key, VERSION, &CONTEXT, header[3].trim())
        .map_err(|_| RecoveryError::WrongPassphraseOrDamaged)?;
    let out = <[u8; 32]>::try_from(plain.as_slice())
        .map_err(|_| RecoveryError::WrongPassphraseOrDamaged);
    plain.zeroize();
    out
}

/// The 24-word BIP-39 (English) encoding of the 32 raw key bytes.
pub fn key_to_words(db_key: &[u8; 32]) -> Vec<String> {
    bip39::Mnemonic::from_entropy(db_key)
        .expect("32 bytes is a valid BIP-39 entropy length")
        .words()
        .map(str::to_owned)
        .collect()
}

/// Inverse of [`key_to_words`]. Each word is trimmed and lowercased first. Any
/// other word count, unknown word or checksum failure is `BadWords`.
pub fn words_to_key(words: &[String]) -> Result<[u8; 32], RecoveryError> {
    if words.len() != 24 {
        return Err(RecoveryError::BadWords);
    }
    let mut phrase = Zeroizing::new(String::with_capacity(words.len() * 9));
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            phrase.push(' ');
        }
        phrase.extend(w.trim().chars().flat_map(char::to_lowercase));
    }
    let m = bip39::Mnemonic::parse_in_normalized(bip39::Language::English, phrase.as_str())
        .map_err(|_| RecoveryError::BadWords)?;
    let mut entropy = m.to_entropy();
    let out = <[u8; 32]>::try_from(entropy.as_slice()).map_err(|_| RecoveryError::BadWords);
    entropy.zeroize();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn argon2_known_answer_for_fixed_salt() {
        // Reference: `printf 'correct horse battery' | argon2 0123456789abcdef -id -m 16 -t 3 -p 1 -l 32 -r`
        let out = derive_key("correct horse battery", b"0123456789abcdef", 65536, 3, 1).unwrap();
        assert_eq!(hex(&out), "00bac562b407875296682e0b8043688e09be94fcea326e99aa62d0edb44404fc");
    }

    #[test]
    fn argon2_known_answer_non_ascii_over_nfc_bytes() {
        // Reference (reference argon2 CLI over the NFC UTF-8 bytes of "caf\u{e9} horse battery"):
        // `printf 'caf\xc3\xa9 horse battery' | argon2 0123456789abcdef -id -m 16 -t 3 -p 1 -l 32 -r`
        let nfd = "cafe\u{301} horse battery";
        let out = derive_key(&normalise(nfd), b"0123456789abcdef", 65536, 3, 1).unwrap();
        assert_eq!(hex(&out), "169faa3277c0cd9f6e34023613aa418af516a0827b77604d84afa25222e6ae44");
    }

    #[test]
    #[ignore]
    fn kdf_under_one_second_on_this_machine() {
        let start = std::time::Instant::now();
        derive_key("correct horse battery", b"0123456789abcdef", 65536, 3, 1).unwrap();
        let elapsed = start.elapsed();
        println!("argon2id m=65536 t=3 p=1 took {elapsed:?}");
        // Unoptimised builds are ~15x slower and say nothing about shipped speed.
        if !cfg!(debug_assertions) {
            assert!(elapsed.as_secs_f64() < 1.0, "KDF took {elapsed:?}");
        }
    }
}
