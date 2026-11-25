//! Credentials in captured text are replaced locally before anything is stored, synced
//! or sent to a model. Screen captures of Terminal and Chrome were found holding live
//! API keys, tokens and passwords. Patterns favor precision: known key formats, plus
//! the value of a `key=`/`token:`-style assignment or URL parameter.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::{Regex, RegexSet};

/// The query parameter names the "URL secret" pattern treats as credentials, as a regex
/// alternation. One list, used by that pattern and by [`strip_url_secrets`].
macro_rules! url_secret_params {
    () => {
        "token|access_token|refresh_token|id_token|api_?key|key|pwd|password|secret|sig|signature|code|auth"
    };
}

/// [`url_secret_params!`] as a constant.
pub const URL_SECRET_PARAMS: &str = url_secret_params!();

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
    (
        "private key",
        r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|\z)",
    ),
    ("Anthropic key", r"sk-ant-[A-Za-z0-9_\-]{20,}"),
    (
        "OpenAI key",
        r"(?:\bsk-|sk-(?:proj|svcacct|admin)-)[A-Za-z0-9_\-]{20,}",
    ),
    (
        "GitHub token",
        r"(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})",
    ),
    ("AWS key", r"(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
    ("Google key", r"AIza[0-9A-Za-z_\-]{35}"),
    ("Slack token", r"xox[abposr]-[A-Za-z0-9\-]{10,}"),
    ("Stripe key", r"[rs]k_(?:live|test)_[A-Za-z0-9]{16,}"),
    ("TypeSafe key", r"apikey_[a-z0-9_]{32,}"),
    (
        "JWT",
        r"eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
    ),
    (
        "bearer token",
        r"(?i)\bbearer\s+(?P<v>[A-Za-z0-9._~+/=\-]{16,})",
    ),
    (
        "secret",
        r#"(?i)\b[A-Z0-9_\-]*(?:api[_\-]?key|secret|token|passw(?:or)?d|pwd|access[_\-]?key)\b\s*[:=]\s*["']?(?P<v>[^\s"'&,;]{8,})"#,
    ),
    (
        "URL secret",
        concat!(r"(?i)[?&](?:", url_secret_params!(), r")=(?P<v>[^&#\s]{8,})"),
    ),
];

static COMPILED: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    PATTERNS
        .iter()
        .map(|(kind, p)| (*kind, Regex::new(p).unwrap()))
        .collect()
});

static ANY: LazyLock<RegexSet> =
    LazyLock::new(|| RegexSet::new(PATTERNS.iter().map(|(_, p)| *p)).unwrap());

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

static URL_SECRET_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(concat!("(?i)^(?:", url_secret_params!(), ")$")).unwrap());

/// Query parameter names that carry a credential whatever their value's length (security review
/// M6): any `*signature` or `*sig` (Azure SAS, CloudFront, GCS `X-Goog-Signature`), OneDrive and
/// Dropbox share keys, and the GCS and S3 credential and token parameters.
static URL_CREDENTIAL_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        "(?i)^(?:.*signature|.*sig|authkey|resourcekey|rlkey|x-goog-credential|x-goog-security-token|x-amz-credential|x-amz-security-token)$",
    )
    .unwrap()
});

/// `url` without the `user:password@` of its authority, if it has one.
fn without_userinfo(url: &str) -> Cow<'_, str> {
    let Some((scheme, rest)) = url.split_once("://") else { return Cow::Borrowed(url) };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    match rest[..end].rfind('@') {
        Some(at) => Cow::Owned(format!("{scheme}://{}", &rest[at + 1..])),
        None => Cow::Borrowed(url),
    }
}

/// Minimum length of a URL parameter's value for it to count as a credential (as in the
/// "URL secret" pattern).
const URL_SECRET_MIN_CHARS: usize = 8;

/// A link with its credentials taken out, so it can be stored as a working link rather than one
/// holding `[REDACTED ...]` (review M3). In three steps:
///
/// 0. Userinfo (`user:password@`) is removed from the authority (security review M6).
/// 1. Each `name=value` query pair whose name is a [`URL_SECRET_PARAMS`] name and whose value is
///    at least 8 characters, or a credential name of any length (`*signature`, `*sig`, `authkey`,
///    `resourcekey`, `rlkey`, the GCS and S3 credential and token names), is removed; the other pairs keep their order, and a `?` left with
///    nothing after it is dropped.
/// 2. If [`redact_secrets`] would still change what is left (a JWT, an AWS key, a `client_secret=`
///    name), the whole query and fragment are dropped.
/// 3. If it would change even that (a key in the path), `None`: there is no safe link to keep.
///
/// Every `Some` is a fixed point of [`redact_secrets`], so the stored-secrets sweep never touches
/// it and a repeat of the same item hashes the same. Borrowed when nothing was removed.
pub fn strip_url_secrets(url: &str) -> Option<Cow<'_, str>> {
    let clean = |s: &str| redact_secrets(s) == s;
    let credential = |pair: &&str| {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        (URL_SECRET_NAME.is_match(name) && value.chars().count() >= URL_SECRET_MIN_CHARS) || URL_CREDENTIAL_NAME.is_match(name)
    };
    let stripped = without_userinfo(url);
    let has_credential = stripped.split('#').next().and_then(|b| b.split_once('?')).is_some_and(|(_, q)| q.split('&').any(|p| credential(&p)));
    if !has_credential && clean(&stripped) {
        return Some(match stripped {
            Cow::Borrowed(_) => Cow::Borrowed(url),
            Cow::Owned(o) => Cow::Owned(o),
        });
    }
    let url: &str = &stripped;
    let (before_fragment, fragment) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let (base, query) = match before_fragment.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (before_fragment, None),
    };
    // Step 1.
    let mut out = base.to_string();
    if let Some(query) = query {
        let kept: Vec<&str> = query.split('&').filter(|pair| !credential(pair)).collect();
        let kept = kept.join("&");
        if !kept.is_empty() {
            out.push('?');
            out.push_str(&kept);
        }
    }
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    if clean(&out) {
        return Some(Cow::Owned(out));
    }
    // Steps 2 and 3.
    clean(base).then(|| Cow::Owned(base.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redacted(s: &str) -> String {
        redact_secrets(s).into_owned()
    }

    #[test]
    fn known_key_formats_are_replaced() {
        assert_eq!(
            redacted("key sk-ant-api03-abcdefghijklmnopqrstuv1234 here"),
            "key [REDACTED Anthropic key] here"
        );
        assert_eq!(
            redacted("sk-proj-ABCDEFGHIJKLMNOPQRSTUVWX12"),
            "[REDACTED OpenAI key]"
        );
        assert_eq!(
            redacted("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123"),
            "[REDACTED GitHub token]"
        );
        assert_eq!(redacted("AKIAIOSFODNN7EXAMPLE"), "[REDACTED AWS key]");
        assert_eq!(
            redacted("AIzaSyA1234567890abcdefghijklmnopqrstuv"),
            "[REDACTED Google key]"
        );
        assert_eq!(
            redacted(&format!("apikey_{}", "a1b2_c3".repeat(10))),
            "[REDACTED TypeSafe key]"
        );
        assert_eq!(
            redacted("eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4"),
            "[REDACTED JWT]"
        );
        assert_eq!(
            redacted("-----BEGIN RSA PRIVATE KEY-----\nMIIEow\n-----END RSA PRIVATE KEY-----"),
            "[REDACTED private key]"
        );
    }

    #[test]
    fn keys_glued_to_preceding_word_characters_are_still_replaced() {
        // OCR and wrapped terminal text lose the space before a key; a `\b` anchor then
        // never fires. Distinctive prefixes don't need it.
        assert_eq!(
            redacted("xxxsk-ant-api03-abcdefghijklmnopqrstuv1234"),
            "xxx[REDACTED Anthropic key]"
        );
        assert_eq!(
            redacted("keysk-proj-ABCDEFGHIJKLMNOPQRSTUVWX12"),
            "key[REDACTED OpenAI key]"
        );
        assert_eq!(
            redacted("keysk-svcacct-ABCDEFGHIJKLMNOPQRSTUVWX12"),
            "key[REDACTED OpenAI key]"
        );
        assert_eq!(
            redacted("tokenghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123"),
            "token[REDACTED GitHub token]"
        );
        assert_eq!(
            redacted("idAIzaSyA1234567890abcdefghijklmnopqrstuv"),
            "id[REDACTED Google key]"
        );
        assert_eq!(
            redacted("tokxoxb-1234567890-abcdefghij"),
            "tok[REDACTED Slack token]"
        );
        assert_eq!(
            redacted("keysk_test_ABCDEFGHIJKLMNOP1234"),
            "key[REDACTED Stripe key]"
        );
        assert_eq!(
            redacted(&format!("xxapikey_{}", "a1b2_c3".repeat(10))),
            "xx[REDACTED TypeSafe key]"
        );
        assert_eq!(
            redacted("xeyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4"),
            "x[REDACTED JWT]"
        );
        assert_eq!(redacted("keyAKIAIOSFODNN7EXAMPLE"), "key[REDACTED AWS key]");
        assert_eq!(redacted("idASIAIOSFODNN7EXAMPLE"), "id[REDACTED AWS key]");
    }

    #[test]
    fn the_short_generic_prefixes_keep_their_word_boundary_so_ordinary_words_are_left_alone() {
        // A deliberate trade-off (see PATTERNS): `sk-` and `bearer` are common word endings.
        assert_eq!(
            redacted("xxxsk-abcdefghijklmnopqrstuvwx"),
            "xxxsk-abcdefghijklmnopqrstuvwx"
        );
        assert_eq!(
            redacted("pallbearer responsibilities"),
            "pallbearer responsibilities"
        );
        // An AWS-shaped run still needs a closing boundary: a longer token is not an AWS key.
        assert_eq!(
            redacted("keyAKIAIOSFODNN7EXAMPLEx"),
            "keyAKIAIOSFODNN7EXAMPLEx"
        );
    }

    #[test]
    fn assignment_and_url_values_are_replaced_but_names_kept() {
        assert_eq!(
            redacted("export OPENAI_API_KEY='abc123def456ghi'"),
            "export OPENAI_API_KEY='[REDACTED secret]'"
        );
        assert_eq!(
            redacted("password: hunter2hunter2"),
            "password: [REDACTED secret]"
        );
        assert_eq!(
            redacted("https://x.test/cb?code=4/0AbCdEfGh12345&state=1"),
            "https://x.test/cb?code=[REDACTED URL secret]&state=1"
        );
        assert_eq!(
            redacted("Authorization: Bearer abcdefghij0123456789"),
            "Authorization: Bearer [REDACTED bearer token]"
        );
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

    #[test]
    fn strip_url_secrets_removes_the_credential_and_keeps_a_link() {
        let cases: &[(&str, Option<&str>)] = &[
            ("https://x.sharepoint.com/f?a=1&sig=abcdefgh12&b=2", Some("https://x.sharepoint.com/f?a=1&b=2")),
            ("https://x/f?sig=abcdefgh12", Some("https://x/f")),
            ("https://x/f?code=short", Some("https://x/f?code=short")),
            ("https://x/f?key=abcdefgh12#frag", Some("https://x/f#frag")),
            ("https://x/f?SIG=abcdefgh12&a=1", Some("https://x/f?a=1")),
            (
                "https://x.sharepoint.com/download.aspx?tempauth=eyJ0eXAiOiJKV1Qi.eyJhdWQiOiIwMDAw.SflKxwRJSMeKKF2QT4",
                Some("https://x.sharepoint.com/download.aspx"),
            ),
            ("https://b.s3.amazonaws.com/f?X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F2026", Some("https://b.s3.amazonaws.com/f")),
            ("https://x/f?client_secret=abcdefgh12", Some("https://x/f")),
            ("https://x/f?api-key=abcdefgh12", Some("https://x/f")),
            ("https://x/f#access_token=abcdefgh12", Some("https://x/f")),
            ("https://x/AKIAIOSFODNN7EXAMPLE/f", None),
            ("https://outlook.office.com/mail/1", Some("https://outlook.office.com/mail/1")),
            ("https://x/f?", Some("https://x/f?")),
        ];
        for (input, want) in cases {
            let got = strip_url_secrets(input);
            assert_eq!(got.as_deref(), *want, "{input}");
            if let Some(out) = got {
                assert_eq!(redact_secrets(&out), out, "{input}: what is kept is a fixed point of redact_secrets");
            }
        }
        assert!(matches!(strip_url_secrets("https://outlook.office.com/mail/1"), Some(Cow::Borrowed(_))));
    }

    #[test]
    fn strip_url_secrets_never_panics_on_odd_input() {
        for odd in ["", "?", "#", "&&&", "?sig=abcdefgh12", "https://x/f?&&sig=abcdefgh12&&", "https://\u{e9}/f?\u{e9}=1&sig=\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}"] {
            if let Some(out) = strip_url_secrets(odd) {
                assert_eq!(redact_secrets(&out), out, "{odd}");
            }
        }
    }

    /// Security review M6: Google Cloud Storage V4 signatures, OneDrive share keys, any `*signature`
    /// or `*sig` parameter, and userinfo are removed too.
    #[test]
    fn strip_url_secrets_removes_more_credential_shapes() {
        let cases: &[(&str, &str)] = &[
            (
                "https://storage.googleapis.com/b/o?X-Goog-Algorithm=GOOG4-RSA-SHA256&X-Goog-Credential=svc%40p.iam.gserviceaccount.com%2F20261122&X-Goog-Date=20261122T000000Z&X-Goog-Signature=0123456789abcdef",
                "https://storage.googleapis.com/b/o?X-Goog-Algorithm=GOOG4-RSA-SHA256&X-Goog-Date=20261122T000000Z",
            ),
            ("https://onedrive.live.com/redir?resid=ABC!123&authkey=!AbCd", "https://onedrive.live.com/redir?resid=ABC!123"),
            ("https://1drv.ms/f?resourcekey=0-abc&x=1", "https://1drv.ms/f?x=1"),
            ("https://www.dropbox.com/scl/fi/a/b.pdf?rlkey=abc12&dl=0", "https://www.dropbox.com/scl/fi/a/b.pdf?dl=0"),
            ("https://cdn.example/f?Expires=1&Signature=abc&Key-Pair-Id=K", "https://cdn.example/f?Expires=1&Key-Pair-Id=K"),
            ("https://x/f?x-sig=abc", "https://x/f"),
            ("https://x/f?sig=abc", "https://x/f"),
            ("https://user:hunter2@host.example/f?a=1", "https://host.example/f?a=1"),
            ("https://user@host.example/f", "https://host.example/f"),
            ("https://host.example/a@b/f", "https://host.example/a@b/f"),
            ("https://x/f?design=1&page=2", "https://x/f?design=1&page=2"),
        ];
        for (input, want) in cases {
            let got = strip_url_secrets(input);
            assert_eq!(got.as_deref(), Some(*want), "{input}");
            assert_eq!(redact_secrets(want), *want);
        }
    }
}

