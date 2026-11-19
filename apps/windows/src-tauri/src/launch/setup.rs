//! The recovery setup's session (brief 3.3.1), held in Rust: the webview gets the 24 words to
//! show and nothing else. The key and the sealed recovery-file text never cross the bridge.
//! One session serves the launch's onboarding, one the Settings opt-in.

use std::path::PathBuf;

use api::encrypt_setup::{self as setup, RecoveryError, SetupMaterial};

use crate::db_key::KeyBytes;

/// Why `begin` made nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginRefusal {
    /// Below core's minimum, counted after NFC.
    PassphraseTooShort,
    /// Words already exist: Start over first.
    AlreadyMade,
    /// A key exists (or cannot be read) and the session was not given it: a fresh key is
    /// never made then.
    KeyExists,
    Failed,
}

#[derive(Default)]
pub struct SetupSession {
    /// The key the setup must wrap (`db-key` or `db-key-pending`), or `None` for a fresh key.
    existing_key: Option<KeyBytes>,
    material: Option<SetupMaterial>,
    saved_to: Option<PathBuf>,
    /// The type-it-back test passed for the current words.
    tested: bool,
    /// The Settings precheck passed (Settings scope only).
    pub opened: bool,
}

impl SetupSession {
    /// A new session over `existing_key`; anything from an earlier one is dropped.
    pub fn open(existing_key: Option<KeyBytes>) -> SetupSession {
        SetupSession { existing_key, opened: true, ..SetupSession::default() }
    }

    pub fn reuses_key(&self) -> bool {
        self.existing_key.is_some()
    }

    /// Words exist for a fresh key (its Quit asks first).
    pub fn has_fresh_material(&self) -> bool {
        self.material.is_some() && self.existing_key.is_none()
    }

    /// Makes the material. `others_exist`: `db-key` or `db-key-pending` is Found or cannot be
    /// read right now; with no existing key that refuses (never a fresh key while either exists).
    pub fn begin(&mut self, passphrase: &str, others_exist: bool) -> Result<Vec<String>, BeginRefusal> {
        if self.material.is_some() {
            return Err(BeginRefusal::AlreadyMade);
        }
        let made = match &self.existing_key {
            Some(k) => setup::setup_for_key(&k.db_key(), passphrase),
            None if others_exist => return Err(BeginRefusal::KeyExists),
            None => setup::begin_setup(passphrase),
        };
        match made {
            Ok(m) => {
                let words = m.words.clone();
                self.material = Some(m);
                self.tested = false;
                self.saved_to = None;
                Ok(words)
            }
            Err(RecoveryError::PassphraseTooShort) => Err(BeginRefusal::PassphraseTooShort),
            Err(e) => {
                tracing::warn!(error = %e, "the recovery setup failed");
                Err(BeginRefusal::Failed)
            }
        }
    }

    /// The sealed text to save (safe on disk: it needs the passphrase).
    pub fn recovery_file_text(&self) -> Option<&str> {
        self.material.as_ref().map(|m| m.recovery_file.as_str())
    }

    pub fn saved(&mut self, to: PathBuf) {
        self.saved_to = Some(to);
    }

    pub fn saved_to(&self) -> Option<&PathBuf> {
        self.saved_to.as_ref()
    }

    /// The type-it-back test, on the in-memory text (no file is read).
    pub fn verify(&mut self, passphrase: &str) -> bool {
        self.tested = self.material.as_ref().is_some_and(|m| setup::verify_passphrase(&m.recovery_file, passphrase));
        self.tested
    }

    pub fn tested(&self) -> bool {
        self.tested
    }

    /// Start over: the words, the file and the test are dropped (a reused key stays, so its
    /// words come back the same).
    pub fn start_over(&mut self) {
        self.material = None;
        self.saved_to = None;
        self.tested = false;
    }

    /// Continue: the key, once the test passed, and the material is dropped.
    pub fn take_key(&mut self) -> Option<KeyBytes> {
        if !self.tested {
            return None;
        }
        let m = self.material.take()?;
        self.tested = false;
        Some(KeyBytes::new(*m.key.expose_secret_bytes()))
    }
}

/// Whether two paths are on the same volume (`GetVolumePathNameW`), for the "same disk as
/// the database" warning. Unknown counts as the same (the warning is the safe side).
pub fn same_volume(a: &std::path::Path, b: &std::path::Path) -> bool {
    fn volume(p: &std::path::Path) -> Option<String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetVolumePathNameW;
        let wide: Vec<u16> = p.as_os_str().encode_wide().chain([0]).collect();
        let mut out = [0u16; 1024];
        unsafe { GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut out) }.ok()?;
        let len = out.iter().position(|&c| c == 0).unwrap_or(out.len());
        Some(String::from_utf16_lossy(&out[..len]).to_lowercase())
    }
    match (volume(a), volume(b)) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "correct horse battery staple";

    #[test]
    fn a_fresh_setup_needs_its_test_before_it_gives_the_key() {
        let mut s = SetupSession::open(None);
        assert_eq!(s.begin("short", false), Err(BeginRefusal::PassphraseTooShort));
        let words = s.begin(PASS, false).unwrap();
        assert_eq!(words.len(), 24);
        assert!(s.has_fresh_material());
        assert_eq!(s.begin(PASS, false), Err(BeginRefusal::AlreadyMade));
        assert!(s.take_key().is_none(), "no key before the test");
        assert!(!s.verify("wrong passphrase here"));
        assert!(s.verify(PASS));
        let key = s.take_key().unwrap();
        assert_eq!(api::encrypt_setup::restore_key_from_words(&words).unwrap().expose_secret_bytes(), key.as_bytes());
        assert!(s.recovery_file_text().is_none(), "the material is dropped");
    }

    #[test]
    fn a_fresh_key_is_refused_while_another_exists_and_a_reused_key_keeps_its_words() {
        let mut s = SetupSession::open(None);
        assert_eq!(s.begin(PASS, true), Err(BeginRefusal::KeyExists));
        let existing = KeyBytes::new([9; 32]);
        let mut s = SetupSession::open(Some(existing.clone()));
        let first = s.begin(PASS, true).unwrap();
        assert!(!s.has_fresh_material());
        s.start_over();
        let again = s.begin("another long passphrase", true).unwrap();
        assert_eq!(first, again, "the same key, so the same words");
        assert!(s.verify("another long passphrase"));
        assert_eq!(s.take_key(), Some(existing));
    }

    #[test]
    fn the_same_folder_is_the_same_volume() {
        let t = std::env::temp_dir();
        assert!(same_volume(&t, &t.join("x.txt")));
    }
}
