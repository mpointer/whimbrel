//! `topic_index`: turns emails, calendar events, slide snapshots, meetings and episodes into
//! `topic_source` rows with an embedding and entity mentions. Driven only by explicit entry
//! points (never `run_jobs_batch`). Every kind advances an id high-water mark in `kv_meta`;
//! edited meetings and episodes are queued in `topic_dirty` by triggers and re-indexed.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{params, Connection, OptionalExtension};

use super::chrome;
use super::entities::{
    extract_phrases, org_key_from_domain, parse_addresses, person_key, EntityKind,
};
use super::schema::PRUNE_DIRTY_KEY;
use crate::{embedding_to_json, Store, EMBEDDING_DIM};

const TEXT_CHARS: usize = 2000;
const VEC_SAMPLE: usize = 200; // ponytail: mean over first 200 member vectors; sample if too coarse
const HOLD_MS: i64 = 3_600_000;
const ME: &str = "me";

/// Every connected account email, lowercased.
pub(crate) fn self_emails(conn: &Connection) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare("SELECT DISTINCT lower(email) FROM connector_account") else {
        return Vec::new();
    };
    stmt.query_map([], |r| r.get(0))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// Person key with "Last, First" folded to "first last".
pub(crate) fn name_key(name: &str) -> String {
    let parts: Vec<&str> = name.split(',').map(str::trim).collect();
    if parts.len() == 2 && parts.iter().all(|p| !p.is_empty()) {
        return person_key(&format!("{} {}", parts[1], parts[0]));
    }
    person_key(name)
}

const DAY_MS: i64 = 86_400_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The first `TEXT_CHARS` characters, redacted before the cut so a secret straddling it
/// cannot survive in part.
fn head(s: &str) -> String {
    crate::redact::redact_secrets(s)
        .chars()
        .take(TEXT_CHARS)
        .collect()
}

struct Ctx<'a> {
    conn: &'a Connection,
    selfs: Vec<String>,
    /// name_key -> person entity id a name-only attendee joins, resolved lazily per key.
    people: HashMap<String, i64>,
    /// [`Ctx::trusted`] answers for this call (its inputs don't change inside the transaction).
    trust: HashMap<String, bool>,
    now: i64,
    /// (source id, text) still needing an embedding; computed after the transaction commits.
    pending: Vec<(i64, String)>,
    /// This device's id, for source keys (`super::steps::attach_source`).
    own: Option<String>,
}

/// Entity ids with their best weight for one source.
type Mentions = HashMap<i64, f64>;

impl<'a> Ctx<'a> {
    fn new(conn: &'a Connection) -> Self {
        Ctx {
            conn,
            selfs: self_emails(conn),
            people: HashMap::new(),
            trust: HashMap::new(),
            now: now_ms(),
            pending: Vec::new(),
            own: None,
        }
    }

    fn entity(&self, kind: EntityKind, key: &str, display: &str) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(kind, key) DO UPDATE SET updated_at_unix_ms = excluded.updated_at_unix_ms \
             RETURNING id",
            params![kind.as_str(), key, display, self.now],
            |r| r.get(0),
        )
    }

    fn add(m: &mut Mentions, id: i64, w: f64) {
        let e = m.entry(id).or_insert(0.0);
        *e = e.max(w);
    }

    fn email_person(
        &mut self,
        m: &mut Mentions,
        email: &str,
        name: Option<&str>,
    ) -> rusqlite::Result<()> {
        if self.selfs.iter().any(|s| s == email) {
            return Ok(());
        }
        let nk = name.map(name_key).filter(|k| !k.is_empty());
        // The first real name seen replaces the bare-address placeholder (and its name_key).
        let id: i64 = self.conn.query_row(
            "INSERT INTO entity (kind, key, display, updated_at_unix_ms, name_key) \
             VALUES ('person', ?1, ?2, ?3, ?4) \
             ON CONFLICT(kind, key) DO UPDATE SET \
               updated_at_unix_ms = excluded.updated_at_unix_ms, \
               display = CASE WHEN display = substr(key, 7) THEN excluded.display ELSE display END, \
               name_key = CASE WHEN display = substr(key, 7) THEN excluded.name_key ELSE name_key END \
             RETURNING id",
            params![format!("email:{email}"), name.unwrap_or(email), self.now, nk],
            |r| r.get(0),
        )?;
        if let Some(k) = nk {
            let old = match self.people.get(&k) {
                Some(&o) => Some(o),
                None => self.name_entity(&k)?,
            };
            if let Some(old) = old.filter(|&o| o != id) {
                self.absorb_name_person(m, old, id, &k, email)?;
            }
        }
        Self::add(m, id, 1.0);
        if let Some(org) = email
            .rsplit_once('@')
            .and_then(|(_, d)| org_key_from_domain(d))
        {
            let oid = self.entity(EntityKind::Org, &org, &org)?;
            Self::add(m, oid, 0.7);
        }
        Ok(())
    }

    /// User-generated signals only (inbound invites and From headers are attacker-controlled):
    /// the address attends a recorded meeting, or the user sent mail to it (SENT label on one
    /// of their own accounts). Exact parsed address comparison; SQL only prefilters.
    fn trusted(&mut self, email: &str) -> rusqlite::Result<bool> {
        if let Some(&t) = self.trust.get(email) {
            return Ok(t);
        }
        let t = self.trusted_uncached(email)?;
        self.trust.insert(email.to_string(), t);
        Ok(t)
    }

    fn trusted_uncached(&self, email: &str) -> rusqlite::Result<bool> {
        let has = |raw: &str| parse_addresses(raw).iter().any(|a| a.email == email);
        let mut stmt = self
            .conn
            .prepare("SELECT attendees FROM meeting WHERE instr(lower(attendees), ?1) > 0")?;
        for raw in stmt.query_map([email], |r| r.get::<_, String>(0))? {
            if json_strings(Some(raw?)).iter().any(|a| has(a)) {
                return Ok(true);
            }
        }
        // ponytail: scans emails that mention the address; fine while merges are rare
        let mut stmt = self.conn.prepare(
            "SELECT json_extract(actors, '$[1]'), lower(json_extract(refs, '$.account_email')) \
             FROM items WHERE kind = 'email' \
               AND instr(lower(json_extract(actors, '$[1]')), ?1) > 0 \
               AND EXISTS (SELECT 1 FROM json_each(items.refs, '$.gmail_labels') WHERE value = 'SENT')",
        )?;
        for row in stmt.query_map([email], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })? {
            let (to, account) = row?;
            if account.is_some_and(|a| self.selfs.contains(&a)) && has(&to.unwrap_or_default()) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// A name-only person turned out to be this email person: move its mentions over.
    fn absorb_name_person(
        &mut self,
        m: &mut Mentions,
        old: i64,
        id: i64,
        k: &str,
        email: &str,
    ) -> rusqlite::Result<()> {
        let is_name: bool = self.conn.query_row(
            "SELECT key LIKE 'name:%' FROM entity WHERE id = ?1",
            [old],
            |r| r.get(0),
        )?;
        // Anyone can put any display name on their own address, so only merge addresses
        // the user has a real relationship with.
        if !is_name || !self.trusted(email)? {
            return Ok(());
        }
        self.conn.execute(
            "UPDATE OR IGNORE entity_mention SET entity_id = ?1 WHERE entity_id = ?2",
            params![id, old],
        )?;
        self.conn
            .execute("DELETE FROM entity_mention WHERE entity_id = ?1", [old])?;
        self.conn
            .execute("DELETE FROM entity WHERE id = ?1", [old])?;
        if let Some(w) = m.remove(&old) {
            Self::add(m, id, w);
        }
        self.people.insert(k.to_string(), id);
        Ok(())
    }

    fn name_entity(&self, k: &str) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT id FROM entity WHERE kind = 'person' AND key = ?1",
                [format!("name:{k}")],
                |r| r.get(0),
            )
            .optional()
    }

    /// Lowest-id email person with this name whose address passes [`Ctx::trusted`].
    fn trusted_email_person(&mut self, k: &str) -> rusqlite::Result<Option<i64>> {
        let cands: Vec<(i64, String)> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT id, substr(key, 7) FROM entity \
                 WHERE name_key = ?1 AND kind = 'person' AND key LIKE 'email:%' ORDER BY id",
            )?;
            let rows = stmt
                .query_map([k], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        for (id, email) in cands {
            if self.trusted(&email)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// A name-only attendee joins the `name:` person, else a trusted email person with that
    /// name, else a new `name:` person. Never an untrusted address: its display name is
    /// whatever the sender chose.
    fn name_person(&mut self, m: &mut Mentions, name: &str) -> rusqlite::Result<()> {
        let k = name_key(name);
        if k.is_empty() || k == ME {
            return Ok(());
        }
        let id = match self.people.get(&k) {
            Some(id) => *id,
            None => {
                let id = match self.name_entity(&k)? {
                    Some(id) => id,
                    None => match self.trusted_email_person(&k)? {
                        Some(id) => id,
                        None => {
                            self.entity(EntityKind::Person, &format!("name:{k}"), name.trim())?
                        }
                    },
                };
                self.people.insert(k, id);
                id
            }
        };
        Self::add(m, id, 1.0);
        Ok(())
    }

    /// Header string(s) (`From`/`To`) -> people + orgs.
    fn addresses(&mut self, m: &mut Mentions, raw: &str) -> rusqlite::Result<()> {
        for a in parse_addresses(raw) {
            self.email_person(m, &a.email, a.name.as_deref())?;
        }
        Ok(())
    }

    /// Attendee strings: emails become people plus an org, plain names become people.
    fn attendees(&mut self, m: &mut Mentions, list: &[String]) -> rusqlite::Result<()> {
        for a in list {
            if a.contains('@') && !parse_addresses(a).is_empty() {
                self.addresses(m, a)?;
            } else {
                self.name_person(m, a)?;
            }
        }
        Ok(())
    }

    fn phrases(&self, m: &mut Mentions, text: &str, max: usize, w: f64) -> rusqlite::Result<()> {
        for p in extract_phrases(text, max) {
            let id = self.entity(EntityKind::Phrase, &p, &p)?;
            Self::add(m, id, w);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    /// Writes (or refreshes) the source, its vector and its mentions.
    fn write(
        &mut self,
        kind: &str,
        ref_id: i64,
        ts: i64,
        title: &str,
        text: &str,
        vec: Option<Vec<f32>>,
        mentions: &Mentions,
    ) -> rusqlite::Result<i64> {
        let sid: i64 = self.conn.query_row(
            "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(kind, ref_id) WHERE ref_id IS NOT NULL DO UPDATE SET \
               ts_unix_ms = excluded.ts_unix_ms, title = excluded.title, \
               indexed_at_unix_ms = excluded.indexed_at_unix_ms \
             RETURNING id",
            params![kind, ref_id, ts, title, self.now],
            |r| r.get(0),
        )?;
        super::steps::attach_source(self.conn, self.own.as_deref(), sid)?;
        // Re-indexing can leave entities with no mentions: let topic_cluster prune.
        if self
            .conn
            .execute("DELETE FROM entity_mention WHERE source_id = ?1", [sid])?
            > 0
        {
            kv_set(self.conn, PRUNE_DIRTY_KEY, "1")?;
        }
        for (eid, w) in mentions {
            self.conn.execute(
                "INSERT INTO entity_mention (entity_id, source_id, weight) VALUES (?1, ?2, ?3)",
                params![eid, sid, w],
            )?;
        }
        // Redacted before the cut (and again when sent to Jev).
        let cap = if kind == "episode" { 1_500 } else { 2_000 };
        let text: String = crate::redact::redact_secrets(text)
            .chars()
            .take(cap)
            .collect();
        self.conn.execute(
            "INSERT INTO topic_source_text (source_id, text) VALUES (?1, ?2) \
             ON CONFLICT(source_id) DO UPDATE SET text = excluded.text",
            params![sid, text],
        )?;
        self.conn
            .execute("DELETE FROM topic_source_vec WHERE rowid = ?1", [sid])?;
        let vec = vec.filter(|v| v.len() == EMBEDDING_DIM);
        if let Some(v) = &vec {
            put_vec(self.conn, sid, v)?;
        }
        // Episodes embed their chrome-stripped text after commit; the mean of the raw capture
        // vectors (chrome included) stays only if that embedding fails.
        if vec.is_none() || kind == "episode" {
            self.pending.push((sid, text));
        }
        Ok(sid)
    }

    #[allow(clippy::chunks_exact_to_as_chunks)]
    fn item_vec(&self, id: i64) -> Option<Vec<f32>> {
        let blob: Vec<u8> = self
            .conn
            .query_row(
                "SELECT embedding FROM items_vec WHERE rowid = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .ok()??;
        Some(
            blob.chunks_exact(4) // ponytail: as_chunks needs a newer toolchain than CI
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
        )
    }

    fn mean_vec(&self, ids: &[i64]) -> Option<Vec<f32>> {
        let vs: Vec<Vec<f32>> = ids
            .iter()
            .take(VEC_SAMPLE)
            .filter_map(|i| self.item_vec(*i))
            .filter(|v| v.len() == EMBEDDING_DIM)
            .collect();
        if vs.is_empty() {
            return None;
        }
        let mut out = vec![0f32; EMBEDDING_DIM];
        for v in &vs {
            out.iter_mut().zip(v).for_each(|(o, x)| *o += x);
        }
        out.iter_mut().for_each(|o| *o /= vs.len() as f32);
        Some(out)
    }
}

fn put_vec(conn: &Connection, sid: i64, v: &[f32]) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO topic_source_vec (rowid, embedding) VALUES (?1, ?2)",
        params![sid, embedding_to_json(v)],
    )?;
    Ok(())
}

fn json_strings(raw: Option<String>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

fn hw_get(conn: &Connection, kind: &str) -> i64 {
    conn.query_row(
        "SELECT value FROM kv_meta WHERE key = ?1",
        [format!("topic_index_hw:{kind}")],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(0)
}

fn kv_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Indexes up to `batch` new sources of each kind. Returns true while work remains.
pub fn run_topic_index_job(store: &Store, batch: usize) -> rusqlite::Result<bool> {
    let tx = store.conn.unchecked_transaction()?;
    chrome::rebuild_chrome_if_stale(&tx, now_ms())?;
    let mut cx = Ctx::new(&tx);
    cx.own = store.own_device_id();
    let mut more = false;
    for kind in ["email", "calendar_event", "snapshot"] {
        more |= index_items(&mut cx, kind, batch)?;
    }
    more |= index_meetings(&mut cx, batch)?;
    more |= index_episodes(&mut cx, batch)?;
    let pending = std::mem::take(&mut cx.pending);
    tx.commit()?;
    // Embedding can be slow; do it outside the write transaction.
    for (sid, text) in pending {
        if let Ok(v) = store.embed(&text) {
            let live = store
                .conn
                .query_row(
                    "SELECT 1 FROM topic_source WHERE id = ?1",
                    [sid],
                    |_| Ok(()),
                )
                .is_ok();
            if live && v.len() == EMBEDDING_DIM {
                store
                    .conn
                    .execute("DELETE FROM topic_source_vec WHERE rowid = ?1", [sid])?;
                put_vec(&store.conn, sid, &v)?;
            }
        }
    }
    Ok(more)
}

/// One key for every occurrence of a recurring event: the id before the last `#`, minus a
/// trailing `_YYYYMMDDTHHMMSS[Z]` instance suffix (Google), prefixed by the source.
pub(crate) fn calendar_series_key(source: &str, external_id: &str) -> String {
    let id = external_id.rsplit_once('#').map_or(external_id, |(a, _)| a);
    let b = id.as_bytes();
    let z = usize::from(b.last() == Some(&b'Z'));
    let n = b.len() - z;
    let stamp = n >= 16
        && b[n - 16] == b'_'
        && b[n - 15..n - 7].iter().all(u8::is_ascii_digit)
        && b[n - 7] == b'T'
        && b[n - 6..n].iter().all(u8::is_ascii_digit);
    format!("{source}:{}", if stamp { &id[..n - 16] } else { id })
}

fn index_items(cx: &mut Ctx, kind: &str, batch: usize) -> rusqlite::Result<bool> {
    let hw = hw_get(cx.conn, kind);
    struct Row {
        id: i64,
        ts: i64,
        actors: Option<String>,
        title: Option<String>,
        body: String,
        bulk: bool,
        unscored: bool,
        source: Option<String>,
        ext: Option<String>,
    }
    let rows: Vec<Row> = {
        let mut stmt = cx.conn.prepare(
            "SELECT i.id, i.ts_unix_ms, i.actors, i.title, i.body_text, \
               COALESCE(json_extract(i.refs, '$.list_unsubscribe') = 1, 0) OR EXISTS ( \
                 SELECT 1 FROM threads t JOIN thread_priority p ON p.thread_id = t.id \
                 WHERE t.source = 'gmail' AND t.external_thread_id = json_extract(i.refs, '$.gmail_thread_id') \
                   AND t.account_email = json_extract(i.refs, '$.account_email') \
                   AND (p.bulk >= 0.5 OR p.automated >= 0.5)), \
               (json_extract(i.refs, '$.gmail_thread_id') IS NOT NULL AND NOT EXISTS ( \
                 SELECT 1 FROM threads t JOIN thread_priority p ON p.thread_id = t.id \
                 WHERE t.source = 'gmail' AND t.external_thread_id = json_extract(i.refs, '$.gmail_thread_id') \
                   AND t.account_email = json_extract(i.refs, '$.account_email'))), \
               i.source, i.external_id \
             FROM items i WHERE i.kind = ?1 AND i.id > ?2 ORDER BY i.id LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![kind, hw, batch as i64], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    actors: r.get(2)?,
                    title: r.get(3)?,
                    body: r.get(4)?,
                    bulk: r.get::<_, i64>(5)? != 0,
                    unscored: r.get::<_, i64>(6)? != 0,
                    source: r.get(7)?,
                    ext: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    let full = rows.len() == batch;
    for r in &rows {
        // Wait (up to an hour) for Reply Priorities to score the thread, so bulk mail is
        // never indexed just because it arrived first. The mark stays; later rows wait too.
        if kind == "email" && r.unscored && cx.now - r.ts < HOLD_MS {
            return Ok(false);
        }
        // ponytail: re-scans today's future events each run; fine for one day's window
        let future = kind == "calendar_event" && r.ts > cx.now;
        if future && r.ts <= cx.now + DAY_MS {
            return Ok(false);
        }
        let title = r.title.clone().unwrap_or_default();
        let series = match (kind, &r.source, &r.ext) {
            ("calendar_event", Some(s), Some(e)) => Some(calendar_series_key(s, e)),
            _ => None,
        };
        // Further than a day ahead: never indexed, and must not stall the mark.
        let mut skip_older = future;
        if let Some(key) = series.as_ref().filter(|_| !future) {
            let cur: Option<(i64, i64)> = cx
                .conn
                .query_row(
                    "SELECT s.id, s.ts_unix_ms FROM topic_series t \
                     JOIN topic_source s ON s.id = t.source_id WHERE t.series_key = ?1",
                    [key],
                    |x| Ok((x.get(0)?, x.get(1)?)),
                )
                .optional()?;
            if let Some((sid, cur_ts)) = cur {
                skip_older = r.ts <= cur_ts;
                if !skip_older {
                    // Repoint; the write below then refreshes it as a re-index.
                    cx.conn.execute(
                        "UPDATE topic_source SET ref_id = ?1 WHERE id = ?2",
                        params![r.id, sid],
                    )?;
                }
            }
        }
        if !skip_older && !(kind == "email" && r.bulk) {
            let mut m = Mentions::new();
            let mut head_len = None;
            let text = match kind {
                "email" => {
                    for raw in json_strings(r.actors.clone()) {
                        cx.addresses(&mut m, &raw)?;
                    }
                    cx.phrases(&mut m, &title, 8, 0.8)?;
                    cx.phrases(&mut m, &head(&r.body), 12, 0.4)?;
                    let from = json_strings(r.actors.clone())
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    head_len = Some(title.chars().count() + 1 + from.chars().count());
                    format!("{title}\n{from}\n{}", head(&r.body))
                }
                "calendar_event" => {
                    cx.attendees(&mut m, &json_strings(r.actors.clone()))?;
                    cx.phrases(&mut m, &title, 8, 0.8)?;
                    title.clone()
                }
                _ => {
                    cx.phrases(&mut m, &r.body, 12, 0.6)?;
                    head(&r.body)
                }
            };
            let vec = cx.item_vec(r.id);
            let sid = cx.write(kind, r.id, r.ts, &title, &text, vec, &m)?;
            set_head_len(cx.conn, sid, head_len)?;
            if let Some(key) = &series {
                cx.conn.execute(
                    "INSERT OR REPLACE INTO topic_series (series_key, source_id) VALUES (?1, ?2)",
                    params![key, sid],
                )?;
            }
        }
        kv_set(
            cx.conn,
            &format!("topic_index_hw:{kind}"),
            &r.id.to_string(),
        )?;
    }
    Ok(full)
}

/// Ids of `table` rows for `kind` to (re)index: rows edited since indexing (`topic_dirty`,
/// set by triggers, cleared here) and up to `batch` rows past the id high-water mark, which
/// advances over rows not ready yet (a running meeting, an unsummarized episode): finishing
/// them is an edit. Returns (ids, more work left).
fn todo_ids(
    cx: &Ctx,
    kind: &str,
    table: &str,
    batch: usize,
) -> rusqlite::Result<(BTreeSet<i64>, bool)> {
    let dirty: Vec<i64> = {
        let mut stmt = cx.conn.prepare_cached(
            "SELECT ref_id FROM topic_dirty WHERE kind = ?1 ORDER BY ref_id LIMIT ?2",
        )?;
        let ids = stmt
            .query_map(params![kind, batch as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids
    };
    if let Some(last) = dirty.last() {
        cx.conn.execute(
            "DELETE FROM topic_dirty WHERE kind = ?1 AND ref_id <= ?2",
            params![kind, last],
        )?;
    }
    let fresh: Vec<i64> = {
        let mut stmt = cx.conn.prepare(&format!(
            "SELECT id FROM {table} WHERE id > ?1 ORDER BY id LIMIT ?2"
        ))?;
        let ids = stmt
            .query_map(params![hw_get(cx.conn, kind), batch as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids
    };
    if let Some(last) = fresh.last() {
        kv_set(
            cx.conn,
            &format!("topic_index_hw:{kind}"),
            &last.to_string(),
        )?;
    }
    let more = dirty.len() == batch || fresh.len() == batch;
    Ok((dirty.into_iter().chain(fresh).collect(), more))
}

fn index_meetings(cx: &mut Ctx, batch: usize) -> rusqlite::Result<bool> {
    let (ids, more) = todo_ids(cx, "meeting", "meeting", batch)?;
    for id in ids {
        #[allow(clippy::type_complexity)]
        let row: Option<(i64, String, String, Option<String>, Option<String>)> = cx
            .conn
            .prepare_cached(
                "SELECT m.start_unix_ms, m.title, m.attendees, \
                        COALESCE(d.summary_edited, m.summary_tldr), \
                        (SELECT group_concat(text, char(10)) FROM meeting_action_item WHERE meeting_id = m.id) \
                 FROM meeting m LEFT JOIN meeting_note_draft d ON d.meeting_id = m.id \
                 WHERE m.id = ?1 AND m.end_unix_ms IS NOT NULL",
            )?
            .query_row([id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .optional()?;
        let Some((start, title, attendees, summary, actions)) = row else {
            continue; // still running (its end is an edit) or deleted
        };
        let mut m = Mentions::new();
        cx.attendees(&mut m, &json_strings(Some(attendees)))?;
        cx.phrases(&mut m, &title, 8, 0.8)?;
        let notes = [summary.unwrap_or_default(), actions.unwrap_or_default()]
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        // Via transcript_segment(meeting_id) + idx_items_source, not a scan of `items`.
        // ponytail: transcript rows synced from a peer have no local segment and are not seen.
        let (transcript, text): (Vec<i64>, String) = {
            let mut stmt = cx.conn.prepare(
                "SELECT i.id, i.body_text FROM transcript_segment t \
                 JOIN items i ON i.source_table = 'transcript_segment' AND i.source_id = t.id \
                 WHERE t.meeting_id = ?1 ORDER BY t.start_cs, t.id",
            )?;
            let mut ids = Vec::new();
            let mut text = String::new();
            for row in
                stmt.query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            {
                let (iid, body) = row?;
                ids.push(iid);
                if text.chars().count() < TEXT_CHARS {
                    text.push_str(&body);
                    text.push(' ');
                }
            }
            (ids, text)
        };
        let body = if notes.is_empty() { head(&text) } else { notes };
        let vec = cx.mean_vec(&transcript);
        let sid = cx.write(
            "meeting",
            id,
            start,
            &title,
            &format!("{title}\n{body}"),
            vec,
            &m,
        )?;
        set_head_len(cx.conn, sid, Some(title.chars().count()))?;
    }
    Ok(more)
}

/// Records where a source's head (title, subject and sender) ends in its stored text; `None`
/// for kinds whose whole text counts (calendar events, snapshots).
fn set_head_len(conn: &Connection, sid: i64, head_len: Option<usize>) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE topic_source_text SET head_len = ?2 WHERE source_id = ?1",
        params![sid, head_len.map(|h| h as i64)],
    )?;
    Ok(())
}

fn index_episodes(cx: &mut Ctx, batch: usize) -> rusqlite::Result<bool> {
    let (ids, more) = todo_ids(cx, "episode", "episodes", batch)?;
    let chrome = chrome::load_chrome(cx.conn)?;
    for id in ids {
        #[allow(clippy::type_complexity)]
        let row: Option<(i64, String, String, String, i64, i64, bool, String)> = cx
            .conn
            .prepare_cached(
                "SELECT start_ts_unix_ms, COALESCE(window_title, ''), COALESCE(summary, ''), \
                        device_id, start_item_id, end_item_id, hidden = 1, COALESCE(app_bundle_id, '') \
                 FROM episodes WHERE id = ?1",
            )?
            .query_row([id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })
            .optional()?;
        let Some((start, title, summary, device, lo, hi, hidden, app)) = row else {
            continue;
        };
        if hidden {
            // Hidden after indexing: drop the source (triggers clear mentions and vector).
            cx.conn.execute(
                "DELETE FROM topic_source WHERE kind = 'episode' AND ref_id = ?1",
                [id],
            )?;
            continue;
        }
        let has_summary = !summary.trim().is_empty();
        let lines = chrome::capture_lines(cx.conn, &device, &app, lo, hi, &chrome)?;
        let mut m = Mentions::new();
        cx.phrases(&mut m, &title, 8, 0.8)?;
        cx.phrases(&mut m, &summary, 12, 0.4)?;
        if !has_summary {
            // The period ends a capitalized run, so no phrase spans a cut-out chrome gap.
            cx.phrases(&mut m, &lines.join(".\n"), 12, 0.4)?;
        }
        let caps: Vec<i64> = {
            let mut stmt = cx.conn.prepare(
                "SELECT id FROM items WHERE kind = 'capture' AND device_id = ?1 AND id BETWEEN ?2 AND ?3 ORDER BY id LIMIT ?4",
            )?;
            let ids = stmt
                .query_map(params![device, lo, hi, VEC_SAMPLE as i64], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            ids
        };
        let vec = cx.mean_vec(&caps);
        let (text, head_len) = chrome::episode_topic_text(&summary, &title, &lines);
        let sid = cx.write("episode", id, start, &title, &text, vec, &m)?;
        set_head_len(cx.conn, sid, Some(head_len))?;
        // Too little text and no summary: not enough to seed or join a topic.
        let noise = !has_summary && text.trim().chars().count() < 80;
        cx.conn.execute(
            "UPDATE topic_source SET noise = ?2 WHERE id = ?1",
            params![sid, noise],
        )?;
    }
    Ok(more)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::FakeEmbedder;
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s =
            Store::open_with_embedder(&dir.path().join("t.db"), Arc::new(FakeEmbedder)).unwrap();
        (dir, s)
    }

    #[test]
    fn episode_vector_embeds_its_stored_text_not_raw_captures() {
        let (_d, s) = store();
        s.conn.execute("INSERT INTO items (device_id, kind, source, ts_unix_ms, body_text) VALUES ('d','capture','ax',1,'Falcon launch plan review with the pricing team')", []).unwrap();
        let item = s.conn.last_insert_rowid();
        let raw: Vec<u8> = vec![0.5f32; EMBEDDING_DIM]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        s.conn
            .execute(
                "INSERT INTO items_vec (rowid, embedding) VALUES (?1, ?2)",
                params![item, raw],
            )
            .unwrap();
        s.conn.execute("INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('d','a','Doc',1,2,?1,?1,1,'summarize_failed',1,1)", [item]).unwrap();
        while run_topic_index_job(&s, 50).unwrap() {}
        let (sid, text): (i64, String) = s
            .conn
            .query_row(
                "SELECT s.id, t.text FROM topic_source s JOIN topic_source_text t ON t.source_id = s.id WHERE s.kind = 'episode'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let v = crate::topics::cluster::source_vec(&s.conn, sid)
            .unwrap()
            .unwrap();
        assert_eq!(v, s.embed(&text).unwrap());
    }

    fn n(s: &Store, sql: &str) -> i64 {
        s.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn email(s: &Store, ext: &str, from: &str, to: &str, subject: &str, refs: &str) -> i64 {
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                 VALUES ('d','email','gmail',1000,json_array(?1, ?2),?3,'Budget review for Project Falcon',?4,?5)",
                params![from, to, subject, refs, ext],
            )
            .unwrap();
        s.conn.last_insert_rowid()
    }

    fn meeting(s: &Store, end: Option<i64>, attendees: &str) -> i64 {
        s.conn
            .execute(
                "INSERT INTO meeting (device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, status) \
                 VALUES ('d', 1000, ?1, 'Falcon Sync', ?2, 'manual', 'completed')",
                params![end, attendees],
            )
            .unwrap();
        s.conn.last_insert_rowid()
    }

    fn people(s: &Store) -> i64 {
        n(s, "SELECT COUNT(*) FROM entity WHERE kind = 'person'")
    }

    fn bare_episode(s: &Store, title: &str, summary: Option<&str>, status: &str) -> i64 {
        s.conn.execute("INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('d','a',?1,1,2,1,2,2,?2,?3,1,1)", params![title, summary, status]).unwrap();
        s.conn.last_insert_rowid()
    }

    fn occurrence(s: &Store, ext: &str, ts: i64) -> i64 {
        s.conn.execute("INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) VALUES ('d','calendar_event','google_calendar',?1,json_array('Ann Roe'),'Weekly Sync','',?2)", params![ts, ext]).unwrap();
        s.conn.last_insert_rowid()
    }

    /// A key straddling the stored-text cap is redacted before the cut, so no part of it
    /// is stored.
    #[test]
    fn secret_at_the_text_cap_is_redacted_before_the_cut() {
        let (_d, s) = store();
        let key = format!("sk-ant-api03-{}", "a".repeat(50));
        let body = format!("{} {key} tail", "x".repeat(1960));
        email(&s, "e1", "a@x.com", "b@y.com", "T", "{}");
        s.conn
            .execute("UPDATE items SET body_text = ?1", [&body])
            .unwrap();
        while run_topic_index_job(&s, 200).unwrap() {}
        let text: String = s
            .conn
            .query_row("SELECT text FROM topic_source_text", [], |r| r.get(0))
            .unwrap();
        assert!(text.starts_with("T\na@x.com\nxxx"), "{}", &text[..20]);
        assert!(!text.contains(&key[..8]), "{}", &text[1950..]);
    }

    #[test]
    fn google_occurrence_ids_share_a_series_key() {
        let a = calendar_series_key("google_calendar", "abc123_20261105T150000Z#1794000000");
        let b = calendar_series_key("google_calendar", "abc123_20261112T150000Z#1794600000");
        assert_eq!(a, b);
        assert_eq!(a, "google_calendar:abc123");
    }

    #[test]
    fn apple_ids_share_a_series_key() {
        assert_eq!(
            calendar_series_key("apple_calendar", "E1F2#100"),
            "apple_calendar:E1F2"
        );
        assert_eq!(
            calendar_series_key("apple_calendar", "E1F2#200"),
            "apple_calendar:E1F2"
        );
    }

    #[test]
    fn three_occurrences_make_one_source_dated_by_the_latest_past() {
        let (_d, s) = store();
        occurrence(&s, "abc_20261105T150000Z#1", 1000);
        occurrence(&s, "abc_20261112T150000Z#2", 2000);
        let c = occurrence(&s, "abc_20261119T150000Z#3", 3000);
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM topic_source WHERE kind = 'calendar_event'"
            ),
            1
        );
        assert_eq!(
            n(
                &s,
                "SELECT ref_id FROM topic_source WHERE kind = 'calendar_event'"
            ),
            c
        );
        assert_eq!(
            n(
                &s,
                "SELECT ts_unix_ms FROM topic_source WHERE kind = 'calendar_event'"
            ),
            3000
        );
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_series"), 1);
        // An older occurrence arriving later is skipped.
        occurrence(&s, "abc_20261001T150000Z#0", 500);
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(
                &s,
                "SELECT ts_unix_ms FROM topic_source WHERE kind = 'calendar_event'"
            ),
            3000
        );
        // Deleting the source drops the series row.
        s.conn.execute("DELETE FROM topic_source", []).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_series"), 0);
    }

    #[test]
    fn future_occurrence_is_not_indexed_until_it_has_started() {
        let (_d, s) = store();
        let id = occurrence(&s, "abc_20261105T150000Z#1", now_ms() + 3_600_000);
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM topic_source WHERE kind = 'calendar_event'"
            ),
            0
        );
        assert_eq!(hw_get(&s.conn, "calendar_event"), 0);
        s.conn
            .execute("UPDATE items SET ts_unix_ms = 1000 WHERE id = ?1", [id])
            .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM topic_source WHERE kind = 'calendar_event'"
            ),
            1
        );
    }

    #[test]
    fn far_future_occurrence_does_not_stall_later_past_ones() {
        let (_d, s) = store();
        occurrence(&s, "far_20261110T150000Z#1", now_ms() + 3 * DAY_MS);
        occurrence(&s, "past_20261101T150000Z#2", 1000);
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM topic_source WHERE kind = 'calendar_event'"
            ),
            1
        );
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM topic_series WHERE series_key LIKE '%:far'"
            ),
            0
        );
    }

    #[test]
    fn unsummarized_episode_is_indexed_and_reindexed_when_summarized() {
        let (_d, s) = store();
        let long = "Reviewing the Falcon launch checklist with the whole team today";
        let ep = bare_episode(&s, long, None, "pending");
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM topic_source WHERE kind = 'episode'"
            ),
            1
        );
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_dirty"), 0);
        s.conn
            .execute(
                "UPDATE episodes SET summary = 'Finished the Falcon checklist' WHERE id = ?1",
                [ep],
            )
            .unwrap();
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM topic_dirty WHERE kind = 'episode'"
            ),
            1
        );
        run_topic_index_job(&s, 200).unwrap();
        let text: String = s
            .conn
            .query_row("SELECT text FROM topic_source_text", [], |r| r.get(0))
            .unwrap();
        assert!(text.contains("\nFinished the Falcon checklist"), "{text}");
    }

    #[test]
    fn short_unsummarized_episode_is_noise_and_never_clustered() {
        let (_d, s) = store();
        bare_episode(&s, "Untitled", None, "pending");
        bare_episode(
            &s,
            "Planning the Falcon launch with everyone on the team this week, covering budget and schedule",
            None,
            "pending",
        );
        bare_episode(&s, "Untitled", Some("ok"), "summarized");
        run_topic_index_job(&s, 200).unwrap();
        let noise = |t: &str| {
            n(&s, &format!("SELECT s.noise FROM topic_source s JOIN topic_source_text x ON x.source_id = s.id WHERE x.text LIKE '{t}%'"))
        };
        assert_eq!(noise("Untitled"), 1, "first row (no summary) is noise");
        assert_eq!(noise("Planning"), 0);
        crate::topics::cluster::run_topic_cluster_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source WHERE noise = 1 AND assigned_at_unix_ms IS NOT NULL"), 0);
    }

    #[test]
    fn indexes_each_source_kind_once() {
        let (_d, s) = store();
        email(&s, "e1", "a@x.com", "b@y.com", "Hello", "{}");
        s.conn.execute("INSERT INTO items (device_id, kind, ts_unix_ms, actors, title, body_text, external_id, source) VALUES ('d','calendar_event',1,json_array('Ann Roe'),'Planning','', 'c1','gcal')", []).unwrap();
        s.conn.execute("INSERT INTO items (device_id, kind, ts_unix_ms, title, body_text, source_table) VALUES ('d','snapshot',1,'Slide','Quarterly Revenue Chart','meeting_snapshot')", []).unwrap();
        meeting(&s, Some(2000), "[]");
        s.conn.execute("INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('d','a','Doc',1,2,1,2,2,'did things','summarized',1,1)", []).unwrap();
        assert!(!run_topic_index_job(&s, 200).unwrap());
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 5);
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source_vec"), 5);
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 5);
    }

    #[test]
    fn self_address_is_never_an_entity() {
        let (_d, s) = store();
        s.conn.execute("INSERT INTO connector_account (connector, email, is_default, position, connected_at_unix_ms) VALUES ('gmail','Me@acme.com',1,0,0)", []).unwrap();
        email(&s, "e1", "me@acme.com", "Bob <bob@acme.com>", "Hi", "{}");
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(people(&s), 1);
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM entity WHERE kind='person' AND key='email:bob@acme.com'"
            ),
            1
        );
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM entity WHERE kind='org' AND key='acme.com'"
            ),
            1
        );
    }

    #[test]
    fn bulk_mail_is_skipped() {
        let (_d, s) = store();
        email(
            &s,
            "e1",
            "a@x.com",
            "b@y.com",
            "Sale",
            r#"{"list_unsubscribe":true}"#,
        );
        let id = email(
            &s,
            "e2",
            "a@x.com",
            "b@y.com",
            "Newsletter",
            r#"{"gmail_thread_id":"t1","account_email":"b@y.com"}"#,
        );
        s.conn.execute("INSERT INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('gmail','t1','b@y.com','s',1,1,'a','open',1,1)", []).ok();
        let tid = s.conn.last_insert_rowid();
        s.conn
            .execute(
                "INSERT INTO thread_priority VALUES (?1,1,0,0,0,0.9,0,0)",
                [tid],
            )
            .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 0, "id {id}");
        assert_eq!(hw_get(&s.conn, "email"), id);
    }

    #[test]
    fn one_person_many_spellings() {
        let (_d, s) = store();
        email(
            &s,
            "e1",
            "Sarah Lee <sarah@acme.com>",
            "b@y.com",
            "Hi",
            "{}",
        );
        // Trusted: she attended a recorded meeting.
        meeting(&s, Some(1), r#"["sarah@acme.com"]"#);
        meeting(&s, Some(2), r#"["Sarah Lee","Lee, Sarah"]"#);
        run_topic_index_job(&s, 200).unwrap();
        let sarah = n(
            &s,
            "SELECT COUNT(*) FROM entity WHERE kind='person' AND display LIKE '%Sarah%'",
        );
        assert_eq!(sarah, 1);
        assert_eq!(n(&s, "SELECT COUNT(*) FROM entity_mention WHERE entity_id = (SELECT id FROM entity WHERE key='email:sarah@acme.com')"), 3);
    }

    fn mentions_of(s: &Store, key: &str) -> i64 {
        s.conn
            .query_row(
                "SELECT COUNT(*) FROM entity_mention WHERE entity_id = (SELECT id FROM entity WHERE key = ?1)",
                [key],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn name_only_attendee_never_joins_an_untrusted_email_person() {
        // Across two ticks and within one: the attacker's entity has the lower id.
        for one_tick in [false, true] {
            let (_d, s) = store();
            email(
                &s,
                "e1",
                "Sarah Lee <attacker@evil.com>",
                "b@y.com",
                "Hi",
                "{}",
            );
            if !one_tick {
                run_topic_index_job(&s, 200).unwrap();
            }
            meeting(&s, Some(2), r#"["Sarah Lee"]"#);
            run_topic_index_job(&s, 200).unwrap();
            assert_eq!(mentions_of(&s, "email:attacker@evil.com"), 1, "{one_tick}");
            assert_eq!(mentions_of(&s, "name:sarah lee"), 1, "{one_tick}");
        }
        // A trusted Sarah with a higher id is the one joined.
        let (_d, s) = store();
        meeting(&s, Some(1), r#"["sarah@acme.com"]"#);
        email(
            &s,
            "e1",
            "Sarah Lee <attacker@evil.com>",
            "b@y.com",
            "Hi",
            "{}",
        );
        email(&s, "e2", SARAH, "b@y.com", "Hi", "{}");
        run_topic_index_job(&s, 200).unwrap();
        meeting(&s, Some(3), r#"["Sarah Lee"]"#);
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(mentions_of(&s, "email:attacker@evil.com"), 1);
        assert_eq!(mentions_of(&s, "email:sarah@acme.com"), 3);
        assert_eq!(mentions_of(&s, "name:sarah lee"), 0);
    }

    #[test]
    fn mailto_attendee_and_sender_are_one_person() {
        let (_d, s) = store();
        meeting(&s, Some(2), r#"["mailto:Sarah@Acme.com"]"#);
        email(&s, "e1", SARAH, "b@y.com", "Hi", "{}");
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(mentions_of(&s, "email:sarah@acme.com"), 2);
        assert_eq!(
            n(&s, "SELECT COUNT(*) FROM entity WHERE key LIKE '%mailto%'"),
            0
        );
    }

    #[test]
    fn trust_is_memoized_per_call_and_people_load_lazily() {
        let (_d, s) = store();
        meeting(&s, Some(2), r#"["sarah@acme.com"]"#);
        run_topic_index_job(&s, 200).unwrap();
        let mut cx = Ctx::new(&s.conn);
        assert!(cx.people.is_empty(), "no up-front load of every person");
        assert!(cx.trusted("sarah@acme.com").unwrap());
        s.conn.execute("DELETE FROM meeting", []).unwrap();
        assert!(
            cx.trusted("sarah@acme.com").unwrap(),
            "second lookup is cached"
        );
        assert!(!Ctx::new(&s.conn).trusted("sarah@acme.com").unwrap());
    }

    #[test]
    fn edits_reach_the_index_through_dirty_marks() {
        let (_d, s) = store();
        let m = meeting(&s, Some(2), "[]");
        s.conn.execute("INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('d','a','Doc',1,2,1,2,2,'x','summarized',1,1)", []).unwrap();
        let ep = s.conn.last_insert_rowid();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            (hw_get(&s.conn, "meeting"), hw_get(&s.conn, "episode")),
            (m, ep)
        );
        let stale = || {
            s.conn
                .execute("UPDATE topic_source SET indexed_at_unix_ms = 7", [])
                .unwrap()
        };
        let fresh = |kind: &str| {
            s.conn
                .query_row(
                    "SELECT indexed_at_unix_ms != 7 FROM topic_source WHERE kind = ?1",
                    [kind],
                    |r| r.get::<_, bool>(0),
                )
                .unwrap()
        };
        stale();
        run_topic_index_job(&s, 200).unwrap();
        assert!(
            !fresh("meeting") && !fresh("episode"),
            "idle tick re-indexes nothing"
        );
        s.conn.execute("INSERT INTO meeting_action_item (meeting_id, text, order_index, source, created_unix_ms, updated_unix_ms) VALUES (?1, 'Send Falcon Plan', 0, 'llm', 1, 1)", [m]).unwrap();
        // Summary rewritten without touching updated_at (lib.rs re-summarize path).
        s.conn
            .execute(
                "UPDATE episodes SET summary = 'Falcon Plan draft' WHERE id = ?1",
                [ep],
            )
            .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert!(fresh("meeting") && fresh("episode"));
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_dirty"), 0);
    }

    #[test]
    fn last_comma_first_alone_joins_a_known_person() {
        let (_d, s) = store();
        meeting(&s, Some(2), r#"["Lee, Sarah"]"#);
        meeting(&s, Some(3), r#"["Sarah Lee"]"#);
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM entity WHERE key = 'name:sarah lee'"
            ),
            1
        );
        assert_eq!(people(&s), 1);
    }

    #[test]
    fn works_without_an_embedding_model() {
        let s = Store::open_in_memory().unwrap();
        email(&s, "e1", "a@x.com", "b@y.com", "Hello", "{}");
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 1);
        assert!(n(&s, "SELECT COUNT(*) FROM entity_mention") > 0);
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source_vec"), 0);
    }

    #[test]
    fn synced_items_are_indexed() {
        let (_d, s) = store();
        let item = crate::AppliedItem {
            kind: "email".into(),
            source: Some("gmail".into()),
            ts_unix_ms: 5,
            actors: Some(r#"["a@x.com","b@y.com"]"#.into()),
            title: Some("Synced".into()),
            body_text: "Body".into(),
            refs: None,
            flags: None,
            hash: None,
            external_id: Some("sx".into()),
            app_bundle_id: None,
            url: None,
        };
        s.apply_synced_item("peer", 1, None, &item).unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(
                &s,
                "SELECT COUNT(*) FROM topic_source WHERE title = 'Synced'"
            ),
            1
        );
    }

    #[test]
    fn hidden_episodes_are_skipped() {
        let (_d, s) = store();
        s.conn.execute("INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms, hidden) VALUES ('d','a','Doc',1,2,1,2,2,'x','summarized',1,1,1)", []).unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 0);
    }

    #[test]
    fn running_meetings_wait_and_summary_changes_reindex() {
        let (_d, s) = store();
        let running = meeting(&s, None, "[]");
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 0);
        s.conn
            .execute(
                "UPDATE meeting SET end_unix_ms = 9 WHERE id = ?1",
                [running],
            )
            .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 1);
        s.conn
            .execute("UPDATE topic_source SET indexed_at_unix_ms = 7", [])
            .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(&s, "SELECT indexed_at_unix_ms FROM topic_source"),
            7,
            "unchanged stays put"
        );
        s.update_meeting_summary(
            running,
            "Discussed Acme Corp Renewal",
            &[],
            &[],
            &[],
            "p",
            "m",
        )
        .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 1);
        assert_ne!(
            n(&s, "SELECT indexed_at_unix_ms FROM topic_source"),
            7,
            "summary change re-indexes"
        );
    }
    #[test]
    fn episode_hidden_after_indexing_loses_its_source() {
        let (_d, s) = store();
        s.conn.execute("INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('d','a','Doc Title',1,2,1,2,2,'x','summarized',1,1)", []).unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 1);
        s.conn
            .execute("UPDATE episodes SET hidden = 1", [])
            .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 0);
        assert_eq!(n(&s, "SELECT COUNT(*) FROM entity_mention"), 0);
    }

    fn thread(s: &Store, ext: &str) -> i64 {
        s.conn.execute("INSERT INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('gmail',?1,'b@y.com','s',1,1,'a','open',1,1)", [ext]).unwrap();
        s.conn.last_insert_rowid()
    }

    #[test]
    fn unscored_recent_email_holds_the_mark_until_scored_or_old() {
        let (_d, s) = store();
        let refs = r#"{"gmail_thread_id":"t9","account_email":"b@y.com"}"#;
        let id = email(&s, "e1", "a@x.com", "b@y.com", "Promo", refs);
        s.conn
            .execute("UPDATE items SET ts_unix_ms = ?1", [now_ms()])
            .unwrap();
        let tid = thread(&s, "t9");
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 0);
        assert_eq!(hw_get(&s.conn, "email"), 0, "mark held");
        s.conn
            .execute(
                "INSERT INTO thread_priority VALUES (?1,1,0,0,0,0.9,0,0)",
                [tid],
            )
            .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(&s, "SELECT COUNT(*) FROM topic_source"),
            0,
            "bulk skipped"
        );
        assert_eq!(hw_get(&s.conn, "email"), id);
        // An unscored email older than an hour is indexed anyway.
        email(
            &s,
            "e2",
            "a@x.com",
            "b@y.com",
            "Old",
            r#"{"gmail_thread_id":"t8","account_email":"b@y.com"}"#,
        );
        thread(&s, "t8");
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM topic_source"), 1);
    }

    fn calendar(s: &Store, attendee: &str) {
        s.conn.execute("INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id) VALUES ('d','calendar_event','gcal',1,json_array(?1),'Sync','',?1)", [attendee]).unwrap();
    }

    fn sent_mail(s: &Store, ext: &str, from: &str, to: &str, labels: &str) {
        email(
            s,
            ext,
            from,
            to,
            "Out",
            &format!(r#"{{"gmail_labels":{labels},"account_email":"me@acme.com"}}"#),
        );
    }

    fn self_account(s: &Store) {
        s.conn.execute("INSERT INTO connector_account (connector, email, is_default, position, connected_at_unix_ms) VALUES ('gmail','me@acme.com',1,0,0)", []).unwrap();
    }

    /// Meeting "Sarah Lee" (name-only) indexed first, then `setup`, then an inbound email from
    /// `from`. Returns whether the name-only person was absorbed.
    fn absorbed(setup: impl Fn(&Store), from: &str) -> bool {
        let (_d, s) = store();
        self_account(&s);
        meeting(&s, Some(2), r#"["Sarah Lee"]"#);
        run_topic_index_job(&s, 200).unwrap();
        setup(&s);
        email(&s, "in1", from, "me@acme.com", "Hi", "{}");
        run_topic_index_job(&s, 200).unwrap();
        n(
            &s,
            "SELECT COUNT(*) FROM entity WHERE key = 'name:sarah lee'",
        ) == 0
    }

    const SARAH: &str = "Sarah Lee <sarah@acme.com>";

    #[test]
    fn later_email_person_absorbs_an_earlier_name_only_person() {
        // Trusted via a recorded meeting's attendee list.
        let (_d, s) = store();
        meeting(&s, Some(2), r#"["Sarah Lee"]"#);
        meeting(&s, Some(3), r#"["sarah@acme.com"]"#);
        run_topic_index_job(&s, 200).unwrap();
        email(&s, "e1", SARAH, "b@y.com", "Hi", "{}");
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(people(&s), 2, "sarah (merged) + b@y.com");
        assert_eq!(
            n(&s, "SELECT COUNT(*) FROM entity WHERE key LIKE 'name:%'"),
            0
        );
        assert_eq!(n(&s, "SELECT COUNT(*) FROM entity_mention WHERE entity_id = (SELECT id FROM entity WHERE key='email:sarah@acme.com')"), 3);
    }

    #[test]
    fn untrusted_sender_does_not_absorb_a_name_only_person() {
        assert!(!absorbed(|_| {}, "Sarah Lee <stranger@evil.com>"));
    }

    #[test]
    fn address_the_user_mailed_is_trusted() {
        assert!(absorbed(
            |s| sent_mail(
                s,
                "e0",
                "me@acme.com",
                "Sarah <sarah@acme.com>",
                r#"["SENT"]"#
            ),
            SARAH
        ));
    }

    #[test]
    fn from_self_without_sent_label_is_not_trust() {
        assert!(!absorbed(
            |s| sent_mail(s, "e0", "me@acme.com", "sarah@acme.com", r#"["INBOX"]"#),
            SARAH
        ));
    }

    #[test]
    fn calendar_invite_is_not_trust() {
        assert!(!absorbed(|s| calendar(s, "sarah@acme.com"), SARAH));
    }

    #[test]
    fn substring_of_a_meeting_attendee_is_not_trusted() {
        assert!(!absorbed(
            |s| {
                meeting(s, Some(5), r#"["sarah@acme.com"]"#);
            },
            "Sarah Lee <ah@acme.com>"
        ));
    }

    #[test]
    fn mailing_a_longer_address_does_not_trust_a_shorter_one() {
        assert!(!absorbed(
            |s| sent_mail(
                s,
                "e0",
                "me@acme.com",
                "xsarah@acme.com.evil",
                r#"["SENT"]"#
            ),
            SARAH
        ));
    }

    #[test]
    fn hold_needs_a_gmail_thread_id() {
        let (_d, s) = store();
        email(&s, "e1", "a@x.com", "b@y.com", "NoThread", "{}");
        s.conn
            .execute("UPDATE items SET ts_unix_ms = ?1", [now_ms()])
            .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(&s, "SELECT COUNT(*) FROM topic_source"),
            1,
            "no thread id: never held"
        );
        email(
            &s,
            "e2",
            "a@x.com",
            "b@y.com",
            "Rowless",
            r#"{"gmail_thread_id":"nothread","account_email":"b@y.com"}"#,
        );
        s.conn
            .execute("UPDATE items SET ts_unix_ms = ?1", [now_ms()])
            .unwrap();
        run_topic_index_job(&s, 200).unwrap();
        assert_eq!(
            n(&s, "SELECT COUNT(*) FROM topic_source"),
            1,
            "thread id without a row: held"
        );
    }

    #[test]
    fn run_jobs_batch_never_runs_topic_jobs() {
        let (_d, s) = store();
        for t in ["topic_index", "topic_cluster"] {
            s.conn.execute("INSERT INTO jobs (job_type, status, priority, attempts, not_before, created_at_unix_ms, updated_at_unix_ms) VALUES (?1,'pending',0,0,0,0,0)", [t]).unwrap();
        }
        s.run_jobs_batch(10, 1_000).unwrap();
        assert_eq!(n(&s, "SELECT COUNT(*) FROM jobs WHERE job_type LIKE 'topic_%' AND status = 'pending' AND attempts = 0"), 2);
    }
}
