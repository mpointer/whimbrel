//! Seed topics: user-named topics with aliases and a kind. A seed is a `topic` row
//! (`name_is_user = 1`) plus a `topic_seed` row; `prune_orphans` keeps it while it has no
//! members. Matching aliases, classification and sync build on this (later tasks).

use rusqlite::{params, Connection, OptionalExtension};

use crate::{now_unix_ms, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedKind {
    Customer,
    Product,
    Project,
    Subject,
}

impl SeedKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SeedKind::Customer => "customer",
            SeedKind::Product => "product",
            SeedKind::Project => "project",
            SeedKind::Subject => "subject",
        }
    }

    pub fn parse(s: &str) -> Option<SeedKind> {
        match s {
            "customer" => Some(SeedKind::Customer),
            "product" => Some(SeedKind::Product),
            "project" => Some(SeedKind::Project),
            "subject" => Some(SeedKind::Subject),
            _ => None,
        }
    }
}

/// A seed as it travels between devices (`topic_seed` sync rows). Last write wins per
/// record on `updated_at_unix_ms`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopicSeedPayload {
    pub key: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub kind: Option<String>,
    pub merged_into_key: Option<String>,
    pub deleted: bool,
    pub updated_at_unix_ms: i64,
}

const SYNC_BACKFILLED_KEY: &str = "topics_sync_backfilled";

#[derive(Debug, Clone, PartialEq)]
pub struct SeedTopic {
    pub topic_id: i64,
    pub key: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub kind: Option<SeedKind>,
    pub item_count: i64,
    pub updated_at_unix_ms: i64,
}

/// An auto topic big enough and varied enough to offer as a seed.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicSuggestion {
    pub topic_id: i64,
    pub name: String,
    pub item_count: i64,
    pub kinds: Vec<String>,
}

/// At least 5 non-removed members from at least 2 source kinds.
pub(super) const SUGGESTION_HAVING: &str = "COUNT(*) >= 5 AND COUNT(DISTINCT s.kind) >= 2";
const DISMISSED_PREFIX: &str = "topic_suggestion_dismissed:";

/// Queues sources to be re-matched against this seed: its former `seed` members (dropped
/// here) and every source whose stored text contains one of its current aliases.
/// For a topic added or edited on this device: items whose text names it are asked about
/// again even if Jev already judged them, since it wasn't a choice then. Not called for a
/// topic pulled from another device; the device where it was made asks, and its verdicts sync.
fn queue_rejudge(conn: &Connection, topic_id: i64) -> rusqlite::Result<()> {
    let aliases: Vec<String> = conn
        .query_row(
            "SELECT aliases FROM topic_seed WHERE topic_id = ?1",
            [topic_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    let seeds = [(topic_id, aliases.clone())];
    for a in &aliases {
        let rows: Vec<(i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT x.source_id, x.text FROM topic_source_text x JOIN topic_source s ON s.id = x.source_id \
                 WHERE s.noise = 0 AND instr(lower(x.text), lower(?1)) > 0",
            )?;
            let rows = stmt
                .query_map([a], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        for (sid, text) in rows {
            if !super::aliases::alias_hits(&text, &seeds).is_empty() {
                conn.execute(
                    "INSERT OR IGNORE INTO topic_rejudge (source_id) VALUES (?1)",
                    [sid],
                )?;
            }
        }
    }
    Ok(())
}

pub(crate) fn mark_seeds_rematch(conn: &Connection, topic_id: i64) -> rusqlite::Result<()> {
    super::aliases::bump_seed_version(conn)?;
    conn.execute(
        "UPDATE topic_source SET assigned_at_unix_ms = NULL WHERE id IN \
         (SELECT source_id FROM topic_member WHERE topic_id = ?1 AND origin = 'seed')",
        [topic_id],
    )?;
    conn.execute(
        "DELETE FROM topic_member WHERE topic_id = ?1 AND origin = 'seed'",
        [topic_id],
    )?;
    let live = conn
        .query_row(
            "SELECT aliases FROM topic_seed WHERE topic_id = ?1 AND deleted = 0 AND merged_into_key IS NULL",
            [topic_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    let aliases: Vec<String> = live
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    for a in aliases {
        // Over-selects (no word-boundary check); assign_seeds decides with `alias_hits`.
        conn.execute(
            "UPDATE topic_source SET assigned_at_unix_ms = NULL WHERE id IN \
             (SELECT source_id FROM topic_source_text WHERE instr(lower(text), lower(?1)) > 0)",
            [a],
        )?;
    }
    Ok(())
}

fn invalid(msg: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(msg.into().into())
}

/// Random v4-style UUID text, built from `rand` (the crate has no `uuid` dependency).
fn new_key() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// The name first, then `aliases`, trimmed and deduped case-insensitively.
pub(crate) fn normalize_aliases(name: &str, aliases: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in std::iter::once(name).chain(aliases.iter().map(String::as_str)) {
        let a = a.trim();
        if !a.is_empty() && !out.iter().any(|o| o.to_lowercase() == a.to_lowercase()) {
            out.push(a.to_string());
        }
    }
    out
}

fn bump(conn: &Connection, topic_id: i64) -> rusqlite::Result<i64> {
    let ms: i64 = conn.query_row(
        "UPDATE topic_seed SET updated_at_unix_ms = MAX(?2, updated_at_unix_ms + 1) \
         WHERE topic_id = ?1 RETURNING updated_at_unix_ms",
        params![topic_id, now_unix_ms()],
        |r| r.get(0),
    )?;
    Ok(ms)
}

fn load(
    conn: &Connection,
    where_sql: &str,
    p: impl rusqlite::Params,
) -> rusqlite::Result<Vec<SeedTopic>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT s.topic_id, s.key, t.name, s.aliases, s.kind, s.updated_at_unix_ms, \
           (SELECT COUNT(*) FROM topic_member m WHERE m.topic_id = s.topic_id AND m.origin != 'removed') \
         FROM topic_seed s JOIN topic t ON t.id = s.topic_id WHERE {where_sql} ORDER BY t.name COLLATE NOCASE, s.topic_id"
    ))?;
    let rows = stmt.query_map(p, |r| {
        let aliases: String = r.get(3)?;
        let kind: Option<String> = r.get(4)?;
        Ok(SeedTopic {
            topic_id: r.get(0)?,
            key: r.get(1)?,
            name: r.get(2)?,
            aliases: serde_json::from_str(&aliases).unwrap_or_default(),
            kind: kind.as_deref().and_then(SeedKind::parse),
            item_count: r.get(6)?,
            updated_at_unix_ms: r.get(5)?,
        })
    })?;
    rows.collect()
}

fn aliases_json(aliases: &[String]) -> rusqlite::Result<String> {
    serde_json::to_string(aliases).map_err(crate::json_to_sql_error)
}

/// Queues the seed record on `topic_id` (if any) for sync, in the caller's transaction. A
/// record whose stored aliases don't parse is skipped: sent as `[]` it would wipe every
/// peer's aliases.
pub(crate) fn enqueue_seed(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    topic_id: i64,
) -> rusqlite::Result<()> {
    let Some((mut p, aliases)) = seed_payload(tx, topic_id)? else {
        return Ok(());
    };
    match serde_json::from_str(&aliases) {
        Ok(a) => p.aliases = a,
        Err(e) => {
            eprintln!(
                "[topics] seed {} not synced, unreadable aliases: {e}",
                p.key
            );
            return Ok(());
        }
    }
    store.enqueue_topic_seed_outbox(tx, &p)
}

/// The seed record on `topic_id` as sent, with `aliases` empty, plus its stored aliases JSON.
fn seed_payload(
    conn: &Connection,
    topic_id: i64,
) -> rusqlite::Result<Option<(TopicSeedPayload, String)>> {
    conn.query_row(
            "SELECT s.key, t.name, s.aliases, s.kind, s.merged_into_key, s.deleted, s.updated_at_unix_ms \
             FROM topic_seed s JOIN topic t ON t.id = s.topic_id WHERE s.topic_id = ?1",
            [topic_id],
            |r| {
                let p = TopicSeedPayload {
                    key: r.get(0)?,
                    name: r.get(1)?,
                    aliases: Vec::new(),
                    kind: r.get(3)?,
                    merged_into_key: r.get(4)?,
                    deleted: r.get(5)?,
                    updated_at_unix_ms: r.get(6)?,
                };
                Ok((p, r.get::<_, String>(2)?))
            },
        )
        .optional()
}

/// Breaks merge cycles (opposite merges made on two devices, each pulled by the other):
/// the cycle's lowest key stays live, on the live topic its cycle's members ended up in, and
/// is bumped and queued so every device lands on the same record.
fn break_merge_cycles(store: &Store, tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<Vec<i64>> {
    let links: std::collections::HashMap<String, String> = {
        let mut stmt = tx.prepare(
            "SELECT key, merged_into_key FROM topic_seed WHERE merged_into_key IS NOT NULL",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    let mut lows: Vec<String> = Vec::new();
    for start in links.keys() {
        let mut cycle = vec![start.clone()];
        let mut at = &links[start];
        while at != start && cycle.len() <= links.len() {
            cycle.push(at.clone());
            let Some(next) = links.get(at) else { break };
            at = next;
        }
        if at == start {
            let low = cycle.into_iter().min().expect("non-empty");
            if !lows.contains(&low) {
                lows.push(low);
            }
        }
    }
    let mut live = Vec::new();
    for low in lows {
        let (id, live_id): (i64, i64) = tx.query_row(
            "SELECT s.topic_id, COALESCE(t.merged_into, t.id) FROM topic_seed s JOIN topic t ON t.id = s.topic_id \
             WHERE s.key = ?1",
            [&low],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if id != live_id {
            // Swap records (and their names) with whatever seed holds the live topic.
            let name = |t: i64| -> rusqlite::Result<String> {
                tx.query_row("SELECT name FROM topic WHERE id = ?1", [t], |r| r.get(0))
            };
            let (low_name, live_name) = (name(id)?, name(live_id)?);
            tx.execute(
                "UPDATE topic_seed SET topic_id = -1 WHERE topic_id = ?1",
                [live_id],
            )?;
            tx.execute(
                "UPDATE topic_seed SET topic_id = ?2 WHERE topic_id = ?1",
                [id, live_id],
            )?;
            tx.execute(
                "UPDATE topic_seed SET topic_id = ?1 WHERE topic_id = -1",
                [id],
            )?;
            tx.execute(
                "UPDATE topic SET name = ?2 WHERE id = ?1",
                params![live_id, low_name],
            )?;
            tx.execute(
                "UPDATE topic SET name = ?2 WHERE id = ?1",
                params![id, live_name],
            )?;
        }
        tx.execute(
            "UPDATE topic_seed SET merged_into_key = NULL WHERE topic_id = ?1",
            [live_id],
        )?;
        bump(tx, live_id)?;
        mark_seeds_rematch(tx, live_id)?;
        enqueue_seed(store, tx, live_id)?;
        live.push(live_id);
    }
    Ok(live)
}

/// Carries out pulled merges whose target seed is known here: each live topic whose seed
/// has `merged_into_key` set is merged into the target's live topic. Returns the topics
/// that absorbed one (to re-embed). Seed records are left as pulled: the origin sends the
/// target's merged aliases as its own row.
fn settle_merges(conn: &Connection) -> rusqlite::Result<Vec<i64>> {
    let mut kept = Vec::new();
    // Each pass merges at least one topic away, so this ends.
    loop {
        let pair: Option<(i64, i64)> = conn
            .query_row(
                "SELECT s.topic_id, COALESCE(tt.merged_into, tt.id) FROM topic_seed s \
                 JOIN topic t ON t.id = s.topic_id \
                 JOIN topic_seed ts ON ts.key = s.merged_into_key JOIN topic tt ON tt.id = ts.topic_id \
                 WHERE t.merged_into IS NULL AND COALESCE(tt.merged_into, tt.id) != s.topic_id LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((absorb, keep)) = pair else {
            return Ok(kept);
        };
        super::edit::merge_rows(conn, keep, absorb)?;
        mark_seeds_rematch(conn, keep)?;
        kept.push(keep);
    }
}

impl Store {
    pub fn create_seed_topic(
        &self,
        name: &str,
        aliases: &[String],
        kind: Option<SeedKind>,
    ) -> rusqlite::Result<SeedTopic> {
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid("seed topic needs a name"));
        }
        let aliases = normalize_aliases(name, aliases);
        let tx = self.conn.unchecked_transaction()?;
        let now = now_unix_ms();
        tx.execute(
            "INSERT INTO topic (name, name_is_user, name_dirty, created_at_unix_ms, updated_at_unix_ms) \
             VALUES (?1, 1, 0, ?2, ?2)",
            params![name, now],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO topic_seed (topic_id, key, kind, aliases, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, new_key(), kind.map(|k| k.as_str()), aliases_json(&aliases)?, now],
        )?;
        mark_seeds_rematch(&tx, id)?;
        queue_rejudge(&tx, id)?;
        enqueue_seed(self, &tx, id)?;
        tx.commit()?;
        self.refresh_seed_vec(id);
        Ok(load(&self.conn, "s.topic_id = ?1", [id])?.remove(0))
    }

    pub fn update_seed_topic(
        &self,
        key: &str,
        name: &str,
        aliases: &[String],
        kind: Option<SeedKind>,
    ) -> rusqlite::Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid("seed topic needs a name"));
        }
        let aliases = normalize_aliases(name, aliases);
        let tx = self.conn.unchecked_transaction()?;
        let id = seed_id(&tx, key)?;
        tx.execute(
            "UPDATE topic SET name = ?2, name_is_user = 1, name_dirty = 0, updated_at_unix_ms = ?3 WHERE id = ?1",
            params![id, name, now_unix_ms()],
        )?;
        tx.execute(
            "UPDATE topic_seed SET kind = ?2, aliases = ?3 WHERE topic_id = ?1",
            params![id, kind.map(|k| k.as_str()), aliases_json(&aliases)?],
        )?;
        bump(&tx, id)?;
        mark_seeds_rematch(&tx, id)?;
        queue_rejudge(&tx, id)?;
        enqueue_seed(self, &tx, id)?;
        tx.commit()?;
        self.refresh_seed_vec(id);
        Ok(())
    }

    /// Marks the seed deleted and drops the members it brought in (`seed`/`jev`);
    /// members the user added or confirmed stay.
    pub fn delete_seed_topic(&self, key: &str) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let id = seed_id(&tx, key)?;
        tx.execute(
            "UPDATE topic_seed SET deleted = 1 WHERE topic_id = ?1",
            [id],
        )?;
        tx.execute(
            "DELETE FROM topic_member WHERE topic_id = ?1 AND origin IN ('seed', 'jev')",
            [id],
        )?;
        bump(&tx, id)?;
        mark_seeds_rematch(&tx, id)?;
        enqueue_seed(self, &tx, id)?;
        tx.commit()
    }

    pub fn list_seed_topics(&self) -> rusqlite::Result<Vec<SeedTopic>> {
        load(
            &self.conn,
            "s.deleted = 0 AND s.merged_into_key IS NULL AND t.merged_into IS NULL",
            [],
        )
    }

    /// WHERE terms every listed suggestion meets, with the HAVING in `SUGGESTION_HAVING`
    /// (rows joined as `t` topic, `m` member, `s` source). Shared with the model naming
    /// pass and the placeholder count so none of them can drift from the list.
    pub(super) fn suggestion_where() -> String {
        format!(
            "t.merged_into IS NULL AND t.id NOT IN (SELECT topic_id FROM topic_seed) \
             AND NOT EXISTS (SELECT 1 FROM kv_meta WHERE key = '{DISMISSED_PREFIX}' || t.id)"
        )
    }

    /// Named auto topics (never a seed, not merged) with at least 5 non-removed members from
    /// at least 2 source kinds, biggest first, minus the ones dismissed on this device.
    pub fn topic_suggestions(&self, limit: usize) -> rusqlite::Result<Vec<TopicSuggestion>> {
        let mut stmt = self.conn.prepare(
            &format!(
                "SELECT t.id, t.name, COUNT(*), GROUP_CONCAT(DISTINCT s.kind) \
                 FROM topic t JOIN topic_member m ON m.topic_id = t.id AND m.origin != 'removed' \
                   JOIN topic_source s ON s.id = m.source_id \
                 WHERE t.name != '' AND t.name_dirty = 0 AND {} \
                 GROUP BY t.id HAVING {SUGGESTION_HAVING} \
                 ORDER BY COUNT(*) DESC, t.id LIMIT ?1",
                Self::suggestion_where()
            ),
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let kinds: String = r.get(3)?;
            Ok(TopicSuggestion {
                topic_id: r.get(0)?,
                name: r.get(1)?,
                item_count: r.get(2)?,
                kinds: kinds.split(',').map(str::to_string).collect(),
            })
        })?;
        rows.collect()
    }

    /// Hides a suggestion on this device only (kv_meta is not synced).
    pub fn dismiss_suggestion(&self, topic_id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO kv_meta (key, value) VALUES (?1 || ?2, '1')",
            params![DISMISSED_PREFIX, topic_id],
        )?;
        Ok(())
    }

    /// Turns an auto topic into a seed named `name`; its members stay.
    pub fn accept_suggestion(&self, topic_id: i64, name: &str) -> rusqlite::Result<SeedTopic> {
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid("seed topic needs a name"));
        }
        let tx = self.conn.unchecked_transaction()?;
        let live: Option<bool> = tx
            .query_row(
                "SELECT merged_into IS NULL FROM topic WHERE id = ?1",
                [topic_id],
                |r| r.get(0),
            )
            .optional()?;
        if live != Some(true) {
            return Err(invalid(format!("topic {topic_id} not found or merged")));
        }
        let now = now_unix_ms();
        tx.execute(
            "UPDATE topic SET name = ?2, name_is_user = 1, name_dirty = 0, updated_at_unix_ms = ?3 WHERE id = ?1",
            params![topic_id, name, now],
        )?;
        // An earlier seed record (deleted) for this topic is revived under its old key, with
        // fresh aliases; a live one keeps its aliases and kind.
        tx.execute(
            "INSERT INTO topic_seed (topic_id, key, aliases, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(topic_id) DO UPDATE SET deleted = 0, \
             aliases = CASE WHEN topic_seed.deleted = 1 THEN excluded.aliases ELSE topic_seed.aliases END",
            params![topic_id, new_key(), aliases_json(&normalize_aliases(name, &[]))?, now],
        )?;
        bump(&tx, topic_id)?;
        mark_seeds_rematch(&tx, topic_id)?;
        queue_rejudge(&tx, topic_id)?;
        enqueue_seed(self, &tx, topic_id)?;
        tx.commit()?;
        self.refresh_seed_vec(topic_id);
        Ok(load(&self.conn, "s.topic_id = ?1", [topic_id])?.remove(0))
    }

    /// Applies a seed pulled from another device. The later `updated_at_unix_ms` wins (a tie
    /// goes to the copy whose JSON hashes higher, the same on every device), stored as sent.
    /// A merge is sticky both ways: an older copy's merge still applies, and a newer unmerged
    /// copy keeps this device's merge (bumped and sent back so the sender converges). A new
    /// key gets its own user-named topic. A deletion drops the members the seed brought in;
    /// a merge runs once its target's key is known here (now, or when that seed arrives).
    /// Otherwise never enqueued, except the live end of a merge cycle this creates
    /// (`break_merge_cycles`).
    pub fn apply_synced_topic_seed(&self, p: &TopicSeedPayload) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT topic_id FROM topic_seed WHERE key = ?1",
                [&p.key],
                |r| r.get(0),
            )
            .optional()?;
        let now = now_unix_ms();
        let aliases = aliases_json(&p.aliases)?;
        let mut wins = true;
        let id = match existing {
            Some(id) => {
                let (mut local, raw) = seed_payload(&tx, id)?
                    .ok_or_else(|| invalid(format!("seed {} has no topic", p.key)))?;
                local.aliases = serde_json::from_str(&raw).unwrap_or_default();
                let ts = local.updated_at_unix_ms;
                wins = super::incoming_wins(p, p.updated_at_unix_ms, &local, ts);
                let local_merged = local.merged_into_key.is_some();
                if wins {
                    tx.execute(
                        "UPDATE topic SET name = ?2, name_is_user = 1, name_dirty = 0, updated_at_unix_ms = ?3 WHERE id = ?1",
                        params![id, p.name, now],
                    )?;
                    // A merge is never undone: a record merged here stays merged.
                    tx.execute(
                        "UPDATE topic_seed SET kind = ?2, aliases = ?3, deleted = ?4, \
                         merged_into_key = COALESCE(merged_into_key, ?5), updated_at_unix_ms = ?6 WHERE topic_id = ?1",
                        params![id, p.kind, aliases, p.deleted, p.merged_into_key, p.updated_at_unix_ms],
                    )?;
                    if local_merged && p.merged_into_key.is_none() {
                        bump(&tx, id)?;
                        enqueue_seed(self, &tx, id)?;
                    }
                } else if !local_merged && p.merged_into_key.is_some() {
                    tx.execute(
                        "UPDATE topic_seed SET merged_into_key = ?2 WHERE topic_id = ?1",
                        params![id, p.merged_into_key],
                    )?;
                } else {
                    return Ok(());
                }
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO topic (name, name_is_user, name_dirty, created_at_unix_ms, updated_at_unix_ms) \
                     VALUES (?1, 1, 0, ?2, ?2)",
                    params![p.name, now],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO topic_seed (topic_id, key, kind, aliases, merged_into_key, deleted, updated_at_unix_ms) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id, p.key, p.kind, aliases, p.merged_into_key, p.deleted, p.updated_at_unix_ms],
                )?;
                id
            }
        };
        if wins && p.deleted {
            tx.execute(
                "DELETE FROM topic_member WHERE topic_id = ?1 AND origin IN ('seed', 'jev')",
                [id],
            )?;
        }
        mark_seeds_rematch(&tx, id)?;
        let mut kept = settle_merges(&tx)?;
        kept.extend(break_merge_cycles(self, &tx)?);
        tx.commit()?;
        for t in std::iter::once(id).chain(kept) {
            self.refresh_seed_vec(t);
        }
        Ok(())
    }

    /// Queues every seed record (deleted and merged ones too) and every verdict for sync,
    /// once per database (kv `topics_sync_backfilled`): rows written before this device had
    /// a device id, or before topic sync existed. `push_content` calls it once every peer can
    /// apply topic rows. Without a device id nothing is queued and nothing is recorded.
    pub fn enqueue_topic_sync_backfill(&self) -> rusqlite::Result<()> {
        if self.own_device_id().is_none() || self.topic_sync_backfilled()? {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        let seeds: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT topic_id FROM topic_seed ORDER BY topic_id")?;
            let rows = stmt
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        for id in seeds {
            enqueue_seed(self, &tx, id)?;
        }
        let verdicts: Vec<String> = {
            let mut stmt =
                tx.prepare("SELECT source_key FROM topic_verdict ORDER BY source_key")?;
            let rows = stmt
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        for key in verdicts {
            super::judge::enqueue_topic_verdict(self, &tx, &key)?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO kv_meta (key, value) VALUES (?1, '1')",
            [SYNC_BACKFILLED_KEY],
        )?;
        tx.commit()
    }

    /// Whether `enqueue_topic_sync_backfill` has run on this database.
    pub fn topic_sync_backfilled(&self) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM kv_meta WHERE key = ?1",
                [SYNC_BACKFILLED_KEY],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Embeds `"{name}: {aliases}"` into `topic_seed.vec`; runs outside write transactions
    /// (embedding can be slow). On failure the old vector stays (NULL for a new seed).
    pub(crate) fn refresh_seed_vec(&self, topic_id: i64) {
        let Ok(Some((name, aliases))) = self
            .conn
            .query_row(
                "SELECT t.name, s.aliases FROM topic_seed s JOIN topic t ON t.id = s.topic_id WHERE s.topic_id = ?1",
                [topic_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
        else {
            return;
        };
        let aliases: Vec<String> = serde_json::from_str(&aliases).unwrap_or_default();
        let Ok(v) = self.embed(&format!("{name}: {}", aliases.join(", "))) else {
            return;
        };
        let blob: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        let _ = self.conn.execute(
            "UPDATE topic_seed SET vec = ?2 WHERE topic_id = ?1",
            params![topic_id, blob],
        );
    }
}

fn seed_id(conn: &Connection, key: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT topic_id FROM topic_seed WHERE key = ?1",
        [key],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| invalid(format!("seed topic {key} not found")))
}

/// Called by `rename_topic` inside its transaction: a renamed seed keeps the new name in
/// its aliases.
pub(crate) fn on_rename(conn: &Connection, topic_id: i64, name: &str) -> rusqlite::Result<bool> {
    let Some(json) = conn
        .query_row(
            "SELECT aliases FROM topic_seed WHERE topic_id = ?1",
            [topic_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(false);
    };
    let old: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
    conn.execute(
        "UPDATE topic_seed SET aliases = ?2 WHERE topic_id = ?1",
        params![topic_id, aliases_json(&normalize_aliases(name, &old))?],
    )?;
    bump(conn, topic_id)?;
    mark_seeds_rematch(conn, topic_id)?;
    Ok(true)
}

/// Called by `merge_topics` inside its transaction. Absorbed seed aliases join the kept
/// seed and the absorbed one records `merged_into_key`. If only the absorbed topic is a
/// seed, the seed record moves to `keep` (which takes the seed's name). Returns whether
/// `keep` is a seed afterwards (so the caller re-embeds it).
pub(crate) fn on_merge(conn: &Connection, keep: i64, absorb: i64) -> rusqlite::Result<bool> {
    let row = |id: i64| -> rusqlite::Result<Option<(String, String, String)>> {
        conn.query_row(
            "SELECT s.key, s.aliases, t.name FROM topic_seed s JOIN topic t ON t.id = s.topic_id WHERE s.topic_id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
    };
    let Some((akey, aaliases, aname)) = row(absorb)? else {
        return Ok(false);
    };
    match row(keep)? {
        Some((kkey, kaliases, kname)) => {
            let mut all: Vec<String> = serde_json::from_str(&kaliases).unwrap_or_default();
            all.extend(serde_json::from_str::<Vec<String>>(&aaliases).unwrap_or_default());
            conn.execute(
                "UPDATE topic_seed SET aliases = ?2 WHERE topic_id = ?1",
                params![keep, aliases_json(&normalize_aliases(&kname, &all))?],
            )?;
            conn.execute(
                "UPDATE topic_seed SET merged_into_key = ?2 WHERE topic_id = ?1",
                params![absorb, kkey],
            )?;
            bump(conn, absorb)?;
            // Seeds that were merged into `absorb` now point at `keep`.
            conn.execute(
                "UPDATE topic_seed SET merged_into_key = ?2 WHERE merged_into_key = ?1",
                params![akey, kkey],
            )?;
            bump(conn, keep)?;
            mark_seeds_rematch(conn, keep)?;
            Ok(true)
        }
        None => {
            conn.execute(
                "UPDATE topic_seed SET topic_id = ?2 WHERE topic_id = ?1",
                [absorb, keep],
            )?;
            conn.execute(
                "UPDATE topic SET name = ?2, name_is_user = 1, name_dirty = 0 WHERE id = ?1",
                params![keep, aname],
            )?;
            bump(conn, keep)?;
            mark_seeds_rematch(conn, keep)?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::FakeEmbedder;
    use crate::topics::schema::prune_orphans;
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s =
            Store::open_with_embedder(&dir.path().join("t.db"), Arc::new(FakeEmbedder)).unwrap();
        (dir, s)
    }

    #[test]
    fn suggestions_need_five_members_from_two_kinds_and_skip_dismissed() {
        let (_d, s) = store();
        let topic = |name: &str, kinds: &[&str]| -> i64 {
            s.conn
                .execute(
                    "INSERT INTO topic (name, name_dirty, created_at_unix_ms, updated_at_unix_ms) VALUES (?1, 0, 1, 1)",
                    [name],
                )
                .unwrap();
            let t = s.conn.last_insert_rowid();
            for (i, k) in kinds.iter().enumerate() {
                s.conn
                    .execute(
                        "INSERT INTO topic_source (kind, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, 1, 1)",
                        [k],
                    )
                    .unwrap();
                let origin = if i == 0 && name.starts_with("Removed") { "removed" } else { "auto" };
                s.conn
                    .execute(
                        "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, ?3)",
                        params![t, s.conn.last_insert_rowid(), origin],
                    )
                    .unwrap();
            }
            t
        };
        let good = topic("Good", &["email", "email", "meeting", "email", "email"]);
        topic("One kind", &["email"; 5]);
        topic("Too few", &["email", "meeting", "email", "meeting"]);
        topic("Removed counts out", &["email", "meeting", "email", "meeting", "email"]);
        let got = s.topic_suggestions(10).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].topic_id, got[0].item_count), (good, 5));
        assert_eq!(got[0].kinds.len(), 2);
        s.dismiss_suggestion(good).unwrap();
        s.dismiss_suggestion(good).unwrap();
        assert!(s.topic_suggestions(10).unwrap().is_empty());
    }

    #[test]
    fn seed_topic_survives_prune_with_no_members() {
        let (_d, s) = store();
        let seed = s.create_seed_topic("Maximus", &[], None).unwrap();
        prune_orphans(&s.conn).unwrap();
        let l = s.list_seed_topics().unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].topic_id, l[0].item_count), (seed.topic_id, 0));
        let vec_len: i64 = s
            .conn
            .query_row("SELECT length(vec) FROM topic_seed", [], |r| r.get(0))
            .unwrap();
        assert!(vec_len > 0);
    }

    #[test]
    fn aliases_include_the_name_once() {
        let (_d, s) = store();
        let t = s
            .create_seed_topic(
                "Genesys AVA",
                &["genesys ava".into(), "AVA".into()],
                Some(SeedKind::Product),
            )
            .unwrap();
        assert_eq!(
            t.aliases,
            vec!["Genesys AVA".to_string(), "AVA".to_string()]
        );
        assert_eq!(t.kind, Some(SeedKind::Product));
    }

    #[test]
    fn merge_of_two_seeds_keeps_one_and_records_merged_into() {
        let (_d, s) = store();
        let a = s
            .create_seed_topic("Acme", &["ACME Corp".into()], None)
            .unwrap();
        let b = s
            .create_seed_topic("Acme Inc", &["Acme Incorporated".into()], None)
            .unwrap();
        s.merge_topics(a.topic_id, b.topic_id).unwrap();
        let merged: String = s
            .conn
            .query_row(
                "SELECT merged_into_key FROM topic_seed WHERE topic_id = ?1",
                [b.topic_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(merged, a.key);
        let l = s.list_seed_topics().unwrap();
        assert_eq!(l.len(), 1);
        for want in ["Acme", "ACME Corp", "Acme Inc", "Acme Incorporated"] {
            assert!(l[0].aliases.iter().any(|x| x == want), "missing {want}");
        }
    }

    #[test]
    fn seed_name_is_never_auto_renamed() {
        let (_d, s) = store();
        let seed = s.create_seed_topic("Maximus", &[], None).unwrap();
        s.apply_topic_name(seed.topic_id, "Other", None).unwrap();
        assert_eq!(s.list_seed_topics().unwrap()[0].name, "Maximus");
    }

    #[test]
    fn rename_update_delete_and_accept() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        s.rename_topic(a.topic_id, "Acme Co").unwrap();
        let l = s.list_seed_topics().unwrap();
        assert_eq!(
            l[0].aliases,
            vec!["Acme Co".to_string(), "Acme".to_string()]
        );
        assert!(l[0].updated_at_unix_ms > a.updated_at_unix_ms);
        s.update_seed_topic(&a.key, "Acme", &["A".into()], Some(SeedKind::Customer))
            .unwrap();
        assert_eq!(
            s.list_seed_topics().unwrap()[0].kind,
            Some(SeedKind::Customer)
        );
        s.delete_seed_topic(&a.key).unwrap();
        assert!(s.list_seed_topics().unwrap().is_empty());
        prune_orphans(&s.conn).unwrap();

        s.conn
            .execute("INSERT INTO topic (name, created_at_unix_ms, updated_at_unix_ms) VALUES ('auto', 1, 1)", [])
            .unwrap();
        let id = s.conn.last_insert_rowid();
        s.conn
            .execute(
                "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, 5, 'auto')",
                [id],
            )
            .unwrap();
        let seed = s.accept_suggestion(id, "Board").unwrap();
        assert_eq!((seed.item_count, seed.name.as_str()), (1, "Board"));
        assert!(s.update_seed_topic("nope", "x", &[], None).is_err());
    }

    #[test]
    fn deleted_seed_with_user_member_is_hidden_everywhere() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        for (sid, o) in [(1, "user"), (2, "user")] {
            s.conn
                .execute(
                    "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, ?3)",
                    params![a.topic_id, sid, o],
                )
                .unwrap();
            s.conn
                .execute("INSERT INTO topic_source (id, kind, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, 'meeting', 1000, 't', 1)", [sid])
                .unwrap();
        }
        assert_eq!(s.list_topics(false, 10).unwrap().len(), 1);
        s.delete_seed_topic(&a.key).unwrap();
        assert!(s.list_topics(false, 10).unwrap().is_empty());
        assert!(s.list_seed_topics().unwrap().is_empty());
        assert!(s.topics_needing_names(10).unwrap().is_empty());
        assert!(s.topic_detail(a.topic_id).unwrap().is_none());
        let n: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM topic_member WHERE topic_id = ?1",
                [a.topic_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
    }

    /// Opposite merges on two devices (here: Y into X locally, X into Y pulled) leave the
    /// seed with the lower key live, on the live topic, and send it so peers agree.
    #[test]
    fn opposite_merges_keep_the_lower_key_live() {
        let (_d, s) = store();
        s.set_own_device_id("me");
        let x = s.create_seed_topic("Acme", &[], None).unwrap();
        let y = s.create_seed_topic("Acme Inc", &[], None).unwrap();
        s.merge_topics(x.topic_id, y.topic_id).unwrap();
        s.apply_synced_topic_seed(&TopicSeedPayload {
            key: x.key.clone(),
            name: "Acme".into(),
            aliases: vec!["Acme".into()],
            kind: None,
            merged_into_key: Some(y.key.clone()),
            deleted: false,
            updated_at_unix_ms: i64::MAX / 2,
        })
        .unwrap();
        let (low, low_name) = if x.key < y.key {
            (&x.key, "Acme")
        } else {
            (&y.key, "Acme Inc")
        };
        let l = s.list_seed_topics().unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!((&l[0].key, l[0].name.as_str()), (low, low_name));
        let sent: Vec<TopicSeedPayload> = s
            .list_outbox_operations()
            .unwrap()
            .into_iter()
            .filter(|o| o.1 == crate::OUTBOX_ROW_TYPE_TOPIC_SEED)
            .map(|o| serde_json::from_str(&o.3).unwrap())
            .collect();
        let last = sent.iter().rev().find(|p| &p.key == low).unwrap();
        assert_eq!(last.merged_into_key, None);
    }

    /// A seed whose stored aliases don't parse is not sent (it would wipe peers' aliases).
    #[test]
    fn corrupt_aliases_are_never_sent() {
        let (_d, s) = store();
        s.create_seed_topic("Acme", &[], None).unwrap();
        let bad = s.create_seed_topic("Board", &[], None).unwrap();
        s.conn
            .execute(
                "UPDATE topic_seed SET aliases = 'not json' WHERE topic_id = ?1",
                [bad.topic_id],
            )
            .unwrap();
        s.set_own_device_id("me");
        s.enqueue_topic_sync_backfill().unwrap();
        let sent: Vec<TopicSeedPayload> = s
            .list_outbox_operations()
            .unwrap()
            .into_iter()
            .filter(|o| o.1 == crate::OUTBOX_ROW_TYPE_TOPIC_SEED)
            .map(|o| serde_json::from_str(&o.3).unwrap())
            .collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].name, "Acme");
    }

    #[test]
    fn pulled_merge_waits_for_its_target() {
        let (_d, s) = store();
        let p = |key: &str, name: &str, merged: Option<&str>, ts: i64| TopicSeedPayload {
            key: key.into(),
            name: name.into(),
            aliases: vec![name.into()],
            kind: None,
            merged_into_key: merged.map(Into::into),
            deleted: false,
            updated_at_unix_ms: ts,
        };
        s.apply_synced_topic_seed(&p("y", "Acme Inc", Some("x"), 5))
            .unwrap();
        assert!(s.list_seed_topics().unwrap().is_empty());
        s.apply_synced_topic_seed(&p("x", "Acme", None, 5)).unwrap();
        let l = s.list_seed_topics().unwrap();
        assert_eq!((l.len(), l[0].key.as_str()), (1, "x"));
        let merged: Option<i64> = s
            .conn
            .query_row(
                "SELECT t.merged_into FROM topic t JOIN topic_seed s ON s.topic_id = t.id WHERE s.key = 'y'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(merged, Some(l[0].topic_id));
        // An older copy loses.
        s.apply_synced_topic_seed(&p("x", "Old", None, 4)).unwrap();
        assert_eq!(s.list_seed_topics().unwrap()[0].name, "Acme");
    }

    fn payload(key: &str, name: &str, deleted: bool, ts: i64) -> TopicSeedPayload {
        TopicSeedPayload {
            key: key.into(),
            name: name.into(),
            aliases: vec![name.into()],
            kind: None,
            merged_into_key: None,
            deleted,
            updated_at_unix_ms: ts,
        }
    }

    /// I1: prune keeps a deleted seed's tombstone, so an older live copy can't revive it.
    #[test]
    fn deleted_seed_stays_deleted_after_prune() {
        let (_d, s) = store();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        s.delete_seed_topic(&a.key).unwrap();
        prune_orphans(&s.conn).unwrap();
        s.apply_synced_topic_seed(&payload(&a.key, "Acme", false, a.updated_at_unix_ms))
            .unwrap();
        assert!(s.list_seed_topics().unwrap().is_empty());
    }

    /// I3: a suggestion accepted and then deleted is not offered again.
    #[test]
    fn deleted_accepted_suggestion_is_not_suggested_again() {
        let (_d, s) = store();
        s.conn
            .execute(
                "INSERT INTO topic (name, name_dirty, created_at_unix_ms, updated_at_unix_ms) VALUES ('Acme', 0, 1, 1)",
                [],
            )
            .unwrap();
        let t = s.conn.last_insert_rowid();
        for k in ["email", "meeting", "email", "meeting", "email"] {
            s.conn
                .execute(
                    "INSERT INTO topic_source (kind, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, 1, 1)",
                    [k],
                )
                .unwrap();
            s.conn
                .execute(
                    "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'auto')",
                    params![t, s.conn.last_insert_rowid()],
                )
                .unwrap();
        }
        assert_eq!(s.topic_suggestions(10).unwrap().len(), 1);
        let seed = s.accept_suggestion(t, "Acme").unwrap();
        s.delete_seed_topic(&seed.key).unwrap();
        assert!(s.topic_suggestions(10).unwrap().is_empty());
    }

    /// Accepting a topic that is already a live seed keeps its aliases and kind.
    #[test]
    fn accept_on_a_live_seed_keeps_aliases_and_kind() {
        let (_d, s) = store();
        let a = s
            .create_seed_topic("Acme", &["ACME Corp".into()], Some(SeedKind::Customer))
            .unwrap();
        let got = s.accept_suggestion(a.topic_id, "Acme").unwrap();
        assert_eq!(
            got.aliases,
            vec!["Acme".to_string(), "ACME Corp".to_string()]
        );
        assert_eq!(got.kind, Some(SeedKind::Customer));
    }

    /// Two copies with the same timestamp and different content: every device keeps the
    /// same one, whichever order they arrive in.
    #[test]
    fn same_timestamp_tie_is_broken_the_same_way_everywhere() {
        let names = |first: &str, second: &str| -> String {
            let (_d, s) = store();
            s.apply_synced_topic_seed(&payload("k", first, false, 5))
                .unwrap();
            s.apply_synced_topic_seed(&payload("k", second, false, 5))
                .unwrap();
            s.list_seed_topics().unwrap()[0].name.clone()
        };
        assert_eq!(names("Acme", "Acme Inc"), names("Acme Inc", "Acme"));
    }

    #[test]
    fn merge_drops_seed_rows_for_rematch() {
        let (_d, s) = store();
        let a = s.create_seed_topic("A", &[], None).unwrap();
        let b = s.create_seed_topic("B", &[], None).unwrap();
        s.conn
            .execute(
                "INSERT INTO topic_source (id, kind, ts_unix_ms, indexed_at_unix_ms, assigned_at_unix_ms) \
                 VALUES (7, 'episode', 1, 1, 5)",
                [],
            )
            .unwrap();
        for (t, o) in [(a.topic_id, "seed"), (b.topic_id, "jev")] {
            s.conn
                .execute(
                    "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, 7, ?2)",
                    params![t, o],
                )
                .unwrap();
        }
        s.merge_topics(a.topic_id, b.topic_id).unwrap();
        // The seed member is dropped for re-matching (alias matching decides again), and
        // only the moved jev row stays.
        let (n, assigned): (i64, Option<i64>) = s
            .conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM topic_member WHERE source_id = 7), \
                        (SELECT assigned_at_unix_ms FROM topic_source WHERE id = 7)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((n, assigned), (1, None));
    }
}
