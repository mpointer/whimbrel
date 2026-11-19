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
//!   live in the store, and Whimbrel Cloud's routine fields and this PC's time zone, see
//!   [`cloud_values`]) ever leave this PC. Device-local and Windows-only keys are never gathered, and
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
    // `cloud.*` (the runner's heartbeat, and keys a later version may add), the routines' cloud
    // fields and the account's time zone. An unknown `cloud.*` key is not applied and is never an error.
    if owns_cloud_key(&field.field_key) {
        return apply_cloud(file, field);
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

/// The routines whose on/off, time and model sync (the Mac's `RoutineSyncFields.templates`): the two
/// that can fail over, so a device standing in for the primary runs them the same way, and the ones
/// that may run in the cloud, whose runner reads them. Focus Time writes to the calendar and stays on
/// this PC.
fn routine_name(template: RoutineTemplate) -> Option<&'static str> {
    match template {
        RoutineTemplate::DailyBrief => Some("daily_brief"),
        RoutineTemplate::ReplyPriorities => Some("reply_priorities"),
        RoutineTemplate::EodRecap => Some("eod_recap"),
        RoutineTemplate::WeeklyDigest => Some("weekly_digest"),
        RoutineTemplate::MeetingPrep => Some("meeting_prep"),
        _ => None,
    }
}

/// [`routine_name`]'s templates, by name.
const SYNCED_ROUTINES: [&str; 5] = ["daily_brief", "reply_priorities", "eod_recap", "weekly_digest", "meeting_prep"];

const ROUTINE_FIELDS: [&str; 3] = ["enabled", "schedule", "model_pref"];

/// `routine.<template>.<field>`, or `None` for a routine whose settings stay on this PC.
pub fn routine_key(template: RoutineTemplate, field: &str) -> Option<String> {
    routine_name(template).map(|name| format!("routine.{name}.{field}"))
}

fn parse_routine_key(key: &str) -> Option<(&str, &str)> {
    let (template, field) = key.strip_prefix("routine.")?.split_once('.')?;
    (SYNCED_ROUTINES.contains(&template) && ROUTINE_FIELDS.contains(&field)).then_some((template, field))
}

// --- Whimbrel Cloud: which routines run in the cloud, the runner's heartbeat, the time zone ------------
//
// docs/superpowers/specs/2026-11-15-cloud-routine-runner-design.md, the Mac's `CloudRoutineSettings`. The
// four cloud routines sync `routine.<template>.cloud` ("true"/"false"), `.cloudGrant` (a JSON array of
// core's `routines::grant_for` entries while in the cloud, `[]` otherwise; informational, the runner
// enforces its own copy) and `.readyBy` ("HH:MM", only once chosen). This PC pushes `profile.timeZone`
// when its zone changes, keeps the runner's `cloud.runner.lastRun` without ever pushing it, and ignores
// other `cloud.*` keys. These live in the settings file under their synced names, outside `encoded()`.

/// The templates core lets run in the cloud (`routines::cloud_eligible`); Reply Priorities is not one.
pub const CLOUD_TEMPLATES: [&str; 4] = ["daily_brief", "eod_recap", "weekly_digest", "meeting_prep"];

/// The runner's heartbeat: written only by the runner, never pushed from here.
pub const HEARTBEAT_KEY: &str = api::runner_config::HEARTBEAT_KEY;

/// The account's IANA time zone, which the runner schedules by.
pub const TIME_ZONE_KEY: &str = api::runner_config::TIME_ZONE_KEY;

/// The zone this PC last pushed (on this PC only, never synced).
const PUSHED_TIME_ZONE: &str = "sync.pushedTimeZone";

/// A cloud routine's synced field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudField {
    Cloud,
    CloudGrant,
    ReadyBy,
}

impl CloudField {
    fn as_str(self) -> &'static str {
        match self {
            CloudField::Cloud => "cloud",
            CloudField::CloudGrant => "cloudGrant",
            CloudField::ReadyBy => "readyBy",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "cloud" => Some(CloudField::Cloud),
            "cloudGrant" => Some(CloudField::CloudGrant),
            "readyBy" => Some(CloudField::ReadyBy),
            _ => None,
        }
    }
}

/// `routine.<template>.cloud|cloudGrant|readyBy`.
pub fn cloud_key(template: &str, field: CloudField) -> String {
    format!("routine.{template}.{}", field.as_str())
}

fn parse_cloud_key(key: &str) -> Option<(&str, CloudField)> {
    let (template, field) = key.strip_prefix("routine.")?.split_once('.')?;
    let field = CloudField::parse(field)?;
    CLOUD_TEMPLATES.contains(&template).then_some((template, field))
}

/// Whether a pulled key is Whimbrel Cloud's: any `cloud.*`, the time zone, or a cloud routine field.
pub fn owns_cloud_key(key: &str) -> bool {
    key.starts_with("cloud.") || key == TIME_ZONE_KEY || parse_cloud_key(key).is_some()
}

/// Exactly `HH:MM` on a 24-hour clock, as the Mac and the runner read a ready-by time.
fn exact_hhmm(text: &str) -> bool {
    crate::routines::valid_schedule(text).as_deref() == Some(text)
}

/// Whether `template` is set to run in the cloud (off unless someone turned it on).
pub fn is_cloud(file: &SettingsFile, template: &str) -> bool {
    CLOUD_TEMPLATES.contains(&template) && file.get::<bool>(&cloud_key(template, CloudField::Cloud)).unwrap_or(false)
}

/// The templates set to run in the cloud, for the routine tick.
pub fn cloud_templates(file: &SettingsFile) -> Vec<String> {
    CLOUD_TEMPLATES.iter().filter(|t| is_cloud(file, t)).map(|t| t.to_string()).collect()
}

/// `template`'s ready-by time, once chosen (here or on another device).
pub fn ready_by(file: &SettingsFile, template: &str) -> Option<String> {
    file.get::<String>(&cloud_key(template, CloudField::ReadyBy)).filter(|at| exact_hhmm(at))
}

/// The runner's last round (unix ms), as its heartbeat said; `None` until one is pulled.
pub fn runner_last_run_ms(file: &SettingsFile) -> Option<i64> {
    file.get::<i64>(HEARTBEAT_KEY).filter(|ms| *ms > 0)
}

/// Why "Run in the cloud" was not changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudRefusal {
    /// The template cannot run in the cloud.
    NotCloud,
    /// Turning it on for a routine that is off on this PC: its stamped `enabled=false` would beat another
    /// device's `true` and switch the routine off everywhere, so the runner would never run it.
    RoutineOff,
}

/// A person's choice in the Automations tab; `routine_enabled` is whether this PC's routine row is on. The grant
/// is stamped with it, so the two always travel together. Turning it on also stamps this routine's on/off, time
/// and model: the runner reads them from sync, and a field never edited here goes out at timestamp 0 and loses
/// to any other device's value, so the PC that hands the routine to the cloud stamps its own values now. That
/// is only done for a routine that is on here, so it never stamps `enabled=false` over another device's value:
/// turning it on for a routine that is off is refused. Turning it off is always allowed.
pub fn set_cloud(file: &mut SettingsFile, template: &str, on: bool, routine_enabled: bool, now_ms: i64) -> Result<(), CloudRefusal> {
    if !CLOUD_TEMPLATES.contains(&template) {
        return Err(CloudRefusal::NotCloud);
    }
    if on && !routine_enabled {
        return Err(CloudRefusal::RoutineOff);
    }
    file.set(&cloud_key(template, CloudField::Cloud), Some(on));
    file.set_stamp(&cloud_key(template, CloudField::Cloud), now_ms);
    file.set_stamp(&cloud_key(template, CloudField::CloudGrant), now_ms);
    if on && routine_enabled {
        for field in ROUTINE_FIELDS {
            file.set_stamp(&format!("routine.{template}.{field}"), now_ms);
        }
    }
    Ok(())
}

/// A ready-by time chosen here, in the canonical `HH:MM`. False for a time that does not parse or a template
/// that cannot run in the cloud.
pub fn set_ready_by(file: &mut SettingsFile, template: &str, hhmm: &str, now_ms: i64) -> bool {
    let Some(at) = crate::routines::valid_schedule(hhmm) else { return false };
    if !CLOUD_TEMPLATES.contains(&template) {
        return false;
    }
    file.set(&cloud_key(template, CloudField::ReadyBy), Some(at));
    file.set_stamp(&cloud_key(template, CloudField::ReadyBy), now_ms);
    true
}

/// The row types the runner reads for `template` (core's `routines::grant_for`); empty for an unknown name.
fn grant_of(template: &str) -> &'static [&'static str] {
    serde_json::from_value::<RoutineTemplate>(serde_json::Value::String(template.to_string()))
        .map(|t| ::routines::grant_for(&t))
        .unwrap_or(&[])
}

/// The cloud fields to push (the Mac's `RoutineSyncFields.cloudValues`): each cloud routine's flag and grant,
/// and its ready-by time once chosen. Never the heartbeat.
pub fn cloud_values(file: &SettingsFile) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for template in CLOUD_TEMPLATES {
        let on = is_cloud(file, template);
        out.insert(cloud_key(template, CloudField::Cloud), flag(on));
        let grant: &[&str] = if on { grant_of(template) } else { &[] };
        out.insert(cloud_key(template, CloudField::CloudGrant), serde_json::to_string(grant).unwrap_or_else(|_| "[]".into()));
        if let Some(at) = ready_by(file, template) {
            out.insert(cloud_key(template, CloudField::ReadyBy), at);
        }
    }
    out
}

/// Records `zone` as this PC's. Its timestamp moves only when the zone differs from the one last pushed from
/// here, so two devices in different zones do not take turns overwriting each other: the one whose zone changed
/// last wins. True when something changed (the caller saves).
pub fn note_time_zone(file: &mut SettingsFile, zone: &str, now_ms: i64) -> bool {
    let zone = zone.trim();
    if zone.is_empty() || file.get::<String>(PUSHED_TIME_ZONE).as_deref() == Some(zone) {
        return false;
    }
    file.set(PUSHED_TIME_ZONE, Some(zone));
    file.set_stamp(TIME_ZONE_KEY, now_ms);
    true
}

/// This PC's IANA time zone ("Europe/Paris"), from Windows' own calendar, or `None` if Windows can't say.
/// Only an IANA-looking id is pushed (`Europe/Paris`, `UTC`). A Windows display name ("Eastern Standard
/// Time") or an empty one would make the runner stop with an unknown time zone before its heartbeat, which
/// would stop every cloud routine on the account.
pub fn system_time_zone() -> Option<String> {
    let calendar = windows::Globalization::Calendar::new().ok()?;
    let zone = calendar.GetTimeZone().ok()?.to_string();
    iana_zone(&zone)
}

/// `zone` trimmed, if it looks like an IANA id: it contains a `/`, or is `UTC`, `GMT` or `Etc/UTC`, and has no
/// spaces. `None` otherwise, so nothing is pushed and any earlier value stays.
pub fn iana_zone(zone: &str) -> Option<String> {
    let zone = zone.trim();
    let looks_iana = (zone.contains('/') || matches!(zone, "UTC" | "GMT"))
        && !zone.contains(char::is_whitespace)
        && !zone.starts_with('/')
        && !zone.ends_with('/');
    looks_iana.then(|| zone.to_string())
}

/// Applies one pulled Whimbrel Cloud field. False (not applied, so not stamped) for an unknown `cloud.*` key or
/// a value that does not parse. Another device's time zone and grant are accepted and stamped only: this PC
/// pushes its own zone when it changes, and the grant is core's list for the template.
fn apply_cloud(file: &mut SettingsFile, field: &PulledField) -> bool {
    let value = field.value.as_str();
    if field.field_key == HEARTBEAT_KEY {
        return match value.trim().parse::<i64>() {
            Ok(ms) if ms > 0 => {
                file.set(HEARTBEAT_KEY, Some(ms));
                true
            }
            _ => false,
        };
    }
    if field.field_key == TIME_ZONE_KEY {
        return !value.trim().is_empty();
    }
    match parse_cloud_key(&field.field_key) {
        Some((template, CloudField::Cloud)) => match parse_flag(value) {
            Some(on) => {
                file.set(&cloud_key(template, CloudField::Cloud), Some(on));
                true
            }
            None => false,
        },
        Some((template, CloudField::ReadyBy)) if exact_hhmm(value) => {
            file.set(&cloud_key(template, CloudField::ReadyBy), Some(value));
            true
        }
        Some((_, CloudField::ReadyBy)) => false,
        Some((_, CloudField::CloudGrant)) => serde_json::from_str::<Vec<String>>(value).is_ok(),
        None => false,
    }
}

/// What the routine tick needs about the cloud (core's `CloudPolicy`): the templates set to run there, their
/// ready-by times (the stored ones, as the runner reads them, never a display default) and whether the
/// runner's heartbeat is fresh at `now_unix_ms`.
pub fn cloud_policy(file: &SettingsFile, now_unix_ms: i64) -> api::routine_runs::CloudPolicy {
    let ready: HashMap<String, String> = CLOUD_TEMPLATES.iter().filter_map(|t| ready_by(file, t).map(|at| (t.to_string(), at))).collect();
    let runner_fresh = runner_last_run_ms(file).is_some_and(|ms| {
        let heartbeat = SettingsField { value: ms.to_string(), updated_at_unix_ms: ms, is_deleted: false };
        api::runner_config::runner_fresh(&HashMap::from([(HEARTBEAT_KEY.to_string(), heartbeat)]), now_unix_ms, ::routines::FAILOVER_GRACE_MS)
    });
    api::routine_runs::CloudPolicy { cloud_templates: cloud_templates(file), runner_fresh, ready_by: ready }
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
/// Mac), the cloud routine fields and this PC's time zone, the hidden marks and the API keys.
pub fn gather_all(file: &SettingsFile, store: &StoreLocal, secrets: &dyn SecretStore) -> LocalSettings {
    let mut local = gather(file);
    for (key, value) in routine_values(&store.routines).into_iter().chain(cloud_values(file)) {
        let updated_at_unix_ms = file.stamp_of(&key);
        local.fields.insert(key, LocalField { value, updated_at_unix_ms });
    }
    // Never `cloud.runner.lastRun`: only the runner writes its heartbeat.
    if let Some(zone) = file.get::<String>(PUSHED_TIME_ZONE) {
        local.fields.insert(TIME_ZONE_KEY.to_string(), LocalField { value: zone, updated_at_unix_ms: file.stamp_of(TIME_ZONE_KEY) });
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
    run_round_in(settings, store, secrets, system_time_zone().as_deref(), iterate)
}

/// [`run_round`] with this PC's time zone given (`None`: Windows couldn't say, so the last one pushed stays).
pub fn run_round_in<F>(settings: &Mutex<SettingsFile>, store: &dyn StoreSide, secrets: &dyn SecretStore, zone: Option<&str>, iterate: F) -> Result<usize, String>
where
    F: FnOnce(&LocalSettings) -> Result<Iteration, String>,
{
    let store_local = store.gather()?;
    let local = {
        let mut file = settings.lock().unwrap();
        if zone.and_then(iana_zone).is_some_and(|z| note_time_zone(&mut file, &z, now_ms())) {
            if let Err(e) = file.save() {
                tracing::warn!(error = %e, "could not save this PC's time zone for sync");
            }
        }
        gather_all(&file, &store_local, secrets)
    };
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
        assert!(!apply_field(&mut f, &pulled(keys::CAPTURE_RESUME_AT, "1800000000000", 5)), "a timed pause never applies from another device");
        assert_eq!(f.get::<String>("prompts.someFutureSurface"), None, "an unknown key isn't written into the file");
    }

    #[test]
    fn device_local_keys_are_never_gathered() {
        let (_d, mut f) = file();
        f.set(keys::CAPTURE_ENABLED, Some(false));
        f.set_capture_resume_at(Some(1_800_000_000_000));
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
            keys::CAPTURE_RESUME_AT,
            keys::MEETING_BAR_POSITION,
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
            // Plus the cloud routine fields and this PC's time zone, which live outside `synced_keys`.
            assert_eq!(local.fields.keys().filter(|k| !owns_cloud_key(k)).count(), synced_keys().len());
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
    fn the_cloud_routines_and_reply_priorities_push_their_fields_but_not_focus_time() {
        let mut brief = routine(1, RoutineTemplate::DailyBrief);
        brief.enabled = true;
        brief.schedule_hhmm = "07:30".into();
        brief.model_pref = "anthropic/claude-sonnet-5".into();
        let mut recap = routine(3, RoutineTemplate::EodRecap);
        recap.schedule_hhmm = "17:30".into();
        let values = routine_values(&[brief, routine(2, RoutineTemplate::FocusTime), recap]);
        assert_eq!(
            values,
            BTreeMap::from([
                ("routine.daily_brief.enabled".to_string(), "true".to_string()),
                ("routine.daily_brief.model_pref".to_string(), "anthropic/claude-sonnet-5".to_string()),
                ("routine.daily_brief.schedule".to_string(), "07:30".to_string()),
                ("routine.eod_recap.enabled".to_string(), "false".to_string()),
                ("routine.eod_recap.model_pref".to_string(), "default".to_string()),
                ("routine.eod_recap.schedule".to_string(), "17:30".to_string()),
            ]),
            "no Reply Priorities row, so nothing for it; focus time stays on this PC"
        );
        let all = [
            routine(1, RoutineTemplate::DailyBrief),
            routine(2, RoutineTemplate::FocusTime),
            routine(3, RoutineTemplate::EodRecap),
            routine(4, RoutineTemplate::WeeklyDigest),
            routine(5, RoutineTemplate::ReplyPriorities),
            routine(6, RoutineTemplate::MeetingPrep),
        ];
        let templates: std::collections::BTreeSet<String> =
            routine_values(&all).keys().map(|k| k.split('.').nth(1).unwrap().to_string()).collect();
        assert_eq!(templates.into_iter().collect::<Vec<_>>(), ["daily_brief", "eod_recap", "meeting_prep", "reply_priorities", "weekly_digest"]);
    }

    #[test]
    fn the_new_routine_fields_parse_and_focus_time_and_unknown_ones_do_not() {
        let rows = [routine(4, RoutineTemplate::EodRecap), routine(5, RoutineTemplate::WeeklyDigest), routine(6, RoutineTemplate::MeetingPrep)];
        assert_eq!(routine_change("routine.eod_recap.enabled", "true", &rows), Some(RoutineChange::Enabled { id: 4, on: true }));
        assert_eq!(routine_change("routine.weekly_digest.schedule", "16:00", &rows), Some(RoutineChange::Schedule { id: 5, hhmm: "16:00".into() }));
        assert_eq!(routine_change("routine.meeting_prep.enabled", "false", &rows), Some(RoutineChange::Enabled { id: 6, on: false }));
        assert_eq!(routine_change("routine.focus_time.enabled", "true", &rows), None, "focus time stays on this PC");
        assert_eq!(routine_change("routine.someday_digest.enabled", "true", &rows), None, "an unknown template key is ignored");
        assert_eq!(routine_change("routine.eod_recap.cloud", "true", &rows), None, "a cloud field is not a routine-row field");
        assert_eq!(routine_key(RoutineTemplate::WeeklyDigest, "schedule").as_deref(), Some("routine.weekly_digest.schedule"));
        assert_eq!(routine_key(RoutineTemplate::FocusTime, "schedule"), None);
    }

    // --- Whimbrel Cloud ------------------------------------------------------------------------------

    #[test]
    fn cloud_cloud_grant_and_ready_by_round_trip_for_eod_recap_and_meeting_prep() {
        let (_d, mut a) = file();
        assert_eq!(set_cloud(&mut a, "eod_recap", true, true, 100), Ok(()));
        assert_eq!(set_cloud(&mut a, "meeting_prep", true, true, 100), Ok(()));
        assert!(set_ready_by(&mut a, "eod_recap", "18:45", 100));
        let pushed = cloud_values(&a);
        assert_eq!(pushed["routine.eod_recap.cloud"], "true");
        assert_eq!(pushed["routine.meeting_prep.cloud"], "true");
        assert_eq!(pushed["routine.daily_brief.cloud"], "false");
        assert_eq!(pushed["routine.daily_brief.cloudGrant"], "[]", "no grant while off");
        let grant: Vec<String> = serde_json::from_str(&pushed["routine.eod_recap.cloudGrant"]).unwrap();
        assert_eq!(grant, ::routines::grant_for(&RoutineTemplate::EodRecap).to_vec(), "core's grant while on");
        assert_eq!(pushed["routine.eod_recap.readyBy"], "18:45");
        assert!(!pushed.contains_key("routine.meeting_prep.readyBy"), "pushed only once chosen");

        let (_d2, mut b) = file();
        for (key, value) in &pushed {
            assert!(apply_field(&mut b, &pulled(key, value, 5)), "{key} should apply");
        }
        assert!(is_cloud(&b, "eod_recap") && is_cloud(&b, "meeting_prep") && !is_cloud(&b, "daily_brief"));
        assert_eq!(ready_by(&b, "eod_recap").as_deref(), Some("18:45"));
        assert_eq!(cloud_values(&b), pushed);
    }

    #[test]
    fn bad_cloud_values_and_non_cloud_templates_are_refused() {
        let (_d, mut f) = file();
        for (key, bad) in [
            ("routine.eod_recap.cloud", "yes"),
            ("routine.eod_recap.readyBy", "7:00"),
            ("routine.eod_recap.readyBy", "25:00"),
            ("routine.eod_recap.readyBy", " 07:00"),
            ("routine.eod_recap.cloudGrant", "meeting"),
            ("routine.reply_priorities.cloud", "true"),
            ("routine.focus_time.cloud", "true"),
        ] {
            assert!(!apply_field(&mut f, &pulled(key, bad, 5)), "{key} = {bad:?} should be refused");
        }
        assert!(cloud_templates(&f).is_empty());
        assert_eq!(set_cloud(&mut f, "reply_priorities", true, true, 9), Err(CloudRefusal::NotCloud), "Reply Priorities never runs in the cloud");
        assert!(!set_ready_by(&mut f, "daily_brief", "noon", 9));
        assert!(!set_ready_by(&mut f, "focus_time", "09:00", 9));
        assert_eq!(f.stamp_of("routine.reply_priorities.cloud"), 0);
        let mut tomb = pulled("routine.daily_brief.cloud", "true", 5);
        tomb.is_deleted = true;
        assert!(!apply_field(&mut f, &tomb), "a deleted field is ignored");
    }

    #[test]
    fn unknown_cloud_pulled_keys_are_tolerated_and_the_runners_last_run_is_kept_but_never_pushed() {
        let (_d, f) = file();
        let pulled_fields = HashMap::from([
            (HEARTBEAT_KEY.to_string(), remote("1793789914162", 1_793_789_914_162)),
            ("cloud.someFutureKey".to_string(), remote("whatever", 20)),
            ("routine.daily_brief.cloudGrant".to_string(), remote(r#"["meeting"]"#, 21)),
            (TIME_ZONE_KEY.to_string(), remote("America/Los_Angeles", 22)),
        ]);
        // No zone known here yet, so the account's (newer) one is taken.
        let (f, result, sent) = pull_once_in(f, None, pulled_fields);
        assert_eq!(result, Ok(3), "the heartbeat, the grant and the zone; the unknown cloud key is not an error");
        assert_eq!(runner_last_run_ms(&f), Some(1_793_789_914_162));
        assert_eq!(f.stamp_of("cloud.someFutureKey"), 0, "an unknown key is not stamped");
        assert_eq!(f.get::<String>("cloud.someFutureKey"), None, "nor written into the file");
        assert_eq!(f.stamp_of(TIME_ZONE_KEY), 22, "another device's newer zone is accepted");
        assert!(!sent.fields.contains_key(HEARTBEAT_KEY), "only the runner writes its heartbeat");
        assert!(!sent.fields.contains_key(PUSHED_TIME_ZONE), "the last pushed zone stays on this PC");
        let (_f, _, sent) = pull_once_in(f, Some("America/New_York"), HashMap::new());
        assert!(!sent.fields.contains_key(HEARTBEAT_KEY), "not even after one was pulled");
        assert_eq!(sent.fields[TIME_ZONE_KEY].value, "America/New_York", "this PC pushes its own zone");
    }

    #[test]
    fn the_time_zone_is_stamped_only_when_this_pcs_zone_changes() {
        let (_d, mut f) = file();
        assert!(note_time_zone(&mut f, "Europe/Paris", 100));
        assert_eq!(f.stamp_of(TIME_ZONE_KEY), 100);
        assert!(!note_time_zone(&mut f, "Europe/Paris", 200), "the same zone again");
        assert_eq!(f.stamp_of(TIME_ZONE_KEY), 100, "so two devices in different zones don't take turns");
        // Another device's newer zone lands here: stamped with its time, not pushed over.
        assert!(apply_field(&mut f, &pulled(TIME_ZONE_KEY, "Asia/Tokyo", 300)));
        f.set_stamp(TIME_ZONE_KEY, 300);
        assert!(!note_time_zone(&mut f, "Europe/Paris", 400), "this PC did not move");
        assert_eq!(f.stamp_of(TIME_ZONE_KEY), 300);
        assert!(note_time_zone(&mut f, "America/New_York", 500), "this PC moved");
        assert_eq!(f.stamp_of(TIME_ZONE_KEY), 500);
        assert!(!note_time_zone(&mut f, "  ", 600), "an empty zone is not one");
        let local = gather_all(&f, &StoreLocal::default(), &MemorySecrets::default());
        assert_eq!(local.fields[TIME_ZONE_KEY], LocalField { value: "America/New_York".into(), updated_at_unix_ms: 500 });
    }

    #[test]
    fn existing_settings_go_out_unstamped_so_an_upgrade_flips_nothing() {
        let (_d, f) = file();
        let store = StoreLocal { routines: vec![routine(1, RoutineTemplate::EodRecap), routine(2, RoutineTemplate::MeetingPrep)], ..StoreLocal::default() };
        let local = gather_all(&f, &store, &MemorySecrets::default());
        for (key, field) in &local.fields {
            if key.starts_with("routine.") {
                assert_eq!(field.updated_at_unix_ms, 0, "{key} was never edited here, so the account's value wins");
            }
        }
        assert_eq!(local.fields["routine.eod_recap.cloud"].value, "false");
        assert!(!local.fields.contains_key(TIME_ZONE_KEY), "no zone known yet");
        // The account's older or equal copy of a never-edited field doesn't apply either.
        let (_d2, f2) = file();
        let (f2, result, _) = pull_once_in(f2, None, HashMap::from([("routine.eod_recap.cloud".to_string(), remote("true", 0))]));
        assert_eq!(result, Ok(0));
        assert!(!is_cloud(&f2, "eod_recap"));
    }

    #[test]
    fn turning_cloud_on_stamps_the_routines_own_fields_for_the_runner_and_off_does_not() {
        let (_d, mut f) = file();
        assert_eq!(set_cloud(&mut f, "daily_brief", true, true, 1_000), Ok(()));
        for field in ["cloud", "cloudGrant", "enabled", "schedule", "model_pref"] {
            assert_eq!(f.stamp_of(&format!("routine.daily_brief.{field}")), 1_000, "{field}");
        }
        assert_eq!(f.stamp_of("routine.eod_recap.enabled"), 0, "only that routine's");
        assert_eq!(set_cloud(&mut f, "daily_brief", false, true, 2_000), Ok(()));
        assert_eq!(f.stamp_of("routine.daily_brief.cloud"), 2_000);
        assert_eq!(f.stamp_of("routine.daily_brief.cloudGrant"), 2_000, "the grant travels with the flag");
        assert_eq!(f.stamp_of("routine.daily_brief.enabled"), 1_000, "turning it off stamps nothing else");
    }

    #[test]
    fn the_cloud_policy_carries_the_flags_the_stored_ready_by_times_and_the_heartbeat() {
        let (_d, mut f) = file();
        let now = 1_800_000_000_000;
        assert_eq!(cloud_policy(&f, now), api::routine_runs::CloudPolicy::default(), "nothing flagged: the old tick");
        set_cloud(&mut f, "daily_brief", true, true, 1).unwrap();
        set_ready_by(&mut f, "daily_brief", "07:00", 1);
        set_ready_by(&mut f, "eod_recap", "19:00", 1);
        assert!(apply_field(&mut f, &pulled(HEARTBEAT_KEY, &(now - 60_000).to_string(), now - 60_000)));
        let policy = cloud_policy(&f, now);
        assert_eq!(policy.cloud_templates, ["daily_brief"]);
        assert!(policy.runner_fresh, "a heartbeat a minute old");
        assert_eq!(policy.ready_by, HashMap::from([("daily_brief".to_string(), "07:00".to_string()), ("eod_recap".to_string(), "19:00".to_string())]));
        assert!(!cloud_policy(&f, now + ::routines::FAILOVER_GRACE_MS).runner_fresh, "stale after the grace");
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

    /// [`pull_once`] with this PC's time zone given rather than read from Windows.
    fn pull_once_in(f: SettingsFile, zone: Option<&str>, pulled: HashMap<String, SettingsField>) -> (SettingsFile, Result<usize, String>, LocalSettings) {
        let settings = Mutex::new(f);
        let mut sent = None;
        let result = run_round_in(&settings, &FakeStore::default(), &MemorySecrets::default(), zone, |local| {
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

    #[test]
    fn cloud_cannot_be_turned_on_for_a_routine_that_is_off_here_and_stamps_nothing() {
        let (_d, mut f) = file();
        assert_eq!(set_cloud(&mut f, "eod_recap", true, false, 1_000), Err(CloudRefusal::RoutineOff));
        assert!(!is_cloud(&f, "eod_recap"));
        for field in ["cloud", "cloudGrant", "enabled", "schedule", "model_pref"] {
            assert_eq!(f.stamp_of(&format!("routine.eod_recap.{field}")), 0, "{field}: enabled=false never beats another device's value");
        }
    }

    #[test]
    fn turning_cloud_off_is_always_allowed_and_never_stamps_enabled() {
        let (_d, mut f) = file();
        set_cloud(&mut f, "eod_recap", true, true, 1_000).unwrap();
        assert_eq!(set_cloud(&mut f, "eod_recap", false, false, 2_000), Ok(()), "the routine was switched off since");
        assert!(!is_cloud(&f, "eod_recap"));
        assert_eq!(f.stamp_of("routine.eod_recap.cloud"), 2_000);
        assert_eq!(f.stamp_of("routine.eod_recap.enabled"), 1_000, "not restamped by turning cloud off");
    }

    #[test]
    fn only_an_iana_looking_time_zone_is_pushed() {
        for good in ["America/New_York", " Europe/Paris ", "UTC", "GMT", "Etc/UTC", "America/Argentina/Buenos_Aires"] {
            assert_eq!(iana_zone(good).as_deref(), Some(good.trim()), "{good:?}");
        }
        for bad in ["", "  ", "Eastern Standard Time", "Pacific Standard Time", "W. Europe Standard Time", "EST", "/", "America/", "Etc/ UTC"] {
            assert_eq!(iana_zone(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_windows_display_name_is_never_pushed_and_the_earlier_zone_stays() {
        let (_d, mut f) = file();
        assert!(note_time_zone(&mut f, "America/New_York", 100));
        let (f, _, sent) = pull_once_in(f, Some("Eastern Standard Time"), HashMap::new());
        assert_eq!(sent.fields[TIME_ZONE_KEY], LocalField { value: "America/New_York".into(), updated_at_unix_ms: 100 });
        let (_d2, fresh) = file();
        let (_f, _, sent) = pull_once_in(fresh, Some("Eastern Standard Time"), HashMap::new());
        assert!(!sent.fields.contains_key(TIME_ZONE_KEY), "nothing pushed rather than a zone the runner cannot read");
        drop(f);
    }
}
