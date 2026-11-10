//! The living vault and the phone's topic pages (port of the Mac's PR 681; hand-off at the end of
//! `docs/superpowers/plans/2026-11-09-living-vault-and-phone-topics.md`).
//!
//! Every `EVERY` on the slow lane, starting `FIRST` after launch:
//! - **Publish** each topic page as a snapshot the phone reads
//!   (`Store::publish_topic_page_snapshots`), built by this PC's name. Only after this launch's first
//!   good content-sync round: a PC that was off for days has the newest build time, so a stale page
//!   from it would win on every device.
//! - **The vault:** when "Keep an Obsidian vault up to date" is on and a folder is chosen, bring
//!   `<folder>/Whimbrel/` up to date (`api::topics_export::update_vault`), and record when, or that it
//!   failed, for Settings. The vault is local only.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;
use crate::syncing::controller::SyncController;

/// The Mac's cadence: 300 s, the first run about a minute after launch.
pub const EVERY: Duration = Duration::from_secs(300);
pub const FIRST: Duration = Duration::from_secs(60);

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Whether this launch has finished a content-sync round, so publishing can't overwrite a fresher page.
pub fn may_publish(content_last_synced_ms: Option<i64>) -> bool {
    content_last_synced_ms.is_some()
}

/// The vault folder to update this tick: on, and a folder chosen.
pub fn vault_root(enabled: bool, path: &str) -> Option<String> {
    (enabled && !path.trim().is_empty()).then(|| path.to_string())
}

/// One tick. Returns whether pages were published (for the log and tests).
pub fn tick(store: &StoreHandle, settings: &Mutex<SettingsFile>, content_last_synced_ms: Option<i64>, built_by: &str) -> bool {
    let mut published = false;
    if may_publish(content_last_synced_ms) {
        let (name, now) = (built_by.to_string(), now_ms());
        match store.call_blocking(move |s| s.publish_topic_page_snapshots(&name, now)) {
            Ok(Ok(queued)) => {
                published = true;
                tracing::debug!(queued, "published topic pages for the phone");
            }
            Ok(Err(e)) => tracing::warn!(error = %e, "publishing topic pages for the phone failed"),
            Err(e) => tracing::warn!(error = %e, "publishing topic pages for the phone failed"),
        }
    }
    let root = {
        let file = settings.lock().unwrap();
        vault_root(file.vault_enabled(), &file.vault_path())
    };
    if let Some(root) = root {
        let now = now_ms();
        let updated = store.call_blocking(move |s| api::topics_export::update_vault(s, Path::new(&root), now));
        let ok = matches!(updated, Ok(Ok(_)));
        if !ok {
            // The path is the user's folder: kept out of the log.
            tracing::warn!("vault update failed");
        }
        let mut file = settings.lock().unwrap();
        if ok {
            file.set_vault_last_updated(now);
        }
        file.set_vault_failed(!ok);
        if let Err(e) = file.save() {
            tracing::warn!(error = %e, "could not save the vault status");
        }
    }
    published
}

/// Starts the loop for the life of the process.
pub fn start(store: StoreHandle, settings: Arc<Mutex<SettingsFile>>, sync: Option<Arc<SyncController>>) {
    let spawned = std::thread::Builder::new().name("whimbrel-topic-vault".into()).spawn(move || {
        std::thread::sleep(FIRST);
        let built_by = crate::syncing::controller::computer_name();
        loop {
            let synced = sync.as_ref().and_then(|c| c.view().content_last_synced_ms);
            tick(&store, &settings, synced, &built_by);
            std::thread::sleep(EVERY);
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the topic pages and vault loop");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishing_waits_for_this_launchs_first_content_sync() {
        assert!(!may_publish(None));
        assert!(may_publish(Some(1)));
    }

    #[test]
    fn the_vault_needs_the_switch_and_a_folder() {
        assert_eq!(vault_root(true, "C:\\Notes"), Some("C:\\Notes".into()));
        assert_eq!(vault_root(false, "C:\\Notes"), None);
        assert_eq!(vault_root(true, "  "), None);
    }

    fn settings() -> (std::path::PathBuf, Mutex<SettingsFile>) {
        let path = std::env::temp_dir().join(format!("whimbrel-vault-{}-{:?}.json", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_file(&path);
        (path.clone(), Mutex::new(SettingsFile::load(&path)))
    }

    #[test]
    fn a_tick_writes_the_vault_and_records_when_or_that_it_failed() {
        let store = StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap();
        let (path, file) = settings();
        let root = std::env::temp_dir().join(format!("whimbrel-vault-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut f = file.lock().unwrap();
            f.set_vault_enabled(true);
            f.set_vault_path(&root.display().to_string());
        }
        assert!(!tick(&store, &file, None, "PC"), "no content sync yet: nothing published");
        assert!(file.lock().unwrap().vault_last_updated().is_some(), "the vault still updates: it is local");
        assert!(!file.lock().unwrap().vault_failed());
        assert!(root.join("Whimbrel").is_dir());

        file.lock().unwrap().set_vault_path(&root.join("missing").display().to_string());
        tick(&store, &file, Some(1), "PC");
        assert!(file.lock().unwrap().vault_failed(), "a folder that doesn't exist fails, and isn't created");
        assert!(!root.join("missing").exists());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(path);
    }
}
