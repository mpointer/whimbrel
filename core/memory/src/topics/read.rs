//! Naming inputs and the read surface for topics: what the naming job needs, the list of
//! topics, and one topic's detail ("where you left off" card plus members).
//! Every query skips merged topics (`merged_into IS NULL`; a merged id resolves to its
//! target in `topic_detail`) and counts only `origin != 'removed'` members.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::entities::parse_addresses;
use crate::{now_unix_ms, Store};

const DAY_MS: i64 = 86_400_000;
const ACTIVE_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicNameInput {
    pub id: i64,
    pub top_entities: Vec<String>,
    pub sample_titles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicSummary {
    pub id: i64,
    pub name: String,
    pub status_line: Option<String>,
    pub member_count: i64,
    pub last_member_ts_unix_ms: i64,
    pub top_people: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenTarget {
    /// "meeting-detail" | "thread-detail" | "capture-detail" | "episode-detail"
    pub window: String,
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicMember {
    /// `topic_source.id`; 0 for an upcoming calendar event that is not indexed yet.
    pub source_id: i64,
    pub kind: String,
    pub ref_id: Option<i64>,
    pub ts_unix_ms: i64,
    pub title: String,
    pub origin: String,
    pub stub: bool,
    pub open: Option<OpenTarget>,
}

/// A person or org entity mentioned by one non-removed member (source) of a topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicEntityMention {
    pub source_id: i64,
    pub entity_id: i64,
    /// `person` or `org`.
    pub kind: String,
    pub display: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LeftOff {
    pub last_email: Option<TopicMember>,
    pub last_meeting: Option<TopicMember>,
    pub last_episode: Option<TopicMember>,
    pub open_action_items: Vec<String>,
    pub next_event: Option<TopicMember>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicDetail {
    pub summary: TopicSummary,
    pub card: LeftOff,
    pub members: Vec<TopicMember>,
}

fn target(window: &str, id: i64) -> Option<OpenTarget> {
    Some(OpenTarget {
        window: window.into(),
        id,
    })
}

fn open_target(
    conn: &Connection,
    kind: &str,
    ref_id: Option<i64>,
) -> rusqlite::Result<Option<OpenTarget>> {
    let Some(r) = ref_id else { return Ok(None) };
    Ok(match kind {
        "meeting" => target("meeting-detail", r),
        "episode" => target("episode-detail", r),
        "snapshot" => conn
            .query_row("SELECT thread_id FROM items WHERE id = ?1", [r], |x| {
                x.get::<_, Option<i64>>(0)
            })
            .optional()?
            .flatten()
            .and_then(|m| target("meeting-detail", m)),
        "email" => conn
            .query_row(
                "SELECT t.id FROM items i JOIN threads t ON t.source = 'gmail' \
                   AND t.external_thread_id = json_extract(i.refs, '$.gmail_thread_id') \
                   AND t.account_email = json_extract(i.refs, '$.account_email') \
                 WHERE i.id = ?1",
                [r],
                |x| x.get::<_, i64>(0),
            )
            .optional()?
            .and_then(|t| target("thread-detail", t)),
        _ => None,
    })
}

fn top_people(conn: &Connection, topic_id: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT e.display FROM entity e \
           JOIN entity_mention em ON em.entity_id = e.id \
           JOIN topic_member m ON m.source_id = em.source_id AND m.topic_id = ?1 AND m.origin != 'removed' \
         WHERE e.kind = 'person' GROUP BY e.id ORDER BY SUM(em.weight) DESC, e.id LIMIT 3",
    )?;
    let rows = stmt.query_map([topic_id], |r| r.get(0))?.collect();
    rows
}

/// Live topics with their non-removed member counts. `id` limits to one topic;
/// `min_members`/`since` are the list filters.
fn summaries(
    conn: &Connection,
    id: Option<i64>,
    min_members: i64,
    since: i64,
    limit: usize,
) -> rusqlite::Result<Vec<TopicSummary>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.status_line, COUNT(s.id), COALESCE(MAX(s.ts_unix_ms), 0) AS last \
         FROM topic t \
           LEFT JOIN topic_member m ON m.topic_id = t.id AND m.origin != 'removed' \
           LEFT JOIN topic_source s ON s.id = m.source_id \
         WHERE t.merged_into IS NULL AND (?1 IS NULL OR t.id = ?1) \
         GROUP BY t.id HAVING COUNT(s.id) >= ?2 AND last >= ?3 \
         ORDER BY last DESC, t.id DESC LIMIT ?4",
    )?;
    let rows: Vec<TopicSummary> = stmt
        .query_map(params![id, min_members, since, limit as i64], |r| {
            Ok(TopicSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                status_line: r.get(2)?,
                member_count: r.get(3)?,
                last_member_ts_unix_ms: r.get(4)?,
                top_people: vec![],
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|mut s| {
            s.top_people = top_people(conn, s.id)?;
            Ok(s)
        })
        .collect()
}

fn members(conn: &Connection, topic_id: i64) -> rusqlite::Result<Vec<TopicMember>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.kind, s.ref_id, s.ts_unix_ms, s.title, m.origin, s.stub \
         FROM topic_member m JOIN topic_source s ON s.id = m.source_id \
         WHERE m.topic_id = ?1 AND m.origin != 'removed' ORDER BY s.ts_unix_ms DESC, s.id DESC",
    )?;
    let mut out: Vec<TopicMember> = stmt
        .query_map([topic_id], |r| {
            Ok(TopicMember {
                source_id: r.get(0)?,
                kind: r.get(1)?,
                ref_id: r.get(2)?,
                ts_unix_ms: r.get(3)?,
                title: r.get(4)?,
                origin: r.get(5)?,
                stub: r.get::<_, i64>(6)? != 0,
                open: None,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    for m in &mut out {
        if !m.stub {
            m.open = open_target(conn, &m.kind, m.ref_id)?;
        }
    }
    Ok(out)
}

/// Earliest future calendar event whose attendees include a person the topic knows:
/// by entity mention when the event is indexed, else by attendee address against the
/// topic's `email:` person keys.
fn next_event(conn: &Connection, topic_id: i64, now: i64) -> rusqlite::Result<Option<TopicMember>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT e.id, e.key FROM entity e \
           JOIN entity_mention em ON em.entity_id = e.id \
           JOIN topic_member m ON m.source_id = em.source_id AND m.topic_id = ?1 AND m.origin != 'removed' \
         WHERE e.kind = 'person'",
    )?;
    let people: Vec<(i64, String)> = stmt
        .query_map([topic_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if people.is_empty() {
        return Ok(None);
    }
    let ids: HashSet<i64> = people.iter().map(|p| p.0).collect();
    let emails: HashSet<&str> = people
        .iter()
        .filter_map(|p| p.1.strip_prefix("email:"))
        .collect();
    // ponytail: scans the next 200 events; raise if someone books further ahead than that
    let mut stmt = conn.prepare(
        "SELECT i.id, i.ts_unix_ms, COALESCE(i.title, ''), i.actors, s.id \
         FROM items i LEFT JOIN topic_source s ON s.kind = 'calendar_event' AND s.ref_id = i.id \
         WHERE i.kind = 'calendar_event' AND i.ts_unix_ms > ?1 ORDER BY i.ts_unix_ms, i.id LIMIT 200",
    )?;
    let mut rows = stmt.query([now])?;
    while let Some(r) = rows.next()? {
        let (item, ts, title, actors, sid): (i64, i64, String, Option<String>, Option<i64>) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
        let by_email = actors
            .and_then(|a| serde_json::from_str::<Vec<String>>(&a).ok())
            .unwrap_or_default()
            .iter()
            .flat_map(|a| parse_addresses(a))
            .any(|a| emails.contains(a.email.as_str()));
        let by_mention = match sid {
            Some(sid) => {
                let mut m = conn
                    .prepare_cached("SELECT entity_id FROM entity_mention WHERE source_id = ?1")?;
                let found = m
                    .query_map([sid], |x| x.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                found.iter().any(|e| ids.contains(e))
            }
            None => false,
        };
        if by_email || by_mention {
            return Ok(Some(TopicMember {
                source_id: sid.unwrap_or(0),
                kind: "calendar_event".into(),
                ref_id: Some(item),
                ts_unix_ms: ts,
                title,
                origin: "auto".into(),
                stub: false,
                open: None,
            }));
        }
    }
    Ok(None)
}

fn card(
    conn: &Connection,
    topic_id: i64,
    members: &[TopicMember],
    now: i64,
) -> rusqlite::Result<LeftOff> {
    // `members` is newest first, so the first of a kind is the latest.
    let last = |kind: &str| members.iter().find(|m| m.kind == kind).cloned();
    let meeting_ids: Vec<i64> = members
        .iter()
        .filter(|m| m.kind == "meeting")
        .filter_map(|m| m.ref_id)
        .collect();
    let mut open_action_items = vec![];
    // Newest meeting first, then the order the meeting lists them in.
    let mut stmt = conn.prepare_cached(
        "SELECT text FROM meeting_action_item WHERE meeting_id = ?1 AND completed = 0 ORDER BY order_index, id",
    )?;
    for id in meeting_ids {
        for t in stmt.query_map([id], |r| r.get::<_, String>(0))? {
            open_action_items.push(t?);
        }
    }
    Ok(LeftOff {
        last_email: last("email"),
        last_meeting: last("meeting"),
        last_episode: last("episode"),
        open_action_items,
        next_event: next_event(conn, topic_id, now)?,
    })
}

impl Store {
    /// Dirty, non-user-named, live topics with at least 2 members, most recent first.
    pub fn topics_needing_names(&self, limit: usize) -> rusqlite::Result<Vec<TopicNameInput>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM topic t JOIN topic_member m ON m.topic_id = t.id AND m.origin != 'removed' \
               JOIN topic_source s ON s.id = m.source_id \
             WHERE t.name_dirty = 1 AND t.name_is_user = 0 AND t.merged_into IS NULL \
             GROUP BY t.id HAVING COUNT(*) >= 2 ORDER BY MAX(s.ts_unix_ms) DESC, t.id DESC LIMIT ?1",
        )?;
        let ids: Vec<i64> = stmt
            .query_map([limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut ents = self.conn.prepare(
            "SELECT e.display FROM entity e JOIN entity_mention em ON em.entity_id = e.id \
               JOIN topic_member m ON m.source_id = em.source_id AND m.topic_id = ?1 AND m.origin != 'removed' \
             GROUP BY e.id ORDER BY SUM(em.weight) DESC, e.id LIMIT 5",
        )?;
        let mut titles = self.conn.prepare(
            "SELECT s.title FROM topic_member m JOIN topic_source s ON s.id = m.source_id \
             WHERE m.topic_id = ?1 AND m.origin != 'removed' AND s.title != '' \
             ORDER BY s.ts_unix_ms DESC, s.id DESC LIMIT 8",
        )?;
        ids.into_iter()
            .map(|id| {
                Ok(TopicNameInput {
                    id,
                    top_entities: ents
                        .query_map([id], |r| r.get(0))?
                        .collect::<rusqlite::Result<_>>()?,
                    sample_titles: titles
                        .query_map([id], |r| r.get(0))?
                        .collect::<rusqlite::Result<_>>()?,
                })
            })
            .collect()
    }

    /// Stores a generated name (and status line); no-op for user-named topics. Clears
    /// `name_dirty` and records the member count `topic_cluster` compares against (+25% renames).
    pub fn apply_topic_name(
        &self,
        id: i64,
        name: &str,
        status_line: Option<&str>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let n = tx.execute(
            "UPDATE topic SET name = ?2, status_line = ?3, name_dirty = 0, updated_at_unix_ms = ?4 \
             WHERE id = ?1 AND name_is_user = 0",
            params![id, name, status_line, now_unix_ms()],
        )?;
        if n > 0 {
            // A new row starts stale (-1) so topic_cluster rebuilds its centroid.
            tx.execute(
                "INSERT INTO topic_stats (topic_id, vec_n, named_members) VALUES (?1, -1, \
                   (SELECT COUNT(*) FROM topic_member WHERE topic_id = ?1 AND origin != 'removed')) \
                 ON CONFLICT(topic_id) DO UPDATE SET named_members = excluded.named_members",
                [id],
            )?;
        }
        tx.commit()
    }

    pub fn list_topics(
        &self,
        active_only: bool,
        limit: usize,
    ) -> rusqlite::Result<Vec<TopicSummary>> {
        self.list_topics_at(active_only, limit, now_unix_ms())
    }

    pub(crate) fn list_topics_at(
        &self,
        active_only: bool,
        limit: usize,
        now: i64,
    ) -> rusqlite::Result<Vec<TopicSummary>> {
        let since = if active_only {
            now - ACTIVE_DAYS * DAY_MS
        } else {
            i64::MIN
        };
        summaries(&self.conn, None, 2, since, limit)
    }

    /// Person/org mentions on the topic's non-removed members (for the Markdown export).
    pub fn topic_entity_mentions(&self, id: i64) -> rusqlite::Result<Vec<TopicEntityMention>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT m.source_id, e.id, e.kind, e.display FROM topic_member m \
               JOIN entity_mention em ON em.source_id = m.source_id \
               JOIN entity e ON e.id = em.entity_id \
             WHERE m.topic_id = ?1 AND m.origin != 'removed' AND e.kind IN ('person', 'org') \
             ORDER BY e.id, m.source_id",
        )?;
        let rows = stmt
            .query_map([id], |r| {
                Ok(TopicEntityMention {
                    source_id: r.get(0)?,
                    entity_id: r.get(1)?,
                    kind: r.get(2)?,
                    display: r.get(3)?,
                })
            })?
            .collect();
        rows
    }

    /// `None` when the topic does not exist. A merged id resolves to the topic it was merged into.
    pub fn topic_detail(&self, id: i64) -> rusqlite::Result<Option<TopicDetail>> {
        self.topic_detail_at(id, now_unix_ms())
    }

    pub(crate) fn topic_detail_at(
        &self,
        id: i64,
        now: i64,
    ) -> rusqlite::Result<Option<TopicDetail>> {
        let id = self
            .conn
            .query_row(
                "SELECT COALESCE(merged_into, id) FROM topic WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else { return Ok(None) };
        let Some(summary) = summaries(&self.conn, Some(id), 0, i64::MIN, 1)?.pop() else {
            return Ok(None);
        };
        let members = members(&self.conn, id)?;
        let card = card(&self.conn, id, &members, now)?;
        Ok(Some(TopicDetail {
            summary,
            card,
            members,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = DAY_MS;
    const NOW: i64 = 100 * DAY;

    fn sql(s: &Store, q: &str, p: impl rusqlite::Params) -> i64 {
        s.conn.execute(q, p).unwrap();
        s.conn.last_insert_rowid()
    }

    fn source(s: &Store, kind: &str, ref_id: Option<i64>, ts: i64, title: &str, stub: bool) -> i64 {
        sql(
            s,
            "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, stub, indexed_at_unix_ms) VALUES (?1,?2,?3,?4,?5,0)",
            params![kind, ref_id, ts, title, stub],
        )
    }

    fn topic(s: &Store, name: &str) -> i64 {
        sql(
            s,
            "INSERT INTO topic (name, created_at_unix_ms, updated_at_unix_ms) VALUES (?1,0,0)",
            [name],
        )
    }

    fn member(s: &Store, t: i64, sid: i64, origin: &str) {
        sql(
            s,
            "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1,?2,?3)",
            params![t, sid, origin],
        );
    }

    fn person(s: &Store, key: &str, display: &str, sid: i64, w: f64) -> i64 {
        sql(
            s,
            "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person',?1,?2,0) \
             ON CONFLICT(kind,key) DO UPDATE SET display = display",
            [key, display],
        );
        let e: i64 = s
            .conn
            .query_row("SELECT id FROM entity WHERE key = ?1", [key], |r| r.get(0))
            .unwrap();
        sql(
            s,
            "INSERT INTO entity_mention (entity_id, source_id, weight) VALUES (?1,?2,?3)",
            params![e, sid, w],
        );
        e
    }

    fn email_item(s: &Store, gmail_thread: &str, account: &str) -> i64 {
        sql(
            s,
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, title, body_text, refs, external_id) \
             VALUES ('d','email','gmail',1,'t','b', json_object('gmail_thread_id', ?1, 'account_email', ?2), ?1 || ?2)",
            [gmail_thread, account],
        )
    }

    /// Live topic with `n` members ending at `last_ts`.
    fn live_topic(s: &Store, name: &str, n: usize, last_ts: i64) -> i64 {
        let t = topic(s, name);
        for i in 0..n {
            let sid = source(
                s,
                "episode",
                Some(1000 + t * 10 + i as i64),
                last_ts - i as i64,
                "e",
                false,
            );
            member(s, t, sid, "auto");
        }
        t
    }

    #[test]
    fn list_active_hides_merged_single_and_stale() {
        let s = Store::open_in_memory().unwrap();
        let fresh = live_topic(&s, "fresh", 3, NOW - DAY);
        let stale = live_topic(&s, "stale", 2, NOW - 60 * DAY);
        let single = live_topic(&s, "single", 1, NOW - DAY);
        let merged = live_topic(&s, "merged", 2, NOW - DAY);
        sql(
            &s,
            "UPDATE topic SET merged_into = ?1 WHERE id = ?2",
            [fresh, merged],
        );
        // Two members, one removed: counts as single.
        let half = live_topic(&s, "half", 2, NOW - DAY);
        sql(&s, "UPDATE topic_member SET origin='removed' WHERE topic_id = ?1 AND source_id = (SELECT MIN(source_id) FROM topic_member WHERE topic_id = ?1)", [half]);

        let active: Vec<i64> = s
            .list_topics_at(true, 10, NOW)
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(active, vec![fresh]);
        let all: Vec<_> = s.list_topics_at(false, 10, NOW).unwrap();
        assert_eq!(
            all.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![fresh, stale]
        );
        assert_eq!(all[0].member_count, 3);
        assert_eq!(all[0].last_member_ts_unix_ms, NOW - DAY);
        let _ = single;
        assert_eq!(s.list_topics_at(false, 1, NOW).unwrap().len(), 1);
    }

    #[test]
    fn top_people_ranked_by_weight_and_merged_detail_resolves() {
        let s = Store::open_in_memory().unwrap();
        let t = live_topic(&s, "x", 2, NOW);
        let sids: Vec<i64> = (1..=2).collect();
        person(&s, "email:a@x.com", "Ann", sids[0], 1.0);
        person(&s, "email:b@x.com", "Bob", sids[0], 1.0);
        person(&s, "email:b@x.com", "Bob", sids[1], 1.0);
        let other = live_topic(&s, "y", 2, NOW);
        sql(
            &s,
            "UPDATE topic SET merged_into = ?1 WHERE id = ?2",
            [t, other],
        );
        let d = s.topic_detail_at(other, NOW).unwrap().unwrap();
        assert_eq!(d.summary.id, t);
        assert_eq!(d.summary.top_people, vec!["Bob", "Ann"]);
        assert!(s.topic_detail_at(9999, NOW).unwrap().is_none());
    }

    #[test]
    fn detail_card_fields() {
        let s = Store::open_in_memory().unwrap();
        let t = topic(&s, "Acme");
        let m_old = sql(&s, "INSERT INTO meeting (device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, status) VALUES ('d',1,2,'old','[]','manual','completed')", []);
        let m_new = sql(&s, "INSERT INTO meeting (device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, status) VALUES ('d',3,4,'new','[]','manual','completed')", []);
        for (m, text, done, ix) in [
            (m_old, "old open", 0, 0),
            (m_new, "new open b", 0, 1),
            (m_new, "new open a", 0, 0),
            (m_new, "done one", 1, 2),
        ] {
            sql(&s, "INSERT INTO meeting_action_item (meeting_id, text, completed, order_index, source, created_unix_ms, updated_unix_ms) VALUES (?1,?2,?3,?4,'llm',0,0)", params![m, text, done, ix]);
        }
        let e1 = email_item(&s, "g1", "me@x.com");
        let e2 = email_item(&s, "g2", "me@x.com");
        let ep = sql(&s, "INSERT INTO episodes (device_id, app_bundle_id, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, created_at_unix_ms, updated_at_unix_ms) VALUES ('d','app',1,2,1,1,1,0,0)", []);
        let src = [
            source(&s, "email", Some(e1), 10, "first mail", false),
            source(&s, "email", Some(e2), 20, "latest mail", false),
            source(&s, "meeting", Some(m_old), 30, "old", false),
            source(&s, "meeting", Some(m_new), 40, "new", false),
            source(&s, "episode", Some(ep), 50, "ep", false),
        ];
        for sid in src {
            member(&s, t, sid, "auto");
        }
        let ann = person(&s, "email:ann@acme.com", "Ann", src[1], 1.0);
        let _ = ann;
        // Future events: one for a stranger, then Ann's (not indexed), then a later one.
        let ev = |ts: i64, who: &str| {
            sql(&s, "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) VALUES ('d','calendar_event','gcal',?1,json_array(?2),?3,'',?1)", params![ts, who, format!("ev{ts}")])
        };
        ev(NOW - DAY, "Ann <ann@acme.com>"); // past
        ev(NOW + DAY, "Zed <zed@else.com>");
        let ann_ev = ev(NOW + 2 * DAY, "Ann <ANN@acme.com>");
        ev(NOW + 3 * DAY, "ann@acme.com");

        let d = s.topic_detail_at(t, NOW).unwrap().unwrap();
        assert_eq!(d.summary.member_count, 5);
        assert_eq!(d.members.len(), 5);
        assert_eq!(d.members[0].kind, "episode"); // newest first
        let c = d.card;
        assert_eq!(c.last_email.unwrap().title, "latest mail");
        assert_eq!(c.last_meeting.unwrap().title, "new");
        assert_eq!(c.last_episode.unwrap().title, "ep");
        assert_eq!(
            c.open_action_items,
            vec!["new open a", "new open b", "old open"]
        );
        let ne = c.next_event.unwrap();
        assert_eq!(
            (ne.kind.as_str(), ne.ref_id, ne.ts_unix_ms),
            ("calendar_event", Some(ann_ev), NOW + 2 * DAY)
        );
    }

    #[test]
    fn next_event_matches_indexed_event_by_mention() {
        let s = Store::open_in_memory().unwrap();
        let t = live_topic(&s, "x", 2, NOW);
        person(&s, "ann", "Ann", 1, 1.0); // name-only person entity
        let it = sql(&s, "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) VALUES ('d','calendar_event','gcal',?1,json_array('Ann'),'Sync','','e')", [NOW + DAY]);
        let sid = source(&s, "calendar_event", Some(it), NOW + DAY, "Sync", false);
        let e: i64 = s
            .conn
            .query_row("SELECT id FROM entity WHERE key='ann'", [], |r| r.get(0))
            .unwrap();
        sql(
            &s,
            "INSERT INTO entity_mention (entity_id, source_id, weight) VALUES (?1,?2,1.0)",
            [e, sid],
        );
        let ne = s
            .topic_detail_at(t, NOW)
            .unwrap()
            .unwrap()
            .card
            .next_event
            .unwrap();
        assert_eq!((ne.source_id, ne.ref_id), (sid, Some(it)));
    }

    #[test]
    fn email_member_opens_gmail_thread_window() {
        let s = Store::open_in_memory().unwrap();
        let t = topic(&s, "x");
        let thread = sql(&s, "INSERT INTO threads (source, external_thread_id, account_email, message_count, last_activity_ts_unix_ms, last_message_from, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('gmail','g1','me@x.com',1,1,'a','open',0,0)", []);
        let e = email_item(&s, "g1", "me@x.com");
        let orphan = email_item(&s, "g2", "me@x.com");
        // Same thread id on another account must not match.
        let other = email_item(&s, "g1", "else@x.com");
        for (i, it) in [e, orphan, other].into_iter().enumerate() {
            member(
                &s,
                t,
                source(&s, "email", Some(it), 10 + i as i64, "m", false),
                "auto",
            );
        }
        let d = s.topic_detail_at(t, NOW).unwrap().unwrap();
        let opens: Vec<_> = d.members.iter().rev().map(|m| m.open.clone()).collect();
        assert_eq!(
            opens,
            vec![
                Some(OpenTarget {
                    window: "thread-detail".into(),
                    id: thread
                }),
                None,
                None
            ]
        );
    }

    #[test]
    fn meeting_snapshot_episode_targets_and_stubs_have_none() {
        let s = Store::open_in_memory().unwrap();
        let t = topic(&s, "x");
        let snap = sql(&s, "INSERT INTO items (device_id, kind, source, thread_id, ts_unix_ms, body_text, external_id) VALUES ('d','snapshot','meeting',77,1,'ocr','s1')", []);
        let rows = [
            source(&s, "meeting", Some(5), 1, "m", false),
            source(&s, "episode", Some(6), 2, "e", false),
            source(&s, "snapshot", Some(snap), 3, "s", false),
            source(&s, "calendar_event", Some(9), 4, "c", false),
            source(&s, "meeting", None, 5, "stub m", true),
            source(&s, "snapshot", None, 6, "stub s", true),
        ];
        for r in rows {
            member(&s, t, r, "auto");
        }
        let d = s.topic_detail_at(t, NOW).unwrap().unwrap();
        let w = |title: &str| d.members.iter().find(|m| m.title == title).unwrap();
        let o = |win: &str, id| {
            Some(OpenTarget {
                window: win.into(),
                id,
            })
        };
        assert_eq!(w("m").open, o("meeting-detail", 5));
        assert_eq!(w("e").open, o("episode-detail", 6));
        assert_eq!(w("s").open, o("meeting-detail", 77));
        assert_eq!(w("c").open, None);
        assert!(w("stub m").stub && w("stub m").open.is_none());
        assert!(w("stub s").open.is_none());
    }

    #[test]
    fn stub_members_have_no_open_target() {
        // Even a stub that still carried a ref would not open.
        let s = Store::open_in_memory().unwrap();
        let t = topic(&s, "x");
        member(&s, t, source(&s, "meeting", Some(5), 1, "m", true), "auto");
        assert!(s.topic_detail_at(t, NOW).unwrap().unwrap().members[0]
            .open
            .is_none());
    }

    #[test]
    fn naming_inputs_and_apply() {
        let s = Store::open_in_memory().unwrap();
        let t = live_topic(&s, "", 3, NOW);
        for (i, k) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
            person(&s, &format!("email:{k}@x.com"), k, 1, 10.0 - i as f64);
        }
        let skip_user = live_topic(&s, "mine", 2, NOW);
        sql(
            &s,
            "UPDATE topic SET name_is_user = 1 WHERE id = ?1",
            [skip_user],
        );
        let skip_single = live_topic(&s, "", 1, NOW);
        let skip_clean = live_topic(&s, "", 2, NOW);
        sql(
            &s,
            "UPDATE topic SET name_dirty = 0 WHERE id = ?1",
            [skip_clean],
        );
        let _ = skip_single;

        let need = s.topics_needing_names(10).unwrap();
        assert_eq!(need.len(), 1);
        assert_eq!(need[0].id, t);
        assert_eq!(need[0].top_entities, vec!["a", "b", "c", "d", "e"]);
        assert_eq!(need[0].sample_titles.len(), 3);

        s.apply_topic_name(t, "Acme pricing", Some("waiting on legal"))
            .unwrap();
        let (name, dirty, status): (String, i64, Option<String>) = s
            .conn
            .query_row(
                "SELECT name, name_dirty, status_line FROM topic WHERE id=?1",
                [t],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), dirty, status.as_deref()),
            ("Acme pricing", 0, Some("waiting on legal"))
        );
        let nm: i64 = s
            .conn
            .query_row(
                "SELECT named_members FROM topic_stats WHERE topic_id=?1",
                [t],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nm, 3);
        assert!(s.topics_needing_names(10).unwrap().is_empty());

        // User names are never overwritten.
        s.apply_topic_name(skip_user, "Auto", None).unwrap();
        let n: String = s
            .conn
            .query_row("SELECT name FROM topic WHERE id=?1", [skip_user], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, "mine");
    }

    #[test]
    fn entity_mentions_skip_removed_members_and_phrases() {
        let s = Store::open_in_memory().unwrap();
        let t = live_topic(&s, "x", 2, NOW);
        person(&s, "email:a@x.com", "Ann", 1, 1.0);
        person(&s, "email:a@x.com", "Ann", 2, 1.0);
        sql(&s, "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('org','acme.com','Acme',0)", []);
        sql(&s, "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('phrase','p','Phrase',0)", []);
        sql(&s, "INSERT INTO entity_mention (entity_id, source_id) SELECT id, 1 FROM entity WHERE key IN ('acme.com','p')", []);
        let m = s.topic_entity_mentions(t).unwrap();
        assert_eq!(
            m.iter().map(|x| (x.source_id, x.kind.as_str())).collect::<Vec<_>>(),
            vec![(1, "person"), (2, "person"), (1, "org")]
        );
        sql(&s, "UPDATE topic_member SET origin='removed' WHERE topic_id=?1 AND source_id=2", [t]);
        assert_eq!(s.topic_entity_mentions(t).unwrap().len(), 2);
    }
}
