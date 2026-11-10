//! Settings sync (docs/superpowers/plans/2026-11-04-windows-settings-sync.md): this PC's synced
//! settings go up to the account, and newer ones from the account's other devices come down.
//!
//! Core owns the rules (`api::sync_ops::coordinators`): field-level last-writer-wins, the
//! newer-than-local gate and the API-key gate. This file maps each synced key to the settings file,
//! as the Mac's `SettingsSyncCoordinator` does.
//!
//! - **Values** are the Mac's strings: plain text, `"true"`/`"false"`, and `profile.facts` as its
//!   JSON array.
//! - **Timestamps** live in the settings file (`SettingsFile::stamp_of`). A field never edited on
//!   this PC is 0, so on a first sync the account's value wins and this PC's can't overwrite it.
//! - **Only the keys [`synced_keys`] lists** (plus the routine fields and hidden-episode marks, which
//!   live in the store) ever leave this PC. Device-local and Windows-only keys are never gathered, and
//!   a pulled key not on the list is ignored and not stamped.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use api::sync_ops::coordinators::{apply_plan, Iteration, LocalField, LocalSettings, PulledField, SettingsField};
use memory::{Routine, RoutineTemplate};

use api::sync_ops::coordinators::ApiKeyState;

use crate::profile::{Fact, MAX_FACTS, MAX_FACT_CHARS};
use crate::secrets::{accounts, SecretStore, APP_SERVICE};
use crate::settings::{keys, PromptSurface, Provider, RoutingSettings, SettingsFile};

/// The API keys that can sync (plan 2026-11-07-windows-api-key-sync.md): the field key, and the
/// Credential Manager account it lives under (the Mac's Keychain names). Each moves only while its
/// own `<field>.syncEnabled` switch is on; the image-search key and the sync secrets never do.
pub const API_KEYS: &[(&str, &str)] = &[
    ("apiKey.anthropic", accounts::ANTHROPIC_API_KEY),
    ("apiKey.openai", accounts::OPENAI_API_KEY),
    ("apiKey.google", accounts::GOOGLE_API_KEY),
    ("apiKey.openrouter", accounts::OPENROUTER_API_KEY),
];

/// A hosted provider's API-key field (`None` for Ollama).
pub fn api_key_field(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Anthropic => Some(API_KEYS[0].0),
        Provider::Openai => Some(API_KEYS[1].0),
        Provider::Google => Some(API_KEYS[2].0),
        Provider::Openrouter => Some(API_KEYS[3].0),
        Provider::Ollama | Provider::Whimbrel => None,
    }
}

/// The settings key of a key field's sync switch.
pub fn api_key_switch(field: &str) -> String {
    format!("{field}.syncEnabled")
}

/// Whether this PC's switch for `field` is on (off unless someone turned it on).
pub fn api_key_sync_enabled(file: &SettingsFile, field: &str) -> bool {
    file.get::<bool>(&api_key_switch(field)).unwrap_or(false)
}

/// Each key's state for core's push rule. The credential is read only while its switch is on. A key
/// Credential Manager can't read is left out of the round: passing "no key" would make core send a
/// tombstone and delete it on the other devices.
pub fn api_key_states(file: &SettingsFile, secrets: &dyn SecretStore) -> Vec<ApiKeyState> {
    let mut out = Vec::new();
    for &(field, account) in API_KEYS {
        let sync_enabled = api_key_sync_enabled(file, field);
        let key = if sync_enabled {
            match secrets.get_string(APP_SERVICE, account) {
                Ok(key) => key.map(|k| k.trim().to_string()),
                Err(e) => {
                    tracing::warn!(field, error = %e, "couldn't read an API key to sync; left out of this round");
                    continue;
                }
            }
        } else {
            None
        };
        out.push(ApiKeyState {
            field_key: field.to_string(),
            sync_enabled,
            key,
            key_updated_at_unix_ms: file.stamp_of(field),
            toggle_updated_at_unix_ms: file.stamp_of(&api_key_switch(field)),
        });
    }
    out
}

/// Writes a pulled key (trimmed) or deletes the credential for a tombstone. Core's gate has already
/// checked this PC's switch. False when Credential Manager failed or the key is blank, so the field
/// isn't stamped and comes again next round.
fn apply_api_key(secrets: &dyn SecretStore, field: &PulledField) -> bool {
    let Some(&(_, account)) = API_KEYS.iter().find(|(f, _)| *f == field.field_key) else { return false };
    let written = if field.is_deleted {
        secrets.delete(APP_SERVICE, account)
    } else {
        match field.value.trim() {
            "" => return false,
            key => secrets.set_string(APP_SERVICE, account, key),
        }
    };
    if let Err(e) = &written {
        tracing::warn!(field = %field.field_key, error = %e, "couldn't save a synced API key");
    }
    written.is_ok()
}

/// The Profile's keys (plan Decision 1).
pub const PROFILE_KEYS: &[&str] = &[
    keys::PROFILE_NAME,
    keys::PROFILE_ROLE,
    keys::PROFILE_ORGANIZATION,
    keys::PROFILE_ABOUT,
    keys::PROFILE_FACTS,
    keys::PROFILE_ENABLED,
    keys::PROFILE_SUGGEST_ENABLED,
    keys::PROFILE_OTHER_NAMES,
];

/// How a flat (non-Profile) key is encoded and checked, as the Mac's stores do (plan Decision 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `ollama`, `anthropic`, `openai`, `google` or `openrouter`.
    Provider,
    /// A default model: blank goes back to this PC's default.
    Model,
    /// Free text (the OpenRouter pick, prompts, the routines host): blank removes the key.
    Text,
    /// A spend cap in cents: `""` is the built-in default, `0` is no cap.
    Cap,
    /// `"true"` / `"false"`, with the default when unset.
    Flag(bool),
    /// Whole days, at least 1, with the default when unset.
    Days(u32),
}

/// Routing, retention, the prompts and the routines host: the flat keys this PC syncs.
fn flat_keys() -> Vec<(String, Kind)> {
    use crate::retention::{DEFAULT_CAPTURE_DAYS, DEFAULT_MEETING_DAYS};
    let d = RoutingSettings::default();
    let mut out: Vec<(String, Kind)> = [
        (keys::PREFERRED_PROVIDER, Kind::Provider),
        (keys::OLLAMA_MODEL, Kind::Model),
        (keys::ANTHROPIC_MODEL, Kind::Model),
        (keys::OPENAI_MODEL, Kind::Model),
        (keys::GOOGLE_MODEL, Kind::Model),
        (keys::OPENROUTER_MODEL, Kind::Text),
        (keys::USER_DAILY_CAP_CENTS, Kind::Cap),
        (keys::GLOBAL_DAILY_CAP_CENTS, Kind::Cap),
        (keys::LOCAL_ONLY_MODE, Kind::Flag(d.local_only)),
        (keys::CHEAP_FIRST, Kind::Flag(d.cheap_first)),
        (keys::USE_LOCAL_MODEL, Kind::Flag(d.use_local_model)),
        (keys::RETENTION_CAPTURE_DAYS, Kind::Days(DEFAULT_CAPTURE_DAYS)),
        (keys::RETENTION_CAPTURE_FOREVER, Kind::Flag(false)),
        (keys::RETENTION_MEETING_DAYS, Kind::Days(DEFAULT_MEETING_DAYS)),
        (keys::RETENTION_MEETING_FOREVER, Kind::Flag(false)),
        (keys::SUMMARY_CUSTOM_PROMPT, Kind::Text),
        (keys::ROUTINES_PRIMARY_DEVICE, Kind::Text),
    ]
    .into_iter()
    .map(|(k, kind)| (k.to_string(), kind))
    .collect();
    out.extend(PromptSurface::ALL.iter().map(|s| (s.key(), Kind::Text)));
    // Monthly budgets read like the daily caps: cents, `""` none (0 is none too).
    out.extend(crate::settings::PAID_PROVIDERS.iter().map(|p| (crate::settings::monthly_budget_key(p), Kind::Cap)));
    // The API keys' sync switches (not the keys, which never go in the settings file).
    out.extend(API_KEYS.iter().map(|(field, _)| (api_key_switch(field), Kind::Flag(false))));
    out
}

/// Every settings-file key this PC syncs: the Profile, then the flat keys.
pub fn synced_keys() -> Vec<String> {
    PROFILE_KEYS.iter().map(|k| k.to_string()).chain(flat_keys().into_iter().map(|(k, _)| k)).collect()
}

fn kind_of(key: &str) -> Option<Kind> {
    flat_keys().into_iter().find(|(k, _)| k == key).map(|(_, kind)| kind)
}

/// A model key's value as the routing settings read it (the stored pick, else this PC's default).
fn model_of(r: &RoutingSettings, key: &str) -> String {
    match key {
        keys::OLLAMA_MODEL => r.ollama_model.clone(),
        keys::ANTHROPIC_MODEL => r.anthropic_model.clone(),
        keys::OPENAI_MODEL => r.openai_model.clone(),
        _ => r.google_model.clone(),
    }
}

fn provider_name(p: Provider) -> String {
    serde_json::to_value(p).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn encode_flat(file: &SettingsFile, routing: &RoutingSettings, key: &str, kind: Kind) -> String {
    match kind {
        Kind::Provider => provider_name(routing.preferred_provider),
        // The stored Ollama model, never this PC's in-memory stand-in for a missing default
        // (`ollama_default`): another computer has its own models.
        Kind::Model if key == keys::OLLAMA_MODEL => file.ollama_model_choice().0,
        Kind::Model => model_of(routing, key),
        Kind::Text => file.get::<String>(key).map(|s| s.trim().to_string()).unwrap_or_default(),
        Kind::Cap => file.get::<i64>(key).map(|c| c.to_string()).unwrap_or_default(),
        Kind::Flag(default) => flag(file.get::<bool>(key).unwrap_or(default)),
        Kind::Days(default) => file.get::<u32>(key).filter(|d| *d > 0).unwrap_or(default).to_string(),
    }
}

/// Writes a pulled flat value, or false if this PC wouldn't accept it as typed in.
fn apply_flat(file: &mut SettingsFile, key: &str, kind: Kind, value: &str) -> bool {
    let text = value.trim();
    match kind {
        Kind::Provider => match serde_json::from_value::<Provider>(serde_json::Value::String(text.to_string())) {
            Ok(p) => file.set(key, Some(p)),
            Err(_) => return false,
        },
        Kind::Model | Kind::Text => file.set(key, (!text.is_empty()).then_some(text)),
        Kind::Cap if text.is_empty() => file.set::<i64>(key, None),
        Kind::Cap => match text.parse::<i64>() {
            Ok(c) if c >= 0 => file.set(key, Some(c)),
            _ => return false,
        },
        Kind::Flag(_) => match parse_flag(text) {
            Some(b) => file.set(key, Some(b)),
            None => return false,
        },
        Kind::Days(_) => match text.parse::<u32>() {
            Ok(d) if d >= 1 => file.set(key, Some(d)),
            _ => return false,
        },
    }
    true
}

const TEXT_KEYS: &[&str] = &[keys::PROFILE_NAME, keys::PROFILE_ROLE, keys::PROFILE_ORGANIZATION, keys::PROFILE_ABOUT];

/// Now, in unix ms: the stamp of an edit made on this PC.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn flag(b: bool) -> String {
    if b { "true" } else { "false" }.to_string()
}

/// Each synced key's value as the Mac encodes it. An unset text field is `""`.
pub fn encoded(file: &SettingsFile) -> BTreeMap<String, String> {
    let profile = file.profile();
    let mut out = BTreeMap::new();
    out.insert(keys::PROFILE_NAME.to_string(), profile.name);
    out.insert(keys::PROFILE_ROLE.to_string(), profile.role);
    out.insert(keys::PROFILE_ORGANIZATION.to_string(), profile.organization);
    out.insert(keys::PROFILE_ABOUT.to_string(), profile.about);
    out.insert(keys::PROFILE_FACTS.to_string(), serde_json::to_string(&profile.facts).unwrap_or_else(|_| "[]".into()));
    out.insert(keys::PROFILE_ENABLED.to_string(), flag(profile.enabled));
    out.insert(keys::PROFILE_SUGGEST_ENABLED.to_string(), flag(profile.suggest_enabled));
    out.insert(keys::PROFILE_OTHER_NAMES.to_string(), file.other_names());
    let routing = file.routing();
    for (key, kind) in flat_keys() {
        let value = encode_flat(file, &routing, &key, kind);
        out.insert(key, value);
    }
    out
}

/// Runs `change` on the settings and stamps "now" on each synced key it changed. Every write path
/// a person drives goes through this, so their edits beat the account's older copies.
pub fn edit<R>(file: &mut SettingsFile, change: impl FnOnce(&mut SettingsFile) -> R) -> R {
    let before = encoded(file);
    let out = change(file);
    stamp_changed(file, &before, now_ms());
    out
}

/// Stamps `now_ms` on each synced key whose value differs from `before` (taken with [`encoded`]
/// ahead of an edit). This is how a real edit becomes newer than the account's copy; a write that
/// changes nothing stamps nothing.
pub fn stamp_changed(file: &mut SettingsFile, before: &BTreeMap<String, String>, now_ms: i64) {
    for (key, value) in encoded(file) {
        if before.get(&key) != Some(&value) {
            file.set_stamp(&key, now_ms);
        }
    }
}

/// What this PC pushes: every synced key with its stamp (0 if never edited here).
pub fn gather(file: &SettingsFile) -> LocalSettings {
    let fields = encoded(file)
        .into_iter()
        .map(|(key, value)| {
            let updated_at_unix_ms = file.stamp_of(&key);
            (key, LocalField { value, updated_at_unix_ms })
        })
        .collect();
    LocalSettings { fields, ..LocalSettings::default() }
}

/// A pulled fact list, if this PC would accept it as typed in: valid JSON, at most 30 facts, each
/// one non-empty line of at most 200 characters.
fn valid_facts(value: &str) -> Option<Vec<Fact>> {
    let facts: Vec<Fact> = serde_json::from_str(value).ok()?;
    let ok = facts.len() <= MAX_FACTS
        && facts.iter().all(|f| {
            let t = f.text.trim();
            !t.is_empty() && !t.contains(['\n', '\r']) && t.chars().count() <= MAX_FACT_CHARS
        });
    ok.then_some(facts)
}

fn parse_flag(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Writes one pulled value. False (nothing written) for a key this PC doesn't sync, a tombstone,
/// or a value it wouldn't accept; the caller then leaves the stamp alone, as the Mac does.
pub fn apply_field(file: &mut SettingsFile, field: &PulledField) -> bool {
    if field.is_deleted {
        return false;
    }
    if let Some(kind) = kind_of(&field.field_key) {
        return apply_flat(file, &field.field_key, kind, &field.value);
    }
    if !PROFILE_KEYS.contains(&field.field_key.as_str()) {
        return false;
    }
    if field.field_key == keys::PROFILE_OTHER_NAMES {
        file.set_other_names(&field.value);
        return true;
    }
    let mut profile = file.profile();
    let key = field.field_key.as_str();
    let value = field.value.as_str();
    match key {
        k if TEXT_KEYS.contains(&k) => {
            let slot = match k {
                keys::PROFILE_NAME => &mut profile.name,
                keys::PROFILE_ROLE => &mut profile.role,
                keys::PROFILE_ORGANIZATION => &mut profile.organization,
                _ => &mut profile.about,
            };
            *slot = value.to_string();
        }
        keys::PROFILE_FACTS => match valid_facts(value) {
            Some(facts) => profile.facts = facts,
            None => return false,
        },
        keys::PROFILE_ENABLED => match parse_flag(value) {
            Some(b) => profile.enabled = b,
            None => return false,
        },
        keys::PROFILE_SUGGEST_ENABLED => match parse_flag(value) {
            Some(b) => profile.suggest_enabled = b,
            None => return false,
        },
        _ => return false,
    }
    file.set_profile(&profile);
    true
}

/// Applies a pulled delta: core's gate against the stamps as they are now (an edit made during the
/// round trip wins), then each field, stamped with its own timestamp when it applied. Returns how
/// many applied. Does not save.
pub fn apply_pulled(file: &mut SettingsFile, pulled: &HashMap<String, api::sync_ops::coordinators::SettingsField>) -> usize {
    let stamps: HashMap<String, i64> = pulled.keys().map(|k| (k.clone(), file.stamp_of(k))).collect();
    let plan = apply_plan(pulled, &stamps, &HashMap::new());
    let mut applied = 0;
    for field in &plan.fields {
        if apply_field(file, field) {
            file.set_stamp(&field.field_key, field.updated_at_unix_ms);
            applied += 1;
        }
    }
    applied
}

// --- Routines and hidden episodes, which live in the store ------------------------------------------

/// The routines that can fail over (the Mac's `RoutineSyncFields.templates`): only their on/off, time
/// and model sync, so a device standing in for the primary runs them the same way.
fn routine_name(template: RoutineTemplate) -> Option<&'static str> {
    match template {
        RoutineTemplate::DailyBrief => Some("daily_brief"),
        RoutineTemplate::ReplyPriorities => Some("reply_priorities"),
        _ => None,
    }
}

const ROUTINE_FIELDS: [&str; 3] = ["enabled", "schedule", "model_pref"];

/// `routine.<template>.<field>`, or `None` for a routine whose settings stay on this PC.
pub fn routine_key(template: RoutineTemplate, field: &str) -> Option<String> {
    routine_name(template).map(|name| format!("routine.{name}.{field}"))
}

fn parse_routine_key(key: &str) -> Option<(&str, &str)> {
    let (template, field) = key.strip_prefix("routine.")?.split_once('.')?;
    (["daily_brief", "reply_priorities"].contains(&template) && ROUTINE_FIELDS.contains(&field)).then_some((template, field))
}

/// The routine fields to push, read from this PC's routine rows. A template with no row pushes
/// nothing, as on the Mac.
pub fn routine_values(routines: &[Routine]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for r in routines {
        let Some(name) = routine_name(r.template) else { continue };
        out.insert(format!("routine.{name}.enabled"), flag(r.enabled));
        out.insert(format!("routine.{name}.schedule"), r.schedule_hhmm.clone());
        out.insert(format!("routine.{name}.model_pref"), r.model_pref.clone());
    }
    out
}

/// A pulled routine field as an edit to this PC's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineChange {
    Enabled { id: i64, on: bool },
    Schedule { id: i64, hhmm: String },
    ModelPref { id: i64, pref: String },
}

/// The edit a pulled routine field asks for; `None` when the key is unknown, the value malformed, or
/// this PC has no row of that template (the stamp then stays put, so it is retried).
pub fn routine_change(key: &str, value: &str, routines: &[Routine]) -> Option<RoutineChange> {
    let (template, field) = parse_routine_key(key)?;
    let id = routines.iter().find(|r| routine_name(r.template) == Some(template))?.id;
    match field {
        "enabled" => parse_flag(value).map(|on| RoutineChange::Enabled { id, on }),
        "schedule" => crate::routines::valid_schedule(value).map(|hhmm| RoutineChange::Schedule { id, hhmm }),
        _ => Some(RoutineChange::ModelPref { id, pref: value.to_string() }),
    }
}

/// What the round reads from the store before the network call.
#[derive(Debug, Clone, Default)]
pub struct StoreLocal {
    pub routines: Vec<Routine>,
    /// `episode.hidden.*` marks; core owns their timestamps and last-writer-wins.
    pub hidden_marks: HashMap<String, SettingsField>,
}

/// What `StoreSide::apply` reports: the routine keys that applied, and how many hidden marks did.
pub type StoreApplied = (Vec<String>, usize);

/// The store's half of a round: the store actor in production, a fake in tests.
pub trait StoreSide {
    fn gather(&self) -> Result<StoreLocal, String>;
    /// Applies each routine change this PC accepts (the same checks the Routines tab makes), then the
    /// hidden marks.
    fn apply(&self, changes: Vec<(String, RoutineChange)>, hidden: HashMap<String, SettingsField>) -> Result<StoreApplied, String>;
}

impl StoreSide for crate::store_actor::StoreHandle {
    fn gather(&self) -> Result<StoreLocal, String> {
        self.call_blocking(|s| -> Result<StoreLocal, String> {
            let routines = s.list_routines().map_err(|e| e.to_string())?;
            let marks = api::sync_ops::hidden_episode_sync_fields(s).map_err(|e| e.to_string())?;
            let hidden_marks = marks.into_iter().filter_map(|(k, v)| Some((k, serde_json::from_value(v).ok()?))).collect();
            Ok(StoreLocal { routines, hidden_marks })
        })
        .map_err(|e| e.to_string())?
    }

    fn apply(&self, changes: Vec<(String, RoutineChange)>, hidden: HashMap<String, SettingsField>) -> Result<StoreApplied, String> {
        use crate::commands::routines::{set_enabled, set_model_pref, set_schedule};
        self.call_blocking(move |s| -> Result<StoreApplied, String> {
            let mut applied = Vec::new();
            for (key, change) in changes {
                let result = match &change {
                    RoutineChange::Enabled { id, on } => set_enabled(s, *id, *on, crate::routines::capabilities(s)),
                    RoutineChange::Schedule { id, hhmm } => set_schedule(s, *id, hhmm),
                    RoutineChange::ModelPref { id, pref } => set_model_pref(s, *id, pref),
                };
                match result {
                    Ok(()) => applied.push(key),
                    Err(e) => tracing::info!(%key, error = %e, "a synced routine setting doesn't apply on this PC"),
                }
            }
            let fields: api::sync_ops::SettingsFields =
                hidden.into_iter().filter_map(|(k, v)| Some((k, serde_json::to_value(v).ok()?))).collect();
            let marks = if fields.is_empty() { 0 } else { api::sync_ops::apply_hidden_episode_sync_fields(s, &fields).map_err(|e| e.to_string())? };
            Ok((applied, marks))
        })
        .map_err(|e| e.to_string())?
    }
}

/// What this PC pushes: [`gather`] plus the routine fields (stamped in the settings file, as on the
/// Mac), the hidden marks and the API keys.
pub fn gather_all(file: &SettingsFile, store: &StoreLocal, secrets: &dyn SecretStore) -> LocalSettings {
    let mut local = gather(file);
    for (key, value) in routine_values(&store.routines) {
        let updated_at_unix_ms = file.stamp_of(&key);
        local.fields.insert(key, LocalField { value, updated_at_unix_ms });
    }
    local.hidden_episode_marks = store.hidden_marks.clone();
    local.api_keys = api_key_states(file, secrets);
    local
}

/// One settings round. The settings lock is held only to gather and to apply, never across the store
/// or `iterate` (the network call: `settings_iteration` in production, a fake in tests).
pub fn run_round<F>(settings: &Mutex<SettingsFile>, store: &dyn StoreSide, secrets: &dyn SecretStore, iterate: F) -> Result<usize, String>
where
    F: FnOnce(&LocalSettings) -> Result<Iteration, String>,
{
    let store_local = store.gather()?;
    let local = gather_all(&settings.lock().unwrap(), &store_local, secrets);
    let iteration = iterate(&local)?;
    if iteration.pulled.is_empty() {
        return Ok(0);
    }
    let mut applied = 0;
    let mut routine_changes = Vec::new();
    let mut routine_stamps = HashMap::new();
    let hidden = {
        let mut file = settings.lock().unwrap();
        let stamps: HashMap<String, i64> = iteration.pulled.keys().map(|k| (k.clone(), file.stamp_of(k))).collect();
        // This PC's switches as they are now: a key lands only where its switch is on.
        let switches: HashMap<String, bool> = API_KEYS.iter().map(|(f, _)| (f.to_string(), api_key_sync_enabled(&file, f))).collect();
        let plan = apply_plan(&iteration.pulled, &stamps, &switches);
        for field in &plan.fields {
            if switches.contains_key(&field.field_key) {
                if apply_api_key(secrets, field) {
                    file.set_stamp(&field.field_key, field.updated_at_unix_ms);
                    applied += 1;
                }
            } else if parse_routine_key(&field.field_key).is_some() {
                let change = if field.is_deleted { None } else { routine_change(&field.field_key, &field.value, &store_local.routines) };
                if let Some(change) = change {
                    routine_changes.push((field.field_key.clone(), change));
                    routine_stamps.insert(field.field_key.clone(), field.updated_at_unix_ms);
                }
            } else if apply_field(&mut file, field) {
                file.set_stamp(&field.field_key, field.updated_at_unix_ms);
                applied += 1;
            }
        }
        if applied > 0 {
            file.save().map_err(|e| e.to_string())?;
        }
        plan.hidden_episode_marks
    };
    if !routine_changes.is_empty() || !hidden.is_empty() {
        let (routine_keys, marks) = store.apply(routine_changes, hidden)?;
        applied += marks;
        if !routine_keys.is_empty() {
            let mut file = settings.lock().unwrap();
            for key in &routine_keys {
                file.set_stamp(key, routine_stamps[key]);
            }
            file.save().map_err(|e| e.to_string())?;
            applied += routine_keys.len();
        }
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::sync_ops::coordinators::SettingsField;
    use crate::secrets::MemorySecrets;

    /// A fresh folder, removed when dropped.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tempdir() -> TempDir {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("whimbrel-settings-sync-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn file() -> (TempDir, SettingsFile) {
        let dir = tempdir();
        let f = SettingsFile::load(&dir.path().join("settings.json"));
        (dir, f)
    }

    fn remote(value: &str, ts: i64) -> SettingsField {
        SettingsField { value: value.into(), updated_at_unix_ms: ts, is_deleted: false }
    }

    fn pulled(key: &str, value: &str, ts: i64) -> PulledField {
        PulledField { field_key: key.into(), value: value.into(), updated_at_unix_ms: ts, is_deleted: false }
    }

    fn routine(id: i64, template: RoutineTemplate) -> Routine {
        Routine {
            id,
            name: format!("{template:?}"),
            template,
            schedule_hhmm: "08:00".into(),
            enabled: false,
            auto_approve_write_actions: false,
            model_pref: "default".into(),
            created_at_unix_ms: 0,
        }
    }

    /// A store that holds `local` and accepts changes unless their key is in `refuse`.
    #[derive(Default)]
    struct FakeStore {
        local: StoreLocal,
        refuse: Vec<String>,
        applied: Mutex<Vec<(String, RoutineChange)>>,
        marks: Mutex<HashMap<String, SettingsField>>,
    }

    impl StoreSide for FakeStore {
        fn gather(&self) -> Result<StoreLocal, String> {
            Ok(self.local.clone())
        }

        fn apply(&self, changes: Vec<(String, RoutineChange)>, hidden: HashMap<String, SettingsField>) -> Result<StoreApplied, String> {
            let accepted: Vec<_> = changes.into_iter().filter(|(k, _)| !self.refuse.contains(k)).collect();
            let keys = accepted.iter().map(|(k, _)| k.clone()).collect();
            self.applied.lock().unwrap().extend(accepted);
            let n = hidden.len();
            self.marks.lock().unwrap().extend(hidden);
            Ok((keys, n))
        }
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort_unstable();
        v
    }

    #[test]
    fn every_key_round_trips_through_its_string() {
        let (_d, mut a) = file();
        let mut p = a.profile();
        p.name = "Mike Pointer".into();
        p.role = "Founder".into();
        p.organization = "Whimbrel".into();
        p.about = "Builds things".into();
        p.facts = vec![Fact { id: "f1".into(), text: "Works in US Eastern time".into(), source_meeting_id: Some(7) }];
        p.enabled = false;
        p.suggest_enabled = false;
        a.set_profile(&p);
        a.set_other_names("Mikey, MP");
        let mut r = a.routing();
        r.preferred_provider = Provider::Openrouter;
        r.anthropic_model = "claude-sonnet-5".into();
        r.openrouter_model = "meta/llama-5".into();
        r.user_daily_cap_cents = Some(0);
        r.global_daily_cap_cents = Some(250);
        r.local_only = true;
        r.cheap_first = false;
        a.set_routing(&r);
        a.set_retention(crate::retention::RetentionSettings { capture_days: None, meeting_days: Some(14), conversation_days: Some(30) });
        a.set_prompt(PromptSurface::Ask, "Answer in bullets");
        a.set(keys::SUMMARY_CUSTOM_PROMPT, Some("Lead with decisions"));
        a.set_primary_device_id("MACKEY");
        let (_d2, mut b) = file();
        for (key, value) in encoded(&a) {
            assert!(apply_field(&mut b, &pulled(&key, &value, 5)), "{key} should apply");
        }
        assert_eq!(b.profile(), a.profile());
        assert_eq!(b.other_names(), "Mikey, MP");
        assert_eq!(b.routing().preferred_provider, Provider::Openrouter);
        assert_eq!((b.routing().user_daily_cap_cents, b.routing().global_daily_cap_cents), (Some(0), Some(250)));
        assert_eq!(b.retention().meeting_days, Some(14));
        assert_eq!(b.retention().capture_days, None, "keep forever");
        assert_eq!(b.prompt(PromptSurface::Ask), "Answer in bullets");
        assert_eq!(b.primary_device_id(), "MACKEY");
        assert_eq!(encoded(&b), encoded(&a));
    }

    #[test]
    fn stamps_default_to_zero_and_survive_a_reload_with_their_values() {
        let dir = tempdir();
        let path = dir.path().join("settings.json");
        let mut f = SettingsFile::load(&path);
        assert_eq!(f.stamp_of(keys::PROFILE_NAME), 0);
        f.set(keys::PROFILE_NAME, Some("Mike"));
        assert_eq!(f.stamp_of(keys::PROFILE_NAME), 0, "a plain set (a migration, a default) never stamps");
        f.set_stamp(keys::PROFILE_NAME, 42);
        f.save().unwrap();
        let back = SettingsFile::load(&path);
        assert_eq!((back.profile().name.as_str(), back.stamp_of(keys::PROFILE_NAME)), ("Mike", 42));

        let mut bad = SettingsFile::load(&path);
        bad.set(keys::SYNC_FIELD_TIMESTAMPS, Some("not an object"));
        assert_eq!(bad.stamp_of(keys::PROFILE_NAME), 0, "a malformed stamps object reads as all zeros");
        assert_eq!(bad.profile().name, "Mike", "and doesn't touch the values");
        bad.set_stamp(keys::PROFILE_ROLE, 7);
        assert_eq!(bad.stamp_of(keys::PROFILE_ROLE), 7, "a stamp replaces the malformed object");
    }

    #[test]
    fn the_stamps_key_is_never_synced() {
        assert!(!synced_keys().iter().any(|k| k == keys::SYNC_FIELD_TIMESTAMPS));
    }

    #[test]
    fn mac_shaped_facts_apply() {
        let (_d, mut f) = file();
        // Swift's Codable: any key order, and a nil sourceMeetingId left out.
        let mac = r#"[{"text":"Prefers mornings","id":"A1B2"},{"sourceMeetingId":42,"id":"C3","text":"Leads the data team"}]"#;
        assert!(apply_field(&mut f, &pulled(keys::PROFILE_FACTS, mac, 5)));
        let facts = f.profile().facts;
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0], Fact { id: "A1B2".into(), text: "Prefers mornings".into(), source_meeting_id: None });
        assert_eq!(facts[1].source_meeting_id, Some(42));
    }

    #[test]
    fn facts_this_pc_would_refuse_are_not_applied() {
        let fact = |text: &str| Fact { id: "x".into(), text: text.into(), source_meeting_id: None };
        let too_many: Vec<Fact> = (0..=MAX_FACTS).map(|i| Fact { id: i.to_string(), text: format!("fact {i}"), source_meeting_id: None }).collect();
        let cases = [
            serde_json::to_string(&too_many).unwrap(),
            serde_json::to_string(&[fact(&"x".repeat(MAX_FACT_CHARS + 1))]).unwrap(),
            serde_json::to_string(&[fact("two\nlines")]).unwrap(),
            serde_json::to_string(&[fact("  ")]).unwrap(),
            "not json".to_string(),
        ];
        for value in cases {
            let (_d, mut f) = file();
            let delta = HashMap::from([(keys::PROFILE_FACTS.to_string(), remote(&value, 9))]);
            assert_eq!(apply_pulled(&mut f, &delta), 0, "{value:.40} should be refused");
            assert_eq!(f.stamp_of(keys::PROFILE_FACTS), 0, "a refused value isn't stamped");
        }
    }

    #[test]
    fn bad_flags_tombstones_and_unknown_keys_are_ignored() {
        let (_d, mut f) = file();
        assert!(!apply_field(&mut f, &pulled(keys::PROFILE_ENABLED, "yes", 5)));
        let mut tomb = pulled(keys::PROFILE_NAME, "", 5);
        tomb.is_deleted = true;
        assert!(!apply_field(&mut f, &tomb));
        assert!(!apply_field(&mut f, &pulled("prompts.someFutureSurface", "Be brief", 5)));
        assert!(!apply_field(&mut f, &pulled("apiKey.anthropic", "sk-x", 5)), "API keys aren't synced yet");
        assert!(!apply_field(&mut f, &pulled(keys::CAPTURE_ENABLED, "false", 5)), "a device-local key never applies");
        assert_eq!(f.get::<String>("prompts.someFutureSurface"), None, "an unknown key isn't written into the file");
    }

    #[test]
    fn device_local_keys_are_never_gathered() {
        let (_d, mut f) = file();
        f.set(keys::CAPTURE_ENABLED, Some(false));
        f.set(keys::APP_LANGUAGE, Some("fr"));
        f.set(keys::LIVE_TRANSCRIPT_ENABLED, Some(false));
        f.set_topic_sort_enabled(true);
        f.set_topic_sort_started(true);
        f.set_topic_steps_enabled(true);
        f.set_topic_steps_started(true);
        f.set_vault_enabled(true);
        f.set_vault_path("C:\\Notes");
        f.set_vault_last_updated(1);
        f.set_vault_failed(true);
        f.set(keys::WHIMBREL_MODEL, Some("claude-sonnet-5"));
        let local = gather(&f);
        assert_eq!(sorted(local.fields.keys().cloned().collect()), sorted(synced_keys()));
        for local_only in [
            keys::CAPTURE_ENABLED,
            keys::APP_LANGUAGE,
            keys::OPENROUTER_PRICES,
            keys::RETENTION_CONVERSATION_DAYS,
            keys::RETENTION_CONVERSATION_FOREVER,
            keys::RETENTION_ACKNOWLEDGED,
            keys::EXCLUDED_BUNDLE_IDS,
            keys::LIVE_QUESTION_ENABLED,
            keys::LIVE_TRANSCRIPT_ENABLED,
            keys::REMEMBER_VOICES,
            keys::TOPIC_SORT_ENABLED,
            keys::TOPIC_SORT_STARTED,
            keys::TOPIC_STEPS_ENABLED,
            keys::TOPIC_STEPS_STARTED,
            keys::VAULT_ENABLED,
            keys::VAULT_PATH,
            keys::VAULT_LAST_UPDATED,
            keys::VAULT_FAILED,
        ] {
            assert!(!local.fields.contains_key(local_only), "{local_only} stays on this PC");
        }
        assert!(local.api_keys.is_empty() && local.hidden_episode_marks.is_empty());
    }

    #[test]
    fn the_typesafe_key_never_syncs() {
        assert!(API_KEYS.iter().all(|(_, account)| *account != accounts::TYPESAFE_API_KEY), "the TypeSafe key stays on this PC, as on the Mac");
    }

    #[test]
    fn first_sync_takes_the_accounts_values_over_unstamped_ones() {
        let (_d, mut f) = file();
        // A name typed here before sync existed: on disk, but never stamped.
        let mut p = f.profile();
        p.name = "Mike (PC)".into();
        f.set_profile(&p);
        assert!(gather(&f).fields.values().all(|field| field.updated_at_unix_ms == 0));
        let delta = HashMap::from([
            (keys::PROFILE_NAME.to_string(), remote("Mike Pointer", 1_793_789_914_162)),
            (keys::PROFILE_ROLE.to_string(), remote("Founder", 1_793_789_914_163)),
        ]);
        assert_eq!(apply_pulled(&mut f, &delta), 2);
        assert_eq!(f.profile().name, "Mike Pointer");
        assert_eq!(f.profile().role, "Founder");
        assert_eq!(f.stamp_of(keys::PROFILE_NAME), 1_793_789_914_162, "stamped with the pulled time, not now");
    }

    #[test]
    fn a_newer_local_edit_beats_an_older_pull() {
        let (_d, mut f) = file();
        let before = encoded(&f);
        let mut p = f.profile();
        p.role = "CEO".into();
        f.set_profile(&p);
        stamp_changed(&mut f, &before, 2_000);
        let delta = HashMap::from([(keys::PROFILE_ROLE.to_string(), remote("Founder", 1_500))]);
        assert_eq!(apply_pulled(&mut f, &delta), 0);
        assert_eq!(f.profile().role, "CEO");
        assert_eq!(gather(&f).fields[keys::PROFILE_ROLE], LocalField { value: "CEO".into(), updated_at_unix_ms: 2_000 });
    }

    #[test]
    fn stamp_changed_stamps_only_what_changed() {
        let (_d, mut f) = file();
        let before = encoded(&f);
        let mut p = f.profile();
        p.role = "Founder".into();
        f.set_profile(&p);
        stamp_changed(&mut f, &before, 1_000);
        assert_eq!(f.stamp_of(keys::PROFILE_ROLE), 1_000);
        for key in synced_keys().iter().filter(|k| *k != keys::PROFILE_ROLE) {
            assert_eq!(f.stamp_of(key), 0, "{key} didn't change");
        }
        let before = encoded(&f);
        f.set_profile(&f.profile());
        stamp_changed(&mut f, &before, 2_000);
        assert_eq!(f.stamp_of(keys::PROFILE_ROLE), 1_000, "saving the same value stamps nothing");
    }

    #[test]
    fn an_edit_during_the_round_trip_wins() {
        let (_d, f) = file();
        let settings = Mutex::new(f);
        let result = run_round(&settings, &FakeStore::default(), &MemorySecrets::default(), |_local| {
            // The user saves a role while the network call is out.
            let mut file = settings.lock().unwrap();
            let before = encoded(&file);
            let mut p = file.profile();
            p.role = "CEO".into();
            file.set_profile(&p);
            stamp_changed(&mut file, &before, 3_000);
            drop(file);
            Ok(Iteration { pushed_count: 7, pulled: HashMap::from([(keys::PROFILE_ROLE.to_string(), remote("Founder", 2_500))]) })
        });
        assert_eq!(result, Ok(0));
        assert_eq!(settings.lock().unwrap().profile().role, "CEO");
    }

    #[test]
    fn a_round_pushes_gathered_fields_and_saves_what_it_applied() {
        let dir = tempdir();
        let path = dir.path().join("settings.json");
        let settings = Mutex::new(SettingsFile::load(&path));
        let result = run_round(&settings, &FakeStore::default(), &MemorySecrets::default(), |local| {
            assert_eq!(local.fields.len(), synced_keys().len());
            Ok(Iteration { pushed_count: local.fields.len(), pulled: HashMap::from([(keys::PROFILE_NAME.to_string(), remote("Mike", 10))]) })
        });
        assert_eq!(result, Ok(1));
        let reloaded = SettingsFile::load(&path);
        assert_eq!(reloaded.profile().name, "Mike");
        assert_eq!(reloaded.stamp_of(keys::PROFILE_NAME), 10, "the value and its stamp were saved together");
    }

    #[test]
    fn a_failed_round_changes_nothing() {
        let (_d, f) = file();
        let settings = Mutex::new(f);
        let before = encoded(&settings.lock().unwrap());
        assert_eq!(run_round(&settings, &FakeStore::default(), &MemorySecrets::default(), |_| Err("settings sync connection failed: offline".into())), Err("settings sync connection failed: offline".into()));
        assert_eq!(encoded(&settings.lock().unwrap()), before);
    }
    #[test]
    fn flat_values_this_pc_would_refuse_are_not_applied() {
        let (_d, mut f) = file();
        for (key, bad) in [
            (keys::PREFERRED_PROVIDER, "gemini"),
            (keys::USER_DAILY_CAP_CENTS, "-5"),
            (keys::USER_DAILY_CAP_CENTS, "lots"),
            (keys::RETENTION_MEETING_DAYS, "0"),
            (keys::RETENTION_CAPTURE_DAYS, "a month"),
            (keys::LOCAL_ONLY_MODE, "yes"),
        ] {
            assert!(!apply_field(&mut f, &pulled(key, bad, 5)), "{key} = {bad:?} should be refused");
        }
        assert_eq!(encoded(&f), encoded(&file().1), "nothing was written");
    }

    #[test]
    fn monthly_budgets_sync_as_cents_and_blank_or_zero_is_none() {
        let (_d, mut f) = file();
        let openai = crate::settings::monthly_budget_key("openai");
        let google = crate::settings::monthly_budget_key("google");
        assert!(synced_keys().contains(&openai), "every paid provider's budget syncs");
        assert!(!synced_keys().contains(&crate::settings::monthly_budget_key("ollama")), "Ollama is free");
        edit(&mut f, |f| f.set_monthly_budget("openai", Some(2500)));
        assert_eq!(encoded(&f)[&openai], "2500");
        assert_eq!(encoded(&f)[&google], "", "no budget");
        assert!(apply_field(&mut f, &pulled(&google, "1000", 5)));
        assert!(apply_field(&mut f, &pulled(&openai, "0", 5)), "the Mac may send 0 for none");
        assert!(!apply_field(&mut f, &pulled(&openai, "-1", 5)));
        assert!(!apply_field(&mut f, &pulled(&openai, "$25", 5)));
        let budgets = f.monthly_budgets();
        assert_eq!(budgets.keys().cloned().collect::<Vec<_>>(), ["anthropic", "google", "openai", "openrouter"]);
        assert_eq!((budgets["openai"], budgets["google"], budgets["anthropic"]), (0, 1000, 0), "every paid provider is pushed; 0 is none");
        assert!(apply_field(&mut f, &pulled(&google, "", 5)));
        assert_eq!(f.monthly_budgets()["google"], 0);
    }

    #[test]
    fn blank_values_go_back_to_the_defaults() {
        let (_d, mut f) = file();
        f.set(keys::USER_DAILY_CAP_CENTS, Some(500));
        f.set_prompt(PromptSurface::Ask, "Be brief");
        f.set(keys::OLLAMA_MODEL, Some("qwen3:8b"));
        assert!(apply_field(&mut f, &pulled(keys::USER_DAILY_CAP_CENTS, "", 5)));
        assert!(apply_field(&mut f, &pulled(&PromptSurface::Ask.key(), "  ", 5)));
        assert!(apply_field(&mut f, &pulled(keys::OLLAMA_MODEL, "", 5)));
        assert_eq!(f.routing().user_daily_cap_cents, None, "the built-in cap");
        assert_eq!(f.prompt(PromptSurface::Ask), "");
        assert_eq!(f.routing().ollama_model, RoutingSettings::default().ollama_model);
    }

    #[test]
    fn edit_stamps_only_what_a_routing_save_changed() {
        let (_d, mut f) = file();
        let mut r = f.routing();
        r.openai_model = "gpt-5-mini".into();
        edit(&mut f, |file| file.set_routing(&r));
        assert!(f.stamp_of(keys::OPENAI_MODEL) > 0);
        for key in [keys::PREFERRED_PROVIDER, keys::OLLAMA_MODEL, keys::USER_DAILY_CAP_CENTS, keys::CHEAP_FIRST] {
            assert_eq!(f.stamp_of(key), 0, "{key} was saved but didn't change");
        }
        edit(&mut f, |file| {
            file.set_retention(crate::retention::RetentionSettings { capture_days: Some(30), meeting_days: None, conversation_days: Some(7) })
        });
        assert!(f.stamp_of(keys::RETENTION_MEETING_FOREVER) > 0);
        assert_eq!(f.stamp_of(keys::RETENTION_CAPTURE_DAYS), 0, "30 is the default it already had");
        assert_eq!(f.stamp_of(keys::RETENTION_CONVERSATION_DAYS), 0, "conversation retention isn't synced");
    }

    #[test]
    fn only_the_failover_routines_push_their_fields() {
        let mut brief = routine(1, RoutineTemplate::DailyBrief);
        brief.enabled = true;
        brief.schedule_hhmm = "07:30".into();
        brief.model_pref = "anthropic/claude-sonnet-5".into();
        let values = routine_values(&[brief, routine(2, RoutineTemplate::FocusTime), routine(3, RoutineTemplate::EodRecap)]);
        assert_eq!(
            values,
            BTreeMap::from([
                ("routine.daily_brief.enabled".to_string(), "true".to_string()),
                ("routine.daily_brief.model_pref".to_string(), "anthropic/claude-sonnet-5".to_string()),
                ("routine.daily_brief.schedule".to_string(), "07:30".to_string()),
            ]),
            "no Reply Priorities row, so nothing for it; focus time and the recap stay on this PC"
        );
    }

    #[test]
    fn a_pulled_routine_field_becomes_an_edit_to_this_pcs_row() {
        let rows = [routine(4, RoutineTemplate::DailyBrief)];
        assert_eq!(routine_change("routine.daily_brief.enabled", "true", &rows), Some(RoutineChange::Enabled { id: 4, on: true }));
        assert_eq!(routine_change("routine.daily_brief.schedule", "06:45", &rows), Some(RoutineChange::Schedule { id: 4, hhmm: "06:45".into() }));
        assert_eq!(routine_change("routine.daily_brief.model_pref", "cheapest", &rows), Some(RoutineChange::ModelPref { id: 4, pref: "cheapest".into() }));
        assert_eq!(routine_change("routine.daily_brief.schedule", "25:99", &rows), None);
        assert_eq!(routine_change("routine.daily_brief.enabled", "on", &rows), None);
        assert_eq!(routine_change("routine.reply_priorities.enabled", "true", &rows), None, "no row of that template here");
        assert_eq!(routine_change("routine.focus_time.enabled", "true", &rows), None, "not a synced routine");
    }

    #[test]
    fn a_round_pushes_routines_and_marks_and_applies_what_the_store_accepts() {
        let (_d, f) = file();
        let settings = Mutex::new(f);
        let mark = SettingsField { value: "1".into(), updated_at_unix_ms: 40, is_deleted: false };
        let store = FakeStore {
            local: StoreLocal {
                routines: vec![routine(1, RoutineTemplate::DailyBrief)],
                hidden_marks: HashMap::from([("episode.hidden.pc|chrome.exe|100".to_string(), mark)]),
            },
            refuse: vec!["routine.daily_brief.enabled".into()],
            ..FakeStore::default()
        };
        let result = run_round(&settings, &store, &MemorySecrets::default(), |local| {
            assert_eq!(local.fields["routine.daily_brief.schedule"], LocalField { value: "08:00".into(), updated_at_unix_ms: 0 });
            assert_eq!(local.hidden_episode_marks.len(), 1);
            Ok(Iteration {
                pushed_count: local.fields.len(),
                pulled: HashMap::from([
                    ("routine.daily_brief.schedule".to_string(), remote("07:15", 50)),
                    ("routine.daily_brief.enabled".to_string(), remote("true", 51)),
                    ("episode.hidden.mac|com.apple.Safari|200".to_string(), remote("1", 52)),
                ]),
            })
        });
        assert_eq!(result, Ok(2), "the schedule and the mark applied; turning it on was refused here");
        let file = settings.lock().unwrap();
        assert_eq!(file.stamp_of("routine.daily_brief.schedule"), 50);
        assert_eq!(file.stamp_of("routine.daily_brief.enabled"), 0, "a refused change isn't stamped, so it is retried");
        assert_eq!(
            store.applied.lock().unwrap()[0],
            ("routine.daily_brief.schedule".to_string(), RoutineChange::Schedule { id: 1, hhmm: "07:15".into() })
        );
        assert!(store.marks.lock().unwrap().contains_key("episode.hidden.mac|com.apple.Safari|200"));
    }

    // --- API keys (plan 2026-11-07-windows-api-key-sync.md) ---

    const ANTHROPIC: &str = "apiKey.anthropic";

    /// Credential Manager failing every call.
    struct BrokenSecrets;

    impl SecretStore for BrokenSecrets {
        fn get(&self, _: &str, _: &str) -> Result<Option<Vec<u8>>, crate::secrets::SecretError> {
            Err(crate::secrets::SecretError::Backend("locked".into()))
        }
        fn set(&self, _: &str, _: &str, _: &[u8]) -> Result<(), crate::secrets::SecretError> {
            Err(crate::secrets::SecretError::Backend("locked".into()))
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), crate::secrets::SecretError> {
            Err(crate::secrets::SecretError::Backend("locked".into()))
        }
    }

    fn with_key(key: &str) -> MemorySecrets {
        let secrets = MemorySecrets::default();
        secrets.set_string(APP_SERVICE, accounts::ANTHROPIC_API_KEY, key).unwrap();
        secrets
    }

    fn switch_on(f: &mut SettingsFile, at: i64) {
        let before = encoded(f);
        f.set(&api_key_switch(ANTHROPIC), Some(true));
        stamp_changed(f, &before, at);
    }

    fn tombstone(ts: i64) -> SettingsField {
        SettingsField { value: String::new(), updated_at_unix_ms: ts, is_deleted: true }
    }

    fn pull_once(f: SettingsFile, secrets: &dyn SecretStore, pulled: HashMap<String, SettingsField>) -> (SettingsFile, Result<usize, String>, LocalSettings) {
        let settings = Mutex::new(f);
        let mut sent = None;
        let result = run_round(&settings, &FakeStore::default(), secrets, |local| {
            sent = Some(local.clone());
            Ok(Iteration { pushed_count: 0, pulled })
        });
        (settings.into_inner().unwrap(), result, sent.unwrap())
    }

    #[test]
    fn with_the_switch_off_a_key_is_neither_read_nor_changed() {
        let (_d, f) = file();
        let secrets = with_key("sk-test-local");
        let (f, result, sent) = pull_once(f, &secrets, HashMap::from([(ANTHROPIC.to_string(), remote("sk-test-other", 10))]));
        assert_eq!(result, Ok(0));
        let state = sent.api_keys.iter().find(|k| k.field_key == ANTHROPIC).unwrap();
        assert!(!state.sync_enabled);
        assert_eq!(state.key, None, "not read while the switch is off");
        assert_eq!(secrets.get_string(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap().as_deref(), Some("sk-test-local"));
        assert_eq!(f.stamp_of(ANTHROPIC), 0);

        let (_f, result, _) = pull_once(f, &secrets, HashMap::from([(ANTHROPIC.to_string(), tombstone(20))]));
        assert_eq!(result, Ok(0));
        assert!(secrets.get_string(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap().is_some(), "a delete doesn't reach an opted-out PC");
    }

    #[test]
    fn with_the_switch_on_the_key_goes_up_dated_by_the_switch() {
        let (_d, mut f) = file();
        switch_on(&mut f, 500);
        let secrets = with_key("sk-test-local");
        let state = api_key_states(&f, &secrets).into_iter().find(|k| k.field_key == ANTHROPIC).unwrap();
        assert_eq!((state.sync_enabled, state.key.as_deref(), state.key_updated_at_unix_ms, state.toggle_updated_at_unix_ms), (true, Some("sk-test-local"), 0, 500));
        let pushed = api::sync_ops::coordinators::plan_api_key_field(&state).unwrap();
        assert_eq!(pushed, SettingsField { value: "sk-test-local".into(), updated_at_unix_ms: 500, is_deleted: false });
        // The switch itself syncs like any flag.
        assert_eq!(gather(&f).fields[&api_key_switch(ANTHROPIC)], LocalField { value: "true".into(), updated_at_unix_ms: 500 });
    }

    #[test]
    fn a_pulled_key_is_saved_trimmed_and_a_pulled_delete_removes_it() {
        let (d, mut f) = file();
        switch_on(&mut f, 500);
        let secrets = MemorySecrets::default();
        let (f, result, _) = pull_once(f, &secrets, HashMap::from([(ANTHROPIC.to_string(), remote("  sk-test-from-mac \n", 600))]));
        assert_eq!(result, Ok(1));
        assert_eq!(secrets.get_string(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap().as_deref(), Some("sk-test-from-mac"));
        assert_eq!(f.stamp_of(ANTHROPIC), 600);
        let saved = std::fs::read_to_string(d.path().join("settings.json")).unwrap();
        assert!(!saved.contains("sk-test-from-mac"), "the key never goes in settings.json");

        let (f, result, _) = pull_once(f, &secrets, HashMap::from([(ANTHROPIC.to_string(), tombstone(700))]));
        assert_eq!(result, Ok(1));
        assert_eq!(secrets.get(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap(), None);
        assert_eq!(f.stamp_of(ANTHROPIC), 700);
    }

    #[test]
    fn an_older_pulled_key_loses_to_a_newer_local_save() {
        let (_d, mut f) = file();
        switch_on(&mut f, 500);
        f.set_stamp(ANTHROPIC, 900);
        let secrets = with_key("sk-test-local");
        let (_f, result, _) = pull_once(f, &secrets, HashMap::from([(ANTHROPIC.to_string(), remote("sk-test-older", 800))]));
        assert_eq!(result, Ok(0));
        assert_eq!(secrets.get_string(APP_SERVICE, accounts::ANTHROPIC_API_KEY).unwrap().as_deref(), Some("sk-test-local"));
    }

    #[test]
    fn a_key_credential_manager_cant_handle_is_left_out_and_not_stamped() {
        let (_d, mut f) = file();
        switch_on(&mut f, 500);
        f.set_stamp(ANTHROPIC, 400);
        let (f, result, sent) = pull_once(f, &BrokenSecrets, HashMap::from([(ANTHROPIC.to_string(), remote("sk-test-from-mac", 600))]));
        assert!(sent.api_keys.iter().all(|k| k.field_key != ANTHROPIC), "an unreadable key isn't sent as missing, which would delete it elsewhere");
        assert_eq!(result, Ok(0));
        assert_eq!(f.stamp_of(ANTHROPIC), 400, "a failed write isn't stamped, so it comes again");
    }

    #[test]
    fn a_pulled_switch_turns_sync_on_here() {
        let (_d, f) = file();
        let (f, result, _) = pull_once(f, &MemorySecrets::default(), HashMap::from([(api_key_switch(ANTHROPIC), remote("true", 300))]));
        assert_eq!(result, Ok(1));
        assert!(api_key_sync_enabled(&f, ANTHROPIC));
    }
}
