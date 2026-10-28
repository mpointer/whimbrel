//! Credentials in captured text are replaced locally before anything is stored, synced
//! or sent to a model. Screen captures of Terminal and Chrome were found holding live
//! API keys, tokens and passwords. Patterns favor precision: known key formats, plus
//! the value of a `key=`/`token:`-style assignment or URL parameter.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::{Regex, RegexSet};

/// (kind, pattern). A pattern with a capture group named `v` redacts only that group;
/// otherwise the whole match is redacted.
///
/// Formats with a distinctive prefix (`sk-ant-`, `ghp_`, `AIza`, `xox?-`, `eyJ`, ...) have no
/// leading `\b`: OCR and wrapped terminal text drop the space before a key, and a word
/// boundary would then miss it. `AKIA`/`ASIA` need no leading `\b` either: 20 capitals and
/// digits starting with that prefix is not an ordinary word, and a closing `\b` still rejects
/// a longer token. The short or common prefixes (generic `sk-`, `bearer`) keep `\b` so they
/// cannot fire inside ordinary words (`disk-...`, `pallbearer ...`).
const PATTERNS: &[(&str, &str)] = &[
    ("private key", r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|\z)"),
    ("Anthropic key", r"sk-ant-[A-Za-z0-9_\-]{20,}"),
    ("OpenAI key", r"(?:\bsk-|sk-(?:proj|svcacct|admin)-)[A-Za-z0-9_\-]{20,}"),
    ("GitHub token", r"(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})"),
    ("AWS key", r"(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
    ("Google key", r"AIza[0-9A-Za-z_\-]{35}"),
    ("Slack token", r"xox[abposr]-[A-Za-z0-9\-]{10,}"),
    ("Stripe key", r"[rs]k_(?:live|test)_[A-Za-z0-9]{16,}"),
    ("TypeSafe key", r"apikey_[a-z0-9_]{32,}"),
    ("JWT", r"eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}"),
    ("bearer token", r"(?i)\bbearer\s+(?P<v>[A-Za-z0-9._~+/=\-]{16,})"),
    (
        "secret",
        r#"(?i)\b[A-Z0-9_\-]*(?:api[_\-]?key|secret|token|passw(?:or)?d|pwd|access[_\-]?key)\b\s*[:=]\s*["']?(?P<v>[^\s"'&,;]{8,})"#,
    ),
    (
        "URL secret",
        r"(?i)[?&](?:token|access_token|refresh_token|id_token|api_?key|key|pwd|password|secret|sig|signature|code|auth)=(?P<v>[^&#\s]{8,})",
    ),
];

static COMPILED: LazyLock<Vec<(&'static str, Regex)>> =
    LazyLock::new(|| PATTERNS.iter().map(|(kind, p)| (*kind, Regex::new(p).unwrap())).collect());

static ANY: LazyLock<RegexSet> = LazyLock::new(|| RegexSet::new(PATTERNS.iter().map(|(_, p)| *p)).unwrap());

/// `text` with every credential replaced by `[REDACTED <kind>]`; borrowed when nothing matched.
pub fn redact_secrets(text: &str) -> Cow<'_, str> {
    if !ANY.is_match(text) {
        return Cow::Borrowed(text);
    }
    let mut out = Cow::Borrowed(text);
    for (kind, re) in COMPILED.iter() {
        if !re.is_match(&out) {
            continue;
        }
        let replaced = re
            .replace_all(&out, |caps: &regex::Captures| match caps.name("v") {
                // Already a placeholder (`token=[REDACTED secret]`): leave it, so
                // redacting twice is the same as redacting once.
                Some(v) if v.as_str().starts_with("[REDACTED") => caps[0].to_string(),
                Some(v) => {
                    let whole = caps.get(0).unwrap();
                    format!(
                        "{}[REDACTED {kind}]{}",
                        &whole.as_str()[..v.start() - whole.start()],
                        &whole.as_str()[v.end() - whole.start()..]
                    )
                }
                None => format!("[REDACTED {kind}]"),
            })
            .into_owned();
        if replaced != out {
            out = Cow::Owned(replaced);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redacted(s: &str) -> String {
        redact_secrets(s).into_owned()
    }

    #[test]
    fn known_key_formats_are_replaced() {
        assert_eq!(redacted("key sk-ant-api03-abcdefghijklmnopqrstuv1234 here"), "key [REDACTED Anthropic key] here");
        assert_eq!(redacted("sk-proj-ABCDEFGHIJKLMNOPQRSTUVWX12"), "[REDACTED OpenAI key]");
        assert_eq!(redacted("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123"), "[REDACTED GitHub token]");
        assert_eq!(redacted("AKIAIOSFODNN7EXAMPLE"), "[REDACTED AWS key]");
        assert_eq!(redacted("AIzaSyA1234567890abcdefghijklmnopqrstuv"), "[REDACTED Google key]");
        assert_eq!(redacted(&format!("apikey_{}", "a1b2_c3".repeat(10))), "[REDACTED TypeSafe key]");
        assert_eq!(redacted("eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4"), "[REDACTED JWT]");
        assert_eq!(
            redacted("-----BEGIN RSA PRIVATE KEY-----\nMIIEow\n-----END RSA PRIVATE KEY-----"),
            "[REDACTED private key]"
        );
    }

    #[test]
    fn keys_glued_to_preceding_word_characters_are_still_replaced() {
        // OCR and wrapped terminal text lose the space before a key; a `\b` anchor then
        // never fires. Distinctive prefixes don't need it.
        assert_eq!(redacted("xxxsk-ant-api03-abcdefghijklmnopqrstuv1234"), "xxx[REDACTED Anthropic key]");
        assert_eq!(redacted("keysk-proj-ABCDEFGHIJKLMNOPQRSTUVWX12"), "key[REDACTED OpenAI key]");
        assert_eq!(redacted("keysk-svcacct-ABCDEFGHIJKLMNOPQRSTUVWX12"), "key[REDACTED OpenAI key]");
        assert_eq!(redacted("tokenghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123"), "token[REDACTED GitHub token]");
        assert_eq!(redacted("idAIzaSyA1234567890abcdefghijklmnopqrstuv"), "id[REDACTED Google key]");
        assert_eq!(redacted("tokxoxb-1234567890-abcdefghij"), "tok[REDACTED Slack token]");
        assert_eq!(redacted("keysk_test_ABCDEFGHIJKLMNOP1234"), "key[REDACTED Stripe key]");
        assert_eq!(redacted(&format!("xxapikey_{}", "a1b2_c3".repeat(10))), "xx[REDACTED TypeSafe key]");
        assert_eq!(redacted("xeyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4"), "x[REDACTED JWT]");
        assert_eq!(redacted("keyAKIAIOSFODNN7EXAMPLE"), "key[REDACTED AWS key]");
        assert_eq!(redacted("idASIAIOSFODNN7EXAMPLE"), "id[REDACTED AWS key]");
    }

    #[test]
    fn the_short_generic_prefixes_keep_their_word_boundary_so_ordinary_words_are_left_alone() {
        // A deliberate trade-off (see PATTERNS): `sk-` and `bearer` are common word endings.
        assert_eq!(redacted("xxxsk-abcdefghijklmnopqrstuvwx"), "xxxsk-abcdefghijklmnopqrstuvwx");
        assert_eq!(redacted("pallbearer responsibilities"), "pallbearer responsibilities");
        // An AWS-shaped run still needs a closing boundary: a longer token is not an AWS key.
        assert_eq!(redacted("keyAKIAIOSFODNN7EXAMPLEx"), "keyAKIAIOSFODNN7EXAMPLEx");
    }

    #[test]
    fn assignment_and_url_values_are_replaced_but_names_kept() {
        assert_eq!(redacted("export OPENAI_API_KEY='abc123def456ghi'"), "export OPENAI_API_KEY='[REDACTED secret]'");
        assert_eq!(redacted("password: hunter2hunter2"), "password: [REDACTED secret]");
        assert_eq!(
            redacted("https://x.test/cb?code=4/0AbCdEfGh12345&state=1"),
            "https://x.test/cb?code=[REDACTED URL secret]&state=1"
        );
        assert_eq!(redacted("Authorization: Bearer abcdefghij0123456789"), "Authorization: Bearer [REDACTED bearer token]");
    }

    #[test]
    fn redacting_twice_is_the_same_as_once() {
        for s in [
            "export OPENAI_API_KEY='abc123def456ghi'",
            "https://x.test/cb?code=4/0AbCdEfGh12345&state=1",
            "password: hunter2hunter2 and sk-proj-ABCDEFGHIJKLMNOPQRSTUVWX12",
        ] {
            let once = redacted(s);
            assert!(matches!(redact_secrets(&once), Cow::Borrowed(_)), "{once}");
        }
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        for s in [
            "commit 9fceb02d0ae598e95dc970b74767f19372d61af8",
            "550e8400-e29b-41d4-a716-446655440000",
            "We discussed token limits and the password reset flow.",
            "max_tokens = 1500",
            "https://github.com/mpointer/kestrel/pull/208",
            // Formats that keep their `\b` anchor must not fire inside ordinary words.
            "see disk-encryption-configuration-guide for details",
            "ASIAPACIFICNETWORKSTEAM01",
            "risk_management_policy_2024_q3_review",
        ] {
            assert!(matches!(redact_secrets(s), Cow::Borrowed(_)), "{s}");
        }
    }
}
