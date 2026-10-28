//! Meeting Prep: matching an attendee to people in memory, the bounded redacted context a
//! brief is built from, and the stored briefs. See
//! docs/superpowers/specs/2026-10-06-meeting-prep-design.md.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};

use crate::redact::redact_secrets;
use crate::{MeetingStatus, Store};

pub const MEETINGS_PER_ATTENDEE: usize = 3;
pub const EMAILS_PER_ATTENDEE: usize = 5;
pub const EXCERPT_CHARS: usize = 300;
pub const MEETING_TEXT_CHARS: usize = 800;

/// Lowercased match keys for one person string: the display name and the address, from
/// `Name <addr>`, `mailto:addr`, a bare address or a bare name.
pub fn person_keys(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    let trimmed = if trimmed.to_lowercase().starts_with("mailto:") { &trimmed[7..] } else { trimmed };
    let (name, addr) = match (trimmed.rfind('<'), trimmed.rfind('>')) {
        (Some(l), Some(r)) if l < r => (trimmed[..l].trim().trim_matches('"').trim(), trimmed[l + 1..r].trim()),
        _ if trimmed.contains('@') => ("", trimmed),
        _ => (trimmed, ""),
    };
    [name, addr].iter().map(|s| s.to_lowercase()).filter(|s| !s.is_empty()).collect()
}

/// Whether two person strings name the same person (they share a name or an address).
pub fn same_person(a: &str, b: &str) -> bool {
    let keys_a = person_keys(a);
    person_keys(b).iter().any(|key| keys_a.contains(key))
}

/// Redacts first and cuts afterwards, so a secret that straddles the cut cannot leak.
pub fn redact_cut(text: &str, max_chars: usize) -> String {
    redact_secrets(text).chars().take(max_chars).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrepSourceKind {
    Meeting,
    Email,
}

/// One piece of evidence a brief may cite. `text` is already redacted and bounded.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PrepSource {
    /// `M1`, `M2`, ... for meetings; `E1`, `E2`, ... for email threads.
    pub id: String,
    pub kind: PrepSourceKind,
    pub title: String,
    pub ts_unix_ms: i64,
    /// The attendee this source was found for.
    pub attendee: String,
    pub text: String,
    /// The earlier meeting's id, for a chip that opens it.
    pub meeting_id: Option<i64>,
    /// The thread's internal id, for a chip that opens it.
    pub thread_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LastInteraction {
    pub attendee: String,
    pub ts_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PrepContext {
    pub sources: Vec<PrepSource>,
    pub last_interaction: Vec<LastInteraction>,
}

/// A stored brief, keyed by the calendar event id (upcoming meetings are not `meeting` rows).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeetingPrepRow {
    pub event_id: String,
    pub title: String,
    pub start_unix_ms: i64,
    pub generated_at_unix_ms: i64,
    /// The serialized `llm::meeting_prep::Brief`.
    pub brief_json: String,
    pub provider: String,
    pub model: String,
}

pub(crate) fn ensure_meeting_prep_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meeting_prep (
            event_id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            start_unix_ms INTEGER NOT NULL,
            generated_at_unix_ms INTEGER NOT NULL,
            brief_json TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL
        );",
    )
}

fn upsert_meeting_prep(conn: &Connection, row: &MeetingPrepRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meeting_prep (event_id, title, start_unix_ms, generated_at_unix_ms, brief_json, provider, model) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(event_id) DO UPDATE SET title = excluded.title, start_unix_ms = excluded.start_unix_ms, \
            generated_at_unix_ms = excluded.generated_at_unix_ms, brief_json = excluded.brief_json, \
            provider = excluded.provider, model = excluded.model",
        params![row.event_id, row.title, row.start_unix_ms, row.generated_at_unix_ms, row.brief_json, row.provider, row.model],
    )?;
    Ok(())
}

impl Store {
    pub fn save_meeting_prep(&self, row: &MeetingPrepRow) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        upsert_meeting_prep(&tx, row)?;
        self.enqueue_meeting_prep_outbox(&tx, row)?;
        tx.commit()
    }

    /// Applies a brief pulled from another device: only when newer than what is stored, bounded
    /// like a synced routine run, and never enqueued. Returns whether it was applied.
    pub fn apply_synced_meeting_prep(&self, row: &MeetingPrepRow) -> rusqlite::Result<bool> {
        if row.brief_json.len() > crate::SYNCED_RUN_MAX_JSON_BYTES || row.event_id.is_empty() {
            return Ok(false);
        }
        let row = MeetingPrepRow {
            generated_at_unix_ms: row.generated_at_unix_ms.min(crate::now_unix_ms() + crate::SYNCED_RUN_MAX_FUTURE_MS),
            title: row.title.chars().take(200).collect(),
            brief_json: self.keep_only_local_meeting_ids(&row.brief_json),
            ..row.clone()
        };
        if let Some(existing) = self.get_meeting_prep(&row.event_id)? {
            if existing.generated_at_unix_ms >= row.generated_at_unix_ms {
                return Ok(false);
            }
        }
        upsert_meeting_prep(&self.conn, &row)?;
        Ok(true)
    }

    /// A source's `meeting_id` is the row id on the device that made the brief, and meeting ids are
    /// not shared between devices, so on a brief copied from a peer it can name an unrelated meeting
    /// here. Keep it only when the local meeting at that id has the source's start and title (case and
    /// padding ignored); otherwise clear it so the chip is a plain label. Unparseable JSON is returned as is.
    fn keep_only_local_meeting_ids(&self, brief_json: &str) -> String {
        let Ok(mut v) = serde_json::from_str::<serde_json::Value>(brief_json) else { return brief_json.to_string() };
        let Some(sources) = v.get_mut("sources").and_then(|s| s.as_array_mut()) else { return brief_json.to_string() };
        for src in sources {
            let Some(id) = src.get("meeting_id").and_then(|m| m.as_i64()) else { continue };
            let title = src.get("title").and_then(|t| t.as_str()).unwrap_or("").trim().to_lowercase();
            let start = src.get("ts_unix_ms").and_then(|t| t.as_i64());
            let same = matches!(self.get_meeting(id), Ok(Some(m)) if Some(m.start_unix_ms) == start && m.title.trim().to_lowercase() == title);
            if !same {
                src["meeting_id"] = serde_json::Value::Null;
            }
        }
        v.to_string()
    }

    /// Calendar ids differ between devices (EventKit's are local; Google's are not), so a brief a
    /// peer wrote for the same meeting (same start, same title ignoring case and padding) is copied
    /// under this device's own id, where Today and the prepared check look. Never enqueued. Returns
    /// whether a copy was made.
    pub fn alias_synced_meeting_prep(&self, event_id: &str, start_unix_ms: i64, title: &str) -> rusqlite::Result<bool> {
        if self.get_meeting_prep(event_id)?.is_some() {
            return Ok(false);
        }
        let key = title.trim().to_lowercase();
        let mut stmt = self.conn.prepare(
            "SELECT event_id, title, start_unix_ms, generated_at_unix_ms, brief_json, provider, model \
             FROM meeting_prep WHERE start_unix_ms = ?1 ORDER BY generated_at_unix_ms DESC",
        )?;
        let rows = stmt.query_map(params![start_unix_ms], |r| {
            Ok(MeetingPrepRow {
                event_id: r.get(0)?, title: r.get(1)?, start_unix_ms: r.get(2)?,
                generated_at_unix_ms: r.get(3)?, brief_json: r.get(4)?, provider: r.get(5)?, model: r.get(6)?,
            })
        })?;
        for row in rows {
            let row = row?;
            if row.title.trim().to_lowercase() == key {
                let brief_json = self.keep_only_local_meeting_ids(&row.brief_json);
                upsert_meeting_prep(&self.conn, &MeetingPrepRow { event_id: event_id.to_string(), brief_json, ..row })?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn get_meeting_prep(&self, event_id: &str) -> rusqlite::Result<Option<MeetingPrepRow>> {
        self.conn
            .query_row(
                "SELECT event_id, title, start_unix_ms, generated_at_unix_ms, brief_json, provider, model FROM meeting_prep WHERE event_id = ?1",
                params![event_id],
                |r| {
                    Ok(MeetingPrepRow {
                        event_id: r.get(0)?,
                        title: r.get(1)?,
                        start_unix_ms: r.get(2)?,
                        generated_at_unix_ms: r.get(3)?,
                        brief_json: r.get(4)?,
                        provider: r.get(5)?,
                        model: r.get(6)?,
                    })
                },
            )
            .optional()
    }

    /// Every event id that already has a brief, so a meeting is prepared once.
    pub fn prepared_event_ids(&self) -> rusqlite::Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT event_id FROM meeting_prep")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    /// Drops briefs for meetings that started before `older_than_start_unix_ms`.
    pub fn prune_meeting_prep(&self, older_than_start_unix_ms: i64) -> rusqlite::Result<usize> {
        self.conn.execute("DELETE FROM meeting_prep WHERE start_unix_ms < ?1", params![older_than_start_unix_ms])
    }
}

/// An email item reduced to what prep needs.
struct EmailRow {
    ts_unix_ms: i64,
    actors: Vec<String>,
    subject: String,
    body: String,
    gmail_thread_id: String,
}

/// How far back each source is searched. Newest first, so a long history only costs a
/// bounded scan.
const MEETING_SCAN: usize = 300;
const EMAIL_SCAN: usize = 500;

fn meeting_text(m: &crate::Meeting) -> String {
    let mut parts = vec![m.title.clone()];
    if let Some(tldr) = &m.summary_tldr {
        parts.push(tldr.clone());
    }
    for (label, items) in [
        ("Decisions", &m.summary_decisions),
        ("Action items", &m.summary_action_items),
        ("Open questions", &m.summary_open_questions),
    ] {
        if !items.is_empty() {
            parts.push(format!("{label}: {}", items.join("; ")));
        }
    }
    redact_cut(&parts.join(" | "), MEETING_TEXT_CHARS)
}

impl Store {
    fn recent_email_items(&self, limit: usize) -> rusqlite::Result<Vec<EmailRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts_unix_ms, actors, COALESCE(title, ''), body_text, COALESCE(json_extract(refs, '$.gmail_thread_id'), '') \
             FROM items WHERE kind = 'email' AND source = 'gmail' ORDER BY ts_unix_ms DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let actors_json: String = r.get::<_, Option<String>>(1)?.unwrap_or_default();
            Ok(EmailRow {
                ts_unix_ms: r.get(0)?,
                actors: serde_json::from_str(&actors_json).unwrap_or_default(),
                subject: r.get(2)?,
                body: r.get(3)?,
                gmail_thread_id: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// The thread's internal id, for a chip that opens it.
    fn internal_thread_id(&self, gmail_thread_id: &str) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT id FROM threads WHERE source = 'gmail' AND external_thread_id = ?1",
                params![gmail_thread_id],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// The bounded, redacted evidence for a brief: per attendee, their last 3 completed
    /// meetings (title, summary, decisions, action items, open questions) and last 5
    /// email threads (subject and a 300-character excerpt). A meeting or thread shared by
    /// several attendees is one source, tagged with the first attendee it was found for.
    pub fn gather_prep_context(&self, attendees: &[String]) -> rusqlite::Result<PrepContext> {
        let meetings = self.recent_meetings(MEETING_SCAN)?;
        let emails = self.recent_email_items(EMAIL_SCAN)?;
        let mut sources = Vec::new();
        let mut last_interaction = Vec::new();
        let (mut seen_meetings, mut seen_threads) = (HashSet::new(), HashSet::new());
        let (mut meeting_n, mut email_n) = (0, 0);

        for attendee in attendees {
            let mut latest: Option<i64> = None;
            let mut taken = 0;
            for m in meetings
                .iter()
                .filter(|m| m.status == MeetingStatus::Completed && m.attendees.iter().any(|a| same_person(a, attendee)))
            {
                latest = latest.max(Some(m.start_unix_ms));
                if taken < MEETINGS_PER_ATTENDEE && seen_meetings.insert(m.id) {
                    taken += 1;
                    meeting_n += 1;
                    sources.push(PrepSource {
                        id: format!("M{meeting_n}"),
                        kind: PrepSourceKind::Meeting,
                        title: m.title.clone(),
                        ts_unix_ms: m.start_unix_ms,
                        attendee: attendee.clone(),
                        text: meeting_text(m),
                        meeting_id: Some(m.id),
                        thread_id: None,
                    });
                }
            }
            let mut taken = 0;
            for e in emails.iter().filter(|e| e.actors.iter().any(|a| same_person(a, attendee))) {
                latest = latest.max(Some(e.ts_unix_ms));
                if taken < EMAILS_PER_ATTENDEE && seen_threads.insert(e.gmail_thread_id.clone()) {
                    taken += 1;
                    email_n += 1;
                    sources.push(PrepSource {
                        id: format!("E{email_n}"),
                        kind: PrepSourceKind::Email,
                        title: redact_cut(&e.subject, 120),
                        ts_unix_ms: e.ts_unix_ms,
                        attendee: attendee.clone(),
                        text: redact_cut(&e.body, EXCERPT_CHARS),
                        meeting_id: None,
                        thread_id: self.internal_thread_id(&e.gmail_thread_id),
                    });
                }
            }
            last_interaction.push(LastInteraction { attendee: attendee.clone(), ts_unix_ms: latest });
        }
        Ok(PrepContext { sources, last_interaction })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_cover_the_name_and_the_address() {
        assert_eq!(person_keys("Ben Lawrick <Ben@AlphaSights.com>"), ["ben lawrick", "ben@alphasights.com"]);
        assert_eq!(person_keys("\"Lawrick, Ben\" <ben@alphasights.com>"), ["lawrick, ben", "ben@alphasights.com"]);
        assert_eq!(person_keys("mailto:Ben@AlphaSights.com"), ["ben@alphasights.com"]);
        assert_eq!(person_keys("ben@alphasights.com"), ["ben@alphasights.com"]);
        assert_eq!(person_keys("  Ben Lawrick "), ["ben lawrick"]);
        assert!(person_keys("   ").is_empty());
    }

    #[test]
    fn people_match_by_name_or_by_address_across_formats() {
        assert!(same_person("Ben Lawrick", "ben lawrick <ben@alphasights.com>"), "shared name");
        assert!(same_person("mailto:ben@alphasights.com", "Ben L. <BEN@alphasights.com>"), "shared address");
        assert!(!same_person("Ben Lawrick", "Hannah Larbi <hannah@x.org>"));
        assert!(!same_person("", ""), "empty never matches");
    }

    fn row(event_id: &str, start: i64) -> MeetingPrepRow {
        MeetingPrepRow {
            event_id: event_id.into(),
            title: "Acme renewal".into(),
            start_unix_ms: start,
            generated_at_unix_ms: start - 1_800_000,
            brief_json: "{\"x\":1}".into(),
            provider: "ollama".into(),
            model: "gemma4:12b".into(),
        }
    }

    #[test]
    fn briefs_round_trip_upsert_list_and_prune() {
        let store = crate::Store::open_in_memory().unwrap();
        assert!(store.get_meeting_prep("e1").unwrap().is_none());

        store.save_meeting_prep(&row("e1", 1_000_000)).unwrap();
        store.save_meeting_prep(&row("e2", 9_000_000)).unwrap();
        assert_eq!(store.get_meeting_prep("e1").unwrap(), Some(row("e1", 1_000_000)));
        assert_eq!(
            store.prepared_event_ids().unwrap(),
            ["e1", "e2"].iter().map(|s| s.to_string()).collect::<HashSet<String>>()
        );

        let mut changed = row("e1", 1_000_000);
        changed.brief_json = "{\"x\":2}".into();
        store.save_meeting_prep(&changed).unwrap();
        assert_eq!(store.get_meeting_prep("e1").unwrap().unwrap().brief_json, "{\"x\":2}", "upsert replaces");

        assert_eq!(store.prune_meeting_prep(5_000_000).unwrap(), 1, "only the older meeting goes");
        assert!(store.get_meeting_prep("e1").unwrap().is_none());
        assert!(store.get_meeting_prep("e2").unwrap().is_some());
    }

    use crate::{MeetingStatus, MeetingTrigger, NewEmailItem, NewMeeting};

    const DAY: i64 = 86_400_000;

    fn completed_meeting(store: &crate::Store, start: i64, title: &str, attendees: &[&str], tldr: &str) -> i64 {
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "dev-1".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 3_600_000),
                title: title.into(),
                attendees: attendees.iter().map(|s| s.to_string()).collect(),
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store
            .update_meeting_summary(id, tldr, &["Ship v2".to_string()], &["Ben sends the deck".to_string()], &["Pricing?".to_string()], "ollama", "m")
            .unwrap();
        id
    }

    fn email(store: &crate::Store, n: i64, thread: &str, from: &str, subject: &str, body: &str) {
        store
            .upsert_email_items(&[NewEmailItem {
                device_id: "dev-1".into(),
                external_id: format!("msg-{n}"),
                ts_unix_ms: n * DAY,
                from: from.into(),
                to: "me@example.com".into(),
                subject: subject.into(),
                body_text: body.into(),
                gmail_thread_id: thread.into(),
                account_email: "me@example.com".into(),
                ..Default::default()
            }])
            .unwrap();
    }

    #[test]
    fn earlier_meetings_and_email_with_an_attendee_become_cited_sources() {
        let store = crate::Store::open_in_memory().unwrap();
        let m = completed_meeting(&store, 10 * DAY, "Acme kickoff", &["Ben Lawrick"], "Kicked off the Acme project");
        completed_meeting(&store, 11 * DAY, "Unrelated", &["Hannah Larbi"], "Not about Ben");
        email(&store, 12, "t-1", "Ben Lawrick <ben@alphasights.com>", "Re: Acme pricing", "Here is the pricing deck");

        let context = store.gather_prep_context(&["Ben Lawrick".to_string()]).unwrap();

        assert_eq!(context.sources.len(), 2);
        let meeting = &context.sources[0];
        assert_eq!((meeting.id.as_str(), meeting.kind, meeting.meeting_id), ("M1", PrepSourceKind::Meeting, Some(m)));
        assert!(meeting.text.contains("Kicked off the Acme project") && meeting.text.contains("Ship v2") && meeting.text.contains("Pricing?"));
        let mail = &context.sources[1];
        assert_eq!((mail.id.as_str(), mail.kind), ("E1", PrepSourceKind::Email));
        assert!(mail.title.contains("Acme pricing") && mail.text.contains("pricing deck"));
        assert_eq!(context.last_interaction, vec![LastInteraction { attendee: "Ben Lawrick".into(), ts_unix_ms: Some(12 * DAY) }]);
    }

    #[test]
    fn limits_apply_per_attendee_newest_first() {
        let store = crate::Store::open_in_memory().unwrap();
        for n in 1..=5 {
            completed_meeting(&store, n * DAY, &format!("Sync {n}"), &["Ben Lawrick"], "s");
        }
        for n in 20..=28 {
            email(&store, n, &format!("t-{n}"), "ben@alphasights.com", &format!("Subject {n}"), "b");
        }

        let context = store.gather_prep_context(&["ben lawrick".to_string(), "ben@alphasights.com".to_string()]).unwrap();

        let meetings: Vec<&str> = context.sources.iter().filter(|s| s.kind == PrepSourceKind::Meeting).map(|s| s.title.as_str()).collect();
        assert_eq!(meetings, ["Sync 5", "Sync 4", "Sync 3"], "3 meetings, newest first");
        let emails = context.sources.iter().filter(|s| s.kind == PrepSourceKind::Email).count();
        assert_eq!(emails, EMAILS_PER_ATTENDEE, "5 threads for the attendee that matches by address");
        assert_eq!(context.last_interaction.len(), 2);
    }

    #[test]
    fn a_thread_with_many_messages_is_one_source_and_secrets_are_redacted_before_the_cut() {
        let store = crate::Store::open_in_memory().unwrap();
        let key = format!("apikey_{}", "a1b2_c3".repeat(10));
        email(&store, 30, "t-9", "ben@alphasights.com", "Keys", "first message, plain");
        // The newest message in the thread is the one that becomes the source.
        email(&store, 31, "t-9", "ben@alphasights.com", "Re: Keys", &format!("{} {key}", "y".repeat(290)));

        let context = store.gather_prep_context(&["ben@alphasights.com".to_string()]).unwrap();

        assert_eq!(context.sources.len(), 1, "one source per thread");
        let text = &context.sources[0].text;
        assert!(text.contains("[REDACTED"), "the key must have been redacted, not just cut off: {text:?}");
        assert!(!text.contains("apikey_"), "raw key leaked: {text:?}");
        assert!(text.chars().count() <= EXCERPT_CHARS);
    }

    #[test]
    fn nobody_known_means_no_sources() {
        let store = crate::Store::open_in_memory().unwrap();
        let context = store.gather_prep_context(&["Stranger".to_string()]).unwrap();
        assert!(context.sources.is_empty());
        assert_eq!(context.last_interaction, vec![LastInteraction { attendee: "Stranger".into(), ts_unix_ms: None }]);
    }

    #[test]
    fn a_secret_straddling_the_cut_is_redacted_before_it_is_cut() {
        let key = format!("apikey_{}", "a1b2_c3".repeat(10));
        let text = format!("{} {}", "x".repeat(290), key);
        let cut = redact_cut(&text, EXCERPT_CHARS);
        assert!(cut.chars().count() <= EXCERPT_CHARS);
        assert!(!cut.contains("apikey_"), "raw key leaked: {cut:?}");
        assert!(cut.contains("[REDACTED"), "{cut:?}");
    }

    fn brief(event_id: &str, generated_at: i64) -> MeetingPrepRow {
        MeetingPrepRow {
            event_id: event_id.into(), title: "Sync".into(), start_unix_ms: 2_000_000,
            generated_at_unix_ms: generated_at, brief_json: "{}".into(), provider: "p".into(), model: "m".into(),
        }
    }

    #[test]
    fn a_locally_saved_brief_enqueues_exactly_one_outbox_row() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        store.save_meeting_prep(&brief("ev1", 1_000)).unwrap();
        let ops = store.list_outbox_operations().unwrap();
        assert_eq!(ops.iter().filter(|o| o.1 == "meeting_prep").count(), 1);
    }

    #[test]
    fn a_synced_brief_applies_only_when_newer_and_never_enqueues() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        assert!(store.apply_synced_meeting_prep(&brief("ev1", 1_000)).unwrap());
        assert!(!store.apply_synced_meeting_prep(&brief("ev1", 500)).unwrap(), "older must not replace");
        assert!(store.apply_synced_meeting_prep(&brief("ev1", 2_000)).unwrap());
        assert_eq!(store.get_meeting_prep("ev1").unwrap().unwrap().generated_at_unix_ms, 2_000);
        assert!(store.list_outbox_operations().unwrap().is_empty(), "applied rows are not re-pushed");
    }

    #[test]
    fn a_synced_brief_with_an_oversized_payload_is_dropped() {
        let store = Store::open_in_memory().unwrap();
        let mut big = brief("ev1", 1_000);
        big.brief_json = "x".repeat(2_000_000);
        assert!(!store.apply_synced_meeting_prep(&big).unwrap());
        assert!(store.get_meeting_prep("ev1").unwrap().is_none());
    }

    #[test]
    fn a_peer_brief_for_the_same_meeting_under_another_id_is_aliased_to_the_local_id_without_enqueueing() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let mut peer = brief("peer-id", 1_000);
        peer.start_unix_ms = 5_000;
        peer.title = "  Acme Renewal ".into();
        store.apply_synced_meeting_prep(&peer).unwrap();
        assert!(store.alias_synced_meeting_prep("local-id", 5_000, "acme renewal").unwrap());
        assert!(store.get_meeting_prep("local-id").unwrap().is_some());
        assert!(!store.alias_synced_meeting_prep("local-id", 5_000, "acme renewal").unwrap(), "already there");
        assert!(!store.alias_synced_meeting_prep("other", 6_000, "acme renewal").unwrap(), "different start");
        assert!(store.list_outbox_operations().unwrap().is_empty(), "an alias is never pushed");
    }

    fn peer_brief_citing(meeting_id: i64, title: &str, ts: i64) -> MeetingPrepRow {
        let mut r = brief("peer-id", 1_000);
        r.start_unix_ms = 5_000;
        r.title = "Acme".into();
        r.brief_json = serde_json::json!({"sources": [
            {"id": "M1", "kind": "meeting", "title": title, "ts_unix_ms": ts, "meeting_id": meeting_id},
            {"id": "E1", "kind": "email", "title": "Re: quote", "ts_unix_ms": 1, "meeting_id": null},
        ]}).to_string();
        r
    }

    fn cited_meeting_ids(store: &Store, event_id: &str) -> Vec<Option<i64>> {
        let json = store.get_meeting_prep(event_id).unwrap().unwrap().brief_json;
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["sources"].as_array().unwrap().iter().map(|s| s["meeting_id"].as_i64()).collect()
    }

    fn put_meeting(store: &Store, title: &str, start: i64) -> i64 {
        store.insert_meeting(&crate::NewMeeting {
            device_id: "dev".into(), start_unix_ms: start, end_unix_ms: Some(start + 1_000), title: title.into(),
            attendees: vec![], trigger: crate::MeetingTrigger::Manual, audio_path: None, status: crate::MeetingStatus::Completed,
        }).unwrap()
    }

    #[test]
    fn a_synced_brief_keeps_a_cited_meeting_id_only_when_that_local_meeting_matches() {
        let store = Store::open_in_memory().unwrap();
        let id = put_meeting(&store, "Lunch", 7_000);
        // The peer's id happens to be a different meeting here: wrong title and start, so it is cleared.
        store.apply_synced_meeting_prep(&peer_brief_citing(id, "Acme kickoff", 3_000)).unwrap();
        assert_eq!(cited_meeting_ids(&store, "peer-id"), vec![None, None]);
        // Same title and start (case and padding aside): the id is real here and stays.
        let mut again = peer_brief_citing(id, " LUNCH ", 7_000);
        again.generated_at_unix_ms = 2_000;
        store.apply_synced_meeting_prep(&again).unwrap();
        assert_eq!(cited_meeting_ids(&store, "peer-id"), vec![Some(id), None]);
    }

    #[test]
    fn an_aliased_brief_clears_a_cited_meeting_id_that_names_another_meeting_here() {
        let store = Store::open_in_memory().unwrap();
        let id = put_meeting(&store, "Lunch", 7_000);
        upsert_meeting_prep(&store.conn, &peer_brief_citing(id, "Acme kickoff", 3_000)).unwrap();
        assert!(store.alias_synced_meeting_prep("local-id", 5_000, "acme").unwrap());
        assert_eq!(cited_meeting_ids(&store, "local-id"), vec![None, None]);
    }
}
