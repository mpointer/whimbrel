//! Jev judging: the queue of sources alias matching could not decide, their stable
//! cross-device keys, the verdicts, and the token budget the batch runs under.

use rusqlite::{params, Connection, OptionalExtension};

use super::aliases::{alias_hits, alias_snippets};
use super::cluster::{cos, source_vec, unblob};
use super::index::calendar_series_key;
use crate::Store;

use super::budget::TOPIC_JUDGE;
pub use super::budget::{JudgeGate, DAILY_ALLOWANCE_TOKENS};

/// Seeds offered beyond the alias hits, ranked by vector similarity.
const NEAREST_SEEDS: usize = 5;
/// Fixed per-request overhead (instructions, labels, answer) in the estimate. Measured on
/// 5,862 real calls: 848 input + 77 output tokens a call against ~200 tokens of item text.
const REQUEST_TOKENS: i64 = 725;
/// Passages added per topic named past the stored-text cap, and characters either side.
const SNIPPETS_PER_TOPIC: usize = 3;
const SNIPPET_RADIUS: usize = 120;
/// Failed calls (other than auth or transient) before a source leaves the queue unjudged.
const MAX_ATTEMPTS: i64 = 3;

pub struct JudgeCandidate {
    pub source_id: i64,
    pub source_key: String,
    pub kind: String,
    pub text: String,
    /// (topic_id, name, other aliases joined with ", "): alias hits first.
    pub topics: Vec<(i64, String, String)>,
    /// The ids in `topics` whose seed kind is `subject` (broad: ML, Rust), which need a
    /// surer pick.
    pub subjects: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JudgeEstimate {
    pub sources: i64,
    pub tokens: i64,
    pub usd: f64,
    /// Sorting isn't done: no live seed yet, or the last `run_topic_batch` had more work
    /// (or none has run). The app shows no estimate or Start meanwhile.
    pub indexing: bool,
}

/// Estimated tokens for judging one source: its text, capped as indexing caps it, at 4
/// characters a token plus `REQUEST_TOKENS`. `topic_judge_estimate` sums the same.
pub fn judge_request_tokens(kind: &str, text: &str) -> i64 {
    let cap = if kind == "episode" { 1_500 } else { 2_000 };
    text.chars().count().min(cap) as i64 / 4 + REQUEST_TOKENS
}

/// The source's key on every device: `email:{gmail id}`, `calendar:{series}`,
/// `meeting:{origin device}:{origin id}`, `episode:{device}:{id}`, `snapshot:{device}:{id}`.
/// `None` when the row is gone or has no stable id. A local meeting uses `own_device`
/// (falling back to the meeting's `device_id` when unset).
pub(crate) fn source_key(
    conn: &Connection,
    own_device: Option<&str>,
    sid: i64,
) -> rusqlite::Result<Option<String>> {
    let Some((kind, r)): Option<(String, i64)> = conn
        .query_row(
            "SELECT kind, ref_id FROM topic_source WHERE id = ?1 AND ref_id IS NOT NULL",
            [sid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let one = |sql: &str| -> rusqlite::Result<Option<(Option<String>, Option<String>)>> {
        conn.query_row(sql, [r], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()
    };
    Ok(match kind.as_str() {
        "email" => one("SELECT external_id, NULL FROM items WHERE id = ?1")?
            .and_then(|(x, _)| x)
            .map(|x| format!("email:{x}")),
        "calendar_event" => match one("SELECT source, external_id FROM items WHERE id = ?1")? {
            Some((Some(s), Some(x))) => Some(format!("calendar:{}", calendar_series_key(&s, &x))),
            _ => None,
        },
        "snapshot" => one("SELECT device_id, NULL FROM items WHERE id = ?1")?
            .and_then(|(d, _)| d)
            .map(|d| format!("snapshot:{d}:{r}")),
        "episode" => one("SELECT device_id, NULL FROM episodes WHERE id = ?1")?
            .and_then(|(d, _)| d)
            .map(|d| format!("episode:{d}:{r}")),
        "meeting" => {
            let synced: Option<(String, i64)> = conn
                .query_row(
                    "SELECT peer_device_id, source_meeting_id FROM sync_meeting_map WHERE local_meeting_id = ?1",
                    [r],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            match synced {
                Some((peer, id)) => Some(format!("meeting:{peer}:{id}")),
                None => one("SELECT device_id, NULL FROM meeting WHERE id = ?1")?
                    .and_then(|(d, _)| own_device.map(str::to_string).or(d))
                    .map(|d| format!("meeting:{d}:{r}")),
            }
        }
        _ => None,
    })
}

/// Applies a verdict to a source: a `'jev'` member of the live seed topic keyed
/// `topic_key`, following `merged_into_key` (none when that chain ends at a deleted seed;
/// upgrades an `auto` row), noise when `work < 0.5`, and off the queue.
fn apply_verdict(
    conn: &Connection,
    sid: i64,
    topic_key: Option<&str>,
    work: Option<f64>,
) -> rusqlite::Result<()> {
    if let Some(key) = topic_key {
        conn.execute(
            "WITH RECURSIVE chain(key, depth) AS (SELECT ?1, 0 UNION ALL \
                 SELECT s.merged_into_key, depth + 1 FROM topic_seed s JOIN chain c ON s.key = c.key \
                 WHERE s.merged_into_key IS NOT NULL AND depth < 16) \
             INSERT INTO topic_member (topic_id, source_id, origin, score) \
             SELECT s.topic_id, ?2, 'jev', 1.0 FROM topic_seed s JOIN topic t ON t.id = s.topic_id \
             WHERE s.key IN (SELECT key FROM chain) AND s.deleted = 0 AND s.merged_into_key IS NULL AND t.merged_into IS NULL \
             ON CONFLICT(topic_id, source_id) DO UPDATE SET origin = 'jev' WHERE origin = 'auto'",
            params![key, sid],
        )?;
    }
    if work.is_some_and(|w| w < 0.5) {
        conn.execute("UPDATE topic_source SET noise = 1 WHERE id = ?1", [sid])?;
    }
    conn.execute("DELETE FROM topic_judge_todo WHERE source_id = ?1", [sid])?;
    Ok(())
}

/// Called by alias filing for a source it cannot decide: applies the verdict stored for
/// its key (judged here earlier, or pulled from another device) and returns true, or
/// returns false when there is none and the source should be queued.
pub(crate) fn apply_stored_verdict(
    conn: &Connection,
    own_device: Option<&str>,
    sid: i64,
) -> rusqlite::Result<bool> {
    let Some(key) = source_key(conn, own_device, sid)? else {
        return Ok(false);
    };
    let rejudge = conn
        .query_row(
            "SELECT 1 FROM topic_rejudge WHERE source_id = ?1",
            [sid],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if rejudge {
        return Ok(false);
    }
    let verdict: Option<(Option<String>, Option<f64>)> = conn
        .query_row(
            "SELECT topic_key, work FROM topic_verdict WHERE source_key = ?1",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match verdict {
        Some((topic_key, work)) => {
            apply_verdict(conn, sid, topic_key.as_deref(), work)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// A verdict as it travels between devices (`topic_verdict` sync rows).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopicVerdictPayload {
    pub source_key: String,
    pub topic_key: Option<String>,
    pub work: Option<f64>,
    pub model: String,
    pub judged_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

fn verdict_payload(
    conn: &Connection,
    source_key: &str,
) -> rusqlite::Result<Option<TopicVerdictPayload>> {
    conn.query_row(
        "SELECT source_key, topic_key, work, model, judged_at_unix_ms, updated_at_unix_ms \
         FROM topic_verdict WHERE source_key = ?1",
        [source_key],
        |r| {
            Ok(TopicVerdictPayload {
                source_key: r.get(0)?,
                topic_key: r.get(1)?,
                work: r.get(2)?,
                model: r.get(3)?,
                judged_at_unix_ms: r.get(4)?,
                updated_at_unix_ms: r.get(5)?,
            })
        },
    )
    .optional()
}

/// Queues the stored verdict for `source_key` for sync to the user's other devices, in the
/// caller's transaction.
pub(crate) fn enqueue_topic_verdict(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    source_key: &str,
) -> rusqlite::Result<()> {
    match verdict_payload(tx, source_key)? {
        Some(p) => store.enqueue_topic_verdict_outbox(tx, &p),
        None => Ok(()),
    }
}

/// Local sources whose key is `key`. Candidates are narrowed by the key's own parts, then
/// confirmed with `source_key`.
pub(crate) fn sources_for_key(
    conn: &Connection,
    own_device: Option<&str>,
    key: &str,
) -> rusqlite::Result<Vec<i64>> {
    let Some((prefix, rest)) = key.split_once(':') else {
        return Ok(Vec::new());
    };
    let id = rest.rsplit_once(':').map_or("", |(_, id)| id);
    let (kind, filter, arg) = match prefix {
        "email" => (
            "email",
            "s.ref_id IN (SELECT id FROM items WHERE kind = 'email' AND external_id = ?2)",
            rest,
        ),
        // `calendar:{source}:{series}`: every instance id starts with its series.
        "calendar" => (
            "calendar_event",
            "s.ref_id IN (SELECT id FROM items WHERE kind = 'calendar_event' AND external_id LIKE ?2 || '%')",
            rest.split_once(':').map_or(rest, |(_, series)| series),
        ),
        "episode" | "snapshot" => (prefix, "s.ref_id = ?2", id),
        "meeting" => (
            "meeting",
            "s.ref_id = ?2 OR s.ref_id IN (SELECT local_meeting_id FROM sync_meeting_map WHERE source_meeting_id = ?2)",
            id,
        ),
        _ => return Ok(Vec::new()),
    };
    let candidates: Vec<i64> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT s.id FROM topic_source s WHERE s.kind = ?1 AND ({filter})"
        ))?;
        let rows = stmt
            .query_map(params![kind, arg], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    let mut out = Vec::new();
    for sid in candidates {
        if source_key(conn, own_device, sid)?.as_deref() == Some(key) {
            out.push(sid);
        }
    }
    Ok(out)
}

fn kv_i64(conn: &Connection, key: &str) -> rusqlite::Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT value FROM kv_meta WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .and_then(|v| v.parse().ok()))
}

pub(crate) fn has_live_seed(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM topic_seed s JOIN topic t ON t.id = s.topic_id \
         WHERE s.deleted = 0 AND s.merged_into_key IS NULL AND t.merged_into IS NULL)",
        [],
        |r| r.get(0),
    )
}

impl Store {
    pub fn source_key(&self, source_id: i64) -> rusqlite::Result<Option<String>> {
        source_key(&self.conn, self.own_device_id().as_deref(), source_id)
    }

    /// Up to `limit` queued sources, newest first, each with the seed topics to offer:
    /// its alias hits, then the `NEAREST_SEEDS` other live seeds closest to its vector.
    /// Empty while no live seed exists; the queue is kept for when one does. Queue rows
    /// whose source is gone, noise, or keyless are dropped here.
    pub fn next_judge_candidates(&self, limit: usize) -> rusqlite::Result<Vec<JudgeCandidate>> {
        #[allow(clippy::type_complexity)]
        let seeds: Vec<(i64, String, Vec<String>, Option<Vec<f32>>, bool)> = {
            let mut stmt = self.conn.prepare(
                "SELECT s.topic_id, t.name, s.aliases, s.vec, s.kind = 'subject' FROM topic_seed s JOIN topic t ON t.id = s.topic_id \
                 WHERE s.deleted = 0 AND s.merged_into_key IS NULL AND t.merged_into IS NULL \
                 ORDER BY s.topic_id",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                        r.get::<_, Option<Vec<u8>>>(3)?.map(|b| unblob(&b)),
                        r.get::<_, Option<bool>>(4)?.unwrap_or(false),
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        if seeds.is_empty() {
            return Ok(Vec::new());
        }
        self.conn.execute(
            "DELETE FROM topic_judge_todo WHERE source_id NOT IN (SELECT id FROM topic_source WHERE noise = 0)",
            [],
        )?;
        let todo: Vec<(i64, String, String, Option<i64>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT s.id, s.kind, COALESCE(x.text, s.title), s.ref_id FROM topic_judge_todo j \
                 JOIN topic_source s ON s.id = j.source_id \
                 LEFT JOIN topic_source_text x ON x.source_id = s.id \
                 ORDER BY s.ts_unix_ms DESC, s.id DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map([limit as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        let alias_sets: Vec<(i64, Vec<String>)> =
            seeds.iter().map(|s| (s.0, s.2.clone())).collect();
        let own = self.own_device_id();
        let mut out = Vec::new();
        for (sid, kind, mut text, ref_id) in todo {
            let Some(key) = source_key(&self.conn, own.as_deref(), sid)? else {
                self.conn
                    .execute("DELETE FROM topic_judge_todo WHERE source_id = ?1", [sid])?;
                continue;
            };
            let mut hits = alias_hits(&text, &alias_sets);
            // Long emails and meetings: names past the stored-text cap still count, with the
            // passages around them added for Jev. Screen text is left out on purpose (its
            // deep mentions are sidebars and tab bars).
            if matches!(kind.as_str(), "email" | "meeting") {
                if let Some(full) = full_source_text(&self.conn, &kind, ref_id)? {
                    for (id, aliases) in &alias_sets {
                        if hits.contains(id)
                            || alias_hits(&full, &[(*id, aliases.clone())]).is_empty()
                        {
                            continue;
                        }
                        hits.push(*id);
                        for p in alias_snippets(&full, aliases, SNIPPET_RADIUS, SNIPPETS_PER_TOPIC)
                        {
                            text.push_str("\n…");
                            text.push_str(&crate::redact::redact_secrets(&p));
                            text.push('…');
                        }
                    }
                }
            }
            let sv = source_vec(&self.conn, sid)?;
            let mut rest: Vec<(f64, i64)> = seeds
                .iter()
                .filter(|s| !hits.contains(&s.0))
                .map(|s| {
                    let sim = match (&sv, &s.3) {
                        (Some(a), Some(b)) => cos(a, b),
                        _ => 0.0,
                    };
                    (sim, s.0)
                })
                .collect();
            rest.sort_by(|a, b| b.0.total_cmp(&a.0)); // stable: ties keep seed order
            let topics: Vec<(i64, String, String)> = hits
                .iter()
                .copied()
                .chain(rest.iter().take(NEAREST_SEEDS).map(|r| r.1))
                .filter_map(|id| seeds.iter().find(|s| s.0 == id))
                .map(|(id, name, aliases, _, _)| {
                    let also: Vec<&str> = aliases
                        .iter()
                        .map(String::as_str)
                        .filter(|a| a.to_lowercase() != name.to_lowercase())
                        .collect();
                    (*id, name.clone(), also.join(", "))
                })
                .collect();
            let subjects = topics
                .iter()
                .filter(|t| seeds.iter().any(|s| s.0 == t.0 && s.4))
                .map(|t| t.0)
                .collect();
            out.push(JudgeCandidate {
                source_id: sid,
                source_key: key,
                kind,
                text,
                topics,
                subjects,
            });
        }
        Ok(out)
    }

    /// Stores Jev's verdict for a source (by its key, the topic by seed key), applies it,
    /// and queues it for sync. A source without a key just leaves the queue.
    pub fn record_topic_verdict(
        &self,
        source_id: i64,
        topic_id: Option<i64>,
        work: Option<f64>,
        model: &str,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let Some(key) = source_key(&tx, self.own_device_id().as_deref(), source_id)? else {
            tx.execute(
                "DELETE FROM topic_judge_todo WHERE source_id = ?1",
                [source_id],
            )?;
            return tx.commit();
        };
        let topic_key: Option<String> = match topic_id {
            Some(t) => tx
                .query_row("SELECT key FROM topic_seed WHERE topic_id = ?1", [t], |r| {
                    r.get(0)
                })
                .optional()?,
            None => None,
        };
        tx.execute(
            "INSERT INTO topic_verdict (source_key, topic_key, work, model, judged_at_unix_ms, updated_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?5) ON CONFLICT(source_key) DO UPDATE SET topic_key = excluded.topic_key, \
             work = excluded.work, model = excluded.model, judged_at_unix_ms = excluded.judged_at_unix_ms, \
             updated_at_unix_ms = MAX(excluded.updated_at_unix_ms, topic_verdict.updated_at_unix_ms + 1)",
            params![key, topic_key, work, model, now_ms],
        )?;
        // A re-judged source's earlier Jev pick gives way to this one.
        tx.execute(
            "DELETE FROM topic_member WHERE source_id = ?1 AND origin = 'jev'",
            [source_id],
        )?;
        tx.execute(
            "DELETE FROM topic_rejudge WHERE source_id = ?1",
            [source_id],
        )?;
        apply_verdict(&tx, source_id, topic_key.as_deref(), work)?;
        enqueue_topic_verdict(self, &tx, &key)?;
        tx.commit()
    }

    /// Applies a verdict pulled from another device: the later `updated_at_unix_ms` wins (a
    /// tie goes to the copy whose JSON hashes higher, the same on every device). Stored even when no source here has that key yet (alias
    /// filing applies it once one is indexed). A source here with the key gets it now: its
    /// `'jev'` member is replaced by the new verdict's (none when it names no live seed) and
    /// its noise follows `work`; `seed` and `user` members stay. Never enqueued.
    pub fn apply_synced_topic_verdict(&self, p: &TopicVerdictPayload) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let wins = match verdict_payload(&tx, &p.source_key)? {
            Some(l) => super::incoming_wins(p, p.updated_at_unix_ms, &l, l.updated_at_unix_ms),
            None => true,
        };
        if wins {
            tx.execute(
                "INSERT OR REPLACE INTO topic_verdict (source_key, topic_key, work, model, judged_at_unix_ms, updated_at_unix_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    p.source_key,
                    p.topic_key,
                    p.work,
                    p.model,
                    p.judged_at_unix_ms,
                    p.updated_at_unix_ms
                ],
            )?;
            let own = self.own_device_id();
            for sid in sources_for_key(&tx, own.as_deref(), &p.source_key)? {
                tx.execute(
                    "DELETE FROM topic_member WHERE source_id = ?1 AND origin = 'jev'",
                    [sid],
                )?;
                if p.work.is_some_and(|w| w >= 0.5) {
                    tx.execute("UPDATE topic_source SET noise = 0 WHERE id = ?1", [sid])?;
                }
                apply_verdict(&tx, sid, p.topic_key.as_deref(), p.work)?;
            }
        }
        tx.commit()
    }

    /// What judging the current queue would cost: per source, its text (capped as
    /// indexing caps it) at 4 characters a token plus `REQUEST_TOKENS`. `indexing` while no
    /// live seed exists or the last `run_topic_batch` had more work (or none has run).
    pub fn topic_judge_estimate(&self, usd_per_million: f64) -> rusqlite::Result<JudgeEstimate> {
        let (sources, tokens): (i64, i64) = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(MIN(LENGTH(COALESCE(x.text, s.title)), \
                    CASE WHEN s.kind = 'episode' THEN 1500 ELSE 2000 END) / 4 + ?1), 0) \
             FROM topic_judge_todo j JOIN topic_source s ON s.id = j.source_id \
             LEFT JOIN topic_source_text x ON x.source_id = s.id WHERE s.noise = 0",
            [REQUEST_TOKENS],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(JudgeEstimate {
            sources,
            tokens,
            usd: tokens as f64 * usd_per_million / 1e6,
            indexing: kv_i64(&self.conn, super::BATCH_MORE_KEY)? != Some(0)
                || !has_live_seed(&self.conn)?,
        })
    }

    /// Starts a first pass: judging up to `budget_tokens` (the caller passes 1.5 × the
    /// estimate) until the queue drains, then `DAILY_ALLOWANCE_TOKENS` a day. Resets the
    /// tokens used. Judging never runs before this.
    pub fn start_topic_judging(&self, budget_tokens: i64) -> rusqlite::Result<()> {
        TOPIC_JUDGE.start(&self.conn, budget_tokens)
    }

    /// Whether judging may call TypeSafe now.
    pub fn topic_judge_gate(&self) -> rusqlite::Result<JudgeGate> {
        TOPIC_JUDGE.gate(&self.conn, crate::now_unix_ms())
    }

    /// Ends the first pass once judging has started, a live seed exists and nothing is
    /// left to judge. Called after each judge batch.
    pub fn end_topic_judge_first_pass_if_drained(&self) -> rusqlite::Result<()> {
        if TOPIC_JUDGE.budget(&self.conn)?.is_none()
            || TOPIC_JUDGE.first_pass_done(&self.conn)?
            || !has_live_seed(&self.conn)?
        {
            return Ok(());
        }
        let left: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM topic_judge_todo j JOIN topic_source s ON s.id = j.source_id WHERE s.noise = 0",
            [],
            |r| r.get(0),
        )?;
        if left == 0 {
            TOPIC_JUDGE.mark_first_pass_done(&self.conn)?;
        }
        Ok(())
    }

    /// `(budget, used)` tokens, or `None` before `start_topic_judging`.
    pub fn topic_judge_budget(&self) -> rusqlite::Result<Option<(i64, i64)>> {
        TOPIC_JUDGE.budget(&self.conn)
    }

    /// Counts a failed Jev call for a queued source (not an auth or transient failure). The
    /// `MAX_ATTEMPTS`th removes it from the queue without a verdict; returns whether it did.
    pub fn record_topic_judge_failure(&self, source_id: i64) -> rusqlite::Result<bool> {
        self.conn.execute(
            "UPDATE topic_judge_todo SET attempts = attempts + 1 WHERE source_id = ?1",
            [source_id],
        )?;
        Ok(self.conn.execute(
            "DELETE FROM topic_judge_todo WHERE source_id = ?1 AND attempts >= ?2",
            params![source_id, MAX_ATTEMPTS],
        )? > 0)
    }

    /// Whole cents to log for a call of `tokens`: the running cost's cents after the call minus
    /// before it, so per-call fractions of a cent carry instead of each rounding up to a cent.
    pub fn topic_judge_cost_cents(
        &self,
        tokens: i64,
        usd_per_million: f64,
    ) -> rusqlite::Result<i64> {
        TOPIC_JUDGE.cost_cents(&self.conn, tokens, usd_per_million)
    }

    /// Counts tokens spent judging: toward the first pass budget, and after the first pass
    /// toward today's allowance.
    pub fn add_topic_judge_used_tokens(&self, tokens: i64) -> rusqlite::Result<()> {
        TOPIC_JUDGE.add_used(&self.conn, tokens, crate::now_unix_ms())
    }
}

/// The whole text of an email (subject and body) or a finished meeting (notes, summary,
/// action items and transcript), for names the stored text's cap cut off.
fn full_source_text(
    conn: &Connection,
    kind: &str,
    ref_id: Option<i64>,
) -> rusqlite::Result<Option<String>> {
    let Some(id) = ref_id else { return Ok(None) };
    match kind {
        "email" => conn
            .query_row(
                "SELECT COALESCE(title, '') || char(10) || COALESCE(body_text, '') FROM items WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional(),
        "meeting" => conn
            .query_row(
                "SELECT COALESCE(m.title, '') || char(10) || COALESCE(m.notes, '') || char(10) \
                        || COALESCE(m.summary_tldr, '') || char(10) \
                        || COALESCE((SELECT group_concat(text, char(10)) FROM meeting_action_item WHERE meeting_id = m.id), '') \
                        || char(10) || COALESCE((SELECT group_concat(i.body_text, ' ') FROM transcript_segment t \
                             JOIN items i ON i.source_table = 'transcript_segment' AND i.source_id = t.id \
                             WHERE t.meeting_id = m.id), '') \
                 FROM meeting m WHERE m.id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional(),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use crate::topics::cluster::run_topic_cluster_job;
    use crate::topics::index::run_topic_index_job;
    use crate::Store;

    fn mail(s: &Store, ext: &str, subject: &str) -> i64 {
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                 VALUES ('d','email','gmail',1000,json_array('a@x.com','b@y.com'),?1,'Budget review','{}',?2)",
                params![subject, ext],
            )
            .unwrap();
        s.conn.last_insert_rowid()
    }

    fn index(s: &Store) {
        while run_topic_index_job(s, 200).unwrap() {}
        while run_topic_cluster_job(s, 200).unwrap() {}
    }

    fn count(s: &Store, sql: &str) -> i64 {
        s.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn sid(s: &Store, title: &str) -> i64 {
        s.conn
            .query_row(
                "SELECT id FROM topic_source WHERE title = ?1",
                [title],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn source(s: &Store, kind: &str, ref_id: i64) -> i64 {
        s.conn
            .execute(
                "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, 0, '', 0)",
                params![kind, ref_id],
            )
            .unwrap();
        s.conn.last_insert_rowid()
    }

    #[test]
    fn estimate_counts_capped_text() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..2 {
            let id = source(&s, "email", 100 + i);
            s.conn
                .execute(
                    "INSERT INTO topic_source_text (source_id, text) VALUES (?1, ?2)",
                    params![id, "x".repeat(10_000)],
                )
                .unwrap();
            s.conn
                .execute("INSERT INTO topic_judge_todo (source_id) VALUES (?1)", [id])
                .unwrap();
        }
        let e = s.topic_judge_estimate(2.0).unwrap();
        assert_eq!(e.sources, 2);
        assert_eq!(e.tokens, 2 * (2000 / 4 + 725));
        assert!((e.usd - e.tokens as f64 * 2.0 / 1e6).abs() < 1e-12);
    }

    /// TypeSafe calls cost thousandths of a cent: whole cents are logged as the running
    /// total crosses them, so the usage log sums to the real cost.
    #[test]
    fn cost_cents_carry_fractions_across_calls() {
        let s = Store::open_in_memory().unwrap();
        let cents: Vec<i64> = (0..5)
            .map(|_| s.topic_judge_cost_cents(4_000, 1.0).unwrap())
            .collect();
        assert_eq!(cents, vec![0, 0, 1, 0, 1]);
    }

    /// I4: the estimate says indexing while there is no live seed or the last background
    /// batch had more to do.
    #[test]
    fn estimate_reports_indexing_until_a_seed_exists_and_the_batch_is_idle() {
        let s = Store::open_in_memory().unwrap();
        assert!(
            s.topic_judge_estimate(1.0).unwrap().indexing,
            "no batch yet"
        );
        mail(&s, "e1", "Lunch plans");
        while crate::topics::run_topic_batch(&s, 200).unwrap() {}
        assert!(
            s.topic_judge_estimate(1.0).unwrap().indexing,
            "no live seed"
        );
        s.create_seed_topic("Maximus", &[], None).unwrap();
        mail(&s, "e2", "Dinner plans");
        mail(&s, "e3", "Breakfast plans");
        assert!(crate::topics::run_topic_batch(&s, 1).unwrap());
        assert!(
            s.topic_judge_estimate(1.0).unwrap().indexing,
            "batch has more"
        );
        while crate::topics::run_topic_batch(&s, 200).unwrap() {}
        let e = s.topic_judge_estimate(1.0).unwrap();
        assert!(!e.indexing);
        assert_eq!(e.sources, 3);
    }

    #[test]
    fn source_keys_are_stable_across_devices() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("me");
        let item = mail(&s, "gm-1", "Hi");
        assert_eq!(
            s.source_key(source(&s, "email", item)).unwrap().as_deref(),
            Some("email:gm-1")
        );
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, body_text, external_id) \
                 VALUES ('d','calendar_event','google_calendar',1,'','abc_20261105T150000Z#1')",
                [],
            )
            .unwrap();
        let cal = s.conn.last_insert_rowid();
        assert_eq!(
            s.source_key(source(&s, "calendar_event", cal)).unwrap(),
            Some(format!(
                "calendar:{}",
                crate::topics::index::calendar_series_key(
                    "google_calendar",
                    "abc_20261105T150000Z#1"
                )
            ))
        );
        s.conn
            .execute("INSERT INTO items (device_id, kind, ts_unix_ms, body_text) VALUES ('dev-s','snapshot',1,'')", [])
            .unwrap();
        let snap = s.conn.last_insert_rowid();
        assert_eq!(
            s.source_key(source(&s, "snapshot", snap)).unwrap(),
            Some(format!("snapshot:dev-s:{snap}"))
        );
        s.conn
            .execute(
                "INSERT INTO episodes (device_id, app_bundle_id, start_ts_unix_ms, end_ts_unix_ms, start_item_id, \
                 end_item_id, item_count, created_at_unix_ms, updated_at_unix_ms) VALUES ('dev-e','a',0,0,0,0,0,0,0)",
                [],
            )
            .unwrap();
        let ep = s.conn.last_insert_rowid();
        assert_eq!(
            s.source_key(source(&s, "episode", ep)).unwrap(),
            Some(format!("episode:dev-e:{ep}"))
        );
        for _ in 0..2 {
            s.conn
                .execute(
                    "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status) \
                     VALUES ('x', 0, 'm', '[]', 'manual', 'done')",
                    [],
                )
                .unwrap();
        }
        let synced = s.conn.last_insert_rowid();
        let local = synced - 1;
        s.record_sync_meeting_map("peer", 42, synced).unwrap();
        assert_eq!(
            s.source_key(source(&s, "meeting", local)).unwrap(),
            Some(format!("meeting:me:{local}"))
        );
        assert_eq!(
            s.source_key(source(&s, "meeting", synced))
                .unwrap()
                .as_deref(),
            Some("meeting:peer:42")
        );
    }

    #[test]
    fn no_live_seeds_means_no_candidates() {
        let s = Store::open_in_memory().unwrap();
        let seed = s.create_seed_topic("Maximus", &[], None).unwrap();
        mail(&s, "e1", "Lunch plans");
        index(&s);
        assert_eq!(s.next_judge_candidates(10).unwrap().len(), 1);
        s.delete_seed_topic(&seed.key).unwrap();
        assert!(s.next_judge_candidates(10).unwrap().is_empty());
    }

    #[test]
    fn candidates_list_alias_hits_first_then_other_seeds() {
        let s = Store::open_in_memory().unwrap();
        let a = s
            .create_seed_topic("Acme", &["Acme Corp".into()], None)
            .unwrap();
        let b = s.create_seed_topic("Maximus", &[], None).unwrap();
        let c = s.create_seed_topic("CMS", &[], None).unwrap();
        mail(&s, "e1", "Maximus CMS handoff");
        index(&s);
        let got = s.next_judge_candidates(10).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source_key, "email:e1");
        assert_eq!(got[0].kind, "email");
        assert!(got[0].text.contains("Maximus CMS handoff"));
        let ids: Vec<i64> = got[0].topics.iter().map(|t| t.0).collect();
        assert_eq!(ids, vec![b.topic_id, c.topic_id, a.topic_id]);
        assert_eq!(
            got[0].topics[2],
            (a.topic_id, "Acme".into(), "Acme Corp".into())
        );
    }

    /// A name past the stored-text cap in a long email still reaches Jev: the topic is a
    /// candidate and the passage around the name is added to the text Jev reads.
    #[test]
    fn mention_past_the_cap_is_a_candidate_with_its_context() {
        let s = Store::open_in_memory().unwrap();
        let m = s.create_seed_topic("Maximus", &[], None).unwrap();
        let body = format!(
            "{} we agreed the Maximus Canada pilot pricing today",
            "filler ".repeat(600)
        );
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                 VALUES ('d','email','gmail',1000,json_array('a@x.com','b@y.com'),'Weekly notes',?1,'{}','e1')",
                [&body],
            )
            .unwrap();
        index(&s);
        let got = s.next_judge_candidates(10).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].topics[0].0, m.topic_id);
        assert!(
            got[0].text.contains("Maximus Canada pilot pricing"),
            "{}",
            got[0].text
        );
    }

    /// Adding a topic re-asks Jev about items that mention it, even ones already judged
    /// "none"; a topic pulled from another device does not (that device asks).
    #[test]
    fn new_topic_rejudges_items_that_mention_it() {
        let s = Store::open_in_memory().unwrap();
        s.create_seed_topic("Maximus", &[], None).unwrap();
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                 VALUES ('d','email','gmail',1000,json_array('a@x.com','b@y.com'),'Weekly notes','Zurich renewal is due','{}','e1')",
                [],
            )
            .unwrap();
        index(&s);
        let sid = sid(&s, "Weekly notes");
        s.record_topic_verdict(sid, None, None, "jev", 5).unwrap();
        index(&s);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_judge_todo"), 0);
        let z = s.create_seed_topic("Zurich", &[], None).unwrap();
        index(&s);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_judge_todo"), 1);
        // The new verdict replaces any earlier Jev pick.
        s.record_topic_verdict(sid, Some(z.topic_id), None, "jev", 9)
            .unwrap();
        index(&s);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_judge_todo"), 0);
        assert_eq!(
            count(
                &s,
                &format!(
                    "SELECT COUNT(*) FROM topic_member WHERE source_id = {sid} AND origin = 'jev'"
                )
            ),
            1
        );
    }

    /// Subject topics are listed so a stricter bar can apply to them.
    #[test]
    fn candidates_mark_subject_topics() {
        let s = Store::open_in_memory().unwrap();
        let ml = s
            .create_seed_topic(
                "ML",
                &["machine learning".into()],
                Some(crate::topics::seeds::SeedKind::Subject),
            )
            .unwrap();
        s.create_seed_topic(
            "Maximus",
            &[],
            Some(crate::topics::seeds::SeedKind::Customer),
        )
        .unwrap();
        mail(&s, "e1", "Lunch plans");
        index(&s);
        let got = s.next_judge_candidates(10).unwrap();
        assert_eq!(got[0].subjects, vec![ml.topic_id]);
    }

    /// R1: a verdict already stored for a source keeps it out of the queue; a "not work"
    /// verdict makes it noise.
    #[test]
    fn stored_verdict_keeps_source_out_of_the_queue() {
        let s = Store::open_in_memory().unwrap();
        s.create_seed_topic("Maximus", &[], None).unwrap();
        s.conn
            .execute(
                "INSERT INTO topic_verdict (source_key, topic_key, work, model, judged_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('email:e1', NULL, 0.2, 'jev', 1, 1)",
                [],
            )
            .unwrap();
        mail(&s, "e1", "Lunch plans");
        mail(&s, "e2", "Dinner plans");
        index(&s);
        let e1 = sid(&s, "Lunch plans");
        assert_eq!(
            count(
                &s,
                &format!("SELECT COUNT(*) FROM topic_judge_todo WHERE source_id = {e1}")
            ),
            0
        );
        assert_eq!(
            count(
                &s,
                &format!("SELECT noise FROM topic_source WHERE id = {e1}")
            ),
            1
        );
        assert_eq!(
            count(&s, "SELECT COUNT(*) FROM topic_judge_todo"),
            1,
            "e2 still queued"
        );
    }

    /// R1: a verdict pulled from another device before the source was indexed takes
    /// effect when the source is filed.
    #[test]
    fn verdict_pulled_before_indexing_applies_when_filed() {
        let s = Store::open_in_memory().unwrap();
        let seed = s.create_seed_topic("Maximus", &[], None).unwrap();
        s.conn
            .execute(
                "INSERT INTO topic_verdict (source_key, topic_key, work, model, judged_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('email:e1', ?1, NULL, 'jev', 1, 1)",
                [&seed.key],
            )
            .unwrap();
        index(&s);
        mail(&s, "e1", "Pilot notes");
        index(&s);
        let e1 = sid(&s, "Pilot notes");
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_judge_todo"), 0);
        let origin: String = s
            .conn
            .query_row(
                "SELECT origin FROM topic_member WHERE topic_id = ?1 AND source_id = ?2",
                [seed.topic_id, e1],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(origin, "jev");
    }

    /// A verdict naming a seed that was merged away lands on the seed it merged into.
    #[test]
    fn verdict_for_a_merged_seed_follows_the_merge() {
        let s = Store::open_in_memory().unwrap();
        let a = s.create_seed_topic("Acme", &[], None).unwrap();
        let b = s.create_seed_topic("Maximus", &[], None).unwrap();
        s.conn
            .execute(
                "INSERT INTO topic_verdict (source_key, topic_key, work, model, judged_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('email:e1', ?1, NULL, 'jev', 1, 1)",
                [&b.key],
            )
            .unwrap();
        s.merge_topics(a.topic_id, b.topic_id).unwrap();
        mail(&s, "e1", "Pilot notes");
        index(&s);
        let e1 = sid(&s, "Pilot notes");
        let origin: String = s
            .conn
            .query_row(
                "SELECT origin FROM topic_member WHERE topic_id = ?1 AND source_id = ?2",
                [a.topic_id, e1],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(origin, "jev");
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_judge_todo"), 0);
    }

    #[test]
    fn third_failure_drops_the_source_from_the_queue() {
        let s = Store::open_in_memory().unwrap();
        let id = source(&s, "email", 1);
        s.conn
            .execute("INSERT INTO topic_judge_todo (source_id) VALUES (?1)", [id])
            .unwrap();
        assert!(!s.record_topic_judge_failure(id).unwrap());
        assert!(!s.record_topic_judge_failure(id).unwrap());
        assert!(s.record_topic_judge_failure(id).unwrap());
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_judge_todo"), 0);
    }

    #[test]
    fn budget_is_unset_until_judging_starts() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.topic_judge_budget().unwrap(), None);
        s.start_topic_judging(900).unwrap();
        s.add_topic_judge_used_tokens(300).unwrap();
        assert_eq!(s.topic_judge_budget().unwrap(), Some((900, 300)));
        s.start_topic_judging(1200).unwrap();
        assert_eq!(s.topic_judge_budget().unwrap(), Some((1200, 0)));
    }

    /// Two verdicts with the same timestamp: every device keeps the same one.
    #[test]
    fn same_timestamp_verdict_tie_is_broken_the_same_way_everywhere() {
        let v = |topic: &str| super::TopicVerdictPayload {
            source_key: "email:e1".into(),
            topic_key: Some(topic.into()),
            work: None,
            model: "jev".into(),
            judged_at_unix_ms: 5,
            updated_at_unix_ms: 5,
        };
        let kept = |first: &str, second: &str| -> String {
            let s = Store::open_in_memory().unwrap();
            s.apply_synced_topic_verdict(&v(first)).unwrap();
            s.apply_synced_topic_verdict(&v(second)).unwrap();
            s.conn
                .query_row("SELECT topic_key FROM topic_verdict", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(kept("x", "y"), kept("y", "x"));
    }

    /// A newer pulled verdict for a source already judged here replaces its `'jev'` member;
    /// a `user` member stays.
    #[test]
    fn newer_pulled_verdict_replaces_the_jev_member() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("me");
        let x = s.create_seed_topic("Maximus", &[], None).unwrap();
        let y = s.create_seed_topic("Board", &[], None).unwrap();
        let z = s.create_seed_topic("Other", &[], None).unwrap();
        mail(&s, "e1", "Lunch plans");
        index(&s);
        let e1 = sid(&s, "Lunch plans");
        s.record_topic_verdict(e1, Some(x.topic_id), Some(0.9), "jev", 5)
            .unwrap();
        s.add_to_topic(z.topic_id, e1).unwrap();
        let jev = |s: &Store| -> Vec<i64> {
            let mut stmt = s
                .conn
                .prepare(
                    "SELECT topic_id FROM topic_member WHERE source_id = ?1 AND origin = 'jev'",
                )
                .unwrap();
            stmt.query_map([e1], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert_eq!(jev(&s), vec![x.topic_id]);
        let mut p = super::TopicVerdictPayload {
            source_key: "email:e1".into(),
            topic_key: Some(y.key.clone()),
            work: Some(0.9),
            model: "jev".into(),
            judged_at_unix_ms: 9,
            updated_at_unix_ms: i64::MAX - 1,
        };
        s.apply_synced_topic_verdict(&p).unwrap();
        assert_eq!(jev(&s), vec![y.topic_id]);
        // No topic and not work: no 'jev' member, noise; the user member stays.
        p.topic_key = None;
        p.work = Some(0.1);
        p.updated_at_unix_ms = i64::MAX;
        s.apply_synced_topic_verdict(&p).unwrap();
        assert!(jev(&s).is_empty());
        assert_eq!(
            count(&s, "SELECT noise FROM topic_source WHERE id IN (SELECT id FROM topic_source WHERE title = 'Lunch plans')"),
            1
        );
        assert_eq!(
            count(
                &s,
                &format!(
                    "SELECT COUNT(*) FROM topic_member WHERE topic_id = {} AND origin = 'user'",
                    z.topic_id
                )
            ),
            1
        );
    }
}
