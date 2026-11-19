use base64::{engine::general_purpose::STANDARD, Engine};
use sealed::{seal, unseal, SealedError, SealedKey};
use sha2::{Digest, Sha256};

const VECTORS: &str = include_str!("vectors.json");

fn vectors() -> serde_json::Value {
    serde_json::from_str(VECTORS).unwrap()
}

fn key_from(v: &serde_json::Value) -> SealedKey {
    let raw = STANDARD.decode(v["key"].as_str().unwrap()).unwrap();
    SealedKey::from_bytes(raw.try_into().unwrap())
}

fn test_key() -> SealedKey {
    SealedKey::from_bytes([7u8; 32])
}

#[test]
fn shared_vectors_unseal_to_expected_plaintext() {
    let v = vectors();
    let key = key_from(&v);
    let expected = v["plaintext"].as_str().unwrap().as_bytes();
    let tokens = v["tokens"].as_object().unwrap();
    assert!(!tokens.is_empty());
    for (name, tok) in tokens {
        let out = unseal(&key, "v1", &[], tok.as_str().unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(out, expected, "{name}");
    }
    let ctx: Vec<&str> = v["contextTokens"]["context"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    for (name, tok) in v["contextTokens"]["tokens"].as_object().unwrap() {
        let tok = tok.as_str().unwrap();
        let out = unseal(&key, "v1", &ctx, tok).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(out, expected, "{name}");
        assert_eq!(
            unseal(&key, "v1", &[], tok),
            Err(SealedError::Authentication),
            "{name} must fail without context"
        );
    }
}

#[test]
fn seal_then_unseal_round_trips() {
    let k = test_key();
    let t = seal(&k, "v1", &["users", "email"], b"hello");
    assert!(t.starts_with("v1:"));
    assert!(!t.contains('='));
    assert_eq!(unseal(&k, "v1", &["users", "email"], &t).unwrap(), b"hello");
    // nonce is random: two seals differ
    assert_ne!(t, seal(&k, "v1", &["users", "email"], b"hello"));
}

#[test]
fn wrong_key_fails_authentication() {
    let t = seal(&test_key(), "v1", &[], b"x");
    let other = SealedKey::from_bytes([8u8; 32]);
    assert_eq!(unseal(&other, "v1", &[], &t), Err(SealedError::Authentication));
}

#[test]
fn wrong_context_fails_authentication() {
    let k = test_key();
    let t = seal(&k, "v1", &["users", "email"], b"x");
    assert_eq!(unseal(&k, "v1", &["users", "name"], &t), Err(SealedError::Authentication));
    assert_eq!(unseal(&k, "v1", &["users"], &t), Err(SealedError::Authentication));
    assert_eq!(unseal(&k, "v1", &[], &t), Err(SealedError::Authentication));
}

#[test]
fn tampered_version_prefix_fails_authentication() {
    let k = test_key();
    let t = seal(&k, "v1", &[], b"x");
    // The token still says v1 so it parses, but the caller-supplied version differs.
    assert_eq!(unseal(&k, "v2", &[], &t), Err(SealedError::Authentication));
    // Re-labelled token: prefix edited to v2, caller expects v1 or v2.
    let relabelled = t.replacen("v1:", "v2:", 1);
    assert_eq!(unseal(&k, "v1", &[], &relabelled), Err(SealedError::Authentication));
    assert_eq!(unseal(&k, "v2", &[], &relabelled), Err(SealedError::Authentication));
}

#[test]
fn sealed_empty_plaintext_round_trips() {
    let k = test_key();
    let t = seal(&k, "v1", &[], b"");
    assert_eq!(unseal(&k, "v1", &[], &t).unwrap(), Vec::<u8>::new());
}

#[test]
fn token_without_v1_prefix_is_malformed() {
    let k = test_key();
    let t = seal(&k, "v1", &[], b"x");
    let body = t.strip_prefix("v1:").unwrap();
    assert_eq!(unseal(&k, "v1", &[], body), Err(SealedError::Malformed));
    assert_eq!(unseal(&k, "v1", &[], &format!("x1:{body}")), Err(SealedError::Malformed));
    assert_eq!(unseal(&k, "v1", &[], "v1:!!!"), Err(SealedError::Malformed));
    // shorter than nonce + tag
    assert_eq!(unseal(&k, "v1", &[], "v1:AAAA"), Err(SealedError::Malformed));
}

#[test]
fn key_debug_does_not_print_bytes() {
    let s = format!("{:?}", SealedKey::from_bytes([0xAB; 32]));
    assert!(!s.contains("171") && !s.to_lowercase().contains("ab"), "{s}");
}

#[test]
fn vectors_file_matches_recorded_source_hash() {
    let md = include_str!("../VECTORS.md");
    let line = md
        .lines()
        .find_map(|l| l.strip_prefix("SHA-256: "))
        .expect("VECTORS.md must contain a `SHA-256: <hex>` line");
    let actual: String = Sha256::digest(VECTORS.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(line.trim(), actual);
}
