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
//! - **Only the keys in [`SYNCED_KEYS`]** ever leave this PC. Device-local and Windows-only keys
//!   are never gathered, and a pulled key not on the list is ignored and not stamped.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use api::sync_ops::coordinators::{apply_plan, Iteration, LocalField, LocalSettings, PulledField};

use crate::profile::{Fact, MAX_FACTS, MAX_FACT_CHARS};
use crate::settings::{keys, SettingsFile};

/// The keys this PC syncs: the Profile (plan Decision 1).
pub const SYNCED_KEYS: &[&str] = &[
    keys::PROFILE_NAME,
    keys::PROFILE_ROLE,
    keys::PROFILE_ORGANIZATION,
    keys::PROFILE_ABOUT,
    keys::PROFILE_FACTS,
    keys::PROFILE_ENABLED,
    keys::PROFILE_SUGGEST_ENABLED,
    keys::PROFILE_OTHER_NAMES,
];

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
    if field.is_deleted || !SYNCED_KEYS.contains(&field.field_key.as_str()) {
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

/// One settings round. The lock is held only to gather and to apply, never across `iterate`
/// (the network call: `settings_iteration` in production, a fake in tests).
pub fn run_round<F>(settings: &Mutex<SettingsFile>, iterate: F) -> Result<usize, String>
where
    F: FnOnce(&LocalSettings) -> Result<Iteration, String>,
{
    let local = gather(&settings.lock().unwrap());
    let iteration = iterate(&local)?;
    if iteration.pulled.is_empty() {
        return Ok(0);
    }
    let mut file = settings.lock().unwrap();
    let applied = apply_pulled(&mut file, &iteration.pulled);
    if applied > 0 {
        file.save().map_err(|e| e.to_string())?;
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::sync_ops::coordinators::SettingsField;

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
        let (_d2, mut b) = file();
        for (key, value) in encoded(&a) {
            assert!(apply_field(&mut b, &pulled(&key, &value, 5)), "{key} should apply");
        }
        assert_eq!(b.profile(), a.profile());
        assert_eq!(b.other_names(), "Mikey, MP");
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
        assert!(!SYNCED_KEYS.contains(&keys::SYNC_FIELD_TIMESTAMPS));
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
        assert!(!apply_field(&mut f, &pulled("prompts.ask", "Be brief", 5)));
        assert!(!apply_field(&mut f, &pulled(keys::CAPTURE_ENABLED, "false", 5)), "a device-local key never applies");
        assert_eq!(f.get::<String>("prompts.ask"), None, "an unknown key isn't written into the file");
    }

    #[test]
    fn device_local_keys_are_never_gathered() {
        let (_d, mut f) = file();
        f.set(keys::CAPTURE_ENABLED, Some(false));
        f.set(keys::APP_LANGUAGE, Some("fr"));
        let local = gather(&f);
        let mut gathered: Vec<_> = local.fields.keys().map(String::as_str).collect();
        gathered.sort_unstable();
        let mut expected = SYNCED_KEYS.to_vec();
        expected.sort_unstable();
        assert_eq!(gathered, expected);
        assert!(local.api_keys.is_empty() && local.hidden_episode_marks.is_empty());
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
        for key in SYNCED_KEYS.iter().filter(|k| **k != keys::PROFILE_ROLE) {
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
        let result = run_round(&settings, |_local| {
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
        let result = run_round(&settings, |local| {
            assert_eq!(local.fields.len(), SYNCED_KEYS.len());
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
        assert_eq!(run_round(&settings, |_| Err("settings sync connection failed: offline".into())), Err("settings sync connection failed: offline".into()));
        assert_eq!(encoded(&settings.lock().unwrap()), before);
    }
}
