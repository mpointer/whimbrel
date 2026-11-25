//! Read-only lookups behind "everything about this client" (X6 client controls, spec 4.1-4.2):
//! the sources a seed owns, the steps filed under it, which other client shares a source,
//! and the rows whose text merely names the client. Nothing here writes.

use std::collections::HashSet;

use rusqlite::{params, OptionalExtension};

use super::read::source_not_removed;
use super::seeds::seed_id;
use crate::Store;

/// One source filed under a seed (origin seed, jev or auto; never `removed`).
#[derive(Debug, Clone, PartialEq)]
pub struct MemberRef {
    pub source_id: i64,
    /// `topic_source.kind`: meeting, episode, email, calendar_event or snapshot.
    pub kind: String,
    pub ref_id: Option<i64>,
    pub origin: String,
    pub ts_unix_ms: i64,
    /// A calendar event marked removed (gone from every calendar account). [`Store::seed_members`] leaves
    /// these out; [`Store::client_members`] lists them, flagged, because a client delete must take them too.
    pub removed: bool,
}

/// A row whose text names the client: `kind` is meeting, email, calendar_event, capture or
/// conversation, `ref_id` the row id in that kind's table (the meeting a slide belongs to
/// for a snapshot; the conversation for an exchange).
#[derive(Debug, Clone, PartialEq)]
pub struct NameHit {
    pub kind: &'static str,
    pub ref_id: i64,
    pub ts_unix_ms: i64,
}

/// The most bytes of one text the excerpt search reads (cut at a character boundary).
pub const MAX_EXCERPT_TEXT: usize = 64 * 1024;

fn bound(t: &str) -> &str {
    if t.len() <= MAX_EXCERPT_TEXT {
        return t;
    }
    let mut end = MAX_EXCERPT_TEXT;
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    &t[..end]
}

/// Candidates read per full-text table before the whole-word check.
const FTS_LIMIT: i64 = 5_000;

/// The spellings of `word` the full-text index (unicode61: case and accents folded) holds
/// for text the whole-word matcher would call the same word: the matcher also spells ß as
/// ss, which the index does not, so "strauss" and "strauß" are each searched as both.
fn fold_variants(word: &str) -> Vec<String> {
    let lower = word.to_lowercase();
    let mut out = vec![lower.clone()];
    if lower.contains('ß') {
        out.push(lower.replace('ß', "ss"));
    }
    // Each "ss" may be a ß: every combination of the first few.
    let spots: Vec<usize> = lower.match_indices("ss").map(|(i, _)| i).take(4).collect();
    for mask in 1..(1u32 << spots.len()) {
        let mut v = String::new();
        let mut last = 0;
        for (n, &i) in spots.iter().enumerate() {
            if mask & (1 << n) != 0 && i >= last {
                v.push_str(&lower[last..i]);
                v.push('ß');
                last = i + 2;
            }
        }
        v.push_str(&lower[last..]);
        out.push(v);
    }
    out.sort();
    out.dedup();
    out
}

/// An FTS5 query matching any of `terms` as a phrase of its words (each word in every
/// spelling of [`fold_variants`]); `None` when no term has a letter or digit. Built only
/// from quoted letter-and-digit words, never from raw term text.
fn fts_query(terms: &[String]) -> Option<String> {
    fts_query_for(terms)
}

/// [`fts_query`], for the exclusion lift.
pub(crate) fn fts_query_for(terms: &[String]) -> Option<String> {
    let mut phrases: Vec<String> = Vec::new();
    for t in terms {
        // The spelling without invisible characters (a soft hyphen would split the word).
        let t = crate::words::without_invisibles(t);
        let words: Vec<Vec<String>> = t
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(fold_variants)
            .collect();
        if words.is_empty() {
            continue;
        }
        let mut combos: Vec<Vec<&str>> = vec![Vec::new()];
        for w in &words {
            let mut next = Vec::new();
            for c in &combos {
                for v in w {
                    if next.len() < 64 {
                        let mut n = c.clone();
                        n.push(v.as_str());
                        next.push(n);
                    }
                }
            }
            combos = next;
        }
        for c in combos {
            phrases.push(format!("\"{}\"", c.join(" ")));
        }
    }
    phrases.sort();
    phrases.dedup();
    (!phrases.is_empty()).then(|| phrases.join(" OR "))
}

/// Rows that name a client, and whether the search stopped at its candidate limit.
#[derive(Debug, Clone, PartialEq)]
pub struct NameHits {
    pub hits: Vec<NameHit>,
    /// A full-text table returned as many candidates as the limit allows: there may be more.
    pub truncated: bool,
}

impl Store {
    /// The raw text a client-controls row is named by on the user's own screen: a meeting's title, an
    /// email's subject, an event's title, a capture's or screen session's window title, an Ask
    /// conversation's title (its first question while the title is a placeholder), a step's text. `kind`
    /// is the inventory's kind (`meeting`, `email`, `calendar_event`, `capture`, `episode`,
    /// `conversation`, `step`); a capture id may be `i:<items.id>`. `None` when there is no such row, no
    /// text, or the kind has no label. Client text: never log it, never put it in a receipt.
    pub fn client_item_label(&self, kind: &str, id: &str) -> rusqlite::Result<Option<String>> {
        let num = || id.parse::<i64>().ok();
        let one = |sql: &str, arg: rusqlite::types::Value| -> rusqlite::Result<Option<String>> {
            Ok(self.conn.query_row(sql, [arg], |r| r.get::<_, Option<String>>(0)).optional()?.flatten())
        };
        let text = match kind {
            "meeting" => match num() {
                Some(n) => one("SELECT title FROM meeting WHERE id = ?1", n.into())?,
                None => None,
            },
            "email" | "calendar_event" => match num() {
                Some(n) => one("SELECT title FROM items WHERE id = ?1", n.into())?,
                None => None,
            },
            "capture" => match id.strip_prefix("i:").and_then(|i| i.parse::<i64>().ok()) {
                Some(n) => one("SELECT title FROM items WHERE id = ?1", n.into())?,
                None => match num() {
                    Some(n) => one("SELECT window_title FROM capture_item WHERE id = ?1", n.into())?,
                    None => None,
                },
            },
            "episode" => match num() {
                Some(n) => one("SELECT window_title FROM episodes WHERE id = ?1", n.into())?,
                None => None,
            },
            "conversation" => match num() {
                Some(n) => one(
                    "SELECT CASE WHEN c.title_is_placeholder = 0 THEN c.title ELSE \
                     COALESCE((SELECT x.query FROM conversation_exchange x WHERE x.conversation_id = c.id \
                     ORDER BY x.id LIMIT 1), c.title) END FROM conversation c WHERE c.id = ?1",
                    n.into(),
                )?,
                None => None,
            },
            "step" => one("SELECT text FROM topic_step WHERE key = ?1", id.to_string().into())?,
            // An added item: its title, else the start of its text.
            "mcp_item" => match num() {
                Some(n) => one(
                    "SELECT coalesce(nullif(trim(title), ''), substr(body_text, 1, 120)) FROM items WHERE id = ?1 AND kind = 'mcp_item'",
                    n.into(),
                )?,
                None => None,
            },
            _ => None,
        };
        Ok(text.filter(|t| !t.trim().is_empty()))
    }

    /// Looks through the texts of a client-controls row that can name a client, body first and title last,
    /// calling `find` on each until it answers `Some` (the review screen's "why it matched" excerpt). `kind` and
    /// `id` as in [`Store::client_item_label`]. A meeting's transcript is read segment by segment and the
    /// read stops at the first answer; no text is read past [`MAX_EXCERPT_TEXT`] bytes. `None` when there is
    /// no such row or no text answers. Client text: display only, never logged.
    pub fn client_item_find(
        &self,
        kind: &str,
        id: &str,
        find: &dyn Fn(&str) -> Option<String>,
    ) -> rusqlite::Result<Option<String>> {
        let num = id.parse::<i64>().ok();
        let try_text = |t: &Option<String>| t.as_deref().filter(|t| !t.trim().is_empty()).and_then(|t| find(bound(t)));
        let row2 = |sql: &str, n: i64| -> rusqlite::Result<Option<String>> {
            let r = self
                .conn
                .query_row(sql, [n], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)))
                .optional()?;
            Ok(r.and_then(|(a, b)| try_text(&a).or_else(|| try_text(&b))))
        };
        match kind {
            "meeting" => {
                let Some(n) = num else { return Ok(None) };
                let mut stmt = self
                    .conn
                    .prepare("SELECT text FROM transcript_segment WHERE meeting_id = ?1 ORDER BY start_cs")?;
                let mut rows = stmt.query([n])?;
                while let Some(r) = rows.next()? {
                    if let Some(x) = try_text(&r.get::<_, Option<String>>(0)?) {
                        return Ok(Some(x));
                    }
                }
                if let Some(x) = row2("SELECT coalesce(notes, ''), coalesce(summary_tldr, '') FROM meeting WHERE id = ?1", n)? {
                    return Ok(Some(x));
                }
                row2("SELECT title, NULL FROM meeting WHERE id = ?1", n)
            }
            "email" | "calendar_event" => num.map_or(Ok(None), |n| row2("SELECT body_text, title FROM items WHERE id = ?1", n)),
            "mcp_item" => num.map_or(Ok(None), |n| row2("SELECT body_text, title FROM items WHERE id = ?1 AND kind = 'mcp_item'", n)),
            "capture" => match id.strip_prefix("i:").and_then(|i| i.parse::<i64>().ok()) {
                Some(n) => row2("SELECT body_text, title FROM items WHERE id = ?1", n),
                None => num.map_or(Ok(None), |n| row2("SELECT text, window_title FROM capture_item WHERE id = ?1", n)),
            },
            "conversation" => {
                let Some(n) = num else { return Ok(None) };
                let mut stmt = self
                    .conn
                    .prepare("SELECT query, answer FROM conversation_exchange WHERE conversation_id = ?1 ORDER BY id")?;
                let mut rows = stmt.query([n])?;
                while let Some(r) = rows.next()? {
                    if let Some(x) = try_text(&r.get::<_, Option<String>>(0)?).or_else(|| try_text(&r.get::<_, Option<String>>(1).ok().flatten())) {
                        return Ok(Some(x));
                    }
                }
                Ok(None)
            }
            "step" => {
                let t: Option<String> = self
                    .conn
                    .query_row("SELECT text FROM topic_step WHERE key = ?1", [id], |r| r.get::<_, Option<String>>(0))
                    .optional()?
                    .flatten();
                Ok(try_text(&t))
            }
            _ => Ok(None),
        }
    }

    /// The app (bundle id) a screen capture came from, or `None` (a capture held only in `items`, or no row).
    pub fn client_capture_app(&self, id: &str) -> rusqlite::Result<Option<String>> {
        let Some(n) = id.parse::<i64>().ok() else { return Ok(None) };
        Ok(self
            .conn
            .query_row("SELECT app_bundle_id FROM capture_item WHERE id = ?1", [n], |r| r.get::<_, Option<String>>(0))
            .optional()?
            .flatten()
            .filter(|a| !a.trim().is_empty()))
    }

    /// The sources filed under seed `key`, newest first. Errors when there is no such seed.
    pub fn seed_members(&self, key: &str) -> rusqlite::Result<Vec<MemberRef>> {
        let topic_id = seed_id(&self.conn, key)?;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT s.id, s.kind, s.ref_id, m.origin, s.ts_unix_ms \
             FROM topic_member m JOIN topic_source s ON s.id = m.source_id \
             WHERE m.topic_id = ?1 AND m.origin != 'removed' AND {} \
             ORDER BY s.ts_unix_ms DESC, s.id DESC",
            source_not_removed("s")
        ))?;
        let rows = stmt
            .query_map([topic_id], |r| {
                Ok(MemberRef {
                    source_id: r.get(0)?,
                    kind: r.get(1)?,
                    ref_id: r.get(2)?,
                    origin: r.get(3)?,
                    ts_unix_ms: r.get(4)?,
                    removed: false,
                })
            })?
            .collect();
        rows
    }

    /// Everything the client `key` owns: the sources filed under its seed, under every seed merged into it
    /// (down the chain) and under the unseeded topics merged into those (never a member marked `removed`),
    /// newest first, each source once. Unlike [`Self::seed_members`] it keeps calendar events marked removed,
    /// flagged `removed`: their rows still hold the client's text. Errors when `key` is not a seed. The one
    /// list both the client inventory and its delete read.
    pub fn client_members(&self, key: &str) -> rusqlite::Result<Vec<MemberRef>> {
        seed_id(&self.conn, key)?;
        let topics = self.client_scope(key)?.topic_ids;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT s.id, s.kind, s.ref_id, m.origin, s.ts_unix_ms, NOT ({}) \
             FROM topic_member m JOIN topic_source s ON s.id = m.source_id \
             WHERE m.topic_id = ?1 AND m.origin != 'removed'",
            source_not_removed("s")
        ))?;
        let mut out: Vec<MemberRef> = Vec::new();
        for t in topics {
            let rows = stmt.query_map([t], |r| {
                Ok(MemberRef {
                    source_id: r.get(0)?,
                    kind: r.get(1)?,
                    ref_id: r.get(2)?,
                    origin: r.get(3)?,
                    ts_unix_ms: r.get(4)?,
                    removed: r.get(5)?,
                })
            })?;
            for m in rows {
                let m = m?;
                if !out.iter().any(|o| o.source_id == m.source_id) {
                    out.push(m);
                }
            }
        }
        out.sort_by(|a, b| b.ts_unix_ms.cmp(&a.ts_unix_ms).then(b.source_id.cmp(&a.source_id)));
        Ok(out)
    }

    /// The captures episode `episode_id` was built over (its `start_item_id..end_item_id` range on its
    /// device): `(items.id, capture_item.id when this device holds the legacy twin, ts)`.
    pub fn episode_captures(&self, episode_id: i64) -> rusqlite::Result<Vec<(i64, Option<i64>, i64)>> {
        self.conn
            .prepare(
                "SELECT i.id, CASE WHEN i.source_table = 'capture_item' THEN i.source_id END, i.ts_unix_ms \
                 FROM items i JOIN episodes e ON i.device_id = e.device_id \
                 WHERE e.id = ?1 AND i.kind = 'capture' AND i.id BETWEEN e.start_item_id AND e.end_item_id \
                 ORDER BY i.id",
            )?
            .query_map([episode_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect()
    }

    /// The steps filed under client `key` or any seed merged into it: (step key, asked-at time).
    pub fn client_step_refs(&self, key: &str) -> rusqlite::Result<Vec<(String, i64)>> {
        let mut out = Vec::new();
        for k in crate::client_forget::merge_chain(&self.conn, key)? {
            out.extend(self.seed_step_refs(&k)?);
        }
        out.sort_by(|a: &(String, i64), b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.dedup();
        Ok(out)
    }

    /// The steps filed under seed `key`: (step key, asked-at time).
    pub fn seed_step_refs(&self, key: &str) -> rusqlite::Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT key, asked_at_unix_ms FROM topic_step WHERE topic_key = ?1 \
             ORDER BY asked_at_unix_ms DESC, key",
        )?;
        let rows = stmt
            .query_map([key], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect();
        rows
    }

    /// The meeting a slide snapshot item belongs to and when it started.
    pub fn snapshot_meeting(&self, item_id: i64) -> rusqlite::Result<Option<(i64, i64)>> {
        self.conn
            .query_row(
                "SELECT m.id, m.start_unix_ms FROM items i JOIN meeting m ON m.id = i.thread_id \
                 WHERE i.id = ?1 AND i.kind = 'snapshot'",
                [item_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
    }

    /// The name of another live client (seed other than `except_key`) that also holds the
    /// source `(kind, ref_id)`, if any.
    pub fn other_client_for(
        &self,
        kind: &str,
        ref_id: i64,
        except_key: &str,
    ) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT t.name FROM topic_source s \
                 JOIN topic_member m ON m.source_id = s.id AND m.origin != 'removed' \
                 JOIN topic_seed sd ON sd.topic_id = m.topic_id AND sd.deleted = 0 AND sd.key != ?3 \
                 JOIN topic t ON t.id = m.topic_id \
                 WHERE s.kind = ?1 AND s.ref_id = ?2 ORDER BY m.topic_id LIMIT 1",
                params![kind, ref_id, except_key],
                |r| r.get(0),
            )
            .optional()
    }

    /// Every live client holding source `(kind, ref_id)`: the key of each seed whose topic has
    /// it as a member, followed up its merge chain to the seed it was merged into. By key.
    pub fn source_clients(&self, kind: &str, ref_id: i64) -> rusqlite::Result<Vec<String>> {
        let direct: Vec<String> = self
            .conn
            .prepare(
                "SELECT sd.key FROM topic_source s \
                 JOIN topic_member m ON m.source_id = s.id AND m.origin != 'removed' \
                 JOIN topic_seed sd ON sd.topic_id = m.topic_id AND sd.deleted = 0 \
                 WHERE s.kind = ?1 AND s.ref_id = ?2",
            )?
            .query_map(params![kind, ref_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for mut key in direct {
            for _ in 0..64 {
                let next: Option<String> = self
                    .conn
                    .query_row("SELECT merged_into_key FROM topic_seed WHERE key = ?1", [&key], |r| r.get(0))
                    .optional()?
                    .flatten();
                match next {
                    Some(n) => key = n,
                    None => break,
                }
            }
            out.push(key);
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Rows whose text names any of `terms`: email and calendar items, meeting titles and
    /// transcripts, slides, screen captures and assistant conversations. One hit per
    /// (kind, ref_id), newest first. Full-text indexes narrow the candidates; `confirm`
    /// (the caller's whole-word matcher: `llm::guard::ExclusionSet::names_in` for the delete
    /// inventory, which never counts merely undecodable text) decides, so
    /// "Ann" never finds "Annual".
    pub fn name_hits(
        &self,
        terms: &[String],
        confirm: &dyn Fn(&str) -> bool,
    ) -> rusqlite::Result<NameHits> {
        self.name_hits_limited(terms, confirm, FTS_LIMIT)
    }

    /// Meetings, as `(id, start)`, whose own text names a client by `confirm`. `only` is a query
    /// of meeting ids to look at (the incremental exclusion path); `None` reads every meeting.
    /// Notes and summaries count too (fix round 3, R1): a meeting whose notes name the client is
    /// the client's everywhere, not only where the summary path looks.
    /// Every text about a meeting that a user or a model wrote (fix round 4, S1): the workspace
    /// draft (the Mac never copies it to `meeting.notes`), action items, follow-ups, the people
    /// named as speakers, live alerts, marker labels, line sources (line, owner, due date, anchor), live
    /// transcript lines, slide text and attendee details.
    /// Keep in step with `Store::meeting_authored_text` and the exclusion triggers.
    pub(crate) fn meeting_text_hits(
        &self,
        confirm: &dyn Fn(&str) -> bool,
        only: Option<&str>,
    ) -> rusqlite::Result<Vec<(i64, i64)>> {
        let filter = only.map(|q| format!(" WHERE m.id IN ({q})")).unwrap_or_default();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT m.id, m.start_unix_ms, m.title, m.attendees, coalesce(m.notes, '') || ' ' || coalesce(m.summary_tldr, '') || ' ' || \
             coalesce(m.summary_decisions_json, '') || ' ' || coalesce(m.summary_action_items_json, '') || ' ' || \
             coalesce(m.summary_open_questions_json, '') || ' ' || coalesce(d.personal_notes, '') || ' ' || \
             coalesce(d.summary_edited, '') || ' ' || coalesce(d.decisions_edited_json, '') || ' ' || \
             coalesce(d.open_questions_edited_json, '') || ' ' || coalesce(m.attendee_details, '') || ' ' || \
             coalesce((SELECT group_concat(text, ' ') FROM meeting_action_item WHERE meeting_id = m.id), '') || ' ' || \
             coalesce((SELECT group_concat(text, ' ') FROM meeting_followup WHERE meeting_id = m.id), '') || ' ' || \
             coalesce((SELECT group_concat(p.name || ' ' || coalesce(p.email, ''), ' ') FROM meeting_speaker sp \
                       JOIN person p ON p.person_key = sp.person_key WHERE sp.meeting_id = m.id), '') || ' ' || \
             coalesce((SELECT group_concat(quote || ' ' || restatement || ' ' || coalesce(asker, '') || ' ' || coalesce(draft_json, ''), ' ') \
                       FROM live_alert WHERE meeting_id = m.id), '') || ' ' || \
             coalesce((SELECT group_concat(label, ' ') FROM meeting_marker WHERE meeting_id = m.id AND label IS NOT NULL), '') || ' ' || \
             coalesce((SELECT group_concat(line_text || ' ' || coalesce(owner, '') || ' ' || coalesce(due, '') || ' ' || coalesce(anchor, ''), ' ') \
                       FROM meeting_line_source WHERE meeting_id = m.id), '') || ' ' || \
             coalesce((SELECT group_concat(text, ' ') FROM live_transcript_segment WHERE meeting_id = m.id), '') || ' ' || \
             coalesce((SELECT group_concat(ocr_text, ' ') FROM meeting_snapshot WHERE meeting_id = m.id), '') \
             FROM meeting m LEFT JOIN meeting_note_draft d ON d.meeting_id = m.id{filter}",
        ))?;
        let mut out = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let text = format!("{}\n{}\n{}", r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?);
            // Its JSON columns (summary and edited lists, attendee details, live-alert drafts) are
            // matched with escapes decoded (X6 fix round 6, J2).
            if confirm(&crate::words::name_text(&text)) {
                out.push((r.get(0)?, r.get(1)?));
            }
        }
        Ok(out)
    }

    /// [`Store::name_hits`] reading at most `limit` candidates per full-text table.
    pub fn name_hits_limited(
        &self,
        terms: &[String],
        confirm: &dyn Fn(&str) -> bool,
        limit: i64,
    ) -> rusqlite::Result<NameHits> {
        let mut truncated = false;
        let mut seen: HashSet<(&'static str, i64)> = HashSet::new();
        let mut out: Vec<NameHit> = Vec::new();
        let mut push = |kind: &'static str, ref_id: i64, ts: i64| {
            if seen.insert((kind, ref_id)) {
                out.push(NameHit { kind, ref_id, ts_unix_ms: ts });
            }
        };
        if let Some(q) = fts_query(terms) {
            // ponytail: first FTS_LIMIT candidates per table; a name that is also a common
            // word would need paging.
            let mut stmt = self.conn.prepare(
                "SELECT i.id, i.kind, i.thread_id, COALESCE(m.start_unix_ms, i.ts_unix_ms), COALESCE(i.title, ''), i.body_text \
                 FROM items_fts f CROSS JOIN items i ON i.id = f.rowid \
                 LEFT JOIN meeting m ON i.kind = 'snapshot' AND m.id = i.thread_id \
                 WHERE items_fts MATCH ?1 AND i.kind IN ('email', 'calendar_event', 'snapshot') \
                 ORDER BY f.rowid DESC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![q, limit])?;
            let mut n = 0i64;
            while let Some(r) = rows.next()? {
                n += 1;
                let (id, kind, thread, ts): (i64, String, Option<i64>, i64) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
                let text = format!("{}\n{}", r.get::<_, String>(4)?, r.get::<_, String>(5)?);
                if !confirm(&text) {
                    continue;
                }
                match kind.as_str() {
                    "email" => push("email", id, ts),
                    "calendar_event" => push("calendar_event", id, ts),
                    _ => {
                        if let Some(m) = thread {
                            push("meeting", m, ts);
                        }
                    }
                }
            }
            truncated |= n >= limit;
            // Added items read in their own page with their own limit, so a large sync naming the
            // client never pushes its emails, events and slides out (review I3).
            let mut stmt = self.conn.prepare(
                "SELECT i.id, i.ts_unix_ms, COALESCE(i.title, ''), i.body_text \
                 FROM items_fts f CROSS JOIN items i ON i.id = f.rowid \
                 WHERE items_fts MATCH ?1 AND i.kind = 'mcp_item' \
                 ORDER BY f.rowid DESC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![q, limit])?;
            let mut n = 0i64;
            while let Some(r) = rows.next()? {
                n += 1;
                let (id, ts): (i64, i64) = (r.get(0)?, r.get(1)?);
                let text = format!("{}\n{}", r.get::<_, String>(2)?, r.get::<_, String>(3)?);
                if confirm(&text) {
                    push("mcp_item", id, ts);
                }
            }
            truncated |= n >= limit;
            // Added items named only outside their text: the adding client's name, label, link, people.
            for (id, ts) in self.mcp_items_naming_outside_text_at(confirm, None)? {
                push("mcp_item", id, ts);
            }
            let mut stmt = self.conn.prepare(
                "SELECT s.meeting_id, m.start_unix_ms, s.text FROM transcript_segment_fts f \
                 CROSS JOIN transcript_segment s ON s.id = f.rowid JOIN meeting m ON m.id = s.meeting_id \
                 WHERE transcript_segment_fts MATCH ?1 ORDER BY f.rowid DESC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![q, limit])?;
            let mut n = 0i64;
            while let Some(r) = rows.next()? {
                n += 1;
                if confirm(&r.get::<_, String>(2)?) {
                    push("meeting", r.get(0)?, r.get(1)?);
                }
            }
            truncated |= n >= limit;
            let mut stmt = self.conn.prepare(
                "SELECT c.id, c.ts_unix_ms, c.window_title, c.text FROM capture_item_fts f \
                 CROSS JOIN capture_item c ON c.id = f.rowid \
                 WHERE capture_item_fts MATCH ?1 ORDER BY f.rowid DESC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![q, limit])?;
            let mut n = 0i64;
            while let Some(r) = rows.next()? {
                n += 1;
                let text = format!("{}\n{}", r.get::<_, String>(2)?, r.get::<_, String>(3)?);
                if confirm(&text) {
                    push("capture", r.get(0)?, r.get(1)?);
                }
            }
            truncated |= n >= limit;
        }
        // Meeting titles, attendees, notes and summaries have no full-text index; the table is
        // small. See `meeting_text_hits` for what counts.
        for (id, ts) in self.meeting_text_hits(confirm, None)? {
            push("meeting", id, ts);
        }
        // Conversations have no full-text index either: one hit per conversation.
        let mut stmt = self.conn.prepare(
            "SELECT conversation_id, created_at_unix_ms, query, answer FROM conversation_exchange",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let text = format!("{}\n{}", r.get::<_, String>(2)?, r.get::<_, String>(3)?);
            if confirm(&text) {
                push("conversation", r.get(0)?, r.get(1)?);
            }
        }
        out.sort_by(|a, b| b.ts_unix_ms.cmp(&a.ts_unix_ms).then(a.ref_id.cmp(&b.ref_id)));
        Ok(NameHits { hits: out, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::FakeEmbedder;
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("t.db"), Arc::new(FakeEmbedder));
        (dir, s)
    }

    fn source(s: &Store, topic: i64, kind: &str, r: i64, ts: i64, origin: &str) {
        s.conn
            .execute(
                "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, ?3, 1)",
                params![kind, r, ts],
            )
            .unwrap();
        s.conn
            .execute(
                "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, ?3)",
                params![topic, s.conn.last_insert_rowid(), origin],
            )
            .unwrap();
    }

    #[test]
    fn seed_members_lists_live_origins_newest_first_and_skips_removed() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        source(&s, a.topic_id, "meeting", 5, 100, "seed");
        source(&s, a.topic_id, "email", 6, 300, "jev");
        source(&s, a.topic_id, "episode", 7, 200, "auto");
        source(&s, a.topic_id, "email", 8, 400, "removed");
        let m = s.seed_members(&a.key).unwrap();
        let got: Vec<(&str, Option<i64>, &str)> = m
            .iter()
            .map(|x| (x.kind.as_str(), x.ref_id, x.origin.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("email", Some(6), "jev"),
                ("episode", Some(7), "auto"),
                ("meeting", Some(5), "seed")
            ]
        );
        assert!(s.seed_members("nope").is_err());
    }

    #[test]
    fn fts_query_quotes_words_and_spells_sharp_s_both_ways() {
        let q = fts_query(&["O'Brien \"x\" *".to_string(), "Strauss".to_string(), "!!".to_string()]).unwrap();
        assert!(q.contains("\"o brien x\""), "{q}");
        assert!(q.contains("\"strauss\"") && q.contains("\"strauß\""), "{q}");
        assert!(!q.contains('*') && !q.contains('\''));
        assert_eq!(fts_query(&["!!".to_string(), " ".to_string()]), None);
        assert_eq!(fold_variants("Weiß"), vec!["weiss".to_string(), "weiß".to_string()]);
    }
}
