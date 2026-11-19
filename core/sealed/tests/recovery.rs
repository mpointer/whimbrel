use sealed::recovery::{
    key_to_words, make_recovery_file, open_recovery_file, words_to_key, RecoveryError,
    MIN_PASSPHRASE_CHARS,
};

const KEY: [u8; 32] = [0x42; 32];
const PASS: &str = "correct horse battery";

#[test]
fn recovery_file_round_trips_with_passphrase() {
    let file = make_recovery_file(&KEY, PASS).unwrap();
    let lines: Vec<&str> = file.lines().collect();
    assert_eq!(lines[0], "whimbrel-recovery 1");
    assert_eq!(lines[1], "kdf argon2id m=65536 t=3 p=1");
    assert!(lines[2].starts_with("salt "));
    assert!(lines[3].starts_with("v1:"));
    assert_eq!(open_recovery_file(&file, PASS).unwrap(), KEY);
}

#[test]
fn short_passphrase_is_refused() {
    assert_eq!(MIN_PASSPHRASE_CHARS, 12);
    assert_eq!(
        make_recovery_file(&KEY, "elevenchars").unwrap_err(),
        RecoveryError::PassphraseTooShort
    );
    assert!(make_recovery_file(&KEY, "twelve chars").is_ok());
    // Characters, not bytes: 11 multi-byte chars are still too short.
    assert_eq!(
        make_recovery_file(&KEY, "ééééééééééé").unwrap_err(),
        RecoveryError::PassphraseTooShort
    );
}

#[test]
fn wrong_passphrase_returns_wrong_passphrase_or_damaged() {
    let file = make_recovery_file(&KEY, PASS).unwrap();
    assert_eq!(
        open_recovery_file(&file, "incorrect horse battery").unwrap_err(),
        RecoveryError::WrongPassphraseOrDamaged
    );
}

#[test]
fn tampered_token_returns_wrong_passphrase_or_damaged() {
    let file = make_recovery_file(&KEY, PASS).unwrap();
    let mut bytes = file.into_bytes();
    let n = bytes.len();
    // Flip a character inside the token body (last char may be a newline).
    let i = bytes.iter().rposition(|b| b.is_ascii_alphanumeric()).unwrap();
    assert!(i < n);
    bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
    let file = String::from_utf8(bytes).unwrap();
    assert_eq!(
        open_recovery_file(&file, PASS).unwrap_err(),
        RecoveryError::WrongPassphraseOrDamaged
    );
}

#[test]
fn edited_header_params_cannot_weaken_below_floor() {
    let file = make_recovery_file(&KEY, PASS).unwrap();
    for weak in ["m=8 t=3 p=1", "m=65536 t=1 p=1", "m=65536 t=3 p=0", "m=19455 t=3 p=1"] {
        let hostile = file.replace("m=65536 t=3 p=1", weak);
        assert_eq!(
            open_recovery_file(&hostile, PASS).unwrap_err(),
            RecoveryError::Malformed,
            "{weak}"
        );
    }
    // The floor itself is allowed (and then fails authentication, not the floor check).
    let at_floor = file.replace("m=65536 t=3 p=1", "m=19456 t=2 p=1");
    assert_eq!(
        open_recovery_file(&at_floor, PASS).unwrap_err(),
        RecoveryError::WrongPassphraseOrDamaged
    );
}

#[test]
fn hostile_huge_params_are_refused() {
    let file = make_recovery_file(&KEY, PASS).unwrap();
    let hostile = file.replace("m=65536 t=3 p=1", "m=4194304 t=3 p=1");
    assert_eq!(open_recovery_file(&hostile, PASS).unwrap_err(), RecoveryError::Malformed);
}

#[test]
fn unknown_format_or_garbage_is_malformed() {
    let file = make_recovery_file(&KEY, PASS).unwrap();
    let v2 = file.replacen("whimbrel-recovery 1", "whimbrel-recovery 2", 1);
    assert_eq!(open_recovery_file(&v2, PASS).unwrap_err(), RecoveryError::Malformed);
    assert_eq!(open_recovery_file("", PASS).unwrap_err(), RecoveryError::Malformed);
    assert_eq!(open_recovery_file("hello\nworld\n", PASS).unwrap_err(), RecoveryError::Malformed);
}

#[test]
fn words_round_trip_and_are_24() {
    let words = key_to_words(&KEY);
    assert_eq!(words.len(), 24);
    assert_eq!(words_to_key(&words).unwrap(), KEY);
    // BIP-39 vector: 32 zero bytes.
    let zero = key_to_words(&[0u8; 32]);
    assert!(zero[..23].iter().all(|w| w == "abandon"));
    assert_eq!(zero[23], "art");
}

#[test]
fn bad_checksum_words_fail() {
    let mut words = key_to_words(&KEY);
    // Swapping the last word for a different one breaks the checksum or the key.
    words[23] = if words[23] == "abandon" { "ability".into() } else { "abandon".into() };
    assert_eq!(words_to_key(&words).unwrap_err(), RecoveryError::BadWords);
    assert_eq!(words_to_key(&words[..23]).unwrap_err(), RecoveryError::BadWords);
    let mut unknown = key_to_words(&KEY);
    unknown[0] = "notaword".into();
    assert_eq!(words_to_key(&unknown).unwrap_err(), RecoveryError::BadWords);
}

#[test]
fn same_key_two_files_have_different_salt_and_token() {
    let a = make_recovery_file(&KEY, PASS).unwrap();
    let b = make_recovery_file(&KEY, PASS).unwrap();
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    assert_ne!(la[2], lb[2], "salt");
    assert_ne!(la[3], lb[3], "token");
}

#[test]
fn passphrase_is_nfc_normalised_both_ways() {
    let nfc = "caf\u{e9} horse battery";
    let nfd = "cafe\u{301} horse battery";
    assert_ne!(nfc, nfd);
    let f1 = make_recovery_file(&KEY, nfc).unwrap();
    assert_eq!(open_recovery_file(&f1, nfd).unwrap(), KEY);
    let f2 = make_recovery_file(&KEY, nfd).unwrap();
    assert_eq!(open_recovery_file(&f2, nfc).unwrap(), KEY);
}

#[test]
fn minimum_length_counts_normalised_chars() {
    // 11 x (e + combining acute) is 22 code points but 11 NFC characters.
    let eleven = "e\u{301}".repeat(11);
    assert_eq!(make_recovery_file(&KEY, &eleven).unwrap_err(), RecoveryError::PassphraseTooShort);
    assert!(make_recovery_file(&KEY, &"e\u{301}".repeat(12)).is_ok());
}

#[test]
fn ceilings_and_floors_on_header_params() {
    let file = make_recovery_file(&KEY, PASS).unwrap();
    for bad in ["m=262145 t=3 p=1", "m=65536 t=17 p=1", "m=65536 t=3 p=17"] {
        let hostile = file.replace("m=65536 t=3 p=1", bad);
        assert_eq!(open_recovery_file(&hostile, PASS).unwrap_err(), RecoveryError::Malformed, "{bad}");
    }
    // The ceiling itself passes the header check (and then fails authentication).
    for ok in ["m=262144 t=3 p=1", "m=65536 t=16 p=16"] {
        let edge = file.replace("m=65536 t=3 p=1", ok);
        assert_eq!(
            open_recovery_file(&edge, PASS).unwrap_err(),
            RecoveryError::WrongPassphraseOrDamaged,
            "{ok}"
        );
    }
}

#[test]
fn published_bip39_vectors() {
    // Trezor/python-mnemonic test vectors (24 words, mixed bytes).
    let cases = [
        (
            "68a79eaca2324873eacc50cb9c6eca8cc68ea5d936f98787c60c7ebc74e6ce7c",
            "hamster diagram private dutch cause delay private meat slide toddler razor book happy fancy gospel tennis maple dilemma loan word shrug inflict delay length",
        ),
        (
            "9f6a2878b2520799a44ef18bc7df394e7061a224d2c33cd015b157d746869863",
            "panda eyebrow bullet gorilla call smoke muffin taste mesh discover soft ostrich alcohol speed nation flash devote level hobby quick inner drive ghost inside",
        ),
    ];
    for (hex, phrase) in cases {
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
        }
        let words: Vec<String> = phrase.split(' ').map(String::from).collect();
        assert_eq!(key_to_words(&key), words);
        assert_eq!(words_to_key(&words).unwrap(), key);
    }
}

#[test]
fn file_then_words_round_trip_with_distinct_bytes() {
    let mut key = [0u8; 32];
    for (i, b) in key.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37).wrapping_add(11);
    }
    let mut seen = key.to_vec();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 32, "all bytes differ");
    let file = make_recovery_file(&key, PASS).unwrap();
    let opened = open_recovery_file(&file, PASS).unwrap();
    assert_eq!(words_to_key(&key_to_words(&opened)).unwrap(), key);
}

#[test]
fn words_accept_extra_whitespace_and_capitals() {
    let words = key_to_words(&KEY);
    let messy: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, w)| if i % 2 == 0 { format!("  {}\n", w.to_uppercase()) } else { format!("\t{w} ") })
        .collect();
    assert_eq!(words_to_key(&messy).unwrap(), KEY);
}
