//! Skin sync between two devices' skins folders (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md,
//! Task 14). Each device is a store and a skins folder; `transfer` stands in for the relay, applying one
//! device's outbox rows on the other exactly as `sync-engine`'s pull does.

use std::io::Cursor;

use memory::skin_sync::{SkinImageChunk, SkinSyncPayload};
use memory::Store;
use serde_json::json;
use skins::format::{Archetype, Image, Placement};
use skins::history;
use skins::palette::default_palette;
use skins::repair::{archetype_pairs, pairs_pass};
use skins::store::archetype_repair;
use skins::sync::{downscale, enqueue_all, enqueue_skin, reconcile_synced_skins, ReconcileReport};
use skins::{whimbrel_default, Skin, UserSkinStore};

struct Device {
    _dir: tempfile::TempDir,
    store: Store,
    skins: UserSkinStore,
}

fn device(id: &str) -> Device {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    store.set_own_device_id(id);
    let skins = UserSkinStore::new(dir.path().join("skins"));
    Device { _dir: dir, store, skins }
}

/// Every row `from` has enqueued and not yet sent, applied on `to`.
fn transfer(from: &Device, to: &Device) {
    let peer = from.store.own_device_id().unwrap();
    let far = i64::MAX / 4;
    loop {
        let batch = from.store.claim_due_outbox_batch(memory::OUTBOX_CONNECTOR, "test", far, far + 1, 100).unwrap();
        if batch.is_empty() {
            return;
        }
        for op in batch {
            match op.row_type.as_str() {
                "skin" => {
                    let payload: SkinSyncPayload = serde_json::from_str(&op.payload_json).unwrap();
                    to.store.apply_synced_skin(&peer, &payload).unwrap();
                }
                "skin_image_chunk" => {
                    let chunk: SkinImageChunk = serde_json::from_str(&op.payload_json).unwrap();
                    to.store.apply_synced_skin_chunk(&chunk).unwrap();
                }
                _ => {}
            }
            from.store.acknowledge_outbox_operation(&op.operation_id, "test", "sent", far).unwrap();
        }
    }
}

/// A clock tick, so the next save is strictly newer.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(3));
}

fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(width, height, image::Rgb(rgb));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A user skin with its own palette and a header image, saved on `d` and enqueued.
fn make(d: &Device, id: &str, name: &str) -> Skin {
    let mut skin = whimbrel_default();
    skin.id = id.into();
    skin.name = name.into();
    skin.palettes = vec![format!("{id}-p1")];
    skin.decoration.imagery = vec![Image { asset: "top.png".into(), placement: Placement::Header, text_free: false }];
    let mut palette = default_palette();
    palette.id = format!("{id}-p1");
    palette.name = "Navy".into();
    d.skins.save(&skin, &[palette], &[("top.png".into(), png(8, 4, [10, 20, 30]))]).unwrap();
    enqueue_skin(&d.store, &d.skins, id).unwrap();
    skin
}

fn loaded(d: &Device, id: &str) -> Option<Skin> {
    d.skins.list().into_iter().map(|(s, _)| s).find(|s| s.id == id)
}

fn rename(d: &Device, id: &str, name: &str) {
    d.skins.rename(id, name).unwrap();
    enqueue_skin(&d.store, &d.skins, id).unwrap();
}

#[test]
fn saved_on_a_appears_on_b_identical() {
    let (a, b) = (device("device-a"), device("device-b"));
    make(&a, "user-stadium-1", "Stadium");
    transfer(&a, &b);
    let report = reconcile_synced_skins(&b.store, &b.skins).unwrap();
    assert_eq!(report, ReconcileReport { changed: vec!["user-stadium-1".into()], deleted: vec![] });
    assert_eq!(loaded(&b, "user-stadium-1"), loaded(&a, "user-stadium-1"));
    assert_eq!(b.skins.palette_files("user-stadium-1"), a.skins.palette_files("user-stadium-1"));
    let read = |d: &Device| std::fs::read(d.skins.image_path("user-stadium-1", "top.png").unwrap()).unwrap();
    assert_eq!(read(&b), read(&a));
    // Applied: a second reconcile changes nothing.
    assert_eq!(reconcile_synced_skins(&b.store, &b.skins).unwrap(), ReconcileReport::default());
}

#[test]
fn rename_follows() {
    let (a, b) = (device("device-a"), device("device-b"));
    make(&a, "user-stadium-1", "Stadium");
    transfer(&a, &b);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    tick();
    rename(&a, "user-stadium-1", "Arena");
    transfer(&a, &b);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    assert_eq!(loaded(&b, "user-stadium-1").unwrap().name, "Arena");
}

#[test]
fn delete_moves_to_deleted_on_b() {
    let (a, b) = (device("device-a"), device("device-b"));
    make(&a, "user-stadium-1", "Stadium");
    transfer(&a, &b);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    tick();
    a.skins.delete("user-stadium-1").unwrap();
    enqueue_skin(&a.store, &a.skins, "user-stadium-1").unwrap();
    transfer(&a, &b);
    let report = reconcile_synced_skins(&b.store, &b.skins).unwrap();
    assert_eq!(report.deleted, ["user-stadium-1"]);
    assert!(loaded(&b, "user-stadium-1").is_none());
    assert_eq!(b.skins.deleted_ids(), ["user-stadium-1"]);
}

#[test]
fn edit_losing_to_delete_survives_in_deleted() {
    let (a, b) = (device("device-a"), device("device-b"));
    make(&a, "user-stadium-1", "Stadium");
    transfer(&a, &b);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    tick();
    // B edits, then A deletes later: the deletion wins, and B's edit is kept in `.deleted/`.
    rename(&b, "user-stadium-1", "Edited on B");
    tick();
    a.skins.delete("user-stadium-1").unwrap();
    enqueue_skin(&a.store, &a.skins, "user-stadium-1").unwrap();
    transfer(&a, &b);
    transfer(&b, &a);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    reconcile_synced_skins(&a.store, &a.skins).unwrap();
    assert!(loaded(&b, "user-stadium-1").is_none());
    let kept = b.skins.root().join(".deleted/user-stadium-1/skin.json");
    let name = serde_json::from_slice::<serde_json::Value>(&std::fs::read(kept).unwrap()).unwrap()["name"].clone();
    assert_eq!(name, "Edited on B");
    // A doesn't bring it back for the older edit.
    assert!(loaded(&a, "user-stadium-1").is_none());
}

#[test]
fn replaced_version_lands_in_history() {
    let (a, b) = (device("device-a"), device("device-b"));
    make(&a, "user-stadium-1", "Stadium");
    transfer(&a, &b);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    tick();
    rename(&b, "user-stadium-1", "B's name");
    tick();
    rename(&a, "user-stadium-1", "A's name");
    transfer(&a, &b);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    assert_eq!(loaded(&b, "user-stadium-1").unwrap().name, "A's name");
    // B's own version is the one before it, readable as a history version, so Undo brings it back.
    let dir = b.skins.folder("user-stadium-1").unwrap().join("history");
    let numbers = history::numbers(&dir);
    let before: history::Version = history::read(&dir, numbers[numbers.len() - 2]).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&before.manifest).unwrap();
    assert_eq!(manifest["name"], "B's name");
    assert!(b.skins.undo("user-stadium-1").unwrap());
    assert_eq!(loaded(&b, "user-stadium-1").unwrap().name, "B's name");
}

#[test]
fn concurrent_rename_converges() {
    let (a, b) = (device("device-a"), device("device-b"));
    make(&a, "user-stadium-1", "Stadium");
    transfer(&a, &b);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    tick();
    // Both rename within the same minute, before either hears of the other.
    rename(&a, "user-stadium-1", "Alpha");
    tick();
    rename(&b, "user-stadium-1", "Beta");
    transfer(&a, &b);
    transfer(&b, &a);
    reconcile_synced_skins(&a.store, &a.skins).unwrap();
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    assert_eq!(loaded(&a, "user-stadium-1").unwrap().name, "Beta");
    assert_eq!(loaded(&b, "user-stadium-1").unwrap().name, "Beta");
    // The other name is in A's history.
    let dir = a.skins.folder("user-stadium-1").unwrap().join("history");
    let names: Vec<String> = history::numbers(&dir)
        .into_iter()
        .map(|n| serde_json::from_slice::<serde_json::Value>(&history::read(&dir, n).unwrap().manifest).unwrap()["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"Alpha".to_string()), "{names:?}");
}

#[test]
fn synced_palette_failing_archetype_is_repaired() {
    // A palette that reads fine on its own but fails one layout's pairs.
    let archetype = Archetype::ALL
        .into_iter()
        .find(|&a| archetype_repair(&default_palette(), "x", a).is_some())
        .expect("Default fails at least one layout's pairs");
    let b = device("device-b");
    let mut manifest = serde_json::to_value(whimbrel_default()).unwrap();
    manifest["id"] = json!("user-x-1");
    manifest["name"] = json!("X");
    manifest["palettes"] = json!(["user-x-1-p1"]);
    manifest["today"]["archetype"] = serde_json::to_value(archetype).unwrap();
    let mut palette = default_palette();
    palette.id = "user-x-1-p1".into();
    palette.name = "Plain".into();
    let payload = SkinSyncPayload {
        skin_id: "user-x-1".into(),
        name: "X".into(),
        manifest,
        palettes: vec![("user-x-1-p1".into(), palette.to_json())],
        images: vec![],
        updated_at_unix_ms: 1,
        device_id: "device-a".into(),
        deleted: false,
    };
    b.store.apply_synced_skin("device-a", &payload).unwrap();
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    let (skin, palettes) = b.skins.list().remove(0);
    assert_eq!(skin.today.archetype, archetype);
    let pairs = archetype_pairs(archetype);
    for p in &palettes {
        assert!(pairs_pass(&p.light, &pairs) && pairs_pass(&p.dark, &pairs), "{} still fails", p.id);
    }
}

#[test]
fn hostile_payload_refused() {
    let b = device("device-b");
    let base = |id: &str| {
        let mut manifest = serde_json::to_value(whimbrel_default()).unwrap();
        manifest["id"] = json!(id);
        manifest["name"] = json!("Hostile");
        SkinSyncPayload {
            skin_id: id.into(),
            name: "Hostile".into(),
            manifest,
            palettes: vec![],
            images: vec![],
            updated_at_unix_ms: 1,
            device_id: "device-a".into(),
            deleted: false,
        }
    };
    // A path in an image name: the skin is refused and nothing is written.
    let mut path = base("user-path-1");
    path.images = vec![("../escape.png".into(), memory::skin_sync::sha256_hex(b"x"))];
    b.store.apply_synced_skin("device-a", &path).unwrap();
    // The manifest naming a different skin than the row.
    let mut other = base("user-other-1");
    other.manifest["id"] = json!("user-someone-else-1");
    b.store.apply_synced_skin("device-a", &other).unwrap();
    // A built-in's id never gets in.
    assert!(!b.store.apply_synced_skin("device-a", &base("whimbrel")).unwrap());
    // A voice over its limit is repaired, not refused.
    let mut loud = base("user-loud-1");
    loud.manifest["voice"]["tagline"] = json!("x".repeat(500));
    b.store.apply_synced_skin("device-a", &loud).unwrap();
    let report = reconcile_synced_skins(&b.store, &b.skins).unwrap();
    assert_eq!(report.changed, ["user-loud-1"]);
    assert_eq!(b.skins.skin_ids(), ["user-loud-1"]);
    let tagline = loaded(&b, "user-loud-1").unwrap().voice.0.get(&skins::format::VoiceKey::Tagline).cloned().unwrap_or_default();
    assert!(tagline.chars().count() <= 60, "{}", tagline.len());
    // Refused ones aren't offered again.
    assert!(b.store.synced_skins_ready().unwrap().is_empty());
}

#[test]
fn builtins_never_enqueued() {
    let a = device("device-a");
    enqueue_skin(&a.store, &a.skins, "whimbrel").unwrap();
    enqueue_skin(&a.store, &a.skins, "game-day").unwrap();
    assert!(a.store.list_outbox_operations().unwrap().is_empty());
}

#[test]
fn enqueue_all_keeps_each_version_and_resends_images() {
    let (a, b, c) = (device("device-a"), device("device-b"), device("device-c"));
    make(&a, "user-stadium-1", "Stadium");
    transfer(&a, &b);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    // B re-pushes everything (say, C just started syncing skins): A's skin goes under A's version.
    b.store.record_skin_peer("device-c").unwrap();
    assert!(b.store.take_skin_repush_request().unwrap());
    enqueue_all(&b.store, &b.skins).unwrap();
    let rows = b.store.list_outbox_operations().unwrap();
    let skin_row: SkinSyncPayload = serde_json::from_str(&rows.iter().find(|r| r.1 == "skin").unwrap().3).unwrap();
    assert_eq!(skin_row.device_id, "device-a");
    // C gets the skin, but its image is A's to send: C waits for it.
    transfer(&b, &c);
    assert!(reconcile_synced_skins(&c.store, &c.skins).unwrap().changed.is_empty());
    transfer(&a, &c);
    a.store.record_skin_peer("device-c").unwrap();
    a.store.take_skin_repush_request().unwrap();
    enqueue_all(&a.store, &a.skins).unwrap();
    transfer(&a, &c);
    assert_eq!(reconcile_synced_skins(&c.store, &c.skins).unwrap().changed, ["user-stadium-1"]);
}

#[test]
fn a_repush_never_hides_a_pulled_edit_not_yet_written() {
    let (a, b) = (device("device-a"), device("device-b"));
    make(&a, "user-stadium-1", "Stadium");
    transfer(&a, &b);
    reconcile_synced_skins(&b.store, &b.skins).unwrap();
    tick();
    rename(&b, "user-stadium-1", "Edited on B");
    // A has B's newer version, not yet written into its folder, when it re-pushes everything.
    transfer(&b, &a);
    a.store.record_skin_peer("device-c").unwrap();
    a.store.take_skin_repush_request().unwrap();
    let sent_before = a.store.list_outbox_operations().unwrap().len();
    enqueue_all(&a.store, &a.skins).unwrap();
    let rows = a.store.list_outbox_operations().unwrap();
    assert!(
        rows[sent_before..].iter().all(|r| r.1 != "skin"),
        "A's older folder must not go out under B's version"
    );
    // B's edit is still offered, and lands.
    assert_eq!(a.store.synced_skins_ready().unwrap().len(), 1);
    reconcile_synced_skins(&a.store, &a.skins).unwrap();
    assert_eq!(loaded(&a, "user-stadium-1").unwrap().name, "Edited on B");
}

#[test]
fn downscale_caps_long_side_at_1600() {
    let out = downscale(&png(2400, 600, [1, 2, 3]), 1600).unwrap();
    let img = image::load_from_memory(&out).unwrap();
    assert_eq!((img.width(), img.height()), (1600, 400));
}
