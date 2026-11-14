//! When the user last emailed a meeting's recap, and to how many people. Device-local and
//! content-free (no addresses, no text): a side table rather than columns on `meeting`, so
//! the synced row is unchanged.

use rusqlite::{params, Connection, OptionalExtension};

use crate::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct RecapSent {
    pub sent_unix_ms: i64,
    pub recipient_count: i64,
}

pub(crate) fn ensure_recap_sent_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meeting_recap_sent (
            meeting_id INTEGER PRIMARY KEY,
            sent_unix_ms INTEGER NOT NULL,
            recipient_count INTEGER NOT NULL
        );",
    )
}

impl Store {
    /// Records a recap send for `meeting_id`, replacing an earlier one. `false` when the
    /// meeting doesn't exist.
    pub fn record_recap_sent(&self, meeting_id: i64, recipient_count: i64, now_unix_ms: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "INSERT INTO meeting_recap_sent (meeting_id, sent_unix_ms, recipient_count) \
             SELECT id, ?2, ?3 FROM meeting WHERE id = ?1 \
             ON CONFLICT(meeting_id) DO UPDATE SET sent_unix_ms = ?2, recipient_count = ?3",
            params![meeting_id, now_unix_ms, recipient_count.max(0)],
        )?;
        Ok(n > 0)
    }

    /// The last recap send for `meeting_id`, if any.
    pub fn recap_sent(&self, meeting_id: i64) -> rusqlite::Result<Option<RecapSent>> {
        self.conn
            .query_row(
                "SELECT sent_unix_ms, recipient_count FROM meeting_recap_sent WHERE meeting_id = ?1",
                params![meeting_id],
                |r| Ok(RecapSent { sent_unix_ms: r.get(0)?, recipient_count: r.get(1)? }),
            )
            .optional()
    }
}

#[cfg(test)]
mod tests {
    use crate::{MeetingStatus, MeetingTrigger, NewMeeting, Store};

    fn meeting(store: &Store) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Standup".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap()
    }

    #[test]
    fn a_send_is_recorded_replaced_and_needs_a_real_meeting() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store);
        assert_eq!(store.recap_sent(id).unwrap(), None);
        assert!(store.record_recap_sent(id, 4, 10).unwrap());
        assert!(store.record_recap_sent(id, 2, 20).unwrap());
        let sent = store.recap_sent(id).unwrap().unwrap();
        assert_eq!((sent.sent_unix_ms, sent.recipient_count), (20, 2));
        assert!(!store.record_recap_sent(id + 99, 1, 30).unwrap());
    }

    #[test]
    fn deleting_the_meeting_forgets_the_send() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store);
        store.record_recap_sent(id, 3, 10).unwrap();
        store.delete_meeting(id).unwrap();
        assert_eq!(store.recap_sent(id).unwrap(), None);
    }
}
