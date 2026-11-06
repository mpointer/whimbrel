//! "Forward me a live question" alerts. A row created on this device (`origin_device_id`
//! NULL) syncs as an upsert keyed by its id; a draft update re-sends it. Email status and the
//! notified mark stay per device. A peer's alert can arrive before its meeting (meetings sync
//! only once complete), so it keeps `meeting_title` and gets `meeting_id` back-filled by
//! `record_sync_meeting_map`. See docs/superpowers/specs for the live question design.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::Store;

const MAX_TEXT_CHARS: usize = 2_000;
const MAX_DRAFT_BYTES: usize = 64 * 1024;
const COLUMNS: &str =
    "id, meeting_id, origin_device_id, source_id, source_meeting_id, meeting_title, quote, \
    restatement, asker, at_cs, created_at_unix_ms, draft_json, email_status, notified_at_unix_ms";

#[derive(Debug, Clone, Deserialize)]
pub struct NewLiveAlert {
    pub meeting_id: i64,
    pub meeting_title: String,
    pub quote: String,
    pub restatement: String,
    pub asker: Option<String>,
    /// Centiseconds from the meeting's start (not the current part's), like the final
    /// transcript's `start_cs`, so a continued meeting's alerts and links line up.
    pub at_cs: i64,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LiveAlert {
    pub id: i64,
    pub meeting_id: Option<i64>,
    pub origin_device_id: Option<String>,
    pub source_id: i64,
    pub source_meeting_id: i64,
    pub meeting_title: String,
    pub quote: String,
    pub restatement: String,
    pub asker: Option<String>,
    pub at_cs: i64,
    pub created_at_unix_ms: i64,
    pub draft_json: Option<String>,
    pub email_status: Option<String>,
    pub notified_at_unix_ms: Option<i64>,
}

/// An alert on the wire. `source_id` and `source_meeting_id` are the origin device's local ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedLiveAlert {
    pub source_id: i64,
    pub source_meeting_id: i64,
    pub meeting_title: String,
    pub quote: String,
    pub restatement: String,
    pub asker: Option<String>,
    pub at_cs: i64,
    pub created_at_unix_ms: i64,
    pub draft_json: Option<String>,
}

pub(crate) fn ensure_live_alert_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS live_alert (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            meeting_id INTEGER,
            origin_device_id TEXT,
            source_id INTEGER NOT NULL,
            source_meeting_id INTEGER NOT NULL,
            meeting_title TEXT NOT NULL,
            quote TEXT NOT NULL,
            restatement TEXT NOT NULL,
            asker TEXT,
            at_cs INTEGER NOT NULL,
            created_at_unix_ms INTEGER NOT NULL,
            draft_json TEXT,
            email_status TEXT,
            notified_at_unix_ms INTEGER,
            UNIQUE(origin_device_id, source_id)
        );
        CREATE INDEX IF NOT EXISTS idx_live_alert_meeting ON live_alert(meeting_id);",
    )
}

fn row_to_alert(r: &Row<'_>) -> rusqlite::Result<LiveAlert> {
    Ok(LiveAlert {
        id: r.get(0)?,
        meeting_id: r.get(1)?,
        origin_device_id: r.get(2)?,
        source_id: r.get(3)?,
        source_meeting_id: r.get(4)?,
        meeting_title: r.get(5)?,
        quote: r.get(6)?,
        restatement: r.get(7)?,
        asker: r.get(8)?,
        at_cs: r.get(9)?,
        created_at_unix_ms: r.get(10)?,
        draft_json: r.get(11)?,
        email_status: r.get(12)?,
        notified_at_unix_ms: r.get(13)?,
    })
}

fn bounded(s: &str) -> String {
    s.trim().chars().take(MAX_TEXT_CHARS).collect()
}

fn payload(a: &LiveAlert) -> AppliedLiveAlert {
    AppliedLiveAlert {
        source_id: a.source_id,
        source_meeting_id: a.source_meeting_id,
        meeting_title: a.meeting_title.clone(),
        quote: a.quote.clone(),
        restatement: a.restatement.clone(),
        asker: a.asker.clone(),
        at_cs: a.at_cs,
        created_at_unix_ms: a.created_at_unix_ms,
        draft_json: a.draft_json.clone(),
    }
}

impl Store {
    /// Inserts this device's alert and enqueues its sync upsert in the same transaction.
    pub fn insert_live_alert(&self, a: &NewLiveAlert) -> rusqlite::Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO live_alert (meeting_id, origin_device_id, source_id, source_meeting_id, meeting_title, quote, \
             restatement, asker, at_cs, created_at_unix_ms) VALUES (?1, NULL, 0, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![a.meeting_id, a.meeting_title, a.quote, a.restatement, a.asker, a.at_cs, a.created_at_unix_ms],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE live_alert SET source_id = id WHERE id = ?1",
            params![id],
        )?;
        let row = Self::live_alert_tx(&tx, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        self.enqueue_live_alert_outbox(&tx, id, &payload(&row))?;
        tx.commit()?;
        Ok(id)
    }

    /// Stores a draft. On this device's own alert it re-enqueues the upsert so peers get it.
    pub fn set_live_alert_draft(&self, id: i64, draft_json: &str) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        if tx.execute(
            "UPDATE live_alert SET draft_json = ?1 WHERE id = ?2",
            params![draft_json, id],
        )? == 0
        {
            return Ok(false);
        }
        let row = Self::live_alert_tx(&tx, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        if row.origin_device_id.is_none() {
            self.enqueue_live_alert_outbox(&tx, id, &payload(&row))?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// `sent`, `failed:<reason>` or `off`. Local only, never synced.
    pub fn set_live_alert_email_status(&self, id: i64, status: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.execute(
            "UPDATE live_alert SET email_status = ?1 WHERE id = ?2",
            params![status, id],
        )? > 0)
    }

    pub fn get_live_alert(&self, id: i64) -> rusqlite::Result<Option<LiveAlert>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM live_alert WHERE id = ?1"),
                params![id],
                row_to_alert,
            )
            .optional()
    }

    fn live_alert_tx(
        tx: &rusqlite::Transaction<'_>,
        id: i64,
    ) -> rusqlite::Result<Option<LiveAlert>> {
        tx.query_row(
            &format!("SELECT {COLUMNS} FROM live_alert WHERE id = ?1"),
            params![id],
            row_to_alert,
        )
        .optional()
    }

    /// A meeting's alerts, in meeting order.
    pub fn list_live_alerts(&self, meeting_id: i64) -> rusqlite::Result<Vec<LiveAlert>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM live_alert WHERE meeting_id = ?1 ORDER BY at_cs, id"
        ))?;
        let rows = stmt.query_map(params![meeting_id], row_to_alert)?;
        rows.collect()
    }

    /// Peer alerts not yet notified on this device and no older than `max_age_ms`. Every
    /// unnotified peer alert is marked in the same statement, so older ones are skipped for good.
    pub fn take_unnotified_peer_alerts(
        &self,
        now_ms: i64,
        max_age_ms: i64,
    ) -> rusqlite::Result<Vec<LiveAlert>> {
        let mut stmt = self.conn.prepare(&format!(
            "UPDATE live_alert SET notified_at_unix_ms = ?1 \
             WHERE origin_device_id IS NOT NULL AND notified_at_unix_ms IS NULL RETURNING {COLUMNS}"
        ))?;
        let rows = stmt
            .query_map(params![now_ms], row_to_alert)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut fresh: Vec<LiveAlert> = rows
            .into_iter()
            .filter(|a| a.created_at_unix_ms >= now_ms - max_age_ms)
            .collect();
        fresh.sort_by_key(|a| (a.created_at_unix_ms, a.id));
        Ok(fresh)
    }

    /// Applies an alert pulled from another device, upserting on `(origin, source_id)` so a
    /// later draft replaces it. Never enqueued. Email status and the notified mark are kept.
    /// `None` when the payload carries no quote.
    pub fn apply_synced_live_alert(
        &self,
        origin_device_id: &str,
        a: &AppliedLiveAlert,
    ) -> rusqlite::Result<Option<i64>> {
        let quote = bounded(&a.quote);
        if quote.is_empty() {
            return Ok(None);
        }
        let created = a
            .created_at_unix_ms
            .min(crate::now_unix_ms() + crate::SYNCED_RUN_MAX_FUTURE_MS);
        let draft = a
            .draft_json
            .as_deref()
            .filter(|d| d.len() <= MAX_DRAFT_BYTES);
        let meeting_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT m.local_meeting_id FROM sync_meeting_map m JOIN meeting ON meeting.id = m.local_meeting_id \
                 WHERE m.peer_device_id = ?1 AND m.source_meeting_id = ?2",
                params![origin_device_id, a.source_meeting_id],
                |r| r.get(0),
            )
            .optional()?;
        self.conn
            .query_row(
                "INSERT INTO live_alert (meeting_id, origin_device_id, source_id, source_meeting_id, meeting_title, quote, \
                 restatement, asker, at_cs, created_at_unix_ms, draft_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                 ON CONFLICT(origin_device_id, source_id) DO UPDATE SET \
                 meeting_id = COALESCE(excluded.meeting_id, live_alert.meeting_id), source_meeting_id = excluded.source_meeting_id, \
                 meeting_title = excluded.meeting_title, quote = excluded.quote, restatement = excluded.restatement, \
                 asker = excluded.asker, at_cs = excluded.at_cs, created_at_unix_ms = excluded.created_at_unix_ms, \
                 draft_json = excluded.draft_json \
                 RETURNING id",
                params![
                    meeting_id,
                    origin_device_id,
                    a.source_id,
                    a.source_meeting_id,
                    bounded(&a.meeting_title),
                    quote,
                    bounded(&a.restatement),
                    a.asker.as_deref().map(bounded),
                    a.at_cs,
                    created,
                    draft,
                ],
                |r| r.get(0),
            )
            .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppliedLiveAlert, NewLiveAlert};
    use crate::{AppliedMeeting, MeetingStatus, MeetingTrigger, NewMeeting, Store};

    const MIN: i64 = 60_000;

    fn meeting(store: &Store) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "dev-a".into(),
                start_unix_ms: 1_000,
                end_unix_ms: None,
                title: "Pricing review".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap()
    }

    fn new_alert(meeting_id: i64, at: i64) -> NewLiveAlert {
        NewLiveAlert {
            meeting_id,
            meeting_title: "Pricing review".into(),
            quote: "Mike, can you send the numbers?".into(),
            restatement: "Send the pricing numbers".into(),
            asker: Some("Dana".into()),
            at_cs: 4_200,
            created_at_unix_ms: at,
        }
    }

    fn peer_alert(source_id: i64, source_meeting_id: i64, at: i64) -> AppliedLiveAlert {
        AppliedLiveAlert {
            source_id,
            source_meeting_id,
            meeting_title: "Pricing review".into(),
            quote: "Mike, can you send the numbers?".into(),
            restatement: "Send the pricing numbers".into(),
            asker: Some("Dana".into()),
            at_cs: 4_200,
            created_at_unix_ms: at,
            draft_json: None,
        }
    }

    fn outbox_count(store: &Store) -> usize {
        store
            .list_outbox_operations()
            .unwrap()
            .iter()
            .filter(|o| o.1 == "live_alert")
            .count()
    }

    #[test]
    fn insert_enqueues_outbox_and_lists() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let m = meeting(&store);
        let id = store.insert_live_alert(&new_alert(m, 5_000)).unwrap();
        assert_eq!(outbox_count(&store), 1);
        let ops = store.list_outbox_operations().unwrap();
        let payload: AppliedLiveAlert =
            serde_json::from_str(&ops.iter().find(|o| o.1 == "live_alert").unwrap().3).unwrap();
        assert_eq!((payload.source_id, payload.source_meeting_id), (id, m));
        let listed = store.list_live_alerts(m).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].meeting_id, Some(m));
        assert_eq!(listed[0].origin_device_id, None);
        assert_eq!(listed[0].quote, "Mike, can you send the numbers?");
        assert_eq!(listed[0].asker.as_deref(), Some("Dana"));
    }

    #[test]
    fn draft_update_reenqueues() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let m = meeting(&store);
        let id = store.insert_live_alert(&new_alert(m, 5_000)).unwrap();
        assert!(store
            .set_live_alert_draft(id, r#"{"body":"Here are the numbers"}"#)
            .unwrap());
        assert_eq!(outbox_count(&store), 2);
        let ops = store.list_outbox_operations().unwrap();
        let last: AppliedLiveAlert =
            serde_json::from_str(&ops.iter().rev().find(|o| o.1 == "live_alert").unwrap().3)
                .unwrap();
        assert_eq!(
            last.draft_json.as_deref(),
            Some(r#"{"body":"Here are the numbers"}"#)
        );
        // Email status is local only.
        assert!(store.set_live_alert_email_status(id, "sent").unwrap());
        assert_eq!(outbox_count(&store), 2);
        assert_eq!(
            store.list_live_alerts(m).unwrap()[0]
                .email_status
                .as_deref(),
            Some("sent")
        );
    }

    #[test]
    fn peer_alert_without_meeting_keeps_title_and_null_meeting() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-b");
        let id = store
            .apply_synced_live_alert("dev-a", &peer_alert(3, 77, 5_000))
            .unwrap()
            .unwrap();
        let row = store.get_live_alert(id).unwrap().unwrap();
        assert_eq!(row.meeting_id, None);
        assert_eq!(row.meeting_title, "Pricing review");
        assert_eq!(row.origin_device_id.as_deref(), Some("dev-a"));
        assert_eq!((row.source_id, row.source_meeting_id), (3, 77));
        assert_eq!(outbox_count(&store), 0, "a peer row is never enqueued");

        // A later draft from the origin replaces the row in place.
        let mut updated = peer_alert(3, 77, 5_000);
        updated.draft_json = Some("{}".into());
        assert_eq!(
            store.apply_synced_live_alert("dev-a", &updated).unwrap(),
            Some(id)
        );
        assert_eq!(
            store
                .get_live_alert(id)
                .unwrap()
                .unwrap()
                .draft_json
                .as_deref(),
            Some("{}")
        );
    }

    #[test]
    fn meeting_arrival_backfills_alert_meeting_id() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .apply_synced_live_alert("dev-a", &peer_alert(3, 77, 5_000))
            .unwrap()
            .unwrap();
        let local = store
            .apply_synced_meeting(
                "dev-a",
                9,
                None,
                &AppliedMeeting {
                    attendee_details: vec![],
                    start_unix_ms: 1_000,
                    end_unix_ms: Some(2_000),
                    title: "Pricing review".into(),
                    attendees: vec![],
                    trigger: MeetingTrigger::Manual,
                    status: MeetingStatus::Completed,
                    transcript_path: None,
                    notes: None,
                    summary_tldr: None,
                    summary_decisions: vec![],
                    summary_action_items: vec![],
                    summary_open_questions: vec![],
                    summary_provider: None,
                    summary_model: None,
                    keep_media: None,
                    trimmed_keep_ms: None,
                },
            )
            .unwrap();
        store.record_sync_meeting_map("dev-a", 77, local).unwrap();
        assert_eq!(
            store.get_live_alert(id).unwrap().unwrap().meeting_id,
            Some(local)
        );
        assert_eq!(store.list_live_alerts(local).unwrap().len(), 1);

        // An alert arriving after its meeting resolves straight away.
        let later = store
            .apply_synced_live_alert("dev-a", &peer_alert(4, 77, 6_000))
            .unwrap()
            .unwrap();
        assert_eq!(
            store.get_live_alert(later).unwrap().unwrap().meeting_id,
            Some(local)
        );
    }

    #[test]
    fn take_unnotified_returns_fresh_once_and_skips_old() {
        let store = Store::open_in_memory().unwrap();
        let now = 100 * MIN;
        let fresh = store
            .apply_synced_live_alert("dev-a", &peer_alert(1, 77, now - 10 * MIN))
            .unwrap()
            .unwrap();
        let old = store
            .apply_synced_live_alert("dev-a", &peer_alert(2, 77, now - 40 * MIN))
            .unwrap()
            .unwrap();
        let taken = store.take_unnotified_peer_alerts(now, 1_800_000).unwrap();
        assert_eq!(taken.iter().map(|a| a.id).collect::<Vec<_>>(), vec![fresh]);
        assert!(store
            .take_unnotified_peer_alerts(now, 1_800_000)
            .unwrap()
            .is_empty());
        assert!(
            store
                .get_live_alert(old)
                .unwrap()
                .unwrap()
                .notified_at_unix_ms
                .is_some(),
            "old rows are marked"
        );

        // A later draft update does not re-notify.
        let mut updated = peer_alert(1, 77, now - 10 * MIN);
        updated.draft_json = Some("{}".into());
        store.apply_synced_live_alert("dev-a", &updated).unwrap();
        assert!(store
            .take_unnotified_peer_alerts(now, 1_800_000)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn delete_meeting_deletes_its_alerts() {
        let store = Store::open_in_memory().unwrap();
        let m = meeting(&store);
        let other = meeting(&store);
        store.insert_live_alert(&new_alert(m, 5_000)).unwrap();
        store.insert_live_alert(&new_alert(other, 5_000)).unwrap();
        store.delete_meeting(m).unwrap();
        assert!(store.list_live_alerts(m).unwrap().is_empty());
        assert_eq!(store.list_live_alerts(other).unwrap().len(), 1);
    }
}
