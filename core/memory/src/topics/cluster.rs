//! `topic_cluster`: groups indexed sources into topics. Zero LLM calls.
//!
//! Sources are taken oldest first (`ts_unix_ms, id`) from those with no `assigned_at`
//! (new, or re-indexed after an edit; a trigger clears it):
//! 1. **Score** each against the live topics it links to:
//!    `W_VEC * cos(source, centroid) + W_ENT * weighted_jaccard(entities)`, or the Jaccard
//!    alone if either side has no vector. Entity rules: one mentioned by a single source links
//!    nothing; an **org** on more than `COMMON_SHARE` of sources (once there are
//!    `COMMON_MIN_SOURCES`) is ignored (the user's own company domain); people never are.
//!    Entity evidence needs a shared **person or phrase**: when the only overlap is an org (a
//!    customer's domain), the entity term is 0 and only the content term `W_VEC * cos` counts. A topic's entity profile is each entity's mean mention weight over its
//!    non-removed members, so both sides of the Jaccard are in [0, 1].
//!    A source with no people or orgs that links to no topic (a screen episode) may join its
//!    single closest topic on content alone (`W_VEC * cos >= JOIN_THRESHOLD`).
//! 2. **Join** every topic scoring `>= JOIN_THRESHOLD` (overlap is allowed). `INSERT OR IGNORE`
//!    leaves `user` and `removed` rows exactly as they are, so corrections survive.
//! 3. **Seed**: a source in no topic pairs with its best-scoring orphan within
//!    `SEED_WINDOW_DAYS` either side, if that score is `>= SEED_THRESHOLD`; nearby orphans of
//!    either seed that score `>= JOIN_THRESHOLD` against the new topic join it at once.
//! 4. **Ideas**: once nothing is left unassigned, a resumable pass (`IDEA_BATCH` phrases per
//!    call) turns a phrase mentioned by `IDEA_MIN_SOURCES`..=`IDEA_MAX_SOURCES` sources
//!    spanning `IDEA_MIN_SPAN_DAYS`+, that no topic already covers `IDEA_COVERED_SHARE` of,
//!    into a topic of exactly those sources; later mentions join it.
//!
//! Centroids are cached in `topic_stats` as a running vector sum per topic, updated on every
//! join, so a batch reads one row per topic instead of every member's vector.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rusqlite::{params, Connection, OptionalExtension};

use super::schema::{prune_orphans, PRUNE_DIRTY_KEY};
use crate::{Store, EMBEDDING_DIM};

pub(crate) const JOIN_THRESHOLD: f64 = 0.38;
pub(crate) const SEED_THRESHOLD: f64 = 0.55;
pub(crate) const W_VEC: f64 = 0.5;
pub(crate) const W_ENT: f64 = 0.5;
pub(crate) const IDEA_MIN_SPAN_DAYS: i64 = 14;
pub(crate) const IDEA_MIN_SOURCES: usize = 4;
pub(crate) const IDEA_COVERED_SHARE: f64 = 0.8;
pub(crate) const SEED_WINDOW_DAYS: i64 = 30;
/// Orgs mentioned by more than this share of all sources are ignored (the user's own company
/// domain), once there are `COMMON_MIN_SOURCES` sources. People are never ignored.
pub(crate) const COMMON_SHARE: f64 = 0.25;
pub(crate) const COMMON_MIN_SOURCES: f64 = 200.0;
/// Orphan peers looked at per entity when seeding (closest in time first).
pub(crate) const SEED_PEERS_PER_ENTITY: i64 = 50;
/// Phrases the idea pass handles per call.
pub(crate) const IDEA_BATCH: usize = 200;
/// A phrase on more sources than this is too broad to be an idea topic.
pub(crate) const IDEA_MAX_SOURCES: usize = 200;
/// Membership growth (vs. the count at last naming) that marks a topic for renaming.
pub(crate) const RENAME_GROWTH: f64 = 1.25;

const DAY_MS: i64 = 86_400_000;
const IDEA_HW_KEY: &str = "topic_cluster_idea_hw";
const IDEA_TARGET_KEY: &str = "topic_cluster_idea_target";
const IDEA_CURSOR_KEY: &str = "topic_cluster_idea_cursor";

fn kv_get(conn: &Connection, key: &str) -> rusqlite::Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT value FROM kv_meta WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .and_then(|v| v.parse().ok()))
}

fn kv_set(conn: &Connection, key: &str, v: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, v.to_string()],
    )?;
    Ok(())
}

/// Entity id -> weight.
type Ents = HashMap<i64, f64>;

struct TopicState {
    /// Running sum of member vectors; `None` when no member has one.
    sum: Option<Vec<f32>>,
    n_vec: i64,
    /// Lazily loaded: (non-removed member count, summed raw mention weights).
    profile: Option<(i64, Ents)>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[allow(clippy::chunks_exact_to_as_chunks)]
fn unblob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4) // ponytail: as_chunks needs a newer toolchain than CI
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn cos(a: &[f32], b: &[f32]) -> f64 {
    let (mut d, mut na, mut nb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x as f64, *y as f64);
        d += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        d / (na.sqrt() * nb.sqrt())
    }
}

/// Σ min / Σ max over the union of keys.
fn weighted_jaccard(a: &Ents, b: &Ents) -> f64 {
    let (mut lo, mut hi) = (0.0, 0.0);
    for (e, x) in a {
        let y = b.get(e).copied().unwrap_or(0.0);
        lo += x.min(y);
        hi += x.max(y);
    }
    hi += b
        .iter()
        .filter(|(e, _)| !a.contains_key(e))
        .map(|(_, y)| y)
        .sum::<f64>();
    if hi == 0.0 {
        0.0
    } else {
        lo / hi
    }
}

fn source_vec(conn: &Connection, sid: i64) -> rusqlite::Result<Option<Vec<f32>>> {
    Ok(conn
        .query_row(
            "SELECT embedding FROM topic_source_vec WHERE rowid = ?1",
            [sid],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .optional()?
        .map(|b| unblob(&b))
        .filter(|v| v.len() == EMBEDDING_DIM))
}

fn raw_mentions(conn: &Connection, sid: i64) -> rusqlite::Result<Ents> {
    let mut stmt =
        conn.prepare_cached("SELECT entity_id, weight FROM entity_mention WHERE source_id = ?1")?;
    let rows = stmt.query_map([sid], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// Recomputes a topic's cached vector sum from its non-removed members. Membership changes
/// made outside this job mark the cache stale via triggers (schema.rs); the next batch
/// calls this for every stale topic.
pub(crate) fn refresh_topic_stats(conn: &Connection, topic_id: i64) -> rusqlite::Result<()> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare_cached(
            "SELECT source_id FROM topic_member WHERE topic_id = ?1 AND origin != 'removed'",
        )?;
        let ids = stmt
            .query_map([topic_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids
    };
    let mut sum: Option<Vec<f32>> = None;
    let mut n = 0i64;
    for id in ids {
        if let Some(v) = source_vec(conn, id)? {
            let s = sum.get_or_insert_with(|| vec![0f32; EMBEDDING_DIM]);
            s.iter_mut().zip(&v).for_each(|(a, b)| *a += b);
            n += 1;
        }
    }
    write_stats(conn, topic_id, n, sum.as_deref())
}

fn write_stats(conn: &Connection, tid: i64, n: i64, sum: Option<&[f32]>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO topic_stats (topic_id, vec_n, vec_sum) VALUES (?1, ?2, ?3) \
         ON CONFLICT(topic_id) DO UPDATE SET vec_n = excluded.vec_n, vec_sum = excluded.vec_sum",
        params![tid, n, sum.map(blob)],
    )?;
    Ok(())
}

struct Cx<'a> {
    conn: &'a Connection,
    n_sources: f64,
    /// Entity id -> (mention count, kind: 0 phrase, 1 person, 2 org).
    df: HashMap<i64, (i64, u8)>,
    topics: BTreeMap<i64, TopicState>,
    now: i64,
}

impl Cx<'_> {
    fn load(conn: &Connection) -> rusqlite::Result<Cx<'_>> {
        let missing: Vec<i64> = {
            let mut stmt = conn.prepare(
                "SELECT id FROM topic WHERE id NOT IN (SELECT topic_id FROM topic_stats WHERE vec_n >= 0)",
            )?;
            let ids = stmt
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            ids
        };
        for t in missing {
            refresh_topic_stats(conn, t)?;
        }
        let mut topics = BTreeMap::new();
        // ponytail: every live topic's centroid is held in memory per batch (~1.5 KB each);
        // move to a vec0 KNN table if topic counts reach the hundreds of thousands.
        let mut stmt = conn.prepare(
            "SELECT t.id, s.vec_n, s.vec_sum FROM topic t JOIN topic_stats s ON s.topic_id = t.id \
             WHERE t.merged_into IS NULL",
        )?;
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<Vec<u8>>>(2)?,
            ))
        })? {
            let (id, n_vec, sum) = row?;
            let sum = sum.map(|b| unblob(&b)).filter(|v| v.len() == EMBEDDING_DIM);
            topics.insert(
                id,
                TopicState {
                    sum,
                    n_vec,
                    profile: None,
                },
            );
        }
        let n_sources: i64 =
            conn.query_row("SELECT COUNT(*) FROM topic_source", [], |r| r.get(0))?;
        Ok(Cx {
            conn,
            n_sources: n_sources as f64,
            df: HashMap::new(),
            topics,
            now: now_ms(),
        })
    }

    fn usable(&mut self, e: i64) -> rusqlite::Result<bool> {
        let (df, org) = match self.df.get(&e) {
            Some(d) => *d,
            None => {
                let d = self.conn.query_row(
                    "SELECT (SELECT COUNT(*) FROM entity_mention WHERE entity_id = ?1), \
                            COALESCE((SELECT CASE kind WHEN 'org' THEN 2 WHEN 'person' THEN 1 ELSE 0 END \
                                      FROM entity WHERE id = ?1), 0)",
                    [e],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                self.df.insert(e, d);
                d
            }
        };
        let common = org == 2
            && self.n_sources >= COMMON_MIN_SOURCES
            && df as f64 > COMMON_SHARE * self.n_sources;
        Ok(df >= 2 && !common)
    }

    /// Whether the raw mentions include a person or org (kinds cached by `filter`).
    fn has_party(&self, raw: &Ents) -> bool {
        raw.keys().any(|e| self.df.get(e).is_some_and(|d| d.1 >= 1))
    }

    /// The combined score; see the module docs. An overlap made only of orgs (a customer's
    /// domain, the user's employer) is no entity evidence: the entity term is 0 but the
    /// content term stays, so same-company colleagues writing about the same thing still
    /// link on wording (`W_VEC * cos`). With no overlap at all, only a source with no people
    /// or orgs of its own (a screen episode, a slide) may score on content.
    fn score(
        &self,
        party: bool,
        sv: Option<&[f32]>,
        se: &Ents,
        tv: Option<&[f32]>,
        te: &Ents,
    ) -> f64 {
        let shared = |org: bool| {
            se.keys()
                .any(|e| te.contains_key(e) && self.df.get(e).is_some_and(|d| d.1 == 2) == org)
        };
        let vec = match (sv, tv) {
            (Some(a), Some(b)) => Some(cos(a, b)),
            _ => None,
        };
        match (shared(false), vec) {
            (true, Some(c)) => W_VEC * c + W_ENT * weighted_jaccard(se, te),
            (true, None) => weighted_jaccard(se, te),
            (false, Some(c)) if !party || shared(true) => W_VEC * c,
            (false, _) => 0.0,
        }
    }

    fn filter(&mut self, raw: &Ents) -> rusqlite::Result<Ents> {
        let mut out = Ents::new();
        for (e, w) in raw {
            if self.usable(*e)? {
                out.insert(*e, *w);
            }
        }
        Ok(out)
    }

    /// The topic's usable entities with their mean weight per member.
    fn profile(&mut self, tid: i64) -> rusqlite::Result<Ents> {
        if self.topics[&tid].profile.is_none() {
            let members: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM topic_member WHERE topic_id = ?1 AND origin != 'removed'",
                [tid],
                |r| r.get(0),
            )?;
            let mut stmt = self.conn.prepare_cached(
                "SELECT em.entity_id, SUM(em.weight) FROM topic_member tm \
                 JOIN entity_mention em ON em.source_id = tm.source_id \
                 WHERE tm.topic_id = ?1 AND tm.origin != 'removed' GROUP BY em.entity_id",
            )?;
            let ents: Ents = stmt
                .query_map([tid], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            self.topics.get_mut(&tid).unwrap().profile = Some((members, ents));
        }
        let (members, sums) = self.topics[&tid].profile.clone().unwrap();
        if members == 0 {
            return Ok(Ents::new());
        }
        let mut out = self.filter(&sums)?;
        out.values_mut().for_each(|w| *w /= members as f64);
        Ok(out)
    }

    /// Live topics with a non-removed member that shares a usable entity with the source.
    /// ponytail: reads every mention of each entity (bounded by COMMON_SHARE of all sources);
    /// keep an entity -> topic index if this shows up in profiles.
    fn entity_topics(&self, ents: &Ents) -> rusqlite::Result<BTreeSet<i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT tm.topic_id FROM entity_mention em \
             JOIN topic_member tm ON tm.source_id = em.source_id \
             WHERE em.entity_id = ?1 AND tm.origin != 'removed'",
        )?;
        let mut out = BTreeSet::new();
        for e in ents.keys() {
            for t in stmt.query_map([e], |r| r.get::<_, i64>(0))? {
                let t = t?;
                if self.topics.contains_key(&t) {
                    out.insert(t);
                }
            }
        }
        Ok(out)
    }

    /// Adds an `auto` member unless any row (user, removed, auto) already exists.
    fn add_member(
        &mut self,
        tid: i64,
        sid: i64,
        ts: i64,
        sc: f64,
        sv: Option<&[f32]>,
        raw: &Ents,
    ) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO topic_member (topic_id, source_id, origin, score) VALUES (?1, ?2, 'auto', ?3)",
            params![tid, sid, sc],
        )?;
        if n == 0 {
            return Ok(false);
        }
        let st = self.topics.get_mut(&tid).unwrap();
        if let Some(v) = sv {
            let s = st.sum.get_or_insert_with(|| vec![0f32; EMBEDDING_DIM]);
            s.iter_mut().zip(v).for_each(|(a, b)| *a += b);
            st.n_vec += 1;
            write_stats(self.conn, tid, st.n_vec, st.sum.as_deref())?;
        }
        let members = match st.profile.as_mut() {
            Some((m, sums)) => {
                *m += 1;
                for (e, w) in raw {
                    *sums.entry(*e).or_insert(0.0) += w;
                }
                *m
            }
            None => self.conn.query_row(
                "SELECT COUNT(*) FROM topic_member WHERE topic_id = ?1 AND origin != 'removed'",
                [tid],
                |r| r.get(0),
            )?,
        };
        self.conn.execute(
            "UPDATE topic SET updated_at_unix_ms = ?2, \
               last_member_ts_unix_ms = MAX(COALESCE(last_member_ts_unix_ms, ?3), ?3), \
               name_dirty = CASE WHEN name_dirty = 1 OR ?4 >= ?5 * \
                 COALESCE((SELECT named_members FROM topic_stats WHERE topic_id = ?1), 0) \
                 THEN 1 ELSE 0 END \
             WHERE id = ?1",
            params![tid, self.now, ts, members as f64, RENAME_GROWTH],
        )?;
        Ok(true)
    }

    fn new_topic(&mut self) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO topic (name, name_dirty, created_at_unix_ms, updated_at_unix_ms) VALUES ('', 1, ?1, ?1)",
            [self.now],
        )?;
        let tid = self.conn.last_insert_rowid();
        write_stats(self.conn, tid, 0, None)?;
        self.topics.insert(
            tid,
            TopicState {
                sum: None,
                n_vec: 0,
                profile: Some((0, Ents::new())),
            },
        );
        Ok(tid)
    }

    fn is_orphan(&self, sid: i64) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM topic_member WHERE source_id = ?1 AND origin != 'removed' LIMIT 1",
                [sid],
                |_| Ok(()),
            )
            .optional()?
            .is_none())
    }

    fn cluster_one(&mut self, sid: i64, ts: i64) -> rusqlite::Result<()> {
        let raw = raw_mentions(self.conn, sid)?;
        let ents = self.filter(&raw)?;
        let sv = source_vec(self.conn, sid)?;
        let mut cands = self.entity_topics(&ents)?;
        let party = self.has_party(&raw);
        if let (Some(v), false, true) = (&sv, party, cands.is_empty()) {
            // A source with no people or orgs that links to no topic by entities (a screen
            // episode, say) can still join its single closest topic on content alone, if
            // W_VEC * cos >= JOIN_THRESHOLD (cos >= 0.76 today). Only one topic: content-only
            // members must not bridge topics.
            let best = self
                .topics
                .iter()
                .filter_map(|(t, st)| st.sum.as_deref().map(|c| (cos(v, c), *t)))
                .max_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, t)) = best.filter(|(c, _)| W_VEC * c >= JOIN_THRESHOLD) {
                cands.insert(t);
            }
        }
        for t in cands {
            let te = self.profile(t)?;
            let sc = self.score(
                party,
                sv.as_deref(),
                &ents,
                self.topics[&t].sum.as_deref(),
                &te,
            );
            if sc >= JOIN_THRESHOLD {
                self.add_member(t, sid, ts, sc, sv.as_deref(), &raw)?;
            }
        }
        if self.is_orphan(sid)? {
            self.seed(sid, ts, &raw, &ents, sv.as_deref())?;
        }
        self.conn.execute(
            "UPDATE topic_source SET assigned_at_unix_ms = ?2 WHERE id = ?1",
            params![sid, self.now],
        )?;
        Ok(())
    }

    /// Orphans within `SEED_WINDOW_DAYS` either side of `ts` that share a usable entity in
    /// `ents`, at most `SEED_PEERS_PER_ENTITY` per entity (closest in time first).
    fn orphans_near(&self, ents: &Ents, ts: i64) -> rusqlite::Result<BTreeMap<i64, i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.id, s.ts_unix_ms FROM entity_mention em JOIN topic_source s ON s.id = em.source_id \
             WHERE em.entity_id = ?1 AND s.ts_unix_ms BETWEEN ?2 AND ?3 \
               AND NOT EXISTS (SELECT 1 FROM topic_member tm WHERE tm.source_id = s.id AND tm.origin != 'removed') \
             ORDER BY abs(s.ts_unix_ms - ?4), s.id LIMIT ?5",
        )?;
        let w = SEED_WINDOW_DAYS * DAY_MS;
        let mut out = BTreeMap::new();
        for e in ents.keys() {
            for r in stmt.query_map(params![e, ts - w, ts + w, ts, SEED_PEERS_PER_ENTITY], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })? {
                let (p, pts) = r?;
                out.insert(p, pts);
            }
        }
        Ok(out)
    }

    /// Pairs an orphan source with its best orphan peer (sharing a usable entity, within
    /// `SEED_WINDOW_DAYS` either side). Peers without a shared entity cannot reach
    /// SEED_THRESHOLD (W_VEC * cos <= 0.5), so they are not looked at. The new topic then
    /// takes every other nearby orphan of either seed that scores `>= JOIN_THRESHOLD`, so a
    /// source processed before its topic existed is not stranded.
    fn seed(
        &mut self,
        sid: i64,
        ts: i64,
        raw: &Ents,
        ents: &Ents,
        sv: Option<&[f32]>,
    ) -> rusqlite::Result<()> {
        let mut peers = self.orphans_near(ents, ts)?;
        peers.remove(&sid);
        let mut best: Option<(f64, i64, i64)> = None;
        for (p, pts) in &peers {
            let pe = self.filter(&raw_mentions(self.conn, *p)?)?;
            let pv = source_vec(self.conn, *p)?;
            let sc = self.score(self.has_party(raw), sv, ents, pv.as_deref(), &pe);
            if sc >= SEED_THRESHOLD && best.is_none_or(|b| sc > b.0) {
                best = Some((sc, *p, *pts));
            }
        }
        let Some((sc, p, pts)) = best else {
            return Ok(());
        };
        let t = self.new_topic()?;
        let (praw, pv) = (raw_mentions(self.conn, p)?, source_vec(self.conn, p)?);
        self.add_member(t, p, pts, sc, pv.as_deref(), &praw)?;
        self.add_member(t, sid, ts, sc, sv, raw)?;
        let pe = self.filter(&praw)?;
        peers.extend(self.orphans_near(&pe, pts)?);
        for (o, ots) in peers {
            if o == p || o == sid {
                continue;
            }
            let oraw = raw_mentions(self.conn, o)?;
            let oe = self.filter(&oraw)?;
            let ov = source_vec(self.conn, o)?;
            let te = self.profile(t)?;
            let osc = self.score(
                self.has_party(&oraw),
                ov.as_deref(),
                &oe,
                self.topics[&t].sum.as_deref(),
                &te,
            );
            if osc >= JOIN_THRESHOLD {
                self.add_member(t, o, ots, osc, ov.as_deref(), &oraw)?;
            }
        }
        Ok(())
    }

    /// One step of the idea pass over phrases mentioned by sources in (hw, target], walked by
    /// entity id from a cursor, `limit` phrases per call. Returns true while the pass is
    /// unfinished. See the module docs.
    fn ideas(&mut self, limit: usize) -> rusqlite::Result<bool> {
        let hw = kv_get(self.conn, IDEA_HW_KEY)?.unwrap_or(0);
        let target = match kv_get(self.conn, IDEA_TARGET_KEY)? {
            Some(t) => t,
            None => {
                let t: i64 = self.conn.query_row(
                    "SELECT COALESCE(MAX(id), 0) FROM topic_source",
                    [],
                    |r| r.get(0),
                )?;
                if t <= hw {
                    return Ok(false);
                }
                kv_set(self.conn, IDEA_TARGET_KEY, t)?;
                t
            }
        };
        let cursor = kv_get(self.conn, IDEA_CURSOR_KEY)?.unwrap_or(0);
        // ponytail: walks phrase entities by id with a PK probe each; an incremental pass over
        // a few new sources still visits every phrase. Drive it from the new mentions if slow.
        let phrases: Vec<i64> = {
            let mut stmt = self.conn.prepare(
                "SELECT e.id FROM entity e WHERE e.kind = 'phrase' AND e.id > ?1 \
                   AND EXISTS (SELECT 1 FROM entity_mention em WHERE em.entity_id = e.id \
                                 AND em.source_id > ?2 AND em.source_id <= ?3) \
                 ORDER BY e.id LIMIT ?4",
            )?;
            let ids = stmt
                .query_map(params![cursor, hw, target, limit as i64], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            ids
        };
        for &e in &phrases {
            self.idea(e)?;
        }
        if phrases.len() < limit {
            kv_set(self.conn, IDEA_HW_KEY, target)?;
            self.conn.execute(
                "DELETE FROM kv_meta WHERE key IN (?1, ?2)",
                [IDEA_TARGET_KEY, IDEA_CURSOR_KEY],
            )?;
            return Ok(false);
        }
        kv_set(self.conn, IDEA_CURSOR_KEY, *phrases.last().unwrap())?;
        Ok(true)
    }

    fn idea(&mut self, e: i64) -> rusqlite::Result<()> {
        if !self.usable(e)? {
            return Ok(());
        }
        let srcs: Vec<(i64, i64)> = {
            let mut stmt = self.conn.prepare_cached(
                    "SELECT s.id, s.ts_unix_ms FROM entity_mention em JOIN topic_source s ON s.id = em.source_id \
                     WHERE em.entity_id = ?1 ORDER BY s.ts_unix_ms, s.id",
                )?;
            let rows = stmt
                .query_map([e], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        let span = srcs.last().map_or(0, |l| l.1) - srcs.first().map_or(0, |f| f.1);
        if srcs.len() < IDEA_MIN_SOURCES
            || srcs.len() > IDEA_MAX_SOURCES
            || span < IDEA_MIN_SPAN_DAYS * DAY_MS
        {
            return Ok(());
        }
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT topic_id FROM topic_idea WHERE entity_id = ?1",
                [e],
                |r| r.get(0),
            )
            .optional()?
            .filter(|t| self.topics.contains_key(t));
        let tid = match existing {
            Some(t) => t,
            None => {
                let ids: BTreeSet<i64> = srcs.iter().map(|s| s.0).collect();
                let mut covered = 0usize;
                for t in self.entity_topics(&Ents::from([(e, 1.0)]))? {
                    let mut stmt = self.conn.prepare_cached(
                            "SELECT source_id FROM topic_member WHERE topic_id = ?1 AND origin != 'removed'",
                        )?;
                    let n = stmt
                        .query_map([t], |r| r.get::<_, i64>(0))?
                        .filter(|m| m.as_ref().is_ok_and(|m| ids.contains(m)))
                        .count();
                    covered = covered.max(n);
                }
                if covered as f64 >= IDEA_COVERED_SHARE * ids.len() as f64 {
                    return Ok(());
                }
                let t = self.new_topic()?;
                self.conn.execute(
                    "INSERT OR REPLACE INTO topic_idea (entity_id, topic_id) VALUES (?1, ?2)",
                    [e, t],
                )?;
                t
            }
        };
        for (sid, ts) in srcs {
            let raw = raw_mentions(self.conn, sid)?;
            let sv = source_vec(self.conn, sid)?;
            self.add_member(
                tid,
                sid,
                ts,
                raw.get(&e).copied().unwrap_or(0.0),
                sv.as_deref(),
                &raw,
            )?;
        }
        Ok(())
    }
}

/// Clusters up to `batch` unassigned sources, oldest first, then (once nothing is left
/// unassigned) runs one step of the idea pass. Returns true while work remains. Results do
/// not depend on batch size.
pub fn run_topic_cluster_job(store: &Store, batch: usize) -> rusqlite::Result<bool> {
    run_cluster(store, batch, IDEA_BATCH)
}

pub(crate) fn run_cluster(
    store: &Store,
    batch: usize,
    idea_batch: usize,
) -> rusqlite::Result<bool> {
    let tx = store.conn.unchecked_transaction()?;
    let todo: Vec<(i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT id, ts_unix_ms FROM topic_source WHERE assigned_at_unix_ms IS NULL \
             ORDER BY ts_unix_ms, id LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([batch as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    let mut more = false;
    // An idle tick (nothing unassigned, idea pass caught up) loads no centroids.
    if !todo.is_empty() || ideas_pending(&tx)? {
        let mut cx = Cx::load(&tx)?;
        for (sid, ts) in todo {
            cx.cluster_one(sid, ts)?;
        }
        more = tx
            .query_row(
                "SELECT 1 FROM topic_source WHERE assigned_at_unix_ms IS NULL LIMIT 1",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !more {
            more = cx.ideas(idea_batch)?;
        }
    }
    if kv_get(&tx, PRUNE_DIRTY_KEY)?.is_some() {
        prune_orphans(&tx)?;
        tx.execute("DELETE FROM kv_meta WHERE key = ?1", [PRUNE_DIRTY_KEY])?;
    }
    tx.commit()?;
    Ok(more)
}

/// The idea pass has a step to take: one in progress, or sources newer than its last pass.
fn ideas_pending(conn: &Connection) -> rusqlite::Result<bool> {
    if kv_get(conn, IDEA_TARGET_KEY)?.is_some() {
        return Ok(true);
    }
    let max: i64 = conn.query_row("SELECT COALESCE(MAX(id), 0) FROM topic_source", [], |r| {
        r.get(0)
    })?;
    Ok(max > kv_get(conn, IDEA_HW_KEY)?.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::topics::fixture::{build, build_with, Fixture, ACME, BOARD, IDEA, LABELS, NOISE};
    use crate::topics::index::run_topic_index_job;

    fn run(s: &Store, batch: usize) {
        while run_topic_cluster_job(s, batch).unwrap() {}
    }

    /// Live topic id -> its non-removed members.
    fn topics(s: &Store) -> BTreeMap<i64, BTreeSet<i64>> {
        let mut out: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
        let mut stmt = s
            .conn
            .prepare(
                "SELECT tm.topic_id, tm.source_id FROM topic_member tm JOIN topic t ON t.id = tm.topic_id \
                 WHERE tm.origin != 'removed' AND t.merged_into IS NULL",
            )
            .unwrap();
        for r in stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .unwrap()
        {
            let (t, m) = r.unwrap();
            out.entry(t).or_default().insert(m);
        }
        out
    }

    /// Best produced topic for a label by F1: (topic id, precision, recall).
    fn best(f: &Fixture, label: &str) -> (i64, f64, f64) {
        let want = &f.labels[label];
        topics(&f.store)
            .into_iter()
            .map(|(t, ms)| {
                let hit = ms.intersection(want).count() as f64;
                (t, hit / ms.len() as f64, hit / want.len() as f64)
            })
            .max_by(|a, b| {
                let f1 = |x: &(i64, f64, f64)| 2.0 * x.1 * x.2 / (x.1 + x.2).max(1e-9);
                f1(a).total_cmp(&f1(b))
            })
            .unwrap_or((0, 0.0, 0.0))
    }

    fn table(f: &Fixture) -> String {
        let mut s = String::from("label                | topic | size | P    | R\n");
        for l in LABELS {
            let (t, p, r) = best(f, l);
            let size = topics(&f.store).get(&t).map_or(0, |m| m.len());
            s += &format!("{l:<20} | {t:>5} | {size:>4} | {p:.2} | {r:.2}\n");
        }
        let all = topics(&f.store);
        s += &format!(
            "topics: {}  sizes: {:?}\n",
            all.len(),
            all.values().map(|m| m.len()).collect::<Vec<_>>()
        );
        s
    }

    #[test]
    fn idle_tick_loads_nothing_and_prunes_only_when_flagged() {
        let f = build(true);
        run(&f.store, 200);
        let c = &f.store.conn;
        let t: i64 = c
            .query_row("SELECT topic_id FROM topic_stats LIMIT 1", [], |r| r.get(0))
            .unwrap();
        c.execute_batch(&format!(
            "DELETE FROM kv_meta WHERE key = 'topic_prune_dirty';
             UPDATE topic_stats SET vec_n = -1, vec_sum = NULL WHERE topic_id = {t};
             INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('phrase', 'orphan', 'orphan', 0);"
        ))
        .unwrap();
        let vec_n = || -> i64 {
            c.query_row("SELECT vec_n FROM topic_stats WHERE topic_id = ?1", [t], |r| r.get(0))
                .unwrap()
        };
        let orphan = || -> i64 {
            c.query_row("SELECT COUNT(*) FROM entity WHERE key = 'orphan'", [], |r| r.get(0))
                .unwrap()
        };
        assert!(!run_topic_cluster_job(&f.store, 200).unwrap());
        assert_eq!((vec_n(), orphan()), (-1, 1), "no centroid load, no prune");
        c.execute("INSERT INTO kv_meta (key, value) VALUES ('topic_prune_dirty', '1')", [])
            .unwrap();
        assert!(!run_topic_cluster_job(&f.store, 200).unwrap());
        assert_eq!((vec_n(), orphan()), (-1, 0), "flag set: pruned");
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM kv_meta WHERE key = 'topic_prune_dirty'", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0,
            "flag cleared"
        );
    }

    #[test]
    fn fixture_topics_are_recovered() {
        let f = build(true);
        run(&f.store, 200);
        let t = table(&f);
        eprintln!("{t}");
        for l in LABELS {
            let (_, p, r) = best(&f, l);
            assert!(p >= 0.8 && r >= 0.7, "{l}: P={p:.2} R={r:.2}\n{t}");
        }
    }

    #[test]
    fn weekly_meeting_lands_in_both_topics() {
        let f = build(true);
        run(&f.store, 200);
        let all = topics(&f.store);
        for l in [ACME, BOARD] {
            let (t, _, _) = best(&f, l);
            assert!(
                all.get(&t).is_some_and(|m| m.contains(&f.weekly)),
                "weekly not in {l}\n{}",
                table(&f)
            );
        }
    }

    #[test]
    fn noise_does_not_seed_topics() {
        let f = build(true);
        run(&f.store, 200);
        let members: BTreeSet<i64> = topics(&f.store).into_values().flatten().collect();
        assert!(!members.is_empty(), "nothing clustered");
        let n = f.labels[NOISE].intersection(&members).count();
        assert!(n <= 2, "{n} noise items are topic members\n{}", table(&f));
    }

    #[test]
    fn idea_topic_emerges_across_unrelated_sources() {
        let f = build(true);
        run(&f.store, 200);
        let (_, p, r) = best(&f, IDEA);
        assert!(p >= 0.8 && r >= 0.7, "P={p:.2} R={r:.2}\n{}", table(&f));
    }

    fn reset_assigned(s: &Store) {
        s.conn
            .execute("UPDATE topic_source SET assigned_at_unix_ms = NULL", [])
            .unwrap();
    }

    fn origin(s: &Store, t: i64, m: i64) -> Option<String> {
        s.conn
            .query_row(
                "SELECT origin FROM topic_member WHERE topic_id = ?1 AND source_id = ?2",
                [t, m],
                |r| r.get(0),
            )
            .ok()
    }

    #[test]
    fn removed_member_never_returns() {
        let f = build(true);
        run(&f.store, 200);
        let (t, _, _) = best(&f, ACME);
        let m = *topics(&f.store)[&t]
            .iter()
            .find(|m| **m != f.weekly)
            .unwrap();
        f.store
            .conn
            .execute(
                "UPDATE topic_member SET origin = 'removed' WHERE topic_id = ?1 AND source_id = ?2",
                [t, m],
            )
            .unwrap();
        reset_assigned(&f.store);
        run(&f.store, 200);
        assert_eq!(origin(&f.store, t, m).as_deref(), Some("removed"));
    }

    #[test]
    fn user_member_survives_recluster() {
        let f = build(true);
        run(&f.store, 200);
        let (t, _, _) = best(&f, ACME);
        assert_ne!(t, 0, "no Acme topic");
        let n = *f.labels[NOISE].iter().next().unwrap();
        f.store
            .conn
            .execute(
                "INSERT OR REPLACE INTO topic_member (topic_id, source_id, origin, score) VALUES (?1, ?2, 'user', 1.0)",
                [t, n],
            )
            .unwrap();
        reset_assigned(&f.store);
        run(&f.store, 200);
        assert_eq!(origin(&f.store, t, n).as_deref(), Some("user"));
    }

    #[test]
    fn entity_only_clustering_without_vectors() {
        let f = build(false);
        let nvec: i64 = f
            .store
            .conn
            .query_row("SELECT COUNT(*) FROM topic_source_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(nvec, 0);
        run(&f.store, 200);
        let (_, p, r) = best(&f, ACME);
        eprintln!("entity-only:\n{}", table(&f));
        assert!(r >= 0.5, "P={p:.2} R={r:.2}\n{}", table(&f));
    }

    #[test]
    fn incremental_batches_match_one_big_batch() {
        let sets = |batch| {
            let f = build(true);
            run(&f.store, batch);
            topics(&f.store).into_values().collect::<BTreeSet<_>>()
        };
        let small = sets(5);
        assert!(!small.is_empty());
        assert_eq!(small, sets(1000));
    }

    /// Sources 31 days apart can never pair (SEED_WINDOW_DAYS), so only the idea pass can
    /// group them; a later mention joins the same idea topic instead of seeding another.
    #[test]
    fn idea_pass_seeds_and_absorbs_later_mentions() {
        let s = Store::open_in_memory().unwrap();
        s.conn
            .execute(
                "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('phrase', 'pricing psychology', 'pricing psychology', 0)",
                [],
            )
            .unwrap();
        let e = s.conn.last_insert_rowid();
        let add = |day: i64| {
            s.conn
                .execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('episode', ?1, ?2, 't', 0)",
                    [day, day * 86_400_000],
                )
                .unwrap();
            let sid = s.conn.last_insert_rowid();
            s.conn
                .execute(
                    "INSERT INTO entity_mention (entity_id, source_id, weight) VALUES (?1, ?2, 0.4)",
                    [e, sid],
                )
                .unwrap();
            sid
        };
        let first: BTreeSet<i64> = [0, 31, 62, 93].into_iter().map(add).collect();
        run(&s, 200);
        let all = topics(&s);
        assert_eq!(all.values().collect::<Vec<_>>(), vec![&first]);
        let late = add(124);
        run(&s, 200);
        let all = topics(&s);
        assert_eq!(all.len(), 1);
        assert!(all.values().next().unwrap().contains(&late));
    }

    fn stats(s: &Store, t: i64) -> (i64, Option<Vec<u8>>) {
        s.conn
            .query_row(
                "SELECT vec_n, vec_sum FROM topic_stats WHERE topic_id = ?1",
                [t],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    /// A manually deleted item, or a member marked removed, stops counting in the centroid.
    #[test]
    fn deleted_or_removed_member_leaves_centroid() {
        let f = build(true);
        run(&f.store, 200);
        let (t, _, _) = best(&f, ACME);
        let (n0, _) = stats(&f.store, t);
        let ext: String = f
            .store
            .conn
            .query_row(
                "SELECT i.external_id FROM topic_member tm JOIN topic_source s ON s.id = tm.source_id \
                 JOIN items i ON i.id = s.ref_id WHERE tm.topic_id = ?1 AND s.kind = 'email' LIMIT 1",
                [t],
                |r| r.get(0),
            )
            .unwrap();
        f.store.delete_email_items(&[ext]).unwrap();
        assert_eq!(stats(&f.store, t).0, -1, "stale after delete");
        // Rebuilt on the next centroid load (an idle tick loads nothing).
        Cx::load(&f.store.conn).unwrap();
        let (n1, sum1) = stats(&f.store, t);
        assert_eq!(n1, n0 - 1);
        refresh_topic_stats(&f.store.conn, t).unwrap();
        assert_eq!(
            stats(&f.store, t),
            (n1, sum1),
            "rebuilt from remaining members"
        );

        let m = *topics(&f.store)[&t]
            .iter()
            .find(|m| **m != f.weekly)
            .unwrap();
        f.store
            .conn
            .execute(
                "UPDATE topic_member SET origin = 'removed' WHERE topic_id = ?1 AND source_id = ?2",
                [t, m],
            )
            .unwrap();
        assert_eq!(stats(&f.store, t).0, -1, "stale after remove");
        Cx::load(&f.store.conn).unwrap();
        assert_eq!(stats(&f.store, t).0, n1 - 1);
    }

    /// Index in small per-kind batches and cluster after each one, so sources arrive out of
    /// time order (all emails first, then calendar, ...). The result must match the normal run.
    #[test]
    fn adversarial_order_matches_normal_run() {
        let normal = build(true);
        run(&normal.store, 200);
        let adv = build_with(true, |s| {
            while run_topic_index_job(s, 3).unwrap() {
                run(s, 200);
            }
            run(s, 200);
        });
        run(&adv.store, 200);
        assert_eq!(
            keyed(&adv),
            keyed(&normal),
            "normal:\n{}adversarial:\n{}",
            table(&normal),
            table(&adv)
        );
    }

    /// Topic memberships as (kind, ref_id) sets, stable across index orders.
    fn keyed(f: &Fixture) -> BTreeSet<BTreeSet<(String, i64)>> {
        topics(&f.store)
            .into_values()
            .map(|ms| {
                ms.into_iter()
                    .map(|m| {
                        f.store
                            .conn
                            .query_row(
                                "SELECT kind, ref_id FROM topic_source WHERE id = ?1",
                                [m],
                                |r| Ok((r.get(0)?, r.get(1)?)),
                            )
                            .unwrap()
                    })
                    .collect()
            })
            .collect()
    }

    /// The idea pass walks phrases from a cursor, a few per call, returning true until done.
    #[test]
    fn idea_pass_resumes_in_small_steps() {
        let one = build(false);
        run(&one.store, 200);
        let small = build(false);
        let mut calls = 0;
        while run_cluster(&small.store, 200, 1).unwrap() {
            calls += 1;
        }
        let ideas = |f: &Fixture| -> i64 {
            f.store
                .conn
                .query_row("SELECT COUNT(*) FROM topic_idea", [], |r| r.get(0))
                .unwrap()
        };
        assert!(ideas(&one) >= 1, "fixture should produce idea topics");
        assert!(calls > 1, "idea pass did not resume across calls");
        let sets = |f: &Fixture| topics(&f.store).into_values().collect::<BTreeSet<_>>();
        assert_eq!(sets(&small), sets(&one));
    }

    fn sources_with_mentions(s: &Store, n: i64, person_in: i64, org_in: i64) -> (i64, i64) {
        let ent = |kind: &str, key: &str| -> i64 {
            s.conn
                .execute(
                    "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES (?1, ?2, ?2, 0)",
                    [kind, key],
                )
                .unwrap();
            s.conn.last_insert_rowid()
        };
        let (p, o) = (
            ent("person", "email:boss@mycorp.com"),
            ent("org", "mycorp.com"),
        );
        for i in 0..n {
            s.conn
                .execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('email', ?1, ?1, 't', 0)",
                    [i],
                )
                .unwrap();
            let sid = s.conn.last_insert_rowid();
            for (e, upto) in [(p, person_in), (o, org_in)] {
                if i < upto {
                    s.conn
                        .execute(
                            "INSERT INTO entity_mention (entity_id, source_id, weight) VALUES (?1, ?2, 1.0)",
                            [e, sid],
                        )
                        .unwrap();
                }
            }
        }
        (p, o)
    }

    /// Only orgs can be "common", and only in a corpus of COMMON_MIN_SOURCES or more.
    #[test]
    fn people_are_never_common() {
        let small = Store::open_in_memory().unwrap();
        let (p, o) = sources_with_mentions(&small, 40, 20, 20);
        let mut cx = Cx::load(&small.conn).unwrap();
        assert!(cx.usable(p).unwrap(), "person in 50% of a small corpus");
        assert!(cx.usable(o).unwrap(), "org in a small corpus");
        let big = Store::open_in_memory().unwrap();
        let (p, o) = sources_with_mentions(&big, 220, 110, 110);
        let mut cx = Cx::load(&big.conn).unwrap();
        assert!(cx.usable(p).unwrap(), "person in 50% of a big corpus");
        assert!(!cx.usable(o).unwrap(), "org in 50% of a big corpus");
    }

    /// An edited meeting is re-indexed: its topics' centroids go stale and it re-clusters.
    #[test]
    fn edited_meeting_reclusters() {
        let f = build(true);
        run(&f.store, 200);
        let c = &f.store.conn;
        let (sid, mid): (i64, i64) = c
            .query_row(
                "SELECT id, ref_id FROM topic_source WHERE kind = 'meeting' AND title = 'Acme Q3 Pricing review'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let t: i64 = c
            .query_row(
                "SELECT topic_id FROM topic_member WHERE source_id = ?1 LIMIT 1",
                [sid],
                |r| r.get(0),
            )
            .unwrap();
        c.execute(
            "UPDATE meeting SET summary_tldr = summary_tldr || ' Sarah also asked about multi-year renewal tiers.' WHERE id = ?1",
            [mid],
        )
        .unwrap();
        while run_topic_index_job(&f.store, 200).unwrap() {}
        assert_eq!(stats(&f.store, t).0, -1, "centroid not marked stale");
        let assigned = |sid: i64| -> Option<i64> {
            c.query_row(
                "SELECT assigned_at_unix_ms FROM topic_source WHERE id = ?1",
                [sid],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            assigned(sid),
            None,
            "edited source not queued for re-clustering"
        );
        run(&f.store, 200);
        assert!(assigned(sid).is_some());
        let rebuilt = stats(&f.store, t);
        assert!(rebuilt.0 >= 0);
        refresh_topic_stats(c, t).unwrap();
        assert_eq!(stats(&f.store, t), rebuilt);
    }

    /// Three Acme people who share no person or phrase with each other or the topic, writing
    /// about the same deal: linked only by acme.com and content, they still cluster (here,
    /// into the Acme topic). The office-move emails from the same org stay out.
    #[test]
    fn same_org_people_cluster_on_content() {
        let f = build(true);
        run(&f.store, 200);
        let c = &f.store.conn;
        let sid = |title: &str| -> i64 {
            c.query_row(
                "SELECT id FROM topic_source WHERE title = ?1",
                [title],
                |r| r.get(0),
            )
            .unwrap()
        };
        let (t, _, _) = best(&f, ACME);
        let all = topics(&f.store);
        for title in [
            "Contract redlines",
            "Invoice schedule for renewal",
            "Procurement checklist",
        ] {
            assert!(
                all[&t].contains(&sid(title)),
                "{title} not in Acme\n{}",
                table(&f)
            );
        }
        for title in ["Acme HQ office move", "Re: Acme HQ office move"] {
            assert!(!all[&t].contains(&sid(title)), "{title} leaked into Acme");
        }
    }
}
