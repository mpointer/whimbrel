use base64::{engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sealed::{seal, unseal, SealedError, SealedKey};

fn key() -> SealedKey {
    SealedKey::from_bytes([7u8; 32])
}

#[test]
fn standard_alphabet_token_is_malformed() {
    let k = key();
    // Find a token whose body uses '+' or '/' when re-encoded in the standard alphabet.
    // 32-byte plaintext gives a 12 + 32 + 16 = 60-byte blob (multiple of 3), so the
    // standard encoding has no '=' padding and a lenient decoder would accept it.
    for i in 0..200u8 {
        let tok = seal(&k, "v1", &["a"], &[i; 32]);
        let raw = URL_SAFE_NO_PAD.decode(tok.strip_prefix("v1:").unwrap()).unwrap();
        let std_body = STANDARD.encode(&raw);
        assert!(!std_body.ends_with('='));
        if std_body.contains('+') || std_body.contains('/') {
            let std_tok = format!("v1:{std_body}");
            assert_eq!(unseal(&k, "v1", &["a"], &std_tok), Err(SealedError::Malformed));
            return;
        }
    }
    panic!("no fixture with standard-alphabet characters found");
}

#[test]
fn padded_urlsafe_token_still_accepted() {
    let k = key();
    let tok = seal(&k, "v1", &["a"], b"hello");
    let padded = format!("{tok}==");
    assert_eq!(unseal(&k, "v1", &["a"], &padded).unwrap(), b"hello");
}

#[test]
fn more_than_two_trailing_equals_is_malformed() {
    let k = key();
    let tok = seal(&k, "v1", &["a"], b"hello");
    assert_eq!(unseal(&k, "v1", &["a"], &format!("{tok}===")), Err(SealedError::Malformed));
}

#[test]
#[should_panic]
fn seal_rejects_non_numeric_version() {
    let _ = seal(&key(), "v1.0", &["a"], b"x");
}

#[test]
#[should_panic]
fn seal_rejects_empty_version_digits() {
    let _ = seal(&key(), "v", &["a"], b"x");
}
