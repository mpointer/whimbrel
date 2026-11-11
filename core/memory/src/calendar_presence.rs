//! Which calendar account last reported each meeting, and the removed marker that
//! follows from it. The table is local to this device (never a sync row type): it is
//! how a computer knows a meeting is gone from every account it can see, so it can mark
//! its own `items` row removed and let the normal items sync carry that to other devices.

use std::collections::HashSet;

use rusqlite::params;
use serde::Serialize;

use crate::calendar_flags::CalendarFlags;
use crate::Store;

/// Entries older than this (relative to `now`) are dropped: the meeting is long over.
const PRUNE_AFTER_MS: i64 = 86_400_000;

/// A meeting an account reports, with its start so the table can be windowed and pruned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentEvent {
    pub external_id: String,
    pub start_unix_ms: i64,
}

/// How many of this device's rows a presence change marked removed or brought back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct MarkOutcome {
    pub marked: usize,
    pub unmarked: usize,
}

impl Store {
    /// `present` is the full list `account` reports for `source`. Inside `window`
    /// (`[from, to)` on start time; everywhere when `None`) entries not in `present`
    /// become absent. Entries outside the window are left alone.
    pub fn calendar_presence_replace(
        &self,
        source: &str,
        account: &str,
        window: Option<(i64, i64)>,
        present: &[PresentEvent],
        now_unix_ms: i64,
    ) -> rusqlite::Result<MarkOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        let keep: HashSet<&str> = present.iter().map(|p| p.external_id.as_str()).collect();
        let stale: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT external_id FROM calendar_presence \
                 WHERE source = ?1 AND account = ?2 AND present = 1 \
                   AND (?3 IS NULL OR (start_unix_ms >= ?3 AND start_unix_ms < ?4))",
            )?;
            let rows = stmt.query_map(
                params![source, account, window.map(|w| w.0), window.map(|w| w.1)],
                |r| r.get::<_, String>(0),
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for id in stale.iter().filter(|id| !keep.contains(id.as_str())) {
            tx.execute(
                "UPDATE calendar_presence SET present = 0 \
                 WHERE source = ?1 AND account = ?2 AND external_id = ?3",
                params![source, account, id],
            )?;
        }
        upsert_present(&tx, source, account, present)?;
        let outcome = self.refresh_calendar_marks(&tx, now_unix_ms)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// A delta for `account`: `present` events are reported now, `absent_event_ids` are
    /// gone (an id matches `external_id = id` or `id#...`). A present event also sets its
    /// other occurrences (`id#<other start>`) absent, so a moved meeting's old slot goes.
    pub fn calendar_presence_update(
        &self,
        source: &str,
        account: &str,
        present: &[PresentEvent],
        absent_event_ids: &[String],
        now_unix_ms: i64,
    ) -> rusqlite::Result<MarkOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        for id in absent_event_ids {
            tx.execute(
                "UPDATE calendar_presence SET present = 0 \
                 WHERE source = ?1 AND account = ?2 \
                   AND (external_id = ?3 OR substr(external_id, 1, length(?3) + 1) = ?3 || '#')",
                params![source, account, id],
            )?;
        }
        // A present event's `external_id` is `"{event id}#{start secs}"` (or just the id
        // in old-format rows). Its other occurrences are `"{event id}#..."`, so match on
        // the id up to the last '#'. Event ids are assumed to contain no '#' (Google ids
        // are base32hex).
        for p in present {
            let event_id = p.external_id.rsplit_once('#').map_or(p.external_id.as_str(), |(id, _)| id);
            tx.execute(
                "UPDATE calendar_presence SET present = 0 \
                 WHERE source = ?1 AND account = ?2 AND external_id != ?3 \
                   AND substr(external_id, 1, length(?4) + 1) = ?4 || '#'",
                params![source, account, p.external_id, event_id],
            )?;
        }
        upsert_present(&tx, source, account, present)?;
        let outcome = self.refresh_calendar_marks(&tx, now_unix_ms)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// The account was disconnected: everything it reported is now absent. For Google
    /// the account's sync token goes in the same transaction, so a reconnect starts with
    /// a baseline whose replace brings back whatever is still on the calendar (a delta
    /// would only report changes, and the hidden meetings would stay hidden).
    pub fn forget_calendar_account(
        &self,
        source: &str,
        account: &str,
        now_unix_ms: i64,
    ) -> rusqlite::Result<MarkOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE calendar_presence SET present = 0 WHERE source = ?1 AND account = ?2",
            params![source, account],
        )?;
        if source == "google_calendar" {
            // Same connection as `tx`, so this is part of the transaction.
            self.clear_calendar_sync_token(account)?;
        }
        let outcome = self.refresh_calendar_marks(&tx, now_unix_ms)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// Prunes old entries, then, for this device's own future `calendar_event` rows that
    /// have presence entries, marks those with no account reporting them and unmarks
    /// those some account reports again. Every change stamps `changed_at = now` and a
    /// fresh `local_seq`, so `unpushed_item_rows` pushes it. No own device id: no marks.
    fn refresh_calendar_marks(
        &self,
        tx: &rusqlite::Transaction<'_>,
        now_unix_ms: i64,
    ) -> rusqlite::Result<MarkOutcome> {
        tx.execute(
            "DELETE FROM calendar_presence WHERE start_unix_ms < ?1",
            params![now_unix_ms - PRUNE_AFTER_MS],
        )?;
        let Some(own) = self.own_device_id() else {
            return Ok(MarkOutcome::default());
        };
        let rows: Vec<(i64, Option<String>, bool)> = {
            let mut stmt = tx.prepare(
                "SELECT i.id, i.flags, MAX(p.present) FROM items i \
                 JOIN calendar_presence p ON p.source = i.source AND p.external_id = i.external_id \
                 WHERE i.device_id = ?1 AND i.kind = 'calendar_event' AND i.ts_unix_ms > ?2 \
                 GROUP BY i.id",
            )?;
            let mapped = stmt.query_map(params![own, now_unix_ms], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? == 1))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut outcome = MarkOutcome::default();
        for (id, flags, any_present) in rows {
            let removed = CalendarFlags::parse(flags.as_deref()).removed;
            if any_present == !removed {
                continue;
            }
            let new_flags = CalendarFlags {
                removed: !any_present,
                changed_at: now_unix_ms,
            }
            .to_json();
            let seq = Store::next_local_seq_tx(tx)?;
            tx.execute(
                "UPDATE items SET flags = ?1, local_seq = ?2 WHERE id = ?3",
                params![new_flags, seq, id],
            )?;
            if any_present {
                outcome.unmarked += 1;
            } else {
                outcome.marked += 1;
            }
        }
        Ok(outcome)
    }
}

fn upsert_present(
    tx: &rusqlite::Transaction<'_>,
    source: &str,
    account: &str,
    present: &[PresentEvent],
) -> rusqlite::Result<()> {
    for p in present {
        tx.execute(
            "INSERT INTO calendar_presence (source, external_id, account, start_unix_ms, present) \
             VALUES (?1, ?2, ?3, ?4, 1) \
             ON CONFLICT(source, external_id, account) DO UPDATE SET \
                start_unix_ms = excluded.start_unix_ms, present = 1",
            params![source, p.external_id, account, p.start_unix_ms],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewCalendarEventItem;

    const SRC: &str = "google_calendar";
    const NOW: i64 = 1_000_000;
    const FUTURE: i64 = 5_000_000;

    fn store(own: Option<&str>) -> Store {
        let s = Store::open_in_memory().unwrap();
        if let Some(o) = own {
            s.set_own_device_id(o);
        }
        s
    }

    fn insert(s: &Store, device: &str, id: &str, ts: i64) {
        s.upsert_calendar_event_items_at(
            &[NewCalendarEventItem {
                device_id: device.into(),
                source: SRC.into(),
                ts_unix_ms: ts,
                actors: vec![],
                title: id.into(),
                external_id: id.into(),
            }],
            10,
        )
        .unwrap();
    }

    fn pe(id: &str, start: i64) -> PresentEvent {
        PresentEvent { external_id: id.into(), start_unix_ms: start }
    }

    fn flags(s: &Store, id: &str) -> CalendarFlags {
        let f: Option<String> = s
            .conn
            .query_row("SELECT flags FROM items WHERE external_id = ?1", [id], |r| r.get(0))
            .unwrap();
        CalendarFlags::parse(f.as_deref())
    }

    fn seq(s: &Store, id: &str) -> i64 {
        s.conn
            .query_row("SELECT local_seq FROM items WHERE external_id = ?1", [id], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn cancelled_meeting_is_marked() {
        let s = store(Some("me"));
        insert(&s, "me", "a", FUTURE);
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", FUTURE)], NOW).unwrap();
        assert!(!flags(&s, "a").removed);
        let out = s.calendar_presence_update(SRC, "x@g", &[], &["a".into()], NOW + 5).unwrap();
        assert_eq!(out, MarkOutcome { marked: 1, unmarked: 0 });
        assert_eq!(flags(&s, "a"), CalendarFlags { removed: true, changed_at: NOW + 5 });
    }

    #[test]
    fn moved_meeting_marks_old_slot_keeps_new() {
        let s = store(Some("me"));
        insert(&s, "me", "m#100", FUTURE);
        insert(&s, "me", "m#200", FUTURE + 1000);
        s.calendar_presence_replace(
            SRC, "x@g", None, &[pe("m#100", FUTURE), pe("m#200", FUTURE + 1000)], NOW,
        )
        .unwrap();
        // The same meeting is reported again at a new time, as the real `id#start` id.
        insert(&s, "me", "m#300", FUTURE + 9000);
        let out = s
            .calendar_presence_update(SRC, "x@g", &[pe("m#300", FUTURE + 9000)], &[], NOW + 1)
            .unwrap();
        assert_eq!(out.marked, 2);
        assert!(flags(&s, "m#100").removed);
        assert!(flags(&s, "m#200").removed);
        assert!(!flags(&s, "m#300").removed);
    }

    #[test]
    fn reappearing_meeting_is_unmarked() {
        let s = store(Some("me"));
        insert(&s, "me", "a", FUTURE);
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", FUTURE)], NOW).unwrap();
        s.calendar_presence_update(SRC, "x@g", &[], &["a".into()], NOW + 1).unwrap();
        assert!(flags(&s, "a").removed);
        let out = s.calendar_presence_update(SRC, "x@g", &[pe("a", FUTURE)], &[], NOW + 2).unwrap();
        assert_eq!(out, MarkOutcome { marked: 0, unmarked: 1 });
        assert_eq!(flags(&s, "a"), CalendarFlags { removed: false, changed_at: NOW + 2 });
    }

    #[test]
    fn declined_in_one_account_accepted_in_another_stays_shown() {
        let s = store(Some("me"));
        insert(&s, "me", "a", FUTURE);
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", FUTURE)], NOW).unwrap();
        s.calendar_presence_replace(SRC, "y@g", None, &[pe("a", FUTURE)], NOW).unwrap();
        let out = s.calendar_presence_replace(SRC, "x@g", None, &[], NOW + 1).unwrap();
        assert_eq!(out, MarkOutcome::default());
        assert!(!flags(&s, "a").removed);
        let out = s.calendar_presence_replace(SRC, "y@g", None, &[], NOW + 2).unwrap();
        assert_eq!(out.marked, 1);
        assert!(flags(&s, "a").removed);
    }

    #[test]
    fn forgotten_account_marks_its_future_rows() {
        let s = store(Some("me"));
        insert(&s, "me", "a", FUTURE);
        insert(&s, "me", "b", FUTURE);
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", FUTURE), pe("b", FUTURE)], NOW).unwrap();
        s.calendar_presence_replace(SRC, "y@g", None, &[pe("b", FUTURE)], NOW).unwrap();
        let out = s.forget_calendar_account(SRC, "x@g", NOW + 1).unwrap();
        assert_eq!(out.marked, 1);
        assert!(flags(&s, "a").removed);
        assert!(!flags(&s, "b").removed);
    }

    #[test]
    fn untracked_rows_are_never_marked() {
        let s = store(Some("me"));
        insert(&s, "me", "old", FUTURE);
        insert(&s, "me", "a", FUTURE);
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", FUTURE)], NOW).unwrap();
        let out = s.calendar_presence_replace(SRC, "x@g", None, &[], NOW + 1).unwrap();
        assert_eq!(out.marked, 1);
        assert!(!flags(&s, "old").removed);
        s.calendar_presence_update(SRC, "x@g", &[], &["old".into()], NOW + 2).unwrap();
        assert!(!flags(&s, "old").removed);
    }

    #[test]
    fn other_devices_rows_are_never_marked() {
        let s = store(Some("me"));
        insert(&s, "them", "a", FUTURE);
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", FUTURE)], NOW).unwrap();
        let out = s.calendar_presence_replace(SRC, "x@g", None, &[], NOW + 1).unwrap();
        assert_eq!(out, MarkOutcome::default());
        assert!(!flags(&s, "a").removed);
    }

    #[test]
    fn started_meetings_are_never_marked() {
        let s = store(Some("me"));
        insert(&s, "me", "a", NOW + 10);
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", NOW + 10)], NOW).unwrap();
        // now == start: already started.
        let out = s.calendar_presence_replace(SRC, "x@g", None, &[], NOW + 10).unwrap();
        assert_eq!(out, MarkOutcome::default());
        assert!(!flags(&s, "a").removed);
    }

    #[test]
    fn replace_outside_window_leaves_entries_alone() {
        let s = store(Some("me"));
        insert(&s, "me", "in", FUTURE);
        insert(&s, "me", "out", FUTURE + 100_000);
        s.calendar_presence_replace(
            SRC, "x@g", None, &[pe("in", FUTURE), pe("out", FUTURE + 100_000)], NOW,
        )
        .unwrap();
        let out = s
            .calendar_presence_replace(SRC, "x@g", Some((FUTURE, FUTURE + 1000)), &[], NOW + 1)
            .unwrap();
        assert_eq!(out.marked, 1);
        assert!(flags(&s, "in").removed);
        assert!(!flags(&s, "out").removed);
    }

    #[test]
    fn no_own_device_id_marks_nothing() {
        let s = store(None);
        insert(&s, "me", "a", FUTURE);
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", FUTURE)], NOW).unwrap();
        let out = s.calendar_presence_replace(SRC, "x@g", None, &[], NOW + 1).unwrap();
        assert_eq!(out, MarkOutcome::default());
        assert!(!flags(&s, "a").removed);
    }

    #[test]
    fn marking_bumps_local_seq_so_the_row_is_pushed() {
        let s = store(Some("me"));
        insert(&s, "me", "a", FUTURE);
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", FUTURE)], NOW).unwrap();
        let before = seq(&s, "a");
        s.calendar_presence_replace(SRC, "x@g", None, &[], NOW + 1).unwrap();
        assert!(seq(&s, "a") > before);
        let pushed = s.unpushed_item_rows("me", before).unwrap();
        assert_eq!(pushed.len(), 1);
    }

    #[test]
    fn repeating_the_same_presence_writes_nothing() {
        for marked in [true, false] {
            let s = store(Some("me"));
            insert(&s, "me", "a", FUTURE);
            s.calendar_presence_replace(SRC, "x@g", None, &[pe("a", FUTURE)], NOW).unwrap();
            let (present, absent): (Vec<PresentEvent>, Vec<String>) = if marked {
                s.calendar_presence_update(SRC, "x@g", &[], &["a".into()], NOW + 1).unwrap();
                assert!(flags(&s, "a").removed);
                (vec![], vec!["a".into()])
            } else {
                (vec![pe("a", FUTURE)], vec![])
            };
            let seq0 = seq(&s, "a");
            let flags0 = flags(&s, "a");
            let replace_input: Vec<PresentEvent> = present.clone();
            let out = s.calendar_presence_replace(SRC, "x@g", None, &replace_input, NOW + 2).unwrap();
            assert_eq!(out, MarkOutcome::default(), "replace, marked={marked}");
            assert_eq!((seq(&s, "a"), flags(&s, "a")), (seq0, flags0), "replace, marked={marked}");
            let out = s.calendar_presence_update(SRC, "x@g", &present, &absent, NOW + 3).unwrap();
            assert_eq!(out, MarkOutcome::default(), "update, marked={marked}");
            assert_eq!((seq(&s, "a"), flags(&s, "a")), (seq0, flags0), "update, marked={marked}");
        }
    }

    #[test]
    fn old_entries_are_pruned() {
        let s = store(Some("me"));
        s.calendar_presence_replace(SRC, "x@g", None, &[pe("gone", 10)], 10).unwrap();
        s.calendar_presence_replace(SRC, "x@g", None, &[], 10 + PRUNE_AFTER_MS + 1).unwrap();
        let n: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM calendar_presence", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
