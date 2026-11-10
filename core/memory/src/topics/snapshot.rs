//! The topic page as one self-contained JSON document a phone can store and render: the Mac
//! builds it from `topic_page`, with no email bodies and no credentials, and capped in size.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::entities::parse_addresses;
use super::page::{NextMeeting, RoleView, StepView, SummaryLead};
use super::read::TopicMember;
use crate::redact::redact_secrets;
use crate::skin_sync::sha256_hex;
use crate::Store;

/// The most a serialized snapshot may take.
pub const SNAPSHOT_MAX_BYTES: usize = 32_768;

/// The longest role a snapshot carries, in characters.
pub const SNAPSHOT_ROLE_MAX_CHARS: usize = 2_000;

/// The longest name, title, sender or step text a snapshot carries, in characters.
pub const SNAPSHOT_TEXT_MAX_CHARS: usize = 500;

/// A changed topic is published again no sooner than this after its last publish.
pub const SNAPSHOT_REPUBLISH_MS: i64 = 5 * 60 * 1000;

/// SHA-256 of the snapshot JSON with the build time zeroed and the builder blanked, so a
/// rebuild of the same page hashes the same.
fn content_hash(snap: &TopicPageSnapshot) -> rusqlite::Result<String> {
    let mut s = snap.clone();
    s.built_at_unix_ms = 0;
    s.built_by.clear();
    let json = serde_json::to_vec(&s).map_err(crate::json_to_sql_error)?;
    Ok(sha256_hex(&json))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicPageSnapshot {
    pub topic_key: String,
    pub name: String,
    pub kind: Option<String>,
    pub deleted: bool,
    pub lead: Option<SummaryLead>,
    pub role: Option<RoleView>,
    pub next_meeting: Option<NextMeeting>,
    pub you_owe: Vec<SnapshotStep>,
    pub owed_to_you: Vec<SnapshotStep>,
    pub done: Vec<SnapshotStep>,
    pub recent: Vec<SnapshotRecent>,
    pub recent_total: i64,
    /// True when steps or recent items were dropped to fit `SNAPSHOT_MAX_BYTES`.
    pub truncated: bool,
    pub built_at_unix_ms: i64,
    pub built_by: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotStep {
    pub text: String,
    pub source_kind: String,
    pub source_title: Option<String>,
    pub asked_at_unix_ms: Option<i64>,
    pub due_date: Option<String>,
    pub meeting: Option<MeetingRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRecent {
    pub kind: String,
    pub title: String,
    /// An email's first sender, by display name; never the body.
    pub sender: Option<String>,
    pub ts_unix_ms: i64,
    pub meeting: Option<MeetingRef>,
}

/// The cross-device identity of a meeting: the device that recorded it and its id there
/// (`MeetingPayload`'s origin device and `source_meeting_id`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingRef {
    pub device_id: String,
    pub source_meeting_id: i64,
}

/// `text` cut to `max_chars` characters, the last one an ellipsis when it was cut.
fn clip(text: String, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars.saturating_sub(1)) {
        Some((i, _)) if text.chars().count() > max_chars => {
            let mut out = text[..i].to_string();
            out.push('…');
            out
        }
        _ => text,
    }
}

/// Free text with secrets redacted, cut to `SNAPSHOT_TEXT_MAX_CHARS`.
fn clean(text: &str) -> String {
    clip(redact_secrets(text).into_owned(), SNAPSHOT_TEXT_MAX_CHARS)
}

/// The snapshot when it fits `SNAPSHOT_MAX_BYTES`, dropping recent items first, then done
/// steps, and only then the tail of the open ones. `None` when it still does not fit with
/// every list empty; a snapshot that cannot be serialized does not fit.
fn fit_snapshot(mut snap: TopicPageSnapshot) -> Option<TopicPageSnapshot> {
    let fits = |s: &TopicPageSnapshot| {
        serde_json::to_vec(s).is_ok_and(|b| b.len() <= SNAPSHOT_MAX_BYTES)
    };
    while !fits(&snap) {
        snap.truncated = true;
        if snap.recent.pop().is_none()
            && snap.done.pop().is_none()
            && snap.owed_to_you.pop().is_none()
            && snap.you_owe.pop().is_none()
        {
            return None;
        }
    }
    Some(snap)
}

impl Store {
    /// A meeting recorded here is (this device, its local id); one that arrived by sync is
    /// (the peer that recorded it, the id it had there). `None` when the origin is unknown.
    fn meeting_ref(&self, local_id: i64) -> rusqlite::Result<Option<MeetingRef>> {
        let mapped = self
            .conn
            .query_row(
                "SELECT peer_device_id, source_meeting_id FROM sync_meeting_map \
                 WHERE local_meeting_id = ?1 ORDER BY peer_device_id, source_meeting_id LIMIT 1",
                [local_id],
                |r| {
                    Ok(MeetingRef {
                        device_id: r.get(0)?,
                        source_meeting_id: r.get(1)?,
                    })
                },
            )
            .optional()?;
        if mapped.is_some() {
            return Ok(mapped);
        }
        let Some(own) = self.own_device_id() else {
            return Ok(None);
        };
        let recorded_here: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM meeting WHERE id = ?1 AND device_id = ?2)",
            params![local_id, own],
            |r| r.get(0),
        )?;
        Ok(recorded_here.then_some(MeetingRef {
            device_id: own,
            source_meeting_id: local_id,
        }))
    }

    /// The display name of the first address in the email item's `from` (first) actor.
    fn email_sender(&self, item_id: i64) -> rusqlite::Result<Option<String>> {
        let actors: Option<String> = self
            .conn
            .query_row("SELECT actors FROM items WHERE id = ?1", [item_id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        let from = actors
            .and_then(|a| serde_json::from_str::<Vec<String>>(&a).ok())
            .and_then(|a| a.into_iter().next());
        Ok(from
            .and_then(|f| parse_addresses(&f).into_iter().next())
            .and_then(|a| a.name)
            .map(|n| clean(&n))
            .filter(|n| !n.is_empty()))
    }

    fn snapshot_step(&self, s: StepView) -> rusqlite::Result<SnapshotStep> {
        let meeting = match (&s.open, s.source_kind.as_str()) {
            (Some(o), "meeting") if o.window == "meeting-detail" => self.meeting_ref(o.id)?,
            _ => None,
        };
        Ok(SnapshotStep {
            text: clean(&s.text),
            source_kind: s.source_kind,
            source_title: Some(clean(&s.source_title)).filter(|t| !t.is_empty()),
            asked_at_unix_ms: Some(s.asked_at_unix_ms).filter(|t| *t > 0),
            due_date: s.due_date.map(|d| clip(d, SNAPSHOT_TEXT_MAX_CHARS)),
            meeting,
        })
    }

    fn snapshot_recent(&self, m: TopicMember) -> rusqlite::Result<SnapshotRecent> {
        let (sender, meeting) = match (m.kind.as_str(), m.ref_id) {
            ("email", Some(id)) if !m.stub => (self.email_sender(id)?, None),
            ("meeting", Some(id)) if !m.stub => (None, self.meeting_ref(id)?),
            _ => (None, None),
        };
        Ok(SnapshotRecent {
            title: clean(&m.title),
            kind: clip(m.kind, SNAPSHOT_TEXT_MAX_CHARS),
            sender,
            ts_unix_ms: m.ts_unix_ms,
            meeting,
        })
    }

    /// The topic page as a phone-sized snapshot; `None` unless `topic_id` is a live topic.
    /// Older steps are left out, and free text is cut to `SNAPSHOT_TEXT_MAX_CHARS` (the role
    /// to `SNAPSHOT_ROLE_MAX_CHARS`). Over `SNAPSHOT_MAX_BYTES`, recent items go first, then
    /// done steps, and only then the tail of the open ones; also `None` when it still does
    /// not fit with every list empty, so an oversize page is never published.
    pub fn build_topic_page_snapshot(
        &self,
        topic_id: i64,
        built_by: &str,
        now_ms: i64,
    ) -> rusqlite::Result<Option<TopicPageSnapshot>> {
        let Some(page) = self.topic_page(topic_id, now_ms)? else {
            return Ok(None);
        };
        let steps = |v: Vec<StepView>| {
            v.into_iter()
                .map(|s| self.snapshot_step(s))
                .collect::<rusqlite::Result<Vec<_>>>()
        };
        let short = |t: String| clip(t, SNAPSHOT_TEXT_MAX_CHARS);
        let snap = TopicPageSnapshot {
            topic_key: page.topic.key,
            name: clean(&page.topic.name),
            kind: page.kind.map(short),
            deleted: false,
            lead: page.lead.map(|l| match l {
                SummaryLead::Date { label, date } => SummaryLead::Date {
                    label: clean(&label),
                    date: short(date),
                },
                SummaryLead::Step { text, due_date } => SummaryLead::Step {
                    text: clean(&text),
                    due_date: due_date.map(short),
                },
            }),
            role: Some(page.role)
                .filter(|r| !r.text.is_empty())
                .map(|r| RoleView {
                    text: clip(redact_secrets(&r.text).into_owned(), SNAPSHOT_ROLE_MAX_CHARS),
                    edited: r.edited,
                }),
            next_meeting: page.next_meeting.map(|n| NextMeeting {
                title: clean(&n.title),
                attendees_by_org: n
                    .attendees_by_org
                    .into_iter()
                    .map(|(domain, count)| (short(domain), count))
                    .collect(),
                ..n
            }),
            you_owe: steps(page.you_owe)?,
            owed_to_you: steps(page.owed_to_you)?,
            done: steps(page.done)?,
            recent: page
                .recent
                .into_iter()
                .map(|m| self.snapshot_recent(m))
                .collect::<rusqlite::Result<_>>()?,
            recent_total: page.recent_total,
            truncated: false,
            built_at_unix_ms: now_ms,
            built_by: built_by.to_string(),
        };
        Ok(fit_snapshot(snap))
    }

    /// Builds the named topics' snapshots and queues those that are due: never published, or
    /// changed and last published at least `SNAPSHOT_REPUBLISH_MS` ago. A topic that was
    /// published and is no longer named (archived, merged away) gets one `deleted` snapshot.
    /// Returns how many were queued. A store with no device id queues, and records, nothing.
    pub fn publish_topic_page_snapshots(&self, built_by: &str, now_ms: i64) -> rusqlite::Result<usize> {
        if self.own_device_id().is_none() {
            return Ok(0);
        }
        let mut queued = 0;
        let mut named = std::collections::HashSet::new();
        for entry in self.topic_page_list(now_ms)? {
            named.insert(entry.topic.key.clone());
            let Some(mut snap) = self.build_topic_page_snapshot(entry.topic.topic_id, built_by, now_ms)?
            else {
                continue;
            };
            let hash = content_hash(&snap)?;
            // Due is judged on this device's own last publish only; applied rows never count.
            let prev: Option<(Option<String>, Option<i64>, i64)> = self
                .conn
                .query_row(
                    "SELECT own_hash, own_published_at_unix_ms, built_at_unix_ms FROM topic_page_snapshot WHERE topic_key = ?1",
                    [&snap.topic_key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let due = match &prev {
                Some((Some(h), Some(at), _)) => {
                    *h != hash && now_ms.saturating_sub(*at) >= SNAPSHOT_REPUBLISH_MS
                }
                _ => true,
            };
            if due {
                // Newer than whatever is stored, even a peer's build from a clock ahead of ours.
                snap.built_at_unix_ms = now_ms.max(prev.map_or(i64::MIN, |p| p.2.saturating_add(1)));
                self.store_and_enqueue_snapshot(&snap, &hash, now_ms)?;
                queued += 1;
            }
        }
        // Published (or applied) rows of topics this device knows but no longer names.
        let mut stmt = self.conn.prepare(
            "SELECT p.topic_key, p.payload FROM topic_page_snapshot p \
             WHERE p.published_at_unix_ms IS NOT NULL \
               AND EXISTS (SELECT 1 FROM topic_seed s WHERE s.key = p.topic_key) \
             ORDER BY p.topic_key",
        )?;
        let gone: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for (key, payload) in gone {
            if named.contains(&key) {
                continue;
            }
            let Ok(mut snap) = serde_json::from_str::<TopicPageSnapshot>(&payload) else {
                continue;
            };
            if snap.deleted {
                continue;
            }
            snap.deleted = true;
            snap.kind = None;
            snap.lead = None;
            snap.role = None;
            snap.next_meeting = None;
            snap.you_owe.clear();
            snap.owed_to_you.clear();
            snap.done.clear();
            snap.recent.clear();
            snap.recent_total = 0;
            snap.truncated = false;
            snap.built_by = built_by.to_string();
            let stored: i64 = self.conn.query_row(
                "SELECT built_at_unix_ms FROM topic_page_snapshot WHERE topic_key = ?1",
                [&key],
                |r| r.get(0),
            )?;
            snap.built_at_unix_ms = now_ms.max(stored.saturating_add(1));
            let hash = content_hash(&snap)?;
            self.store_and_enqueue_snapshot(&snap, &hash, now_ms)?;
            queued += 1;
        }
        Ok(queued)
    }

    /// Stores `snap` as the last published build and queues it, in one transaction.
    fn store_and_enqueue_snapshot(
        &self,
        snap: &TopicPageSnapshot,
        hash: &str,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        let json = serde_json::to_string(snap).map_err(crate::json_to_sql_error)?;
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO topic_page_snapshot (topic_key, payload, built_at_unix_ms, content_hash, published_at_unix_ms, own_hash, own_published_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?5) \
             ON CONFLICT(topic_key) DO UPDATE SET payload = excluded.payload, \
               built_at_unix_ms = excluded.built_at_unix_ms, content_hash = excluded.content_hash, \
               published_at_unix_ms = excluded.published_at_unix_ms, \
               own_hash = excluded.own_hash, own_published_at_unix_ms = excluded.own_published_at_unix_ms",
            params![snap.topic_key, json, snap.built_at_unix_ms, hash, now_ms],
        )?;
        self.enqueue_topic_page_outbox(
            &tx,
            crate::OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT,
            snap.topic_key.clone(),
            snap,
        )?;
        tx.commit()
    }

    /// Stores a snapshot that arrived by sync. Last write wins on `built_at_unix_ms`: an
    /// equal or older build is ignored. The row counts as published, so this device does not
    /// publish the same page back. A build dated before 0 or past `now_ms` plus the synced-run
    /// allowance, or over `SNAPSHOT_MAX_BYTES`, is dropped. This device's own publish state
    /// is never touched.
    pub fn apply_synced_topic_page_snapshot(
        &self,
        snap: &TopicPageSnapshot,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        if snap.built_at_unix_ms < 0
            || snap.built_at_unix_ms > now_ms.saturating_add(crate::SYNCED_RUN_MAX_FUTURE_MS)
        {
            return Ok(());
        }
        let json = serde_json::to_string(snap).map_err(crate::json_to_sql_error)?;
        if json.len() > SNAPSHOT_MAX_BYTES {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO topic_page_snapshot (topic_key, payload, built_at_unix_ms, content_hash, published_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?3) \
             ON CONFLICT(topic_key) DO UPDATE SET payload = excluded.payload, \
               built_at_unix_ms = excluded.built_at_unix_ms, content_hash = excluded.content_hash, \
               published_at_unix_ms = excluded.published_at_unix_ms \
             WHERE excluded.built_at_unix_ms > topic_page_snapshot.built_at_unix_ms",
            params![snap.topic_key, json, snap.built_at_unix_ms, content_hash(snap)?],
        )?;
        Ok(())
    }

    /// The stored snapshots that are not deleted: the next meeting soonest first, then by name.
    pub fn topic_page_snapshots(&self) -> rusqlite::Result<Vec<TopicPageSnapshot>> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload FROM topic_page_snapshot ORDER BY topic_key")?;
        let mut out: Vec<TopicPageSnapshot> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .filter_map(|p| serde_json::from_str(p).ok())
            .filter(|s: &TopicPageSnapshot| !s.deleted)
            .collect();
        out.sort_by(|a, b| {
            let next = |s: &TopicPageSnapshot| s.next_meeting.as_ref().map_or(i64::MAX, |n| n.start_unix_ms);
            next(a)
                .cmp(&next(b))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    /// One stored snapshot by topic key, deleted ones included.
    pub fn topic_page_snapshot(&self, topic_key: &str) -> rusqlite::Result<Option<TopicPageSnapshot>> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM topic_page_snapshot WHERE topic_key = ?1",
                [topic_key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(payload.and_then(|p| serde_json::from_str(&p).ok()))
    }

    /// The local meeting id for a `MeetingRef` (the inverse of `meeting_ref`): a meeting recorded
    /// on this device is its own `source_meeting_id`; any other is found through the sync map.
    pub fn meeting_id_for_source(&self, device_id: &str, source_meeting_id: i64) -> rusqlite::Result<Option<i64>> {
        if self.own_device_id().as_deref() == Some(device_id) {
            return self
                .conn
                .query_row(
                    "SELECT id FROM meeting WHERE id = ?1 AND device_id = ?2",
                    params![source_meeting_id, device_id],
                    |r| r.get(0),
                )
                .optional();
        }
        self.conn
            .query_row(
                "SELECT local_meeting_id FROM sync_meeting_map WHERE peer_device_id = ?1 AND source_meeting_id = ?2",
                params![device_id, source_meeting_id],
                |r| r.get(0),
            )
            .optional()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use rusqlite::params;

    const NOW: i64 = 1_794_139_200_000;
    const DAY_MS: i64 = 86_400_000;

    fn sql(s: &Store, q: &str, p: impl rusqlite::Params) -> i64 {
        s.conn.execute(q, p).unwrap();
        s.conn.last_insert_rowid()
    }

    /// A topic_source of `kind` titled `title`, filed on `topic`.
    fn filed(s: &Store, topic: i64, kind: &str, ref_id: Option<i64>, ts: i64, title: &str) -> i64 {
        let sid = sql(
            s,
            "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, ?3, ?4, 0)",
            params![kind, ref_id, ts, title],
        );
        sql(
            s,
            "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')",
            params![topic, sid],
        );
        sid
    }

    fn step(s: &Store, key: &str, topic_key: &str, text: &str, dir: &str, status: &str, sid: i64) {
        sql(
            s,
            "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, \
             asked_at_unix_ms, status, updated_at_unix_ms, source_id) VALUES (?1, ?2, 'meeting:x', 'meeting', 'Sync', ?3, ?4, ?5, ?6, 0, ?7)",
            params![key, topic_key, text, dir, NOW - DAY_MS, status, sid],
        );
    }

    fn meeting(s: &Store, device: &str) -> i64 {
        sql(
            s,
            "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status) VALUES (?1, 1, 'Sync', '[]', 'manual', 'done')",
            [device],
        )
    }

    fn email(s: &Store, from: &str, title: &str, body: &str) -> i64 {
        sql(
            s,
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
             VALUES ('d', 'email', 'gmail', ?1, json_array(?2, 'me@x.com'), ?3, ?4, '{}', 'e1')",
            params![NOW - DAY_MS, from, title, body],
        )
    }

    fn seeded(s: &Store) -> (super::super::page::TopicRef, i64) {
        let t = s.create_seed_topic("Maximus", &[], None).unwrap();
        let mid = meeting(s, "mac-1");
        let sid = filed(s, t.topic_id, "meeting", Some(mid), NOW - 2 * DAY_MS, "Sync");
        step(s, "mine", &t.key, "Send the SOW.", "mine", "open", sid);
        sql(
            s,
            "INSERT INTO topic_extraction (source_key, topic_key, dates, model, extracted_at_unix_ms, updated_at_unix_ms) \
             VALUES ('email:x', ?1, ?2, 'm', 0, 0)",
            params![t.key, r#"[{"date":"2026-11-14","label":"SOW due"}]"#],
        );
        s.set_topic_role(&t.key, "lead consultant").unwrap();
        (
            super::super::page::TopicRef {
                topic_id: t.topic_id,
                key: t.key,
                name: t.name,
            },
            mid,
        )
    }

    #[test]
    fn snapshot_carries_the_topic_page() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("mac-1");
        let (t, mid) = seeded(&s);
        let page = s.topic_page(t.topic_id, NOW).unwrap().unwrap();
        let snap = s
            .build_topic_page_snapshot(t.topic_id, "MacBook", NOW)
            .unwrap()
            .unwrap();
        assert_eq!(snap.topic_key, t.key);
        assert_eq!(snap.name, "Maximus");
        assert_eq!(snap.lead, page.lead);
        assert!(snap.lead.is_some());
        assert_eq!(snap.role.as_ref().unwrap().text, page.role.text);
        assert_eq!(snap.you_owe.len(), 1);
        assert_eq!(snap.you_owe[0].text, page.you_owe[0].text);
        assert_eq!(snap.recent[0].kind, "meeting");
        assert_eq!(snap.recent_total, 1);
        assert_eq!(snap.built_by, "MacBook");
        assert_eq!(snap.built_at_unix_ms, NOW);
        assert!(!snap.deleted && !snap.truncated);
        // A meeting recorded here is (this device, local id).
        let me = MeetingRef {
            device_id: "mac-1".into(),
            source_meeting_id: mid,
        };
        assert_eq!(snap.recent[0].meeting, Some(me.clone()));
        assert_eq!(snap.you_owe[0].meeting, Some(me));
        // It round-trips as JSON.
        let back: TopicPageSnapshot = serde_json::from_slice(&serde_json::to_vec(&snap).unwrap()).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn meeting_ref_is_the_origin_device_and_source_id() {
        let s = Store::open_in_memory().unwrap();
        let t = s.create_seed_topic("Maximus", &[], None).unwrap();
        let local = meeting(&s, "mac-1");
        let pulled = meeting(&s, "peer-9");
        let unmapped = meeting(&s, "peer-9");
        s.record_sync_meeting_map("peer-9", 77, pulled).unwrap();
        for (i, m) in [local, pulled, unmapped].into_iter().enumerate() {
            filed(&s, t.topic_id, "meeting", Some(m), NOW - i as i64 * DAY_MS, "M");
        }
        let refs = |s: &Store| {
            s.build_topic_page_snapshot(t.topic_id, "x", NOW)
                .unwrap()
                .unwrap()
                .recent
                .into_iter()
                .map(|r| r.meeting)
                .collect::<Vec<_>>()
        };
        let r = |d: &str, id| Some(MeetingRef { device_id: d.into(), source_meeting_id: id });
        // No device id of its own: a local meeting cannot be named; a pulled one still can.
        assert_eq!(refs(&s), vec![None, r("peer-9", 77), None]);
        s.set_own_device_id("mac-1");
        assert_eq!(refs(&s), vec![r("mac-1", local), r("peer-9", 77), None]);
    }

    #[test]
    fn meeting_id_for_source_inverts_meeting_ref() {
        let s = Store::open_in_memory().unwrap();
        let local = meeting(&s, "mac-1");
        let pulled = meeting(&s, "peer-9");
        s.record_sync_meeting_map("peer-9", 77, pulled).unwrap();
        // Without a device id of its own, nothing local can be named.
        assert_eq!(s.meeting_id_for_source("mac-1", local).unwrap(), None);
        s.set_own_device_id("mac-1");
        assert_eq!(s.meeting_id_for_source("mac-1", local).unwrap(), Some(local));
        assert_eq!(s.meeting_id_for_source("mac-1", local + 100).unwrap(), None);
        // A peer's meeting is not found under this device, even when the id matches.
        assert_eq!(s.meeting_id_for_source("mac-1", pulled).unwrap(), None);
        assert_eq!(s.meeting_id_for_source("peer-9", 77).unwrap(), Some(pulled));
        assert_eq!(s.meeting_id_for_source("peer-9", 78).unwrap(), None);
        assert_eq!(s.meeting_id_for_source("peer-8", 77).unwrap(), None);
    }

    #[test]
    fn snapshot_email_has_no_body_and_secrets_are_redacted() {
        let s = Store::open_in_memory().unwrap();
        let t = s.create_seed_topic("Maximus", &[], None).unwrap();
        let eid = email(
            &s,
            "Dana Lee <dana@acme.com>",
            "Key sk-ant-api03-AAAAAAAAAAAAAAAAAAAA rotated",
            "SECRET-BODY",
        );
        filed(
            &s,
            t.topic_id,
            "email",
            Some(eid),
            NOW - DAY_MS,
            "Key sk-ant-api03-AAAAAAAAAAAAAAAAAAAA rotated",
        );
        let snap = s
            .build_topic_page_snapshot(t.topic_id, "MacBook", NOW)
            .unwrap()
            .unwrap();
        let json = String::from_utf8(serde_json::to_vec(&snap).unwrap()).unwrap();
        assert!(!json.contains("sk-ant-api03-AAAA"), "{json}");
        assert!(!json.contains("SECRET-BODY"));
        assert_eq!(snap.recent[0].sender.as_deref(), Some("Dana Lee"));
        assert!(snap.recent[0].title.contains("[REDACTED"));
        assert_eq!(snap.recent[0].meeting, None);
    }

    #[test]
    fn snapshot_is_capped_at_32k() {
        let s = Store::open_in_memory().unwrap();
        let t = s.create_seed_topic("Maximus", &[], None).unwrap();
        let long = "t".repeat(200);
        let mut sid = 0;
        for i in 0..400 {
            sid = filed(&s, t.topic_id, "email", None, NOW - DAY_MS - i, &long);
        }
        // Under SNAPSHOT_TEXT_MAX_CHARS, so only dropping steps can make it fit.
        let done_text = "d".repeat(450);
        for i in 0..100 {
            step(&s, &format!("d{i}"), &t.key, &done_text, "mine", "done", sid);
        }
        step(&s, "o1", &t.key, "Send the SOW.", "mine", "open", sid);
        step(&s, "o2", &t.key, "Reply to Dana.", "owed", "open", sid);
        let snap = s
            .build_topic_page_snapshot(t.topic_id, "MacBook", NOW)
            .unwrap()
            .unwrap();
        assert!(serde_json::to_vec(&snap).unwrap().len() <= SNAPSHOT_MAX_BYTES);
        assert!(snap.truncated);
        assert_eq!(snap.recent_total, 400);
        assert_eq!(snap.you_owe.len(), 1);
        assert_eq!(snap.owed_to_you.len(), 1);
        assert_eq!(snap.you_owe[0].text, "Send the SOW.");
        assert!(snap.done.len() < 100);
    }

    #[test]
    fn snapshot_of_unknown_topic_is_none() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.build_topic_page_snapshot(999, "MacBook", NOW).unwrap().is_none());
    }

    fn queued(s: &Store, row_type: &str) -> Vec<String> {
        let mut st = s
            .conn
            .prepare("SELECT payload_json FROM sync_outbox WHERE row_type = ?1 ORDER BY local_seq")
            .unwrap();
        st.query_map([row_type], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    const MIN: i64 = 60_000;
    const SNAP: &str = crate::OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT;

    #[test]
    fn publish_enqueues_once_and_skips_unchanged() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("mac-1");
        let (t, _) = seeded(&s);
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW).unwrap(), 1);
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW + 6 * MIN).unwrap(), 0);
        let sid = filed(&s, t.topic_id, "meeting", None, NOW, "Other");
        step(&s, "two", &t.key, "Call back.", "mine", "open", sid);
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW + 7 * MIN).unwrap(), 1);
        assert_eq!(queued(&s, SNAP).len(), 2);
    }

    #[test]
    fn publish_is_throttled_to_five_minutes() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("mac-1");
        let (t, _) = seeded(&s);
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW).unwrap(), 1);
        let sid = filed(&s, t.topic_id, "meeting", None, NOW, "Other");
        step(&s, "two", &t.key, "Call back.", "mine", "open", sid);
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW + MIN).unwrap(), 0);
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW + SNAPSHOT_REPUBLISH_MS).unwrap(), 1);
    }

    #[test]
    fn archived_topic_publishes_a_deleted_snapshot() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("mac-1");
        let (t, _) = seeded(&s);
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW).unwrap(), 1);
        assert_eq!(s.topic_page_snapshots().unwrap().len(), 1);
        s.delete_seed_topic(&t.key).unwrap();
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW + MIN).unwrap(), 1);
        let last: TopicPageSnapshot = serde_json::from_str(queued(&s, SNAP).last().unwrap()).unwrap();
        assert!(last.deleted);
        assert_eq!(last.name, "Maximus");
        assert!(last.you_owe.is_empty() && last.recent.is_empty());
        assert_eq!(last.built_at_unix_ms, NOW + MIN);
        assert!(s.topic_page_snapshots().unwrap().is_empty());
        assert!(s.topic_page_snapshot(&t.key).unwrap().unwrap().deleted);
        // Once only.
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW + 10 * MIN).unwrap(), 0);
    }

    #[test]
    fn snapshot_apply_keeps_the_newer_build() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seeded(&s);
        let mut snap = s.build_topic_page_snapshot(t.topic_id, "Mac", 200).unwrap().unwrap();
        s.apply_synced_topic_page_snapshot(&snap, 1_000).unwrap();
        snap.built_at_unix_ms = 100;
        snap.name = "Older".into();
        s.apply_synced_topic_page_snapshot(&snap, 1_000).unwrap();
        snap.built_at_unix_ms = 200;
        s.apply_synced_topic_page_snapshot(&snap, 1_000).unwrap();
        let got = s.topic_page_snapshot(&t.key).unwrap().unwrap();
        assert_eq!(got.built_at_unix_ms, 200);
        assert_eq!(got.name, "Maximus");
    }

    #[test]
    fn publish_without_device_id_enqueues_nothing() {
        let s = Store::open_in_memory().unwrap();
        seeded(&s);
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW).unwrap(), 0);
        assert!(queued(&s, SNAP).is_empty());
        let rows: i64 = s.conn.query_row("SELECT COUNT(*) FROM topic_page_snapshot", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0);
        // Once a device id exists, the topic publishes.
        s.set_own_device_id("mac-1");
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW + MIN).unwrap(), 1);
    }

    #[test]
    fn two_computers_with_different_builds_do_not_ping_pong() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("mac-1");
        let (t, _) = seeded(&s);
        assert_eq!(s.publish_topic_page_snapshots("A", NOW).unwrap(), 1);
        let mut peer = s.topic_page_snapshot(&t.key).unwrap().unwrap();
        peer.built_by = "B".into();
        peer.recent_total += 5;
        peer.built_at_unix_ms = NOW + 1000;
        s.apply_synced_topic_page_snapshot(&peer, NOW + 2000).unwrap();
        assert_eq!(s.topic_page_snapshot(&t.key).unwrap().unwrap().recent_total, peer.recent_total);
        assert_eq!(s.publish_topic_page_snapshots("A", NOW + 6 * MIN).unwrap(), 0);
        assert_eq!(queued(&s, SNAP).len(), 1);
    }

    #[test]
    fn local_change_beats_a_peer_clock_ahead() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("mac-1");
        let (t, _) = seeded(&s);
        assert_eq!(s.publish_topic_page_snapshots("A", NOW).unwrap(), 1);
        let mut peer = s.topic_page_snapshot(&t.key).unwrap().unwrap();
        peer.built_by = "B".into();
        peer.built_at_unix_ms = NOW + 60_000;
        s.apply_synced_topic_page_snapshot(&peer, NOW).unwrap();
        let sid = filed(&s, t.topic_id, "meeting", None, NOW, "Other");
        step(&s, "two", &t.key, "Call back.", "mine", "open", sid);
        assert_eq!(s.publish_topic_page_snapshots("A", NOW + 6 * MIN).unwrap(), 1);
        let last: TopicPageSnapshot = serde_json::from_str(queued(&s, SNAP).last().unwrap()).unwrap();
        assert!(last.built_at_unix_ms > peer.built_at_unix_ms);
        // Even when the peer's clock is ahead of the publish time itself.
        peer.built_at_unix_ms = last.built_at_unix_ms + 120_000;
        s.apply_synced_topic_page_snapshot(&peer, last.built_at_unix_ms + 100_000).unwrap();
        let sid = filed(&s, t.topic_id, "meeting", None, NOW, "Third");
        step(&s, "three", &t.key, "Again.", "mine", "open", sid);
        assert_eq!(s.publish_topic_page_snapshots("A", NOW + 12 * MIN).unwrap(), 1);
        let last2: TopicPageSnapshot = serde_json::from_str(queued(&s, SNAP).last().unwrap()).unwrap();
        assert!(last2.built_at_unix_ms > peer.built_at_unix_ms);
    }

    #[test]
    fn stale_computer_does_not_overwrite_after_catching_up() {
        // This computer published, was off, and on waking has pulled a peer's newer build of
        // the same topic. Its own page is what it last published, so it does not publish again.
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("mac-2");
        let (t, _) = seeded(&s);
        assert_eq!(s.publish_topic_page_snapshots("B", NOW).unwrap(), 1);
        let mut peer = s.topic_page_snapshot(&t.key).unwrap().unwrap();
        peer.built_by = "A".into();
        peer.recent_total += 5;
        peer.built_at_unix_ms = NOW + 2 * DAY_MS;
        s.apply_synced_topic_page_snapshot(&peer, NOW + 3 * DAY_MS).unwrap();
        assert_eq!(s.publish_topic_page_snapshots("B", NOW + 3 * DAY_MS).unwrap(), 0);
        assert_eq!(queued(&s, SNAP).len(), 1);
        let kept = s.topic_page_snapshot(&t.key).unwrap().unwrap();
        assert_eq!(kept.built_by, "A");
        assert_eq!(kept.built_at_unix_ms, NOW + 2 * DAY_MS);
    }

    #[test]
    fn long_role_is_capped_and_snapshot_fits() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("mac-1");
        let (t, _) = seeded(&s);
        s.set_topic_role(&t.key, &"r".repeat(40_000)).unwrap();
        let sid = filed(&s, t.topic_id, "email", None, NOW - DAY_MS, &"t".repeat(5_000));
        step(&s, "long", &t.key, &"s".repeat(5_000), "mine", "open", sid);
        let snap = s
            .build_topic_page_snapshot(t.topic_id, "MacBook", NOW)
            .unwrap()
            .unwrap();
        assert!(serde_json::to_vec(&snap).unwrap().len() <= SNAPSHOT_MAX_BYTES);
        let role = &snap.role.as_ref().unwrap().text;
        assert_eq!(role.chars().count(), SNAPSHOT_ROLE_MAX_CHARS);
        assert!(role.ends_with('…'));
        assert!(snap.recent.iter().all(|r| r.title.chars().count() <= SNAPSHOT_TEXT_MAX_CHARS));
        assert!(snap.you_owe.iter().all(|x| x.text.chars().count() <= SNAPSHOT_TEXT_MAX_CHARS));
        // Short text is untouched.
        assert!(snap.you_owe.iter().any(|x| x.text == "Send the SOW."));
        // It publishes, and peers keep it.
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW).unwrap(), 1);
        let peer = Store::open_in_memory().unwrap();
        peer.apply_synced_topic_page_snapshot(&snap, NOW).unwrap();
        assert!(peer.topic_page_snapshot(&t.key).unwrap().is_some());
    }

    #[test]
    fn oversize_after_trimming_is_not_published() {
        // A next meeting with thousands of attendee domains: no list the cap trims can bring
        // it under SNAPSHOT_MAX_BYTES, so nothing is built, stored or queued.
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("mac-1");
        let (t, _) = seeded(&s);
        let actors: Vec<String> = (0..3_000).map(|i| format!("p{i}@org-{i:05}.example.com")).collect();
        sql(
            &s,
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
             VALUES ('d', 'calendar_event', 'google_calendar', ?1, ?2, 'Maximus QBR', '', 'q1')",
            params![NOW + DAY_MS, serde_json::to_string(&actors).unwrap()],
        );
        assert!(s.topic_page(t.topic_id, NOW).unwrap().unwrap().next_meeting.is_some());
        assert!(s.build_topic_page_snapshot(t.topic_id, "Mac", NOW).unwrap().is_none());
        assert_eq!(s.publish_topic_page_snapshots("Mac", NOW).unwrap(), 0);
        assert!(queued(&s, SNAP).is_empty());
        assert!(s.topic_page_snapshot(&t.key).unwrap().is_none());
    }

    #[test]
    fn apply_drops_far_future_and_oversize_snapshots() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seeded(&s);
        let snap = s.build_topic_page_snapshot(t.topic_id, "Mac", 1_000).unwrap().unwrap();
        let mut future = snap.clone();
        future.built_at_unix_ms = 1_000 + crate::SYNCED_RUN_MAX_FUTURE_MS + 1;
        s.apply_synced_topic_page_snapshot(&future, 1_000).unwrap();
        let mut negative = snap.clone();
        negative.built_at_unix_ms = -1;
        s.apply_synced_topic_page_snapshot(&negative, 1_000).unwrap();
        let mut big = snap.clone();
        big.role = Some(RoleView { text: "x".repeat(SNAPSHOT_MAX_BYTES), edited: true });
        s.apply_synced_topic_page_snapshot(&big, 1_000).unwrap();
        assert!(s.topic_page_snapshot(&t.key).unwrap().is_none());
        let mut ok = snap;
        ok.built_at_unix_ms = 1_000 + crate::SYNCED_RUN_MAX_FUTURE_MS;
        s.apply_synced_topic_page_snapshot(&ok, 1_000).unwrap();
        assert!(s.topic_page_snapshot(&t.key).unwrap().is_some());
    }
}
