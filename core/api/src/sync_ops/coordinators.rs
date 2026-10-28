//! Settings-sync one-iteration rules (M3.0 part 2, Task 12), moved from
//! apps/macos `SettingsSyncCoordinator.swift`. The shell keeps the timer, the `.ready`
//! gating and the mapping between a `field_key` and its typed setting (UserDefaults /
//! Keychain); this module owns everything in between:
//!
//! * [`plan_push`] — which fields go up and with which timestamp. The API-key rule is
//!   the subtle part: a key only ever pushes while its own sync toggle is on, and the
//!   timestamp stamped on a tombstone is deliberately asymmetric (see
//!   [`plan_api_key_field`]) — getting this wrong was a real data-loss bug.
//! * [`settings_iteration`] — `plan_push` plus the one network round trip
//!   (`sync_engine::sync_settings`). Network only, no `Store`.
//! * [`apply_plan`] — which pulled fields the shell should apply, given the local
//!   timestamps *as they are after the round trip* (a local edit that landed during the
//!   network wait must win) and the local API-key toggles. Pure.
//!
//! Content sync has no twin here: `sync_engine::sync_content` is already a single
//! core call the shell invokes under the store handle (pre-existing ADR 0001 debt; its
//! prepare/exchange/apply split changes lease and cursor semantics and is tracked
//! separately), and the shell's `ContentSyncCoordinator` is timer and gating only.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
pub use sync_engine::{SettingsField, SyncEngineError};

/// Prefix of the cross-device hidden-episode marks (#90). The core owns their
/// timestamps and last-writer-wins (`super::apply_hidden_episode_sync_fields`), so
/// they bypass the shell's per-field timestamps entirely.
pub const HIDDEN_EPISODE_FIELD_PREFIX: &str = "episode.hidden.";

/// A plain local setting with its "last locally modified" timestamp (`0` = never
/// modified locally, so it always loses to whatever the server has).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalField {
    pub value: String,
    pub updated_at_unix_ms: i64,
}

/// One provider API key as the shell sees it. `key` is only consulted while
/// `sync_enabled` is true; a shell may pass `None` when the toggle is off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyState {
    /// e.g. `"apiKey.anthropic"`; its toggle is `"<field_key>.syncEnabled"`.
    pub field_key: String,
    pub sync_enabled: bool,
    #[serde(default)]
    pub key: Option<String>,
    /// Timestamp of the key field itself (bumped by save/clear of the key).
    pub key_updated_at_unix_ms: i64,
    /// Timestamp of the `.syncEnabled` toggle.
    pub toggle_updated_at_unix_ms: i64,
}

/// Everything the shell gathers for one iteration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSettings {
    #[serde(default)]
    pub fields: HashMap<String, LocalField>,
    #[serde(default)]
    pub api_keys: Vec<ApiKeyState>,
    /// From `super::hidden_episode_sync_fields`, merged into the push unchanged.
    #[serde(default)]
    pub hidden_episode_marks: HashMap<String, SettingsField>,
}

/// What the shell should push this iteration.
pub fn plan_push(local: &LocalSettings) -> HashMap<String, SettingsField> {
    let mut out: HashMap<String, SettingsField> = local
        .fields
        .iter()
        .map(|(k, f)| (k.clone(), SettingsField { value: f.value.clone(), updated_at_unix_ms: f.updated_at_unix_ms, is_deleted: false }))
        .collect();
    for key in &local.api_keys {
        if let Some(field) = plan_api_key_field(key) {
            out.insert(key.field_key.clone(), field);
        }
    }
    out.extend(local.hidden_episode_marks.iter().map(|(k, v)| (k.clone(), v.clone())));
    out
}

/// The API-key push rule.
///
/// Toggle ON: push the real key (or, if this device's secret store is empty, a
/// tombstone) stamped `max(key_ts, toggle_ts)`. The engine's push is a strict `>`
/// against the server's timestamp, so taking the toggle's timestamp when it is the more
/// recent event is what lets toggling back on re-push an existing key over a stale
/// server-side tombstone even though the key value itself hasn't been re-saved.
///
/// Toggle OFF: push a tombstone stamped with the toggle's timestamp ONLY, never the max.
/// That tombstone means exactly "the user turned sync off on this device", so only the
/// toggle may date it: an opted-out device typing its own unrelated key bumps `key_ts`,
/// and dating the tombstone by it would beat — and silently wipe — the real key an
/// opted-in device pushed earlier, with no user anywhere taking a delete action.
///
/// Either way, a device that has never held a key for this field (`key_ts == 0`) has no
/// basis to assert a delete and pushes nothing: a tombstone from it could beat a real
/// key still in flight from another device. It simply sits this tick out.
pub fn plan_api_key_field(key: &ApiKeyState) -> Option<SettingsField> {
    let timestamp = if key.sync_enabled {
        key.key_updated_at_unix_ms.max(key.toggle_updated_at_unix_ms)
    } else {
        key.toggle_updated_at_unix_ms
    };
    if key.sync_enabled {
        if let Some(value) = key.key.as_deref().filter(|k| !k.is_empty()) {
            return Some(SettingsField { value: value.to_string(), updated_at_unix_ms: timestamp, is_deleted: false });
        }
    }
    if key.key_updated_at_unix_ms <= 0 {
        return None;
    }
    Some(SettingsField { value: String::new(), updated_at_unix_ms: timestamp, is_deleted: true })
}

/// Result of one round trip: `pulled` is the DELTA the server had newer than what was
/// pushed — feed it to [`apply_plan`] with fresh local timestamps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Iteration {
    pub pushed_count: usize,
    pub pulled: HashMap<String, SettingsField>,
}

/// `plan_push` + `sync_engine::sync_settings`. **Blocking network call**; never run it
/// on the UI thread or under the store handle.
pub fn settings_iteration(
    turso_url: &str,
    turso_token: &str,
    group_key: &sync::GroupKey,
    local: &LocalSettings,
) -> Result<Iteration, SyncEngineError> {
    let push = plan_push(local);
    let pulled = sync_engine::sync_settings(turso_url, turso_token, group_key, &push)?;
    Ok(Iteration { pushed_count: push.len(), pulled })
}

/// One pulled field the shell should apply. After applying it successfully the shell
/// sets the field's local timestamp to `updated_at_unix_ms` (not "now"), so the next
/// iteration doesn't push it straight back; a field the shell could not apply (unparseable
/// value, unknown key) must leave the timestamp alone so it is re-pulled next time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PulledField {
    pub field_key: String,
    pub value: String,
    pub updated_at_unix_ms: i64,
    pub is_deleted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApplyPlan {
    /// Sorted by `field_key`, so application order is deterministic.
    pub fields: Vec<PulledField>,
    /// `episode.hidden.*` marks, for `super::apply_hidden_episode_sync_fields`.
    pub hidden_episode_marks: HashMap<String, SettingsField>,
}

/// Filters a pulled delta down to what the shell should apply.
///
/// * `local_timestamps` — the shell's per-field timestamps read AFTER the round trip; a
///   pulled field is applied only if strictly newer (closes the lost-update race with
///   an edit made during the network wait). Missing = `0`.
/// * `api_key_sync_enabled` — this device's toggle per API-key field key. A key field
///   is applied in NEITHER direction (save or delete) while the local toggle is off:
///   the toggle is an account-level synced field, so another device's opt-in must not
///   write key material into this device's secret store, nor delete its local key. A
///   freshly enrolled device therefore receives a key one tick after the toggle — by
///   design. A non-deleted key with an empty value is malformed and skipped.
pub fn apply_plan(
    pulled: &HashMap<String, SettingsField>,
    local_timestamps: &HashMap<String, i64>,
    api_key_sync_enabled: &HashMap<String, bool>,
) -> ApplyPlan {
    let mut plan = ApplyPlan::default();
    for (key, field) in pulled {
        if key.starts_with(HIDDEN_EPISODE_FIELD_PREFIX) {
            if !field.is_deleted {
                plan.hidden_episode_marks.insert(key.clone(), field.clone());
            }
            continue;
        }
        if field.updated_at_unix_ms <= local_timestamps.get(key).copied().unwrap_or(0) {
            continue;
        }
        if let Some(enabled) = api_key_sync_enabled.get(key) {
            if !enabled || (!field.is_deleted && field.value.is_empty()) {
                continue;
            }
        }
        plan.fields.push(PulledField {
            field_key: key.clone(),
            value: field.value.clone(),
            updated_at_unix_ms: field.updated_at_unix_ms,
            is_deleted: field.is_deleted,
        });
    }
    plan.fields.sort_by(|a, b| a.field_key.cmp(&b.field_key));
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(sync_enabled: bool, key: Option<&str>, key_ts: i64, toggle_ts: i64) -> ApiKeyState {
        ApiKeyState {
            field_key: "apiKey.anthropic".into(),
            sync_enabled,
            key: key.map(str::to_string),
            key_updated_at_unix_ms: key_ts,
            toggle_updated_at_unix_ms: toggle_ts,
        }
    }

    fn field(value: &str, ts: i64, deleted: bool) -> SettingsField {
        SettingsField { value: value.into(), updated_at_unix_ms: ts, is_deleted: deleted }
    }

    // ---- the API-key push rule (SettingsSyncCoordinator.addApiKeyField) ----

    #[test]
    fn toggle_on_pushes_key_stamped_with_max_of_key_and_toggle() {
        let f = plan_api_key_field(&key(true, Some("sk-1"), 100, 500)).unwrap();
        assert_eq!(f, field("sk-1", 500, false), "toggle newer than key re-pushes an unchanged key over a stale tombstone");
        let f = plan_api_key_field(&key(true, Some("sk-1"), 900, 500)).unwrap();
        assert_eq!(f.updated_at_unix_ms, 900);
    }

    #[test]
    fn toggle_on_with_empty_keychain_pushes_tombstone_with_max() {
        assert_eq!(plan_api_key_field(&key(true, None, 100, 500)).unwrap(), field("", 500, true));
        assert_eq!(plan_api_key_field(&key(true, Some(""), 700, 500)).unwrap(), field("", 700, true));
    }

    #[test]
    fn toggle_off_tombstone_is_dated_by_the_toggle_only() {
        // The data-loss case: an opted-out device typed its own key (key_ts 900 > toggle
        // 500); its tombstone must NOT carry 900 or it would wipe an opted-in device's key.
        assert_eq!(plan_api_key_field(&key(false, Some("local-only"), 900, 500)).unwrap(), field("", 500, true));
    }

    #[test]
    fn device_that_never_held_a_key_pushes_nothing() {
        assert_eq!(plan_api_key_field(&key(false, None, 0, 500)), None);
        assert_eq!(plan_api_key_field(&key(true, None, 0, 500)), None, "toggle on but nothing ever known: sit the tick out");
        assert!(plan_api_key_field(&key(true, Some("sk-1"), 0, 500)).is_some(), "a real key pushes even at ts 0");
    }

    #[test]
    fn plan_push_merges_plain_fields_keys_and_hidden_marks() {
        let mut local = LocalSettings::default();
        local.fields.insert("routing.localOnlyMode".into(), LocalField { value: "true".into(), updated_at_unix_ms: 10 });
        local.api_keys.push(key(true, Some("sk-1"), 20, 5));
        local.api_keys.push(ApiKeyState { field_key: "apiKey.openai".into(), ..key(false, None, 0, 5) });
        local.hidden_episode_marks.insert("episode.hidden.d|b|1".into(), field("1", 30, false));
        let push = plan_push(&local);
        assert_eq!(push.len(), 3);
        assert_eq!(push["routing.localOnlyMode"], field("true", 10, false));
        assert_eq!(push["apiKey.anthropic"], field("sk-1", 20, false));
        assert_eq!(push["episode.hidden.d|b|1"], field("1", 30, false));
        assert!(!push.contains_key("apiKey.openai"));
    }

    // ---- the apply gate (SettingsSyncCoordinator.applyDelta / applyApiKey) ----

    #[test]
    fn apply_only_fields_strictly_newer_than_local() {
        let pulled = HashMap::from([
            ("a".to_string(), field("1", 100, false)),
            ("b".to_string(), field("2", 100, false)),
            ("c".to_string(), field("3", 100, false)),
        ]);
        let local = HashMap::from([("a".to_string(), 50), ("b".to_string(), 100)]);
        let plan = apply_plan(&pulled, &local, &HashMap::new());
        let keys: Vec<_> = plan.fields.iter().map(|f| f.field_key.as_str()).collect();
        assert_eq!(keys, ["a", "c"], "equal timestamp is not newer; missing local = 0");
        assert_eq!(plan.fields[0], PulledField { field_key: "a".into(), value: "1".into(), updated_at_unix_ms: 100, is_deleted: false });
    }

    #[test]
    fn hidden_episode_marks_are_partitioned_and_tombstones_dropped() {
        let pulled = HashMap::from([
            ("episode.hidden.d|b|1".to_string(), field("1", 5, false)),
            ("episode.hidden.d|b|2".to_string(), field("", 5, true)),
            ("routing.localOnlyMode".to_string(), field("true", 5, false)),
        ]);
        // Marks bypass local timestamps entirely: even a huge local timestamp doesn't gate them.
        let local = HashMap::from([("episode.hidden.d|b|1".to_string(), 1_000)]);
        let plan = apply_plan(&pulled, &local, &HashMap::new());
        assert_eq!(plan.hidden_episode_marks.len(), 1);
        assert!(plan.hidden_episode_marks.contains_key("episode.hidden.d|b|1"));
        assert_eq!(plan.fields.len(), 1);
        assert_eq!(plan.fields[0].field_key, "routing.localOnlyMode");
    }

    #[test]
    fn api_key_fields_are_gated_by_the_local_toggle_in_both_directions() {
        let pulled = HashMap::from([
            ("apiKey.anthropic".to_string(), field("sk-remote", 100, false)),
            ("apiKey.openai".to_string(), field("", 100, true)),
            ("apiKey.google".to_string(), field("sk-g", 100, false)),
            ("apiKey.google.syncEnabled".to_string(), field("true", 100, false)),
        ]);
        let toggles = HashMap::from([
            ("apiKey.anthropic".to_string(), false),
            ("apiKey.openai".to_string(), false),
            ("apiKey.google".to_string(), true),
        ]);
        let plan = apply_plan(&pulled, &HashMap::new(), &toggles);
        let keys: Vec<_> = plan.fields.iter().map(|f| f.field_key.as_str()).collect();
        assert_eq!(keys, ["apiKey.google", "apiKey.google.syncEnabled"], "opted-out device neither receives a key nor deletes its own");
    }

    #[test]
    fn malformed_non_deleted_empty_api_key_is_skipped() {
        let pulled = HashMap::from([("apiKey.anthropic".to_string(), field("", 100, false))]);
        let toggles = HashMap::from([("apiKey.anthropic".to_string(), true)]);
        assert!(apply_plan(&pulled, &HashMap::new(), &toggles).fields.is_empty());
        let pulled = HashMap::from([("apiKey.anthropic".to_string(), field("", 100, true))]);
        assert_eq!(apply_plan(&pulled, &HashMap::new(), &toggles).fields.len(), 1, "a tombstone is applied");
    }

    #[test]
    fn iteration_against_unreachable_database_is_an_error_not_a_panic() {
        let key = sync::GroupKey::from_bytes([7u8; 32]);
        let err = settings_iteration("libsql://nope.invalid", "token", &key, &LocalSettings::default());
        assert!(err.is_err());
    }
}
