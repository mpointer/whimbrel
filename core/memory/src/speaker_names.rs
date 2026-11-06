//! Speaker names (docs/superpowers/specs/2026-11-05-speaker-names-design.md): the people
//! Whimbrel knows (`person`), each meeting's voices and who they were assigned to
//! (`meeting_speaker`), and resolving "Speaker N" labels to names when a transcript is read.
//! A person's voice print is never stored: it is the mean of their confirmed embeddings.
//! Rows sync last-write-wins on `updated_at_unix_ms`. A local write stamps past the row's last
//! stamp, so it beats what it replaces on every device, even within one ms or behind a peer's clock.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

pub use crate::speaker_match::NameStatus;
use crate::speaker_match::{cosine, STRONG_MATCH};
use crate::Store;

/// A calendar invitee. `Meeting.attendees` keeps the names alone for older readers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attendee {
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// Email is the identity when present; otherwise the name, case-folded.
pub fn person_key(a: &Attendee) -> String {
    match a.email.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        Some(email) => email.to_lowercase(),
        None => format!("name:{}", a.name.trim().to_lowercase()),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingSpeaker {
    pub speaker_id: i32,
    pub person_key: Option<String>,
    pub name: Option<String>,
    pub status: Option<NameStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoicePerson {
    pub person_key: String,
    pub name: String,
    pub confirmed_count: i64,
}

/// A `person` row on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedPerson {
    pub person_key: String,
    pub name: String,
    pub email: Option<String>,
    pub updated_at_unix_ms: i64,
}

/// A `meeting_speaker` row on the wire. `origin_device_id` is the device that recorded the
/// meeting (`None`: the sender did) and `source_meeting_id` is that device's meeting id, so an
/// edit made on any device reaches the same meeting everywhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedMeetingSpeaker {
    #[serde(default)]
    pub origin_device_id: Option<String>,
    pub source_meeting_id: i64,
    pub speaker_id: i32,
    pub embedding: Option<Vec<f32>>,
    pub person_key: Option<String>,
    pub status: Option<NameStatus>,
    pub updated_at_unix_ms: i64,
}

pub(crate) fn ensure_speaker_names_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS person (
            person_key TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT,
            updated_at_unix_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS meeting_speaker (
            meeting_id INTEGER NOT NULL,
            speaker_id INTEGER NOT NULL,
            embedding BLOB,
            person_key TEXT,
            status TEXT,
            updated_at_unix_ms INTEGER NOT NULL,
            PRIMARY KEY(meeting_id, speaker_id)
        );
        CREATE INDEX IF NOT EXISTS idx_meeting_speaker_person ON meeting_speaker(person_key);
        -- A peer's speaker rows whose meeting has not synced here yet, by the recording device.
        CREATE TABLE IF NOT EXISTS meeting_speaker_pending (
            origin_device_id TEXT NOT NULL,
            source_meeting_id INTEGER NOT NULL,
            speaker_id INTEGER NOT NULL,
            embedding BLOB,
            person_key TEXT,
            status TEXT,
            updated_at_unix_ms INTEGER NOT NULL,
            PRIMARY KEY(origin_device_id, source_meeting_id, speaker_id)
        );",
    )
}

/// "Speaker N" (labels are written as `Speaker {id + 1}`) becomes the name assigned to
/// speaker N-1; the bool is whether that name is a guess. Anything else stays as written.
pub fn display_label(raw: &str, speakers: &[MeetingSpeaker]) -> (String, bool) {
    match labelled_speaker(raw, speakers) {
        Some(MeetingSpeaker { name: Some(name), status, .. }) => (name.clone(), *status == Some(NameStatus::Guess)),
        _ => (raw.to_string(), false),
    }
}

/// The speaker a "Speaker N" label names, if any.
pub fn labelled_speaker<'a>(raw: &str, speakers: &'a [MeetingSpeaker]) -> Option<&'a MeetingSpeaker> {
    let n = raw.strip_prefix("Speaker ").and_then(|n| n.parse::<i32>().ok()).filter(|n| *n >= 1)?;
    speakers.iter().find(|s| s.speaker_id == n - 1)
}

/// A label for text a model or a reader takes as written: the name, a guess as "Name (unconfirmed)".
pub fn transcript_label(raw: &str, speakers: &[MeetingSpeaker]) -> String {
    match display_label(raw, speakers) {
        (name, true) => format!("{name} (unconfirmed)"),
        (name, false) => name,
    }
}

fn status_str(s: NameStatus) -> &'static str {
    match s {
        NameStatus::Confirmed => "confirmed",
        NameStatus::Guess => "guess",
    }
}

fn parse_status(s: Option<String>) -> Option<NameStatus> {
    match s.as_deref() {
        Some("confirmed") => Some(NameStatus::Confirmed),
        Some("guess") => Some(NameStatus::Guess),
        _ => None,
    }
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// A `meeting.attendee_details` column value; NULL (a row from before the column) is empty.
pub(crate) fn attendee_details_from(json: Option<String>) -> Vec<Attendee> {
    json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

fn row_to_applied(r: &Row<'_>) -> rusqlite::Result<AppliedMeetingSpeaker> {
    Ok(AppliedMeetingSpeaker {
        origin_device_id: None,
        source_meeting_id: r.get(0)?,
        speaker_id: r.get(1)?,
        embedding: r.get::<_, Option<Vec<u8>>>(2)?.map(|b| from_blob(&b)),
        person_key: r.get(3)?,
        status: parse_status(r.get(4)?),
        updated_at_unix_ms: r.get(5)?,
    })
}

/// Writes a synced speaker row unless this device's copy is as new or newer.
fn upsert_newer_speaker(
    conn: &Connection,
    meeting_id: i64,
    speaker_id: i32,
    embedding: Option<Vec<u8>>,
    person_key: Option<&str>,
    status: Option<&str>,
    updated_at_unix_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meeting_speaker (meeting_id, speaker_id, embedding, person_key, status, updated_at_unix_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(meeting_id, speaker_id) DO UPDATE SET \
         embedding = excluded.embedding, person_key = excluded.person_key, status = excluded.status, \
         updated_at_unix_ms = excluded.updated_at_unix_ms WHERE excluded.updated_at_unix_ms > meeting_speaker.updated_at_unix_ms",
        params![meeting_id, speaker_id, embedding, person_key, status, updated_at_unix_ms],
    )?;
    Ok(())
}

impl Store {
    /// Stores a meeting's invitees with their emails. `attendees` (names) is left as is.
    /// ponytail: not restamped for sync; a meeting already pushed carries them on its next push.
    pub fn set_meeting_attendee_details(&self, meeting_id: i64, a: &[Attendee]) -> rusqlite::Result<()> {
        let json = serde_json::to_string(a).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute("UPDATE meeting SET attendee_details = ?1 WHERE id = ?2", params![json, meeting_id])?;
        Ok(())
    }

    /// A meeting's invitees with their emails; a meeting without them gives its `attendees`
    /// names with no email. Empty for an unknown meeting.
    pub fn meeting_attendee_details(&self, meeting_id: i64) -> rusqlite::Result<Vec<Attendee>> {
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row("SELECT attendees, attendee_details FROM meeting WHERE id = ?1", params![meeting_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((names, details)) = row else { return Ok(Vec::new()) };
        let details = attendee_details_from(details);
        if !details.is_empty() {
            return Ok(details);
        }
        let names: Vec<String> = serde_json::from_str(&names).unwrap_or_default();
        Ok(names.into_iter().map(|name| Attendee { name, email: None }).collect())
    }

    /// The stored column alone (no fallback), for the meeting's sync payload.
    pub(crate) fn stored_attendee_details_tx(&self, tx: &rusqlite::Transaction<'_>, meeting_id: i64) -> rusqlite::Result<Vec<Attendee>> {
        let json: Option<Option<String>> = tx
            .query_row("SELECT attendee_details FROM meeting WHERE id = ?1", params![meeting_id], |r| r.get(0))
            .optional()?;
        Ok(attendee_details_from(json.flatten()))
    }

    pub fn upsert_person(&self, key: &str, name: &str, email: Option<&str>) -> rusqlite::Result<()> {
        let mut p = AppliedPerson {
            person_key: key.to_string(),
            name: name.to_string(),
            email: email.map(str::to_string),
            updated_at_unix_ms: crate::now_unix_ms(),
        };
        let tx = self.conn.unchecked_transaction()?;
        p.updated_at_unix_ms = tx.query_row(
            "INSERT INTO person (person_key, name, email, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(person_key) DO UPDATE SET name = excluded.name, email = excluded.email, \
             updated_at_unix_ms = MAX(excluded.updated_at_unix_ms, person.updated_at_unix_ms + 1) \
             RETURNING updated_at_unix_ms",
            params![p.person_key, p.name, p.email, p.updated_at_unix_ms],
            |r| r.get(0),
        )?;
        self.enqueue_person_outbox(&tx, &p)?;
        tx.commit()
    }

    /// Stores each speaker's embedding (NULL when `remember` is off), keeping any assignment,
    /// unless the stored embedding is another voice (transcribed again, the ids reshuffled):
    /// then that speaker's name no longer applies and is cleared. No stored embedding keeps it.
    pub fn save_meeting_voices(&self, meeting_id: i64, voices: &[(i32, Vec<f32>)], remember: bool) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (speaker_id, embedding) in voices {
            let old: Option<Option<Vec<u8>>> = tx
                .query_row(
                    "SELECT embedding FROM meeting_speaker WHERE meeting_id = ?1 AND speaker_id = ?2",
                    params![meeting_id, speaker_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(Some(old)) = old {
                if cosine(&from_blob(&old), embedding) < STRONG_MATCH {
                    tx.execute(
                        "UPDATE meeting_speaker SET person_key = NULL, status = NULL WHERE meeting_id = ?1 AND speaker_id = ?2",
                        params![meeting_id, speaker_id],
                    )?;
                }
            }
            let blob = remember.then(|| to_blob(embedding));
            tx.execute(
                "INSERT INTO meeting_speaker (meeting_id, speaker_id, embedding, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(meeting_id, speaker_id) DO UPDATE SET embedding = excluded.embedding, \
                 updated_at_unix_ms = MAX(excluded.updated_at_unix_ms, meeting_speaker.updated_at_unix_ms + 1)",
                params![meeting_id, speaker_id, blob, crate::now_unix_ms()],
            )?;
            self.enqueue_meeting_speaker_row(&tx, meeting_id, *speaker_id)?;
        }
        tx.commit()
    }

    /// Assigns (or, with `None`, clears) the person a meeting's speaker is, keeping its embedding.
    pub fn set_meeting_speaker(
        &self,
        meeting_id: i64,
        speaker_id: i32,
        person_key: Option<&str>,
        status: Option<NameStatus>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO meeting_speaker (meeting_id, speaker_id, person_key, status, updated_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(meeting_id, speaker_id) DO UPDATE SET \
             person_key = excluded.person_key, status = excluded.status, \
             updated_at_unix_ms = MAX(excluded.updated_at_unix_ms, meeting_speaker.updated_at_unix_ms + 1)",
            params![meeting_id, speaker_id, person_key, status.map(status_str), crate::now_unix_ms()],
        )?;
        self.enqueue_meeting_speaker_row(&tx, meeting_id, speaker_id)?;
        tx.commit()
    }

    /// Confirms a speaker's guess as the person it already is. Only when the speaker is still
    /// `person_key` (a page shown before a sync or a rerun could be stale); false otherwise.
    pub fn confirm_meeting_speaker(&self, meeting_id: i64, speaker_id: i32, person_key: &str) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE meeting_speaker SET status = 'confirmed', updated_at_unix_ms = MAX(?4, updated_at_unix_ms + 1) \
             WHERE meeting_id = ?1 AND speaker_id = ?2 AND person_key = ?3",
            params![meeting_id, speaker_id, person_key, crate::now_unix_ms()],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        self.enqueue_meeting_speaker_row(&tx, meeting_id, speaker_id)?;
        tx.commit()?;
        Ok(true)
    }

    fn enqueue_meeting_speaker_row(&self, tx: &rusqlite::Transaction<'_>, meeting_id: i64, speaker_id: i32) -> rusqlite::Result<()> {
        let mut row = tx.query_row(
            "SELECT meeting_id, speaker_id, embedding, person_key, status, updated_at_unix_ms \
             FROM meeting_speaker WHERE meeting_id = ?1 AND speaker_id = ?2",
            params![meeting_id, speaker_id],
            row_to_applied,
        )?;
        // A meeting synced from a peer is named by its recorder's id (`meeting.device_id`).
        let origin: Option<(String, i64)> = tx
            .query_row(
                "SELECT m.peer_device_id, m.source_meeting_id FROM sync_meeting_map m \
                 JOIN meeting ON meeting.id = m.local_meeting_id AND meeting.device_id = m.peer_device_id \
                 WHERE m.local_meeting_id = ?1",
                params![meeting_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((peer, source_meeting_id)) = origin {
            row.origin_device_id = Some(peer);
            row.source_meeting_id = source_meeting_id;
        }
        self.enqueue_meeting_speaker_outbox(tx, &row)
    }

    /// Applies a person pulled from another device; the later `updated_at_unix_ms` wins (a tie
    /// keeps this device's). Never enqueued.
    pub fn apply_synced_person(&self, _peer_device_id: &str, p: &AppliedPerson) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO person (person_key, name, email, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(person_key) DO UPDATE SET name = excluded.name, email = excluded.email, \
             updated_at_unix_ms = excluded.updated_at_unix_ms WHERE excluded.updated_at_unix_ms > person.updated_at_unix_ms",
            params![p.person_key, p.name, p.email, p.updated_at_unix_ms],
        )?;
        Ok(())
    }

    /// Applies a speaker row pulled from `peer_device_id`; the later `updated_at_unix_ms` wins
    /// (a tie keeps this device's). Never enqueued. A meeting deleted here drops the row; one
    /// not synced here yet parks it in `meeting_speaker_pending` (`None`) until
    /// `record_sync_meeting_map` attaches it.
    pub fn apply_synced_meeting_speaker(&self, peer_device_id: &str, s: &AppliedMeetingSpeaker) -> rusqlite::Result<Option<()>> {
        let origin = s.origin_device_id.as_deref().unwrap_or(peer_device_id);
        let meeting_id: Option<i64> = if self.own_device_id().as_deref() == Some(origin) {
            let exists = self
                .conn
                .query_row("SELECT 1 FROM meeting WHERE id = ?1", params![s.source_meeting_id], |_| Ok(()))
                .optional()?;
            if exists.is_none() {
                return Ok(None);
            }
            Some(s.source_meeting_id)
        } else {
            self.conn
                .query_row(
                    "SELECT m.local_meeting_id FROM sync_meeting_map m JOIN meeting ON meeting.id = m.local_meeting_id \
                     WHERE m.peer_device_id = ?1 AND m.source_meeting_id = ?2",
                    params![origin, s.source_meeting_id],
                    |r| r.get(0),
                )
                .optional()?
        };
        let blob = s.embedding.as_deref().map(to_blob);
        let status = s.status.map(status_str);
        match meeting_id {
            Some(id) => {
                upsert_newer_speaker(&self.conn, id, s.speaker_id, blob, s.person_key.as_deref(), status, s.updated_at_unix_ms)?;
                Ok(Some(()))
            }
            None => {
                self.conn.execute(
                    "INSERT INTO meeting_speaker_pending (origin_device_id, source_meeting_id, speaker_id, embedding, \
                     person_key, status, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT(origin_device_id, source_meeting_id, speaker_id) DO UPDATE SET \
                     embedding = excluded.embedding, person_key = excluded.person_key, status = excluded.status, \
                     updated_at_unix_ms = excluded.updated_at_unix_ms \
                     WHERE excluded.updated_at_unix_ms > meeting_speaker_pending.updated_at_unix_ms",
                    params![origin, s.source_meeting_id, s.speaker_id, blob, s.person_key, status, s.updated_at_unix_ms],
                )?;
                Ok(None)
            }
        }
    }

    /// Moves a peer meeting's pending speaker rows onto its local id, now that it has one.
    pub(crate) fn attach_pending_meeting_speakers(&self, origin_device_id: &str, source_meeting_id: i64, local_meeting_id: i64) -> rusqlite::Result<()> {
        let pending: Vec<(i32, Option<Vec<u8>>, Option<String>, Option<String>, i64)> = {
            let mut stmt = self.conn.prepare(
                "DELETE FROM meeting_speaker_pending WHERE origin_device_id = ?1 AND source_meeting_id = ?2 \
                 RETURNING speaker_id, embedding, person_key, status, updated_at_unix_ms",
            )?;
            let rows = stmt.query_map(params![origin_device_id, source_meeting_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (speaker_id, embedding, person_key, status, updated) in pending {
            upsert_newer_speaker(&self.conn, local_meeting_id, speaker_id, embedding, person_key.as_deref(), status.as_deref(), updated)?;
        }
        Ok(())
    }

    /// A meeting's speakers with their assigned names, by speaker id.
    pub fn meeting_speakers(&self, meeting_id: i64) -> rusqlite::Result<Vec<MeetingSpeaker>> {
        let mut stmt = self.conn.prepare(
            "SELECT ms.speaker_id, ms.person_key, p.name, ms.status FROM meeting_speaker ms \
             LEFT JOIN person p ON p.person_key = ms.person_key WHERE ms.meeting_id = ?1 ORDER BY ms.speaker_id",
        )?;
        let rows = stmt.query_map(params![meeting_id], |r| {
            Ok(MeetingSpeaker { speaker_id: r.get(0)?, person_key: r.get(1)?, name: r.get(2)?, status: parse_status(r.get(3)?) })
        })?;
        rows.collect()
    }

    /// The mean of a person's confirmed embeddings, L2-normalized. Embeddings whose length
    /// differs from the first one's (another embedding model) are skipped.
    pub fn voice_print(&self, person_key: &str) -> rusqlite::Result<Option<Vec<f32>>> {
        let mut stmt = self.conn.prepare(
            "SELECT embedding FROM meeting_speaker WHERE person_key = ?1 AND status = 'confirmed' \
             AND embedding IS NOT NULL ORDER BY meeting_id, speaker_id",
        )?;
        let mut sum: Option<Vec<f32>> = None;
        for blob in stmt.query_map(params![person_key], |r| r.get::<_, Vec<u8>>(0))? {
            let e = from_blob(&blob?);
            match &mut sum {
                None => sum = Some(e),
                Some(s) if s.len() == e.len() => s.iter_mut().zip(&e).for_each(|(a, b)| *a += b),
                Some(_) => {}
            }
        }
        Ok(sum.and_then(|mut s| {
            let norm = s.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm == 0.0 {
                return None;
            }
            s.iter_mut().for_each(|x| *x /= norm);
            Some(s)
        }))
    }

    /// People with at least one confirmed embedding, by name.
    pub fn voice_people(&self) -> rusqlite::Result<Vec<VoicePerson>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.person_key, p.name, COUNT(*) FROM meeting_speaker ms JOIN person p ON p.person_key = ms.person_key \
             WHERE ms.status = 'confirmed' AND ms.embedding IS NOT NULL GROUP BY p.person_key ORDER BY p.name, p.person_key",
        )?;
        let rows = stmt.query_map([], |r| Ok(VoicePerson { person_key: r.get(0)?, name: r.get(1)?, confirmed_count: r.get(2)? }))?;
        rows.collect()
    }

    /// Clears every embedding confirmed to this person (pending peer rows too), so no print is left. Names stay.
    pub fn forget_voice(&self, person_key: &str) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let cleared: Vec<(i64, i32)> = {
            let mut stmt = tx.prepare(
                "UPDATE meeting_speaker SET embedding = NULL, updated_at_unix_ms = MAX(?2, updated_at_unix_ms + 1) \
                 WHERE person_key = ?1 AND status = 'confirmed' AND embedding IS NOT NULL RETURNING meeting_id, speaker_id",
            )?;
            let rows = stmt.query_map(params![person_key, crate::now_unix_ms()], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (meeting_id, speaker_id) in cleared {
            self.enqueue_meeting_speaker_row(&tx, meeting_id, speaker_id)?;
        }
        // A peer's rows still waiting for their meeting; never enqueued, so the stamp stays.
        tx.execute(
            "UPDATE meeting_speaker_pending SET embedding = NULL WHERE person_key = ?1 AND status = 'confirmed'",
            params![person_key],
        )?;
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeetingStatus, MeetingTrigger, NewMeeting, NewTranscriptSegment, Store};

    fn attendee(name: &str, email: Option<&str>) -> Attendee {
        Attendee { name: name.into(), email: email.map(str::to_string) }
    }

    fn meeting(s: &Store, start_unix_ms: i64, details: Vec<Attendee>) -> i64 {
        let id = s
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms,
                end_unix_ms: None,
                title: "t".into(),
                attendees: details.iter().map(|a| a.name.clone()).collect(),
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        if !details.is_empty() {
            s.set_meeting_attendee_details(id, &details).unwrap();
        }
        id
    }

    #[test]
    fn attendee_details_round_trip() {
        let s = Store::open_in_memory().unwrap();
        let details = vec![attendee("Sam", Some("sam@x.com")), attendee("Ana", None)];
        let id = meeting(&s, 1_000, details.clone());
        assert_eq!(s.meeting_attendee_details(id).unwrap(), details);
        assert_eq!(s.get_meeting(id).unwrap().unwrap().attendees, vec!["Sam".to_string(), "Ana".to_string()]);
        // A meeting from before emails were kept falls back to its names.
        let old = s
            .insert_meeting(&NewMeeting { attendees: vec!["Bo".into()], ..sample_new_meeting() })
            .unwrap();
        assert_eq!(s.meeting_attendee_details(old).unwrap(), vec![attendee("Bo", None)]);
        assert!(s.meeting_attendee_details(9_999).unwrap().is_empty());
    }

    fn sample_new_meeting() -> NewMeeting {
        NewMeeting {
            device_id: "d".into(),
            start_unix_ms: 1,
            end_unix_ms: None,
            title: "t".into(),
            attendees: vec![],
            trigger: MeetingTrigger::Calendar,
            audio_path: None,
            status: MeetingStatus::Completed,
        }
    }

    #[test]
    fn person_key_prefers_email_then_name() {
        assert_eq!(person_key(&attendee("Sam", Some(" Sam@X.com "))), "sam@x.com");
        assert_eq!(person_key(&attendee(" Sam Lee ", None)), "name:sam lee");
        assert_eq!(person_key(&attendee("Sam", Some("  "))), "name:sam");
    }

    #[test]
    fn same_name_different_email_are_two_people() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        let a = person_key(&attendee("Sam", Some("sam@a.com")));
        let b = person_key(&attendee("Sam", Some("sam@b.com")));
        assert_ne!(a, b);
        s.upsert_person(&a, "Sam", Some("sam@a.com")).unwrap();
        s.upsert_person(&b, "Sam", Some("sam@b.com")).unwrap();
        let id = meeting(&s, 1_000, vec![]);
        s.set_meeting_speaker(id, 0, Some(&a), Some(NameStatus::Confirmed)).unwrap();
        s.set_meeting_speaker(id, 1, Some(&b), Some(NameStatus::Guess)).unwrap();
        let speakers = s.meeting_speakers(id).unwrap();
        assert_eq!(speakers.len(), 2);
        assert_eq!(speakers[0].person_key.as_deref(), Some(a.as_str()));
        assert_eq!(speakers[1].person_key.as_deref(), Some(b.as_str()));
        assert_eq!(speakers[1].name.as_deref(), Some("Sam"));
        // Each write enqueued its row, in its own transaction.
        let ops = s.list_outbox_operations().unwrap();
        let types: Vec<&str> = ops.iter().map(|o| o.1.as_str()).collect();
        assert_eq!(
            types,
            [
                crate::OUTBOX_ROW_TYPE_PERSON,
                crate::OUTBOX_ROW_TYPE_PERSON,
                crate::OUTBOX_ROW_TYPE_MEETING_SPEAKER,
                crate::OUTBOX_ROW_TYPE_MEETING_SPEAKER
            ]
        );
        let p: AppliedPerson = serde_json::from_str(&ops[1].3).unwrap();
        assert_eq!((p.person_key.as_str(), p.email.as_deref()), (b.as_str(), Some("sam@b.com")));
        let ms: AppliedMeetingSpeaker = serde_json::from_str(&ops[3].3).unwrap();
        assert_eq!((ms.source_meeting_id, ms.speaker_id), (id, 1));
        assert_eq!(ms.status, Some(NameStatus::Guess));
    }

    #[test]
    fn confirming_a_guess_keeps_its_person_when_invitees_share_a_name() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        let id = meeting(&s, 1_000, vec![attendee("Sam", Some("sam@a.com")), attendee("Sam", Some("sam@b.com"))]);
        s.upsert_person("sam@a.com", "Sam", Some("sam@a.com")).unwrap();
        s.upsert_person("sam@b.com", "Sam", Some("sam@b.com")).unwrap();
        s.set_meeting_speaker(id, 0, Some("sam@b.com"), Some(NameStatus::Guess)).unwrap();
        let before = s.list_outbox_operations().unwrap().len();
        // Shown Sam A by a stale page: nothing is confirmed.
        assert!(!s.confirm_meeting_speaker(id, 0, "sam@a.com").unwrap());
        assert_eq!(s.list_outbox_operations().unwrap().len(), before);
        assert!(s.confirm_meeting_speaker(id, 0, "sam@b.com").unwrap());
        let sp = &s.meeting_speakers(id).unwrap()[0];
        assert_eq!((sp.person_key.as_deref(), sp.status), (Some("sam@b.com"), Some(NameStatus::Confirmed)));
        assert_eq!(s.list_outbox_operations().unwrap().len(), before + 1, "the confirmed row syncs");
        assert!(!s.confirm_meeting_speaker(id, 7, "sam@b.com").unwrap(), "no such speaker");
    }

    #[test]
    fn synced_rows_keep_the_newer_and_ties_keep_existing() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        let id = meeting(&s, 1_000, vec![]);
        s.set_meeting_speaker(id, 0, Some("ana"), Some(NameStatus::Guess)).unwrap();
        let mine = s.list_outbox_operations().unwrap();
        let mine: AppliedMeetingSpeaker = serde_json::from_str(&mine[0].3).unwrap();
        let theirs = |key: &str, at: i64| AppliedMeetingSpeaker {
            origin_device_id: Some("dev".into()),
            person_key: Some(key.into()),
            status: Some(NameStatus::Confirmed),
            updated_at_unix_ms: at,
            ..mine.clone()
        };
        let key = |s: &Store| s.meeting_speakers(id).unwrap()[0].person_key.clone();
        s.apply_synced_meeting_speaker("peer", &theirs("bo", mine.updated_at_unix_ms)).unwrap();
        assert_eq!(key(&s).as_deref(), Some("ana"), "a tie keeps this device's");
        s.apply_synced_meeting_speaker("peer", &theirs("bo", mine.updated_at_unix_ms - 1)).unwrap();
        assert_eq!(key(&s).as_deref(), Some("ana"), "an older edit is dropped");
        s.apply_synced_meeting_speaker("peer", &theirs("bo", mine.updated_at_unix_ms + 1)).unwrap();
        assert_eq!(key(&s).as_deref(), Some("bo"));
        // A local edit stamps past the peer's, however far ahead its clock is.
        s.apply_synced_meeting_speaker("peer", &theirs("cy", i64::MAX / 2)).unwrap();
        s.set_meeting_speaker(id, 0, Some("ana"), Some(NameStatus::Confirmed)).unwrap();
        let last: AppliedMeetingSpeaker = serde_json::from_str(&s.list_outbox_operations().unwrap().last().unwrap().3).unwrap();
        assert_eq!(last.updated_at_unix_ms, i64::MAX / 2 + 1);
        assert_eq!(s.list_outbox_operations().unwrap().len(), 2, "applied rows are never enqueued");
    }

    #[test]
    fn voice_print_averages_confirmed_only() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_person("sam@x.com", "Sam", Some("sam@x.com")).unwrap();
        let m1 = meeting(&s, 1_000, vec![]);
        let m2 = meeting(&s, 2_000, vec![]);
        let m3 = meeting(&s, 3_000, vec![]);
        s.save_meeting_voices(m1, &[(0, vec![1.0, 0.0])], true).unwrap();
        s.save_meeting_voices(m2, &[(0, vec![0.0, 1.0])], true).unwrap();
        s.save_meeting_voices(m3, &[(0, vec![-1.0, 0.0]), (1, vec![1.0, 0.0, 0.0])], true).unwrap();
        s.set_meeting_speaker(m1, 0, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        s.set_meeting_speaker(m2, 0, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        // A guess doesn't count, nor does an embedding from another model (other length).
        s.set_meeting_speaker(m3, 0, Some("sam@x.com"), Some(NameStatus::Guess)).unwrap();
        s.set_meeting_speaker(m3, 1, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        let print = s.voice_print("sam@x.com").unwrap().unwrap();
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(print.len(), 2);
        assert!((print[0] - h).abs() < 1e-6 && (print[1] - h).abs() < 1e-6, "{print:?}");
        assert_eq!(s.voice_print("nobody").unwrap(), None);
        let people = s.voice_people().unwrap();
        assert_eq!(people, vec![VoicePerson { person_key: "sam@x.com".into(), name: "Sam".into(), confirmed_count: 3 }]);
    }

    #[test]
    fn transcribing_again_unnames_a_speaker_whose_voice_changed() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_person("sam@x.com", "Sam", None).unwrap();
        let id = meeting(&s, 1_000, vec![]);
        let key = |s: &Store, sp: i32| {
            let m = s.meeting_speakers(id).unwrap().into_iter().find(|m| m.speaker_id == sp).unwrap();
            (m.person_key, m.status)
        };
        let sam = (Some("sam@x.com".to_string()), Some(NameStatus::Confirmed));
        // Speaker 0 had a voice; speaker 1 was named without one (remember off).
        s.save_meeting_voices(id, &[(0, vec![1.0, 0.0])], true).unwrap();
        s.save_meeting_voices(id, &[(1, vec![1.0, 0.0])], false).unwrap();
        s.set_meeting_speaker(id, 0, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        s.set_meeting_speaker(id, 1, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        // The same voice again keeps the name.
        s.save_meeting_voices(id, &[(0, vec![0.99, 0.05])], true).unwrap();
        assert_eq!(key(&s, 0), sam);
        // No old voice to compare: the name stays.
        s.save_meeting_voices(id, &[(1, vec![0.0, 1.0])], true).unwrap();
        assert_eq!(key(&s, 1), sam);
        // Another voice behind speaker 0 now: the name no longer applies.
        s.save_meeting_voices(id, &[(0, vec![0.0, 1.0])], true).unwrap();
        assert_eq!(key(&s, 0), (None, None));
    }

    #[test]
    fn remember_off_stores_no_embedding() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_person("sam@x.com", "Sam", None).unwrap();
        let id = meeting(&s, 1_000, vec![]);
        s.save_meeting_voices(id, &[(0, vec![1.0, 0.0])], false).unwrap();
        s.set_meeting_speaker(id, 0, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        assert_eq!(s.meeting_speakers(id).unwrap().len(), 1);
        assert_eq!(s.voice_print("sam@x.com").unwrap(), None);
        assert!(s.voice_people().unwrap().is_empty());
    }

    #[test]
    fn forget_voice_clears_embeddings_keeps_names() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        s.upsert_person("sam@x.com", "Sam", None).unwrap();
        let id = meeting(&s, 1_000, vec![]);
        s.save_meeting_voices(id, &[(0, vec![1.0, 0.0])], true).unwrap();
        s.set_meeting_speaker(id, 0, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        assert!(s.voice_print("sam@x.com").unwrap().is_some());
        let before = s.list_outbox_operations().unwrap().len();
        s.forget_voice("sam@x.com").unwrap();
        assert_eq!(s.voice_print("sam@x.com").unwrap(), None);
        assert!(s.voice_people().unwrap().is_empty());
        let sp = &s.meeting_speakers(id).unwrap()[0];
        assert_eq!((sp.name.as_deref(), sp.status), (Some("Sam"), Some(NameStatus::Confirmed)));
        let ops = s.list_outbox_operations().unwrap();
        assert_eq!(ops.len(), before + 1, "the cleared row syncs");
        let ms: AppliedMeetingSpeaker = serde_json::from_str(&ops.last().unwrap().3).unwrap();
        assert_eq!(ms.embedding, None);
    }

    /// Parks a peer's speaker row (its meeting never synced here) and reads its embedding back.
    fn park(s: &Store, source_meeting_id: i64, person_key: &str, status: NameStatus, at: i64) {
        let row = AppliedMeetingSpeaker {
            origin_device_id: None,
            source_meeting_id,
            speaker_id: 0,
            embedding: Some(vec![1.0, 0.0]),
            person_key: Some(person_key.into()),
            status: Some(status),
            updated_at_unix_ms: at,
        };
        assert_eq!(s.apply_synced_meeting_speaker("peer", &row).unwrap(), None);
    }

    fn pending_embeddings(s: &Store) -> Vec<(i64, bool)> {
        let mut stmt = s
            .conn
            .prepare("SELECT source_meeting_id, embedding IS NOT NULL FROM meeting_speaker_pending ORDER BY source_meeting_id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    #[test]
    fn forget_voice_clears_pending_confirmed_embeddings() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        park(&s, 1, "sam@x.com", NameStatus::Confirmed, 10);
        park(&s, 2, "sam@x.com", NameStatus::Guess, 10);
        park(&s, 3, "ana@x.com", NameStatus::Confirmed, 10);
        s.forget_voice("sam@x.com").unwrap();
        assert_eq!(pending_embeddings(&s), vec![(1, false), (2, true), (3, true)]);
    }

    #[test]
    fn delete_meetings_before_ages_out_pending_speakers() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        park(&s, 1, "sam@x.com", NameStatus::Guess, 10);
        park(&s, 2, "sam@x.com", NameStatus::Guess, 1_000);
        s.delete_meetings_before(100).unwrap();
        assert_eq!(pending_embeddings(&s), vec![(2, true)]);
    }

    #[test]
    fn display_label_resolves_assigned_speakers_only() {
        let speakers = vec![
            MeetingSpeaker { speaker_id: 0, person_key: Some("a".into()), name: Some("Sam".into()), status: Some(NameStatus::Guess) },
            MeetingSpeaker { speaker_id: 9, person_key: Some("b".into()), name: Some("Ana".into()), status: Some(NameStatus::Confirmed) },
            MeetingSpeaker { speaker_id: 2, person_key: None, name: None, status: None },
        ];
        assert_eq!(display_label("Speaker 1", &speakers), ("Sam".to_string(), true));
        assert_eq!(display_label("Speaker 10", &speakers), ("Ana".to_string(), false));
        assert_eq!(display_label("Speaker 3", &speakers), ("Speaker 3".to_string(), false));
        assert_eq!(display_label("Speaker 0", &speakers), ("Speaker 0".to_string(), false));
        assert_eq!(display_label("Me", &speakers), ("Me".to_string(), false));
    }

    #[test]
    fn transcript_text_resolves_names_and_marks_guesses() {
        use chrono::{Local, TimeZone};
        let s = Store::open_in_memory().unwrap();
        let midnight = Local.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).single().unwrap().timestamp_millis();
        let id = meeting(&s, midnight, vec![attendee("Sam", None), attendee("Ana", Some("ana@x.com"))]);
        s.insert_transcript_segments(&[
            NewTranscriptSegment { meeting_id: id, start_cs: 100, speaker_label: "Speaker 1".into(), text: "hi".into() },
            NewTranscriptSegment { meeting_id: id, start_cs: 200, speaker_label: "Speaker 2".into(), text: "hey".into() },
            NewTranscriptSegment { meeting_id: id, start_cs: 300, speaker_label: "Speaker 3".into(), text: "yo".into() },
        ])
        .unwrap();
        s.upsert_person("name:sam", "Sam", None).unwrap();
        s.upsert_person("ana@x.com", "Ana", Some("ana@x.com")).unwrap();
        s.set_meeting_speaker(id, 0, Some("name:sam"), Some(NameStatus::Guess)).unwrap();
        s.set_meeting_speaker(id, 1, Some("ana@x.com"), Some(NameStatus::Confirmed)).unwrap();
        assert_eq!(
            s.meeting_transcript_text(id).unwrap(),
            "Invitees: Sam, Ana\n[00:00:01] Sam (unconfirmed): hi\n[00:00:02] Ana: hey\n[00:00:03] Speaker 3: yo"
        );
    }

    #[test]
    fn display_label_over_stored_speakers_marks_guess() {
        let s = Store::open_in_memory().unwrap();
        let id = meeting(&s, 1_000, vec![]);
        s.upsert_person("name:sam", "Sam", None).unwrap();
        s.save_meeting_voices(id, &[(1, vec![1.0])], true).unwrap();
        s.set_meeting_speaker(id, 0, Some("name:sam"), Some(NameStatus::Guess)).unwrap();
        let speakers = s.meeting_speakers(id).unwrap();
        assert_eq!(display_label("Speaker 1", &speakers), ("Sam".to_string(), true));
        assert_eq!(display_label("Speaker 2", &speakers), ("Speaker 2".to_string(), false), "voice, no name");
    }

    #[test]
    fn delete_meeting_removes_meeting_speakers() {
        let s = Store::open_in_memory().unwrap();
        let id = meeting(&s, 1_000, vec![]);
        let old = meeting(&s, 10, vec![]);
        s.save_meeting_voices(id, &[(0, vec![1.0])], true).unwrap();
        s.save_meeting_voices(old, &[(0, vec![1.0])], true).unwrap();
        s.delete_meeting_returning_files(id).unwrap();
        assert!(s.meeting_speakers(id).unwrap().is_empty());
        s.delete_meetings_before(100).unwrap();
        assert!(s.meeting_speakers(old).unwrap().is_empty());
    }

    #[test]
    fn fold_duplicate_repoints_meeting_speakers() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_person("name:sam", "Sam", None).unwrap();
        let keep = meeting(&s, 1_000, vec![]);
        let drop = meeting(&s, 1_000, vec![]);
        s.save_meeting_voices(keep, &[(0, vec![1.0])], true).unwrap();
        // Speaker 0 collides with the kept copy's; speaker 1 is only on the dropped copy.
        s.save_meeting_voices(drop, &[(0, vec![2.0]), (1, vec![3.0])], true).unwrap();
        s.set_meeting_speaker(drop, 1, Some("name:sam"), Some(NameStatus::Confirmed)).unwrap();
        assert!(s.fold_duplicate_meeting(keep, drop).unwrap());
        let speakers = s.meeting_speakers(keep).unwrap();
        assert_eq!(speakers.iter().map(|m| m.speaker_id).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(speakers[1].name.as_deref(), Some("Sam"));
        assert!(s.meeting_speakers(drop).unwrap().is_empty());
        assert_eq!(s.voice_print("name:sam").unwrap(), Some(vec![1.0]));
    }

    #[test]
    fn synced_meeting_keeps_attendee_details_and_old_payloads_parse() {
        let old: crate::AppliedMeeting = serde_json::from_str(
            r#"{"start_unix_ms":1,"end_unix_ms":null,"title":"t","attendees":["Sam"],"trigger":"calendar",
                "status":"completed","transcript_path":null,"notes":null,"summary_tldr":null,"summary_decisions":[],
                "summary_action_items":[],"summary_open_questions":[],"summary_provider":null,"summary_model":null}"#,
        )
        .unwrap();
        assert!(old.attendee_details.is_empty());
        let s = Store::open_in_memory().unwrap();
        let details = vec![attendee("Sam", Some("sam@x.com"))];
        let applied = crate::AppliedMeeting { attendee_details: details.clone(), ..old };
        let id = s.apply_synced_meeting("peer", 1, None, &applied).unwrap();
        assert_eq!(s.meeting_attendee_details(id).unwrap(), details);
        s.apply_synced_meeting("peer", 2, Some(id), &crate::AppliedMeeting { attendee_details: vec![], ..applied }).unwrap();
        assert_eq!(s.meeting_attendee_details(id).unwrap(), vec![attendee("Sam", None)], "falls back to the names");
    }
}
