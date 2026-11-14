//! Calendar events other devices wrote into `items` and sync delivered, for the shells'
//! Today and Upcoming lists. Display only: nothing here says the meeting can be recorded.

use rusqlite::params;

use crate::{calendar_flags, Store};

/// One calendar event another device sent. The item holds no end time, RSVP, link or
/// all-day marker, so none appear here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SyncedCalendarEvent {
    pub row_id: i64,
    /// `"{event id}#{start secs}"`, as the sending device stored it.
    pub external_id: String,
    /// `"apple_calendar"` or `"google_calendar"`; empty when the row carries none.
    pub source: String,
    pub title: String,
    pub start_unix_ms: i64,
    pub attendees: Vec<String>,
    /// The device that last wrote the row, when known.
    pub device_id: Option<String>,
}

impl Store {
    /// `calendar_event` rows starting in `[from_unix_ms, until_unix_ms)` that were not
    /// last written by `exclude_device_id` (this device: its events are already read
    /// from the calendar itself) and are not marked removed. Soonest first. Rows are not deduped here: the same meeting
    /// can arrive from several sources.
    pub fn synced_calendar_events(
        &self,
        exclude_device_id: &str,
        from_unix_ms: i64,
        until_unix_ms: i64,
    ) -> rusqlite::Result<Vec<SyncedCalendarEvent>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, external_id, source, title, ts_unix_ms, actors, device_id FROM items \
             WHERE kind = 'calendar_event' AND external_id IS NOT NULL \
               AND ts_unix_ms >= ?1 AND ts_unix_ms < ?2 \
               AND device_id IS NOT ?3 \
               AND {} \
             ORDER BY ts_unix_ms ASC, id ASC",
            calendar_flags::CALENDAR_NOT_REMOVED
        ))?;
        let rows = stmt.query_map(params![from_unix_ms, until_unix_ms, exclude_device_id], |row| {
            let actors: Option<String> = row.get(5)?;
            Ok(SyncedCalendarEvent {
                row_id: row.get(0)?,
                external_id: row.get(1)?,
                source: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                title: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                start_unix_ms: row.get(4)?,
                attendees: actors.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or_default(),
                device_id: row.get(6)?,
            })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewCalendarEventItem;

    fn item(device: &str, source: &str, id: &str, ts: i64, title: &str) -> NewCalendarEventItem {
        NewCalendarEventItem {
            device_id: device.into(),
            source: source.into(),
            external_id: id.into(),
            ts_unix_ms: ts,
            title: title.into(),
            actors: vec!["Ana".into()],
        }
    }

    #[test]
    fn returns_other_devices_rows_in_the_window_soonest_first() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_calendar_event_items(&[
            item("other", "apple_calendar", "late#2", 2000, "Late"),
            item("other", "apple_calendar", "early#1", 1000, "Early"),
            item("other", "apple_calendar", "out#9", 9000, "Outside"),
            item("me", "apple_calendar", "mine#1", 1500, "Mine"),
        ])
        .unwrap();
        let got = s.synced_calendar_events("me", 0, 5000).unwrap();
        let titles: Vec<&str> = got.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Early", "Late"]);
        assert_eq!(got[0].attendees, ["Ana"]);
        assert_eq!(got[0].external_id, "early#1");
        assert_eq!(got[0].device_id.as_deref(), Some("other"));
    }

    #[test]
    fn window_is_start_inclusive_end_exclusive() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_calendar_event_items(&[item("o", "google_calendar", "a#1", 1000, "A"), item("o", "google_calendar", "b#2", 2000, "B")]).unwrap();
        let got = s.synced_calendar_events("me", 1000, 2000).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "A");
    }

    #[test]
    fn a_row_marked_removed_is_left_out() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_calendar_event_items(&[item("o", "google_calendar", "a#1", 1000, "A")]).unwrap();
        s.conn.execute("UPDATE items SET flags = '{\"removed\":true,\"changed_at\":5}'", []).unwrap();
        assert!(s.synced_calendar_events("me", 0, 5000).unwrap().is_empty());
    }

    #[test]
    fn a_row_with_no_source_still_reads() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_calendar_event_items(&[item("o", "google_calendar", "a#1", 1000, "A")]).unwrap();
        s.conn.execute("UPDATE items SET source = NULL", []).unwrap();
        let got = s.synced_calendar_events("me", 0, 5000).unwrap();
        assert_eq!((got.len(), got[0].source.as_str()), (1, ""));
    }

    #[test]
    fn ignores_other_kinds() {
        let s = Store::open_in_memory().unwrap();
        s.conn
            .execute("INSERT INTO items (device_id, kind, source, ts_unix_ms, title, body_text, external_id) VALUES ('o','email','gmail',1000,'Mail','x','m1')", [])
            .unwrap();
        assert!(s.synced_calendar_events("me", 0, 5000).unwrap().is_empty());
    }
}
