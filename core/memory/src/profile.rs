//! Profile suggestions and the daily-job markers. Never mirrored into `items`. A suggestion
//! created on this device syncs as a created row only; its status stays per device. See
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

pub(crate) fn ensure_profile_suggestion_table(conn: &Connection) -> rusqlite::Result<()> {
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
            conn.execute(&format!("ALTER TABLE profile_suggestion ADD COLUMN {name} {ty}"), [])?;
        }
    }
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_profile_suggestion_origin ON profile_suggestion(origin_device_id, source_id) WHERE origin_device_id IS NOT NULL",
        [],
    )?;
    Ok(())
}

impl Store {
    pub fn add_profile_suggestion(&self, text: &str, source_meeting_id: Option<i64>, now_unix_ms: i64) -> rusqlite::Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO profile_suggestion (text, source_meeting_id, status, created_at_unix_ms) VALUES (?1, ?2, 'pending', ?3)",
            params![text, source_meeting_id, now_unix_ms],
        )?;
        let id = tx.last_insert_rowid();
        self.enqueue_profile_suggestion_outbox(
            &tx,
            id,
            &AppliedProfileSuggestion { source_id: id, text: text.to_string(), created_at_unix_ms: now_unix_ms },
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Applies a suggestion pulled from another device: pending, once per `(origin, source_id)`,
    /// bounded, skipped when the same text is already a suggestion here, and never enqueued.
    pub fn apply_synced_profile_suggestion(&self, origin_device_id: &str, s: &AppliedProfileSuggestion) -> rusqlite::Result<Option<i64>> {
        let text: String = s.text.trim().chars().take(500).collect();
        if text.is_empty() {
            return Ok(None);
        }
        let created = s.created_at_unix_ms.min(crate::now_unix_ms() + crate::SYNCED_RUN_MAX_FUTURE_MS);
        let key = text.to_lowercase();
        for status in ["pending", "dismissed"] {
            if self.list_profile_suggestions(status)?.iter().any(|x| x.text.trim().to_lowercase() == key) {
                return Ok(None);
            }
        }
        let changed = self.conn.execute(
            "INSERT INTO profile_suggestion (text, source_meeting_id, status, created_at_unix_ms, origin_device_id, source_id) \
             VALUES (?1, NULL, 'pending', ?2, ?3, ?4) ON CONFLICT DO NOTHING",
            params![text, created, origin_device_id, s.source_id],
        )?;
        Ok((changed > 0).then(|| self.conn.last_insert_rowid()))
    }

    /// Whether another device created a suggestion at or after `since_unix_ms` (the start of the
    /// local day): one suggestion run per day across devices.
    pub fn has_synced_profile_suggestion_since(&self, since_unix_ms: i64) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM profile_suggestion WHERE origin_device_id IS NOT NULL AND created_at_unix_ms >= ?1)",
            params![since_unix_ms],
            |r| r.get(0),
        )
    }

    pub fn list_profile_suggestions(&self, status: &str) -> rusqlite::Result<Vec<ProfileSuggestion>> {
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

    pub fn set_profile_suggestion_status(&self, id: i64, status: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.execute("UPDATE profile_suggestion SET status = ?1 WHERE id = ?2", params![status, id])? > 0)
    }

    pub fn delete_profile_suggestion(&self, id: i64) -> rusqlite::Result<bool> {
        Ok(self.conn.execute("DELETE FROM profile_suggestion WHERE id = ?1", params![id])? > 0)
    }

    /// Summarized meetings that started in `[start, end)`, oldest first.
    pub fn meetings_between(&self, start_unix_ms: i64, end_unix_ms: i64) -> rusqlite::Result<Vec<Meeting>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, audio_path, status, transcript_path, notes, summary_tldr, summary_decisions_json, summary_action_items_json, summary_open_questions_json, summary_provider, summary_model, local_seq, estimated_speaker_count, away_outcome, away_outcome_at_unix_ms, keep_media FROM meeting WHERE start_unix_ms >= ?1 AND start_unix_ms < ?2 AND summary_tldr IS NOT NULL ORDER BY start_unix_ms",
        )?;
        let rows = stmt.query_map(params![start_unix_ms, end_unix_ms], Self::row_to_meeting)?;
        rows.collect()
    }

    fn kv_get(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn.query_row("SELECT value FROM kv_meta WHERE key = ?1", params![key], |r| r.get(0)).optional()
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
        let a = s.add_profile_suggestion("Priya is the CFO", Some(7), 1_000).unwrap();
        let b = s.add_profile_suggestion("Falcon is the Q4 launch", None, 2_000).unwrap();
        assert_eq!(s.list_profile_suggestions("pending").unwrap().len(), 2);
        assert!(s.set_profile_suggestion_status(a, "dismissed").unwrap());
        let pending = s.list_profile_suggestions("pending").unwrap();
        assert_eq!(pending.iter().map(|x| x.id).collect::<Vec<_>>(), vec![b]);
        assert_eq!(s.list_profile_suggestions("dismissed").unwrap()[0].text, "Priya is the CFO");
        assert!(s.delete_profile_suggestion(b).unwrap());
        assert!(!s.delete_profile_suggestion(b).unwrap());
    }

    #[test]
    fn a_suggestion_with_a_deleted_source_meeting_is_still_listed() {
        let s = store();
        s.add_profile_suggestion("x", Some(999), 1).unwrap();
        assert_eq!(s.list_profile_suggestions("pending").unwrap()[0].source_meeting_id, Some(999));
    }

    #[test]
    fn meetings_between_returns_only_summarized_meetings_inside_the_window_in_time_order() {
        let s = store();
        let early = meeting(&s, "early", 1_000);
        let late = meeting(&s, "late", 5_000);
        let unsummarized = meeting(&s, "no summary", 3_000);
        let outside = meeting(&s, "outside", 90_000);
        for id in [early, late, outside] {
            s.update_meeting_summary(id, "tldr", &[], &[], &[], "p", "m").unwrap();
        }
        let _ = unsummarized;
        let got = s.meetings_between(0, 10_000).unwrap();
        assert_eq!(got.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), vec!["early", "late"]);
    }

    #[test]
    fn the_day_and_attempt_markers_persist_and_start_empty() {
        let s = store();
        assert_eq!(s.profile_suggest_last_day().unwrap(), None);
        s.set_profile_suggest_last_day("2026-10-06").unwrap();
        assert_eq!(s.profile_suggest_last_day().unwrap().as_deref(), Some("2026-10-06"));
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
        s.add_profile_suggestion("from kept", Some(kept), 1).unwrap();
        let dismissed = s.add_profile_suggestion("dismissed from forgotten", Some(forgotten), 1).unwrap();
        s.set_profile_suggestion_status(dismissed, "dismissed").unwrap();
        s.add_profile_suggestion("pending from forgotten", Some(forgotten), 1).unwrap();
        s.add_profile_suggestion("from expired", Some(expired), 1).unwrap();
        s.add_profile_suggestion("no source", None, 1).unwrap();
        assert!(s.delete_meeting(forgotten).unwrap());
        s.delete_meetings_before(2_001).unwrap();
        let texts: Vec<String> = ["pending", "dismissed"]
            .iter()
            .flat_map(|st| s.list_profile_suggestions(st).unwrap())
            .map(|x| x.text)
            .collect();
        assert_eq!(texts, vec!["from kept".to_string(), "no source".to_string()]);
    }

    fn sug(source_id: i64, text: &str, at: i64) -> super::AppliedProfileSuggestion {
        super::AppliedProfileSuggestion { source_id, text: text.into(), created_at_unix_ms: at }
    }

    #[test]
    fn a_locally_added_suggestion_enqueues_one_outbox_row() {
        let store = store();
        store.set_own_device_id("dev-a");
        store.add_profile_suggestion("Prefers mornings", None, 1_000).unwrap();
        assert_eq!(store.list_outbox_operations().unwrap().iter().filter(|o| o.1 == "profile_suggestion").count(), 1);
    }

    #[test]
    fn a_synced_suggestion_is_pending_once_per_origin_id_and_never_enqueued() {
        let store = store();
        store.set_own_device_id("dev-b");
        assert!(store.apply_synced_profile_suggestion("dev-a", &sug(7, "Prefers mornings", 1_000)).unwrap().is_some());
        assert!(store.apply_synced_profile_suggestion("dev-a", &sug(7, "Prefers mornings", 1_000)).unwrap().is_none(), "same origin id twice");
        assert_eq!(store.list_profile_suggestions("pending").unwrap().len(), 1);
        assert!(store.list_outbox_operations().unwrap().is_empty());
    }

    #[test]
    fn the_same_text_from_two_devices_is_shown_once() {
        let store = store();
        store.apply_synced_profile_suggestion("dev-a", &sug(1, "Prefers mornings", 1_000)).unwrap();
        assert!(store.apply_synced_profile_suggestion("dev-c", &sug(9, "  prefers MORNINGS ", 1_100)).unwrap().is_none());
        assert_eq!(store.list_profile_suggestions("pending").unwrap().len(), 1);
    }

    #[test]
    fn has_synced_suggestion_since_ignores_local_rows() {
        let store = store();
        store.add_profile_suggestion("local", None, 5_000).unwrap();
        assert!(!store.has_synced_profile_suggestion_since(0).unwrap());
        store.apply_synced_profile_suggestion("dev-a", &sug(1, "from peer", 6_000)).unwrap();
        assert!(store.has_synced_profile_suggestion_since(5_500).unwrap());
        assert!(!store.has_synced_profile_suggestion_since(7_000).unwrap());
    }

    #[test]
    fn the_retention_sweep_removes_synced_suggestions_older_than_the_cutoff() {
        let s = store();
        s.apply_synced_profile_suggestion("dev-a", &sug(1, "old synced", 1_000)).unwrap();
        s.apply_synced_profile_suggestion("dev-a", &sug(2, "recent synced", 90_000)).unwrap();
        s.add_profile_suggestion("local no source", None, 1_000).unwrap();
        s.delete_meetings_before(2_001).unwrap();
        let texts: Vec<String> = s.list_profile_suggestions("pending").unwrap().into_iter().map(|x| x.text).collect();
        assert_eq!(texts, vec!["recent synced".to_string(), "local no source".to_string()]);
    }
}
