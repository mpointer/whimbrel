//! Skin sync's local side (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md, Task 13).
//!
//! A user skin travels as one `skin` row (its manifest, palettes and the hashes of its images) and its
//! images as `skin_image_chunk` rows, sent once per image however many skins use it. Pulled rows land
//! here, not in the skins folder: a skin is held until every image it names is whole, then
//! `synced_skins_ready` hands it to `core/skins` (Task 14), which writes the folder and calls
//! `mark_skin_applied`. The newest version wins by `(updated_at_unix_ms, device_id)`, and this device's
//! own saves are recorded the same way, so an older edit from a peer never replaces a newer local one.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::sync_outbox::{self, NewOutboxOperation, OutboxOperationKind};
use crate::{json_to_sql_error, now_unix_ms, Store, OUTBOX_CONNECTOR, OUTBOX_ROW_TYPE_SKIN, OUTBOX_ROW_TYPE_SKIN_IMAGE_CHUNK};

/// Raw bytes per image chunk. ponytail: Turso documents no row limit below SQLite's own
/// (`SQLITE_MAX_LENGTH`, 1 GB by default), but every row is one HTTP request body; 192 KB is
/// 256 KB once base64-encoded and sealed, which keeps each request small. Raise it only after
/// checking the relay's request size limit.
pub const SKIN_CHUNK_BYTES: usize = 192 * 1024;
/// An image is at most the package limit (64 MB), so at most this many chunks.
pub const SKIN_IMAGE_MAX_CHUNKS: u32 = (64 * 1024 * 1024 / SKIN_CHUNK_BYTES as u32) + 1;
/// A skin row's payload beyond this is refused (a manifest and a few palettes are a few KB).
const SKIN_PAYLOAD_MAX_BYTES: usize = 1024 * 1024;

/// One user skin on the wire. Chat, history and the selected skin never sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinSyncPayload {
    pub skin_id: String,
    pub name: String,
    pub manifest: serde_json::Value,
    /// `(palette id, palette file)`.
    #[serde(default)]
    pub palettes: Vec<(String, serde_json::Value)>,
    /// `(image file name, sha256 hex)`.
    #[serde(default)]
    pub images: Vec<(String, String)>,
    pub updated_at_unix_ms: i64,
    pub device_id: String,
    #[serde(default)]
    pub deleted: bool,
}

/// A skin's newest known version (`Store::synced_skin_version`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinVersion {
    pub updated_at_unix_ms: i64,
    /// The device that saved it.
    pub device_id: String,
    pub deleted: bool,
    /// Whether the skins folder holds this version. A pulled one waits until `mark_skin_applied`.
    pub applied: bool,
}

/// One piece of an image: `data` is base64 of at most `SKIN_CHUNK_BYTES` raw bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinImageChunk {
    pub sha256: String,
    pub index: u32,
    pub count: u32,
    pub data: String,
}

pub(crate) fn ensure_skin_sync_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS synced_skin (
            skin_id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL,
            updated_at_unix_ms INTEGER NOT NULL,
            device_id TEXT NOT NULL,
            deleted INTEGER NOT NULL DEFAULT 0,
            applied_updated_at_unix_ms INTEGER,
            applied_device_id TEXT
        );
        CREATE TABLE IF NOT EXISTS synced_skin_image (
            sha256 TEXT PRIMARY KEY,
            count INTEGER NOT NULL,
            complete INTEGER NOT NULL DEFAULT 0,
            sent INTEGER NOT NULL DEFAULT 0,
            own INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS synced_skin_chunk (
            sha256 TEXT NOT NULL,
            idx INTEGER NOT NULL,
            data BLOB NOT NULL,
            PRIMARY KEY (sha256, idx)
        );
        CREATE TABLE IF NOT EXISTS synced_skin_peer (
            device_id TEXT PRIMARY KEY,
            repush INTEGER NOT NULL DEFAULT 0
        );",
    )
}

/// The lower-case hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `user-` then `a-z0-9-`: the only skins that sync (the built-ins ship with every app).
fn is_user_skin_id(id: &str) -> bool {
    id.len() > 5 && id.starts_with("user-") && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Store {
    /// Records this device's current version of a skin (it is what the folder holds) and enqueues it.
    /// Its images must be enqueued with `enqueue_skin_chunks` too. A store with no device id does neither.
    pub fn enqueue_skin_row(&self, payload: &SkinSyncPayload) -> rusqlite::Result<()> {
        let Some(own) = self.own_device_id.borrow().clone() else { return Ok(()) };
        if !is_user_skin_id(&payload.skin_id) {
            return Ok(());
        }
        let json = serde_json::to_string(payload).map_err(json_to_sql_error)?;
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO synced_skin (skin_id, payload_json, updated_at_unix_ms, device_id, deleted, applied_updated_at_unix_ms, applied_device_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?3, ?4) \
             ON CONFLICT (skin_id) DO UPDATE SET payload_json = ?2, updated_at_unix_ms = ?3, device_id = ?4, deleted = ?5, \
             applied_updated_at_unix_ms = ?3, applied_device_id = ?4",
            params![payload.skin_id, json, payload.updated_at_unix_ms, payload.device_id, payload.deleted],
        )?;
        let local_seq = Self::next_local_seq_tx(&tx)?;
        sync_outbox::enqueue_outbox_tx(
            &tx,
            NewOutboxOperation {
                device_id: own,
                local_seq,
                row_type: OUTBOX_ROW_TYPE_SKIN.to_string(),
                aggregate_type: "skin".to_string(),
                aggregate_id: payload.skin_id.clone(),
                operation_kind: if payload.deleted { OutboxOperationKind::Tombstone } else { OutboxOperationKind::Upsert },
                payload_json: json,
                connector: OUTBOX_CONNECTOR.to_string(),
            },
            now_unix_ms(),
        )?;
        tx.commit()
    }

    /// Enqueues an image's chunks, unless it was sent before or arrived from a peer (so an image shared by
    /// two skins goes once). Returns whether it enqueued. `sha256` must be the bytes' own hash.
    pub fn enqueue_skin_chunks(&self, sha256: &str, bytes: &[u8]) -> rusqlite::Result<bool> {
        let Some(own) = self.own_device_id.borrow().clone() else { return Ok(false) };
        if sha256_hex(bytes) != sha256 {
            return Err(rusqlite::Error::InvalidParameterName(format!("{sha256} is not the hash of these bytes")));
        }
        let known: Option<(bool, bool)> = self
            .conn
            .query_row("SELECT complete, sent FROM synced_skin_image WHERE sha256 = ?1", params![sha256], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if known.is_some_and(|(complete, sent)| complete || sent) {
            return Ok(false);
        }
        let pieces: Vec<&[u8]> = if bytes.is_empty() { vec![&[][..]] } else { bytes.chunks(SKIN_CHUNK_BYTES).collect() };
        let count = pieces.len() as u32;
        let tx = self.conn.unchecked_transaction()?;
        for (index, piece) in pieces.into_iter().enumerate() {
            let chunk = SkinImageChunk { sha256: sha256.to_string(), index: index as u32, count, data: STANDARD.encode(piece) };
            let local_seq = Self::next_local_seq_tx(&tx)?;
            sync_outbox::enqueue_outbox_tx(
                &tx,
                NewOutboxOperation {
                    device_id: own.clone(),
                    local_seq,
                    row_type: OUTBOX_ROW_TYPE_SKIN_IMAGE_CHUNK.to_string(),
                    aggregate_type: "skin_image".to_string(),
                    aggregate_id: format!("{sha256}:{index}"),
                    operation_kind: OutboxOperationKind::Upsert,
                    payload_json: serde_json::to_string(&chunk).map_err(json_to_sql_error)?,
                    connector: OUTBOX_CONNECTOR.to_string(),
                },
                now_unix_ms(),
            )?;
        }
        tx.execute(
            "INSERT INTO synced_skin_image (sha256, count, complete, sent, own) VALUES (?1, ?2, 0, 1, 1) \
             ON CONFLICT (sha256) DO UPDATE SET sent = 1, own = 1",
            params![sha256, count],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// A skin pulled from a peer: kept when it is newer than what this device has (by
    /// `(updated_at_unix_ms, device_id)`), else ignored. Returns whether it was kept. Never enqueued.
    /// `device_id` is the device that saved this version, which a re-push by another peer keeps.
    pub fn apply_synced_skin(&self, _peer_device_id: &str, payload: &SkinSyncPayload) -> rusqlite::Result<bool> {
        if !is_user_skin_id(&payload.skin_id) {
            return Ok(false);
        }
        let json = serde_json::to_string(payload).map_err(json_to_sql_error)?;
        if json.len() > SKIN_PAYLOAD_MAX_BYTES || payload.images.iter().any(|(_, sha)| !is_sha256_hex(sha)) {
            return Ok(false);
        }
        let current: Option<(i64, String)> = self
            .conn
            .query_row("SELECT updated_at_unix_ms, device_id FROM synced_skin WHERE skin_id = ?1", params![payload.skin_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if current.is_some_and(|(at, device)| (at, device.as_str()) >= (payload.updated_at_unix_ms, payload.device_id.as_str())) {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO synced_skin (skin_id, payload_json, updated_at_unix_ms, device_id, deleted) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (skin_id) DO UPDATE SET payload_json = ?2, updated_at_unix_ms = ?3, device_id = ?4, deleted = ?5",
            params![payload.skin_id, json, payload.updated_at_unix_ms, payload.device_id, payload.deleted],
        )?;
        Ok(true)
    }

    /// One pulled image chunk. When the last one arrives the image is checked against its hash: whole, it
    /// is complete; otherwise its chunks are dropped (a peer re-sending the image starts it again).
    pub fn apply_synced_skin_chunk(&self, chunk: &SkinImageChunk) -> rusqlite::Result<()> {
        if !is_sha256_hex(&chunk.sha256) || chunk.count == 0 || chunk.count > SKIN_IMAGE_MAX_CHUNKS || chunk.index >= chunk.count {
            return Ok(());
        }
        let Ok(data) = STANDARD.decode(&chunk.data) else { return Ok(()) };
        if data.len() > SKIN_CHUNK_BYTES {
            return Ok(());
        }
        let known: Option<(u32, bool, bool)> = self
            .conn
            .query_row("SELECT count, complete, own FROM synced_skin_image WHERE sha256 = ?1", params![chunk.sha256], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        match known {
            Some((_, true, _)) | Some((_, _, true)) => return Ok(()),
            Some((count, _, _)) if count != chunk.count => return Ok(()),
            _ => {}
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("INSERT OR IGNORE INTO synced_skin_image (sha256, count) VALUES (?1, ?2)", params![chunk.sha256, chunk.count])?;
        tx.execute("INSERT OR IGNORE INTO synced_skin_chunk (sha256, idx, data) VALUES (?1, ?2, ?3)", params![chunk.sha256, chunk.index, data])?;
        let have: u32 = tx.query_row("SELECT COUNT(*) FROM synced_skin_chunk WHERE sha256 = ?1", params![chunk.sha256], |r| r.get(0))?;
        if have == chunk.count {
            let whole = assemble(&tx, &chunk.sha256)?;
            if sha256_hex(&whole) == chunk.sha256 {
                tx.execute("UPDATE synced_skin_image SET complete = 1 WHERE sha256 = ?1", params![chunk.sha256])?;
            } else {
                tx.execute("DELETE FROM synced_skin_chunk WHERE sha256 = ?1", params![chunk.sha256])?;
                tx.execute("DELETE FROM synced_skin_image WHERE sha256 = ?1", params![chunk.sha256])?;
            }
        }
        tx.commit()
    }

    /// Skins pulled since the folder last changed, newer than what it holds, with every image they name
    /// whole (or this device's own). A deletion is always ready.
    pub fn synced_skins_ready(&self) -> rusqlite::Result<Vec<SkinSyncPayload>> {
        let mut stmt = self.conn.prepare(
            "SELECT payload_json FROM synced_skin \
             WHERE applied_updated_at_unix_ms IS NULL OR applied_updated_at_unix_ms != updated_at_unix_ms OR applied_device_id != device_id \
             ORDER BY skin_id",
        )?;
        let payloads: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut ready = Vec::new();
        for json in payloads {
            let Ok(payload) = serde_json::from_str::<SkinSyncPayload>(&json) else { continue };
            let mut whole = true;
            if !payload.deleted {
                for (_, sha) in &payload.images {
                    let have: bool = self
                        .conn
                        .query_row("SELECT complete OR own FROM synced_skin_image WHERE sha256 = ?1", params![sha], |r| r.get(0))
                        .optional()?
                        .unwrap_or(false);
                    if !have {
                        whole = false;
                        break;
                    }
                }
            }
            if whole {
                ready.push(payload);
            }
        }
        Ok(ready)
    }

    /// A whole image pulled from a peer. `None` for one still arriving, and for this device's own images,
    /// which live only in the skins folder (look there by hash).
    pub fn synced_image(&self, sha256: &str) -> rusqlite::Result<Option<Vec<u8>>> {
        let complete: bool = self
            .conn
            .query_row("SELECT complete FROM synced_skin_image WHERE sha256 = ?1", params![sha256], |r| r.get(0))
            .optional()?
            .unwrap_or(false);
        if !complete {
            return Ok(None);
        }
        Ok(Some(assemble(&self.conn, sha256)?))
    }

    /// The newest version of `skin_id` this device knows, saved here or pulled. A re-push sends a skin under
    /// it, so the version keeps the device that saved it, but only once the folder holds it (`applied`).
    pub fn synced_skin_version(&self, skin_id: &str) -> rusqlite::Result<Option<SkinVersion>> {
        self.conn
            .query_row(
                "SELECT updated_at_unix_ms, device_id, deleted,                  COALESCE(applied_updated_at_unix_ms = updated_at_unix_ms AND applied_device_id = device_id, 0)                  FROM synced_skin WHERE skin_id = ?1",
                params![skin_id],
                |r| Ok(SkinVersion { updated_at_unix_ms: r.get(0)?, device_id: r.get(1)?, deleted: r.get(2)?, applied: r.get(3)? }),
            )
            .optional()
    }

    /// The skins folder now holds `skin_id` at this version, so `synced_skins_ready` stops offering it.
    pub fn mark_skin_applied(&self, skin_id: &str, updated_at_unix_ms: i64, device_id: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE synced_skin SET applied_updated_at_unix_ms = ?2, applied_device_id = ?3 WHERE skin_id = ?1",
            params![skin_id, updated_at_unix_ms, device_id],
        )?;
        Ok(())
    }

    /// Notes that `peer_device_id` has announced it syncs skins. The first time, a full re-push is
    /// requested (`take_skin_repush_request`), so a peer that just updated gets every skin made before.
    pub fn record_skin_peer(&self, peer_device_id: &str) -> rusqlite::Result<bool> {
        let added = self.conn.execute("INSERT OR IGNORE INTO synced_skin_peer (device_id, repush) VALUES (?1, 1)", params![peer_device_id])?;
        Ok(added > 0)
    }

    /// True once after a peer's skin marker is first seen: the app should enqueue every skin again. Its own
    /// images count as unsent again, so `enqueue_skin_chunks` resends them: a peer that updated after
    /// pulling past them (an older build skips skin rows) has its cursor beyond the first copies. Images
    /// that came from peers stay as they are; the device that made them resends them.
    pub fn take_skin_repush_request(&self) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute("UPDATE synced_skin_peer SET repush = 0 WHERE repush = 1", [])?;
        if changed > 0 {
            tx.execute("UPDATE synced_skin_image SET sent = 0 WHERE own = 1", [])?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    /// True the first time it is asked on this store: the app should enqueue every skin it has once, so
    /// skins made before sync existed reach the other devices.
    pub fn take_skin_first_push(&self) -> rusqlite::Result<bool> {
        let added = self.conn.execute(
            "INSERT OR IGNORE INTO kv_meta (key, value) VALUES ('skin_sync_first_push_done', '1')",
            [],
        )?;
        Ok(added > 0)
    }

    /// Whether `peer_device_id`'s skin marker has been seen, so the relay need not be asked again.
    pub fn is_skin_peer(&self, peer_device_id: &str) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM synced_skin_peer WHERE device_id = ?1", params![peer_device_id], |_| Ok(()))
            .optional()?
            .is_some())
    }
}

/// The image's chunks in order, joined.
fn assemble(conn: &Connection, sha256: &str) -> rusqlite::Result<Vec<u8>> {
    let mut stmt = conn.prepare("SELECT data FROM synced_skin_chunk WHERE sha256 = ?1 ORDER BY idx")?;
    let parts: Vec<Vec<u8>> = stmt.query_map(params![sha256], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(parts.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(device: &str) -> Store {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id(device);
        store
    }

    fn payload(id: &str, at: i64, device: &str, images: &[(&str, &str)]) -> SkinSyncPayload {
        SkinSyncPayload {
            skin_id: id.into(),
            name: "Stadium".into(),
            manifest: serde_json::json!({ "id": id, "name": "Stadium" }),
            palettes: vec![],
            images: images.iter().map(|(n, s)| (n.to_string(), s.to_string())).collect(),
            updated_at_unix_ms: at,
            device_id: device.into(),
            deleted: false,
        }
    }

    fn outbox(store: &Store, row_type: &str) -> Vec<String> {
        let mut stmt = store.conn.prepare("SELECT payload_json FROM sync_outbox WHERE row_type = ?1 ORDER BY local_seq").unwrap();
        stmt.query_map(params![row_type], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    fn chunks_of(store: &Store) -> Vec<SkinImageChunk> {
        outbox(store, OUTBOX_ROW_TYPE_SKIN_IMAGE_CHUNK).iter().map(|j| serde_json::from_str(j).unwrap()).collect()
    }

    #[test]
    fn older_skin_loses_to_newer() {
        let b = store("device-b");
        assert!(b.apply_synced_skin("device-a", &payload("user-x-1", 200, "device-a", &[])).unwrap());
        // Older, or the same version again: ignored.
        assert!(!b.apply_synced_skin("device-a", &payload("user-x-1", 100, "device-a", &[])).unwrap());
        assert!(!b.apply_synced_skin("device-a", &payload("user-x-1", 200, "device-a", &[])).unwrap());
        // A tie on time goes to the larger device id, so every device picks the same one.
        assert!(b.apply_synced_skin("device-c", &payload("user-x-1", 200, "device-c", &[])).unwrap());
        // A local save newer than a peer's version keeps winning.
        b.enqueue_skin_row(&payload("user-x-1", 300, "device-b", &[])).unwrap();
        assert!(!b.apply_synced_skin("device-a", &payload("user-x-1", 250, "device-a", &[])).unwrap());
        // What this device saved itself is already in the folder.
        assert!(b.synced_skins_ready().unwrap().is_empty());
    }

    #[test]
    fn refuses_what_is_not_a_user_skin() {
        let b = store("device-b");
        assert!(!b.apply_synced_skin("device-a", &payload("whimbrel", 1, "device-a", &[])).unwrap());
        assert!(!b.apply_synced_skin("device-a", &payload("user-../x", 1, "device-a", &[])).unwrap());
        assert!(!b.apply_synced_skin("device-a", &payload("user-x-1", 1, "device-a", &[("a.png", "nothex")])).unwrap());
    }

    #[test]
    fn image_sent_once_for_two_skins() {
        let a = store("device-a");
        let bytes = vec![7u8; SKIN_CHUNK_BYTES * 2 + 5];
        let sha = sha256_hex(&bytes);
        assert!(a.enqueue_skin_chunks(&sha, &bytes).unwrap());
        assert!(!a.enqueue_skin_chunks(&sha, &bytes).unwrap());
        let chunks = chunks_of(&a);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.count == 3));
        assert!(a.enqueue_skin_chunks(&"0".repeat(64), &bytes).is_err(), "the hash must match the bytes");
    }

    #[test]
    fn partial_chunks_hold_skin() {
        let a = store("device-a");
        let bytes: Vec<u8> = (0..SKIN_CHUNK_BYTES * 2 + 9).map(|i| (i % 251) as u8).collect();
        let sha = sha256_hex(&bytes);
        a.enqueue_skin_chunks(&sha, &bytes).unwrap();
        let chunks = chunks_of(&a);
        let b = store("device-b");
        b.apply_synced_skin("device-a", &payload("user-x-1", 1, "device-a", &[("big.png", &sha)])).unwrap();
        b.apply_synced_skin_chunk(&chunks[0]).unwrap();
        b.apply_synced_skin_chunk(&chunks[2]).unwrap();
        // Two of three: held, and nothing half-written is offered.
        assert!(b.synced_skins_ready().unwrap().is_empty());
        assert_eq!(b.synced_image(&sha).unwrap(), None);
        b.apply_synced_skin_chunk(&chunks[1]).unwrap();
        let ready = b.synced_skins_ready().unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(b.synced_image(&sha).unwrap().as_deref(), Some(&bytes[..]));
        // Applied: no longer offered, until a newer version arrives.
        b.mark_skin_applied("user-x-1", 1, "device-a").unwrap();
        assert!(b.synced_skins_ready().unwrap().is_empty());
        b.apply_synced_skin("device-a", &payload("user-x-1", 2, "device-a", &[("big.png", &sha)])).unwrap();
        assert_eq!(b.synced_skins_ready().unwrap().len(), 1);
    }

    #[test]
    fn a_corrupt_image_is_dropped_whole() {
        let b = store("device-b");
        let sha = sha256_hex(b"the real bytes");
        let forged = SkinImageChunk { sha256: sha.clone(), index: 0, count: 1, data: STANDARD.encode(b"other bytes") };
        b.apply_synced_skin_chunk(&forged).unwrap();
        assert_eq!(b.synced_image(&sha).unwrap(), None);
        let real = SkinImageChunk { data: STANDARD.encode(b"the real bytes"), ..forged };
        b.apply_synced_skin_chunk(&real).unwrap();
        assert_eq!(b.synced_image(&sha).unwrap().as_deref(), Some(&b"the real bytes"[..]));
        // Out-of-range pieces are ignored.
        b.apply_synced_skin_chunk(&SkinImageChunk { sha256: sha256_hex(b"x"), index: 3, count: 2, data: String::new() }).unwrap();
    }

    #[test]
    fn a_deletion_is_ready_without_its_images() {
        let b = store("device-b");
        let mut gone = payload("user-x-1", 5, "device-a", &[("a.png", &sha256_hex(b"never sent"))]);
        gone.deleted = true;
        b.apply_synced_skin("device-a", &gone).unwrap();
        assert_eq!(b.synced_skins_ready().unwrap(), vec![gone]);
    }

    #[test]
    fn a_new_peer_marker_requests_one_repush() {
        let b = store("device-b");
        assert!(!b.take_skin_repush_request().unwrap());
        assert!(b.record_skin_peer("device-a").unwrap());
        assert!(!b.record_skin_peer("device-a").unwrap());
        assert!(b.take_skin_repush_request().unwrap());
        assert!(!b.take_skin_repush_request().unwrap());
        assert!(b.is_skin_peer("device-a").unwrap());
        assert!(!b.is_skin_peer("device-z").unwrap());
    }

    #[test]
    fn a_repush_sends_own_images_again_but_not_received_ones() {
        let a = store("device-a");
        let own = b"made here".to_vec();
        a.enqueue_skin_chunks(&sha256_hex(&own), &own).unwrap();
        let received = b"made elsewhere";
        a.apply_synced_skin_chunk(&SkinImageChunk { sha256: sha256_hex(received), index: 0, count: 1, data: STANDARD.encode(received) }).unwrap();
        a.record_skin_peer("device-b").unwrap();
        assert!(a.take_skin_repush_request().unwrap());
        assert!(a.enqueue_skin_chunks(&sha256_hex(&own), &own).unwrap(), "own image resent");
        assert!(!a.enqueue_skin_chunks(&sha256_hex(received), received).unwrap(), "a received image is its maker's to resend");
        assert_eq!(chunks_of(&a).len(), 2);
    }

    #[test]
    fn the_first_push_is_asked_for_once() {
        let s = store("device-a");
        assert!(s.take_skin_first_push().unwrap());
        assert!(!s.take_skin_first_push().unwrap());
    }

    #[test]
    fn a_store_with_no_device_id_enqueues_nothing() {
        let s = Store::open_in_memory().unwrap();
        s.enqueue_skin_row(&payload("user-x-1", 1, "d", &[])).unwrap();
        assert!(!s.enqueue_skin_chunks(&sha256_hex(b"x"), b"x").unwrap());
        assert!(outbox(&s, OUTBOX_ROW_TYPE_SKIN).is_empty());
    }
}
