//! The shell's own text (the tray, window titles, file dialogs), from the UI's catalog, so one
//! catalog holds every Windows string (docs/superpowers/plans/2026-10-31-windows-localization.md).
//! Keys are the English source with `{name}` placeholders, as in the UI's `t()`;
//! `translate.py extract` collects every `tr()` call whose first argument is a string literal.

use std::sync::{LazyLock, RwLock};

use api::l10n::Catalog;

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    Catalog::parse(include_str!("../../ui/src/l10n/Localizable.xcstrings")).expect("the bundled Windows catalog must parse")
});

/// The UI languages this build ships: English plus `shipped` in `l10n/languages.json`.
static SHIPPED: LazyLock<Vec<String>> = LazyLock::new(|| {
    #[derive(serde::Deserialize)]
    struct Languages {
        shipped: Vec<String>,
    }
    let shipped = serde_json::from_str::<Languages>(include_str!("../../../../l10n/languages.json"))
        .expect("the bundled languages.json must parse")
        .shipped;
    let mut all = vec!["en".to_string()];
    all.extend(shipped);
    all
});

/// "Write notes and summaries in" (the Mac's `AppLanguage.writeInOptions`).
pub const WRITE_IN_OPTIONS: [&str; 6] = ["en-US", "en-GB", "en-CA", "en-AU", "es-US", "fr-CA"];

const ENGLISH_VARIANTS: [&str; 3] = ["GB", "CA", "AU"];

static UI_LANGUAGE: RwLock<String> = RwLock::new(String::new());

/// The language the shell shows: set at startup and when the Language picker changes.
pub fn ui_language() -> String {
    let lang = UI_LANGUAGE.read().unwrap();
    if lang.is_empty() {
        "en-US".to_string()
    } else {
        lang.clone()
    }
}

/// Resolves the Language picker's `choice` (`""` is Automatic) against Windows' languages and
/// makes it the shell's language. Returns it.
pub fn apply_ui_language(choice: &str) -> String {
    let lang = resolve_ui_language(choice, &os_languages(), home_region().as_deref(), &SHIPPED);
    *UI_LANGUAGE.write().unwrap() = lang.clone();
    lang
}

/// The languages the Language picker offers besides Automatic.
pub fn shipped_languages() -> Vec<String> {
    SHIPPED.clone()
}

/// The Mac's `AppLanguage.uiTag`: the choice when set, otherwise Windows' languages in order; the
/// first whose base language this build ships wins. A tag without a region (the picker's plain
/// "English", say) takes Windows' home region, as the Mac takes the system's. English keeps a
/// variant region (`en-GB`) and is otherwise `en-US`; a Windows with no shipped language (Italian)
/// gets `en-US` too.
pub fn resolve_ui_language(choice: &str, os_languages: &[String], home_region: Option<&str>, shipped: &[String]) -> String {
    let base = |tag: &str| tag.split('-').next().unwrap_or(tag).to_ascii_lowercase();
    let candidates: Vec<&str> = if choice.is_empty() { os_languages.iter().map(String::as_str).collect() } else { vec![choice] };
    for tag in candidates {
        let lang = base(tag);
        // The region subtag: two letters (`en-GB`, `zh-Hant-TW`'s TW), else the home region.
        let region = tag.split('-').skip(1).find(|p| p.len() == 2).or(home_region).map(str::to_ascii_uppercase);
        if lang == "en" {
            return match region {
                Some(r) if ENGLISH_VARIANTS.contains(&r.as_str()) => format!("en-{r}"),
                _ => "en-US".to_string(),
            };
        }
        if shipped.iter().any(|s| base(s) == lang) {
            return match region {
                Some(r) => format!("{lang}-{r}"),
                None => lang,
            };
        }
    }
    "en-US".to_string()
}

/// The Mac's `AppLanguage.outputTag`: the write-in choice when set, else the UI language; `""`
/// for `en-US` with the English-only model, which keeps every prompt byte-identical.
pub fn output_tag(ui: &str, write_in: &str, multilingual: bool) -> String {
    let tag = if write_in.is_empty() { ui } else { write_in };
    if tag == "en-US" && !multilingual {
        String::new()
    } else {
        tag.to_string()
    }
}

/// The tag notes, summaries and drafts are written in, from this PC's settings: the write-in
/// choice and "I have meetings in other languages".
pub fn output_language(write_in: &str, multilingual: bool) -> String {
    output_tag(&ui_language(), write_in, multilingual)
}

/// Windows' display languages, in the user's order (Settings → Time & language).
fn os_languages() -> Vec<String> {
    use windows::System::UserProfile::GlobalizationPreferences;
    GlobalizationPreferences::Languages().map(|langs| langs.into_iter().map(|t| t.to_string()).collect()).unwrap_or_default()
}

/// Windows' "Country or region" (`GB`), for a language picked without one.
fn home_region() -> Option<String> {
    use windows::System::UserProfile::GlobalizationPreferences;
    GlobalizationPreferences::HomeGeographicRegion().ok().map(|r| r.to_string()).filter(|r| r.len() == 2)
}

/// `key` in the shell's language, with its `{name}` placeholders filled from `params`. A `count`
/// param chooses the plural form, as in the UI's `t()`: the key is the English "other" form
/// (`"{count} models"`), and its "one" form lives in the catalog.
pub fn tr(key: &str, params: &[(&str, &str)]) -> String {
    tr_in(&CATALOG, &ui_language(), key, params)
}

/// The exact tag, then the base language (both in `Catalog::lookup`), then English, then the key
/// itself, which is the English source: never empty.
fn tr_in(catalog: &Catalog, lang: &str, key: &str, params: &[(&str, &str)]) -> String {
    let count = params.iter().find(|(name, _)| *name == "count").and_then(|(_, n)| n.parse::<i64>().ok());
    let text = catalog.lookup(lang, key, count).or_else(|| catalog.lookup("en", key, count)).unwrap_or(key);
    api::l10n::interpolate(text, params)
}

/// A clock time ("2:05 PM", "14:05") as `lang`'s region writes it, in this PC's time zone.
pub fn short_time(unix_ms: i64, lang: &str) -> String {
    short_time_in(unix_ms, lang, None).unwrap_or_else(|| {
        use chrono::TimeZone;
        chrono::Local.timestamp_millis_opt(unix_ms).single().map(|t| t.format("%-I:%M %p").to_string()).unwrap_or_default()
    })
}

/// Windows' own short-time format for `lang` (whether it's 12- or 24-hour, and its AM/PM words),
/// so no table of ours has to know. `time_zone` is for tests; `None` is this PC's zone.
fn short_time_in(unix_ms: i64, lang: &str, time_zone: Option<&str>) -> Option<String> {
    use windows::core::HSTRING;
    use windows_collections::IIterable;
    use windows::Foundation::DateTime;
    use windows::Globalization::DateTimeFormatting::DateTimeFormatter;
    // WinRT time: 100-nanosecond ticks since 1601-01-01 UTC.
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    let at = DateTime { UniversalTime: unix_ms.checked_mul(10_000)?.checked_add(UNIX_EPOCH_TICKS)? };
    let languages = IIterable::<HSTRING>::from(vec![HSTRING::from(lang)]);
    let formatter = DateTimeFormatter::CreateDateTimeFormatterLanguages(&HSTRING::from("shorttime"), &languages).ok()?;
    let text = match time_zone {
        Some(zone) => formatter.FormatUsingTimeZone(at, &HSTRING::from(zone)),
        None => formatter.Format(at),
    }
    .ok()?;
    // The formatter brackets fields with direction marks, which would show as boxes in some fonts.
    Some(text.to_string().chars().filter(|c| !matches!(c, '\u{200e}' | '\u{200f}')).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG_JSON: &str = r#"{"sourceLanguage":"en","strings":{
        "Quit Whimbrel":{"localizations":{"fr":{"stringUnit":{"state":"translated","value":"Quitter Whimbrel"}}}},
        "Synced {time}":{"localizations":{"es":{"stringUnit":{"state":"translated","value":"Sincronizado {time}"}}}},
        "{count} models":{"localizations":{"en":{"variations":{"plural":{
            "one":{"stringUnit":{"state":"translated","value":"{count} model"}},
            "other":{"stringUnit":{"state":"translated","value":"{count} models"}}}}}}}
    }}"#;

    #[test]
    fn tr_falls_back_to_english() {
        let c = Catalog::parse(CATALOG_JSON).unwrap();
        assert_eq!(tr_in(&c, "fr-CA", "Quit Whimbrel", &[]), "Quitter Whimbrel");
        assert_eq!(tr_in(&c, "de", "Quit Whimbrel", &[]), "Quit Whimbrel");
        assert_eq!(tr_in(&c, "fr", "Not in the catalog", &[]), "Not in the catalog");
        assert_eq!(tr_in(&c, "es-US", "Synced {time}", &[("time", "14:05")]), "Sincronizado 14:05");
        assert_eq!(tr_in(&c, "en", "Synced {time}", &[("time", "2:05 PM")]), "Synced 2:05 PM");
    }

    #[test]
    fn a_count_chooses_the_plural_form() {
        let c = Catalog::parse(CATALOG_JSON).unwrap();
        let key = "{count} models";
        assert_eq!(tr_in(&c, "en-US", key, &[("count", "1")]), "1 model");
        assert_eq!(tr_in(&c, "en-US", key, &[("count", "2")]), "2 models");
        // No Spanish yet: the English plural, not the key.
        assert_eq!(tr_in(&c, "es", key, &[("count", "1")]), "1 model");
    }

    #[test]
    fn the_bundled_catalog_parses() {
        // Not a literal: translate.py extract would collect a literal key from a test into the catalog.
        let key = "Open Whimbrel";
        assert_eq!(tr(key, &[]), "Open Whimbrel");
    }

    // 2026-10-30 14:05 UTC.
    const AT: i64 = 1_793_369_100_000;

    #[test]
    fn tray_time_en_gb_is_24h() {
        assert_eq!(short_time_in(AT, "en-GB", Some("UTC")).as_deref(), Some("14:05"));
    }

    #[test]
    fn tray_time_en_us_is_12h() {
        let us = short_time_in(AT, "en-US", Some("UTC")).unwrap();
        assert!(us.starts_with("2:05") && us.ends_with("PM"), "{us:?}");
    }

    fn langs(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn ui_language_follows_windows_then_falls_back_to_en_us() {
        let shipped = langs(&["en", "en-GB", "es"]);
        let resolve = |os: &[&str], region| resolve_ui_language("", &langs(os), region, &shipped);
        assert_eq!(resolve(&["en-GB", "fr-FR"], Some("GB")), "en-GB");
        assert_eq!(resolve(&["en-IN"], Some("IN")), "en-US", "not an English variant this build ships");
        assert_eq!(resolve(&["it-IT", "es-MX"], Some("IT")), "es-MX", "the first shipped language wins");
        assert_eq!(resolve(&["it-IT"], Some("IT")), "en-US", "no shipped language");
        assert_eq!(resolve(&["en"], Some("AU")), "en-AU", "a bare tag takes the home region");
        assert_eq!(resolve(&[], None), "en-US");
    }

    #[test]
    fn an_explicit_choice_wins_over_windows() {
        let shipped = langs(&["en", "en-GB", "es"]);
        assert_eq!(resolve_ui_language("en-AU", &langs(&["en-GB"]), Some("GB"), &shipped), "en-AU");
        assert_eq!(resolve_ui_language("en", &langs(&["es-MX"]), Some("GB"), &shipped), "en-GB");
        assert_eq!(resolve_ui_language("es", &langs(&["en-US"]), Some("US"), &shipped), "es-US");
    }

    #[test]
    fn output_tag_en_us_english_model_is_empty() {
        assert_eq!(output_tag("en-US", "", false), "");
        assert_eq!(output_tag("en-US", "en-US", false), "", "an explicit en-US keeps prompts byte-identical too");
        assert_eq!(output_tag("en-US", "", true), "en-US");
        assert_eq!(output_tag("en-GB", "", false), "en-GB");
    }

    #[test]
    fn write_in_override_wins() {
        assert_eq!(output_tag("en-US", "es-US", false), "es-US");
        assert_eq!(output_tag("fr-CA", "en-GB", false), "en-GB");
    }

    #[test]
    fn the_shipped_languages_start_with_english() {
        assert_eq!(shipped_languages().first().map(String::as_str), Some("en"));
    }
}
