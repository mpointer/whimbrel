//! The topic page: one seed topic's role, key date or step to lead with, next meeting, and
//! steps (You owe, Owed to you, Done, Older); the topic list; and the lookups that tie a
//! meeting, a calendar event or free text to a topic.

use std::collections::{HashMap, HashSet};

use chrono::{Local, TimeZone};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use super::aliases::{alias_hits, live_seeds};
use super::entities::parse_addresses;
use super::index::calendar_series_key;
use super::read::{
    members, next_event, next_event_in, open_target, series_topic, upcoming_events, OpenTarget,
    TopicMember,
};
use super::steps::{hash_completed, primary_topic_for_source};
use crate::Store;

const DAY_MS: i64 = 86_400_000;
const OLDER_DAYS: i64 = 60;
const RECENT: usize = 20;
/// An edited topic within this distance (in signal space) lends its wording.
const NEAREST: f64 = 0.25;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TopicRef {
    pub topic_id: i64,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TopicListEntry {
    pub topic: TopicRef,
    pub kind: Option<String>,
    pub next_meeting_unix_ms: Option<i64>,
    pub open_mine: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepView {
    pub key: String,
    pub text: String,
    pub direction: String,
    pub source_kind: String,
    pub source_title: String,
    pub from_screen: bool,
    pub open: Option<OpenTarget>,
    pub asked_at_unix_ms: i64,
    pub due_date: Option<String>,
    pub status: String,
    pub confirmed: bool,
    pub note: Option<String>,
    pub closed_by: Option<String>,
    /// When the step row last changed (for a done step, about when it was done).
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SummaryLead {
    Date {
        label: String,
        date: String,
    },
    Step {
        text: String,
        due_date: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextMeeting {
    pub title: String,
    pub start_unix_ms: i64,
    /// (email domain, attendees), largest first.
    /// Attendees other than Mike, by email domain.
    pub attendees_by_org: Vec<(String, i64)>,
    /// Attendees whose address is exactly one of Mike's (case-insensitive).
    pub me: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleView {
    pub text: String,
    pub edited: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TopicPage {
    pub topic: TopicRef,
    pub kind: Option<String>,
    pub role: RoleView,
    pub lead: Option<SummaryLead>,
    pub next_meeting: Option<NextMeeting>,
    pub you_owe: Vec<StepView>,
    pub owed_to_you: Vec<StepView>,
    pub done: Vec<StepView>,
    pub older: Vec<StepView>,
    /// The first 20 members, newest first.
    pub recent: Vec<TopicMember>,
    pub recent_total: i64,
}

/// A role edit as it travels between devices (`topic_role` sync rows).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicRolePayload {
    pub topic_key: String,
    pub text: String,
    pub signals: Vec<f64>,
    pub deleted: bool,
    pub updated_at_unix_ms: i64,
}

fn role_payload(conn: &Connection, topic_key: &str) -> rusqlite::Result<Option<TopicRolePayload>> {
    conn.query_row(
        "SELECT topic_key, text, signals, deleted, updated_at_unix_ms FROM topic_role WHERE topic_key = ?1",
        [topic_key],
        |r| {
            let signals: String = r.get(2)?;
            Ok(TopicRolePayload {
                topic_key: r.get(0)?,
                text: r.get(1)?,
                signals: serde_json::from_str(&signals).unwrap_or_default(),
                deleted: r.get(3)?,
                updated_at_unix_ms: r.get(4)?,
            })
        },
    )
    .optional()
}

/// Queues the stored role edit for `topic_key` for sync, in the caller's transaction.
pub(crate) fn enqueue_role_row(
    store: &Store,
    tx: &Transaction<'_>,
    topic_key: &str,
) -> rusqlite::Result<()> {
    match role_payload(tx, topic_key)? {
        Some(p) => store.enqueue_topic_page_outbox(
            tx,
            crate::OUTBOX_ROW_TYPE_TOPIC_ROLE,
            p.topic_key.clone(),
            &p,
        ),
        None => Ok(()),
    }
}

fn local_date(ms: i64) -> String {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.date_naive().to_string())
        .unwrap_or_default()
}

/// The live seed topic `topic_id` is (or was merged into), with its kind.
fn live_topic(
    conn: &Connection,
    topic_id: i64,
) -> rusqlite::Result<Option<(TopicRef, Option<String>)>> {
    conn.query_row(
        "SELECT t.id, s.key, t.name, s.kind FROM topic t JOIN topic_seed s ON s.topic_id = t.id \
         WHERE t.id = (SELECT COALESCE(merged_into, id) FROM topic WHERE id = ?1) \
           AND t.merged_into IS NULL AND s.deleted = 0 AND s.merged_into_key IS NULL",
        [topic_id],
        |r| {
            Ok((
                TopicRef {
                    topic_id: r.get(0)?,
                    key: r.get(1)?,
                    name: r.get(2)?,
                },
                r.get(3)?,
            ))
        },
    )
    .optional()
}

fn topic_ref(conn: &Connection, topic_id: i64) -> rusqlite::Result<Option<TopicRef>> {
    Ok(live_topic(conn, topic_id)?.map(|(t, _)| t))
}

/// `key` plus every seed key whose `merged_into_key` chain ends at it: their steps, dates
/// and roles belong to this topic.
fn merged_keys(conn: &Connection, key: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT key, merged_into_key FROM topic_seed WHERE merged_into_key IS NOT NULL")?;
    let links: HashMap<String, String> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = vec![key.to_string()];
    for start in links.keys() {
        let mut at = start;
        // Bounded: a merge cycle (opposite merges on two devices) is broken at seed time.
        for _ in 0..links.len() {
            match links.get(at) {
                Some(next) => at = next,
                None => break,
            }
        }
        if at == key {
            out.push(start.clone());
        }
    }
    Ok(out)
}

fn keys_json(keys: &[String]) -> String {
    serde_json::to_string(keys).unwrap_or_default()
}

/// The topic's visible steps (no `none` direction, nothing dismissed), newest first. An open
/// meeting step whose action item is checked off here reads as done.
fn steps(conn: &Connection, keys: &[String]) -> rusqlite::Result<Vec<StepView>> {
    let mut stmt = conn.prepare(
        "SELECT st.key, st.text, st.direction, st.source_kind, st.source_title, st.asked_at_unix_ms, \
           st.due_date, st.status, st.confirmed, st.note, st.closed_by, st.source_id, s.kind, s.ref_id, s.stub, \
           st.updated_at_unix_ms \
         FROM topic_step st LEFT JOIN topic_source s ON s.id = st.source_id \
         WHERE st.topic_key IN (SELECT value FROM json_each(?1)) \
           AND st.direction != 'none' AND st.status != 'dismissed' \
         ORDER BY st.asked_at_unix_ms DESC, st.key",
    )?;
    type Src = (Option<i64>, Option<String>, Option<i64>, Option<bool>);
    let rows: Vec<(StepView, Src)> = stmt
        .query_map([keys_json(keys)], |r| {
            let kind: String = r.get(3)?;
            Ok((
                StepView {
                    key: r.get(0)?,
                    text: r.get(1)?,
                    direction: r.get(2)?,
                    from_screen: kind == "episode",
                    source_kind: kind,
                    source_title: r.get(4)?,
                    open: None,
                    asked_at_unix_ms: r.get(5)?,
                    due_date: r.get(6)?,
                    status: r.get(7)?,
                    confirmed: r.get(8)?,
                    note: r.get(9)?,
                    closed_by: r.get(10)?,
                    updated_at_unix_ms: r.get(15)?,
                },
                (r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (mut v, (sid, kind, ref_id, stub)) in rows {
        // A step whose source is not on this device (or is a stub) has no link.
        if let (Some(kind), Some(false)) = (kind, stub) {
            v.open = open_target(conn, &kind, ref_id)?;
        }
        if v.status == "open" && v.source_kind == "meeting" {
            if let (Some(sid), Some((_, hash))) = (sid, v.key.rsplit_once("#h")) {
                if hash_completed(conn, sid, hash)? {
                    v.status = "done".into();
                }
            }
        }
        out.push(v);
    }
    Ok(out)
}

/// The topic's newest member is over 60 days old: its open steps are "Older". A topic with
/// no members here yet (steps pulled before indexing) is not.
fn stale(conn: &Connection, topic_id: i64, now_ms: i64) -> rusqlite::Result<bool> {
    let last: Option<i64> = conn.query_row(
        "SELECT MAX(s.ts_unix_ms) FROM topic_member m JOIN topic_source s ON s.id = m.source_id \
         WHERE m.topic_id = ?1 AND m.origin != 'removed'",
        [topic_id],
        |r| r.get(0),
    )?;
    Ok(last.is_some_and(|t| now_ms - t > OLDER_DAYS * DAY_MS))
}

/// (email domain, attendees) of a calendar item, largest first, then by domain, leaving out
/// attendees whose address is one of `mine` (lowercase); those are counted in the second value.
fn attendees_by_org(
    conn: &Connection,
    item_id: Option<i64>,
    mine: &HashSet<String>,
) -> rusqlite::Result<(Vec<(String, i64)>, i64)> {
    let actors: Option<String> = match item_id {
        Some(id) => conn
            .query_row("SELECT actors FROM items WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .flatten(),
        None => None,
    };
    let mut counts: HashMap<String, i64> = HashMap::new();
    let mut me = 0;
    let list: Vec<String> = actors
        .and_then(|a| serde_json::from_str(&a).ok())
        .unwrap_or_default();
    for a in list.iter().flat_map(|a| parse_addresses(a)) {
        if mine.contains(&a.email.to_lowercase()) {
            me += 1;
        } else if let Some((_, domain)) = a.email.rsplit_once('@') {
            *counts.entry(domain.to_lowercase()).or_default() += 1;
        }
    }
    let mut out: Vec<(String, i64)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok((out, me))
}

/// One key date in `topic_extraction.dates`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyDate {
    pub date: String,
    pub label: String,
}

/// The earliest of the topic's key dates (from today on) and its open mine steps' due dates;
/// else the newest open mine step. The next meeting is shown beside it, not as a candidate.
fn lead(
    conn: &Connection,
    keys: &[String],
    you_owe: &[StepView],
    now_ms: i64,
) -> rusqlite::Result<Option<SummaryLead>> {
    let today = local_date(now_ms);
    let mut stmt = conn.prepare(
        "SELECT dates FROM topic_extraction WHERE topic_key IN (SELECT value FROM json_each(?1))",
    )?;
    let mut dates: Vec<(String, String)> = Vec::new();
    for json in stmt.query_map([keys_json(keys)], |r| r.get::<_, String>(0))? {
        let found: Vec<KeyDate> = serde_json::from_str(&json?).unwrap_or_default();
        dates.extend(found.into_iter().map(|d| (d.date, d.label)));
    }
    let date = dates.into_iter().filter(|(d, _)| *d >= today).min();
    let due = you_owe
        .iter()
        .filter_map(|s| s.due_date.as_ref().map(|d| (d, s)))
        .min_by(|a, b| a.0.cmp(b.0));
    let step = |s: &StepView| SummaryLead::Step {
        text: s.text.clone(),
        due_date: s.due_date.clone(),
    };
    Ok(match (date, due) {
        (Some((d, _)), Some((due, s))) if *due < d => Some(step(s)),
        (Some((date, label)), _) => Some(SummaryLead::Date { label, date }),
        (None, Some((_, s))) => Some(step(s)),
        // `you_owe` is newest first.
        (None, None) => you_owe.first().map(step),
    })
}

/// `[share of topic emails from me, share with me in To, min(mine steps, 5) / 5,
/// share of topic meetings with a mine step]`.
fn role_signals(
    conn: &Connection,
    topic_id: i64,
    keys: &[String],
    me: &HashSet<String>,
) -> rusqlite::Result<[f64; 4]> {
    let mut stmt = conn.prepare(
        "SELECT i.actors FROM topic_member m JOIN topic_source s ON s.id = m.source_id \
           JOIN items i ON i.id = s.ref_id \
         WHERE m.topic_id = ?1 AND m.origin != 'removed' AND s.kind = 'email'",
    )?;
    let emails: Vec<Option<String>> = stmt
        .query_map([topic_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mine = |xs: &[String]| {
        xs.iter()
            .flat_map(|x| parse_addresses(x))
            .any(|a| me.contains(&a.email.to_lowercase()))
    };
    let (mut from, mut to) = (0, 0);
    for a in &emails {
        // actors: [From, To...]
        let list: Vec<String> = a
            .as_deref()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or_default();
        if mine(&list[..list.len().min(1)]) {
            from += 1;
        }
        if list.len() > 1 && mine(&list[1..]) {
            to += 1;
        }
    }
    let share = |n: i64, d: i64| if d == 0 { 0.0 } else { n as f64 / d as f64 };
    let mine_steps: i64 = conn.query_row(
        "SELECT COUNT(*) FROM topic_step WHERE topic_key IN (SELECT value FROM json_each(?1)) \
         AND direction = 'mine' AND status != 'dismissed'",
        [keys_json(keys)],
        |r| r.get(0),
    )?;
    let (meetings, with_mine): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(EXISTS (SELECT 1 FROM topic_step st WHERE st.source_id = s.id \
           AND st.direction = 'mine' AND st.status != 'dismissed')), 0) \
         FROM topic_member m JOIN topic_source s ON s.id = m.source_id \
         WHERE m.topic_id = ?1 AND m.origin != 'removed' AND s.kind = 'meeting'",
        [topic_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let n = emails.len() as i64;
    Ok([
        share(from, n),
        share(to, n),
        mine_steps.min(5) as f64 / 5.0,
        share(with_mine, meetings),
    ])
}

/// Mike's edit for the topic; else the wording of the nearest edited topic (within 0.25);
/// else a rule on the signals.
fn infer_role(conn: &Connection, key: &str, s: [f64; 4]) -> rusqlite::Result<RoleView> {
    let mut stmt = conn.prepare(
        "SELECT topic_key, text, signals FROM topic_role WHERE deleted = 0 ORDER BY topic_key",
    )?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if let Some((_, text, _)) = rows.iter().find(|r| r.0 == key) {
        return Ok(RoleView {
            text: text.clone(),
            edited: true,
        });
    }
    let nearest = rows
        .iter()
        .filter_map(|(_, text, sig)| {
            let v: [f64; 4] = serde_json::from_str(sig).ok()?;
            let d = v
                .iter()
                .zip(s)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt();
            (d <= NEAREST).then_some((d, text))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let text = match nearest {
        Some((_, t)) => t.clone(),
        None if s[0] >= 0.3 || s[2] >= 0.4 => "lead".into(),
        None if s[1] >= 0.3 || s[3] > 0.0 => "supporting".into(),
        None => "kept informed".into(),
    };
    Ok(RoleView {
        text,
        edited: false,
    })
}

impl Store {
    /// Mike's addresses, lowercased, from `self_identity`.
    fn my_addresses(&self) -> rusqlite::Result<HashSet<String>> {
        Ok(self
            .self_identity()?
            .into_iter()
            .filter(|x| x.contains('@'))
            .map(|x| x.to_lowercase())
            .collect())
    }

    /// The primary topic of the meeting's source.
    pub fn topic_for_meeting(&self, meeting_id: i64) -> rusqlite::Result<Option<TopicRef>> {
        let sid: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM topic_source WHERE kind = 'meeting' AND ref_id = ?1",
                [meeting_id],
                |r| r.get(0),
            )
            .optional()?;
        match sid {
            Some(sid) => match primary_topic_for_source(&self.conn, sid)? {
                Some((id, _)) => topic_ref(&self.conn, id),
                None => Ok(None),
            },
            None => Ok(None),
        }
    }

    /// A calendar event's topic: its series' primary topic (Apple or Google ids), else the
    /// first seed topic its title names.
    pub fn topic_for_calendar_event(
        &self,
        event_id: &str,
        title: &str,
    ) -> rusqlite::Result<Option<TopicRef>> {
        for source in ["apple_calendar", "google_calendar"] {
            if let Some((id, _)) = series_topic(&self.conn, &calendar_series_key(source, event_id))?
            {
                return topic_ref(&self.conn, id);
            }
        }
        Ok(self.topics_named_in(title)?.into_iter().next())
    }

    /// Live seed topics whose aliases appear in `text`.
    pub fn topics_named_in(&self, text: &str) -> rusqlite::Result<Vec<TopicRef>> {
        let mut out = Vec::new();
        for id in alias_hits(text, &live_seeds(&self.conn)?) {
            out.extend(topic_ref(&self.conn, id)?);
        }
        Ok(out)
    }

    /// Live seed topics by name, with what the list's hint needs.
    pub fn topic_page_list(&self, now_ms: i64) -> rusqlite::Result<Vec<TopicListEntry>> {
        let conn = &self.conn;
        let mut stmt = conn.prepare(
            "SELECT t.id, s.key, t.name, s.kind FROM topic_seed s JOIN topic t ON t.id = s.topic_id \
             WHERE s.deleted = 0 AND s.merged_into_key IS NULL AND t.merged_into IS NULL \
             ORDER BY t.name COLLATE NOCASE, t.id",
        )?;
        let rows: Vec<(TopicRef, Option<String>)> = stmt
            .query_map([], |r| {
                Ok((
                    TopicRef {
                        topic_id: r.get(0)?,
                        key: r.get(1)?,
                        name: r.get(2)?,
                    },
                    r.get(3)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::with_capacity(rows.len());
        // Read the coming events once for every topic, not once per topic.
        let events = upcoming_events(conn, now_ms)?;
        for (topic, kind) in rows {
            // As the page's "You owe": open mine steps of a topic active in the last 60 days.
            let open_mine = if stale(conn, topic.topic_id, now_ms)? {
                0
            } else {
                steps(conn, &merged_keys(conn, &topic.key)?)?
                    .iter()
                    .filter(|s| s.status == "open" && s.direction == "mine")
                    .count() as i64
            };
            out.push(TopicListEntry {
                next_meeting_unix_ms: next_event_in(conn, topic.topic_id, &events)?
                    .map(|e| e.ts_unix_ms),
                topic,
                kind,
                open_mine,
            });
        }
        Ok(out)
    }

    /// `None` unless `topic_id` is (or was merged into) a live seed topic.
    pub fn topic_page(&self, topic_id: i64, now_ms: i64) -> rusqlite::Result<Option<TopicPage>> {
        let conn = &self.conn;
        let Some((topic, kind)) = live_topic(conn, topic_id)? else {
            return Ok(None);
        };
        let keys = merged_keys(conn, &topic.key)?;
        let older_only = stale(conn, topic.topic_id, now_ms)?;
        let (mut you_owe, mut owed_to_you, mut done, mut older) = (vec![], vec![], vec![], vec![]);
        for s in steps(conn, &keys)? {
            match (s.status.as_str(), s.direction.as_str()) {
                ("done", _) => done.push(s),
                _ if older_only => older.push(s),
                (_, "mine") => you_owe.push(s),
                _ => owed_to_you.push(s),
            }
        }
        let mine = self.my_addresses()?;
        let next_meeting = match next_event(conn, topic.topic_id, now_ms)? {
            Some(ev) => {
                let (attendees_by_org, me) = attendees_by_org(conn, ev.ref_id, &mine)?;
                Some(NextMeeting {
                    title: ev.title,
                    start_unix_ms: ev.ts_unix_ms,
                    attendees_by_org,
                    me,
                })
            }
            None => None,
        };
        let lead = lead(conn, &keys, &you_owe, now_ms)?;
        let signals = role_signals(conn, topic.topic_id, &keys, &mine)?;
        let role = infer_role(conn, &topic.key, signals)?;
        let members = members(conn, topic.topic_id)?;
        Ok(Some(TopicPage {
            recent_total: members.len() as i64,
            recent: members.into_iter().take(RECENT).collect(),
            role,
            lead,
            next_meeting,
            you_owe,
            owed_to_you,
            done,
            older,
            topic,
            kind,
        }))
    }

    /// Stores Mike's wording of his role with the topic's current signals (which is how it
    /// teaches the nearest topics). Empty text goes back to inferred; the row stays so the
    /// latest write wins across devices.
    pub fn set_topic_role(&self, topic_key: &str, text: &str) -> rusqlite::Result<()> {
        let conn = &self.conn;
        let topic_id: i64 = conn.query_row(
            "SELECT topic_id FROM topic_seed WHERE key = ?1",
            [topic_key],
            |r| r.get(0),
        )?;
        let signals = role_signals(
            conn,
            topic_id,
            &merged_keys(conn, topic_key)?,
            &self.my_addresses()?,
        )?;
        let text = text.trim();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO topic_role (topic_key, text, signals, deleted, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(topic_key) DO UPDATE SET text = excluded.text, signals = excluded.signals, \
             deleted = excluded.deleted, updated_at_unix_ms = MAX(excluded.updated_at_unix_ms, updated_at_unix_ms + 1)",
            params![
                topic_key,
                text,
                serde_json::to_string(&signals).unwrap_or_default(),
                text.is_empty(),
                crate::now_unix_ms()
            ],
        )?;
        enqueue_role_row(self, &tx, topic_key)?;
        tx.commit()
    }

    /// Applies a role edit pulled from another device: last write wins (`incoming_wins`).
    /// Never enqueued.
    pub fn apply_synced_topic_role(&self, p: &TopicRolePayload) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let wins = match role_payload(&tx, &p.topic_key)? {
            Some(l) => super::incoming_wins(p, p.updated_at_unix_ms, &l, l.updated_at_unix_ms),
            None => true,
        };
        if wins {
            tx.execute(
                "INSERT OR REPLACE INTO topic_role (topic_key, text, signals, deleted, updated_at_unix_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    p.topic_key,
                    p.text,
                    serde_json::to_string(&p.signals).unwrap_or_default(),
                    p.deleted,
                    p.updated_at_unix_ms
                ],
            )?;
        }
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-08 12:00 UTC: the same calendar day in every zone from -11 to +11.
    const NOW: i64 = 1_794_139_200_000;

    fn sql(s: &Store, q: &str, p: impl rusqlite::Params) -> i64 {
        s.conn.execute(q, p).unwrap();
        s.conn.last_insert_rowid()
    }

    fn seed(s: &Store, name: &str) -> TopicRef {
        let t = s.create_seed_topic(name, &[], None).unwrap();
        TopicRef {
            topic_id: t.topic_id,
            key: t.key,
            name: t.name,
        }
    }

    /// A topic_source of `kind` filed on `topic` with `origin`.
    fn filed(s: &Store, topic: i64, kind: &str, ref_id: Option<i64>, ts: i64, origin: &str) -> i64 {
        let sid = sql(
            s,
            "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, ?3, 't', 0)",
            params![kind, ref_id, ts],
        );
        sql(
            s,
            "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, ?3)",
            params![topic, sid, origin],
        );
        sid
    }

    fn step(s: &Store, key: &str, topic_key: &str, dir: &str, status: &str, sid: Option<i64>) {
        sql(
            s,
            "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, \
             asked_at_unix_ms, status, updated_at_unix_ms, source_id) VALUES (?1, ?2, 'email:x', 'email', 'SOW', ?1, ?3, ?4, ?5, 0, ?6)",
            params![key, topic_key, dir, NOW - DAY_MS, status, sid],
        );
    }

    fn keys(v: &[StepView]) -> Vec<&str> {
        v.iter().map(|s| s.key.as_str()).collect()
    }

    fn event(s: &Store, ext: &str, ts: i64, title: &str, actors: &[&str]) -> i64 {
        sql(
            s,
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) \
             VALUES ('d', 'calendar_event', 'google_calendar', ?1, ?2, ?3, '', ?4)",
            params![ts, serde_json::to_string(actors).unwrap(), title, ext],
        )
    }

    #[test]
    fn page_splits_you_owe_owed_done_and_older() {
        let s = Store::open_in_memory().unwrap();
        let t = seed(&s, "Maximus");
        let sid = filed(&s, t.topic_id, "email", None, NOW - DAY_MS, "seed");
        step(&s, "mine", &t.key, "mine", "open", Some(sid));
        step(&s, "owed", &t.key, "owed", "open", Some(sid));
        step(&s, "done", &t.key, "mine", "done", Some(sid));
        step(&s, "none", &t.key, "none", "open", Some(sid));
        step(&s, "gone", &t.key, "mine", "dismissed", Some(sid));
        // An open meeting step whose action item was checked off here is done.
        let mid = sql(&s, "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status) VALUES ('d', 1, 'Sync', '[]', 'manual', 'done')", []);
        sql(&s, "INSERT INTO meeting_action_item (meeting_id, text, completed, order_index, source, created_unix_ms, updated_unix_ms) VALUES (?1, 'Send the SOW.', 1, 0, 'llm', 0, 0)", [mid]);
        let msid = filed(
            &s,
            t.topic_id,
            "meeting",
            Some(mid),
            NOW - 2 * DAY_MS,
            "seed",
        );
        let mkey = format!(
            "meeting:x#h{}",
            super::super::step_text::step_hash("Send the SOW.")
        );
        sql(
            &s,
            "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, \
             asked_at_unix_ms, status, updated_at_unix_ms, source_id) VALUES (?1, ?2, 'meeting:x', 'meeting', 'Sync', 'Send the SOW.', 'mine', 0, 'open', 0, ?3)",
            params![mkey, t.key, msid],
        );

        let p = s.topic_page(t.topic_id, NOW).unwrap().unwrap();
        assert_eq!(keys(&p.you_owe), vec!["mine"]);
        assert_eq!(keys(&p.owed_to_you), vec!["owed"]);
        assert_eq!(keys(&p.done), vec!["done", mkey.as_str()]);
        assert!(p.older.is_empty());
        assert_eq!(p.recent_total, 2);

        // Newest member 61 days old: open steps are Older; done stays Done.
        let old = seed(&s, "Beta");
        let osid = filed(&s, old.topic_id, "email", None, NOW - 61 * DAY_MS, "seed");
        step(&s, "o_mine", &old.key, "mine", "open", Some(osid));
        step(&s, "o_owed", &old.key, "owed", "open", Some(osid));
        step(&s, "o_done", &old.key, "mine", "done", Some(osid));
        let p = s.topic_page(old.topic_id, NOW).unwrap().unwrap();
        assert!(p.you_owe.is_empty() && p.owed_to_you.is_empty());
        assert_eq!(keys(&p.older), vec!["o_mine", "o_owed"]);
        assert_eq!(keys(&p.done), vec!["o_done"]);
    }

    #[test]
    fn merged_seed_steps_show_on_the_survivor() {
        let s = Store::open_in_memory().unwrap();
        let keep = seed(&s, "Maximus");
        let gone = seed(&s, "Max");
        let gone2 = seed(&s, "MX program");
        // `jev` members: a merge re-matches `seed` members, dropping them until the next pass.
        filed(&s, keep.topic_id, "email", None, NOW - DAY_MS, "jev");
        filed(&s, gone.topic_id, "email", None, NOW - DAY_MS, "jev");
        filed(&s, gone2.topic_id, "email", None, NOW - DAY_MS, "jev");
        step(&s, "a", &gone.key, "mine", "open", None);
        step(&s, "b", &gone2.key, "owed", "open", None);
        // gone2 -> gone, then gone -> keep: a chain of two.
        s.merge_topics(gone.topic_id, gone2.topic_id).unwrap();
        s.merge_topics(keep.topic_id, gone.topic_id).unwrap();
        let p = s.topic_page(keep.topic_id, NOW).unwrap().unwrap();
        assert_eq!(keys(&p.you_owe), vec!["a"]);
        assert_eq!(keys(&p.owed_to_you), vec!["b"]);
        // A merged id opens the survivor.
        let p = s.topic_page(gone.topic_id, NOW).unwrap().unwrap();
        assert_eq!(p.topic.key, keep.key);
        let list = s.topic_page_list(NOW).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].open_mine, 1);
    }

    #[test]
    fn step_whose_source_is_gone_renders_without_link() {
        let s = Store::open_in_memory().unwrap();
        let t = seed(&s, "Maximus");
        filed(&s, t.topic_id, "email", None, NOW - DAY_MS, "seed");
        // A step pulled from another device whose source is not here.
        step(&s, "pulled", &t.key, "mine", "open", None);
        sql(
            &s,
            "UPDATE topic_step SET source_kind = 'episode' WHERE key = 'pulled'",
            [],
        );
        let p = s.topic_page(t.topic_id, NOW).unwrap().unwrap();
        assert_eq!(keys(&p.you_owe), vec!["pulled"]);
        assert_eq!(p.you_owe[0].open, None);
        assert!(p.you_owe[0].from_screen);
        assert_eq!(p.you_owe[0].source_title, "SOW");
        // Pulled before this device has any member of the topic: still "You owe".
        let u = seed(&s, "Beta");
        step(&s, "early", &u.key, "mine", "open", None);
        let p = s.topic_page(u.topic_id, NOW).unwrap().unwrap();
        assert_eq!(keys(&p.you_owe), vec!["early"]);
    }

    fn mail(s: &Store, topic: i64, from: &str, to: &str) {
        let it = sql(
            s,
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
             VALUES ('d', 'email', 'gmail', ?1, json_array(?2, ?3), 'News', 'b', '{}', ?2 || ?3 || random())",
            params![NOW - DAY_MS, from, to],
        );
        filed(s, topic, "email", Some(it), NOW - DAY_MS, "seed");
    }

    #[test]
    fn email_with_me_in_neither_from_nor_to_infers_kept_informed() {
        let s = Store::open_in_memory().unwrap();
        s.insert_connector_account("gmail", "mike@acme.com", true, 0, 0)
            .unwrap();
        let t = seed(&s, "Maximus");
        mail(&s, t.topic_id, "Pat <pat@x.com>", "list@x.com, ann@x.com");
        mail(&s, t.topic_id, "Pat <pat@x.com>", "team@x.com");
        let p = s.topic_page(t.topic_id, NOW).unwrap().unwrap();
        assert_eq!(
            p.role,
            RoleView {
                text: "kept informed".into(),
                edited: false
            }
        );
        // Mike sends one of three: a third from me is "lead".
        mail(&s, t.topic_id, "Mike <MIKE@acme.com>", "pat@x.com");
        let p = s.topic_page(t.topic_id, NOW).unwrap().unwrap();
        assert_eq!(p.role.text, "lead");
        // Mike in To on another topic: "supporting".
        let u = seed(&s, "Beta");
        mail(&s, u.topic_id, "Pat <pat@x.com>", "x@x.com, mike@acme.com");
        let p = s.topic_page(u.topic_id, NOW).unwrap().unwrap();
        assert_eq!(p.role.text, "supporting");
    }

    #[test]
    fn lead_is_the_nearest_date_or_due() {
        let s = Store::open_in_memory().unwrap();
        let t = seed(&s, "Maximus");
        filed(&s, t.topic_id, "email", None, NOW - DAY_MS, "seed");
        sql(
            &s,
            "INSERT INTO topic_extraction (source_key, topic_key, dates, model, extracted_at_unix_ms, updated_at_unix_ms) \
             VALUES ('email:x', ?1, ?2, 'm', 0, 0)",
            params![
                t.key,
                r#"[{"date":"2026-11-01","label":"kickoff"},{"date":"2026-11-14","label":"SOW due"}]"#
            ],
        );
        let nov18 = NOW + 10 * DAY_MS;
        event(&s, "q1", nov18, "Maximus QBR", &[]);
        step(&s, "later", &t.key, "mine", "open", None);
        sql(
            &s,
            "UPDATE topic_step SET due_date = '2026-11-20' WHERE key = 'later'",
            [],
        );
        let p = s.topic_page(t.topic_id, NOW).unwrap().unwrap();
        assert_eq!(
            p.lead,
            Some(SummaryLead::Date {
                label: "SOW due".into(),
                date: "2026-11-14".into()
            })
        );
        let nm = p.next_meeting.unwrap();
        assert_eq!(
            (nm.title.as_str(), nm.start_unix_ms),
            ("Maximus QBR", nov18)
        );
        // A due date before the key date leads.
        sql(
            &s,
            "UPDATE topic_step SET due_date = '2026-11-10' WHERE key = 'later'",
            [],
        );
        let p = s.topic_page(t.topic_id, NOW).unwrap().unwrap();
        assert_eq!(
            p.lead,
            Some(SummaryLead::Step {
                text: "later".into(),
                due_date: Some("2026-11-10".into())
            })
        );
        // No dates at all: the newest open mine step.
        let u = seed(&s, "Beta");
        filed(&s, u.topic_id, "email", None, NOW - DAY_MS, "seed");
        step(&s, "u1", &u.key, "mine", "open", None);
        let p = s.topic_page(u.topic_id, NOW).unwrap().unwrap();
        assert_eq!(
            p.lead,
            Some(SummaryLead::Step {
                text: "u1".into(),
                due_date: None
            })
        );
    }

    #[test]
    fn list_next_meetings_match_each_topic_page() {
        let s = Store::open_in_memory().unwrap();
        let a = seed(&s, "Maximus");
        let b = seed(&s, "Greensky");
        let c = seed(&s, "Realti");
        filed(&s, a.topic_id, "email", None, NOW - DAY_MS, "seed");
        filed(&s, b.topic_id, "email", None, NOW - DAY_MS, "seed");
        event(&s, "m1", NOW + 2 * DAY_MS, "Maximus QBR", &[]);
        event(&s, "g1", NOW + 5 * DAY_MS, "Greensky sync", &[]);
        event(&s, "x1", NOW + DAY_MS, "Lunch", &[]);
        let list = s.topic_page_list(NOW).unwrap();
        for t in [&a, &b, &c] {
            let from_list = list
                .iter()
                .find(|e| e.topic.topic_id == t.topic_id)
                .unwrap()
                .next_meeting_unix_ms;
            let from_page = s
                .topic_page(t.topic_id, NOW)
                .unwrap()
                .unwrap()
                .next_meeting
                .map(|m| m.start_unix_ms);
            assert_eq!(from_list, from_page, "{}", t.name);
        }
        let ts = |t: &TopicRef| {
            list.iter()
                .find(|e| e.topic.topic_id == t.topic_id)
                .unwrap()
                .next_meeting_unix_ms
        };
        assert_eq!(ts(&a), Some(NOW + 2 * DAY_MS));
        assert_eq!(ts(&b), Some(NOW + 5 * DAY_MS));
        assert_eq!(ts(&c), None);
    }

    #[test]
    fn future_event_matches_by_series_then_alias() {
        let s = Store::open_in_memory().unwrap();
        let t = seed(&s, "Maximus");
        // A past occurrence of a weekly series, filed on Maximus (title does not name it).
        let past = event(
            &s,
            "abc_20261105T150000Z#1",
            NOW - 3 * DAY_MS,
            "Weekly sync",
            &[],
        );
        let sid = filed(
            &s,
            t.topic_id,
            "calendar_event",
            Some(past),
            NOW - 3 * DAY_MS,
            "jev",
        );
        sql(
            &s,
            "INSERT INTO topic_series (series_key, source_id) VALUES (?1, ?2)",
            params![
                calendar_series_key("google_calendar", "abc_20261105T150000Z#1"),
                sid
            ],
        );
        let found = s
            .topic_for_calendar_event("abc_20261112T150000Z#2", "Weekly sync")
            .unwrap();
        assert_eq!(found, Some(t.clone()));
        assert_eq!(
            s.topic_for_calendar_event("one-off", "Maximus QBR")
                .unwrap(),
            Some(t.clone())
        );
        assert_eq!(
            s.topic_for_calendar_event("one-off", "Lunch").unwrap(),
            None
        );
        assert_eq!(
            s.topics_named_in("re: maximus pricing").unwrap(),
            vec![t.clone()]
        );
        // The next occurrence is the topic's next meeting, with no attendee in common.
        let next = event(
            &s,
            "abc_20261112T150000Z#2",
            NOW + 4 * DAY_MS,
            "Weekly sync",
            &[],
        );
        let _ = next;
        let p = s.topic_page(t.topic_id, NOW).unwrap().unwrap();
        assert_eq!(p.next_meeting.unwrap().start_unix_ms, NOW + 4 * DAY_MS);
        let list = s.topic_page_list(NOW).unwrap();
        assert_eq!(list[0].next_meeting_unix_ms, Some(NOW + 4 * DAY_MS));
    }

    #[test]
    fn attendees_split_by_domain() {
        let s = Store::open_in_memory().unwrap();
        let t = seed(&s, "Maximus");
        event(
            &s,
            "q1",
            NOW + DAY_MS,
            "Maximus QBR",
            &[
                "Cy <cy@gov.org>",
                "Ann <ann@acme.com>",
                "bob@ACME.com",
                "Room 4",
            ],
        );
        let nm = s
            .topic_page(t.topic_id, NOW)
            .unwrap()
            .unwrap()
            .next_meeting
            .unwrap();
        assert_eq!(
            nm.attendees_by_org,
            vec![("acme.com".to_string(), 2), ("gov.org".to_string(), 1)]
        );
    }

    #[test]
    fn next_meeting_counts_me_by_exact_address() {
        let s = Store::open_in_memory().unwrap();
        s.set_self_names(&["Mike".into(), "Mike@Acme.com".into()])
            .unwrap();
        let t = seed(&s, "Maximus");
        event(
            &s,
            "q1",
            NOW + DAY_MS,
            "Maximus QBR",
            &["Mike <mike@ACME.com>", "Ann <ann@acme.com>", "cy@gov.org"],
        );
        let nm = s
            .topic_page(t.topic_id, NOW)
            .unwrap()
            .unwrap()
            .next_meeting
            .unwrap();
        // A coworker on Mike's domain is not "me".
        assert_eq!(nm.me, 1);
        assert_eq!(
            nm.attendees_by_org,
            vec![("acme.com".to_string(), 1), ("gov.org".to_string(), 1)]
        );
    }

    #[test]
    fn role_edit_wins_and_teaches_the_nearest_topic() {
        let s = Store::open_in_memory().unwrap();
        let a = seed(&s, "Maximus");
        let b = seed(&s, "Beta");
        s.set_topic_role(&a.key, "lead consultant").unwrap();
        let ra = s.topic_page(a.topic_id, NOW).unwrap().unwrap().role;
        assert_eq!(
            ra,
            RoleView {
                text: "lead consultant".into(),
                edited: true
            }
        );
        let rb = s.topic_page(b.topic_id, NOW).unwrap().unwrap().role;
        assert_eq!(
            rb,
            RoleView {
                text: "lead consultant".into(),
                edited: false
            }
        );
        // Clearing the edit goes back to inferred; the row stays.
        s.set_topic_role(&a.key, " ").unwrap();
        let ra = s.topic_page(a.topic_id, NOW).unwrap().unwrap().role;
        assert_eq!(ra.text, "kept informed");
        assert!(!ra.edited);
        let n: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM topic_role WHERE deleted = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn primary_topic_prefers_seed_over_jev_over_auto() {
        let s = Store::open_in_memory().unwrap();
        let (a, j, sd) = (seed(&s, "Alpha"), seed(&s, "Juno"), seed(&s, "Sigma"));
        let mid = sql(&s, "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status) VALUES ('d', 1, 'Sync', '[]', 'manual', 'done')", []);
        let sid = filed(&s, a.topic_id, "meeting", Some(mid), 1, "auto");
        for (t, o) in [(&j, "jev"), (&sd, "seed")] {
            sql(
                &s,
                "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, ?3)",
                params![t.topic_id, sid, o],
            );
        }
        assert_eq!(s.topic_for_meeting(mid).unwrap(), Some(sd.clone()));
        sql(
            &s,
            "UPDATE topic_member SET origin = 'removed' WHERE topic_id = ?1",
            [sd.topic_id],
        );
        assert_eq!(s.topic_for_meeting(mid).unwrap(), Some(j.clone()));
        sql(
            &s,
            "UPDATE topic_member SET origin = 'removed' WHERE topic_id = ?1",
            [j.topic_id],
        );
        assert_eq!(s.topic_for_meeting(mid).unwrap(), Some(a));
        assert_eq!(s.topic_for_meeting(mid + 1).unwrap(), None);
    }
}
