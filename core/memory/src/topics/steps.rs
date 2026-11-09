//! Topic steps: the extraction queue (which topic items still need reading), recording a
//! model's line labels as steps and key dates, failure handling, and Mike's identity.

use chrono::{Local, NaiveDate, TimeZone};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::budget::{JudgeGate, TOPIC_STEPS};
use super::chrome::{self, Chrome};
use super::entities::parse_addresses;
use super::judge::{has_live_seed, source_key, sources_for_key, JudgeEstimate};
use super::page::KeyDate;
use super::step_text::{parse_due, step_candidates, step_hash};
use crate::redact::redact_secrets;
use crate::Store;

const DAY_MS: i64 = 86_400_000;
const FLOOR_DAYS: i64 = 30;
const MAX_ATTEMPTS: i64 = 3;
const CONTEXT_CHARS: usize = 2_000;
const EPISODE_CONTEXT_CHARS: usize = 1_500;
const FLOOR_KEY: &str = "topic_steps_floor_ms";
/// Per-call overhead of a TypeSafe step request, on top of the context at 4 chars a token.
// ponytail: unmeasured overhead; re-measure from usage_log after the first paid pass, as REQUEST_TOKENS was
pub const STEP_REQUEST_TOKENS: i64 = 1_200;
const NAMES_KEY: &str = "topic_self_names";
const NAMES_AT_KEY: &str = "topic_self_names_updated_at";
const PAGE_SYNC_BACKFILLED_KEY: &str = "topic_page_sync_backfilled";
/// Teams as each platform stores it in `episodes.app_bundle_id`: Mac bundle ids, then Windows.
/// Windows keeps a packaged app's AUMID (new Teams), and the exe name for classic Teams.
const TEAMS_APPS: [&str; 5] = [
    "com.microsoft.teams2",
    "com.microsoft.teams",
    "MSTeams_8wekyb3d8bbwe!MSTeams",
    "ms-teams.exe",
    "Teams.exe",
];

pub struct StepSource {
    pub source_id: i64,
    pub source_key: String,
    pub kind: String,
    pub title: String,
    pub asked_at_unix_ms: i64,
    pub topic_key: String,
    /// Redacted, capped (2,000 chars; episodes 1,500).
    pub context: String,
    /// Redacted candidate lines the model labels.
    pub candidates: Vec<String>,
    /// The topic's open mine/owed steps, newest 10: (key, text).
    pub open_steps: Vec<(String, String)>,
    /// Newest 5 dismissed steps on any topic: (text, reason).
    pub examples: Vec<(String, String)>,
    pub from: Option<String>,
}

pub struct LineLabel {
    /// "mine" | "owed" | None
    pub direction: Option<&'static str>,
    pub is_date: bool,
}

/// A step as it travels between devices (`topic_step` sync rows): every column but the
/// local-only `source_id`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopicStepPayload {
    pub key: String,
    pub topic_key: String,
    pub source_key: String,
    pub source_kind: String,
    pub source_title: String,
    pub text: String,
    pub direction: String,
    pub asked_at_unix_ms: i64,
    pub due_date: Option<String>,
    pub status: String,
    pub dismiss_reason: Option<String>,
    pub confirmed: bool,
    pub note: Option<String>,
    pub closed_by: Option<String>,
    pub origin: String,
    pub updated_at_unix_ms: i64,
}

/// A source's finished step read as it travels between devices (`topic_extraction` sync
/// rows), without the local-only `source_id`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopicExtractionPayload {
    pub source_key: String,
    pub topic_key: Option<String>,
    pub dates: Vec<KeyDate>,
    pub model: String,
    pub extracted_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

/// "Names I go by" as it travels between devices (`topic_names`, aggregate id `"self"`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopicNamesPayload {
    pub names: Vec<String>,
    pub updated_at_unix_ms: i64,
}

fn step_payload(conn: &Connection, key: &str) -> rusqlite::Result<Option<TopicStepPayload>> {
    conn.query_row(
        "SELECT key, topic_key, source_key, source_kind, source_title, text, direction, asked_at_unix_ms, \
         due_date, status, dismiss_reason, confirmed, note, closed_by, origin, updated_at_unix_ms \
         FROM topic_step WHERE key = ?1",
        [key],
        |r| {
            Ok(TopicStepPayload {
                key: r.get(0)?,
                topic_key: r.get(1)?,
                source_key: r.get(2)?,
                source_kind: r.get(3)?,
                source_title: r.get(4)?,
                text: r.get(5)?,
                direction: r.get(6)?,
                asked_at_unix_ms: r.get(7)?,
                due_date: r.get(8)?,
                status: r.get(9)?,
                dismiss_reason: r.get(10)?,
                confirmed: r.get(11)?,
                note: r.get(12)?,
                closed_by: r.get(13)?,
                origin: r.get(14)?,
                updated_at_unix_ms: r.get(15)?,
            })
        },
    )
    .optional()
}

fn extraction_payload(
    conn: &Connection,
    source_key: &str,
) -> rusqlite::Result<Option<TopicExtractionPayload>> {
    conn.query_row(
        "SELECT source_key, topic_key, dates, model, extracted_at_unix_ms, updated_at_unix_ms \
         FROM topic_extraction WHERE source_key = ?1",
        [source_key],
        |r| {
            let dates: String = r.get(2)?;
            Ok(TopicExtractionPayload {
                source_key: r.get(0)?,
                topic_key: r.get(1)?,
                dates: serde_json::from_str(&dates).unwrap_or_default(),
                model: r.get(3)?,
                extracted_at_unix_ms: r.get(4)?,
                updated_at_unix_ms: r.get(5)?,
            })
        },
    )
    .optional()
}

fn names_payload(conn: &Connection) -> rusqlite::Result<Option<TopicNamesPayload>> {
    let Some(at) = kv_get(conn, NAMES_AT_KEY)?.and_then(|v| v.parse().ok()) else {
        return Ok(None);
    };
    Ok(Some(TopicNamesPayload {
        names: kv_get(conn, NAMES_KEY)?
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default(),
        updated_at_unix_ms: at,
    }))
}

/// Queues the stored steps and extraction rows for sync, in the caller's transaction.
pub(crate) fn enqueue_step_rows(
    store: &Store,
    tx: &Transaction<'_>,
    step_keys: &[String],
    extraction_keys: &[String],
) -> rusqlite::Result<()> {
    for key in step_keys {
        if let Some(p) = step_payload(tx, key)? {
            store.enqueue_topic_page_outbox(
                tx,
                crate::OUTBOX_ROW_TYPE_TOPIC_STEP,
                p.key.clone(),
                &p,
            )?;
        }
    }
    for key in extraction_keys {
        if let Some(p) = extraction_payload(tx, key)? {
            store.enqueue_topic_page_outbox(
                tx,
                crate::OUTBOX_ROW_TYPE_TOPIC_EXTRACTION,
                p.source_key.clone(),
                &p,
            )?;
        }
    }
    Ok(())
}

/// `(origin device, meeting id)` of a meeting source key, `meeting:{device}:{id}`.
pub(crate) fn meeting_of_key(key: &str) -> Option<(&str, i64)> {
    let (device, id) = key.strip_prefix("meeting:")?.rsplit_once(':')?;
    Some((device, id.parse().ok()?))
}

/// Whether "Forget meeting" removed the meeting a pulled row's source key names: a tombstone
/// names it and no copy of it is here (as sync-engine's `forgotten_child`). This device's own
/// meeting matches only in this database's epoch.
fn forgotten_source(store: &Store, conn: &Connection, key: &str) -> rusqlite::Result<bool> {
    let Some((device, id)) = meeting_of_key(key) else {
        return Ok(false);
    };
    if store.own_device_id().as_deref() == Some(device) {
        let here: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM meeting WHERE id = ?1)",
            [id],
            |r| r.get(0),
        )?;
        return Ok(
            !here && store.is_meeting_forgotten(device, Some(&store.sync_entity_epoch()?), id)?
        );
    }
    let mapped: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_meeting_map WHERE peer_device_id = ?1 AND source_meeting_id = ?2)",
        params![device, id],
        |r| r.get(0),
    )?;
    Ok(!mapped && store.is_meeting_forgotten(device, None, id)?)
}

/// Deletes the steps and step read of the meeting `meeting:{device}:{id}`, and their unsent
/// outbox rows, in the caller's transaction (`forget_tx`).
pub(crate) fn delete_meeting_steps(
    tx: &Transaction<'_>,
    device: &str,
    id: i64,
) -> rusqlite::Result<()> {
    let key = format!("meeting:{device}:{id}");
    tx.execute("DELETE FROM topic_step WHERE source_key = ?1", [&key])?;
    tx.execute("DELETE FROM topic_extraction WHERE source_key = ?1", [&key])?;
    tx.execute(
        "DELETE FROM sync_outbox WHERE state IN ('pending', 'blocked') AND row_type IN (?2, ?3) \
         AND json_extract(payload_json, '$.source_key') = ?1",
        params![
            key,
            crate::OUTBOX_ROW_TYPE_TOPIC_STEP,
            crate::OUTBOX_ROW_TYPE_TOPIC_EXTRACTION
        ],
    )?;
    Ok(())
}

/// The local source a pulled row's `source_key` names, if it is here.
fn local_source(
    store: &Store,
    conn: &Connection,
    source_key: &str,
) -> rusqlite::Result<Option<i64>> {
    Ok(
        sources_for_key(conn, store.own_device_id().as_deref(), source_key)?
            .first()
            .copied(),
    )
}

/// Called when indexing writes source `sid`: steps and extraction rows pulled before it was
/// here get their local `source_id` (so they go when it goes), and a meeting's action items
/// take the done state of its pulled steps.
pub(crate) fn attach_source(
    conn: &Connection,
    own_device: Option<&str>,
    sid: i64,
) -> rusqlite::Result<()> {
    let Some(key) = source_key(conn, own_device, sid)? else {
        return Ok(());
    };
    let steps = conn.execute(
        "UPDATE topic_step SET source_id = ?1 WHERE source_key = ?2 AND source_id IS NULL",
        params![sid, key],
    )?;
    conn.execute(
        "UPDATE topic_extraction SET source_id = ?1 WHERE source_key = ?2 AND source_id IS NULL",
        params![sid, key],
    )?;
    if steps > 0 {
        if let Some(mid) = meeting_of_source(conn, sid)? {
            sync_meeting_item_state(conn, mid)?;
        }
    }
    Ok(())
}

/// What Mike can do to a step on the topic page.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum StepAction {
    Done,
    Reopen,
    /// `reason`: `not_mine` | `not_task`.
    Dismiss {
        reason: String,
    },
    Confirm {
        confirmed: bool,
    },
    Note {
        text: String,
    },
    Edit {
        text: String,
        due_date: Option<String>,
    },
}

/// The step hash of an action item: of the redacted, trimmed text, as the steps are keyed.
fn item_hash(text: &str) -> String {
    step_hash(&redact_secrets(text.trim()))
}

/// The meeting a `topic_source` row stands for.
fn meeting_of_source(conn: &Connection, source_id: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT ref_id FROM topic_source WHERE id = ?1 AND kind = 'meeting'",
        [source_id],
        |r| r.get(0),
    )
    .optional()
}

/// Ids of the meeting's local action items whose text hashes to `hash`.
fn items_with_hash(conn: &Connection, meeting_id: i64, hash: &str) -> rusqlite::Result<Vec<i64>> {
    let mut stmt =
        conn.prepare("SELECT id, text FROM meeting_action_item WHERE meeting_id = ?1")?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([meeting_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .into_iter()
        .filter(|(_, t)| item_hash(t) == hash)
        .map(|(id, _)| id)
        .collect())
}

/// Is the meeting source's action item for `line` (redacted, trimmed) checked off?
fn item_completed(conn: &Connection, source_id: i64, line: &str) -> rusqlite::Result<bool> {
    hash_completed(conn, source_id, &step_hash(line))
}

/// Is an action item of the meeting source whose text hashes to `hash` checked off?
pub(crate) fn hash_completed(
    conn: &Connection,
    source_id: i64,
    hash: &str,
) -> rusqlite::Result<bool> {
    let Some(mid) = meeting_of_source(conn, source_id)? else {
        return Ok(false);
    };
    for id in items_with_hash(conn, mid, hash)? {
        let done: bool = conn.query_row(
            "SELECT completed != 0 FROM meeting_action_item WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?;
        if done {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Keys of the meeting's steps whose key carries `hash`.
fn meeting_step_keys(
    conn: &Connection,
    meeting_id: i64,
    hash: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT key FROM topic_step WHERE source_id IN \
         (SELECT id FROM topic_source WHERE kind = 'meeting' AND ref_id = ?1) AND key LIKE '%#h' || ?2",
    )?;
    let rows = stmt
        .query_map(params![meeting_id, hash], |r| r.get(0))?
        .collect();
    rows
}

/// Moves the step for a checked (or unchecked) meeting action item: open -> done, or
/// done -> open. A dismissed step stays dismissed. Enqueues what changed.
pub(crate) fn sync_step_for_item(
    store: &Store,
    tx: &Transaction<'_>,
    meeting_id: i64,
    text: &str,
    completed: bool,
    now_ms: i64,
) -> rusqlite::Result<()> {
    let (from, to) = if completed {
        ("open", "done")
    } else {
        ("done", "open")
    };
    let mut changed = Vec::new();
    for key in meeting_step_keys(tx, meeting_id, &item_hash(text))? {
        let n = tx.execute(
            "UPDATE topic_step SET status = ?3, closed_by = NULL, \
             updated_at_unix_ms = MAX(?4, updated_at_unix_ms + 1) WHERE key = ?1 AND status = ?2",
            params![key, from, to, now_ms],
        )?;
        if n > 0 {
            changed.push(key);
        }
    }
    enqueue_step_rows(store, tx, &changed, &[])
}

/// For each local action item of the meeting: a `done` step with its hash checks it off.
pub(crate) fn sync_meeting_item_state(conn: &Connection, meeting_id: i64) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, text FROM meeting_action_item WHERE meeting_id = ?1 AND completed = 0",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([meeting_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, text) in rows {
        let done: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM topic_step WHERE status = 'done' AND source_id IN \
             (SELECT id FROM topic_source WHERE kind = 'meeting' AND ref_id = ?1) AND key LIKE '%#h' || ?2)",
            params![meeting_id, item_hash(&text)],
            |r| r.get(0),
        )?;
        if done {
            conn.execute(
                "UPDATE meeting_action_item SET completed = 1 WHERE id = ?1",
                [id],
            )?;
        }
    }
    Ok(())
}

/// The topic a source counts for: among its non-removed members on live seed topics, origin
/// user > seed > jev > auto, ties to the topic with more members, then the lower id.
pub(crate) fn primary_topic_for_source(
    conn: &Connection,
    source_id: i64,
) -> rusqlite::Result<Option<(i64, String)>> {
    conn.query_row(
        "SELECT m.topic_id, sd.key FROM topic_member m \
         JOIN topic_seed sd ON sd.topic_id = m.topic_id \
         JOIN topic t ON t.id = m.topic_id \
         WHERE m.source_id = ?1 AND m.origin != 'removed' AND sd.deleted = 0 \
           AND sd.merged_into_key IS NULL AND t.merged_into IS NULL \
         ORDER BY CASE m.origin WHEN 'user' THEN 0 WHEN 'seed' THEN 1 WHEN 'jev' THEN 2 ELSE 3 END, \
                  (SELECT COUNT(*) FROM topic_member x WHERE x.topic_id = m.topic_id AND x.origin != 'removed') DESC, \
                  m.topic_id LIMIT 1",
        [source_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

fn kv_get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM kv_meta WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

fn kv_put(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn write_names(conn: &Connection, p: &TopicNamesPayload) -> rusqlite::Result<()> {
    kv_put(
        conn,
        NAMES_KEY,
        &serde_json::to_string(&p.names).unwrap_or_default(),
    )?;
    kv_put(conn, NAMES_AT_KEY, &p.updated_at_unix_ms.to_string())
}

/// Estimated tokens for one step request over `context` (already capped).
pub fn step_request_tokens(context: &str) -> i64 {
    context.chars().count() as i64 / 4 + STEP_REQUEST_TOKENS
}

fn redacted_capped(text: &str, max: usize) -> String {
    redact_secrets(text).chars().take(max).collect()
}

fn local_date(ms: i64) -> NaiveDate {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.date_naive())
        .unwrap_or_default()
}

/// Hashes of every step on `topic_key` in any status, so a copied line is not stepped twice.
fn topic_step_hashes(
    conn: &Connection,
    topic_key: &str,
) -> rusqlite::Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare_cached("SELECT text FROM topic_step WHERE topic_key = ?1")?;
    let rows = stmt.query_map([topic_key], |r| r.get::<_, String>(0))?;
    rows.map(|t| t.map(|t| step_hash(&t))).collect()
}

fn meeting_key(source_key: &str, text: &str) -> String {
    format!("{source_key}#h{}", step_hash(text))
}

/// Action item texts of a meeting source that have no `topic_step` row yet.
fn unstepped_items(
    conn: &Connection,
    meeting_id: i64,
    source_key: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT text FROM meeting_action_item WHERE meeting_id = ?1 ORDER BY order_index, id",
    )?;
    let texts: Vec<String> = stmt
        .query_map([meeting_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for t in texts {
        let t = redact_secrets(t.trim()).into_owned();
        let key = meeting_key(source_key, &t);
        if t.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        let have: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM topic_step WHERE key = ?1)",
            [&key],
            |r| r.get(0),
        )?;
        if !have {
            out.push(t);
        }
    }
    Ok(out)
}

impl Store {
    /// Writes as it goes: sets the 30-day floor once, seeds meeting action items, and marks
    /// sources with no candidate lines read ("none" extraction rows).
    ///
    /// The step-reading work still to do, newest first: members of a live seed topic from the
    /// last 30 days (floor fixed at the first call) that were never read (emails, Teams
    /// screens) or have action items without a step (meetings).
    pub fn next_step_sources(&self, limit: usize) -> rusqlite::Result<Vec<StepSource>> {
        let conn = &self.conn;
        let floor = match kv_get(conn, FLOOR_KEY)?.and_then(|v| v.parse::<i64>().ok()) {
            Some(f) => f,
            None => {
                let f = crate::now_unix_ms() - FLOOR_DAYS * DAY_MS;
                kv_put(conn, FLOOR_KEY, &f.to_string())?;
                f
            }
        };
        let ids: Vec<i64> = {
            let mut stmt = conn.prepare(
                "SELECT s.id FROM topic_source s WHERE s.kind IN ('email', 'meeting', 'episode') \
                 AND s.ts_unix_ms >= ?1 AND s.ref_id IS NOT NULL \
                 AND EXISTS (SELECT 1 FROM topic_member m JOIN topic_seed sd ON sd.topic_id = m.topic_id \
                             WHERE m.source_id = s.id AND m.origin != 'removed' AND sd.deleted = 0) \
                 ORDER BY s.ts_unix_ms DESC, s.id DESC",
            )?;
            let rows = stmt
                .query_map([floor], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        let me = self.self_identity()?;
        let chrome = chrome::load_chrome(conn)?;
        let mut out = Vec::new();
        for sid in ids {
            if out.len() >= limit {
                break;
            }
            if let Some(src) = self.build_step_source(sid, &me, &chrome)? {
                out.push(src);
            }
        }
        Ok(out)
    }

    /// The source as the model should read it, or `None` when it is not (or no longer) queued.
    fn build_step_source(
        &self,
        sid: i64,
        me: &[String],
        chrome: &Chrome,
    ) -> rusqlite::Result<Option<StepSource>> {
        let conn = &self.conn;
        let Some((kind, ref_id, ts, title)): Option<(String, i64, i64, String)> = conn
            .query_row(
                "SELECT kind, ref_id, ts_unix_ms, title FROM topic_source WHERE id = ?1 AND ref_id IS NOT NULL",
                [sid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let Some((_, topic_key)) = primary_topic_for_source(conn, sid)? else {
            return Ok(None);
        };
        let Some(key) = source_key(conn, self.own_device_id().as_deref(), sid)? else {
            return Ok(None);
        };
        let read = kind != "meeting";
        if read {
            let done: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM topic_extraction WHERE source_key = ?1)",
                [&key],
                |r| r.get(0),
            )?;
            if done {
                return Ok(None);
            }
        }
        let (context, candidates, from) = match kind.as_str() {
            "email" => {
                let Some((actors, subject, body)): Option<(Option<String>, Option<String>, Option<String>)> = conn
                    .query_row(
                        "SELECT json_extract(actors, '$[0]'), title, body_text FROM items WHERE id = ?1",
                        [ref_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?
                else {
                    return Ok(None);
                };
                let text = format!(
                    "{}\n{}",
                    subject.unwrap_or_default(),
                    body.unwrap_or_default()
                );
                let text = redact_secrets(&text);
                (
                    text.chars().take(CONTEXT_CHARS).collect(),
                    step_candidates(&text, me),
                    actors.map(|a| redact_secrets(&a).into_owned()),
                )
            }
            "episode" => {
                let Some((device, app, win, lo, hi)): Option<(String, String, String, i64, i64)> = conn
                    .query_row(
                        "SELECT device_id, app_bundle_id, COALESCE(window_title, ''), start_item_id, end_item_id \
                         FROM episodes WHERE id = ?1",
                        [ref_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                    )
                    .optional()?
                else {
                    return Ok(None);
                };
                if !TEAMS_APPS.contains(&app.as_str()) {
                    return Ok(None);
                }
                let lines = chrome::capture_lines(conn, &device, &app, lo, hi, chrome)?;
                let text = redact_secrets(&lines.join("\n")).into_owned();
                (
                    redacted_capped(&format!("{win}\n{text}"), EPISODE_CONTEXT_CHARS),
                    step_candidates(&text, me),
                    None,
                )
            }
            _ => {
                let json: Option<String> = conn
                    .query_row(
                        "SELECT summary_action_items_json FROM meeting WHERE id = ?1",
                        [ref_id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                let generated: Vec<String> = json
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default();
                self.seed_meeting_action_items_from_generated(
                    ref_id,
                    &generated,
                    crate::now_unix_ms(),
                )?;
                let tldr: String = conn
                    .query_row(
                        "SELECT COALESCE(summary_tldr, '') FROM meeting WHERE id = ?1",
                        [ref_id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .unwrap_or_default();
                (
                    redacted_capped(&format!("{title}\n{tldr}"), CONTEXT_CHARS),
                    unstepped_items(conn, ref_id, &key)?,
                    None,
                )
            }
        };
        let candidates = if read {
            let have = topic_step_hashes(conn, &topic_key)?;
            candidates
                .into_iter()
                .filter(|c| !have.contains(&step_hash(c)))
                .collect()
        } else {
            candidates
        };
        if candidates.is_empty() {
            // Nothing to label: an email with no candidate lines is read for free.
            if read {
                let tx = conn.unchecked_transaction()?;
                self.write_extraction(
                    &tx,
                    sid,
                    &key,
                    &topic_key,
                    "[]",
                    "none",
                    crate::now_unix_ms(),
                )?;
                tx.commit()?;
            }
            return Ok(None);
        }
        Ok(Some(StepSource {
            source_id: sid,
            source_key: key,
            kind,
            title,
            asked_at_unix_ms: ts,
            open_steps: self.query_pairs(
                "SELECT key, text FROM topic_step WHERE topic_key = ?1 AND status = 'open' \
                 AND direction IN ('mine', 'owed') ORDER BY asked_at_unix_ms DESC LIMIT 10",
                Some(&topic_key),
            )?,
            examples: self.query_pairs(
                "SELECT text, COALESCE(dismiss_reason, '') FROM topic_step WHERE status = 'dismissed' \
                 ORDER BY updated_at_unix_ms DESC LIMIT 5",
                None,
            )?,
            topic_key,
            context,
            candidates,
            from,
        }))
    }

    fn query_pairs(&self, sql: &str, arg: Option<&str>) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(sql)?;
        // Redacted: Mike may have typed a secret into a step or a dismiss reason.
        let map = |r: &rusqlite::Row| {
            let (a, b): (String, String) = (r.get(0)?, r.get(1)?);
            Ok((
                redact_secrets(&a).into_owned(),
                redact_secrets(&b).into_owned(),
            ))
        };
        let rows = match arg {
            Some(a) => stmt.query_map([a], map)?.collect(),
            None => stmt.query_map([], map)?.collect(),
        };
        rows
    }

    #[allow(clippy::too_many_arguments)]
    fn write_extraction(
        &self,
        tx: &Transaction<'_>,
        source_id: i64,
        source_key: &str,
        topic_key: &str,
        dates: &str,
        model: &str,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        tx.execute(
            "INSERT INTO topic_extraction (source_key, topic_key, dates, model, extracted_at_unix_ms, updated_at_unix_ms, source_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6) ON CONFLICT(source_key) DO UPDATE SET \
             topic_key = excluded.topic_key, dates = excluded.dates, model = excluded.model, \
             extracted_at_unix_ms = excluded.extracted_at_unix_ms, updated_at_unix_ms = excluded.updated_at_unix_ms",
            params![source_key, topic_key, dates, model, now_ms, source_id],
        )?;
        enqueue_step_rows(self, tx, &[], &[source_key.to_string()])
    }

    /// Stores the model's labels for `src.candidates` (same order) as steps and key dates,
    /// marks `closes` (open step keys) done, and finishes the source's read.
    pub fn record_step_extraction(
        &self,
        src: &StepSource,
        labels: &[LineLabel],
        closes: &[String],
        model: &str,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        if labels.len() != src.candidates.len() {
            return Err(rusqlite::Error::InvalidParameterCount(
                labels.len(),
                src.candidates.len(),
            ));
        }
        let tx = self.conn.unchecked_transaction()?;
        let meeting = src.kind == "meeting";
        let anchor = local_date(src.asked_at_unix_ms);
        let (mut keys, mut dates) = (Vec::new(), Vec::new());
        // Two copies of one line can arrive in one batch; only the first becomes a step.
        let mut have = if meeting {
            Default::default()
        } else {
            topic_step_hashes(&tx, &src.topic_key)?
        };
        for (i, (line, label)) in src.candidates.iter().zip(labels).enumerate() {
            let due = parse_due(line, anchor);
            if label.is_date {
                if let Some(d) = due {
                    dates.push(serde_json::json!({"date": d.to_string(), "label": line}));
                }
            }
            let direction = match (label.direction, meeting) {
                (Some(d), _) => d,
                (None, true) => "none",
                (None, false) => continue,
            };
            if !meeting && !have.insert(step_hash(line)) {
                continue;
            }
            let key = if meeting {
                meeting_key(&src.source_key, line)
            } else {
                format!("{}#{i}", src.source_key)
            };
            let due = (direction != "none")
                .then_some(due)
                .flatten()
                .map(|d| d.to_string());
            let status = if meeting && item_completed(&tx, src.source_id, line)? {
                "done"
            } else {
                "open"
            };
            let n = tx.execute(
                "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, \
                 asked_at_unix_ms, due_date, updated_at_unix_ms, source_id, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT(key) DO NOTHING",
                params![
                    key, src.topic_key, src.source_key, src.kind, src.title, line, direction,
                    src.asked_at_unix_ms, due, now_ms, src.source_id, status
                ],
            )?;
            if n > 0 {
                keys.push(key);
            }
        }
        let closed_by = format!("{} {}: {}", src.kind, anchor, src.title);
        for key in closes {
            let n = tx.execute(
                "UPDATE topic_step SET status = 'done', closed_by = ?2, updated_at_unix_ms = ?3 \
                 WHERE key = ?1 AND status = 'open'",
                params![key, closed_by, now_ms],
            )?;
            if n > 0 {
                keys.push(key.clone());
            }
        }
        if !meeting {
            let json = serde_json::Value::Array(dates).to_string();
            self.write_extraction(
                &tx,
                src.source_id,
                &src.source_key,
                &src.topic_key,
                &json,
                model,
                now_ms,
            )?;
        }
        tx.execute(
            "DELETE FROM topic_step_attempt WHERE source_id = ?1",
            [src.source_id],
        )?;
        enqueue_step_rows(self, &tx, &keys, &[])?;
        tx.commit()
    }

    /// Applies Mike's action to a step; false for an unknown key. Done and Reopen on a
    /// meeting step also check or uncheck the meeting's matching action items.
    pub fn apply_step_action(&self, key: &str, action: &StepAction) -> rusqlite::Result<bool> {
        let conn = &self.conn;
        let tx = conn.unchecked_transaction()?;
        let now_ms = crate::now_unix_ms();
        let (sql, a, b): (&str, String, Option<String>) = match action {
            StepAction::Done => ("status = 'done'", String::new(), None),
            StepAction::Reopen => (
                "status = 'open', closed_by = NULL, dismiss_reason = NULL",
                String::new(),
                None,
            ),
            StepAction::Dismiss { reason } => (
                "status = 'dismissed', dismiss_reason = ?3",
                reason.clone(),
                None,
            ),
            StepAction::Confirm { confirmed } => ("confirmed = ?3", (*confirmed as i64).to_string(), None),
            StepAction::Note { text } => ("note = ?3", text.clone(), None),
            StepAction::Edit { text, due_date } => (
                "text = ?3, due_date = ?4, origin = 'user'",
                text.clone(),
                due_date.clone(),
            ),
        };
        let q = format!("UPDATE topic_step SET {sql}, updated_at_unix_ms = MAX(?2, updated_at_unix_ms + 1) WHERE key = ?1");
        let n = match action {
            StepAction::Confirm { confirmed } => tx.execute(&q, params![key, now_ms, *confirmed as i64])?,
            StepAction::Edit { .. } => tx.execute(&q, params![key, now_ms, a, b])?,
            StepAction::Dismiss { .. } | StepAction::Note { .. } => {
                tx.execute(&q, params![key, now_ms, a])?
            }
            _ => tx.execute(&q, params![key, now_ms])?,
        };
        if n == 0 {
            return Ok(false);
        }
        if matches!(action, StepAction::Done | StepAction::Reopen) {
            let sid: Option<i64> = tx.query_row(
                "SELECT source_id FROM topic_step WHERE key = ?1",
                [key],
                |r| r.get(0),
            )?;
            let mid = match sid {
                Some(sid) => meeting_of_source(&tx, sid)?,
                None => None,
            };
            if let (Some(mid), Some(hash)) = (mid, key.rsplit_once("#h").map(|(_, h)| h)) {
                for id in items_with_hash(&tx, mid, hash)? {
                    tx.execute(
                        "UPDATE meeting_action_item SET completed = ?2, updated_unix_ms = ?3 WHERE id = ?1",
                        params![id, matches!(action, StepAction::Done) as i64, now_ms],
                    )?;
                }
            }
        }
        enqueue_step_rows(self, &tx, &[key.to_string()], &[])?;
        tx.commit()?;
        Ok(true)
    }

    /// Counts a failed read. The third failure records the source as read with nothing found
    /// (model `"failed"`) so it leaves the queue; returns whether it did.
    pub fn record_step_failure(&self, source_id: i64) -> rusqlite::Result<bool> {
        self.conn.execute(
            "INSERT INTO topic_step_attempt (source_id, attempts) VALUES (?1, 1) \
             ON CONFLICT(source_id) DO UPDATE SET attempts = attempts + 1",
            [source_id],
        )?;
        let attempts: i64 = self.conn.query_row(
            "SELECT attempts FROM topic_step_attempt WHERE source_id = ?1",
            [source_id],
            |r| r.get(0),
        )?;
        if attempts < MAX_ATTEMPTS {
            return Ok(false);
        }
        let chrome = chrome::load_chrome(&self.conn)?;
        if let Some(src) = self.build_step_source(source_id, &self.self_identity()?, &chrome)? {
            let none: Vec<LineLabel> = src
                .candidates
                .iter()
                .map(|_| LineLabel {
                    direction: None,
                    is_date: false,
                })
                .collect();
            self.record_step_extraction(&src, &none, &[], "failed", crate::now_unix_ms())?;
        }
        Ok(true)
    }

    /// What reading the current step queue with TypeSafe would cost: per source, its capped
    /// context at 4 characters a token plus `STEP_REQUEST_TOKENS`. Walks the queue itself,
    /// so it sets the 30-day floor and marks candidate-less sources read as the queue does.
    pub fn topic_steps_estimate(&self, usd_per_million: f64) -> rusqlite::Result<JudgeEstimate> {
        let srcs = self.next_step_sources(usize::MAX)?;
        let tokens: i64 = srcs.iter().map(|s| step_request_tokens(&s.context)).sum();
        Ok(JudgeEstimate {
            sources: srcs.len() as i64,
            tokens,
            usd: tokens as f64 * usd_per_million / 1e6,
            indexing: kv_get(&self.conn, super::BATCH_MORE_KEY)?.as_deref() != Some("0")
                || !has_live_seed(&self.conn)?,
        })
    }

    /// Starts a paid first pass of up to `budget_tokens` (the caller passes 1.5 × the
    /// estimate), then `DAILY_ALLOWANCE_TOKENS` a day. Separate from the judge budget.
    pub fn start_topic_steps_judging(&self, budget_tokens: i64) -> rusqlite::Result<()> {
        TOPIC_STEPS.start(&self.conn, budget_tokens)
    }

    pub fn topic_steps_gate(&self) -> rusqlite::Result<JudgeGate> {
        TOPIC_STEPS.gate(&self.conn, crate::now_unix_ms())
    }

    pub fn add_topic_steps_used_tokens(&self, tokens: i64) -> rusqlite::Result<()> {
        TOPIC_STEPS.add_used(&self.conn, tokens, crate::now_unix_ms())
    }

    pub fn topic_steps_cost_cents(
        &self,
        tokens: i64,
        usd_per_million: f64,
    ) -> rusqlite::Result<i64> {
        TOPIC_STEPS.cost_cents(&self.conn, tokens, usd_per_million)
    }

    /// `(budget, used)` tokens, or `None` before `start_topic_steps_judging`.
    pub fn topic_steps_budget(&self) -> rusqlite::Result<Option<(i64, i64)>> {
        TOPIC_STEPS.budget(&self.conn)
    }

    /// Ends the paid first pass once it has started and the step queue is empty.
    pub fn end_topic_steps_first_pass_if_drained(&self) -> rusqlite::Result<()> {
        if TOPIC_STEPS.budget(&self.conn)?.is_some()
            && !TOPIC_STEPS.first_pass_done(&self.conn)?
            && self.next_step_sources(1)?.is_empty()
        {
            TOPIC_STEPS.mark_first_pass_done(&self.conn)?;
        }
        Ok(())
    }

    /// Names set in "Names I go by" plus every connected account address.
    pub fn self_identity(&self) -> rusqlite::Result<Vec<String>> {
        let mut out = self.self_names()?;
        let mut stmt = self
            .conn
            .prepare("SELECT email FROM connector_account ORDER BY position, email")?;
        let emails: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for e in emails {
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&e)) {
                out.push(e);
            }
        }
        Ok(out)
    }

    pub fn set_self_names(&self, names: &[String]) -> rusqlite::Result<()> {
        let mut clean: Vec<String> = Vec::new();
        for n in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
            if !clean.iter().any(|c| c.eq_ignore_ascii_case(n)) {
                clean.push(n.to_string());
            }
        }
        let tx = self.conn.unchecked_transaction()?;
        // Later than any copy stored here, pulled ones included, so this edit wins.
        let at = names_payload(&tx)?.map_or(0, |p| p.updated_at_unix_ms + 1);
        let p = TopicNamesPayload {
            names: clean,
            updated_at_unix_ms: crate::now_unix_ms().max(at),
        };
        write_names(&tx, &p)?;
        self.enqueue_topic_page_outbox(&tx, crate::OUTBOX_ROW_TYPE_TOPIC_NAMES, "self".into(), &p)?;
        tx.commit()
    }

    /// Applies "Names I go by" pulled from another device: last write wins (`incoming_wins`).
    /// Never enqueued.
    pub fn apply_synced_topic_names(&self, p: &TopicNamesPayload) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let wins = match names_payload(&tx)? {
            Some(l) => super::incoming_wins(p, p.updated_at_unix_ms, &l, l.updated_at_unix_ms),
            None => true,
        };
        if wins {
            write_names(&tx, p)?;
        }
        tx.commit()
    }

    /// Applies a step pulled from another device: last write wins (`incoming_wins`). Its local
    /// `source_id` is the source here with its key (or none until indexing brings one). A done
    /// meeting step checks off the meeting's matching action items here. Never enqueued.
    pub fn apply_synced_topic_step(&self, p: &TopicStepPayload) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        if forgotten_source(self, &tx, &p.source_key)? {
            return Ok(());
        }
        let wins = match step_payload(&tx, &p.key)? {
            Some(l) => super::incoming_wins(p, p.updated_at_unix_ms, &l, l.updated_at_unix_ms),
            None => true,
        };
        if wins {
            let sid = local_source(self, &tx, &p.source_key)?;
            tx.execute(
                "INSERT OR REPLACE INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, \
                 direction, asked_at_unix_ms, due_date, status, dismiss_reason, confirmed, note, closed_by, origin, \
                 updated_at_unix_ms, source_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    p.key, p.topic_key, p.source_key, p.source_kind, p.source_title, p.text, p.direction,
                    p.asked_at_unix_ms, p.due_date, p.status, p.dismiss_reason, p.confirmed, p.note,
                    p.closed_by, p.origin, p.updated_at_unix_ms, sid
                ],
            )?;
            if p.status == "done" {
                if let Some(mid) = sid
                    .map(|s| meeting_of_source(&tx, s))
                    .transpose()?
                    .flatten()
                {
                    sync_meeting_item_state(&tx, mid)?;
                }
            }
        }
        tx.commit()
    }

    /// Applies a source's step read pulled from another device (so this device never reads it
    /// again): last write wins (`incoming_wins`), `source_id` as for steps. Never enqueued.
    pub fn apply_synced_topic_extraction(
        &self,
        p: &TopicExtractionPayload,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        if forgotten_source(self, &tx, &p.source_key)? {
            return Ok(());
        }
        let wins = match extraction_payload(&tx, &p.source_key)? {
            Some(l) => super::incoming_wins(p, p.updated_at_unix_ms, &l, l.updated_at_unix_ms),
            None => true,
        };
        if wins {
            let sid = local_source(self, &tx, &p.source_key)?;
            tx.execute(
                "INSERT OR REPLACE INTO topic_extraction (source_key, topic_key, dates, model, extracted_at_unix_ms, \
                 updated_at_unix_ms, source_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    p.source_key,
                    p.topic_key,
                    serde_json::to_string(&p.dates).unwrap_or_else(|_| "[]".into()),
                    p.model,
                    p.extracted_at_unix_ms,
                    p.updated_at_unix_ms,
                    sid
                ],
            )?;
        }
        tx.commit()
    }

    /// Queues every step, extraction, role edit and "Names I go by" for sync, once per database
    /// (kv `topic_page_sync_backfilled`): rows written before this device had a device id, or
    /// before topic page sync existed. `push_content` calls it once every peer pushes
    /// `topic_page_capability`. Without a device id nothing is queued and nothing is recorded.
    pub fn enqueue_topic_page_sync_backfill(&self) -> rusqlite::Result<()> {
        if self.own_device_id().is_none() || self.topic_page_sync_backfilled()? {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        let keys = |sql: &str| -> rusqlite::Result<Vec<String>> {
            let mut stmt = tx.prepare(sql)?;
            let rows = stmt.query_map([], |r| r.get(0))?.collect();
            rows
        };
        let steps = keys("SELECT key FROM topic_step ORDER BY key")?;
        let extractions = keys("SELECT source_key FROM topic_extraction ORDER BY source_key")?;
        let roles = keys("SELECT topic_key FROM topic_role ORDER BY topic_key")?;
        enqueue_step_rows(self, &tx, &steps, &extractions)?;
        for key in roles {
            super::page::enqueue_role_row(self, &tx, &key)?;
        }
        if let Some(p) = names_payload(&tx)? {
            self.enqueue_topic_page_outbox(
                &tx,
                crate::OUTBOX_ROW_TYPE_TOPIC_NAMES,
                "self".into(),
                &p,
            )?;
        }
        kv_put(&tx, PAGE_SYNC_BACKFILLED_KEY, "1")?;
        tx.commit()
    }

    /// Whether `enqueue_topic_page_sync_backfill` has run on this database.
    pub fn topic_page_sync_backfilled(&self) -> rusqlite::Result<bool> {
        Ok(kv_get(&self.conn, PAGE_SYNC_BACKFILLED_KEY)?.is_some())
    }

    pub fn self_names(&self) -> rusqlite::Result<Vec<String>> {
        Ok(kv_get(&self.conn, NAMES_KEY)?
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default())
    }

    /// Display names Mike's accounts send as, to prefill "Names I go by".
    pub fn suggested_self_names(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT json_extract(actors, '$[0]') FROM items WHERE kind = 'email' \
             ORDER BY ts_unix_ms DESC LIMIT 500",
        )?;
        let froms: Vec<Option<String>> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt = self
            .conn
            .prepare("SELECT lower(email) FROM connector_account")?;
        let accounts: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out: Vec<String> = Vec::new();
        for a in froms
            .into_iter()
            .flatten()
            .filter_map(|f| parse_addresses(&f).into_iter().next())
        {
            if let (Some(name), true) = (a.name, accounts.contains(&a.email)) {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(&name)) {
                    out.push(name);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRI_NOV_6_NOON_UTC: i64 = 1_793_966_400_000; // 2026-11-06 12:00 UTC

    fn now() -> i64 {
        crate::now_unix_ms()
    }

    fn seed(s: &Store) -> (i64, String) {
        let t = s.create_seed_topic("Maximus", &[], None).unwrap();
        (t.topic_id, t.key)
    }

    /// An email item plus its topic_source, filed on `topic` with `origin`.
    fn mail(s: &Store, ext: &str, ts: i64, body: &str, topic: Option<(i64, &str)>) -> i64 {
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                 VALUES ('d','email','gmail',?1,json_array('Pat <pat@x.com>'),'SOW',?2,'{}',?3)",
                params![ts, body, ext],
            )
            .unwrap();
        let item = s.conn.last_insert_rowid();
        filed(s, "email", item, ts, "SOW", topic)
    }

    fn filed(
        s: &Store,
        kind: &str,
        ref_id: i64,
        ts: i64,
        title: &str,
        topic: Option<(i64, &str)>,
    ) -> i64 {
        s.conn
            .execute(
                "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, ?3, ?4, 0)",
                params![kind, ref_id, ts, title],
            )
            .unwrap();
        let sid = s.conn.last_insert_rowid();
        if let Some((t, origin)) = topic {
            s.conn
                .execute(
                    "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, ?3)",
                    params![t, sid, origin],
                )
                .unwrap();
        }
        sid
    }

    fn episode(s: &Store, app: &str, ts: i64, topic: i64) -> i64 {
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, body_text) VALUES ('d','capture','screen',?1,'Please send the revised budget to Priya by Friday afternoon')",
                [ts],
            )
            .unwrap();
        let cap = s.conn.last_insert_rowid();
        s.conn
            .execute(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('d', ?1, 'Chat', ?2, ?2, ?3, ?3, 1, 0, 0)",
                params![app, ts, cap],
            )
            .unwrap();
        let ep = s.conn.last_insert_rowid();
        filed(s, "episode", ep, ts, "Chat", Some((topic, "seed")))
    }

    fn mine() -> LineLabel {
        LineLabel {
            direction: Some("mine"),
            is_date: false,
        }
    }

    #[test]
    fn only_seed_topic_items_from_the_last_30_days_are_queued() {
        let s = Store::open_in_memory().unwrap();
        let (t, key) = seed(&s);
        let auto = s.create_seed_topic("Other", &[], None).unwrap();
        s.conn
            .execute(
                "DELETE FROM topic_seed WHERE topic_id = ?1",
                [auto.topic_id],
            )
            .unwrap();
        let body = "Please send the SOW by Friday.";
        mail(&s, "new", now() - 10 * DAY_MS, body, Some((t, "seed")));
        mail(&s, "old", now() - 40 * DAY_MS, body, Some((t, "seed")));
        mail(
            &s,
            "auto",
            now() - DAY_MS,
            body,
            Some((auto.topic_id, "auto")),
        );
        let q = s.next_step_sources(10).unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].source_key, "email:new");
        assert_eq!(q[0].topic_key, key);
    }

    #[test]
    fn non_teams_episode_is_not_queued() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        let teams = episode(&s, "com.microsoft.teams2", now() - DAY_MS, t);
        episode(&s, "com.salesforce", now() - DAY_MS, t);
        let q = s.next_step_sources(10).unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].source_id, teams);
    }

    #[test]
    fn windows_teams_episodes_are_queued_by_their_app_ids() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        let new_teams = episode(&s, "MSTeams_8wekyb3d8bbwe!MSTeams", now() - DAY_MS, t);
        let classic = episode(&s, "Teams.exe", now() - DAY_MS, t);
        episode(&s, "OpenAI.Codex_2p2nqsd0c76g0!App", now() - DAY_MS, t);
        let mut ids: Vec<i64> = s
            .next_step_sources(10)
            .unwrap()
            .iter()
            .map(|q| q.source_id)
            .collect();
        ids.sort();
        let mut want = vec![new_teams, classic];
        want.sort();
        assert_eq!(ids, want);
    }

    #[test]
    fn recorded_source_is_never_queued_again() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        mail(
            &s,
            "x",
            now() - DAY_MS,
            "Please send the SOW by Friday.",
            Some((t, "seed")),
        );
        let src = s.next_step_sources(10).unwrap().remove(0);
        s.record_step_extraction(&src, &[mine()], &[], "m", now())
            .unwrap();
        assert!(s.next_step_sources(10).unwrap().is_empty());
    }

    #[test]
    fn refile_to_another_topic_does_not_requeue() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        let other = s.create_seed_topic("Beta", &[], None).unwrap();
        let sid = mail(
            &s,
            "x",
            now() - DAY_MS,
            "Please send the SOW by Friday.",
            Some((t, "seed")),
        );
        let src = s.next_step_sources(10).unwrap().remove(0);
        s.record_step_extraction(&src, &[mine()], &[], "m", now())
            .unwrap();
        s.conn
            .execute(
                "UPDATE topic_member SET origin = 'removed' WHERE source_id = ?1",
                [sid],
            )
            .unwrap();
        s.conn
            .execute(
                "INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'user')",
                params![other.topic_id, sid],
            )
            .unwrap();
        assert!(s.next_step_sources(10).unwrap().is_empty());
        assert_eq!(
            primary_topic_for_source(&s.conn, sid).unwrap().unwrap().1,
            other.key
        );
    }

    #[test]
    fn mine_and_owed_lines_become_steps_with_due_dates() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        mail(&s, "x", FRI_NOV_6_NOON_UTC, "Please send the SOW Friday.\nPriya will confirm the budget soon.\nHow was the trip last week?", Some((t, "seed")));
        let src = s.next_step_sources(10).unwrap().remove(0);
        assert_eq!(src.candidates.len(), 3, "{:?}", src.candidates);
        let owed = LineLabel {
            direction: Some("owed"),
            is_date: false,
        };
        let none = LineLabel {
            direction: None,
            is_date: false,
        };
        s.record_step_extraction(&src, &[mine(), owed, none], &[], "m", now())
            .unwrap();
        let rows: Vec<(String, String, Option<String>)> = s
            .conn
            .prepare("SELECT key, direction, due_date FROM topic_step ORDER BY key")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            ("email:x#0".into(), "mine".into(), Some("2026-11-13".into()))
        );
        assert_eq!(rows[1].0, "email:x#1");
        assert_eq!(rows[1].1, "owed");
    }

    #[test]
    fn close_marks_done_with_closed_by() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        mail(
            &s,
            "a",
            FRI_NOV_6_NOON_UTC,
            "Please send the SOW Friday.",
            Some((t, "seed")),
        );
        let a = s.next_step_sources(10).unwrap().remove(0);
        s.record_step_extraction(&a, &[mine()], &[], "m", now())
            .unwrap();
        mail(
            &s,
            "b",
            FRI_NOV_6_NOON_UTC + 3 * DAY_MS,
            "Sent the SOW over, can you confirm?",
            Some((t, "seed")),
        );
        let b = s.next_step_sources(10).unwrap().remove(0);
        assert_eq!(b.open_steps.len(), 1);
        s.record_step_extraction(
            &b,
            &[LineLabel {
                direction: None,
                is_date: false,
            }],
            &["email:a#0".into()],
            "m",
            now(),
        )
        .unwrap();
        let (status, by): (String, String) = s
            .conn
            .query_row(
                "SELECT status, closed_by FROM topic_step WHERE key = 'email:a#0'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "done");
        assert_eq!(by, "email 2026-11-09: SOW");
    }

    #[test]
    fn third_failure_leaves_the_queue() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        mail(
            &s,
            "x",
            now() - DAY_MS,
            "Please send the SOW by Friday.",
            Some((t, "seed")),
        );
        let id = s.next_step_sources(10).unwrap()[0].source_id;
        assert!(!s.record_step_failure(id).unwrap());
        assert!(!s.record_step_failure(id).unwrap());
        assert_eq!(s.next_step_sources(10).unwrap().len(), 1);
        assert!(s.record_step_failure(id).unwrap());
        assert!(s.next_step_sources(10).unwrap().is_empty());
        let model: String = s
            .conn
            .query_row(
                "SELECT model FROM topic_extraction WHERE source_key = 'email:x'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(model, "failed");
    }

    #[test]
    fn identity_includes_account_emails_and_names() {
        let s = Store::open_in_memory().unwrap();
        s.insert_connector_account("gmail", "mike@acme.com", true, 0, 0)
            .unwrap();
        s.set_self_names(&["Mike P".into(), " ".into(), "mike p".into()])
            .unwrap();
        assert_eq!(s.self_names().unwrap(), vec!["Mike P"]);
        assert_eq!(s.self_identity().unwrap(), vec!["Mike P", "mike@acme.com"]);
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                 VALUES ('d','email','gmail',1,json_array('\"Michael Pointer\" <MIKE@acme.com>'),'t','b','{}','e1')",
                [],
            )
            .unwrap();
        assert_eq!(s.suggested_self_names().unwrap(), vec!["Michael Pointer"]);
    }

    #[test]
    fn meeting_action_items_queue_until_stepped() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("me");
        let (t, _) = seed(&s);
        s.conn
            .execute(
                "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status, summary_action_items_json) \n                 VALUES ('me', ?1, 'Sync', '[]', 'manual', 'done', '[\"Send the SOW.\",\"Book a room\"]')",
                [now() - DAY_MS],
            )
            .unwrap();
        let mid = s.conn.last_insert_rowid();
        filed(
            &s,
            "meeting",
            mid,
            now() - DAY_MS,
            "Sync",
            Some((t, "seed")),
        );
        let src = s.next_step_sources(10).unwrap().remove(0);
        assert_eq!(src.candidates, vec!["Send the SOW.", "Book a room"]);
        let none = LineLabel {
            direction: None,
            is_date: false,
        };
        s.record_step_extraction(&src, &[mine(), none], &[], "m", now())
            .unwrap();
        assert!(s.next_step_sources(10).unwrap().is_empty());
        let n: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM topic_step WHERE direction = 'none'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn mismatched_label_count_writes_nothing() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        mail(
            &s,
            "x",
            now() - DAY_MS,
            "Please send the SOW by Friday.",
            Some((t, "seed")),
        );
        let src = s.next_step_sources(10).unwrap().remove(0);
        assert!(s
            .record_step_extraction(&src, &[], &[], "m", now())
            .is_err());
        assert_eq!(s.next_step_sources(10).unwrap().len(), 1);
        let n: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM topic_extraction", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    fn read_one(s: &Store, ext: &str) -> i64 {
        let (t, _) = seed(s);
        let sid = mail(
            s,
            ext,
            now() - DAY_MS,
            "Please send the SOW by Friday.",
            Some((t, "seed")),
        );
        let src = s.next_step_sources(10).unwrap().remove(0);
        s.record_step_extraction(&src, &[mine()], &[], "m", now())
            .unwrap();
        s.record_step_failure(sid).unwrap();
        sid
    }

    fn leftovers(s: &Store) -> i64 {
        ["topic_step", "topic_extraction", "topic_step_attempt"]
            .iter()
            .map(|t| {
                s.conn
                    .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap()
            })
            .sum()
    }

    #[test]
    fn deleting_a_source_removes_its_steps_and_extraction() {
        let s = Store::open_in_memory().unwrap();
        read_one(&s, "x");
        assert!(leftovers(&s) >= 3);
        s.conn
            .execute("DELETE FROM items WHERE external_id = 'x'", [])
            .unwrap();
        assert_eq!(leftovers(&s), 0);
    }

    #[test]
    fn retention_stub_removes_steps_and_extraction() {
        let s = Store::open_in_memory().unwrap();
        let sid = read_one(&s, "x");
        let r: i64 = s
            .conn
            .query_row(
                "SELECT ref_id FROM topic_source WHERE id = ?1",
                [sid],
                |r| r.get(0),
            )
            .unwrap();
        crate::topics::schema::stub_sources_for_retention(&s.conn, "email", &[r]).unwrap();
        assert_eq!(leftovers(&s), 0);
        let stub: i64 = s
            .conn
            .query_row("SELECT stub FROM topic_source WHERE id = ?1", [sid], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stub, 1);
    }

    #[test]
    fn context_is_redacted() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH";
        mail(
            &s,
            "x",
            now() - DAY_MS,
            &format!("Please use key {secret} for the SOW by Friday."),
            Some((t, "seed")),
        );
        let src = s.next_step_sources(10).unwrap().remove(0);
        assert!(!src.context.contains(secret));
        assert!(!src.candidates.iter().any(|c| c.contains(secret)));
        assert!(!src.candidates.is_empty());
    }

    fn step_count(s: &Store) -> i64 {
        s.conn
            .query_row("SELECT COUNT(*) FROM topic_step", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn same_email_under_two_ids_makes_one_step() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        let body = "Please send the SOW by Friday.";
        mail(&s, "a", now() - 2 * DAY_MS, body, Some((t, "seed")));
        mail(&s, "b", now() - DAY_MS, body, Some((t, "seed")));
        // Both copies in one batch: the second insert is skipped.
        for src in s.next_step_sources(10).unwrap() {
            s.record_step_extraction(&src, &[mine()], &[], "m", now())
                .unwrap();
        }
        assert_eq!(step_count(&s), 1);
        // A third copy read later has no candidates left and is read for free.
        mail(&s, "c", now(), body, Some((t, "seed")));
        assert!(s.next_step_sources(10).unwrap().is_empty());
        assert_eq!(step_count(&s), 1);
    }

    #[test]
    fn same_teams_line_in_two_episodes_makes_one_step() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        episode(&s, "com.microsoft.teams2", now() - 2 * DAY_MS, t);
        let first = s.next_step_sources(10).unwrap().remove(0);
        s.record_step_extraction(&first, &[mine()], &[], "m", now())
            .unwrap();
        episode(&s, "com.microsoft.teams2", now() - DAY_MS, t);
        assert!(s.next_step_sources(10).unwrap().is_empty());
        assert_eq!(step_count(&s), 1);
    }

    #[test]
    fn open_steps_and_examples_are_redacted() {
        let s = Store::open_in_memory().unwrap();
        let (t, _) = seed(&s);
        let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH";
        for ext in ["a", "b"] {
            mail(
                &s,
                ext,
                now() - 2 * DAY_MS,
                &format!("Please send the {ext} file by Friday."),
                Some((t, "seed")),
            );
            let src = s.next_step_sources(10).unwrap().remove(0);
            s.record_step_extraction(&src, &[mine()], &[], "m", now())
                .unwrap();
        }
        let edit = StepAction::Edit {
            text: format!("Send key {secret}"),
            due_date: None,
        };
        assert!(s.apply_step_action("email:a#0", &edit).unwrap());
        let dismiss = StepAction::Dismiss {
            reason: format!("wrong, key is {secret}"),
        };
        assert!(s.apply_step_action("email:b#0", &dismiss).unwrap());
        mail(
            &s,
            "c",
            now() - DAY_MS,
            "Can you confirm the budget?",
            Some((t, "seed")),
        );
        let src = s.next_step_sources(10).unwrap().remove(0);
        assert_eq!(src.open_steps.len(), 1);
        assert_eq!(src.examples.len(), 1);
        let mut pairs = src.open_steps.iter().chain(&src.examples);
        assert!(!pairs.any(|(a, b)| a.contains(secret) || b.contains(secret)));
    }

    /// A filed meeting with `items` as generated action items, read into steps (all `mine`).
    fn meeting_steps(s: &Store, items: &[&str], completed: &[&str]) -> (i64, String) {
        s.set_own_device_id("me");
        let (t, _) = seed(s);
        let json = serde_json::to_string(items).unwrap();
        s.conn
            .execute(
                "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status, summary_action_items_json) \
                 VALUES ('me', ?1, 'Sync', '[]', 'manual', 'done', ?2)",
                params![now() - DAY_MS, json],
            )
            .unwrap();
        let mid = s.conn.last_insert_rowid();
        filed(s, "meeting", mid, now() - DAY_MS, "Sync", Some((t, "seed")));
        for c in completed {
            s.conn
                .execute(
                    "UPDATE meeting_action_item SET completed = 1 WHERE meeting_id = ?1 AND text = ?2",
                    params![mid, c],
                )
                .ok();
        }
        let src = s.next_step_sources(10).unwrap().remove(0);
        // seeding happens in next_step_sources; mark completed items now, then record
        for c in completed {
            s.conn
                .execute(
                    "UPDATE meeting_action_item SET completed = 1 WHERE meeting_id = ?1 AND text = ?2",
                    params![mid, c],
                )
                .unwrap();
        }
        let labels: Vec<LineLabel> = src.candidates.iter().map(|_| mine()).collect();
        s.record_step_extraction(&src, &labels, &[], "m", now())
            .unwrap();
        (mid, src.source_key)
    }

    fn step_status(s: &Store, key: &str) -> String {
        s.conn
            .query_row("SELECT status FROM topic_step WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .unwrap()
    }

    fn completed_of(s: &Store, mid: i64, text: &str) -> i64 {
        s.conn
            .query_row(
                "SELECT completed FROM meeting_action_item WHERE meeting_id = ?1 AND text = ?2",
                params![mid, text],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn regenerated_item_keeps_done_and_dismissed_state() {
        let s = Store::open_in_memory().unwrap();
        let (mid, sk) = meeting_steps(&s, &["send the SOW", "Book room"], &[]);
        let sow = meeting_key(&sk, "send the SOW");
        let room = meeting_key(&sk, "Book room");
        s.apply_step_action(&sow, &StepAction::Done).unwrap();
        s.apply_step_action(
            &room,
            &StepAction::Dismiss {
                reason: "not_task".into(),
            },
        )
        .unwrap();
        s.replace_meeting_action_items_from_generated(
            mid,
            &[
                "Send the SOW.".into(),
                "Book room".into(),
                "New thing".into(),
            ],
            now(),
        )
        .unwrap();
        assert_eq!(completed_of(&s, mid, "Send the SOW."), 1);
        assert_eq!(completed_of(&s, mid, "Book room"), 0);
        assert_eq!(step_status(&s, &room), "dismissed");
        let src = s.next_step_sources(10).unwrap().remove(0);
        assert_eq!(src.candidates, vec!["New thing"]);
    }

    #[test]
    fn check_off_in_meeting_detail_marks_the_step_done() {
        let s = Store::open_in_memory().unwrap();
        let (mid, sk) = meeting_steps(&s, &["Send the SOW."], &[]);
        let id: i64 = s
            .conn
            .query_row(
                "SELECT id FROM meeting_action_item WHERE meeting_id = ?1",
                [mid],
                |r| r.get(0),
            )
            .unwrap();
        let key = meeting_key(&sk, "Send the SOW.");
        s.set_meeting_action_item_completed(id, true, now())
            .unwrap();
        assert_eq!(step_status(&s, &key), "done");
        s.set_meeting_action_item_completed(id, false, now())
            .unwrap();
        assert_eq!(step_status(&s, &key), "open");
    }

    #[test]
    fn done_on_topic_page_completes_the_action_item() {
        let s = Store::open_in_memory().unwrap();
        let (mid, sk) = meeting_steps(&s, &["Send the SOW."], &[]);
        let key = meeting_key(&sk, "Send the SOW.");
        assert!(s.apply_step_action(&key, &StepAction::Done).unwrap());
        assert_eq!(completed_of(&s, mid, "Send the SOW."), 1);
        s.apply_step_action(&key, &StepAction::Reopen).unwrap();
        assert_eq!(completed_of(&s, mid, "Send the SOW."), 0);
    }

    #[test]
    fn already_completed_item_becomes_a_done_step() {
        let s = Store::open_in_memory().unwrap();
        let (_, sk) = meeting_steps(&s, &["Send the SOW.", "Book room"], &["Send the SOW."]);
        assert_eq!(step_status(&s, &meeting_key(&sk, "Send the SOW.")), "done");
        assert_eq!(step_status(&s, &meeting_key(&sk, "Book room")), "open");
    }

    #[test]
    fn dismiss_hides_and_becomes_an_example() {
        let s = Store::open_in_memory().unwrap();
        let (_, sk) = meeting_steps(&s, &["Book room"], &[]);
        let key = meeting_key(&sk, "Book room");
        s.apply_step_action(
            &key,
            &StepAction::Dismiss {
                reason: "not_mine".into(),
            },
        )
        .unwrap();
        let (t, _) = (s.create_seed_topic("Other", &[], None).unwrap().topic_id, 0);
        let e = mail(
            &s,
            "x1",
            now(),
            "Please send the budget to Pat",
            Some((t, "seed")),
        );
        let _ = e;
        let src = s.next_step_sources(10).unwrap().remove(0);
        assert_eq!(src.examples, vec![("Book room".into(), "not_mine".into())]);
    }

    #[test]
    fn edit_sets_origin_user_and_due() {
        let s = Store::open_in_memory().unwrap();
        let (_, sk) = meeting_steps(&s, &["Book room"], &[]);
        let key = meeting_key(&sk, "Book room");
        let before: i64 = s
            .conn
            .query_row(
                "SELECT updated_at_unix_ms FROM topic_step WHERE key = ?1",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        s.apply_step_action(
            &key,
            &StepAction::Edit {
                text: "Book the big room".into(),
                due_date: Some("2026-12-01".into()),
            },
        )
        .unwrap();
        let (text, due, origin, at): (String, Option<String>, String, i64) = s
            .conn
            .query_row(
                "SELECT text, due_date, origin, updated_at_unix_ms FROM topic_step WHERE key = ?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(text, "Book the big room");
        assert_eq!(due.as_deref(), Some("2026-12-01"));
        assert_eq!(origin, "user");
        assert!(at > before);
        s.apply_step_action(&key, &StepAction::Confirm { confirmed: true })
            .unwrap();
        s.apply_step_action(&key, &StepAction::Note { text: "call Ana".into() })
            .unwrap();
        let (c, n): (i64, String) = s
            .conn
            .query_row(
                "SELECT confirmed, note FROM topic_step WHERE key = ?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((c, n.as_str()), (1, "call Ana"));
    }

    #[test]
    fn reopen_clears_closed_by() {
        let s = Store::open_in_memory().unwrap();
        let (_, sk) = meeting_steps(&s, &["Book room"], &[]);
        let key = meeting_key(&sk, "Book room");
        s.conn
            .execute(
                "UPDATE topic_step SET status = 'done', closed_by = 'email x' WHERE key = ?1",
                [&key],
            )
            .unwrap();
        s.apply_step_action(&key, &StepAction::Reopen).unwrap();
        let cb: Option<String> = s
            .conn
            .query_row(
                "SELECT closed_by FROM topic_step WHERE key = ?1",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cb, None);
        assert_eq!(step_status(&s, &key), "open");
    }

    #[test]
    fn unknown_key_is_false() {
        let s = Store::open_in_memory().unwrap();
        assert!(!s.apply_step_action("nope", &StepAction::Done).unwrap());
    }

    fn pulled_step(key: &str, source_key: &str) -> TopicStepPayload {
        TopicStepPayload {
            key: key.into(),
            topic_key: "t".into(),
            source_key: source_key.into(),
            source_kind: "email".into(),
            source_title: "SOW".into(),
            text: "Send the SOW".into(),
            direction: "mine".into(),
            asked_at_unix_ms: 1,
            due_date: None,
            status: "open".into(),
            dismiss_reason: None,
            confirmed: false,
            note: None,
            closed_by: None,
            origin: "extracted".into(),
            updated_at_unix_ms: 1,
        }
    }

    fn source_id_of(s: &Store, key: &str) -> Option<i64> {
        s.conn
            .query_row(
                "SELECT source_id FROM topic_step WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn pulled_step_gets_local_source_id_and_is_cleaned_with_its_source() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("me");
        let email = |ext: &str| {
            s.conn
                .execute(
                    "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id) \
                     VALUES ('d','email','gmail',1000,json_array('Pat <pat@x.com>'),'SOW','Hi','{}',?1)",
                    [ext],
                )
                .unwrap();
            while crate::topics::index::run_topic_index_job(&s, 200).unwrap() {}
            s.conn
                .query_row(
                    "SELECT id FROM topic_source WHERE kind = 'email' AND ref_id = (SELECT id FROM items WHERE external_id = ?1)",
                    [ext],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap()
        };
        // Pulled before its source is here: no source yet; indexing it fills source_id.
        s.apply_synced_topic_step(&pulled_step("email:x#0", "email:x"))
            .unwrap();
        assert_eq!(source_id_of(&s, "email:x#0"), None);
        let x = email("x");
        assert_eq!(source_id_of(&s, "email:x#0"), Some(x));
        // Pulled after: set on apply.
        let y = email("y");
        s.apply_synced_topic_step(&pulled_step("email:y#0", "email:y"))
            .unwrap();
        assert_eq!(source_id_of(&s, "email:y#0"), Some(y));
        assert!(
            s.list_outbox_operations().unwrap().is_empty(),
            "never re-enqueued"
        );

        s.conn
            .execute("DELETE FROM items WHERE external_id = 'x'", [])
            .unwrap();
        let left: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM topic_step", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 1, "x's step goes with its source");
    }

    #[test]
    fn forgetting_a_meeting_deletes_its_steps() {
        let s = Store::open_in_memory().unwrap();
        let (mid, key) = meeting_steps(&s, &["Send the deck"], &[]);
        let count = |k: &str| -> i64 {
            s.conn
                .query_row(
                    "SELECT COUNT(*) FROM topic_step WHERE source_key = ?1",
                    [k],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(count(&key), 1);
        // A peer's meeting that is not here: its pulled step has no local source.
        let peer = "meeting:peer:7";
        s.apply_synced_topic_step(&pulled_step("meeting:peer:7#h1", peer))
            .unwrap();
        assert_eq!(count(peer), 1);

        s.forget_meeting(mid).unwrap().unwrap();
        assert_eq!(count(&key), 0);
        let pending: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sync_outbox WHERE row_type = 'topic_step' AND state = 'pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0, "its unsent step rows are cancelled");
        s.apply_meeting_forgotten(&crate::MeetingForgotten {
            origin_device_id: "peer".into(),
            origin_epoch: None,
            source_meeting_id: 7,
            forgotten_at_unix_ms: 1,
            source_start_unix_ms: None,
        })
        .unwrap();
        assert_eq!(count(peer), 0, "deleted by key, not only by source_id");
        // A late row for either forgotten meeting is dropped.
        s.apply_synced_topic_step(&pulled_step("meeting:peer:7#h1", peer))
            .unwrap();
        s.apply_synced_topic_step(&pulled_step(&format!("{key}#h1"), &key))
            .unwrap();
        assert_eq!(count(peer) + count(&key), 0);
    }
}
