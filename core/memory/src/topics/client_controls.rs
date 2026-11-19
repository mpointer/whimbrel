//! Per-client settings: keep a client's meetings away from hosted AI, and how long to keep
//! them. Stored per seed (`client_control`) and synced inside the seed record
//! (`TopicSeedPayload`). Callers that enforce the setting must treat an unreadable setting
//! as "every client excluded" (`exclusion_unreadable`).

use rusqlite::{params, Connection, OptionalExtension};

use super::seeds::{enqueue_seed, seed_id, TopicSeedPayload};
use crate::Store;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientControl {
    pub exclude_hosted: bool,
    pub keep_months: Option<u32>,
}

/// Material global retention leaves alone because its client keeps it for its own period.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientKeptMaterial {
    pub meetings: Vec<i64>,
    /// Captures by `items.id`.
    pub capture_items: Vec<i64>,
    /// The same captures' legacy `capture_item` twins.
    pub capture_item_rows: Vec<i64>,
}

/// Retention's predicates (`?1` the cutoff) with the kept material left out.
pub(crate) struct RetentionPredicates {
    pub meetings: String,
    pub items: String,
    pub capture_item: String,
}

impl ClientKeptMaterial {
    pub(crate) fn predicates(&self) -> RetentionPredicates {
        // Row ids only (integers), so the list is safe to write into the SQL.
        let not_in = |ids: &[i64]| {
            if ids.is_empty() {
                String::new()
            } else {
                let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
                format!(" AND id NOT IN ({list})")
            }
        };
        RetentionPredicates {
            meetings: format!("{}{}", crate::EXPIRED_MEETINGS, not_in(&self.meetings)),
            items: format!("{}{}", crate::EXPIRED_CAPTURE_ITEMS, not_in(&self.capture_items)),
            capture_item: format!("{}{}", crate::EXPIRED_CAPTURE_ITEMS, not_in(&self.capture_item_rows)),
        }
    }
}

/// The stored row for `key`; `None` when the client has no row (all defaults). Errors when
/// the table or the row cannot be read.
pub(crate) fn read_row(conn: &Connection, key: &str) -> rusqlite::Result<Option<ClientControl>> {
    conn.query_row(
        "SELECT exclude_hosted, keep_months FROM client_control WHERE topic_key = ?1",
        [key],
        |r| {
            Ok(ClientControl {
                exclude_hosted: r.get::<_, i64>(0)? != 0,
                keep_months: r.get::<_, Option<u32>>(1)?,
            })
        },
    )
    .optional()
}

/// The stored row for `key` with its timestamp.
pub(crate) fn read_row_ts(
    conn: &Connection,
    key: &str,
) -> rusqlite::Result<Option<(ClientControl, i64)>> {
    conn.query_row(
        "SELECT exclude_hosted, keep_months, updated_at_unix_ms FROM client_control WHERE topic_key = ?1",
        [key],
        |r| {
            Ok((
                ClientControl {
                    exclude_hosted: r.get::<_, i64>(0)? != 0,
                    keep_months: r.get::<_, Option<u32>>(1)?,
                },
                r.get(2)?,
            ))
        },
    )
    .optional()
}

/// Applies the control fields of a pulled seed record, last write wins on the control's own
/// timestamp (never the seed's); on an exact tie, excluded wins. Records without a control
/// timestamp (older devices) or without `exclude_hosted` change nothing.
pub(crate) fn apply_synced(conn: &Connection, p: &TopicSeedPayload) -> rusqlite::Result<()> {
    let (Some(excl), Some(ts)) = (p.exclude_hosted, p.control_updated_at_unix_ms) else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO client_control (topic_key, exclude_hosted, keep_months, updated_at_unix_ms) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(topic_key) DO UPDATE SET exclude_hosted = excluded.exclude_hosted, \
           keep_months = excluded.keep_months, updated_at_unix_ms = excluded.updated_at_unix_ms \
         WHERE excluded.updated_at_unix_ms > client_control.updated_at_unix_ms \
            OR (excluded.updated_at_unix_ms = client_control.updated_at_unix_ms \
                AND excluded.exclude_hosted != 0 AND client_control.exclude_hosted = 0)",
        params![p.key, excl, p.keep_months, ts],
    )?;
    Ok(())
}

/// A merge of seed `from_key` into `to_key`: the survivor is excluded if either was. The
/// control timestamp moves forward (at least `ts`) so the change syncs.
pub(crate) fn carry_exclusion(
    conn: &Connection,
    from_key: &str,
    to_key: &str,
    ts: i64,
) -> rusqlite::Result<()> {
    let from = read_row(conn, from_key)?.unwrap_or_default();
    if !from.exclude_hosted {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO client_control (topic_key, exclude_hosted, keep_months, updated_at_unix_ms) \
         VALUES (?1, 1, NULL, ?2) \
         ON CONFLICT(topic_key) DO UPDATE SET exclude_hosted = 1, \
           updated_at_unix_ms = MAX(?2, client_control.updated_at_unix_ms + 1)",
        params![to_key, ts],
    )?;
    Ok(())
}

impl Store {
    /// The settings for seed `key`; defaults when none were set.
    pub fn client_control(&self, key: &str) -> rusqlite::Result<ClientControl> {
        Ok(read_row(&self.conn, key)?.unwrap_or_default())
    }

    /// Saves the settings for seed `key` and queues the seed record for sync. Only the control's own timestamp
    /// moves, never the seed's: the record goes out with the seed fields as this device last had them, and a peer
    /// that has a newer name or alias list keeps it and takes only the control fields (`apply_synced`; X6 final
    /// review, Major 9). A device on an older build still bumps the seed, so its control write can undo a newer
    /// edit made elsewhere until it updates.
    pub fn set_client_control(&self, key: &str, c: &ClientControl) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let id = seed_id(&tx, key)?;
        let ts = crate::now_unix_ms();
        tx.execute(
            "INSERT INTO client_control (topic_key, exclude_hosted, keep_months, updated_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(topic_key) DO UPDATE SET exclude_hosted = excluded.exclude_hosted, \
               keep_months = excluded.keep_months, updated_at_unix_ms = MAX(excluded.updated_at_unix_ms, client_control.updated_at_unix_ms + 1)",
            params![key, c.exclude_hosted, c.keep_months, ts],
        )?;
        enqueue_seed(self, &tx, id)?;
        tx.commit()
    }

    /// Queues the seed record of every client with saved settings again, so a device that just updated to a
    /// build that reads client controls learns them. An older build applied those records but dropped the
    /// control fields, and nothing would send them again (Nov 19 2026: a PC updated from 0.9.72 never saw an
    /// exclusion set on the Mac before its update). Returns how many were queued.
    pub fn requeue_client_control_seeds(&self) -> rusqlite::Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let keys: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT c.topic_key FROM client_control c JOIN topic_seed s ON s.key = c.topic_key \
                 WHERE s.deleted = 0 ORDER BY c.topic_key",
            )?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for key in &keys {
            let id = seed_id(&tx, key)?;
            enqueue_seed(self, &tx, id)?;
        }
        tx.commit()?;
        Ok(keys.len())
    }

    /// Name and aliases of every undeleted seed (merged ones too) whose client is excluded from hosted AI. Only the
    /// seeds themselves; sources shared with an excluded client are handled at source level.
    pub fn excluded_terms(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.name, s.aliases FROM client_control c \
             JOIN topic_seed s ON s.key = c.topic_key JOIN topic t ON t.id = s.topic_id \
             WHERE c.exclude_hosted != 0 AND s.deleted = 0 \
             ORDER BY s.topic_id",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out: Vec<String> = Vec::new();
        for (name, aliases) in rows {
            // An alias list that does not parse would silently drop terms: unreadable.
            let aliases: Vec<String> = serde_json::from_str(&aliases)
                .map_err(crate::json_to_sql_error)?;
            for term in std::iter::once(name).chain(aliases) {
                if !term.trim().is_empty()
                    && !out.iter().any(|o| o.to_lowercase() == term.to_lowercase())
                {
                    out.push(term);
                }
            }
        }
        Ok(out)
    }

    /// The live clients (not deleted, not merged away) with a keep-for-N-months period set (more than 0;
    /// 0 is no period):
    /// `(key, months)`, by key. Errors when the settings cannot be read; a caller deleting by
    /// age must then delete nothing.
    pub fn clients_with_keep_period(&self) -> rusqlite::Result<Vec<(String, u32)>> {
        self.conn
            .prepare(
                "SELECT c.topic_key, c.keep_months FROM client_control c \
                 JOIN topic_seed s ON s.key = c.topic_key \
                 WHERE c.keep_months > 0 AND s.deleted = 0 AND s.merged_into_key IS NULL \
                 ORDER BY c.topic_key",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
    }

    /// The meetings and captures linked to a client with its own keep period
    /// ([`Self::clients_with_keep_period`]): global retention leaves them to that period
    /// (spec 5, "the client's period overrides the global capture, meeting and conversation
    /// retention for this client's material").
    pub fn client_kept_material(&self) -> rusqlite::Result<ClientKeptMaterial> {
        let mut out = ClientKeptMaterial::default();
        for (key, _) in self.clients_with_keep_period()? {
            for m in self.client_members(&key)? {
                let Some(r) = m.ref_id else { continue };
                match m.kind.as_str() {
                    "meeting" => out.meetings.push(r),
                    "snapshot" => {
                        if let Some((meeting, _)) = self.snapshot_meeting(r)? {
                            out.meetings.push(meeting);
                        }
                    }
                    "episode" => {
                        for (item, twin, _) in self.episode_captures(r)? {
                            out.capture_items.push(item);
                            out.capture_item_rows.extend(twin);
                        }
                    }
                    _ => {}
                }
            }
        }
        for v in [&mut out.meetings, &mut out.capture_items, &mut out.capture_item_rows] {
            v.sort_unstable();
            v.dedup();
        }
        Ok(out)
    }

    /// Marks the exclusion settings as not yet known (`true`) or known (`false`). While marked,
    /// [`Self::exclusion_unreadable`] is true. For a store that starts empty and is filled by a pull
    /// (the cloud runner): it holds no settings until the pull has read them, and "no settings"
    /// must not read as "nothing excluded" before then. Off by default.
    pub fn set_exclusion_unconfirmed(&self, on: bool) {
        self.exclusion_unconfirmed.set(on);
    }

    /// True when the exclusion settings cannot be read on this device (missing table, bad
    /// row), or are not yet known ([`Self::set_exclusion_unconfirmed`]). Callers then treat
    /// every client as excluded.
    pub fn exclusion_unreadable(&self) -> bool {
        self.exclusion_unconfirmed.get()
            || self.excluded_terms().is_err()
            || self
                .conn
                .prepare("SELECT topic_key, exclude_hosted, keep_months FROM client_control")
                .and_then(|mut s| {
                    s.query_map([], |r| {
                        r.get::<_, String>(0)?;
                        let e = r.get::<_, i64>(1)?;
                        r.get::<_, Option<u32>>(2)?;
                        Ok(e)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
                })
                .is_err()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use crate::tests::FakeEmbedder;
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("t.db"), Arc::new(FakeEmbedder));
        (dir, s)
    }

    #[test]
    fn set_and_read_client_control_roundtrip() {
        let (_d, s) = store();
        let seed = s.create_seed_topic("Acme", &[], None).unwrap();
        assert_eq!(s.client_control(&seed.key).unwrap(), ClientControl::default());
        let c = ClientControl {
            exclude_hosted: true,
            keep_months: Some(18),
        };
        s.set_client_control(&seed.key, &c).unwrap();
        assert_eq!(s.client_control(&seed.key).unwrap(), c);
        assert!(s.set_client_control("nope", &c).is_err());
    }

    #[test]
    fn excluded_terms_include_aliases_of_excluded_seeds_only() {
        let (_d, s) = store();
        let a = s
            .create_seed_topic("Acme", &["Acme Corp".to_string()], None)
            .unwrap();
        s.create_seed_topic("Globex", &["Globex Inc".to_string()], None)
            .unwrap();
        s.set_client_control(
            &a.key,
            &ClientControl {
                exclude_hosted: true,
                keep_months: None,
            },
        )
        .unwrap();
        let mut t = s.excluded_terms().unwrap();
        t.sort();
        assert_eq!(t, vec!["Acme".to_string(), "Acme Corp".to_string()]);
        assert!(!s.exclusion_unreadable());
    }

    #[test]
    fn unreadable_setting_fails_closed() {
        let (_d, s) = store();
        assert!(!s.exclusion_unreadable());
        s.conn.execute("DROP TABLE client_control", []).unwrap();
        assert!(s.exclusion_unreadable());
    }

    /// The cloud runner's scratch store: until the runner has read every exclusion row, a store with
    /// no client settings must not read as "nothing excluded".
    #[test]
    fn an_unconfirmed_store_reads_as_unreadable_until_confirmed() {
        let (_d, s) = store();
        assert!(!s.exclusion_unreadable(), "a store with no settings has nothing excluded");
        s.set_exclusion_unconfirmed(true);
        assert!(s.exclusion_unreadable());
        assert!(s.excluded_material().is_err(), "every exclusion path fails closed");
        s.set_exclusion_unconfirmed(false);
        assert!(!s.exclusion_unreadable());
        assert!(s.excluded_material().is_ok());
    }

    #[test]
    fn only_an_in_memory_store_says_it_is_in_memory() {
        let (_d, on_disk) = store();
        assert!(!on_disk.is_in_memory());
        assert!(Store::open_in_memory().unwrap().is_in_memory());
    }

    #[test]
    fn bad_row_fails_closed() {
        let (_d, s) = store();
        s.conn
            .execute(
                "INSERT INTO client_control (topic_key, exclude_hosted, keep_months, updated_at_unix_ms) \
                 VALUES ('k', 'garbage', NULL, 1)",
                [],
            )
            .unwrap();
        assert!(s.exclusion_unreadable());
    }

    #[test]
    fn older_payload_without_fields_deserializes() {
        let p: TopicSeedPayload = serde_json::from_str(
            r#"{"key":"k","name":"n","aliases":[],"kind":null,"merged_into_key":null,"deleted":false,"updated_at_unix_ms":5}"#,
        )
        .unwrap();
        assert_eq!(p.exclude_hosted, None);
        assert_eq!(p.keep_months, None);
    }

    fn payload(key: &str, excl: Option<bool>, cts: Option<i64>, ts: i64) -> TopicSeedPayload {
        TopicSeedPayload {
            key: key.into(),
            name: "Acme".into(),
            aliases: vec!["Acme".into()],
            kind: None,
            merged_into_key: None,
            deleted: false,
            updated_at_unix_ms: ts,
            exclude_hosted: excl,
            keep_months: None,
            control_updated_at_unix_ms: cts,
            purged: false,
        }
    }

    #[test]
    fn stale_seed_rename_does_not_turn_exclusion_off() {
        // Device B excluded at T1; device A (unaware) renamed later and sends exclude=false
        // with its old control timestamp. B must stay excluded.
        let (_d, b) = store();
        let seed = b.create_seed_topic("Acme", &[], None).unwrap();
        b.set_client_control(&seed.key, &ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        let t1 = b.conn.query_row("SELECT updated_at_unix_ms FROM client_control", [], |r| r.get::<_, i64>(0)).unwrap();
        let mut stale = payload(&seed.key, Some(false), Some(t1 - 1000), t1 + 5000);
        stale.name = "Acme Renamed".into();
        b.apply_synced_topic_seed(&stale).unwrap();
        assert!(b.client_control(&seed.key).unwrap().exclude_hosted);
        // The seed rename itself still applied.
        assert_eq!(b.list_seed_topics().unwrap()[0].name, "Acme Renamed");
        // And B's re-sent record carries the control timestamp, not the seed's.
        let (c, ts) = read_row_ts(&b.conn, &seed.key).unwrap().unwrap();
        assert!(c.exclude_hosted);
        assert_eq!(ts, t1);
    }

    /// The record `s` queued last for sync.
    fn last_seed_record(s: &Store) -> TopicSeedPayload {
        s.list_outbox_operations()
            .unwrap()
            .into_iter()
            .rev()
            .find(|o| o.1 == crate::OUTBOX_ROW_TYPE_TOPIC_SEED)
            .map(|o| serde_json::from_str(&o.3).unwrap())
            .unwrap()
    }

    fn aliases(s: &Store, key: &str) -> Vec<String> {
        s.list_seed_topics().unwrap().into_iter().find(|t| t.key == key).unwrap().aliases
    }

    /// X6 final review, Major 9: device A adds an alias; device B, not synced yet, turns exclusion on. A control
    /// write is not a seed edit: B's record must not undo A's alias anywhere, in either delivery order, and a third
    /// device ends with both.
    #[test]
    fn a_stale_control_write_never_undoes_an_alias_from_another_device() {
        let device = |id: &str| {
            let (d, s) = store();
            s.set_own_device_id(id);
            (d, s)
        };
        let ((_da, a), (_db, b)) = (device("dev-a"), device("dev-b"));
        let seed = a.create_seed_topic("Acme", &[], None).unwrap();
        let first = last_seed_record(&a);
        b.apply_synced_topic_seed(&first).unwrap();

        a.update_seed_topic(&seed.key, "Acme", &["Roadrunner Co".to_string()], None).unwrap();
        let alias = last_seed_record(&a);
        std::thread::sleep(std::time::Duration::from_millis(3));
        b.set_client_control(&seed.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        let control = last_seed_record(&b);

        a.apply_synced_topic_seed(&control).unwrap();
        b.apply_synced_topic_seed(&alias).unwrap();
        for (who, s) in [("A", &a), ("B", &b)] {
            assert!(aliases(s, &seed.key).contains(&"Roadrunner Co".to_string()), "{who} lost the alias");
            assert!(s.client_control(&seed.key).unwrap().exclude_hosted, "{who} lost the exclusion");
        }
        for order in [[&alias, &control], [&control, &alias]] {
            let (_dc, c) = device("dev-c");
            c.apply_synced_topic_seed(&first).unwrap();
            for r in order {
                c.apply_synced_topic_seed(r).unwrap();
            }
            assert!(aliases(&c, &seed.key).contains(&"Roadrunner Co".to_string()));
            assert!(c.client_control(&seed.key).unwrap().exclude_hosted);
            assert!(c.excluded_terms().unwrap().contains(&"Roadrunner Co".to_string()), "the alias is matched");
        }
    }

    #[test]
    fn older_device_payload_without_control_timestamp_leaves_control_alone() {
        let (_d, s) = store();
        let seed = s.create_seed_topic("Acme", &[], None).unwrap();
        s.set_client_control(&seed.key, &ClientControl { exclude_hosted: true, keep_months: Some(6) })
            .unwrap();
        s.apply_synced_topic_seed(&payload(&seed.key, Some(false), None, i64::MAX / 2)).unwrap();
        s.apply_synced_topic_seed(&payload(&seed.key, None, None, i64::MAX / 2 + 1)).unwrap();
        assert_eq!(
            s.client_control(&seed.key).unwrap(),
            ClientControl { exclude_hosted: true, keep_months: Some(6) }
        );
    }

    #[test]
    fn tie_prefers_excluded() {
        let (_d, s) = store();
        let seed = s.create_seed_topic("Acme", &[], None).unwrap();
        s.conn
            .execute(
                "INSERT INTO client_control (topic_key, exclude_hosted, keep_months, updated_at_unix_ms) VALUES (?1, 0, NULL, 500)",
                [&seed.key],
            )
            .unwrap();
        s.apply_synced_topic_seed(&payload(&seed.key, Some(true), Some(500), i64::MAX / 2)).unwrap();
        assert!(s.client_control(&seed.key).unwrap().exclude_hosted);
        // A tie in the other direction does not turn it off.
        s.apply_synced_topic_seed(&payload(&seed.key, Some(false), Some(500), i64::MAX / 2 + 1)).unwrap();
        assert!(s.client_control(&seed.key).unwrap().exclude_hosted);
    }

    #[test]
    fn merging_excluded_seed_keeps_exclusion_on_survivor_and_terms() {
        let (_d, s) = store();
        let x = s.create_seed_topic("Xco", &["Xco Ltd".to_string()], None).unwrap();
        let y = s.create_seed_topic("Yco", &[], None).unwrap();
        s.set_client_control(&x.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        s.merge_topics(y.topic_id, x.topic_id).unwrap();
        assert!(s.client_control(&y.key).unwrap().exclude_hosted);
        let mut t = s.excluded_terms().unwrap();
        t.sort();
        assert_eq!(t, vec!["Xco", "Xco Ltd", "Yco"]);
    }

    /// Spec 5: a client's own keep period overrides global retention for the client's material
    /// (meetings, and the captures under its screen-activity episodes); everything else still
    /// goes, and the preview counts what the sweep removes.
    #[test]
    fn global_retention_leaves_a_kept_clients_material_to_its_period() {
        let (_d, s) = store();
        let kept = s.create_seed_topic("Acme", &[], None).unwrap();
        let other = s.create_seed_topic("Globex", &[], None).unwrap();
        s.set_client_control(&kept.key, &ClientControl { exclude_hosted: false, keep_months: Some(24) }).unwrap();
        let meeting = |start: i64| {
            s.insert_meeting(&crate::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 1),
                title: "m".into(),
                attendees: vec![],
                trigger: crate::MeetingTrigger::Manual,
                audio_path: None,
                status: crate::MeetingStatus::Completed,
            })
            .unwrap()
        };
        let capture = |ts: i64| {
            s.insert_capture_item(&crate::NewCaptureItem {
                device_id: "d".into(),
                ts_unix_ms: ts,
                app_bundle_id: "app".into(),
                window_title: "w".into(),
                url: None,
                text: format!("t{ts}"),
                hash: ts,
                source: "ax".into(),
            })
            .unwrap()
            .unwrap()
        };
        let file = |topic: i64, kind: &str, r: i64| {
            s.conn
                .execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, 1, 1)",
                    params![kind, r],
                )
                .unwrap();
            let sid = s.conn.last_insert_rowid();
            s.conn
                .execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![topic, sid])
                .unwrap();
        };
        let m_kept = meeting(1_000);
        let m_other = meeting(1_100);
        let m_loose = meeting(1_200);
        file(kept.topic_id, "meeting", m_kept);
        file(other.topic_id, "meeting", m_other);
        let (c1, c2, c_loose) = (capture(100), capture(200), capture(300));
        let item = |c: i64| -> i64 {
            s.conn
                .query_row("SELECT id FROM items WHERE source_table = 'capture_item' AND source_id = ?1", [c], |r| r.get(0))
                .unwrap()
        };
        s.conn
            .execute(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, \
                 start_item_id, end_item_id, item_count, summary, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('d', 'app', 'w', 100, 200, ?1, ?2, 2, 's', 1, 1)",
                params![item(c1), item(c2)],
            )
            .unwrap();
        let episode = s.conn.last_insert_rowid();
        file(kept.topic_id, "episode", episode);

        let policy = crate::RetentionPolicy { capture_item_days: Some(1), meeting_days: Some(1), conversation_days: None };
        let now = 10 * 86_400_000;
        let preview = s.retention_preview(&policy, now).unwrap();
        let result = s.apply_retention(&policy, now).unwrap();
        assert_eq!((preview.meetings, preview.capture_items), (2, 1));
        assert_eq!((result.meetings_deleted, result.capture_items_deleted), (2, 1));
        let alive = |table: &str, id: i64| -> bool {
            s.conn
                .query_row(&format!("SELECT EXISTS (SELECT 1 FROM {table} WHERE id = ?1)"), [id], |r| r.get(0))
                .unwrap()
        };
        assert!(alive("meeting", m_kept));
        assert!(!alive("meeting", m_other) && !alive("meeting", m_loose));
        assert!(alive("capture_item", c1) && alive("capture_item", c2) && !alive("capture_item", c_loose));
        assert!(alive("items", item(c1)));

        // The period turned off: global retention applies to the client's material again.
        s.set_client_control(&kept.key, &ClientControl::default()).unwrap();
        let result = s.apply_retention(&policy, now).unwrap();
        assert_eq!((result.meetings_deleted, result.capture_items_deleted), (1, 2));
    }
    /// X6: the capture sweep spans many batches, and what a client keeps can change meanwhile on
    /// another connection. An episode filed there under a kept client after the first batch has
    /// run keeps its captures: the sweep sees the other connection's commit and reads the kept
    /// material again (#850 review, I4: it reads it outside the batches' write transactions).
    #[test]
    fn kept_material_that_changes_between_batches_is_respected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let s = crate::test_store::open_with_embedder(&path, Arc::new(FakeEmbedder));
        let other = crate::test_store::open_with_embedder(&path, Arc::new(FakeEmbedder));
        let kept = s.create_seed_topic("Acme", &[], None).unwrap();
        s.set_client_control(&kept.key, &ClientControl { exclude_hosted: false, keep_months: Some(24) }).unwrap();
        let n = 2_500i64;
        let ids = captures(&s, n);
        let item = |c: i64| -> i64 {
            s.conn
                .query_row("SELECT id FROM items WHERE source_table = 'capture_item' AND source_id = ?1", [c], |r| r.get(0))
                .unwrap()
        };
        // The last captures, which only a later batch reaches.
        let tail = &ids[ids.len() - 20..];
        let (first, last) = (item(tail[0]), item(*tail.last().unwrap()));
        let reads = std::cell::Cell::new(0);
        let gaps = std::cell::Cell::new(0);
        let deleted = s
            .delete_captures_batched_with(
                1_000_000,
                || {
                    reads.set(reads.get() + 1);
                    s.capture_retention_predicates()
                },
                || {
                    if gaps.replace(gaps.get() + 1) == 0 {
                        // Between batch 1 and 2, on the other connection: the judge files an
                        // episode under the kept client.
                        file_episode(&other, kept.topic_id, first, last);
                    }
                },
            )
            .unwrap();
        assert!(gaps.get() >= 2, "the sweep ran in batches ({} gaps)", gaps.get());
        assert!(reads.get() >= 2, "the other connection's commit made the sweep read again ({} reads)", reads.get());
        assert_eq!(deleted as i64, n - 20, "the kept client's captures survive");
        let left: i64 = s.conn.query_row("SELECT COUNT(*) FROM capture_item", [], |r| r.get(0)).unwrap();
        let left_items: i64 = s.conn.query_row("SELECT COUNT(*) FROM items WHERE kind = 'capture'", [], |r| r.get(0)).unwrap();
        assert_eq!((left, left_items), (20, 20), "the items mirror matches");
    }

    /// With nothing committed elsewhere, the sweep reads the kept material once, not per batch,
    /// and still keeps it.
    #[test]
    fn a_quiet_sweep_reads_the_kept_material_once() {
        let (_d, s) = store();
        let kept = s.create_seed_topic("Acme", &[], None).unwrap();
        s.set_client_control(&kept.key, &ClientControl { exclude_hosted: false, keep_months: Some(24) }).unwrap();
        let n = 2_500i64;
        let ids = captures(&s, n);
        let item = |c: i64| -> i64 {
            s.conn
                .query_row("SELECT id FROM items WHERE source_table = 'capture_item' AND source_id = ?1", [c], |r| r.get(0))
                .unwrap()
        };
        let tail = &ids[ids.len() - 20..];
        file_episode(&s, kept.topic_id, item(tail[0]), item(*tail.last().unwrap()));
        let reads = std::cell::Cell::new(0);
        let gaps = std::cell::Cell::new(0);
        let deleted = s
            .delete_captures_batched_with(
                1_000_000,
                || {
                    reads.set(reads.get() + 1);
                    s.capture_retention_predicates()
                },
                || gaps.set(gaps.get() + 1),
            )
            .unwrap();
        assert!(gaps.get() >= 2, "the sweep ran in batches");
        assert_eq!(reads.get(), 1);
        assert_eq!(deleted as i64, n - 20);
    }

    /// A commit by another connection between the outside read and the batch's write lock: the
    /// batch reads again under the lock, so material kept in that window is not deleted either.
    #[test]
    fn kept_material_committed_just_before_a_batch_takes_the_lock_is_respected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let s = crate::test_store::open_with_embedder(&path, Arc::new(FakeEmbedder));
        let other = crate::test_store::open_with_embedder(&path, Arc::new(FakeEmbedder));
        let kept = s.create_seed_topic("Acme", &[], None).unwrap();
        s.set_client_control(&kept.key, &ClientControl { exclude_hosted: false, keep_months: Some(24) }).unwrap();
        let n = 1_500i64;
        let ids = captures(&s, n);
        let item = |c: i64| -> i64 {
            s.conn
                .query_row("SELECT id FROM items WHERE source_table = 'capture_item' AND source_id = ?1", [c], |r| r.get(0))
                .unwrap()
        };
        let tail = &ids[ids.len() - 20..];
        let (first, last) = (item(tail[0]), item(*tail.last().unwrap()));
        let reads = std::cell::Cell::new(0);
        let deleted = s
            .delete_captures_batched_with(
                1_000_000,
                || {
                    let r = s.capture_retention_predicates();
                    if reads.replace(reads.get() + 1) == 1 {
                        // The second read (batch 2, outside the lock) is already stale when it
                        // returns: the other connection files the episode right after it.
                        file_episode(&other, kept.topic_id, first, last);
                    }
                    r
                },
                || {
                    // Batch 2 must read again: make the first read stale for it.
                    other.set_meta("test.nudge", "1").unwrap();
                },
            )
            .unwrap();
        assert!(reads.get() >= 3, "read outside, then again under the lock ({} reads)", reads.get());
        assert_eq!(deleted as i64, n - 20, "the kept client's captures survive");
    }

    /// A client given a keep period while the capture phase runs (on another connection) keeps
    /// its old meetings: the meeting phase reads the kept material as it stands then, not as it
    /// stood before the capture sweep (#850 review round 1, Important 2).
    #[test]
    fn a_keep_period_set_during_the_capture_phase_keeps_the_clients_meetings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let s = crate::test_store::open_with_embedder(&path, Arc::new(FakeEmbedder));
        let other = crate::test_store::open_with_embedder(&path, Arc::new(FakeEmbedder));
        let acme = s.create_seed_topic("Acme", &[], None).unwrap();
        let meeting = |start: i64| {
            s.insert_meeting(&crate::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 1),
                title: "m".into(),
                attendees: vec![],
                trigger: crate::MeetingTrigger::Manual,
                audio_path: None,
                status: crate::MeetingStatus::Completed,
            })
            .unwrap()
        };
        let (m_acme, m_loose) = (meeting(1_000), meeting(1_100));
        s.conn
            .execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES ('meeting', ?1, 1, 1)", [m_acme])
            .unwrap();
        let sid = s.conn.last_insert_rowid();
        s.conn
            .execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![acme.topic_id, sid])
            .unwrap();
        let policy = crate::RetentionPolicy { capture_item_days: Some(1), meeting_days: Some(1), conversation_days: None };
        let result = s
            .apply_retention_with(&policy, 10 * 86_400_000, || {
                other.set_client_control(&acme.key, &ClientControl { exclude_hosted: false, keep_months: Some(24) }).unwrap();
            })
            .unwrap();
        let alive = |id: i64| -> bool {
            s.conn.query_row("SELECT EXISTS (SELECT 1 FROM meeting WHERE id = ?1)", [id], |r| r.get(0)).unwrap()
        };
        assert!(alive(m_acme), "the client's meeting is kept");
        assert!(!alive(m_loose));
        assert_eq!(result.meetings_deleted, 1);
    }

    fn captures(s: &Store, n: i64) -> Vec<i64> {
        (0..n)
            .map(|i| {
                s.insert_capture_item(&crate::NewCaptureItem {
                    device_id: "d".into(),
                    ts_unix_ms: 100 + i,
                    app_bundle_id: "app".into(),
                    window_title: "w".into(),
                    url: None,
                    text: format!("t{i}"),
                    hash: i,
                    source: "ax".into(),
                })
                .unwrap()
                .unwrap()
            })
            .collect()
    }

    /// Files an episode over `items` ids `first..=last` under the client `topic_id`, on `s`'s connection.
    fn file_episode(s: &Store, topic_id: i64, first: i64, last: i64) {
        s.conn
            .execute(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, \
                 start_item_id, end_item_id, item_count, summary, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('d', 'app', 'w', 1, 2, ?1, ?2, 20, 's', 1, 1)",
                params![first, last],
            )
            .unwrap();
        let episode = s.conn.last_insert_rowid();
        s.conn
            .execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES ('episode', ?1, 1, 1)", [episode])
            .unwrap();
        let sid = s.conn.last_insert_rowid();
        s.conn
            .execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![topic_id, sid])
            .unwrap();
    }
}
