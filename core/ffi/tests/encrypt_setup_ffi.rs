//! The C ABI over recovery-key setup and restore (X4, Task 10, `api::encrypt_setup`).
//! The crypto calls are tested on every build; the two that open a keyed store need
//! SQLCipher: `cargo test -p ffi --features sqlcipher --test encrypt_setup_ffi`.

use std::ffi::{c_char, CStr, CString};
use std::ptr;

use whimbrel_core::*;

const PASS: &str = "correct horse battery staple";

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn take(p: *mut c_char) -> String {
    assert!(!p.is_null());
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    whimbrel_core_free_secret_string(p);
    s
}

struct Setup {
    code: i32,
    key: [u8; 32],
    file: Option<String>,
    words: Option<String>,
}

fn begin(passphrase: &str, existing: Option<&[u8]>) -> Setup {
    let p = c(passphrase);
    let mut key = [0xeeu8; 32];
    let mut file: *mut c_char = ptr::null_mut();
    let mut words: *mut c_char = ptr::null_mut();
    let code = whimbrel_encrypt_setup_begin(
        p.as_ptr(),
        existing.map_or(ptr::null(), |k| k.as_ptr()),
        existing.map_or(0, |k| k.len()),
        key.as_mut_ptr(),
        key.len(),
        &mut file,
        &mut words,
    );
    Setup {
        code,
        key,
        file: (!file.is_null()).then(|| take(file)),
        words: (!words.is_null()).then(|| take(words)),
    }
}

fn from_file(file: &str, passphrase: &str) -> (i32, [u8; 32]) {
    let (f, p) = (c(file), c(passphrase));
    let mut key = [0xeeu8; 32];
    let code = whimbrel_encrypt_restore_from_file(f.as_ptr(), p.as_ptr(), key.as_mut_ptr(), key.len());
    (code, key)
}

fn from_words(words: &str) -> (i32, [u8; 32]) {
    let w = c(words);
    let mut key = [0xeeu8; 32];
    let code = whimbrel_encrypt_restore_from_words(w.as_ptr(), key.as_mut_ptr(), key.len());
    (code, key)
}

fn verify(file: &str, passphrase: &str) -> bool {
    let (f, p) = (c(file), c(passphrase));
    whimbrel_encrypt_setup_verify_passphrase(f.as_ptr(), p.as_ptr())
}

#[test]
fn recovery_codes_are_pinned() {
    assert_eq!(WHIMBREL_RECOVERY_OK, 0);
    assert_eq!(WHIMBREL_RECOVERY_PASSPHRASE_TOO_SHORT, 1);
    assert_eq!(WHIMBREL_RECOVERY_WRONG_PASSPHRASE_OR_DAMAGED, 2);
    assert_eq!(WHIMBREL_RECOVERY_MALFORMED, 3);
    assert_eq!(WHIMBREL_RECOVERY_BAD_WORDS, 4);
    assert_eq!(WHIMBREL_RECOVERY_INVALID_ARGUMENT, -1);
    let header = include_str!("../include/whimbrel_core.h");
    for (name, value) in [
        ("WHIMBREL_RECOVERY_OK", 0),
        ("WHIMBREL_RECOVERY_PASSPHRASE_TOO_SHORT", 1),
        ("WHIMBREL_RECOVERY_WRONG_PASSPHRASE_OR_DAMAGED", 2),
        ("WHIMBREL_RECOVERY_MALFORMED", 3),
        ("WHIMBREL_RECOVERY_BAD_WORDS", 4),
        ("WHIMBREL_RECOVERY_INVALID_ARGUMENT", -1),
    ] {
        let line = format!("#define {name} ({value})");
        assert!(header.contains(&line), "header lacks `{line}`");
    }
    for f in [
        "whimbrel_encrypt_setup_begin(",
        "whimbrel_encrypt_setup_verify_passphrase(",
        "whimbrel_encrypt_restore_from_file(",
        "whimbrel_encrypt_restore_from_words(",
        "whimbrel_encrypt_passphrase_length(",
        "whimbrel_encrypt_min_passphrase_chars(",
        "whimbrel_core_free_secret_string(",
    ] {
        assert!(header.contains(f), "header lacks {f}");
    }
}

#[test]
fn begin_setup_over_ffi_gives_a_key_file_and_words_that_agree() {
    let s = begin(PASS, None);
    assert_eq!(s.code, WHIMBREL_RECOVERY_OK);
    assert_ne!(s.key, [0xee; 32]);
    let file = s.file.unwrap();
    let words = s.words.unwrap();
    assert_eq!(words.split(' ').count(), 24);
    assert!(verify(&file, PASS));
    assert!(!verify(&file, "correct horse battery stapler"));
    assert_eq!(from_file(&file, PASS), (WHIMBREL_RECOVERY_OK, s.key));
    assert_eq!(from_words(&words), (WHIMBREL_RECOVERY_OK, s.key));
    // Pasted messily.
    let messy = format!("  {}\n", words.to_uppercase().replace(' ', "  \n "));
    assert_eq!(from_words(&messy), (WHIMBREL_RECOVERY_OK, s.key));
}

#[test]
fn begin_setup_for_an_existing_key_keeps_it() {
    let s = begin(PASS, Some(&[9u8; 32]));
    assert_eq!(s.code, WHIMBREL_RECOVERY_OK);
    assert_eq!(s.key, [9u8; 32]);
    assert_eq!(from_file(&s.file.unwrap(), PASS), (WHIMBREL_RECOVERY_OK, [9u8; 32]));
    // An existing key that is not 32 bytes is malformed: nothing is made.
    let bad = begin(PASS, Some(&[9u8; 31]));
    assert_eq!(bad.code, WHIMBREL_RECOVERY_INVALID_ARGUMENT);
    assert!(bad.file.is_none() && bad.words.is_none());
}

#[test]
fn failures_return_codes_and_zero_the_key_buffer() {
    let short = begin("elevenchars", None);
    assert_eq!(short.code, WHIMBREL_RECOVERY_PASSPHRASE_TOO_SHORT);
    assert_eq!(short.key, [0u8; 32], "out key zeroed on failure");
    assert!(short.file.is_none() && short.words.is_none());

    let s = begin(PASS, None);
    let file = s.file.unwrap();
    assert_eq!(from_file(&file, "wrong passphrase!!"), (WHIMBREL_RECOVERY_WRONG_PASSPHRASE_OR_DAMAGED, [0u8; 32]));
    assert_eq!(from_file("not a recovery file", PASS), (WHIMBREL_RECOVERY_MALFORMED, [0u8; 32]));
    assert_eq!(from_words("abandon abandon"), (WHIMBREL_RECOVERY_BAD_WORDS, [0u8; 32]));
}

#[test]
fn invalid_arguments() {
    let mut key = [0xeeu8; 32];
    let p = c(PASS);
    let mut file: *mut c_char = ptr::null_mut();
    let mut words: *mut c_char = ptr::null_mut();
    // Null passphrase, null out pointers, out buffer not 32 bytes.
    assert_eq!(
        whimbrel_encrypt_setup_begin(ptr::null(), ptr::null(), 0, key.as_mut_ptr(), 32, &mut file, &mut words),
        WHIMBREL_RECOVERY_INVALID_ARGUMENT
    );
    assert_eq!(
        whimbrel_encrypt_setup_begin(p.as_ptr(), ptr::null(), 0, key.as_mut_ptr(), 31, &mut file, &mut words),
        WHIMBREL_RECOVERY_INVALID_ARGUMENT
    );
    assert_eq!(
        whimbrel_encrypt_setup_begin(p.as_ptr(), ptr::null(), 0, ptr::null_mut(), 32, &mut file, &mut words),
        WHIMBREL_RECOVERY_INVALID_ARGUMENT
    );
    assert_eq!(
        whimbrel_encrypt_setup_begin(p.as_ptr(), ptr::null(), 0, key.as_mut_ptr(), 32, ptr::null_mut(), &mut words),
        WHIMBREL_RECOVERY_INVALID_ARGUMENT
    );
    assert!(file.is_null() && words.is_null());
    assert_eq!(whimbrel_encrypt_restore_from_file(ptr::null(), p.as_ptr(), key.as_mut_ptr(), 32), WHIMBREL_RECOVERY_INVALID_ARGUMENT);
    assert_eq!(whimbrel_encrypt_restore_from_file(p.as_ptr(), ptr::null(), key.as_mut_ptr(), 32), WHIMBREL_RECOVERY_INVALID_ARGUMENT);
    assert_eq!(whimbrel_encrypt_restore_from_words(ptr::null(), key.as_mut_ptr(), 32), WHIMBREL_RECOVERY_INVALID_ARGUMENT);
    assert_eq!(whimbrel_encrypt_restore_from_words(p.as_ptr(), key.as_mut_ptr(), 16), WHIMBREL_RECOVERY_INVALID_ARGUMENT);
    assert!(!whimbrel_encrypt_setup_verify_passphrase(ptr::null(), p.as_ptr()));
    assert!(!whimbrel_encrypt_setup_verify_passphrase(p.as_ptr(), ptr::null()));
    // Freeing null is a no-op.
    whimbrel_core_free_secret_string(ptr::null_mut());
}

#[test]
fn passphrase_length_is_nfc_characters() {
    assert_eq!(whimbrel_encrypt_min_passphrase_chars(), 12);
    assert_eq!(whimbrel_encrypt_passphrase_length(c("abc").as_ptr()), 3);
    assert_eq!(whimbrel_encrypt_passphrase_length(c("cafe\u{301}").as_ptr()), 4);
    assert_eq!(whimbrel_encrypt_passphrase_length(ptr::null()), 0);
}

#[cfg(feature = "sqlcipher")]
mod keyed {
    use super::*;
    use std::path::Path;

    fn open(path: &Path, key: &[u8; 32]) -> (i32, *mut StoreHandle) {
        let p = c(path.to_str().unwrap());
        let mut store: *mut StoreHandle = ptr::null_mut();
        let code = whimbrel_memory_open_keyed(
            p.as_ptr(), ptr::null(), WHIMBREL_KEY_FOUND, key.as_ptr(), 32, ptr::null(), &mut store, ptr::null_mut(),
        );
        (code, store)
    }

    fn make_encrypted_store(path: &Path) -> Setup {
        let s = begin(PASS, None);
        assert_eq!(s.code, WHIMBREL_RECOVERY_OK);
        let (code, store) = open(path, &s.key);
        assert_eq!(code, WHIMBREL_OPEN_OPENED);
        whimbrel_memory_close(store);
        s
    }

    #[test]
    fn recovery_from_file_restores_key_and_opens_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kestrel.sqlite");
        let s = make_encrypted_store(&db);
        // Without a key the file is "needs key", never a new install.
        let p = c(db.to_str().unwrap());
        let mut none: *mut StoreHandle = ptr::null_mut();
        let code = whimbrel_memory_open_keyed(
            p.as_ptr(), ptr::null(), WHIMBREL_KEY_ABSENT, ptr::null(), 0, ptr::null(), &mut none, ptr::null_mut(),
        );
        assert_eq!(code, WHIMBREL_OPEN_NEEDS_KEY);
        // The file and passphrase bring the key back, and it opens the store.
        let (code, key) = from_file(s.file.as_deref().unwrap(), PASS);
        assert_eq!(code, WHIMBREL_RECOVERY_OK);
        assert_eq!(key, s.key);
        let (code, store) = open(&db, &key);
        assert_eq!(code, WHIMBREL_OPEN_OPENED);
        whimbrel_memory_close(store);
        // A wrong passphrase gives no key (the buffer is zeroed).
        assert_eq!(
            from_file(s.file.as_deref().unwrap(), "not the passphrase"),
            (WHIMBREL_RECOVERY_WRONG_PASSPHRASE_OR_DAMAGED, [0u8; 32])
        );
    }

    #[test]
    fn recovery_from_words_restores_key() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kestrel.sqlite");
        let s = make_encrypted_store(&db);
        let words = s.words.unwrap();
        let (code, key) = from_words(&words.to_uppercase());
        assert_eq!(code, WHIMBREL_RECOVERY_OK);
        let (code, store) = open(&db, &key);
        assert_eq!(code, WHIMBREL_OPEN_OPENED);
        whimbrel_memory_close(store);
        // Another setup's words give another key, which this file rejects.
        let other = begin(PASS, None).words.unwrap();
        let (_, wrong) = from_words(&other);
        assert_eq!(open(&db, &wrong).0, WHIMBREL_OPEN_KEY_REJECTED);
    }
}
