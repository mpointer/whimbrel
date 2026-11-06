//! Profile suggestions and the daily-job markers. Never mirrored into `items`. A suggestion syncs when
//! created, and again when dismissed or added (a `profile_suggestion_status` row), so every device
//! resolves it. See
//! docs/superpowers/specs/2026-10-06-profile-design.md and
//! docs/superpowers/specs/2026-10-13-primary-prep-suggestions-recap-design.md.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{Meeting, Store};

const LAST_DAY_KEY: &str = "profile_suggest_last_day";
const LAST_ATTEMPT_KEY: &str = "profile_suggest_last_attempt_ms";

/// A profile suggestion on the wire. `source_id` is the origin device's own `profile_suggestion.id`.
/// `source_meeting_id` is deliberately absent: it is a local meeting id. Status is per device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedProfileSuggestion {
    pub source_id: i64,
    pub text: String,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProfileSuggestion {
    pub id: i64,
    pub text: String,
    pub source_meeting_id: Option<i64>,
    pub status: String,
    pub created_at_unix_ms: i64,
}

/// A suggestion resolved on another device, on the wire: named by its creator's identity, with its text
/// for devices that hold the same suggestion under another identity. `status` is "dismissed" or "added".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedProfileSuggestionStatus {
    pub origin_device_id: String,
    pub source_id: i64,
    pub status: String,
    pub text: String,
}

const DISMISSED: &str = "dismissed";
const ADDED: &str = "added";

fn text_key(text: &str) -> String {
    text.trim().to_lowercase()
}

pub(crate) fn ensure_profile_suggestion_table(conn: &Connection) -> rusqlite::Result<()> {
    // Statuses from other devices, kept so a suggestion that arrives after its status lands resolved.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS profile_suggestion_resolution (
            origin_device_id TEXT NOT NULL,
            source_id INTEGER NOT NULL,
            status TEXT NOT NULL,
            text_key TEXT NOT NULL,
            PRIMARY KEY (origin_device_id, source_id)
        );
        CREATE INDEX IF NOT EXISTS idx_profile_suggestion_resolution_text ON profile_suggestion_resolution(text_key);",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS profile_suggestion (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            text TEXT NOT NULL,
            source_meeting_id INTEGER,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at_unix_ms INTEGER NOT NULL
        );",
    )?;
    let existing: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(profile_suggestion)")?;
        let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
        names.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (name, ty) in [("origin_device_id", "TEXT"), ("source_id", "INTEGER")] {
        if !existing.iter().any(|c| c == name) {
            conn.execute(
                &format!("ALTER TABLE profile_suggestion ADD COLUMN {name} {ty}"),
                [],
            )?;
        }
    }
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_profile_suggestion_origin ON profile_suggestion(origin_device_id, source_id) WHERE origin_device_id IS NOT NULL",
        [],
    )?;
    Ok(())
}

impl Store {
    pub fn add_profile_suggestion(
        &self,
        text: &str,
        source_meeting_id: Option<i64>,
        now_unix_ms: i64,
    ) -> rusqlite::Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO profile_suggestion (text, source_meeting_id, status, created_at_unix_ms) VALUES (?1, ?2, 'pending', ?3)",
            params![text, source_meeting_id, now_unix_ms],
        )?;
        let id = tx.last_insert_rowid();
        self.enqueue_profile_suggestion_outbox(
            &tx,
            id,
            &AppliedProfileSuggestion {
                source_id: id,
                text: text.to_string(),
                created_at_unix_ms: now_unix_ms,
            },
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Applies a suggestion pulled from another device: pending, once per `(origin, source_id)`,
    /// bounded, skipped when the same text is already a suggestion here, and never enqueued.
    pub fn apply_synced_profile_suggestion(
        &self,
        origin_device_id: &str,
        s: &AppliedProfileSuggestion,
    ) -> rusqlite::Result<Option<i64>> {
        let text: String = s.text.trim().chars().take(500).collect();
        if text.is_empty() {
            return Ok(None);
        }
        let created = s
            .created_at_unix_ms
            .min(crate::now_unix_ms() + crate::SYNCED_RUN_MAX_FUTURE_MS);
        let key = text.to_lowercase();
        // Resolved elsewhere before it got here: dismissed lands dismissed, added never lands.
        let resolved: Option<String> = self
            .conn
            .query_row(
                "SELECT status FROM profile_suggestion_resolution WHERE (origin_device_id = ?1 AND source_id = ?2) OR text_key = ?3 \
                 ORDER BY status = 'added' DESC LIMIT 1",
                params![origin_device_id, s.source_id, key],
                |r| r.get(0),
            )
            .optional()?;
        if resolved.as_deref() == Some(ADDED) {
            return Ok(None);
        }
        let status = if resolved.is_some() { DISMISSED } else { "pending" };
        for status in ["pending", "dismissed"] {
            if self
                .list_profile_suggestions(status)?
                .iter()
                .any(|x| x.text.trim().to_lowercase() == key)
            {
                return Ok(None);
            }
        }
        let changed = self.conn.execute(
            "INSERT INTO profile_suggestion (text, source_meeting_id, status, created_at_unix_ms, origin_device_id, source_id) \
             VALUES (?1, NULL, ?5, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
            params![text, created, origin_device_id, s.source_id, status],
        )?;
        Ok((changed > 0).then(|| self.conn.last_insert_rowid()))
    }

    /// Whether another device created a suggestion at or after `since_unix_ms` (the start of the
    /// local day): one suggestion run per day across devices.
    pub fn has_synced_profile_suggestion_since(
        &self,
        since_unix_ms: i64,
    ) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM profile_suggestion WHERE origin_device_id IS NOT NULL AND created_at_unix_ms >= ?1)",
            params![since_unix_ms],
            |r| r.get(0),
        )
    }

    pub fn list_profile_suggestions(
        &self,
        status: &str,
    ) -> rusqlite::Result<Vec<ProfileSuggestion>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, source_meeting_id, status, created_at_unix_ms FROM profile_suggestion WHERE status = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![status], |r| {
            Ok(ProfileSuggestion {
                id: r.get(0)?,
                text: r.get(1)?,
                source_meeting_id: r.get(2)?,
                status: r.get(3)?,
                created_at_unix_ms: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Sets a suggestion's status. Dismissing it also tells the other devices, which dismiss their copy.
    pub fn set_profile_suggestion_status(&self, id: i64, status: &str) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let before: Option<String> = tx
            .query_row("SELECT status FROM profile_suggestion WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        let Some(before) = before else { return Ok(false) };
        let identity = self.suggestion_identity(id)?;
        tx.execute("UPDATE profile_suggestion SET status = ?1 WHERE id = ?2", params![status, id])?;
        if before != status && status == DISMISSED {
            self.enqueue_suggestion_status_tx(&tx, identity, DISMISSED)?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Deletes a suggestion: its fact was added to the profile (which syncs on its own). The other
    /// devices drop their pending copy too.
    pub fn delete_profile_suggestion(&self, id: i64) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let identity = self.suggestion_identity(id)?;
        let deleted = tx.execute("DELETE FROM profile_suggestion WHERE id = ?1", params![id])? > 0;
        if deleted {
            self.enqueue_suggestion_status_tx(&tx, identity, ADDED)?;
        }
        tx.commit()?;
        Ok(deleted)
    }

    /// The suggestion's creator identity and text: its origin for a pulled row, this device for its own
    /// (`None` without a device id, when nothing syncs anyway, or when the row is gone).
    fn suggestion_identity(&self, id: i64) -> rusqlite::Result<Option<(String, i64, String)>> {
        let own = self.own_device_id();
        let row: Option<(Option<String>, Option<i64>, String)> = self
            .conn
            .query_row(
                "SELECT origin_device_id, source_id, text FROM profile_suggestion WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(origin, source, text)| match (origin, source) {
            (Some(o), Some(sid)) => Some((o, sid, text)),
            _ => own.map(|o| (o, id, text)),
        }))
    }

    fn enqueue_suggestion_status_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        identity: Option<(String, i64, String)>,
        status: &str,
    ) -> rusqlite::Result<()> {
        let Some((origin_device_id, source_id, text)) = identity else {
            return Ok(());
        };
        let payload = AppliedProfileSuggestionStatus { origin_device_id, source_id, status: status.to_string(), text };
        let aggregate = format!("{}:{}", payload.origin_device_id, payload.source_id);
        self.enqueue_upsert_tx(tx, crate::OUTBOX_ROW_TYPE_PROFILE_SUGGESTION_STATUS, aggregate, &payload)
    }

    /// Applies a suggestion's status from another device: dismisses (or, for "added", drops) the pending
    /// copy here, matched by creator identity or by text, and remembers it for a copy that arrives later.
    /// Never enqueued; an unknown status is ignored.
    pub fn apply_synced_profile_suggestion_status(&self, s: &AppliedProfileSuggestionStatus) -> rusqlite::Result<()> {
        if s.status != DISMISSED && s.status != ADDED {
            return Ok(());
        }
        let key = text_key(&s.text);
        let own = self.own_device_id();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO profile_suggestion_resolution (origin_device_id, source_id, status, text_key) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (origin_device_id, source_id) DO UPDATE SET status = excluded.status, text_key = excluded.text_key",
            params![s.origin_device_id, s.source_id, s.status, key],
        )?;
        let matches = "status = 'pending' AND ((origin_device_id = ?1 AND source_id = ?2) \
             OR (origin_device_id IS NULL AND id = ?2 AND ?3 = ?1) OR lower(trim(text)) = ?4)";
        let own = own.unwrap_or_default();
        if s.status == ADDED {
            tx.execute(&format!("DELETE FROM profile_suggestion WHERE {matches}"), params![s.origin_device_id, s.source_id, own, key])?;
        } else {
            tx.execute(
                &format!("UPDATE profile_suggestion SET status = 'dismissed' WHERE {matches}"),
                params![s.origin_device_id, s.source_id, own, key],
            )?;
        }
        tx.commit()
    }

    /// Summarized meetings that started in `[start, end)`, oldest first.
    pub fn meetings_between(
        &self,
        start_unix_ms: i64,
        end_unix_ms: i64,
    ) -> rusqlite::Result<Vec<Meeting>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, audio_path, status, transcript_path, notes, summary_tldr, summary_decisions_json, summary_action_items_json, summary_open_questions_json, summary_provider, summary_model, local_seq, estimated_speaker_count, away_outcome, away_outcome_at_unix_ms, keep_media FROM meeting WHERE start_unix_ms >= ?1 AND start_unix_ms < ?2 AND summary_tldr IS NOT NULL ORDER BY start_unix_ms",
        )?;
        let rows = stmt.query_map(params![start_unix_ms, end_unix_ms], Self::row_to_meeting)?;
        rows.collect()
    }

    fn kv_get(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
    }

    fn kv_set(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn profile_suggest_last_day(&self) -> rusqlite::Result<Option<String>> {
        self.kv_get(LAST_DAY_KEY)
    }

    pub fn set_profile_suggest_last_day(&self, day: &str) -> rusqlite::Result<()> {
        self.kv_set(LAST_DAY_KEY, day)
    }

    pub fn profile_suggest_last_attempt(&self) -> rusqlite::Result<Option<i64>> {
        Ok(self.kv_get(LAST_ATTEMPT_KEY)?.and_then(|v| v.parse().ok()))
    }

    pub fn set_profile_suggest_last_attempt(&self, now_unix_ms: i64) -> rusqlite::Result<()> {
        self.kv_set(LAST_ATTEMPT_KEY, &now_unix_ms.to_string())
    }
}

#[cfg(test)]
mod tests {

    use super::{AppliedProfileSuggestion, AppliedProfileSuggestionStatus};

    fn device(device: &str) -> Store {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id(device);
        s
    }

    fn status_ops(s: &Store) -> Vec<AppliedProfileSuggestionStatus> {
        s.list_outbox_operations()
            .unwrap()
            .into_iter()
            .filter(|o| o.1 == crate::OUTBOX_ROW_TYPE_PROFILE_SUGGESTION_STATUS)
            .map(|o| serde_json::from_str(&o.3).unwrap())
            .collect()
    }

    #[test]
    fn a_dismissal_reaches_the_other_devices_copy() {
        let mac = device("mac");
        let id = mac.add_profile_suggestion("Priya is the CFO", None, 1).unwrap();
        let pc = device("pc");
        pc.apply_synced_profile_suggestion("mac", &AppliedProfileSuggestion { source_id: id, text: "Priya is the CFO".into(), created_at_unix_ms: 1 }).unwrap();
        assert!(mac.set_profile_suggestion_status(id, "dismissed").unwrap());
        let ops = status_ops(&mac);
        assert_eq!(ops, vec![AppliedProfileSuggestionStatus { origin_device_id: "mac".into(), source_id: id, status: "dismissed".into(), text: "Priya is the CFO".into() }]);
        pc.apply_synced_profile_suggestion_status(&ops[0]).unwrap();
        assert!(pc.list_profile_suggestions("pending").unwrap().is_empty());
        assert_eq!(pc.list_profile_suggestions("dismissed").unwrap().len(), 1);
        // Dismissing a pulled copy names its creator, so the creator dismisses its own row too.
        let phone_side = device("phone");
        phone_side.apply_synced_profile_suggestion("mac", &AppliedProfileSuggestion { source_id: 9, text: "Sam runs QA".into(), created_at_unix_ms: 1 }).unwrap();
        let copy = phone_side.list_profile_suggestions("pending").unwrap()[0].id;
        phone_side.set_profile_suggestion_status(copy, "dismissed").unwrap();
        assert_eq!((status_ops(&phone_side)[0].origin_device_id.as_str(), status_ops(&phone_side)[0].source_id), ("mac", 9));
    }

    #[test]
    fn a_status_that_arrives_first_resolves_the_suggestion_when_it_lands() {
        let pc = device("pc");
        let dismissed = AppliedProfileSuggestionStatus { origin_device_id: "mac".into(), source_id: 1, status: "dismissed".into(), text: "A".into() };
        let added = AppliedProfileSuggestionStatus { origin_device_id: "mac".into(), source_id: 2, status: "added".into(), text: "B".into() };
        pc.apply_synced_profile_suggestion_status(&dismissed).unwrap();
        pc.apply_synced_profile_suggestion_status(&added).unwrap();
        pc.apply_synced_profile_suggestion("mac", &AppliedProfileSuggestion { source_id: 1, text: "A".into(), created_at_unix_ms: 1 }).unwrap();
        pc.apply_synced_profile_suggestion("mac", &AppliedProfileSuggestion { source_id: 2, text: "B".into(), created_at_unix_ms: 1 }).unwrap();
        assert!(pc.list_profile_suggestions("pending").unwrap().is_empty(), "neither shows as pending");
        assert_eq!(pc.list_profile_suggestions("dismissed").unwrap().len(), 1);
    }

    #[test]
    fn adding_a_suggestion_drops_the_pending_copy_everywhere_and_matches_by_text() {
        let mac = device("mac");
        let id = mac.add_profile_suggestion("Sam runs QA", None, 1).unwrap();
        // The PC made the same suggestion itself: matched by text.
        let pc = device("pc");
        pc.add_profile_suggestion("sam runs qa ", None, 2).unwrap();
        assert!(mac.delete_profile_suggestion(id).unwrap());
        let ops = status_ops(&mac);
        assert_eq!(ops[0].status, "added");
        pc.apply_synced_profile_suggestion_status(&ops[0]).unwrap();
        assert!(pc.list_profile_suggestions("pending").unwrap().is_empty());
        assert!(status_ops(&pc).is_empty(), "applying never enqueues");
    }
    use crate::{MeetingStatus, MeetingTrigger, NewMeeting, Store};

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn meeting(store: &Store, title: &str, start: i64) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 1),
                title: title.into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap()
    }

    #[test]
    fn suggestions_round_trip_and_status_filters() {
        let s = store();
        let a = s
            .add_profile_suggestion("Priya is the CFO", Some(7), 1_000)
            .unwrap();
        let b = s
            .add_profile_suggestion("Falcon is the Q4 launch", None, 2_000)
            .unwrap();
        assert_eq!(s.list_profile_suggestions("pending").unwrap().len(), 2);
        assert!(s.set_profile_suggestion_status(a, "dismissed").unwrap());
        let pending = s.list_profile_suggestions("pending").unwrap();
        assert_eq!(pending.iter().map(|x| x.id).collect::<Vec<_>>(), vec![b]);
        assert_eq!(
            s.list_profile_suggestions("dismissed").unwrap()[0].text,
            "Priya is the CFO"
        );
        assert!(s.delete_profile_suggestion(b).unwrap());
        assert!(!s.delete_profile_suggestion(b).unwrap());
    }

    #[test]
    fn a_suggestion_with_a_deleted_source_meeting_is_still_listed() {
        let s = store();
        s.add_profile_suggestion("x", Some(999), 1).unwrap();
        assert_eq!(
            s.list_profile_suggestions("pending").unwrap()[0].source_meeting_id,
            Some(999)
        );
    }

    #[test]
    fn meetings_between_returns_only_summarized_meetings_inside_the_window_in_time_order() {
        let s = store();
        let early = meeting(&s, "early", 1_000);
        let late = meeting(&s, "late", 5_000);
        let unsummarized = meeting(&s, "no summary", 3_000);
        let outside = meeting(&s, "outside", 90_000);
        for id in [early, late, outside] {
            s.update_meeting_summary(id, "tldr", &[], &[], &[], "p", "m")
                .unwrap();
        }
        let _ = unsummarized;
        let got = s.meetings_between(0, 10_000).unwrap();
        assert_eq!(
            got.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(),
            vec!["early", "late"]
        );
    }

    #[test]
    fn the_day_and_attempt_markers_persist_and_start_empty() {
        let s = store();
        assert_eq!(s.profile_suggest_last_day().unwrap(), None);
        s.set_profile_suggest_last_day("2026-10-06").unwrap();
        assert_eq!(
            s.profile_suggest_last_day().unwrap().as_deref(),
            Some("2026-10-06")
        );
        assert_eq!(s.profile_suggest_last_attempt().unwrap(), None);
        s.set_profile_suggest_last_attempt(42).unwrap();
        assert_eq!(s.profile_suggest_last_attempt().unwrap(), Some(42));
    }

    #[test]
    fn deleting_a_meeting_or_sweeping_it_by_age_removes_the_suggestions_derived_from_it() {
        let s = store();
        let kept = meeting(&s, "kept", 90_000);
        let forgotten = meeting(&s, "forgotten", 1_000);
        let expired = meeting(&s, "expired", 2_000);
        s.add_profile_suggestion("from kept", Some(kept), 1)
            .unwrap();
        let dismissed = s
            .add_profile_suggestion("dismissed from forgotten", Some(forgotten), 1)
            .unwrap();
        s.set_profile_suggestion_status(dismissed, "dismissed")
            .unwrap();
        s.add_profile_suggestion("pending from forgotten", Some(forgotten), 1)
            .unwrap();
        s.add_profile_suggestion("from expired", Some(expired), 1)
            .unwrap();
        s.add_profile_suggestion("no source", None, 1).unwrap();
        assert!(s.delete_meeting(forgotten).unwrap());
        s.delete_meetings_before(2_001).unwrap();
        let texts: Vec<String> = ["pending", "dismissed"]
            .iter()
            .flat_map(|st| s.list_profile_suggestions(st).unwrap())
            .map(|x| x.text)
            .collect();
        assert_eq!(
            texts,
            vec!["from kept".to_string(), "no source".to_string()]
        );
    }

    fn sug(source_id: i64, text: &str, at: i64) -> super::AppliedProfileSuggestion {
        super::AppliedProfileSuggestion {
            source_id,
            text: text.into(),
            created_at_unix_ms: at,
        }
    }

    #[test]
    fn a_locally_added_suggestion_enqueues_one_outbox_row() {
        let store = store();
        store.set_own_device_id("dev-a");
        store
            .add_profile_suggestion("Prefers mornings", None, 1_000)
            .unwrap();
        assert_eq!(
            store
                .list_outbox_operations()
                .unwrap()
                .iter()
                .filter(|o| o.1 == "profile_suggestion")
                .count(),
            1
        );
    }

    #[test]
    fn a_synced_suggestion_is_pending_once_per_origin_id_and_never_enqueued() {
        let store = store();
        store.set_own_device_id("dev-b");
        assert!(store
            .apply_synced_profile_suggestion("dev-a", &sug(7, "Prefers mornings", 1_000))
            .unwrap()
            .is_some());
        assert!(
            store
                .apply_synced_profile_suggestion("dev-a", &sug(7, "Prefers mornings", 1_000))
                .unwrap()
                .is_none(),
            "same origin id twice"
        );
        assert_eq!(store.list_profile_suggestions("pending").unwrap().len(), 1);
        assert!(store.list_outbox_operations().unwrap().is_empty());
    }

    #[test]
    fn the_same_text_from_two_devices_is_shown_once() {
        let store = store();
        store
            .apply_synced_profile_suggestion("dev-a", &sug(1, "Prefers mornings", 1_000))
            .unwrap();
        assert!(store
            .apply_synced_profile_suggestion("dev-c", &sug(9, "  prefers MORNINGS ", 1_100))
            .unwrap()
            .is_none());
        assert_eq!(store.list_profile_suggestions("pending").unwrap().len(), 1);
    }

    #[test]
    fn has_synced_suggestion_since_ignores_local_rows() {
        let store = store();
        store.add_profile_suggestion("local", None, 5_000).unwrap();
        assert!(!store.has_synced_profile_suggestion_since(0).unwrap());
        store
            .apply_synced_profile_suggestion("dev-a", &sug(1, "from peer", 6_000))
            .unwrap();
        assert!(store.has_synced_profile_suggestion_since(5_500).unwrap());
        assert!(!store.has_synced_profile_suggestion_since(7_000).unwrap());
    }

    #[test]
    fn the_retention_sweep_removes_synced_suggestions_older_than_the_cutoff() {
        let s = store();
        s.apply_synced_profile_suggestion("dev-a", &sug(1, "old synced", 1_000))
            .unwrap();
        s.apply_synced_profile_suggestion("dev-a", &sug(2, "recent synced", 90_000))
            .unwrap();
        s.add_profile_suggestion("local no source", None, 1_000)
            .unwrap();
        s.delete_meetings_before(2_001).unwrap();
        let texts: Vec<String> = s
            .list_profile_suggestions("pending")
            .unwrap()
            .into_iter()
            .map(|x| x.text)
            .collect();
        assert_eq!(
            texts,
            vec!["recent synced".to_string(), "local no source".to_string()]
        );
    }
}
