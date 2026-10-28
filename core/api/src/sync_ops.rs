//! Settings-sync field shaping for cross-device hidden-episode marks (#90). Moved from
//! `core/ffi::whimbrel_memory_hidden_episode_sync_fields` /
//! `whimbrel_memory_apply_hidden_episode_sync_fields` (M3.0).
//!
//! The `whimbrel_sync_*` C functions in `core/ffi/src/sync_ffi.rs` — the crypto
//! primitives and `whimbrel_sync_content` — decode their base64/JSON arguments and make
//! exactly one `sync` / `sync_engine` call each, so they have no twin here: a Rust shell
//! calls those crates directly with typed bytes. The settings-sync iteration rules
//! (push planning, apply gating) live in [`coordinators`].

pub mod coordinators;

pub mod enrollment;

/// The settings-sync field map: `{"<field_key>": {"value": "...", "updated_at_unix_ms":
/// N, "is_deleted": bool}}` — the same shape `sync_engine::sync_settings` pushes and pulls.
pub type SettingsFields = serde_json::Map<String, serde_json::Value>;

/// Every hidden-episode mark in the settings-sync field shape, keyed
/// `episode.hidden.<device>|<bundle>|<start>`, so a settings-sync coordinator merges
/// them into its push map unchanged.
pub fn hidden_episode_sync_fields(store: &memory::Store) -> rusqlite::Result<SettingsFields> {
    Ok(store
        .hidden_episode_sync_fields()?
        .iter()
        .map(|m| {
            (
                m.field_key(),
                serde_json::json!({
                    "value": m.field_value(),
                    "updated_at_unix_ms": m.updated_at_unix_ms,
                    "is_deleted": false,
                }),
            )
        })
        .collect())
}

/// Applies pulled `episode.hidden.*` fields. Keys outside the prefix, malformed entries
/// and tombstones are ignored. Returns the number of marks that were newer than local
/// state and applied.
pub fn apply_hidden_episode_sync_fields(
    store: &memory::Store,
    fields: &SettingsFields,
) -> rusqlite::Result<usize> {
    let marks: Vec<memory::HiddenEpisodeMark> = fields
        .iter()
        .filter(|(_, f)| {
            !f.get("is_deleted")
                .and_then(|d| d.as_bool())
                .unwrap_or(false)
        })
        .filter_map(|(key, f)| {
            let value = f.get("value")?.as_str()?;
            let updated_at = f.get("updated_at_unix_ms")?.as_i64()?;
            memory::HiddenEpisodeMark::from_field(key, value, updated_at)
        })
        .collect();
    store.apply_hidden_episode_sync_fields(&marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_one_finalized_episode() -> (memory::Store, i64) {
        let store = memory::Store::open_in_memory().unwrap();
        for (i, text) in ["alpha one", "alpha two"].iter().enumerate() {
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "dev".to_string(),
                    ts_unix_ms: i as i64 * 60_000,
                    app_bundle_id: "com.app.a".to_string(),
                    window_title: "A".to_string(),
                    url: None,
                    text: text.to_string(),
                    hash: i as i64 + 1,
                    source: "ax".to_string(),
                })
                .unwrap();
        }
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "dev".to_string(),
                ts_unix_ms: 120_000,
                app_bundle_id: "com.app.b".to_string(),
                window_title: "B".to_string(),
                url: None,
                text: "bravo one".to_string(),
                hash: 3,
                source: "ax".to_string(),
            })
            .unwrap();
        store.run_jobs_batch(10, 10_000_000).unwrap();
        let episode_id = store.recent_episodes(10).unwrap()[0].id;
        (store, episode_id)
    }

    /// Mirrors `core/ffi`'s `hidden_episode_sync_fields_round_trip_through_the_settings_
    /// field_shape`.
    #[test]
    fn hidden_marks_round_trip_through_the_settings_field_shape() {
        let (store, episode_id) = store_with_one_finalized_episode();
        assert!(store.set_episode_hidden(episode_id, true, 5000).unwrap());

        let fields = hidden_episode_sync_fields(&store).unwrap();
        assert_eq!(fields.len(), 1);
        let (key, field) = fields.iter().next().unwrap();
        assert!(key.starts_with("episode.hidden.dev|"), "{key}");
        assert_eq!(
            serde_json::Value::Object(fields.clone()).to_string(),
            format!(r#"{{"{key}":{{"is_deleted":false,"updated_at_unix_ms":5000,"value":"1"}}}}"#)
        );
        assert_eq!(field["value"], "1");

        // A newer unhide pulled from a peer, plus noise that must be ignored.
        let pulled: SettingsFields = serde_json::from_str(&format!(
            r#"{{"{key}":{{"value":"0","updated_at_unix_ms":6000,"is_deleted":false}},
                "routing.localOnlyMode":{{"value":"true","updated_at_unix_ms":1,"is_deleted":false}},
                "episode.hidden.x|y|1":{{"value":"1","updated_at_unix_ms":9,"is_deleted":true}}}}"#
        ))
        .unwrap();
        assert_eq!(
            apply_hidden_episode_sync_fields(&store, &pulled).unwrap(),
            1
        );
        assert!(store.list_hidden_episodes("dev").unwrap().is_empty());
    }
}
