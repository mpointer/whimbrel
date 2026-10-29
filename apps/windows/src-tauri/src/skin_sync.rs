//! Skins sync on Windows (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md, Task 15): this PC
//! sends its own skins and writes the other devices' into its skins folder. `core/skins::sync` does the
//! work; this decides when, on the store actor.
//!
//! The shell emits no events (the page polls everything), so a change pulled from another device bumps
//! [`revision`], and every window's `skin.ts` reloads the skins when it sees a new one.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use skins::sync::{enqueue_all, enqueue_skin, reconcile_synced_skins, ReconcileReport};
use skins::UserSkinStore;

/// A skin moved to `.deleted/` is kept this long (so an edit that lost to a deletion can be found), then purged.
pub const DELETED_KEPT: Duration = Duration::from_secs(30 * 24 * 60 * 60);

static REVISION: AtomicU64 = AtomicU64::new(0);

/// Bumped whenever a sync changes the skins folder.
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

/// At launch: drops skins deleted more than 30 days ago.
pub fn purge_deleted(skins_root: &Path) -> usize {
    UserSkinStore::new(skins_root).purge_deleted(DELETED_KEPT)
}

/// Sends one skin after a change on this PC (a save, rename, delete, undo or import).
pub fn after_local_change(store: &memory::Store, skins_root: &Path, skin_id: &str) {
    // Never mid-save: a turn or an image save may be writing the folder.
    let _saving = crate::skin_maker::saving();
    if let Err(e) = enqueue_skin(store, &UserSkinStore::new(skins_root), skin_id) {
        tracing::warn!(error = %e, "could not queue a skin for sync");
    }
}

/// After a content sync round: writes the skins that arrived, then, on the first run here or when a peer
/// has just started syncing skins, sends every skin again (as the folder now holds them). Bumps
/// [`revision`] when the folder changed.
pub fn after_round(store: &memory::Store, skins_root: &Path) -> ReconcileReport {
    let _saving = crate::skin_maker::saving();
    let skins = UserSkinStore::new(skins_root);
    let report = match reconcile_synced_skins(store, &skins) {
        Ok(report) => {
            if !report.changed.is_empty() || !report.deleted.is_empty() {
                tracing::info!(changed = report.changed.len(), deleted = report.deleted.len(), "skins from other devices");
                REVISION.fetch_add(1, Ordering::Relaxed);
            }
            report
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not write the skins from other devices");
            ReconcileReport::default()
        }
    };
    let first = store.take_skin_first_push().unwrap_or(false);
    let repush = store.take_skin_repush_request().unwrap_or(false);
    if first || repush {
        if let Err(e) = enqueue_all(store, &skins) {
            tracing::warn!(error = %e, "could not queue the skins for sync");
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::skin_sync::SkinSyncPayload;

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("whimbrel-skin-sync-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(device: &str) -> memory::Store {
        let s = memory::Store::open_in_memory().unwrap();
        s.set_own_device_id(device);
        s
    }

    fn save(root: &Path, id: &str, name: &str) {
        let mut skin = skins::whimbrel_default();
        skin.id = id.into();
        skin.name = name.into();
        UserSkinStore::new(root).save(&skin, &[], &[]).unwrap();
    }

    fn skin_rows(s: &memory::Store) -> Vec<SkinSyncPayload> {
        s.list_outbox_operations().unwrap().into_iter().filter(|r| r.1 == "skin").map(|r| serde_json::from_str(&r.3).unwrap()).collect()
    }

    #[test]
    fn save_enqueues() {
        let scratch = Scratch::new("save");
        let s = store("pc");
        save(&scratch.0, "user-a-1", "A");
        after_local_change(&s, &scratch.0, "user-a-1");
        let rows = skin_rows(&s);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].skin_id.as_str(), rows[0].name.as_str(), rows[0].device_id.as_str(), rows[0].deleted), ("user-a-1", "A", "pc", false));
        // A delete sends a deletion; a built-in sends nothing.
        UserSkinStore::new(&scratch.0).delete("user-a-1").unwrap();
        after_local_change(&s, &scratch.0, "user-a-1");
        after_local_change(&s, &scratch.0, "whimbrel");
        let rows = skin_rows(&s);
        assert_eq!(rows.len(), 2);
        assert!(rows[1].deleted);
    }

    #[test]
    fn first_run_enqueues_all_once() {
        let scratch = Scratch::new("first");
        let s = store("pc");
        save(&scratch.0, "user-a-1", "A");
        save(&scratch.0, "user-b-1", "B");
        after_round(&s, &scratch.0);
        assert_eq!(skin_rows(&s).len(), 2);
        after_round(&s, &scratch.0);
        assert_eq!(skin_rows(&s).len(), 2, "only the first round sends everything");
        // A peer that starts syncing skins asks for them again.
        s.record_skin_peer("mac").unwrap();
        after_round(&s, &scratch.0);
        assert_eq!(skin_rows(&s).len(), 4);
    }

    #[test]
    fn reconcile_emits_skins_changed() {
        let scratch = Scratch::new("reconcile");
        let s = store("pc");
        // Nothing arrived: no change, no new revision.
        let before = revision();
        assert_eq!(after_round(&s, &scratch.0), ReconcileReport::default());
        assert_eq!(revision(), before);
        let mut manifest = serde_json::to_value(skins::whimbrel_default()).unwrap();
        manifest["id"] = "user-mac-1".into();
        manifest["name"] = "From the Mac".into();
        let payload = SkinSyncPayload {
            skin_id: "user-mac-1".into(),
            name: "From the Mac".into(),
            manifest,
            palettes: vec![],
            images: vec![],
            updated_at_unix_ms: 1,
            device_id: "mac".into(),
            deleted: false,
        };
        s.apply_synced_skin("mac", &payload).unwrap();
        let report = after_round(&s, &scratch.0);
        assert_eq!(report.changed, ["user-mac-1"]);
        assert!(revision() > before);
        assert!(UserSkinStore::new(&scratch.0).manifest("user-mac-1").is_some());
    }

    #[test]
    fn purge_drops_only_old_deletions() {
        let scratch = Scratch::new("purge");
        save(&scratch.0, "user-a-1", "A");
        UserSkinStore::new(&scratch.0).delete("user-a-1").unwrap();
        assert_eq!(purge_deleted(&scratch.0), 0, "deleted just now: kept");
        assert_eq!(UserSkinStore::new(&scratch.0).deleted_ids(), ["user-a-1"]);
    }
}
