//! Google API Services "Limited Use": Gmail and Google Calendar text read through Google's
//! APIs may only reach an AI model that does not train on it. This module answers the one
//! question every prompt path asks first: is this row Google user data? The per-provider
//! rules (which links may receive it) live in `llm::limited_use`; the TypeSafe hold is here
//! because `memory` builds the TypeSafe queues itself.
//!
//! Only data read through Google's APIs counts: `gmail` emails (and the threads and
//! summaries built from them) and `google_calendar` events. An Apple Calendar (EventKit)
//! event does not, even when the calendar behind it is a Google one, and neither does
//! screen text.

/// `items.source` of a Gmail message, and `threads.source` of a Gmail thread.
pub const GMAIL_SOURCE: &str = "gmail";
/// `items.source` of a Google Calendar event read through the Google OAuth connector.
pub const GOOGLE_CALENDAR_SOURCE: &str = "google_calendar";

/// Whether TypeSafe has confirmed in writing that it does not train on API data. Until it
/// has, no Gmail or Google Calendar text is sent to TypeSafe: Reply Priorities scoring,
/// the calendar title check, topic judging and paid topic steps all leave it out.
pub const TYPESAFE_GOOGLE_DATA_CONFIRMED: bool = false;

/// Whether a row of `kind` from `source` is Google user data under Limited Use. `kind` is
/// an `items.kind` (`email`, `calendar_event`, ...), a `topic_source.kind`, or `thread`
/// for a `threads` row, or `routine_run` for a routine's output. An email with no recorded source counts as Gmail, the only email
/// source there is; a calendar event counts only when it came from Google Calendar.
pub fn is_google_user_data(kind: &str, source: Option<&str>) -> bool {
    match kind {
        "email" | "thread" => matches!(source, None | Some(GMAIL_SOURCE)),
        "calendar_event" => source == Some(GOOGLE_CALENDAR_SOURCE),
        // Generated text inherits the taint of what it was written from. A routine run
        // is written from Gmail threads and Calendar events, and some of its text (the
        // meeting list, a prep line with the event's title) is theirs verbatim; the row
        // does not record which, so every run counts, whatever its source.
        "routine_run" => true,
        _ => false,
    }
}

/// SQL true for a `topic_source s` row whose item is Google user data, the same rule as
/// [`is_google_user_data`] and, for a `meeting` source, as
/// [`Store::meeting_may_hold_google_calendar_data`] (tests hold them together). Callers alias `topic_source`
/// as `s`.
pub(crate) const TOPIC_SOURCE_IS_GOOGLE_SQL: &str = "((s.kind IN ('email', 'calendar_event') AND EXISTS ( \
     SELECT 1 FROM items gi WHERE gi.id = s.ref_id AND ( \
       (gi.kind = 'email' AND COALESCE(gi.source, 'gmail') = 'gmail') \
       OR (gi.kind = 'calendar_event' AND gi.source = 'google_calendar')))) \
   OR (s.kind = 'meeting' AND EXISTS ( \
     SELECT 1 FROM meeting gm WHERE gm.id = s.ref_id AND ( \
       gm.calendar_source = 'google' \
       OR (gm.calendar_source = 'other' AND EXISTS ( \
         SELECT 1 FROM items ge WHERE ge.kind = 'calendar_event' AND ge.source = 'google_calendar' \
         AND ge.ts_unix_ms BETWEEN gm.start_unix_ms - 43200000 AND gm.start_unix_ms + 43200000)) \
       OR (gm.calendar_source IS NULL AND gm.trigger IN ('calendar', 'away'))))))";

/// Creates the table of Ask answers' provenance (device-local, additive).
pub(crate) fn ensure_answer_provenance_table(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS answer_provenance (
            text_hash TEXT PRIMARY KEY,
            google INTEGER NOT NULL,
            recorded_unix_ms INTEGER NOT NULL
        );",
    )
}

/// The key an answer's provenance is stored under: a hash of its text without the
/// whitespace a shell may trim.
pub(crate) fn answer_key(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.trim().as_bytes()))
}

/// `meeting.calendar_source` of a meeting that came from a Google Calendar event.
pub const CALENDAR_SOURCE_GOOGLE: &str = "google";
/// `meeting.calendar_source` of a meeting known to come from another calendar.
pub const CALENDAR_SOURCE_OTHER: &str = "other";
/// Stored for an `other` that core inferred when the meeting was created: re-checked on
/// every read, since Google may not have synced the event yet. A shell's explicit
/// [`CALENDAR_SOURCE_OTHER`] is stored as [`STORED_OTHER_CONFIRMED`] and is final.
const STORED_OTHER_INFERRED: &str = "other";
const STORED_OTHER_CONFIRMED: &str = "other_confirmed";

/// How far from a meeting's start a calendar event with its title still counts as the
/// event the meeting came from.
const CALENDAR_MATCH_WINDOW_MS: i64 = 12 * 60 * 60 * 1000;

impl crate::Store {
    /// Whether a Google Calendar event whose title is, or is inside, `title` starts within
    /// twelve hours of `start_unix_ms`. Only the lookup the meeting's source is stamped from
    /// (see [`Store::meeting_may_hold_google_calendar_data`]): a later rename or removal of
    /// the event changes this answer, never the stamp.
    pub fn google_calendar_event_matches(&self, title: &str, start_unix_ms: i64) -> rusqlite::Result<bool> {
        self.calendar_event_matches(title, start_unix_ms, true)
    }

    fn calendar_event_matches(&self, title: &str, start_unix_ms: i64, google: bool) -> rusqlite::Result<bool> {
        // `instr` rather than `=`: a room-audio meeting's title wraps the event title. Only
        // ever used to decide toward Google.
        self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM items WHERE kind = 'calendar_event' \
             AND (COALESCE(source, '') = 'google_calendar') = ?4 \
             AND title <> '' AND instr(?1, title) > 0 AND ts_unix_ms BETWEEN ?2 AND ?3)",
            rusqlite::params![title, start_unix_ms - CALENDAR_MATCH_WINDOW_MS, start_unix_ms + CALENDAR_MATCH_WINDOW_MS, google],
            |r| r.get(0),
        )
    }

    /// Whether the meeting can be placed with another calendar for certain: exactly one
    /// non-Google event has exactly this title (case and surrounding space aside) in the
    /// window, and no Google event of any title is in the window.
    fn exactly_one_other_calendar_event(&self, title: &str, start_unix_ms: i64) -> rusqlite::Result<bool> {
        let (lo, hi) = (start_unix_ms - CALENDAR_MATCH_WINDOW_MS, start_unix_ms + CALENDAR_MATCH_WINDOW_MS);
        let google_in_window: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM items WHERE kind = 'calendar_event' AND source = 'google_calendar' \
             AND ts_unix_ms BETWEEN ?1 AND ?2)",
            rusqlite::params![lo, hi],
            |r| r.get(0),
        )?;
        if google_in_window {
            return Ok(false);
        }
        let exact: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE kind = 'calendar_event' AND COALESCE(source, '') <> 'google_calendar' \
             AND lower(trim(title)) = lower(trim(?1)) AND trim(title) <> '' AND ts_unix_ms BETWEEN ?2 AND ?3",
            rusqlite::params![title, lo, hi],
            |r| r.get(0),
        )?;
        Ok(exact == 1)
    }

    /// Records, when a calendar-triggered meeting is created, which calendar its title and
    /// attendees came from: a matching Google Calendar event (the title inside the
    /// meeting's) means Google; another calendar is recorded only when it is certain (see
    /// `exactly_one_other_calendar_event`); anything else stays unrecorded (unknown) and the
    /// meeting is treated as Google. Best effort: an error leaves it unknown.
    pub(crate) fn stamp_meeting_calendar_source(&self, id: i64, m: &crate::NewMeeting) {
        if !matches!(m.trigger, crate::MeetingTrigger::Calendar | crate::MeetingTrigger::Away) {
            return;
        }
        let source = match self.google_calendar_event_matches(&m.title, m.start_unix_ms) {
            Ok(true) => CALENDAR_SOURCE_GOOGLE,
            Ok(false) => match self.exactly_one_other_calendar_event(&m.title, m.start_unix_ms) {
                Ok(true) => STORED_OTHER_INFERRED,
                _ => return,
            },
            Err(_) => return,
        };
        let _ = self.conn.execute("UPDATE meeting SET calendar_source = ?1 WHERE id = ?2", rusqlite::params![source, id]);
    }

    /// Records whether an answer Whimbrel wrote used Google user data (Gmail or Calendar
    /// text in its prompt, or an earlier answer that did). The answer text comes back later
    /// as a turn of the conversation history, which is how its provenance is found again;
    /// an answer recorded as Google once stays Google.
    pub fn record_answer_provenance(&self, text: &str, google: bool, now_unix_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO answer_provenance (text_hash, google, recorded_unix_ms) VALUES (?1, ?2, ?3) \
             ON CONFLICT(text_hash) DO UPDATE SET google = MAX(google, excluded.google), recorded_unix_ms = excluded.recorded_unix_ms",
            rusqlite::params![answer_key(text), google, now_unix_ms],
        )?;
        Ok(())
    }

    /// Whether an answer may hold Google user data: recorded as such, or not recorded at all
    /// (an answer from before this was tracked, written on another device, or edited since),
    /// or an error. Only an answer recorded as clean is not.
    pub fn answer_may_hold_google_data(&self, text: &str) -> bool {
        self.conn
            .query_row("SELECT google FROM answer_provenance WHERE text_hash = ?1", [answer_key(text)], |r| r.get::<_, bool>(0))
            .unwrap_or(true)
    }

    /// States which calendar a meeting came from: [`CALENDAR_SOURCE_GOOGLE`] or
    /// [`CALENDAR_SOURCE_OTHER`], for a shell that knows the event's account. Device-local,
    /// not synced. `false` for an unknown meeting or an unknown source name.
    pub fn set_meeting_calendar_source(&self, id: i64, source: &str) -> rusqlite::Result<bool> {
        if source != CALENDAR_SOURCE_GOOGLE && source != CALENDAR_SOURCE_OTHER {
            return Ok(false);
        }
        let stored = if source == CALENDAR_SOURCE_OTHER { STORED_OTHER_CONFIRMED } else { source };
        Ok(self.conn.execute("UPDATE meeting SET calendar_source = ?1 WHERE id = ?2", rusqlite::params![stored, id])? > 0)
    }

    /// Whether the meeting's title and attendees may be Google Calendar text. Reads the
    /// source recorded at creation, never a live title lookup, so renaming or removing the
    /// event does not release it. A calendar-triggered meeting with no recorded source
    /// (from before this was recorded, or synced from another device), a missing meeting
    /// and an error all count as Google. A meeting that did not start from a calendar
    /// event does not.
    pub fn meeting_may_hold_google_calendar_data(&self, id: i64) -> bool {
        let row = self.conn.query_row(
            "SELECT trigger, calendar_source, start_unix_ms FROM meeting WHERE id = ?1",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)?)),
        );
        match row {
            Ok((_, Some(source), start)) if source == STORED_OTHER_INFERRED => {
                // Inferred when the meeting was created; Google's sync may have brought
                // the event in since. Once Google, always Google.
                let google_now: bool = self
                    .conn
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM items WHERE kind = 'calendar_event' AND source = 'google_calendar' \
                         AND ts_unix_ms BETWEEN ?1 AND ?2)",
                        rusqlite::params![start - CALENDAR_MATCH_WINDOW_MS, start + CALENDAR_MATCH_WINDOW_MS],
                        |r| r.get(0),
                    )
                    .unwrap_or(true);
                if google_now {
                    let _ = self.conn.execute(
                        "UPDATE meeting SET calendar_source = ?1 WHERE id = ?2 AND calendar_source = ?3",
                        rusqlite::params![CALENDAR_SOURCE_GOOGLE, id, STORED_OTHER_INFERRED],
                    );
                }
                google_now
            }
            Ok((_, Some(source), _)) => source != STORED_OTHER_CONFIRMED,
            Ok((trigger, None, _)) => trigger == crate::MeetingTrigger::Calendar.as_str() || trigger == crate::MeetingTrigger::Away.as_str(),
            Err(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_item_kinds_and_sources() {
        assert!(is_google_user_data("email", Some("gmail")));
        assert!(is_google_user_data("email", None));
        assert!(is_google_user_data("thread", Some("gmail")));
        assert!(is_google_user_data("calendar_event", Some("google_calendar")));

        assert!(!is_google_user_data("calendar_event", Some("apple_calendar")));
        assert!(!is_google_user_data("calendar_event", None));
        assert!(!is_google_user_data("email", Some("outlook")));
        assert!(is_google_user_data("routine_run", None));
        assert!(is_google_user_data("routine_run", Some("anything")));
        for kind in ["capture", "transcript", "snapshot", "episode", "meeting"] {
            assert!(!is_google_user_data(kind, Some("gmail")), "{kind}");
            assert!(!is_google_user_data(kind, None), "{kind}");
        }
    }

    #[test]
    fn the_sql_rule_matches_the_helper() {
        let s = crate::Store::open_in_memory().unwrap();
        let rows: [(&str, Option<&str>); 6] = [
            ("email", Some("gmail")),
            ("email", None),
            ("calendar_event", Some("google_calendar")),
            ("calendar_event", Some("apple_calendar")),
            ("capture", None),
            ("snapshot", None),
        ];
        for (i, (kind, source)) in rows.iter().enumerate() {
            s.conn
                .execute(
                    "INSERT INTO items (device_id, kind, source, ts_unix_ms, body_text, external_id) VALUES ('d', ?1, ?2, 1, '', ?3)",
                    rusqlite::params![kind, source, format!("x{i}")],
                )
                .unwrap();
            let item = s.conn.last_insert_rowid();
            s.conn
                .execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, 0, '', 0)",
                    rusqlite::params![kind, item],
                )
                .unwrap();
            let sid = s.conn.last_insert_rowid();
            let sql_says: bool = s
                .conn
                .query_row(
                    &format!("SELECT {TOPIC_SOURCE_IS_GOOGLE_SQL} FROM topic_source s WHERE s.id = ?1"),
                    [sid],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sql_says, is_google_user_data(kind, *source), "{kind} {source:?}");
        }
    }

    fn new_meeting(title: &str, trigger: crate::MeetingTrigger) -> crate::NewMeeting {
        crate::NewMeeting {
            device_id: "d".into(),
            start_unix_ms: 1,
            end_unix_ms: None,
            title: title.into(),
            attendees: vec![],
            trigger,
            audio_path: None,
            status: crate::MeetingStatus::Completed,
        }
    }

    fn event(s: &crate::Store, source: &str, title: &str) {
        s.upsert_calendar_event_items(&[crate::NewCalendarEventItem {
            device_id: "d".into(),
            source: source.into(),
            external_id: "evt#1".into(),
            ts_unix_ms: 1,
            title: title.into(),
            actors: vec![],
        }])
        .unwrap();
    }

    #[test]
    fn a_calendar_meetings_source_is_recorded_when_it_is_created() {
        use crate::MeetingTrigger::*;
        let s = crate::Store::open_in_memory().unwrap();
        event(&s, "google_calendar", "Board sync");
        let google = s.insert_meeting(&new_meeting("Board sync", Calendar)).unwrap();
        assert!(s.meeting_may_hold_google_calendar_data(google));
        // The event is renamed and then removed: the recorded source still holds.
        event(&s, "google_calendar", "Something else");
        s.delete_calendar_event_items(&["evt".to_string()]).unwrap();
        assert!(s.meeting_may_hold_google_calendar_data(google));

        let s = crate::Store::open_in_memory().unwrap();
        event(&s, "apple_calendar", "Board sync");
        let other = s.insert_meeting(&new_meeting("Board sync", Calendar)).unwrap();
        assert!(!s.meeting_may_hold_google_calendar_data(other));
    }

    #[test]
    fn an_unrecorded_calendar_source_counts_as_google() {
        use crate::MeetingTrigger::*;
        let s = crate::Store::open_in_memory().unwrap();
        let calendar = s.insert_meeting(&new_meeting("No event matches", Calendar)).unwrap();
        let away = s.insert_meeting(&new_meeting("No event matches", Away)).unwrap();
        let manual = s.insert_meeting(&new_meeting("Typed by hand", Manual)).unwrap();
        let detected = s.insert_meeting(&new_meeting("zoom.us", Detected)).unwrap();
        assert!(s.meeting_may_hold_google_calendar_data(calendar));
        assert!(s.meeting_may_hold_google_calendar_data(away));
        assert!(!s.meeting_may_hold_google_calendar_data(manual));
        assert!(!s.meeting_may_hold_google_calendar_data(detected));
        assert!(s.meeting_may_hold_google_calendar_data(9_999), "a missing meeting is unknown");

        // A shell that knows the account states it.
        assert!(s.set_meeting_calendar_source(calendar, CALENDAR_SOURCE_OTHER).unwrap());
        assert!(!s.meeting_may_hold_google_calendar_data(calendar));
        assert!(s.set_meeting_calendar_source(calendar, CALENDAR_SOURCE_GOOGLE).unwrap());
        assert!(s.meeting_may_hold_google_calendar_data(calendar));
        assert!(!s.set_meeting_calendar_source(calendar, "nonsense").unwrap());
        assert!(!s.set_meeting_calendar_source(9_999, CALENDAR_SOURCE_OTHER).unwrap());
    }

    #[test]
    fn an_answers_provenance_is_google_unless_recorded_clean() {
        let s = crate::Store::open_in_memory().unwrap();
        assert!(s.answer_may_hold_google_data("never recorded"));
        s.record_answer_provenance("clean answer", false, 1).unwrap();
        s.record_answer_provenance("gmail answer", true, 1).unwrap();
        assert!(!s.answer_may_hold_google_data("clean answer"));
        assert!(!s.answer_may_hold_google_data("  clean answer\n"), "a shell may trim it");
        assert!(s.answer_may_hold_google_data("gmail answer"));
        // The same words recorded clean later do not release it.
        s.record_answer_provenance("gmail answer", false, 2).unwrap();
        assert!(s.answer_may_hold_google_data("gmail answer"));
    }

    fn event_at(s: &crate::Store, source: &str, external_id: &str, title: &str, ts: i64) {
        s.upsert_calendar_event_items(&[crate::NewCalendarEventItem {
            device_id: "d".into(),
            source: source.into(),
            external_id: external_id.into(),
            ts_unix_ms: ts,
            title: title.into(),
            actors: vec![],
        }])
        .unwrap();
    }

    /// Fails closed: another calendar's source is stamped only for one exact title match
    /// with no Google event of any title in the window.
    #[test]
    fn other_is_stamped_only_for_one_exact_match_and_no_google_event_in_the_window() {
        use crate::MeetingTrigger::Calendar;
        let may_hold = |setup: &dyn Fn(&crate::Store), title: &str| {
            let s = crate::Store::open_in_memory().unwrap();
            setup(&s);
            let id = s.insert_meeting(&new_meeting(title, Calendar)).unwrap();
            s.meeting_may_hold_google_calendar_data(id)
        };
        // Substring-only match with a non-Google event ("Sync" inside a longer title).
        assert!(may_hold(&|s| event_at(s, "apple_calendar", "a#1", "Sync", 1), "Board Sync with Priya"));
        // Two non-Google candidates with the title.
        assert!(may_hold(
            &|s| {
                event_at(s, "apple_calendar", "a#1", "Sync", 1);
                event_at(s, "apple_calendar", "b#1", "Sync", 2);
            },
            "Sync"
        ));
        // An exact non-Google match, but a Google event (any title) sits in the window.
        assert!(may_hold(
            &|s| {
                event_at(s, "apple_calendar", "a#1", "Sync", 1);
                event_at(s, "google_calendar", "g#1", "Something else", 5);
            },
            "Sync"
        ));
        // The one exact match (case and surrounding space ignored) stamps other.
        assert!(!may_hold(&|s| event_at(s, "apple_calendar", "a#1", "Sync", 1), "  sync "));
        // A Google event far outside the window does not count.
        assert!(!may_hold(
            &|s| {
                event_at(s, "apple_calendar", "a#1", "Sync", 1);
                event_at(s, "google_calendar", "g#1", "Else", 1 + 13 * 60 * 60 * 1000);
            },
            "Sync"
        ));
    }

    /// OTHER is stamped when the meeting is created, before Google may have synced its
    /// event: a Google event that shows up in the window afterwards makes it Google, for good.
    /// Only the shell's explicit statement is final.
    #[test]
    fn a_stamped_other_becomes_google_when_a_google_event_appears_in_the_window() {
        use crate::MeetingTrigger::Calendar;
        let s = crate::Store::open_in_memory().unwrap();
        event_at(&s, "apple_calendar", "a#1", "Sync", 1);
        let id = s.insert_meeting(&new_meeting("Sync", Calendar)).unwrap();
        assert!(!s.meeting_may_hold_google_calendar_data(id));
        event_at(&s, "google_calendar", "g#1", "Sync (Google)", 5);
        assert!(s.meeting_may_hold_google_calendar_data(id));
        // Stays Google when that event is removed: never downgraded automatically.
        s.delete_calendar_event_items(&["g".to_string()]).unwrap();
        assert!(s.meeting_may_hold_google_calendar_data(id));

        // The shell's explicit statement is not re-checked.
        let s = crate::Store::open_in_memory().unwrap();
        let id = s.insert_meeting(&new_meeting("Sync", Calendar)).unwrap();
        assert!(s.set_meeting_calendar_source(id, CALENDAR_SOURCE_OTHER).unwrap());
        event_at(&s, "google_calendar", "g#1", "Else", 5);
        assert!(!s.meeting_may_hold_google_calendar_data(id));
    }

    /// A `meeting` topic source counts as Google exactly when the meeting may hold Google
    /// Calendar text, so topic text built from it stays off restricted links.
    #[test]
    fn the_sql_rule_for_meeting_sources_matches_the_meeting_helper() {
        use crate::MeetingTrigger::*;
        let s = crate::Store::open_in_memory().unwrap();
        event_at(&s, "apple_calendar", "a#1", "Inferred other", 1);
        let unknown = s.insert_meeting(&new_meeting("No event", Calendar)).unwrap();
        let inferred_other = s.insert_meeting(&new_meeting("Inferred other", Calendar)).unwrap();
        let confirmed_other = s.insert_meeting(&new_meeting("No event", Calendar)).unwrap();
        s.set_meeting_calendar_source(confirmed_other, CALENDAR_SOURCE_OTHER).unwrap();
        let google = s.insert_meeting(&new_meeting("No event", Calendar)).unwrap();
        s.set_meeting_calendar_source(google, CALENDAR_SOURCE_GOOGLE).unwrap();
        let manual = s.insert_meeting(&new_meeting("Typed", Manual)).unwrap();
        // A Google event syncs in after the first stamp.
        let ids = |s: &crate::Store| [unknown, inferred_other, confirmed_other, google, manual].map(|id| (id, s.meeting_may_hold_google_calendar_data(id)));
        let check = |s: &crate::Store| {
            for (id, expected) in ids(s) {
                s.conn
                    .execute(
                        "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('meeting', ?1, 0, '', 0)",
                        [id],
                    )
                    .unwrap();
                let sid = s.conn.last_insert_rowid();
                let sql_says: bool = s
                    .conn
                    .query_row(&format!("SELECT {TOPIC_SOURCE_IS_GOOGLE_SQL} FROM topic_source s WHERE s.id = ?1"), [sid], |r| r.get(0))
                    .unwrap();
                assert_eq!(sql_says, expected, "meeting {id}");
            }
        };
        s.conn.execute("DELETE FROM topic_source", []).unwrap();
        check(&s);
        assert!(!s.meeting_may_hold_google_calendar_data(inferred_other));
        event_at(&s, "google_calendar", "g#1", "Late", 5);
        s.conn.execute("DELETE FROM topic_source", []).unwrap();
        check(&s);
        assert!(s.meeting_may_hold_google_calendar_data(inferred_other));
    }
}
