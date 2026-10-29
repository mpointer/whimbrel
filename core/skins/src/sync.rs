//! Skin sync between this device's skins folder and the store's synced tables
//! (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md, Task 14).
//!
//! The app calls [`enqueue_skin`] after each save, rename, delete, undo and import, [`enqueue_all`] on
//! the first launch and when the store asks for a re-push, and [`reconcile_synced_skins`] after each
//! content sync. Reconcile writes a pulled skin through the same rules an import uses (the loader's
//! repairs, the palette gate, the image name rule), so nothing synced can draw below 4.5:1 or name a
//! file outside the skin's folder. Only `user-` skins sync; chat, history and the selected skin never do.

use std::collections::HashMap;
use std::fmt;

use memory::skin_sync::{sha256_hex, SkinSyncPayload};
use memory::Store;
use serde_json::Value;

use crate::format::Skin;
use crate::loader::parse_skin;
use crate::palette::{default_palette, parse_palette, Palette};
use crate::store::{is_image_name, is_user_id, UserSkinStore};
use crate::whimbrel_default;

pub use crate::images::downscale;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncError(pub String);

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "skin sync: {}", self.0)
    }
}

impl std::error::Error for SyncError {}

fn err(e: impl fmt::Display) -> SyncError {
    SyncError(e.to_string())
}

/// What a reconcile changed in the skins folder, so the app can reload (and fall back when the selected
/// skin went).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The image files a skin places. Files left behind by a replaced image (kept for undo) don't sync.
fn placed_images(skin: &Skin) -> Vec<String> {
    let mut names: Vec<String> = skin
        .decoration
        .imagery
        .iter()
        .map(|i| i.asset.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The skin as it is in the folder, under `version`; a missing folder is a deletion.
fn payload(
    store: &Store,
    skins: &UserSkinStore,
    skin_id: &str,
    version: (i64, String),
) -> Result<SkinSyncPayload, SyncError> {
    let (updated_at_unix_ms, device_id) = version;
    let Some(bytes) = skins.manifest(skin_id) else {
        return Ok(SkinSyncPayload {
            skin_id: skin_id.to_string(),
            name: String::new(),
            manifest: Value::Null,
            palettes: vec![],
            images: vec![],
            updated_at_unix_ms,
            device_id,
            deleted: true,
        });
    };
    let manifest: Value = serde_json::from_slice(&bytes).map_err(err)?;
    let (skin, _) = parse_skin(&bytes, &whimbrel_default())
        .ok_or_else(|| err(format!("{skin_id}'s manifest can't be read")))?;
    let palettes = skins
        .palette_files(skin_id)
        .into_iter()
        .filter_map(|(id, data)| Some((id, serde_json::from_slice::<Value>(&data).ok()?)))
        .collect();
    let mut images = Vec::new();
    for name in placed_images(&skin) {
        // An image that went missing from the folder is left out rather than stopping the skin.
        let Ok(path) = skins.image_path(skin_id, &name) else {
            continue;
        };
        let data = std::fs::read(&path).map_err(err)?;
        let sha = sha256_hex(&data);
        store.enqueue_skin_chunks(&sha, &data).map_err(err)?;
        images.push((name, sha));
    }
    Ok(SkinSyncPayload {
        skin_id: skin_id.to_string(),
        name: skin.name,
        manifest,
        palettes,
        images,
        updated_at_unix_ms,
        device_id,
        deleted: false,
    })
}

/// Sends the skin as the folder now holds it, after a local change: its images' chunks (each image
/// once), then the row. A missing folder sends a deletion. A built-in, or a store with no device id,
/// sends nothing.
pub fn enqueue_skin(store: &Store, skins: &UserSkinStore, skin_id: &str) -> Result<(), SyncError> {
    let Some(own) = store.own_device_id() else {
        return Ok(());
    };
    if !is_user_id(skin_id) {
        return Ok(());
    }
    // Newer than the version this change was made from, even if a peer's clock ran ahead.
    let after = store
        .synced_skin_version(skin_id)
        .map_err(err)?
        .map_or(0, |v| v.updated_at_unix_ms + 1);
    let payload = payload(store, skins, skin_id, (now_ms().max(after), own))?;
    store.enqueue_skin_row(&payload).map_err(err)
}

/// Sends every skin again (the first launch, or a peer that just started syncing skins), each under the
/// version this device already knows for it, so a re-push never makes an old version look new. A skin
/// with a newer pulled version not in the folder yet is skipped: the folder is older than that version,
/// and its author sends it. Skins moved to `.deleted/` go as deletions when their deletion is known here.
pub fn enqueue_all(store: &Store, skins: &UserSkinStore) -> Result<(), SyncError> {
    let Some(own) = store.own_device_id() else {
        return Ok(());
    };
    for skin_id in skins.skin_ids() {
        let version = match store.synced_skin_version(&skin_id).map_err(err)? {
            Some(v) if !v.applied => continue,
            Some(v) if !v.deleted => (v.updated_at_unix_ms, v.device_id),
            // Never synced, or deleted and since made here again: a new version from here.
            _ => (now_ms(), own.clone()),
        };
        let payload = payload(store, skins, &skin_id, version)?;
        store.enqueue_skin_row(&payload).map_err(err)?;
    }
    for skin_id in skins.deleted_ids() {
        if let Some(v) = store.synced_skin_version(&skin_id).map_err(err)? {
            if v.applied && v.deleted {
                let payload = payload(store, skins, &skin_id, (v.updated_at_unix_ms, v.device_id))?;
                store.enqueue_skin_row(&payload).map_err(err)?;
            }
        }
    }
    Ok(())
}

/// The bytes of an image by hash: pulled whole into the store, or one of this device's own, which lives
/// only in a skin folder.
fn image_bytes(
    store: &Store,
    skins: &UserSkinStore,
    sha: &str,
    local: &mut Option<HashMap<String, Vec<u8>>>,
) -> Result<Option<Vec<u8>>, SyncError> {
    if let Some(bytes) = store.synced_image(sha).map_err(err)? {
        return Ok(Some(bytes));
    }
    // Hashed once per reconcile, and only when a skin needs one of this device's images.
    let local = local.get_or_insert_with(|| {
        let mut found = HashMap::new();
        for id in skins.skin_ids() {
            let Ok(dir) = skins.folder(&id).map(|d| d.join("images")) else {
                continue;
            };
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if let Ok(data) = std::fs::read(entry.path()) {
                    found.entry(sha256_hex(&data)).or_insert(data);
                }
            }
        }
        found
    });
    Ok(local.get(sha).cloned())
}

/// Why a pulled skin can't be written; it is then left as it is here and marked seen, so it isn't
/// offered again until a newer version comes.
fn refuse(store: &Store, payload: &SkinSyncPayload, _why: &str) -> Result<(), SyncError> {
    store
        .mark_skin_applied(&payload.skin_id, payload.updated_at_unix_ms, &payload.device_id)
        .map_err(err)
}

/// Writes every pulled skin that is ready into the folder: the replaced local version stays in its
/// history (each save is a version), and a deletion moves the skin to `.deleted/`.
pub fn reconcile_synced_skins(
    store: &Store,
    skins: &UserSkinStore,
) -> Result<ReconcileReport, SyncError> {
    let mut report = ReconcileReport::default();
    let mut local = None;
    for payload in store.synced_skins_ready().map_err(err)? {
        let id = payload.skin_id.clone();
        if !is_user_id(&id) {
            refuse(store, &payload, "not a user skin")?;
            continue;
        }
        if payload.deleted {
            if skins.manifest(&id).is_some() {
                skins.delete(&id).map_err(err)?;
                report.deleted.push(id.clone());
            }
            store
                .mark_skin_applied(&id, payload.updated_at_unix_ms, &payload.device_id)
                .map_err(err)?;
            continue;
        }
        // The import rules: the loader's repairs (a voice over its limit, an unknown font or icon), the id
        // it claims, and image names that stay inside the skin's folder.
        let manifest = serde_json::to_vec(&payload.manifest).map_err(err)?;
        let Some((mut skin, _)) = parse_skin(&manifest, &whimbrel_default()) else {
            refuse(store, &payload, "the manifest can't be read")?;
            continue;
        };
        if skin.id != id {
            refuse(store, &payload, "the manifest names another skin")?;
            continue;
        }
        if payload.images.iter().any(|(name, _)| !is_image_name(name)) {
            refuse(store, &payload, "an image name is not a plain file name")?;
            continue;
        }
        let fallback = default_palette();
        let palettes: Vec<Palette> = payload
            .palettes
            .iter()
            .filter_map(|(_, v)| parse_palette(&serde_json::to_vec(v).ok()?, &fallback))
            .collect();
        let mut images = Vec::new();
        let mut missing = false;
        for (name, sha) in &payload.images {
            match image_bytes(store, skins, sha, &mut local)? {
                Some(bytes) => images.push((name.clone(), bytes)),
                None => missing = true,
            }
        }
        if missing {
            // One of this device's own images that is no longer anywhere here: try again later.
            continue;
        }
        // Only the images that came with it may be placed.
        skin.decoration
            .imagery
            .retain(|i| images.iter().any(|(name, _)| *name == i.asset));
        // `save` gates the palettes for the skin's layout, so a synced palette below 4.5:1 is repaired.
        skins.save(&skin, &palettes, &images).map_err(err)?;
        store
            .mark_skin_applied(&id, payload.updated_at_unix_ms, &payload.device_id)
            .map_err(err)?;
        report.changed.push(id);
    }
    Ok(report)
}
