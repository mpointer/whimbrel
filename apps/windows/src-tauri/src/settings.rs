//! Settings in `%LOCALAPPDATA%\Whimbrel\data\settings.json` (plan Decision 7).
//!
//! The file is a flat JSON object keyed by the settings-sync field keys the Mac uses
//! (`routing.preferredProvider`, …), so settings sync can slot in later without a
//! mapping table. Keys this build doesn't know are preserved on save. Each typed
//! getter degrades a missing or wrong-typed field to its default on its own, the same
//! rule `api::RoutingConfig::from_json` follows: one bad field never resets the rest.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::profile::{Fact, Profile};
use crate::secrets::{accounts, SecretError, SecretStore, APP_SERVICE};

/// The settings-sync field keys (Mac: `SettingsSyncCoordinator.swift`).
pub mod keys {
    pub const PREFERRED_PROVIDER: &str = "routing.preferredProvider";
    pub const OLLAMA_MODEL: &str = "routing.ollamaDefaultModel";
    pub const ANTHROPIC_MODEL: &str = "routing.anthropicDefaultModel";
    pub const OPENAI_MODEL: &str = "routing.openaiDefaultModel";
    pub const GOOGLE_MODEL: &str = "routing.googleDefaultModel";
    pub const OPENROUTER_MODEL: &str = "routing.openrouterDefaultModel";
    pub const USER_DAILY_CAP_CENTS: &str = "routing.userDailyCapCents";
    pub const GLOBAL_DAILY_CAP_CENTS: &str = "routing.globalDailyCapCents";
    pub const LOCAL_ONLY_MODE: &str = "routing.localOnlyMode";
    /// Cheap-first routing and the local-model switch (the Mac's keys;
    /// docs/superpowers/specs/2026-10-29-cheap-first-routing-and-key-on-ramp-design.md).
    pub const CHEAP_FIRST: &str = "routing.cheapFirst";
    pub const USE_LOCAL_MODEL: &str = "routing.useLocalModel";
    /// Device-local: OpenRouter prices cached from the catalogue when the key was last
    /// checked, `{model: {in, out}}` in cents per 1K tokens, the shape core parses.
    pub const OPENROUTER_PRICES: &str = "routing.openrouterPrices";
    pub const SUMMARY_CUSTOM_PROMPT: &str = "summary.customPromptInstructions";
    pub const EXCLUDED_BUNDLE_IDS: &str = "captureExclusions.excludedBundleIds";
    /// Device-local, not synced (the Mac keeps its capture toggle local too).
    pub const CAPTURE_ENABLED: &str = "capture.enabled";
    /// Device-local. The Mac's equivalent (`showScreenCaptures`) defaults off; on
    /// Windows screen captures show in the Library by default (plan Task 4).
    pub const LIBRARY_SHOW_CAPTURES: &str = "library.showScreenCaptures";
    /// Device-local, Windows-only: apps whose episodes the Episodes list hides.
    pub const LIBRARY_HIDDEN_EPISODE_APPS: &str = "library.hiddenEpisodeApps";
    /// Device-local, as on the Mac (plain UserDefaults there, not settings-sync fields).
    pub const HEURISTIC_DETECTION: &str = "meetings.heuristicDetectionEnabled";
    pub const MEETING_AUTO_START: &str = "meetings.autoStart";
    pub const MEETING_LEAD_MINUTES: &str = "meetings.leadMinutes";
    /// Device-local, as on the Mac (`MeetingPrepSettings`): how long before a meeting its brief is made.
    pub const MEETING_PREP_LEAD_MINUTES: &str = "meetingPrep.leadMinutes";
    /// Device-local, as on the Mac (`SnapshotSettings`): whether recorded meetings save their
    /// slides, on unless turned off, and the folder they go in (absent: Pictures\Whimbrel Snapshots).
    pub const MEETING_SNAPSHOTS_ENABLED: &str = "meetingSnapshots.enabled";
    pub const MEETING_SNAPSHOTS_ROOT: &str = "meetingSnapshots.root";
    /// Device-local, as on the Mac (UserDefaults): whether the MCP server runs. Off unless the user turns it on.
    pub const MCP_SERVER_ENABLED: &str = "mcpServer.enabled";
    pub const RETENTION_CAPTURE_DAYS: &str = "retention.captureItemDays";
    pub const RETENTION_CAPTURE_FOREVER: &str = "retention.captureItemKeepForever";
    pub const RETENTION_MEETING_DAYS: &str = "retention.meetingDays";
    pub const RETENTION_MEETING_FOREVER: &str = "retention.meetingKeepForever";
    pub const RETENTION_CONVERSATION_DAYS: &str = "retention.conversationDays";
    pub const RETENTION_CONVERSATION_FOREVER: &str = "retention.conversationKeepForever";
    /// Windows-only (the Mac sweeps on first launch without asking): set once the user
    /// has seen the first-run notice and let the sweep proceed.
    pub const RETENTION_ACKNOWLEDGED: &str = "retention.noticeAcknowledged";
    /// Device-local: start at login (Task 10). Unset until the first release launch.
    pub const GENERAL_AUTOSTART: &str = "general.autostart";
    /// Device-local: calendar events already prompted, id -> event end (unix ms), kept
    /// until each ends so a relaunch mid-meeting doesn't prompt or record it again.
    pub const MEETINGS_PROMPTED: &str = "meetings.promptedEvents";
    /// The Profile (docs/superpowers/specs/2026-10-06-profile-design.md), under the Mac's settings-sync
    /// field keys. Synced on the Mac; on Windows it stays on this PC until settings sync exists here.
    pub const PROFILE_NAME: &str = "profile.name";
    pub const PROFILE_ROLE: &str = "profile.role";
    pub const PROFILE_ORGANIZATION: &str = "profile.organization";
    pub const PROFILE_ABOUT: &str = "profile.about";
    /// A JSON array of `{id, text, sourceMeetingId}`.
    pub const PROFILE_FACTS: &str = "profile.facts";
    pub const PROFILE_ENABLED: &str = "profile.enabled";
    pub const PROFILE_SUGGEST_ENABLED: &str = "profile.suggest_enabled";
    /// Which device runs routines: that device's public key, or absent for "any device". The Mac's
    /// settings-sync field key; on Windows it stays on this PC until a settings-sync coordinator exists.
    pub const ROUTINES_PRIMARY_DEVICE: &str = "routines.primaryDeviceId";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Ollama,
    Anthropic,
    Openai,
    Google,
    /// One key, any model on OpenRouter's catalogue, free ones included.
    Openrouter,
}

impl Provider {
    /// The string `api::RoutingConfig::preferred_provider` and the Mac store.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Ollama => "ollama",
            Provider::Anthropic => "anthropic",
            Provider::Openai => "openai",
            Provider::Google => "google",
            Provider::Openrouter => "openrouter",
        }
    }

    pub const HOSTED: [Provider; 4] = [Provider::Anthropic, Provider::Openai, Provider::Google, Provider::Openrouter];
}

/// Provider choice, default models and governance, as the Settings UI edits them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingSettings {
    pub preferred_provider: Provider,
    pub ollama_model: String,
    pub anthropic_model: String,
    pub openai_model: String,
    pub google_model: String,
    /// The OpenRouter pick, also the model light tasks use there. Empty means none yet:
    /// core then leaves OpenRouter out of every chain, because their catalogue changes
    /// weekly and there is no fallback model to substitute.
    pub openrouter_model: String,
    /// `None` = the built-in default cap; `Some(0)` = no cap (Mac semantics).
    pub user_daily_cap_cents: Option<i64>,
    pub global_daily_cap_cents: Option<i64>,
    /// FR-CHAT-5 local-only mode: Ollama only, whatever else is configured.
    pub local_only: bool,
    /// Light tasks (titles, topic names, profile suggestions) use the cheapest model the
    /// user has. On by default, matching core.
    pub cheap_first: bool,
    /// Whether Ollama is a link in the chain at all; ignored in local-only mode. On by default.
    pub use_local_model: bool,
    /// Cents per 1K input and output per OpenRouter model id, cached from the catalogue by
    /// the key check. Device-local and never part of the DTO: the UI never edits it.
    #[serde(skip)]
    #[schemars(skip)]
    pub openrouter_prices: BTreeMap<String, (f64, f64)>,
}

impl Default for RoutingSettings {
    /// The Mac's defaults (`RoutingSettings.swift`).
    fn default() -> Self {
        RoutingSettings {
            preferred_provider: Provider::Ollama,
            ollama_model: "gemma4:12b".into(),
            anthropic_model: "claude-haiku-4-5".into(),
            openai_model: "gpt-4o-mini".into(),
            google_model: "gemini-2.5-flash".into(),
            openrouter_model: String::new(),
            user_daily_cap_cents: None,
            global_daily_cap_cents: None,
            local_only: false,
            cheap_first: true,
            use_local_model: true,
            openrouter_prices: BTreeMap::new(),
        }
    }
}

impl RoutingSettings {
    /// The model the preferred provider is set to use.
    pub fn preferred_model(&self) -> &str {
        match self.preferred_provider {
            Provider::Ollama => &self.ollama_model,
            Provider::Anthropic => &self.anthropic_model,
            Provider::Openai => &self.openai_model,
            Provider::Google => &self.google_model,
            Provider::Openrouter => &self.openrouter_model,
        }
    }

    /// `provider`'s default model as the Settings UI holds it.
    pub fn model_for(&self, provider: Provider) -> &str {
        match provider {
            Provider::Ollama => &self.ollama_model,
            Provider::Anthropic => &self.anthropic_model,
            Provider::Openai => &self.openai_model,
            Provider::Google => &self.google_model,
            Provider::Openrouter => &self.openrouter_model,
        }
    }

    /// One call's `RoutingConfig`. Built fresh per call, as the Mac does, because the
    /// instructions differ by surface (Ask, digest, …) and keys can change at any time.
    /// Never persisted: it carries API keys.
    pub fn routing_config(&self, keys: &ApiKeys, instructions: &Instructions) -> api::RoutingConfig {
        api::RoutingConfig {
            anthropic_key: keys.anthropic.clone(),
            openai_key: keys.openai.clone(),
            google_key: keys.google.clone(),
            // The calendar title check is Mac-only for now; Windows never sends a TypeSafe key.
            typesafe_key: None,
            user_daily_cap_cents: self.user_daily_cap_cents,
            global_daily_cap_cents: self.global_daily_cap_cents,
            sensitive: self.local_only,
            preferred_provider: self.preferred_provider.as_str().to_string(),
            preferred_model: self.preferred_model().to_string(),
            custom_instructions: instructions.custom.trim().to_string(),
            daily_digest_instructions: instructions.daily_digest.trim().to_string(),
            weekly_digest_instructions: instructions.weekly_digest.trim().to_string(),
            profile: instructions.profile.clone(),
            cheap_first: self.cheap_first,
            use_local_model: self.use_local_model,
            openrouter_key: keys.openrouter.clone(),
            openrouter_model: self.openrouter_model.clone(),
            openrouter_prices: self.openrouter_prices.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        }
    }
}

/// The LLM call sites that carry their own free-text instructions — the Mac's
/// `PromptSurface` (`PromptLibrarySettings.swift`). Stored under `prompts.<raw value>`,
/// the Mac's settings-sync field key; empty means "no extra instructions".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PromptSurface {
    Ask,
    MeetingSummary,
    DailyDigest,
    WeeklyDigest,
    EpisodeSummary,
    ThreadSummary,
}

impl PromptSurface {
    pub const ALL: [PromptSurface; 6] = [
        PromptSurface::Ask,
        PromptSurface::MeetingSummary,
        PromptSurface::DailyDigest,
        PromptSurface::WeeklyDigest,
        PromptSurface::EpisodeSummary,
        PromptSurface::ThreadSummary,
    ];

    /// The Mac's `rawValue`.
    pub fn raw(self) -> &'static str {
        match self {
            PromptSurface::Ask => "ask",
            PromptSurface::MeetingSummary => "meetingSummary",
            PromptSurface::DailyDigest => "dailyDigest",
            PromptSurface::WeeklyDigest => "weeklyDigest",
            PromptSurface::EpisodeSummary => "episodeSummary",
            PromptSurface::ThreadSummary => "threadSummary",
        }
    }

    pub fn key(self) -> String {
        format!("prompts.{}", self.raw())
    }
}

/// Per-surface prompt instructions for one call.
#[derive(Debug, Clone, Default)]
pub struct Instructions {
    pub custom: String,
    pub daily_digest: String,
    pub weekly_digest: String,
    /// The composed Profile block, empty when the Profile is off or blank. Core trims, caps and
    /// redacts it before it reaches a prompt.
    pub profile: String,
}

/// Hosted-provider API keys, read fresh from Credential Manager for each call.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ApiKeys {
    pub anthropic: Option<String>,
    pub openai: Option<String>,
    pub google: Option<String>,
    pub openrouter: Option<String>,
}

impl ApiKeys {
    /// Whether a key is saved for `provider` (Ollama needs none).
    pub fn has(&self, provider: Provider) -> bool {
        let key = match provider {
            Provider::Ollama => return true,
            Provider::Anthropic => &self.anthropic,
            Provider::Openai => &self.openai,
            Provider::Google => &self.google,
            Provider::Openrouter => &self.openrouter,
        };
        key.as_deref().is_some_and(|k| !k.is_empty())
    }
}

impl std::fmt::Debug for ApiKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeys")
            .field("anthropic", &self.anthropic.is_some())
            .field("openai", &self.openai.is_some())
            .field("google", &self.google.is_some())
            .finish()
    }
}

impl ApiKeys {
    pub fn load(secrets: &dyn SecretStore) -> Result<Self, SecretError> {
        Ok(ApiKeys {
            anthropic: secrets.get_string(APP_SERVICE, accounts::ANTHROPIC_API_KEY)?,
            openai: secrets.get_string(APP_SERVICE, accounts::OPENAI_API_KEY)?,
            google: secrets.get_string(APP_SERVICE, accounts::GOOGLE_API_KEY)?,
            openrouter: secrets.get_string(APP_SERVICE, accounts::OPENROUTER_API_KEY)?,
        })
    }
}

/// Pre-#180 names of the device-local keys, copied forward on load. The old keys stay so
/// an older build still reads its settings.
const LEGACY_KEYS: [(&str, &str); 10] = [
    ("littlebird.meetingHeuristicDetectionEnabled", keys::HEURISTIC_DETECTION),
    ("littlebird.meetingAutoStart", keys::MEETING_AUTO_START),
    ("littlebird.meetingLeadMinutes", keys::MEETING_LEAD_MINUTES),
    ("littlebird.retentionCaptureItemDays", keys::RETENTION_CAPTURE_DAYS),
    ("littlebird.retentionCaptureItemKeepForever", keys::RETENTION_CAPTURE_FOREVER),
    ("littlebird.retentionMeetingDays", keys::RETENTION_MEETING_DAYS),
    ("littlebird.retentionMeetingKeepForever", keys::RETENTION_MEETING_FOREVER),
    ("littlebird.retentionConversationDays", keys::RETENTION_CONVERSATION_DAYS),
    ("littlebird.retentionConversationKeepForever", keys::RETENTION_CONVERSATION_FOREVER),
    ("littlebird.retentionNoticeAcknowledged", keys::RETENTION_ACKNOWLEDGED),
];

/// The settings file.
#[derive(Debug, Clone)]
pub struct SettingsFile {
    path: PathBuf,
    fields: BTreeMap<String, Value>,
}

impl SettingsFile {
    /// Loads `path`. A missing file is empty settings. A corrupt one is renamed to
    /// `settings.json.corrupt` (kept for the user, never silently overwritten) and
    /// replaced by empty settings.
    pub fn load(path: &Path) -> Self {
        let mut fields = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<BTreeMap<String, Value>>(&text) {
                Ok(fields) => fields,
                Err(e) => {
                    let aside = path.with_extension("json.corrupt");
                    tracing::warn!(error = %e, aside = %aside.display(), "settings file is corrupt; starting from defaults");
                    let _ = std::fs::rename(path, &aside);
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                tracing::warn!(error = %e, "could not read the settings file; starting from defaults");
                BTreeMap::new()
            }
        };
        for (old, new) in LEGACY_KEYS {
            if let (false, Some(v)) = (fields.contains_key(new), fields.get(old).cloned()) {
                fields.insert(new.to_string(), v);
            }
        }
        SettingsFile { path: path.to_path_buf(), fields }
    }

    /// Writes atomically: a temp file next to the target, then a rename over it.
    pub fn save(&self) -> io::Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&self.fields).map_err(io::Error::other)?;
        std::fs::write(&tmp, text + "\n")?;
        std::fs::rename(&tmp, &self.path)
    }

    /// The field's value, or `None` if it is missing or has the wrong type.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.fields.get(key).and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Sets a field; `None` removes it (so "use the default" is an absent key).
    pub fn set<T: Serialize>(&mut self, key: &str, value: Option<T>) {
        match value.and_then(|v| serde_json::to_value(v).ok()) {
            Some(v) => {
                self.fields.insert(key.to_string(), v);
            }
            None => {
                self.fields.remove(key);
            }
        }
    }

    pub fn routing(&self) -> RoutingSettings {
        let d = RoutingSettings::default();
        RoutingSettings {
            preferred_provider: self.get(keys::PREFERRED_PROVIDER).unwrap_or(d.preferred_provider),
            ollama_model: self.get(keys::OLLAMA_MODEL).unwrap_or(d.ollama_model),
            anthropic_model: self.get(keys::ANTHROPIC_MODEL).unwrap_or(d.anthropic_model),
            openai_model: self.get(keys::OPENAI_MODEL).unwrap_or(d.openai_model),
            google_model: self.get(keys::GOOGLE_MODEL).unwrap_or(d.google_model),
            openrouter_model: self.get(keys::OPENROUTER_MODEL).unwrap_or(d.openrouter_model),
            user_daily_cap_cents: self.get(keys::USER_DAILY_CAP_CENTS),
            global_daily_cap_cents: self.get(keys::GLOBAL_DAILY_CAP_CENTS),
            local_only: self.get(keys::LOCAL_ONLY_MODE).unwrap_or(d.local_only),
            cheap_first: self.get(keys::CHEAP_FIRST).unwrap_or(d.cheap_first),
            use_local_model: self.get(keys::USE_LOCAL_MODEL).unwrap_or(d.use_local_model),
            openrouter_prices: self.openrouter_prices(),
        }
    }

    /// The UI's half of the routing settings; the cached OpenRouter prices stay as they are
    /// (`set_openrouter_prices` is the only writer).
    pub fn set_routing(&mut self, r: &RoutingSettings) {
        self.set(keys::PREFERRED_PROVIDER, Some(r.preferred_provider));
        self.set(keys::OLLAMA_MODEL, Some(&r.ollama_model));
        self.set(keys::ANTHROPIC_MODEL, Some(&r.anthropic_model));
        self.set(keys::OPENAI_MODEL, Some(&r.openai_model));
        self.set(keys::GOOGLE_MODEL, Some(&r.google_model));
        self.set(keys::OPENROUTER_MODEL, Some(r.openrouter_model.trim()));
        self.set(keys::USER_DAILY_CAP_CENTS, r.user_daily_cap_cents);
        self.set(keys::GLOBAL_DAILY_CAP_CENTS, r.global_daily_cap_cents);
        self.set(keys::LOCAL_ONLY_MODE, Some(r.local_only));
        self.set(keys::CHEAP_FIRST, Some(r.cheap_first));
        self.set(keys::USE_LOCAL_MODEL, Some(r.use_local_model));
    }

    /// Stored as `{model: {"in": cents, "out": cents}}`, the shape core's parser reads.
    pub fn openrouter_prices(&self) -> BTreeMap<String, (f64, f64)> {
        self.get::<BTreeMap<String, Value>>(keys::OPENROUTER_PRICES)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(model, price)| Some((model, (price.get("in")?.as_f64()?, price.get("out")?.as_f64()?))))
            .collect()
    }

    pub fn set_openrouter_prices(&mut self, prices: &BTreeMap<String, (f64, f64)>) {
        let map: BTreeMap<String, Value> =
            prices.iter().map(|(model, (input, output))| (model.clone(), serde_json::json!({"in": input, "out": output}))).collect();
        self.set(keys::OPENROUTER_PRICES, (!map.is_empty()).then_some(map));
    }

    /// Whether the user has the MCP server switched on. Off by default: it opens memory to other programs.
    pub fn mcp_server_enabled(&self) -> bool {
        self.get(keys::MCP_SERVER_ENABLED).unwrap_or(false)
    }

    pub fn set_mcp_server_enabled(&mut self, enabled: bool) {
        self.set(keys::MCP_SERVER_ENABLED, Some(enabled));
    }

    /// FR-MTG-2: whether the audio/app heuristic may prompt at all. On by default, as on
    /// the Mac: a prompt-only feature.
    pub fn heuristic_detection_enabled(&self) -> bool {
        self.get(keys::HEURISTIC_DETECTION).unwrap_or(true)
    }

    /// FR-MTG-1: start recording without asking once the alert's countdown runs out.
    /// Off by default, as on the Mac.
    pub fn auto_start_enabled(&self) -> bool {
        self.get(keys::MEETING_AUTO_START).unwrap_or(false)
    }

    /// FR-MTG-1: how many minutes before a calendar meeting the prompt appears (1–10).
    pub fn lead_minutes(&self) -> f64 {
        self.get::<f64>(keys::MEETING_LEAD_MINUTES).filter(|m| *m > 0.0).unwrap_or(1.0).clamp(1.0, 10.0)
    }

    /// How many minutes before a calendar meeting its Meeting Prep brief is made (5-120, default 30,
    /// the Mac's range). Separate from the 1-10 minute alert lead above.
    pub fn prep_lead_minutes(&self) -> f64 {
        self.get::<f64>(keys::MEETING_PREP_LEAD_MINUTES).filter(|m| *m > 0.0).unwrap_or(30.0).clamp(5.0, 120.0)
    }

    pub fn set_prep_lead_minutes(&mut self, minutes: f64) {
        self.set(keys::MEETING_PREP_LEAD_MINUTES, Some(minutes.clamp(5.0, 120.0)));
    }

    pub fn set_meeting_settings(&mut self, heuristic: bool, auto_start: bool, lead_minutes: f64) {
        self.set(keys::HEURISTIC_DETECTION, Some(heuristic));
        self.set(keys::MEETING_AUTO_START, Some(auto_start));
        self.set(keys::MEETING_LEAD_MINUTES, Some(lead_minutes.clamp(1.0, 10.0)));
    }

    /// FR-MEM-4: how long each kind of data is kept. The Mac stores the window and a
    /// "keep forever" flag separately, so a user who turns keeping-forever off gets their
    /// old number back; Windows reads and writes the same pair.
    pub fn retention(&self) -> crate::retention::RetentionSettings {
        use crate::retention::{DEFAULT_CAPTURE_DAYS, DEFAULT_CONVERSATION_DAYS, DEFAULT_MEETING_DAYS};
        let window = |days_key: &str, forever_key: &str, default: u32| {
            if self.get::<bool>(forever_key).unwrap_or(false) {
                return None;
            }
            Some(self.get::<u32>(days_key).filter(|d| *d > 0).unwrap_or(default))
        };
        crate::retention::RetentionSettings {
            capture_days: window(keys::RETENTION_CAPTURE_DAYS, keys::RETENTION_CAPTURE_FOREVER, DEFAULT_CAPTURE_DAYS),
            meeting_days: window(keys::RETENTION_MEETING_DAYS, keys::RETENTION_MEETING_FOREVER, DEFAULT_MEETING_DAYS),
            conversation_days: window(keys::RETENTION_CONVERSATION_DAYS, keys::RETENTION_CONVERSATION_FOREVER, DEFAULT_CONVERSATION_DAYS),
        }
    }

    /// Saves the windows. A `None` sets "keep forever" and leaves the stored number, so
    /// turning it off again restores what the user had.
    pub fn set_retention(&mut self, settings: crate::retention::RetentionSettings) {
        for (days, days_key, forever_key) in [
            (settings.capture_days, keys::RETENTION_CAPTURE_DAYS, keys::RETENTION_CAPTURE_FOREVER),
            (settings.meeting_days, keys::RETENTION_MEETING_DAYS, keys::RETENTION_MEETING_FOREVER),
            (settings.conversation_days, keys::RETENTION_CONVERSATION_DAYS, keys::RETENTION_CONVERSATION_FOREVER),
        ] {
            self.set(forever_key, Some(days.is_none()));
            if let Some(days) = days {
                self.set(days_key, Some(days.max(1)));
            }
        }
    }

    /// Whether the user has seen the first-run retention notice. Until they have, the
    /// scheduled sweep only counts — nothing on this PC is deleted before someone has
    /// been told what the windows are.
    pub fn retention_acknowledged(&self) -> bool {
        self.get(keys::RETENTION_ACKNOWLEDGED).unwrap_or(false)
    }

    /// Calendar events already prompted that haven't ended yet.
    pub fn prompted_meetings(&self, now_unix_ms: i64) -> std::collections::HashMap<String, i64> {
        let mut prompted: std::collections::HashMap<String, i64> = self.get(keys::MEETINGS_PROMPTED).unwrap_or_default();
        prompted.retain(|_, end| *end > now_unix_ms);
        prompted
    }

    pub fn set_prompted_meetings(&mut self, prompted: &std::collections::HashMap<String, i64>) {
        self.set(keys::MEETINGS_PROMPTED, Some(prompted));
    }

    pub fn set_retention_acknowledged(&mut self, acknowledged: bool) {
        self.set(keys::RETENTION_ACKNOWLEDGED, Some(acknowledged));
    }

    /// Ambient capture is on by default.
    pub fn capture_enabled(&self) -> bool {
        self.get(keys::CAPTURE_ENABLED).unwrap_or(true)
    }

    pub fn set_capture_enabled(&mut self, enabled: bool) {
        self.set(keys::CAPTURE_ENABLED, Some(enabled));
    }

    /// User-excluded app identities or exe names.
    pub fn excluded_bundle_ids(&self) -> Vec<String> {
        self.get(keys::EXCLUDED_BUNDLE_IDS).unwrap_or_default()
    }

    pub fn prompt(&self, surface: PromptSurface) -> String {
        self.get(&surface.key()).unwrap_or_default()
    }

    /// Empty text removes the key (back to "no extra instructions").
    pub fn set_prompt(&mut self, surface: PromptSurface, text: &str) {
        let text = text.trim();
        self.set(&surface.key(), (!text.is_empty()).then_some(text));
    }

    /// One call's instructions for `surface`, plus the digest surfaces' text, which
    /// `RoutingConfig` carries alongside for the daily and weekly digests.
    pub fn instructions(&self, surface: PromptSurface) -> Instructions {
        Instructions {
            custom: self.prompt(surface),
            daily_digest: self.prompt(PromptSurface::DailyDigest),
            weekly_digest: self.prompt(PromptSurface::WeeklyDigest),
            profile: self.profile().routing_profile(),
        }
    }

    /// The Profile. A missing or wrong-typed field reads as its default on its own, and one bad
    /// entry in the facts list drops only that entry, like every other getter here.
    pub fn profile(&self) -> Profile {
        let d = Profile::default();
        let facts = match self.fields.get(keys::PROFILE_FACTS) {
            Some(Value::Array(items)) => items.iter().filter_map(|v| serde_json::from_value::<Fact>(v.clone()).ok()).collect(),
            _ => Vec::new(),
        };
        Profile {
            name: self.get(keys::PROFILE_NAME).unwrap_or_default(),
            role: self.get(keys::PROFILE_ROLE).unwrap_or_default(),
            organization: self.get(keys::PROFILE_ORGANIZATION).unwrap_or_default(),
            about: self.get(keys::PROFILE_ABOUT).unwrap_or_default(),
            facts,
            enabled: self.get(keys::PROFILE_ENABLED).unwrap_or(d.enabled),
            suggest_enabled: self.get(keys::PROFILE_SUGGEST_ENABLED).unwrap_or(d.suggest_enabled),
        }
    }

    /// The device the user chose to run routines (a public key), or empty for "any device".
    pub fn primary_device_id(&self) -> String {
        self.get::<String>(keys::ROUTINES_PRIMARY_DEVICE).map(|s| s.trim().to_string()).unwrap_or_default()
    }

    /// Blank removes the key, which is "any device".
    pub fn set_primary_device_id(&mut self, id: &str) {
        let id = id.trim();
        self.set(keys::ROUTINES_PRIMARY_DEVICE, (!id.is_empty()).then(|| id.to_string()));
    }

    /// An empty field or list removes its key (back to the default); the switches are always stored,
    /// so a choice is explicit.
    pub fn set_profile(&mut self, p: &Profile) {
        let text = |s: &str| (!s.trim().is_empty()).then(|| s.to_string());
        self.set(keys::PROFILE_NAME, text(&p.name));
        self.set(keys::PROFILE_ROLE, text(&p.role));
        self.set(keys::PROFILE_ORGANIZATION, text(&p.organization));
        self.set(keys::PROFILE_ABOUT, text(&p.about));
        self.set(keys::PROFILE_FACTS, (!p.facts.is_empty()).then_some(&p.facts));
        self.set(keys::PROFILE_ENABLED, Some(p.enabled));
        self.set(keys::PROFILE_SUGGEST_ENABLED, Some(p.suggest_enabled));
    }

    pub fn meeting_snapshots_enabled(&self) -> bool {
        self.get(keys::MEETING_SNAPSHOTS_ENABLED).unwrap_or(true)
    }

    pub fn set_meeting_snapshots_enabled(&mut self, enabled: bool) {
        self.set(keys::MEETING_SNAPSHOTS_ENABLED, Some(enabled));
    }

    /// The folder the user chose for snapshots, or `None` for the default.
    pub fn meeting_snapshots_root(&self) -> Option<PathBuf> {
        self.get::<String>(keys::MEETING_SNAPSHOTS_ROOT).filter(|p| !p.trim().is_empty()).map(PathBuf::from)
    }

    /// `None` goes back to the default.
    pub fn set_meeting_snapshots_root(&mut self, root: Option<&Path>) {
        self.set(keys::MEETING_SNAPSHOTS_ROOT, root.map(|p| p.display().to_string()));
    }

    pub fn show_screen_captures(&self) -> bool {
        self.get(keys::LIBRARY_SHOW_CAPTURES).unwrap_or(true)
    }

    pub fn set_show_screen_captures(&mut self, show: bool) {
        self.set(keys::LIBRARY_SHOW_CAPTURES, Some(show));
    }

    /// App identities whose episodes the Library hides. Captures stay searchable.
    pub fn hidden_episode_apps(&self) -> Vec<String> {
        self.get(keys::LIBRARY_HIDDEN_EPISODE_APPS).unwrap_or_default()
    }

    /// Adds or removes `app`, compared case-insensitively.
    pub fn set_episode_app_hidden(&mut self, app: &str, hidden: bool) {
        let mut apps = self.hidden_episode_apps();
        apps.retain(|a| !a.eq_ignore_ascii_case(app));
        if hidden {
            apps.push(app.to_string());
        }
        self.set(keys::LIBRARY_HIDDEN_EPISODE_APPS, Some(apps));
    }

    /// The recorded start-at-login choice; `None` before the first release launch.
    pub fn autostart(&self) -> Option<bool> {
        self.get(keys::GENERAL_AUTOSTART)
    }

    pub fn set_autostart(&mut self, enabled: bool) {
        self.set(keys::GENERAL_AUTOSTART, Some(enabled));
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn prompt_keys_are_the_macs_field_keys() {
        let keys: Vec<String> = PromptSurface::ALL.iter().map(|s| s.key()).collect();
        assert_eq!(
            keys,
            ["prompts.ask", "prompts.meetingSummary", "prompts.dailyDigest", "prompts.weeklyDigest", "prompts.episodeSummary", "prompts.threadSummary"]
        );
        // The serde form is the same raw value, so the UI and the file agree.
        assert_eq!(serde_json::to_value(PromptSurface::MeetingSummary).unwrap(), "meetingSummary");
    }

    #[test]
    fn prompts_round_trip_and_blank_clears() {
        let dir = std::env::temp_dir().join(format!("whimbrel-settings-prompts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let mut s = SettingsFile::load(&path);
        assert_eq!(s.prompt(PromptSurface::Ask), "");
        s.set_prompt(PromptSurface::Ask, "  answer in bullets \n");
        s.set_prompt(PromptSurface::DailyDigest, "be terse");
        s.save().unwrap();
        let mut s = SettingsFile::load(&path);
        assert_eq!(s.prompt(PromptSurface::Ask), "answer in bullets");
        let i = s.instructions(PromptSurface::Ask);
        assert_eq!((i.custom.as_str(), i.daily_digest.as_str(), i.weekly_digest.as_str()), ("answer in bullets", "be terse", ""));
        s.set_prompt(PromptSurface::Ask, "   ");
        assert!(s.get::<String>("prompts.ask").is_none(), "blank removes the key");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod capture_settings_tests {
    use super::*;

    #[test]
    fn capture_defaults_on_with_no_exclusions_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("whimbrel-settings-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let mut s = SettingsFile::load(&path);
        assert!(s.capture_enabled());
        assert!(s.excluded_bundle_ids().is_empty());
        s.set_capture_enabled(false);
        s.set(keys::EXCLUDED_BUNDLE_IDS, Some(vec!["spotify.exe"]));
        s.save().unwrap();
        let s = SettingsFile::load(&path);
        assert!(!s.capture_enabled());
        assert_eq!(s.excluded_bundle_ids(), vec!["spotify.exe".to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whimbrel-settings-{name}-{}", std::process::id()));
        // Windows reuses process ids, and the self-hosted runner keeps its temp directory
        // between runs, so a test that saved here last time would otherwise be read back as
        // this run's "fresh" file (the prep-lead default read as 45 on main, 2026-10-21).
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn the_prep_lead_defaults_to_thirty_and_stays_between_five_and_one_twenty() {
        let path = temp_path("prep-lead");
        let mut s = SettingsFile::load(&path);
        assert_eq!(s.prep_lead_minutes(), 30.0);
        s.set_prep_lead_minutes(2.0);
        assert_eq!(s.prep_lead_minutes(), 5.0);
        s.set_prep_lead_minutes(500.0);
        assert_eq!(s.prep_lead_minutes(), 120.0);
        s.set_prep_lead_minutes(45.0);
        s.save().unwrap();
        assert_eq!(SettingsFile::load(&path).prep_lead_minutes(), 45.0, "kept on this PC between launches");
        let mut hand_edited = SettingsFile::load(&path);
        hand_edited.set(keys::MEETING_PREP_LEAD_MINUTES, Some(-3.0_f64));
        assert_eq!(hand_edited.prep_lead_minutes(), 30.0, "a nonsense stored value falls back to the default");
        hand_edited.set(keys::MEETING_PREP_LEAD_MINUTES, Some(2.0_f64));
        assert_eq!(hand_edited.prep_lead_minutes(), 5.0, "a stored value below the range is raised on read");
        hand_edited.set(keys::MEETING_PREP_LEAD_MINUTES, Some(999.0_f64));
        assert_eq!(hand_edited.prep_lead_minutes(), 120.0, "and one above it lowered");
    }

    #[test]
    fn prompted_meetings_survive_a_reload_until_they_end() {
        let path = temp_path("prompted-meetings");
        let mut s = SettingsFile::load(&path);
        s.set_prompted_meetings(&std::collections::HashMap::from([("ended".to_string(), 100), ("live".to_string(), 5_000)]));
        s.save().unwrap();
        let reloaded = SettingsFile::load(&path);
        assert_eq!(reloaded.prompted_meetings(1_000), std::collections::HashMap::from([("live".to_string(), 5_000)]));
    }

    #[test]
    fn load_copies_pre_rename_keys_forward_without_overwriting() {
        let path = temp_path("legacy-keys");
        std::fs::write(
            &path,
            r#"{"littlebird.meetingAutoStart": false, "littlebird.retentionMeetingDays": 30, "retention.meetingDays": 90}"#,
        )
        .unwrap();
        let s = SettingsFile::load(&path);
        assert_eq!(s.get::<bool>(keys::MEETING_AUTO_START), Some(false));
        assert_eq!(s.get::<i64>(keys::RETENTION_MEETING_DAYS), Some(90), "an existing new key wins");
        assert_eq!(s.get::<bool>("littlebird.meetingAutoStart"), Some(false), "old keys stay for downgrades");
    }

    #[test]
    fn missing_file_gives_the_mac_defaults() {
        let s = SettingsFile::load(&temp_path("missing"));
        assert_eq!(s.routing(), RoutingSettings::default());
        assert_eq!(s.routing().preferred_model(), "gemma4:12b");
    }

    #[test]
    fn round_trips_through_the_file_under_the_sync_field_keys() {
        let path = temp_path("roundtrip");
        let mut s = SettingsFile::load(&path);
        let routing = RoutingSettings {
            preferred_provider: Provider::Anthropic,
            anthropic_model: "claude-sonnet-5".into(),
            user_daily_cap_cents: Some(500),
            local_only: true,
            ..RoutingSettings::default()
        };
        s.set_routing(&routing);
        s.save().unwrap();

        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["routing.preferredProvider"], "anthropic");
        assert_eq!(raw["routing.anthropicDefaultModel"], "claude-sonnet-5");
        assert_eq!(raw["routing.userDailyCapCents"], 500);
        assert!(raw.get("routing.globalDailyCapCents").is_none(), "an unset cap is an absent key");

        assert_eq!(SettingsFile::load(&path).routing(), routing);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn cheap_first_local_model_and_openrouter_round_trip_and_default_like_core() {
        let path = temp_path("cheapfirst");
        let mut s = SettingsFile::load(&path);
        let r = s.routing();
        assert!(r.cheap_first && r.use_local_model, "unset reads as core's defaults");
        assert_eq!(r.openrouter_model, "");
        assert!(r.openrouter_prices.is_empty());

        s.set_routing(&RoutingSettings { cheap_first: false, use_local_model: false, openrouter_model: " vendor/model:free ".into(), ..RoutingSettings::default() });
        s.set_openrouter_prices(&[("vendor/model".to_string(), (0.1, 0.5))].into_iter().collect());
        s.save().unwrap();
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["routing.cheapFirst"], false);
        assert_eq!(raw["routing.useLocalModel"], false);
        assert_eq!(raw["routing.openrouterDefaultModel"], "vendor/model:free", "trimmed");
        assert_eq!(raw["routing.openrouterPrices"]["vendor/model"]["in"], 0.1);

        let loaded = SettingsFile::load(&path);
        let r = loaded.routing();
        assert!(!r.cheap_first && !r.use_local_model);
        assert_eq!(r.openrouter_prices.get("vendor/model"), Some(&(0.1, 0.5)));

        // The UI's save never touches the cached prices.
        let mut s = loaded;
        s.set_routing(&RoutingSettings::default());
        assert_eq!(s.openrouter_prices().get("vendor/model"), Some(&(0.1, 0.5)));

        let keys = ApiKeys { openrouter: Some("sk-or-1".into()), ..ApiKeys::default() };
        let cfg = r.routing_config(&keys, &Instructions::default());
        assert!(!cfg.cheap_first && !cfg.use_local_model);
        assert_eq!(cfg.openrouter_key.as_deref(), Some("sk-or-1"));
        assert_eq!(cfg.openrouter_model, "vendor/model:free");
        assert_eq!(cfg.openrouter_prices.get("vendor/model"), Some(&(0.1, 0.5)));
        assert!(keys.has(Provider::Openrouter) && !keys.has(Provider::Google) && keys.has(Provider::Ollama));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let path = temp_path("unknown");
        std::fs::write(&path, r#"{"prompts.ask":"be brief","episode.hidden.42":true}"#).unwrap();
        let mut s = SettingsFile::load(&path);
        s.set_routing(&RoutingSettings::default());
        s.save().unwrap();
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["prompts.ask"], "be brief");
        assert_eq!(raw["episode.hidden.42"], true);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_wrong_typed_field_degrades_only_that_field() {
        let path = temp_path("wrongtype");
        std::fs::write(
            &path,
            r#"{"routing.preferredProvider":"google","routing.googleDefaultModel":"gemini-2.5-pro","routing.localOnlyMode":"yes","routing.userDailyCapCents":"lots"}"#,
        )
        .unwrap();
        let r = SettingsFile::load(&path).routing();
        assert_eq!(r.preferred_provider, Provider::Google);
        assert_eq!(r.google_model, "gemini-2.5-pro");
        assert!(!r.local_only);
        assert_eq!(r.user_daily_cap_cents, None);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_not_overwritten() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ not json").unwrap();
        let s = SettingsFile::load(&path);
        assert_eq!(s.routing(), RoutingSettings::default());
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(path.with_extension("json.corrupt")).unwrap(), "{ not json");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn routing_config_carries_keys_model_caps_and_local_only() {
        let secrets = MemorySecrets::default();
        secrets.set_string(APP_SERVICE, accounts::GOOGLE_API_KEY, "goog").unwrap();
        let keys = ApiKeys::load(&secrets).unwrap();
        let routing = RoutingSettings {
            preferred_provider: Provider::Google,
            google_model: "gemini-2.5-pro".into(),
            global_daily_cap_cents: Some(0),
            ..RoutingSettings::default()
        };
        let cfg = routing.routing_config(&keys, &Instructions { custom: "  be terse \n".into(), ..Default::default() });
        assert_eq!(cfg.preferred_provider, "google");
        assert_eq!(cfg.preferred_model, "gemini-2.5-pro");
        assert_eq!(cfg.google_key.as_deref(), Some("goog"));
        assert!(cfg.anthropic_key.is_none() && cfg.openai_key.is_none());
        assert_eq!(cfg.global_daily_cap_cents, Some(0));
        assert_eq!(cfg.user_daily_cap_cents, None);
        assert_eq!(cfg.custom_instructions, "be terse");
        assert!(!cfg.sensitive);

        let local = RoutingSettings { local_only: true, ..routing }.routing_config(&keys, &Instructions::default());
        assert!(local.sensitive, "local-only mode must reach the core as `sensitive`");
    }

    #[test]
    fn api_keys_debug_never_prints_a_key() {
        let keys = ApiKeys { anthropic: Some("sk-ant-secret".into()), ..Default::default() };
        assert!(!format!("{keys:?}").contains("sk-ant-secret"));
    }
}

#[cfg(test)]
mod profile_settings_tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("whimbrel-settings-profile-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fact(id: &str, text: &str, source: Option<i64>) -> Fact {
        Fact { id: id.into(), text: text.into(), source_meeting_id: source }
    }

    #[test]
    fn nothing_stored_is_an_empty_profile_with_both_switches_on_and_nothing_sent() {
        let d = dir("empty");
        let s = SettingsFile::load(&d.join("settings.json"));
        assert_eq!(s.profile(), Profile::default());
        assert!(s.profile().enabled && s.profile().suggest_enabled);
        assert_eq!(s.instructions(PromptSurface::Ask).profile, "");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_profile_round_trips_through_the_file_under_the_macs_field_keys() {
        let d = dir("round-trip");
        let path = d.join("settings.json");
        let mut s = SettingsFile::load(&path);
        let p = Profile {
            name: "Mike".into(),
            role: "Founder".into(),
            organization: "Whimbrel".into(),
            about: "Builds a meeting notebook.".into(),
            facts: vec![fact("a", "Works in UTC-5", None), fact("b", "Dana is the designer", Some(42))],
            enabled: false,
            suggest_enabled: false,
        };
        s.set_profile(&p);
        s.save().unwrap();
        let raw: BTreeMap<String, Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for key in ["profile.name", "profile.role", "profile.organization", "profile.about", "profile.facts", "profile.enabled", "profile.suggest_enabled"] {
            assert!(raw.contains_key(key), "{key}");
        }
        assert_eq!(raw["profile.facts"][1]["sourceMeetingId"], 42, "camelCase, the Mac's fact shape");
        assert_eq!(SettingsFile::load(&path).profile(), p);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn blank_fields_and_an_empty_list_remove_their_keys_but_the_switches_are_stored() {
        let d = dir("blank");
        let mut s = SettingsFile::load(&d.join("settings.json"));
        s.set_profile(&Profile { name: "Mike".into(), facts: vec![fact("a", "x", None)], ..Profile::default() });
        s.set_profile(&Profile { name: "   ".into(), ..Profile::default() });
        for key in ["profile.name", "profile.role", "profile.organization", "profile.about", "profile.facts"] {
            assert!(s.get::<Value>(key).is_none(), "{key} should be absent");
        }
        assert_eq!(s.get::<bool>("profile.enabled"), Some(true));
        assert_eq!(s.get::<bool>("profile.suggest_enabled"), Some(true));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn one_wrong_typed_field_degrades_on_its_own() {
        let d = dir("wrong-types");
        let path = d.join("settings.json");
        std::fs::write(&path, r#"{"profile.name": 5, "profile.role": "CTO", "profile.facts": "not a list", "profile.enabled": "yes", "profile.suggest_enabled": false}"#).unwrap();
        let p = SettingsFile::load(&path).profile();
        assert_eq!(p.name, "");
        assert_eq!(p.role, "CTO");
        assert!(p.facts.is_empty());
        assert!(p.enabled, "a wrong-typed switch reads as its default");
        assert!(!p.suggest_enabled);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn one_bad_fact_drops_only_itself() {
        let d = dir("bad-fact");
        let path = d.join("settings.json");
        std::fs::write(
            &path,
            r#"{"profile.facts": [{"id": "a", "text": "kept"}, 7, {"id": "b"}, {"id": "c", "text": "also kept", "sourceMeetingId": 3}]}"#,
        )
        .unwrap();
        let p = SettingsFile::load(&path).profile();
        assert_eq!(p.facts, vec![fact("a", "kept", None), fact("c", "also kept", Some(3))]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn every_surface_carries_the_composed_profile_and_the_switch_off_sends_none() {
        let d = dir("instructions");
        let mut s = SettingsFile::load(&d.join("settings.json"));
        s.set_profile(&Profile { name: "Mike".into(), role: "Founder".into(), facts: vec![fact("a", "Works in UTC-5", None)], ..Profile::default() });
        let expected = "Name: Mike\nRole: Founder\nFacts:\n- Works in UTC-5";
        for surface in PromptSurface::ALL {
            assert_eq!(s.instructions(surface).profile, expected, "{surface:?}");
        }
        let mut off = s.profile();
        off.enabled = false;
        s.set_profile(&off);
        for surface in PromptSurface::ALL {
            assert_eq!(s.instructions(surface).profile, "", "{surface:?}");
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn routing_config_carries_the_profile_and_a_call_without_instructions_carries_none() {
        let d = dir("routing");
        let mut s = SettingsFile::load(&d.join("settings.json"));
        s.set_profile(&Profile { role: "CTO".into(), ..Profile::default() });
        let keys = ApiKeys::default();
        let config = s.routing().routing_config(&keys, &s.instructions(PromptSurface::MeetingSummary));
        assert_eq!(config.profile, "Role: CTO");
        let title = s.routing().routing_config(&keys, &Instructions::default());
        assert_eq!(title.profile, "", "the conversation-title call does not need to know who the user is");
        std::fs::remove_dir_all(&d).unwrap();
    }
}

#[cfg(test)]
mod routine_host_settings_tests {
    use super::*;

    fn file(name: &str) -> (SettingsFile, PathBuf) {
        let d = std::env::temp_dir().join(format!("whimbrel-settings-host-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        (SettingsFile::load(&d.join("settings.json")), d)
    }

    #[test]
    fn meeting_snapshots_are_on_by_default_and_their_folder_can_be_chosen_and_reset() {
        let (mut s, d) = file("snapshots");
        assert!(s.meeting_snapshots_enabled());
        assert_eq!(s.meeting_snapshots_root(), None);
        s.set_meeting_snapshots_enabled(false);
        s.set_meeting_snapshots_root(Some(Path::new(r"D:\Slides")));
        s.save().unwrap();
        let reloaded = SettingsFile::load(&d.join("settings.json"));
        assert!(!reloaded.meeting_snapshots_enabled());
        assert_eq!(reloaded.meeting_snapshots_root(), Some(PathBuf::from(r"D:\Slides")));
        s.set_meeting_snapshots_root(None);
        s.save().unwrap();
        assert_eq!(SettingsFile::load(&d.join("settings.json")).meeting_snapshots_root(), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nothing_chosen_means_any_device_and_uses_the_macs_field_key() {
        let (s, d) = file("unset");
        assert_eq!(s.primary_device_id(), "");
        assert_eq!(keys::ROUTINES_PRIMARY_DEVICE, "routines.primaryDeviceId");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_choice_round_trips_through_the_file_trimmed_and_blank_clears_it() {
        let (mut s, d) = file("round-trip");
        s.set_primary_device_id("  AbC123+/= \n");
        s.save().unwrap();
        let mut again = SettingsFile::load(&d.join("settings.json"));
        assert_eq!(again.primary_device_id(), "AbC123+/=");
        again.set_primary_device_id("   ");
        assert!(again.get::<String>("routines.primaryDeviceId").is_none(), "blank removes the key");
        assert_eq!(again.primary_device_id(), "");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_wrong_typed_value_reads_as_any_device() {
        let (_, d) = file("wrong-type");
        let path = d.join("settings.json");
        std::fs::write(&path, r#"{"routines.primaryDeviceId": 7}"#).unwrap();
        assert_eq!(SettingsFile::load(&path).primary_device_id(), "");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
