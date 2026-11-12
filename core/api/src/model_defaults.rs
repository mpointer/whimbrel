//! Hosted model defaults that can't go stale (plan: 2026-11-11-model-defaults.md).
//!
//! Each hosted provider has a default model, named in `defaults.json` on the updates
//! server and read daily, with [`Defaults::builtin`] as the fallback when the file has
//! never been fetched (or is missing or invalid). Every chain builder and shell reads
//! defaults from here; no app keeps its own model-id literal.
//!
//! The file, schema version 1:
//!
//! ```json
//! {"version":1,"providers":{
//!   "google":{"default":"gemini-flash-latest","family":"flash","previous":["gemini-2.5-flash"]},
//!   "anthropic":{"default":"claude-haiku-5-5","family":"haiku","previous":["claude-haiku-4-5"]},
//!   "openai":{"default":"gpt-5.4-mini","family":"mini","previous":["gpt-4o-mini"]}}}
//! ```
//!
//! `previous` (optional) lists earlier defaults: the shells saved the default with every
//! routing change, so a stored copy of one is read as "never chosen" and follows the
//! current default. Unknown providers and fields are ignored; a file with another
//! `version`, or with no usable provider, is ignored as a whole.
//!
//! Two rules from Mike (2026-11-11): never override a model the user chose, and never
//! move to a pricier family silently. A chosen model the provider no longer has is kept
//! as the choice; each call retries once with the default ([`fallback_for`]), and the
//! substitution is recorded ([`note_answer`]) so Settings can say so.

use std::collections::BTreeMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// Where the file lives. The Mac session publishes it; until then it 404s, which keeps
/// the built-in table.
pub const DEFAULTS_URL: &str = "https://updates.whimbrelapp.com/models/defaults.json";

/// The schema version this build reads.
pub const SCHEMA_VERSION: u64 = 1;

/// How often a shell should call [`refresh`]: daily.
pub const REFRESH_EVERY_MS: i64 = 24 * 60 * 60 * 1000;

const META_FILE: &str = "model_defaults:file";
const META_FETCHED_AT: &str = "model_defaults:fetched_at_ms";
const UNAVAILABLE_PREFIX: &str = "model_unavailable:";
/// A defaults file is a few hundred bytes; anything this large is not one.
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// The providers that have a default. OpenRouter has none (the user picks from a catalogue
/// that changes weekly), Ollama's comes from what is installed, Whimbrel AI's from its server.
const PROVIDERS_WITH_DEFAULTS: [llm::Provider; 3] = [llm::Provider::Google, llm::Provider::Anthropic, llm::Provider::OpenAI];

/// One provider's default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDefault {
    /// The model id calls use, preferring the provider's moving alias where one exists.
    pub default: String,
    /// The price family the default belongs to (`flash`, `haiku`, `mini`): a fallback never
    /// moves a chosen model into a pricier family than its own.
    pub family: String,
    /// Earlier defaults; a stored copy of one counts as never chosen.
    #[serde(default)]
    pub previous: Vec<String>,
}

/// Where the current defaults came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Builtin,
    File,
}

/// The defaults in force: the built-in table with the fetched file's entries over it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Defaults {
    /// Keyed by `llm::Provider::as_str()`.
    pub providers: BTreeMap<String, ProviderDefault>,
    pub source: Source,
    /// When the file in use was fetched; `None` for the built-in table.
    pub fetched_at_ms: Option<i64>,
}

impl Defaults {
    /// Core's own table, checked live 2026-11-11 (Anthropic lists `claude-haiku-5-5`, OpenAI
    /// lists `gpt-5.4-mini`, Google's alias answers). `previous` holds the ids the apps
    /// shipped as defaults before, which installs saved with every routing change.
    pub fn builtin() -> Self {
        let entry = |default: &str, family: &str, previous: &[&str]| ProviderDefault {
            default: default.to_string(),
            family: family.to_string(),
            previous: previous.iter().map(|p| p.to_string()).collect(),
        };
        let providers = BTreeMap::from([
            ("google".to_string(), entry(llm::GOOGLE_FALLBACK_MODEL, "flash", &["gemini-2.5-flash"])),
            ("anthropic".to_string(), entry("claude-haiku-5-5", "haiku", &["claude-haiku-4-5"])),
            ("openai".to_string(), entry("gpt-5.4-mini", "mini", &["gpt-4o-mini"])),
        ]);
        Defaults { providers, source: Source::Builtin, fetched_at_ms: None }
    }

    pub fn get(&self, provider: llm::Provider) -> Option<&ProviderDefault> {
        self.providers.get(provider.as_str())
    }

    /// `provider`'s default model, or `""` for a provider without one.
    pub fn model(&self, provider: llm::Provider) -> &str {
        self.get(provider).map(|d| d.default.as_str()).unwrap_or("")
    }

    /// The built-in table with `file`'s entries laid over it. An entry's `previous` keeps the
    /// built-in ones too, and the built-in default when the file names another: this build
    /// saved it, so a stored copy is no choice either.
    fn with_file(file: BTreeMap<String, ProviderDefault>, fetched_at_ms: Option<i64>) -> Self {
        let mut merged = Self::builtin();
        for (provider, mut entry) in file {
            if let Some(builtin) = merged.providers.get(&provider) {
                let mut previous: Vec<String> = builtin.previous.clone();
                if builtin.default != entry.default {
                    previous.push(builtin.default.clone());
                }
                previous.append(&mut entry.previous);
                previous.retain(|p| *p != entry.default);
                previous.sort();
                previous.dedup();
                entry.previous = previous;
            }
            merged.providers.insert(provider, entry);
        }
        merged.source = Source::File;
        merged.fetched_at_ms = fetched_at_ms;
        merged
    }
}

/// The usable entries of a defaults file, or `None` when the file is unusable as a whole:
/// not JSON, another `version`, or no known provider with a non-empty `default` and `family`.
pub fn parse(json: &str) -> Option<BTreeMap<String, ProviderDefault>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    if value.get("version")?.as_u64()? != SCHEMA_VERSION {
        return None;
    }
    let providers = value.get("providers")?.as_object()?;
    let entries: BTreeMap<String, ProviderDefault> = PROVIDERS_WITH_DEFAULTS
        .iter()
        .filter_map(|p| {
            let raw = providers.get(p.as_str())?;
            let text = |key: &str| raw.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
            let previous = raw
                .get("previous")
                .and_then(|v| v.as_array())
                .map(|list| list.iter().filter_map(|m| m.as_str()).map(str::trim).filter(|m| !m.is_empty()).map(String::from).collect())
                .unwrap_or_default();
            Some((p.as_str().to_string(), ProviderDefault { default: text("default")?, family: text("family")?, previous }))
        })
        .collect();
    (!entries.is_empty()).then_some(entries)
}

/// The defaults stored in `store`: the cached file over the built-in table, or the table
/// alone when nothing usable is cached. Reads only; see [`load`].
pub fn current(store: &memory::Store) -> Defaults {
    let file = store.get_meta(META_FILE).ok().flatten().and_then(|raw| parse(&raw));
    match file {
        Some(file) => {
            let fetched_at = store.get_meta(META_FETCHED_AT).ok().flatten().and_then(|v| v.parse().ok());
            Defaults::with_file(file, fetched_at)
        }
        None => Defaults::builtin(),
    }
}

static SNAPSHOT: RwLock<Option<Defaults>> = RwLock::new(None);

/// The defaults the chain builders use. They take no store (`inference` is store-free by
/// design), so the store's defaults are copied here by [`load`] (run when a shell opens
/// its store, `crate::store::open`) and by [`refresh`]. Before either, the built-in table.
pub fn cached() -> Defaults {
    SNAPSHOT.read().ok().and_then(|s| s.clone()).unwrap_or_else(Defaults::builtin)
}

fn install(defaults: &Defaults) {
    if let Ok(mut slot) = SNAPSHOT.write() {
        *slot = Some(defaults.clone());
    }
}

/// [`current`], also made the process-wide defaults [`cached`] answers with.
pub fn load(store: &memory::Store) -> Defaults {
    let defaults = current(store);
    install(&defaults);
    defaults
}

/// GETs a defaults file from `url`: short timeouts, 200 only, a size cap, and it must
/// [`parse`]. Never touches a store, so a shell can run it off its store lanes.
pub fn fetch(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build();
    let response = agent.get(url).call().map_err(|e| match e {
        ureq::Error::Status(status, _) => format!("HTTP {status}"),
        ureq::Error::Transport(t) => t.to_string(),
    })?;
    let mut body = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(response.into_reader(), MAX_FILE_BYTES), &mut body)
        .map_err(|e| e.to_string())?;
    parse(&body).ok_or_else(|| "not a usable defaults file".to_string())?;
    Ok(body)
}

/// Stores a fetched file (already checked by [`fetch`]) and when it was fetched, and
/// returns the defaults the store now holds. An unusable file changes nothing. Call
/// [`load`] after it to put them in force for the chain builders.
pub fn store_file(store: &memory::Store, raw: &str, now_unix_ms: i64) -> rusqlite::Result<Defaults> {
    if parse(raw).is_some() {
        store.set_meta(META_FILE, raw)?;
        store.set_meta(META_FETCHED_AT, &now_unix_ms.to_string())?;
    }
    Ok(current(store))
}

/// Fetches the file now and, when it is usable, caches it. `false` when the fetch failed
/// (offline, the file not published yet, an invalid file): the cached file or the built-in
/// table stays in force. Holds `store` across a network call; a shell with store lanes
/// runs [`fetch`] itself and hands the result to [`store_file`].
pub fn refresh(store: &memory::Store) -> bool {
    match fetch(DEFAULTS_URL) {
        Ok(raw) => {
            let stored = store_file(store, &raw, crate::util::now_unix_ms()).is_ok();
            load(store);
            stored
        }
        Err(_) => false,
    }
}

/// Whether `model` counts as never chosen for `provider`: empty, the current default, or
/// an earlier one (saved by an old routing save, not picked).
pub fn is_unchosen(provider: llm::Provider, model: &str, defaults: &Defaults) -> bool {
    let model = model.trim();
    match defaults.get(provider) {
        Some(d) => model.is_empty() || model == d.default || d.previous.iter().any(|p| p == model),
        None => model.is_empty(),
    }
}

/// The model to use for `provider` given what is stored: the stored model when the user
/// chose it, else the current default. For a provider without a default, the stored
/// model (or `""`).
pub fn resolve(provider: llm::Provider, stored: Option<&str>, defaults: &Defaults) -> String {
    let stored = stored.map(str::trim).unwrap_or("");
    match defaults.get(provider) {
        Some(d) if is_unchosen(provider, stored, defaults) => d.default.clone(),
        _ => stored.to_string(),
    }
}

/// Ids of a model that is lighter (and cheaper) than any family default.
const LIGHTER_THAN_A_FAMILY: [&str; 2] = ["lite", "nano"];

/// The model a call for `chosen` retries with once if `provider` says `chosen` doesn't
/// exist: the provider's default, unless that is `chosen` itself or would move a chosen
/// model into a pricier family. A model in the default's family (its id names the family)
/// or priced at least as high (`llm::price_rank`; a model the table doesn't know is priced
/// high) may fall back; a lighter one (`-lite`, `-nano`, or priced lower) never does.
pub fn fallback_for(provider: llm::Provider, chosen: &str, defaults: &Defaults) -> Option<String> {
    let d = defaults.get(provider)?;
    if chosen == d.default {
        return None;
    }
    if is_unchosen(provider, chosen, defaults) {
        return Some(d.default.clone());
    }
    let lower = chosen.to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split(['-', '/', ':', '.', '_']).collect();
    if tokens.iter().any(|t| LIGHTER_THAN_A_FAMILY.contains(t)) {
        return None;
    }
    let family = d.family.to_ascii_lowercase();
    let same_family = tokens.iter().any(|t| *t == family);
    let not_cheaper = llm::price_rank(provider, &d.default) <= llm::price_rank(provider, chosen);
    (same_family || not_cheaper).then(|| d.default.clone())
}

/// A chosen model the provider said it doesn't have, and what answered instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUnavailable {
    /// `llm::Provider::as_str()`.
    pub provider: String,
    /// The model the user chose, kept as their choice.
    pub model: String,
    /// The default that answered for it.
    pub using: String,
    /// When this was first seen.
    pub since_ms: i64,
}

/// Records how one call went: `answered` is the model that stood in for `requested`
/// (`llm::LlmClient::substituted_model`), `None` when `requested` answered itself. A
/// substitution of a chosen model is recorded per provider (the first sighting's time
/// kept); the requested model answering clears its record.
pub fn note_answer(store: &memory::Store, provider: llm::Provider, requested: &str, answered: Option<&str>, now_unix_ms: i64) {
    let key = format!("{UNAVAILABLE_PREFIX}{}", provider.as_str());
    let existing: Option<ModelUnavailable> = store.get_meta(&key).ok().flatten().and_then(|raw| serde_json::from_str(&raw).ok());
    // A model the user never chose (an old default the shell passed on) is not theirs to warn about.
    let chosen = !is_unchosen(provider, requested, &cached());
    match answered.filter(|a| *a != requested && chosen) {
        Some(using) => {
            let since_ms = existing.as_ref().filter(|e| e.model == requested).map(|e| e.since_ms).unwrap_or(now_unix_ms);
            let record = ModelUnavailable { provider: provider.as_str().to_string(), model: requested.to_string(), using: using.to_string(), since_ms };
            if existing.as_ref() != Some(&record) {
                if let Ok(raw) = serde_json::to_string(&record) {
                    let _ = store.set_meta(&key, &raw);
                }
            }
        }
        None => {
            if existing.is_some_and(|e| e.model == requested) {
                let _ = store.delete_meta(&key);
            }
        }
    }
}

/// Every recorded unavailable model, by provider.
pub fn unavailable(store: &memory::Store) -> Vec<ModelUnavailable> {
    store
        .list_meta_with_prefix(UNAVAILABLE_PREFIX)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(_, raw)| serde_json::from_str(&raw).ok())
        .collect()
}

/// Drops `provider`'s record, e.g. when the user picks another model.
pub fn clear_unavailable(store: &memory::Store, provider: llm::Provider) {
    let _ = store.delete_meta(&format!("{UNAVAILABLE_PREFIX}{}", provider.as_str()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::Provider;

    const FILE: &str = r#"{"version":1,"providers":{
        "google":{"default":"gemini-next-flash","family":"flash","previous":["gemini-1.5-flash"]},
        "openai":{"default":"","family":"mini"},
        "mistral":{"default":"m","family":"small"},
        "anthropic":{"default":"claude-haiku-6","family":"haiku","extra":true}},
        "comment":"ignored"}"#;

    #[test]
    fn the_built_in_table_names_mikes_defaults() {
        let d = Defaults::builtin();
        assert_eq!(d.model(Provider::Google), "gemini-flash-latest");
        assert_eq!(d.model(Provider::Anthropic), "claude-haiku-5-5");
        assert_eq!(d.model(Provider::OpenAI), "gpt-5.4-mini");
        assert_eq!(d.model(Provider::OpenRouter), "", "OpenRouter has no default");
        assert_eq!((d.source, d.fetched_at_ms), (Source::Builtin, None));
        assert_eq!(
            serde_json::to_value(&d).unwrap()["providers"]["anthropic"],
            serde_json::json!({"default":"claude-haiku-5-5","family":"haiku","previous":["claude-haiku-4-5"]})
        );
    }

    #[test]
    fn parse_keeps_known_complete_entries_and_rejects_unusable_files() {
        let entries = parse(FILE).unwrap();
        assert_eq!(entries.keys().collect::<Vec<_>>(), ["anthropic", "google"], "unknown providers and an empty default dropped");
        assert_eq!(entries["google"].previous, ["gemini-1.5-flash"]);
        for bad in [
            "",
            "not json",
            "[]",
            r#"{"providers":{"google":{"default":"g","family":"flash"}}}"#,
            r#"{"version":2,"providers":{"google":{"default":"g","family":"flash"}}}"#,
            r#"{"version":1,"providers":{"google":{"default":"g"}}}"#,
            r#"{"version":1,"providers":{"mistral":{"default":"m","family":"small"}}}"#,
            r#"{"version":1}"#,
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_cached_file_overrides_the_table_and_an_invalid_one_changes_nothing() {
        let store = memory::Store::open_in_memory().unwrap();
        assert_eq!(current(&store), Defaults::builtin());

        let d = store_file(&store, FILE, 1_000).unwrap();
        assert_eq!((d.source, d.fetched_at_ms), (Source::File, Some(1_000)));
        assert_eq!(d.model(Provider::Google), "gemini-next-flash");
        assert_eq!(d.model(Provider::Anthropic), "claude-haiku-6");
        assert_eq!(d.model(Provider::OpenAI), "gpt-5.4-mini", "an unusable entry keeps the table's");
        assert_eq!(
            d.get(Provider::Google).unwrap().previous,
            ["gemini-1.5-flash", "gemini-2.5-flash", "gemini-flash-latest"],
            "the built-in default this build saved counts as an earlier one"
        );
        assert_eq!(current(&store), d);

        let after = store_file(&store, r#"{"version":9}"#, 2_000).unwrap();
        assert_eq!(after, d, "an unusable file leaves the cached one in force");
        store.set_meta(META_FILE, "garbage").unwrap();
        assert_eq!(current(&store), Defaults::builtin(), "a corrupt cache falls back to the table");
    }

    #[test]
    fn an_old_default_or_nothing_follows_the_current_default_and_a_choice_is_kept() {
        let d = Defaults::builtin();
        for stored in [None, Some(""), Some("  "), Some("gemini-2.5-flash"), Some("gemini-flash-latest")] {
            assert_eq!(resolve(Provider::Google, stored, &d), "gemini-flash-latest", "{stored:?}");
        }
        assert_eq!(resolve(Provider::Anthropic, Some("claude-haiku-4-5"), &d), "claude-haiku-5-5");
        assert_eq!(resolve(Provider::OpenAI, Some("gpt-4o-mini"), &d), "gpt-5.4-mini");
        assert_eq!(resolve(Provider::Google, Some("gemini-2.5-pro"), &d), "gemini-2.5-pro", "a choice is never overridden");
        assert_eq!(resolve(Provider::Anthropic, Some(" claude-opus-5 "), &d), "claude-opus-5");
        assert_eq!(resolve(Provider::OpenRouter, Some("vendor/m"), &d), "vendor/m");
        assert_eq!(resolve(Provider::OpenRouter, None, &d), "");
        assert!(is_unchosen(Provider::OpenAI, "gpt-4o-mini", &d));
        assert!(!is_unchosen(Provider::OpenAI, "gpt-4.1", &d));
    }

    #[test]
    fn a_fallback_never_moves_a_chosen_model_to_a_pricier_family() {
        let d = Defaults::builtin();
        // Same family, or a pricier model: falls back to the default.
        assert_eq!(fallback_for(Provider::Google, "gemini-2.5-flash", &d).as_deref(), Some("gemini-flash-latest"));
        assert_eq!(fallback_for(Provider::Google, "gemini-3.0-flash-preview", &d).as_deref(), Some("gemini-flash-latest"));
        assert_eq!(fallback_for(Provider::Google, "gemini-2.5-pro", &d).as_deref(), Some("gemini-flash-latest"));
        assert_eq!(fallback_for(Provider::OpenAI, "gpt-4.1-mini", &d).as_deref(), Some("gpt-5.4-mini"));
        assert_eq!(fallback_for(Provider::Anthropic, "claude-opus-4-1", &d).as_deref(), Some("claude-haiku-5-5"), "unknown price: priced high");
        assert_eq!(fallback_for(Provider::Anthropic, "claude-sonnet-5", &d).as_deref(), Some("claude-haiku-5-5"));
        // Lighter than the family: no fallback, the link fails as unavailable.
        assert_eq!(fallback_for(Provider::Google, "gemini-2.0-flash-lite", &d), None);
        assert_eq!(fallback_for(Provider::OpenAI, "gpt-4.1-nano", &d), None);
        // The default itself, and providers without one.
        assert_eq!(fallback_for(Provider::Google, "gemini-flash-latest", &d), None);
        assert_eq!(fallback_for(Provider::OpenRouter, "vendor/m", &d), None);
    }

    #[test]
    fn a_substitution_is_recorded_until_the_chosen_model_answers_again() {
        let store = memory::Store::open_in_memory().unwrap();
        note_answer(&store, Provider::Google, "gemini-2.5-pro", None, 1);
        assert!(unavailable(&store).is_empty(), "answering by itself records nothing");
        note_answer(&store, Provider::Google, "gemini-2.5-flash", Some("gemini-flash-latest"), 1);
        assert!(unavailable(&store).is_empty(), "an old default standing down is no warning");

        note_answer(&store, Provider::Google, "gemini-2.5-pro", Some("gemini-flash-latest"), 10);
        note_answer(&store, Provider::Google, "gemini-2.5-pro", Some("gemini-flash-latest"), 20);
        note_answer(&store, Provider::OpenAI, "gpt-4.1", Some("gpt-5.4-mini"), 30);
        let seen = unavailable(&store);
        assert_eq!(
            seen[0],
            ModelUnavailable { provider: "google".into(), model: "gemini-2.5-pro".into(), using: "gemini-flash-latest".into(), since_ms: 10 },
            "the first sighting's time is kept"
        );
        assert_eq!(seen[1].provider, "openai");

        // Another model answering for Google says nothing about the recorded one.
        note_answer(&store, Provider::Google, "gemini-flash-latest", None, 40);
        assert_eq!(unavailable(&store).len(), 2);
        note_answer(&store, Provider::Google, "gemini-2.5-pro", None, 50);
        assert_eq!(unavailable(&store).iter().map(|u| u.provider.as_str()).collect::<Vec<_>>(), ["openai"]);
        clear_unavailable(&store, Provider::OpenAI);
        assert!(unavailable(&store).is_empty());
    }

    /// Serves `reply` with `status` to every request, reading each whole request first.
    fn serve(status: &'static str, reply: &'static str) -> String {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/models/defaults.json", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { return };
                let mut reader = std::io::BufReader::new(conn.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }
                let mut conn = conn;
                let _ = write!(conn, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
                let _ = conn.flush();
            }
        });
        url
    }

    #[test]
    fn fetch_takes_only_a_usable_file() {
        assert_eq!(fetch(&serve("200 OK", FILE)).unwrap(), FILE);
        assert_eq!(fetch(&serve("404 Not Found", "nope")).unwrap_err(), "HTTP 404");
        assert!(fetch(&serve("200 OK", r#"{"version":2}"#)).is_err());
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert!(fetch(&format!("http://{closed}/d.json")).is_err(), "offline keeps the cache");
    }
}
