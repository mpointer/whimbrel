//! Localized core strings, looked up in a String Catalog (`.xcstrings`).
//!
//! Keys are the English source text. A missing language, key or value falls back
//! to the key itself, so callers always get English, never an empty string.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

/// The English source of every string core passes to `text`/`text_plural`.
pub mod keys {
    // Persisted: rendered once, in the run's output language, and stored as written.
    pub const NO_ACTIVITY_TODAY: &str = "No activity captured today.";
    pub const BRIEFLY_VIEWED_APP: &str = "Briefly viewed {app}.";
    pub const BRIEFLY_VIEWED_TITLE_IN_APP: &str = "Briefly viewed \"{title}\" in {app}.";
    pub const NO_EVENTS_TODAY: &str = "No events on today's calendar.";
    pub const TODAY_EVENTS: &str = "Today: {titles}.";
    pub const NOTHING_TO_RANK: &str =
        "Nothing to rank yet. Turn on Rank email by reply priority in AI & Routing settings.";
    pub const THREADS_NEED_REPLY: &str =
        "{count} threads look like they need a reply, most likely first: {names}.";
    pub const NO_SUBJECT: &str = "(no subject)";
    pub const AWAY_TODO: &str = "Ask {organizer} for the recording of {title}";
    pub const AWAY_THE_ORGANIZER: &str = "the organizer";
    pub const AWAY_GREETING_FALLBACK: &str = "there";
    pub const AWAY_EMAIL_SUBJECT: &str = "Recording of {title}";
    pub const AWAY_EMAIL_BODY: &str = "Hi {first_name},\n\nI couldn't join {title} on {day}. Could you share the recording or transcript when you have a moment?\n\nThanks,\n{signature}";
    /// A chrono strftime pattern, not prose: translations reorder the fields ("%A, %-d de %B").
    pub const DAY_LONG: &str = "%A, %B %-d";
    pub const AWAY_RUN_SUMMARY: &str = "Email {to} asking for the recording of {title}";
    pub const ACTION_UNDECODABLE: &str = "Couldn't read the drafted action.";
    pub const CALENDAR_ADDED: &str = "Added \"{title}\" to your calendar.";
    pub const CALENDAR_REJECTED: &str = "Google rejected the calendar event request.";
    pub const EMAIL_SENT: &str = "Sent \"{subject}\" to {to}.";
    pub const EMAIL_REJECTED: &str = "Google rejected the email send request.";
    // UI, not stored: the Ask starter chips, by id (`api::prompts::starter_prompts`).
    pub const STARTER_SLIPPING_TITLE: &str = "What's quietly slipping?";
    pub const STARTER_SLIPPING_QUERY: &str = "What's quietly slipping? Look across my recent meetings and captures for things that keep getting mentioned but never resolved.";
    pub const STARTER_RECURRING_TITLE: &str = "Which recurring meetings can I kill?";
    pub const STARTER_RECURRING_QUERY: &str = "Which recurring meetings can I kill? Look for meetings that keep happening but don't seem to produce decisions or action items.";
    pub const STARTER_TODAY_TITLE: &str = "Summarize today";
    pub const STARTER_TODAY_QUERY: &str =
        "Summarize what happened today across my meetings and captures.";
    pub const STARTER_ACTIONS_TITLE: &str = "What action items are still open?";
    pub const STARTER_ACTIONS_QUERY: &str =
        "What action items from my recent meetings are still open?";
    pub const STARTER_TREND_TITLE: &str = "Any trend this week?";
    pub const STARTER_TREND_QUERY: &str =
        "Is there a trend across my meetings this week — recurring topics, blockers, or themes?";
    pub const STARTER_DECISIONS_TITLE: &str = "What decisions did I make recently?";
    pub const STARTER_DECISIONS_QUERY: &str =
        "Summarize the decisions I've made in my recent meetings.";

    // Transient: core renders these in English only; the shells localize them by code (Task 4).
    pub const CALENDAR_NOT_CONNECTED: &str = "Calendar isn't connected, or needs to be reconnected. Check Settings → Connectors, then try Approve again.";
    pub const GMAIL_NOT_CONNECTED: &str = "Gmail isn't connected, or needs to be reconnected. Check Settings → Connectors, then try Approve again.";
    pub const ACTION_RETRYABLE: &str = "Couldn't complete this action just now ({reason}). It's still pending -- try Approve again in a bit.";
    pub const KEY_REFUSED: &str = "{provider} refused this key. Check it on {provider}'s key page.";
    pub const KEY_UNREACHABLE: &str =
        "Couldn't reach {provider}. The key is saved; models will load when you're online.";
    pub const DEVICE_REVOKED: &str =
        "This device's access was revoked. Sign in again from Settings to request access.";
    pub const DEVICE_NOT_APPROVED: &str = "This device isn't approved on this account yet.";
    pub const APPROVE_ON_OTHER_DEVICE_UNNAMED: &str =
        "Approve this computer on your other device; it was active in the last week.";
    pub const APPROVE_ON_OTHER_DEVICE: &str =
        "Approve this computer on {devices}; it was active in the last week.";
    pub const RESET_REQUIRES_COMPUTER: &str = "Starting fresh has to be done from a computer.";
    pub const ACCOUNT_NOT_ALLOWED: &str =
        "This Google account isn't allowed to sign in to Whimbrel.";
    pub const SIGNIN_EXPIRED: &str = "Your Google sign-in expired. Try signing in again.";
    pub const DEVICE_LIMIT: &str =
        "Your free sync plan includes one computer and one phone, and they're already in use.";
    pub const RATE_LIMITED: &str = "Too many sign-in attempts. Try again in a few minutes.";
    pub const ACCOUNT_CHANGING: &str =
        "Your sync account is being changed. Try again in a few minutes.";
    pub const SWAP_KIND_MISMATCH: &str = "That device can't take the place of this one.";
    pub const WANTS_REQUIRED: &str = "Pick at least one option to join the waitlist.";
    pub const RESET_CONFIRM_REQUIRED: &str = "Confirm that you want to start fresh to continue.";
    pub const DEVICE_NOT_FOUND: &str = "That device is no longer on this account.";
    pub const SERVER_ERROR: &str = "Sync is temporarily unavailable (server error {status}).";
    pub const NETWORK: &str =
        "Couldn't reach the sync server. Check your connection and try again.";
    pub const MALFORMED_RESPONSE: &str =
        "The sync server returned an unexpected response. Try again.";
}

/// Every key core passes to `text`/`text_plural`; each must exist in the bundled catalog.
pub const KEYS: &[&str] = {
    use keys::*;
    &[
        NO_ACTIVITY_TODAY,
        BRIEFLY_VIEWED_APP,
        BRIEFLY_VIEWED_TITLE_IN_APP,
        NO_EVENTS_TODAY,
        TODAY_EVENTS,
        NOTHING_TO_RANK,
        THREADS_NEED_REPLY,
        NO_SUBJECT,
        AWAY_TODO,
        AWAY_THE_ORGANIZER,
        AWAY_GREETING_FALLBACK,
        AWAY_EMAIL_SUBJECT,
        AWAY_EMAIL_BODY,
        DAY_LONG,
        AWAY_RUN_SUMMARY,
        ACTION_UNDECODABLE,
        CALENDAR_ADDED,
        CALENDAR_REJECTED,
        EMAIL_SENT,
        EMAIL_REJECTED,
        STARTER_SLIPPING_TITLE,
        STARTER_SLIPPING_QUERY,
        STARTER_RECURRING_TITLE,
        STARTER_RECURRING_QUERY,
        STARTER_TODAY_TITLE,
        STARTER_TODAY_QUERY,
        STARTER_ACTIONS_TITLE,
        STARTER_ACTIONS_QUERY,
        STARTER_TREND_TITLE,
        STARTER_TREND_QUERY,
        STARTER_DECISIONS_TITLE,
        STARTER_DECISIONS_QUERY,
        CALENDAR_NOT_CONNECTED,
        GMAIL_NOT_CONNECTED,
        ACTION_RETRYABLE,
        KEY_REFUSED,
        KEY_UNREACHABLE,
        DEVICE_REVOKED,
        DEVICE_NOT_APPROVED,
        APPROVE_ON_OTHER_DEVICE_UNNAMED,
        APPROVE_ON_OTHER_DEVICE,
        RESET_REQUIRES_COMPUTER,
        ACCOUNT_NOT_ALLOWED,
        SIGNIN_EXPIRED,
        DEVICE_LIMIT,
        RATE_LIMITED,
        ACCOUNT_CHANGING,
        SWAP_KIND_MISMATCH,
        WANTS_REQUIRED,
        RESET_CONFIRM_REQUIRED,
        DEVICE_NOT_FOUND,
        SERVER_ERROR,
        NETWORK,
        MALFORMED_RESPONSE,
    ]
};

static BUNDLED: LazyLock<Catalog> = LazyLock::new(|| {
    Catalog::parse(include_str!("../l10n/Localizable.xcstrings"))
        .expect("bundled Localizable.xcstrings must parse")
});

#[derive(Deserialize)]
pub struct Catalog {
    strings: HashMap<String, Entry>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Entry {
    localizations: HashMap<String, Localization>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Localization {
    string_unit: Option<StringUnit>,
    variations: Option<Variations>,
}

#[derive(Deserialize)]
struct Variations {
    #[serde(default)]
    plural: HashMap<String, PluralCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluralCase {
    string_unit: StringUnit,
}

#[derive(Deserialize)]
struct StringUnit {
    value: String,
}

impl Catalog {
    pub fn parse(json: &str) -> Result<Catalog, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    /// Exact tag, then base language, then `None`. With `count`, the plural
    /// variation for that count's category wins over a plain string unit.
    pub fn lookup(&self, lang: &str, key: &str, count: Option<i64>) -> Option<&str> {
        let locs = &self.strings.get(key)?.localizations;
        let base = base_language(lang);
        [lang, base].into_iter().find_map(|tag| {
            let loc = locs.get(tag)?;
            let plural = count.and_then(|n| {
                let cases = &loc.variations.as_ref()?.plural;
                Some(
                    cases
                        .get(plural_category(lang, n))?
                        .string_unit
                        .value
                        .as_str(),
                )
            });
            plural
                .or(loc.string_unit.as_ref().map(|u| u.value.as_str()))
                .filter(|v| !v.is_empty())
        })
    }
}

fn base_language(lang: &str) -> &str {
    lang.split(['-', '_']).next().unwrap_or(lang)
}

pub fn text(lang: &str, key: &str, params: &[(&str, &str)]) -> String {
    interpolate(BUNDLED.lookup(lang, key, None).unwrap_or(key), params)
}

/// `text` for a key with no placeholders, borrowed from the bundled catalog.
pub fn text_static(lang: &str, key: &'static str) -> &'static str {
    BUNDLED.lookup(lang, key, None).unwrap_or(key)
}

pub fn text_plural(lang: &str, key: &str, count: i64, params: &[(&str, &str)]) -> String {
    plural_in(&BUNDLED, lang, key, count, params)
}

fn plural_in(c: &Catalog, lang: &str, key: &str, count: i64, params: &[(&str, &str)]) -> String {
    let n = count.to_string();
    let mut all = vec![("count", n.as_str())];
    all.extend_from_slice(params);
    interpolate(c.lookup(lang, key, Some(count)).unwrap_or(key), &all)
}

/// One left-to-right pass: each `{name}` found in `params` is replaced, everything
/// else (unknown placeholders, stray braces, and the substituted values) is copied as-is.
pub fn interpolate(template: &str, params: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let name = &after[..close];
            let (_, v) = params.iter().find(|(k, _)| *k == name)?;
            Some((*v, close))
        });
        match value {
            Some((v, close)) => {
                out.push_str(v);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn plural_category(lang: &str, n: i64) -> &'static str {
    // ponytail: hand-coded CLDR one/other for our 5 base languages; use icu_plurals if a language with few/many is added
    match base_language(lang) {
        "fr" | "pt" => {
            if n == 0 || n == 1 {
                "one"
            } else {
                "other"
            }
        }
        _ => {
            if n == 1 {
                "one"
            } else {
                "other"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_values_are_not_resubstituted() {
        assert_eq!(
            text(
                "",
                "{a} met {b} {unknown}",
                &[("a", "Sync {b}"), ("b", "Ana {a}")]
            ),
            "Sync {b} met Ana {a} {unknown}"
        );
        assert_eq!(
            text_plural("", "{title}: {count} notes", 2, &[("title", "{count}")]),
            "{count}: 2 notes"
        );
        assert_eq!(text("", "a {b", &[("b", "x")]), "a {b");
    }

    #[test]
    fn text_empty_lang_returns_english_interpolated() {
        assert_eq!(
            text("", "Briefly viewed {app}.", &[("app", "Safari")]),
            "Briefly viewed Safari."
        );
    }

    #[test]
    fn lookup_falls_back_exact_then_base_then_none() {
        let c = Catalog::parse(r#"{"sourceLanguage":"en","strings":{"Hi":{"localizations":{"es":{"stringUnit":{"state":"needs_review","value":"Hola"}}}}}}"#).unwrap();
        assert_eq!(c.lookup("es-MX", "Hi", None), Some("Hola"));
        assert_eq!(c.lookup("fr", "Hi", None), None);
    }

    #[test]
    fn missing_key_returns_english_not_empty() {
        assert_eq!(
            text("es", "Not in catalog {x}", &[("x", "1")]),
            "Not in catalog 1"
        );
    }

    #[test]
    fn plural_category_fr_zero_is_one() {
        assert_eq!(plural_category("fr-CA", 0), "one");
        assert_eq!(plural_category("en", 0), "other");
    }

    #[test]
    fn plural_category_table() {
        for (l, n, want) in [
            ("en", 1, "one"),
            ("es", 1, "one"),
            ("es", 0, "other"),
            ("de", 1, "one"),
            ("pt", 0, "one"),
            ("pt", 1, "one"),
            ("fr", 2, "other"),
        ] {
            assert_eq!(plural_category(l, n), want, "{l} {n}");
        }
    }

    #[test]
    fn every_key_used_in_core_exists_in_catalog() {
        let strings = &BUNDLED.strings; // forces the bundled catalog to parse
        for key in KEYS {
            assert!(strings.contains_key(*key), "missing from catalog: {key}");
        }
    }

    #[test]
    fn shipped_languages_render_every_key() {
        for lang in ["es", "fr", "fr-CA", "de", "pt"] {
            for key in KEYS {
                assert!(
                    BUNDLED.lookup(lang, key, Some(2)).is_some(),
                    "{lang} missing: {key}"
                );
            }
        }
    }

    #[test]
    fn text_plural_picks_category_and_interpolates_count() {
        let c = Catalog::parse(r#"{"sourceLanguage":"en","strings":{"{count} meetings":{"localizations":{"fr":{"variations":{"plural":{"one":{"stringUnit":{"state":"translated","value":"{count} réunion"}},"other":{"stringUnit":{"state":"translated","value":"{count} réunions"}}}}}}}}}"#).unwrap();
        assert_eq!(
            plural_in(&c, "fr-CA", "{count} meetings", 0, &[]),
            "0 réunion"
        );
        assert_eq!(
            plural_in(&c, "fr", "{count} meetings", 3, &[]),
            "3 réunions"
        );
        assert_eq!(plural_in(&c, "", "{count} meetings", 1, &[]), "1 meetings");
    }
}
