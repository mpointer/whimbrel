//! Recovery-key setup and restore (X4 encryption at rest, design 3.4).
//!
//! The recovery key is the database key itself (the same 32 bytes). Setup produces it in
//! three forms from one key: the passphrase-sealed file `Whimbrel recovery key.txt`, the 24
//! words and (in the shell) a QR of the words. The passphrase protects the file only; the
//! words are the raw key.
//!
//! Everything here is in memory: nothing reads or writes a file, and nothing touches a key
//! store (the shell saves the file and stores the key). The passphrase is normalised to NFC
//! by `core/sealed` before both the 12-character check and the KDF.
//!
//! Each call that takes a passphrase runs Argon2id (64 MiB, 3 passes): about a second on a
//! slow PC, so shells call these off their UI thread.

pub use sealed::recovery::{RecoveryError, MIN_PASSPHRASE_CHARS};

use memory::DbKey;
use rand::RngCore;
use sealed::recovery::{key_to_words, make_recovery_file, open_recovery_file, words_to_key};
use zeroize::{Zeroize, Zeroizing};

/// What setup hands the shell: the key to encrypt with (and store in the key store), the
/// recovery file's text (sealed under the passphrase, safe to write to disk) and the 24
/// words (the raw key: show, never log). `Debug` shows neither the key nor the words; the
/// words are wiped on drop.
pub struct SetupMaterial {
    pub key: DbKey,
    pub recovery_file: String,
    pub words: Vec<String>,
}

impl std::fmt::Debug for SetupMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupMaterial")
            .field("key", &self.key)
            .field("recovery_file", &format_args!("<{} bytes>", self.recovery_file.len()))
            .field("words", &format_args!("<{} words, redacted>", self.words.len()))
            .finish()
    }
}

impl Drop for SetupMaterial {
    fn drop(&mut self) {
        for w in self.words.iter_mut() {
            w.zeroize();
        }
    }
}

/// Start recovery setup with a fresh random 32-byte key. Fails with `PassphraseTooShort`
/// below [`MIN_PASSPHRASE_CHARS`] characters (counted after NFC).
pub fn begin_setup(passphrase: &str) -> Result<SetupMaterial, RecoveryError> {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let key = DbKey::new(bytes);
    bytes.zeroize();
    material(key, passphrase)
}

/// Recovery setup for a key that already exists (the launch flow's `db-key-pending`, which
/// must never be replaced by a fresh one). Same checks as [`begin_setup`].
pub fn setup_for_key(key: &DbKey, passphrase: &str) -> Result<SetupMaterial, RecoveryError> {
    let mut bytes = *key.expose_secret_bytes();
    let copy = DbKey::new(bytes);
    bytes.zeroize();
    material(copy, passphrase)
}

fn material(key: DbKey, passphrase: &str) -> Result<SetupMaterial, RecoveryError> {
    let recovery_file = make_recovery_file(key.expose_secret_bytes(), passphrase)?;
    let words = key_to_words(key.expose_secret_bytes());
    Ok(SetupMaterial { key, recovery_file, words })
}

/// The type-it-back test: does `passphrase` open `recovery_file` (the text setup just made,
/// held in memory; no file is read)? Any failure (wrong passphrase, damaged or foreign
/// text) is `false`. The unsealed key is wiped at once.
pub fn verify_passphrase(recovery_file: &str, passphrase: &str) -> bool {
    match open_recovery_file(recovery_file, passphrase) {
        Ok(mut bytes) => {
            bytes.zeroize();
            true
        }
        Err(_) => false,
    }
}

/// The key from a recovery file's text and its passphrase. A wrong passphrase and a
/// damaged file both give `WrongPassphraseOrDamaged`; text that is not a recovery file
/// gives `Malformed`. The passphrase length is not checked here.
pub fn restore_key_from_file(recovery_file: &str, passphrase: &str) -> Result<DbKey, RecoveryError> {
    let mut bytes = open_recovery_file(recovery_file, passphrase)?;
    let key = DbKey::new(bytes);
    bytes.zeroize();
    Ok(key)
}

/// The key from the 24 words, however they were pasted: one string or several, any mix of
/// spaces, tabs, newlines and commas, any capitals, and numbered ("1. word", "2) word")
/// as the setup screen lists them, also with the number glued on ("1.word"). Anything else (wrong count, unknown word, checksum) is
/// `BadWords`.
pub fn restore_key_from_words(words: &[String]) -> Result<DbKey, RecoveryError> {
    let tokens: Vec<Zeroizing<String>> = words
        .iter()
        .flat_map(|chunk| chunk.split(|c: char| c.is_whitespace() || c == ','))
        .map(|t| t.trim_matches(|c: char| matches!(c, '.' | ')' | '(' | ';' | ':')))
        // A list number glued to its word ("1.abandon", "2)ability"): drop the number and its mark.
        .map(|t| match t.trim_start_matches(|c: char| c.is_ascii_digit()) {
            rest if rest.len() < t.len() && (rest.starts_with('.') || rest.starts_with(')')) => &rest[1..],
            _ => t,
        })
        .filter(|t| !t.is_empty() && !t.chars().all(|c| c.is_ascii_digit()))
        .map(|t| Zeroizing::new(t.to_owned()))
        .collect();
    let plain: Vec<String> = tokens.iter().map(|t| t.as_str().to_owned()).collect();
    let result = words_to_key(&plain);
    let mut plain = plain;
    for t in plain.iter_mut() {
        t.zeroize();
    }
    let mut bytes = result?;
    let key = DbKey::new(bytes);
    bytes.zeroize();
    Ok(key)
}

/// The passphrase length the minimum counts (characters of the NFC form), for the setup
/// screen's live count.
pub fn passphrase_length(passphrase: &str) -> usize {
    sealed::recovery::passphrase_length(passphrase)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "correct horse battery staple";

    fn strings(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn begin_setup_rejects_short_passphrase() {
        assert_eq!(begin_setup("elevenchars").err(), Some(RecoveryError::PassphraseTooShort));
        assert_eq!(begin_setup("").err(), Some(RecoveryError::PassphraseTooShort));
        // Counted after NFC: "e" + combining acute is one character, so 11 + 1 = 12 is
        // short by one when typed decomposed.
        assert!(begin_setup("abcdefghijke\u{301}").is_ok());
        assert_eq!(begin_setup("abcdefghije\u{301}").err(), Some(RecoveryError::PassphraseTooShort));
    }

    #[test]
    fn begin_setup_produces_matching_file_and_words() {
        let m = begin_setup(PASS).unwrap();
        assert_eq!(m.words.len(), 24);
        assert!(m.recovery_file.starts_with("whimbrel-recovery 1\n"));
        let from_file = restore_key_from_file(&m.recovery_file, PASS).unwrap();
        let from_words = restore_key_from_words(&m.words).unwrap();
        assert_eq!(from_file.expose_secret_bytes(), m.key.expose_secret_bytes());
        assert_eq!(from_words.expose_secret_bytes(), m.key.expose_secret_bytes());
        // A second setup makes a different key.
        let other = begin_setup(PASS).unwrap();
        assert_ne!(other.key.expose_secret_bytes(), m.key.expose_secret_bytes());
    }

    #[test]
    fn setup_for_key_keeps_the_given_key() {
        let key = DbKey::new([7u8; 32]);
        let m = setup_for_key(&key, PASS).unwrap();
        assert_eq!(m.key.expose_secret_bytes(), &[7u8; 32]);
        assert_eq!(restore_key_from_file(&m.recovery_file, PASS).unwrap().expose_secret_bytes(), &[7u8; 32]);
        assert_eq!(restore_key_from_words(&m.words).unwrap().expose_secret_bytes(), &[7u8; 32]);
        assert_eq!(setup_for_key(&key, "short").err(), Some(RecoveryError::PassphraseTooShort));
        // A retry with the same (pending) key shows the same words; only the file's salt differs.
        assert_eq!(setup_for_key(&key, PASS).unwrap().words, m.words);
    }

    #[test]
    fn verify_passphrase_true_for_right_false_for_wrong() {
        let m = begin_setup(PASS).unwrap();
        assert!(verify_passphrase(&m.recovery_file, PASS));
        assert!(!verify_passphrase(&m.recovery_file, "correct horse battery stapler"));
        assert!(!verify_passphrase(&m.recovery_file, ""));
        assert!(!verify_passphrase("not a recovery file", PASS));

        // The same passphrase typed composed (NFC) or decomposed (NFD) is the same passphrase.
        let nfc = "caf\u{e9} horse battery";
        let nfd = "cafe\u{301} horse battery";
        let accented = begin_setup(nfc).unwrap();
        assert!(verify_passphrase(&accented.recovery_file, nfd));
        let accented = begin_setup(nfd).unwrap();
        assert!(verify_passphrase(&accented.recovery_file, nfc));
    }

    #[test]
    fn restore_from_words_accepts_spaces_and_capitals() {
        let key = DbKey::new([42u8; 32]);
        let m = setup_for_key(&key, PASS).unwrap();
        let words = m.words.clone();
        let joined = words.join(" ");

        // One pasted string, extra spaces, tabs, newlines and capitals.
        let messy = format!("  {}  \n", words.iter().map(|w| w.to_uppercase()).collect::<Vec<_>>().join("   \t"));
        let k = restore_key_from_words(&[messy]).unwrap();
        assert_eq!(k.expose_secret_bytes(), &[42u8; 32]);

        // Words split across a few pasted lines, some capitalised.
        let lines: Vec<String> = words.chunks(6).map(|c| {
            let mut c = c.to_vec();
            c[0] = c[0][..1].to_uppercase() + &c[0][1..];
            c.join(" ")
        }).collect();
        assert_eq!(restore_key_from_words(&lines).unwrap().expose_secret_bytes(), &[42u8; 32]);

        // A numbered list as the setup screen shows it ("1. word", "2) word").
        let numbered = words.iter().enumerate().map(|(i, w)| format!("{}. {w}", i + 1)).collect::<Vec<_>>().join("\n");
        assert_eq!(restore_key_from_words(&[numbered]).unwrap().expose_secret_bytes(), &[42u8; 32]);
        let numbered = words.iter().enumerate().map(|(i, w)| format!("{}) {w},", i + 1)).collect::<Vec<_>>().join(" ");
        assert_eq!(restore_key_from_words(&[numbered]).unwrap().expose_secret_bytes(), &[42u8; 32]);
        // The number glued to the word, as people write it by hand ("1.abandon", "2)ability").
        let glued = words.iter().enumerate()
            .map(|(i, w)| if i % 2 == 0 { format!("{}.{w}", i + 1) } else { format!("{}){w}", i + 1) })
            .collect::<Vec<_>>().join(" ");
        assert_eq!(restore_key_from_words(&[glued]).unwrap().expose_secret_bytes(), &[42u8; 32]);

        // Wrong count, unknown word, checksum failure.
        assert_eq!(restore_key_from_words(&strings(&joined)[..23]).err(), Some(RecoveryError::BadWords));
        let mut unknown = strings(&joined);
        unknown[3] = "notaword".into();
        assert_eq!(restore_key_from_words(&unknown).err(), Some(RecoveryError::BadWords));
        let mut swapped = strings(&joined);
        swapped.swap(0, 23);
        if swapped != strings(&joined) {
            // Swapping two different words almost always breaks the checksum; when it does
            // not, it gives a different key, never this one.
            match restore_key_from_words(&swapped) {
                Err(e) => assert_eq!(e, RecoveryError::BadWords),
                Ok(k) => assert_ne!(k.expose_secret_bytes(), &[42u8; 32]),
            }
        }
        assert_eq!(restore_key_from_words(&[]).err(), Some(RecoveryError::BadWords));
    }

    #[test]
    fn restore_from_file_wrong_passphrase_fails() {
        let m = begin_setup(PASS).unwrap();
        assert_eq!(
            restore_key_from_file(&m.recovery_file, "correct horse battery stapler").err(),
            Some(RecoveryError::WrongPassphraseOrDamaged)
        );
        // A short passphrase is not refused for length on restore, just wrong.
        assert_eq!(restore_key_from_file(&m.recovery_file, "x").err(), Some(RecoveryError::WrongPassphraseOrDamaged));
        assert_eq!(restore_key_from_file("hello", PASS).err(), Some(RecoveryError::Malformed));
        // A file whose sealed token was changed.
        let mut lines: Vec<String> = m.recovery_file.lines().map(str::to_owned).collect();
        let last = lines.len() - 1;
        let tampered = lines[last].replace('A', "B").replace('a', "A");
        lines[last] = if tampered == lines[last] { format!("{}x", lines[last]) } else { tampered };
        assert!(restore_key_from_file(&lines.join("\n"), PASS).is_err());
    }

    #[test]
    fn setup_material_debug_prints_no_key() {
        let key = DbKey::new([0xabu8; 32]);
        let m = setup_for_key(&key, PASS).unwrap();
        let shown = format!("{m:?} {m:#?}");
        assert!(!shown.contains("abab"), "{shown}");
        assert!(!shown.contains("171"), "{shown}");
        for w in &m.words {
            assert!(!shown.contains(w.as_str()), "word {w} in {shown}");
        }
        // The sealed token is not secret, but the debug form need not carry it either.
        assert!(!shown.contains("v1:"), "{shown}");
    }

    #[test]
    fn passphrase_length_counts_nfc_characters() {
        assert_eq!(passphrase_length(""), 0);
        assert_eq!(passphrase_length("abc"), 3);
        assert_eq!(passphrase_length("e\u{301}"), 1);
        assert_eq!(passphrase_length("\u{e9}"), 1);
        assert_eq!(MIN_PASSPHRASE_CHARS, 12);
    }
}
