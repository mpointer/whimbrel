//! Why a meeting summary was written by a different provider than the one the user
//! picked (for example Whimbrel AI picked, a local model answered). Device-local and
//! content-free: provider ids, a model id and a reason code, never a prompt or an error
//! text. A side table rather than columns on `meeting`, so the synced row is unchanged.

use rusqlite::{params, Connection, OptionalExtension};

use crate::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryFallbackRow {
    pub preferred: String,
    pub used: String,
    pub model: String,
    pub reason: String,
    pub http_status: Option<u16>,
}

pub(crate) fn ensure_summary_fallback_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meeting_summary_fallback (
            meeting_id INTEGER PRIMARY KEY,
            preferred TEXT NOT NULL,
            used TEXT NOT NULL,
            model TEXT NOT NULL,
            reason TEXT NOT NULL,
            http_status INTEGER
        );",
    )
}

impl Store {
    /// Records the fallback behind the meeting's current summary, or forgets an earlier
    /// one (`None`) because the summary was just written by the provider that was picked.
    pub fn set_summary_fallback(&self, meeting_id: i64, row: Option<&SummaryFallbackRow>) -> rusqlite::Result<()> {
        match row {
            Some(r) => self.conn.execute(
                "INSERT INTO meeting_summary_fallback (meeting_id, preferred, used, model, reason, http_status) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(meeting_id) DO UPDATE SET preferred = ?2, used = ?3, model = ?4, reason = ?5, http_status = ?6",
                params![meeting_id, r.preferred, r.used, r.model, r.reason, r.http_status.map(i64::from)],
            ),
            None => self.conn.execute("DELETE FROM meeting_summary_fallback WHERE meeting_id = ?1", params![meeting_id]),
        }
        .map(|_| ())
    }

    /// The fallback behind the meeting's summary, only while that summary is still the one
    /// it was recorded for: a later summary from another provider or model (a synced one,
    /// or an edit that cleared it) leaves the old row stale, and it reads as `None`.
    pub fn summary_fallback(&self, meeting_id: i64) -> rusqlite::Result<Option<SummaryFallbackRow>> {
        self.conn
            .query_row(
                "SELECT f.preferred, f.used, f.model, f.reason, f.http_status FROM meeting_summary_fallback f \
                 JOIN meeting m ON m.id = f.meeting_id \
                 WHERE f.meeting_id = ?1 AND m.summary_tldr IS NOT NULL AND m.summary_provider = f.used AND m.summary_model = f.model",
                params![meeting_id],
                |r| {
                    Ok(SummaryFallbackRow {
                        preferred: r.get(0)?,
                        used: r.get(1)?,
                        model: r.get(2)?,
                        reason: r.get(3)?,
                        http_status: r.get::<_, Option<i64>>(4)?.and_then(|s| u16::try_from(s).ok()),
                    })
                },
            )
            .optional()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeetingStatus, MeetingTrigger, NewMeeting};

    fn row(reason: &str) -> SummaryFallbackRow {
        SummaryFallbackRow {
            preferred: "whimbrel".into(),
            used: "ollama".into(),
            model: "gemma4:12b".into(),
            reason: reason.into(),
            http_status: None,
        }
    }

    #[test]
    fn a_notice_reads_back_only_while_its_summary_is_the_stored_one() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store.set_summary_fallback(id, Some(&row("not_signed_in"))).unwrap();
        assert_eq!(store.summary_fallback(id).unwrap(), None, "no summary yet");
        store.update_meeting_summary(id, "t", &[], &[], &[], "ollama", "gemma4:12b").unwrap();
        assert_eq!(store.summary_fallback(id).unwrap(), Some(row("not_signed_in")));
        store.set_summary_fallback(id, Some(&SummaryFallbackRow { http_status: Some(502), ..row("unavailable") })).unwrap();
        assert_eq!(store.summary_fallback(id).unwrap().unwrap().http_status, Some(502), "one row per meeting, replaced");
        store.update_meeting_summary(id, "t", &[], &[], &[], "whimbrel", "claude-sonnet-5-5").unwrap();
        assert_eq!(store.summary_fallback(id).unwrap(), None, "another provider's summary leaves it stale");
        store.set_summary_fallback(id, None).unwrap();
    }
}
