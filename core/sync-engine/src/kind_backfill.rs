//! The one-time `kind_tag` backfill (docs/superpowers/specs/2026-11-17-items-kind-tag-design.md, "Backfill"):
//! tags the `items` rows on the relay that were pushed before clients tagged them, so the runner's kind-filtered
//! pull can skip them at the query. Run from a full device in short steps ([`kind_backfill_step`]).
//!
//! It walks every device's rows (this device's too) by primary-key range, `(device_id, local_seq)` window by
//! window, like the tombstone catch-up (`catch_up_tombstones`): Turso bills rows read, which counts rows scanned,
//! so a repeated `WHERE kind_tag IS NULL` over the table would read the whole table every time. Each `items` row
//! still NULL in a window is decrypted (any device's: the group key and `content_row_aad` open them all), decoded
//! as the pull decodes it, and tagged by primary key, one UPDATE per kind present in a chunk. Progress per peer is
//! saved in the local kv after every chunk, so a step cut short resumes where it stopped. An undecryptable or undecodable row stays NULL (the pull fetches
//! NULL rows, so it is never hidden) and is counted.
//!
//! One device at a time, by an advisory claim row ([`ROW_TYPE_KIND_BACKFILL_CLAIM`]): a step that sees another
//! device's claim younger than [`CLAIM_FRESH_MS`] does nothing. The claim is only an optimization: the UPDATE
//! writes only a NULL `items` row, and every device computes the same tag for it, so two devices that miss each
//! other's claim still leave every row correctly tagged.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use libsql::{params, params::IntoParams, Connection};
use memory::Store;
use std::time::{Duration, Instant};
use sync::GroupKey;

use crate::content::{
    content_row_aad, distinct_peer_device_ids, ensure_sync_row_column, item_payload_kind_tag, peer_has_marker,
    KIND_TAG_COLUMN_SEEN, Latch,
    refresh_marker, ROW_TYPE_ITEMS, ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_KIND_BACKFILL_CLAIM,
};
use crate::{now_unix_ms, SyncEngineError};

/// Bump to walk every relay again (a new `kind_tag` version): it is part of every progress key, and a
/// `kind_backfill_done` of another version does not count as finished.
const KIND_BACKFILL_VERSION: &str = "1";

/// kv: set once every peer is done, so later steps return at once without touching the relay. Its value is
/// `"<version>:<total>:<undecryptable>"`, the last progress, which a finished step reports.
const KIND_BACKFILL_DONE_KEY: &str = "kind_backfill_done";

/// One backfill pass over the relay's `items` rows: the column it fills and the keys and claim it keeps. The kind
/// pass is [`KIND_PASS`]; the runner's slice pass (`slice_backfill`) shares the walk, claim, retries and progress.
pub(crate) struct Pass {
    /// The `sync_row` column it fills (one of our own constant names, never input).
    pub(crate) column: &'static str,
    /// Part of every progress key; a done marker of another version does not count.
    pub(crate) version: &'static str,
    /// kv: `<prefix>:<version>:<peer>` per peer and `<prefix>_undecryptable:<version>`.
    pub(crate) kv_prefix: &'static str,
    /// kv: set once every peer is done, `"<version>:<total>:<undecryptable>"`.
    pub(crate) done_key: &'static str,
    /// Its advisory claim's row type, at `local_seq` 0 under the claimer's id.
    pub(crate) claim_row_type: &'static str,
    /// One chunk of a peer's window: `items` rows whose column is still NULL.
    pub(crate) chunk_sql: &'static str,
    /// Set once this process has seen the column.
    pub(crate) column_seen: &'static Latch,
    /// The tag of a decrypted `items` payload, `None` when it does not decode.
    pub(crate) tag: fn(&GroupKey, &[u8]) -> Option<String>,
}

impl Pass {
    /// The pass's name in log lines: `kind` or `slice`.
    pub(crate) fn name(&self) -> &'static str {
        self.column.strip_suffix("_tag").unwrap_or(self.column)
    }

    /// kv per peer: how far the walk has gone through that peer's rows (`local_seq`), or `done` at its top.
    fn peer_key(&self, peer: &str) -> String {
        format!("{}:{}:{peer}", self.kv_prefix, self.version)
    }

    /// kv: undecryptable rows seen so far, over all steps.
    fn undecryptable_key(&self) -> String {
        format!("{}_undecryptable:{}", self.kv_prefix, self.version)
    }

    /// Tags one chunk's rows of one tag by primary key (`device_id` plus each `local_seq`), only while each is
    /// still NULL: running again, or two devices at once, changes nothing, and a row tagged since the chunk was read
    /// keeps its tag. One statement per distinct tag in a chunk, not one per row: a round trip per row would be
    /// about 2M round trips over the relay. `?3..` are the chunk's `local_seq`s: at most [`CHUNK_ROWS`] (500), so
    /// with the two others well under SQLite's 999-variable floor.
    fn update_sql(&self, seq_count: usize) -> String {
        let seqs = (0..seq_count).map(|i| format!("?{}", i + 3)).collect::<Vec<_>>().join(", ");
        let column = self.column;
        format!(
            "UPDATE sync_row SET {column} = ?1 \
             WHERE device_id = ?2 AND row_type = 'items' AND {column} IS NULL AND local_seq IN ({seqs})"
        )
    }
}

/// The `kind_tag` pass, the apps' and the runner's.
pub(crate) static KIND_PASS: Pass = Pass {
    column: "kind_tag",
    version: KIND_BACKFILL_VERSION,
    kv_prefix: "kind_backfill",
    done_key: KIND_BACKFILL_DONE_KEY,
    claim_row_type: ROW_TYPE_KIND_BACKFILL_CLAIM,
    chunk_sql: CHUNK_SQL,
    column_seen: &KIND_TAG_COLUMN_SEEN,
    tag: item_payload_kind_tag,
};

/// A hard stop past the budget for a relay call that never returns, as `CATCH_UP_BACKSTOP_SLACK`: what was saved
/// before it stays, and the step returns an error.
pub(crate) const BACKSTOP_SLACK: Duration = Duration::from_secs(3);

/// Rows one chunk reads (and at most updates) before the pause.
const CHUNK_ROWS: i64 = 500;

/// How many `local_seq` values one chunk query spans, as `CATCH_UP_SPAN`: a window bounds the rows one query
/// scans when a peer has long runs of other row types (or of rows already tagged).
const SPAN: i64 = 20_000;

/// The pause after a chunk that had rows. None in production: the step runs on the store's serial lane, so a sleep
/// would hold it for nothing; the short budget and one step per round are the throttle. Tests can set one.
const PAUSE: Duration = Duration::ZERO;

/// Another device's claim younger than this holds our step; an older one belongs to a device that stopped.
const CLAIM_FRESH_MS: i64 = 600_000;

/// How far in the future (another device's clock ahead of ours) a claim may be dated and still count as fresh. A
/// claim dated further ahead is ignored: otherwise a skewed clock would hold every other device until real time
/// passed its date plus [`CLAIM_FRESH_MS`].
const CLAIM_SKEW_MS: i64 = 60_000;

/// Whether a claim `age_ms` old (negative: dated in the future) still holds other devices.
fn claim_is_fresh(age_ms: i64) -> bool {
    (-CLAIM_SKEW_MS..CLAIM_FRESH_MS).contains(&age_ms)
}

/// One relay row of a peer's window: the `items` row's `local_seq` and its ciphertext.
const CHUNK_SQL: &str = "SELECT local_seq, ciphertext FROM sync_row \
     WHERE device_id = ?1 AND local_seq > ?2 AND local_seq <= ?3 AND row_type = 'items' AND kind_tag IS NULL \
     ORDER BY local_seq LIMIT ?4";

const _: () = assert!(CHUNK_ROWS + 2 <= 999, "a chunk's UPDATE must stay under SQLite's variable limit");

/// A peer's highest `local_seq`: where its walk is done (a primary-key seek, as `CATCH_UP_END_SQL`).
const PEER_END_SQL: &str = "SELECT MAX(local_seq) FROM sync_row WHERE device_id = ?1";

/// A peer's claim: a primary-key point lookup, never a scan by `row_type`.
const CLAIM_SQL: &str = "SELECT created_at_unix_ms FROM sync_row WHERE device_id = ?1 AND local_seq = 0 AND row_type = ?2";

/// kv: the last unfinished progress as `"<done>:<total>"`, what a desktop's Settings reads for "Indexing history".
/// Removed once the backfill finishes.
const KIND_BACKFILL_PROGRESS_KEY: &str = "kind_backfill_progress";

/// Saves a step's progress where Settings reads it. Finished removes it. Unfinished with nothing walked yet
/// (`done == 0`, which includes `total == 0`: a relay with no rows, a column that could not be confirmed) keeps
/// what was saved before: a step another device's claim holds, or one turned away before this device's first
/// push, is not this device indexing, so it never shows "0%".
pub(crate) fn record_progress(store: &Store, progress: &KindBackfillProgress) -> Result<(), SyncEngineError> {
    if progress.finished {
        // Only a write when a value is there: a finished device runs this every round.
        if store.get_meta(KIND_BACKFILL_PROGRESS_KEY).map_err(local_err)?.is_some() {
            store.delete_meta(KIND_BACKFILL_PROGRESS_KEY).map_err(local_err)?;
        }
        Ok(())
    } else if progress.done > 0 && progress.total > 0 {
        store
            .set_meta(KIND_BACKFILL_PROGRESS_KEY, &format!("{}:{}", progress.done, progress.total))
            .map_err(local_err)
    } else {
        Ok(())
    }
}

/// The percentage Settings shows as "Indexing history: N%", `None` when nothing should show: not running yet,
/// finished, or nothing to count. Below 100 while unfinished.
pub fn kind_backfill_percent(store: &Store) -> Option<u8> {
    let saved = store.get_meta(KIND_BACKFILL_PROGRESS_KEY).ok().flatten()?;
    let (done, total) = saved.split_once(':')?;
    let (done, total) = (done.parse::<u64>().ok()?, total.parse::<u64>().ok()?);
    if total == 0 {
        return None;
    }
    Some((u128::from(done.min(total)) * 100 / u128::from(total)).min(99) as u8)
}

/// Where the backfill stands. `done` and `total` count `local_seq` positions over every device's rows (the sum of
/// each device's walked position, and of its highest `local_seq`), not `items` rows: counting rows would read the
/// whole table. Fine for a percentage. `undecryptable` counts rows left NULL because they would not open or decode,
/// over all steps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KindBackfillProgress {
    pub done: u64,
    pub total: u64,
    pub finished: bool,
    pub undecryptable: u64,
}

#[cfg(test)]
thread_local! {
    /// Tests: `(chunk rows, span, pause)`.
    pub(crate) static TEST_PACE: std::cell::Cell<Option<(i64, i64, Duration)>> = const { std::cell::Cell::new(None) };
    /// Tests: two steps that missed each other's claim.
    pub(crate) static TEST_IGNORE_CLAIMS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: when `Some`, every relay statement this module issues is appended.
    pub(crate) static CAPTURED_SQL: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

fn pace() -> (i64, i64, Duration) {
    #[cfg(test)]
    if let Some(p) = TEST_PACE.with(|c| c.get()) {
        return p;
    }
    (CHUNK_ROWS, SPAN, PAUSE)
}

fn claims_ignored() -> bool {
    #[cfg(test)]
    return TEST_IGNORE_CLAIMS.with(|c| c.get());
    #[cfg(not(test))]
    false
}

fn capture(_sql: &str) {
    #[cfg(test)]
    CAPTURED_SQL.with(|c| {
        if let Some(v) = c.borrow_mut().as_mut() {
            v.push(_sql.to_string());
        }
    });
}

fn relay_err(e: impl std::fmt::Display) -> SyncEngineError {
    SyncEngineError::Query(e.to_string())
}

fn local_err(e: impl std::fmt::Display) -> SyncEngineError {
    SyncEngineError::Query(format!("local store: {e}"))
}

async fn query(conn: &Connection, sql: &str, params: impl IntoParams) -> Result<libsql::Rows, SyncEngineError> {
    capture(sql);
    conn.query(sql, params).await.map_err(relay_err)
}

async fn execute(conn: &Connection, sql: &str, params: impl IntoParams) -> Result<u64, SyncEngineError> {
    capture(sql);
    conn.execute(sql, params).await.map_err(relay_err)
}

/// One peer's walk: its `local_seq` reached so far and its top.
struct Walk {
    peer: String,
    after: i64,
    end: i64,
}

/// Runs the backfill for at most about `budget` (at least one chunk or window, so every step moves forward), in
/// chunks of [`CHUNK_ROWS`] with a pause between them, then returns where it stands. Never fails for an
/// undecryptable row or another device's claim; a relay or kv error returns `Err`, with every chunk written before
/// it saved. Once finished, later calls return from the local kv without touching the relay.
pub async fn kind_backfill_step(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    budget: Duration,
) -> Result<KindBackfillProgress, SyncEngineError> {
    pass_step(&KIND_PASS, conn, group_key, store, own_device_id, budget).await
}

/// One bounded step of `pass` ([`kind_backfill_step`] for [`KIND_PASS`]).
pub(crate) async fn pass_step(
    pass: &Pass,
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    budget: Duration,
) -> Result<KindBackfillProgress, SyncEngineError> {
    if let Some(done) = store.get_meta(pass.done_key).map_err(local_err)?.as_deref().and_then(|v| finished_progress(pass, v)) {
        return Ok(done);
    }
    match tokio::time::timeout(budget + BACKSTOP_SLACK, step(pass, conn, group_key, store, own_device_id, budget)).await {
        Ok(result) => result,
        Err(_) => Err(SyncEngineError::Query(format!("{} backfill: a relay call took too long", pass.column))),
    }
}

async fn step(
    pass: &Pass,
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    budget: Duration,
) -> Result<KindBackfillProgress, SyncEngineError> {
    // Without the column there is nothing to write into; a later step tries again. Once this process has seen
    // the column (the latch `ensure_content_schema` sets), a step does no PRAGMA.
    if !pass.column_seen.get() {
        if !ensure_sync_row_column(conn, pass.column).await {
            return Ok(KindBackfillProgress::default());
        }
        pass.column_seen.set();
    }
    let deadline = Instant::now() + budget;
    let mut undecryptable = store
        .get_meta(&pass.undecryptable_key())
        .map_err(local_err)?
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let mut peers = distinct_peer_device_ids(conn, own_device_id).await?;
    if !own_device_id.is_empty() {
        peers.push(own_device_id.to_string());
    }
    peers.sort();
    let mut walks = Vec::with_capacity(peers.len());
    for peer in peers {
        let end = peer_end(conn, &peer).await?;
        let after = match store.get_meta(&pass.peer_key(&peer)).map_err(local_err)?.as_deref() {
            Some("done") => end,
            Some(seq) => seq.parse::<i64>().unwrap_or(0),
            None => 0,
        };
        walks.push(Walk { peer, after, end });
    }

    if walks.iter().any(|w| w.after < w.end) {
        // A device not yet on the relay as itself does not claim: a claim row alone would make it a peer without
        // capability markers, and hold every other device's marker gates (`all_peers_have_marker`).
        if !peer_has_marker(conn, own_device_id, ROW_TYPE_ITEMS_CAPABILITY).await? {
            return Ok(progress(&walks, undecryptable, false));
        }
        if !claims_ignored() && another_fresh_claim(pass, conn, &walks, own_device_id).await? {
            return Ok(progress(&walks, undecryptable, false));
        }
        refresh_marker(conn, group_key, own_device_id, pass.claim_row_type).await?;
        let mut moved = false;
        for walk in walks.iter_mut() {
            if !walk_peer(pass, conn, group_key, store, walk, deadline, &mut moved, &mut undecryptable).await? {
                return Ok(progress(&walks, undecryptable, false));
            }
        }
    }

    for walk in &walks {
        store.set_meta(&pass.peer_key(&walk.peer), "done").map_err(local_err)?;
    }
    let done = progress(&walks, undecryptable, true);
    store
        .set_meta(pass.done_key, &format!("{}:{}:{}", pass.version, done.total, done.undecryptable))
        .map_err(local_err)?;
    Ok(done)
}

/// Why a runner backfill run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillStop {
    /// Every position was walked.
    Finished,
    /// The total budget was spent (or too little was left to start another step).
    Budget,
    /// Two steps in a row moved nothing: another device's fresh claim holds the walk.
    ClaimHeld,
    /// The pass was not run at all (the runner's `backfill --slice-only` skips the kind pass). No pass returns it.
    Skipped,
}

/// A step that fails is retried this many times in a row (same store: progress is saved per chunk) before the run
/// gives up with the last error.
const STEP_RETRIES: u32 = 3;

/// Below this much of the total budget left, no new step starts.
pub(crate) const MIN_STEP_REMAINING: Duration = Duration::from_secs(10);

/// The pause before retry `n` (1-based): 1 s, 2 s, 4 s. None in tests.
fn retry_pause(n: u32) -> Duration {
    if cfg!(test) {
        Duration::ZERO
    } else {
        Duration::from_secs(1 << (n - 1).min(2))
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: how many of the next steps fail with a relay error before one is allowed to run.
    pub(crate) static TEST_FAIL_STEPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The cloud runner's one-off long run (`runner_kind_backfill`): [`kind_backfill_step`] again and again, each for at
/// most `step_budget` (and no more than what is left of `total_budget`; at least one step always runs), until it
/// finishes, `total_budget` is spent (no step starts with under [`MIN_STEP_REMAINING`] left), or two steps in a row
/// move nothing (another device's fresh claim holds the walk: stop and report, never spin). A failed step is retried
/// up to [`STEP_RETRIES`] times in a row with the same store, then its error is returned; a step that succeeds resets
/// the count. `on_step` sees each step's progress and the step count so far.
///
/// Announces the runner's pull-only markers first (the same single idempotent statement `runner_pull` runs every
/// time, `DO NOTHING` on conflict, so a marker already there is untouched): without its `items_capability` marker
/// a step does nothing. A relay still without it after that is an error, not a silent no-op.
pub(crate) async fn run_runner_backfill(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    device_id: &str,
    total_budget: Duration,
    step_budget: Duration,
    on_step: &mut dyn FnMut(&KindBackfillProgress, usize),
) -> Result<(KindBackfillProgress, BackfillStop), SyncEngineError> {
    run_runner_pass(&KIND_PASS, conn, group_key, store, device_id, total_budget, step_budget, on_step).await
}

/// [`run_runner_backfill`] for any [`Pass`]: the same markers, steps, retries, stall rule and floor.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_runner_pass(
    pass: &Pass,
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    device_id: &str,
    total_budget: Duration,
    step_budget: Duration,
    on_step: &mut dyn FnMut(&KindBackfillProgress, usize),
) -> Result<(KindBackfillProgress, BackfillStop), SyncEngineError> {
    crate::content::ensure_content_schema(conn).await?;
    crate::content::announce_pull_only_markers(conn, group_key, device_id).await;
    if !peer_has_marker(conn, device_id, ROW_TYPE_ITEMS_CAPABILITY).await? {
        return Err(SyncEngineError::Query(format!(
            "{} backfill: the runner's capability marker is not on the relay",
            pass.column
        )));
    }
    // The runner's store is new each execution: take the walk's saved position back from the relay.
    load_cursors(pass, conn, group_key, store, device_id).await;
    let deadline = Instant::now() + total_budget;
    let (mut steps, mut stalled, mut prev_done, mut failures) = (0usize, 0u8, 0u64, 0u32);
    loop {
        let budget = step_budget.min(deadline.saturating_duration_since(Instant::now()));
        let result = {
            #[cfg(test)]
            let injected = TEST_FAIL_STEPS.with(|c| {
                let n = c.get();
                c.set(n.saturating_sub(1));
                n > 0
            });
            #[cfg(not(test))]
            let injected = false;
            if injected {
                Err(SyncEngineError::Query("injected relay error".into()))
            } else {
                pass_step(pass, conn, group_key, store, device_id, budget).await
            }
        };
        let p = match result {
            Ok(p) => {
                failures = 0;
                p
            }
            Err(e) => {
                failures += 1;
                if failures > STEP_RETRIES {
                    return Err(e);
                }
                eprintln!("backfill: {} step failed, retrying ({failures}/{STEP_RETRIES})", pass.name());
                tokio::time::sleep(retry_pause(failures)).await;
                continue;
            }
        };
        steps += 1;
        save_cursors(pass, conn, group_key, store, device_id).await;
        on_step(&p, steps);
        if p.finished {
            return Ok((p, BackfillStop::Finished));
        }
        stalled = if p.done > prev_done { 0 } else { stalled + 1 };
        prev_done = p.done;
        if stalled >= 2 {
            return Ok((p, BackfillStop::ClaimHeld));
        }
        if deadline.saturating_duration_since(Instant::now()) < MIN_STEP_REMAINING {
            return Ok((p, BackfillStop::Budget));
        }
    }
}

/// The walk's saved position as the runner's claim row's payload (the claim row of the pass, `local_seq` 0, which
/// only carries "1" until a runner saves its position there; its `created_at_unix_ms` is what other devices read).
/// Positions only: no row content, kind or date. Encrypted like every row.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SavedCursors {
    /// The pass's version; another version's cursors do not count.
    v: String,
    /// kv value per peer key (`local_seq` or `done`).
    peers: std::collections::BTreeMap<String, String>,
    /// The done marker, once finished.
    done: Option<String>,
    undecryptable: Option<String>,
}

/// Puts the cursors saved on the relay into `store`, unless it already has a position for the pass (it is not new).
/// Best effort: a payload that is missing, will not open or is another version's leaves the walk starting at 0,
/// which is only slower, never wrong (the walk skips rows that already have the column).
async fn load_cursors(pass: &Pass, conn: &Connection, group_key: &GroupKey, store: &Store, device_id: &str) {
    let Ok(existing) = store.list_meta_with_prefix(pass.kv_prefix) else { return };
    if !existing.is_empty() {
        return;
    }
    let Ok(mut rows) = query(conn, CLAIM_PAYLOAD_SQL, params![device_id, pass.claim_row_type]).await else { return };
    let Ok(Some(row)) = rows.next().await else { return };
    let Ok(text) = row.get::<String>(0) else { return };
    let Ok(sealed) = STANDARD.decode(text) else { return };
    let aad = content_row_aad(device_id, 0, pass.claim_row_type);
    let Ok(plain) = sync::decrypt_payload(group_key, &sealed, &aad) else { return };
    let Ok(saved) = serde_json::from_slice::<SavedCursors>(&plain) else { return };
    if saved.v != pass.version {
        return;
    }
    for (key, value) in saved.peers {
        let _ = store.set_meta(&key, &value);
    }
    if let Some(done) = saved.done {
        let _ = store.set_meta(pass.done_key, &done);
    }
    if let Some(u) = saved.undecryptable {
        let _ = store.set_meta(&pass.undecryptable_key(), &u);
    }
}

/// Saves the walk's position into the runner's claim row after a step (the row `step` refreshed; a step that
/// walked nothing has none, so nothing is written). Best effort: a failed save costs only a longer next walk.
async fn save_cursors(pass: &Pass, conn: &Connection, group_key: &GroupKey, store: &Store, device_id: &str) {
    let prefix = format!("{}:{}:", pass.kv_prefix, pass.version);
    let Ok(list) = store.list_meta_with_prefix(&prefix) else { return };
    let saved = SavedCursors {
        v: pass.version.to_string(),
        peers: list.into_iter().collect(),
        done: store.get_meta(pass.done_key).ok().flatten(),
        undecryptable: store.get_meta(&pass.undecryptable_key()).ok().flatten(),
    };
    let Ok(json) = serde_json::to_vec(&saved) else { return };
    let aad = content_row_aad(device_id, 0, pass.claim_row_type);
    let sealed = STANDARD.encode(sync::encrypt_payload(group_key, &json, &aad));
    let _ = execute(conn, SAVE_CURSORS_SQL, params![sealed.as_str(), device_id, pass.claim_row_type]).await;
}

const CLAIM_PAYLOAD_SQL: &str = "SELECT ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq = 0 AND row_type = ?2";
const SAVE_CURSORS_SQL: &str = "UPDATE sync_row SET ciphertext = ?1 WHERE device_id = ?2 AND local_seq = 0 AND row_type = ?3";

/// The progress a `kind_backfill_done` value records, or `None` when it is another version's (walk again).
fn finished_progress(pass: &Pass, value: &str) -> Option<KindBackfillProgress> {
    let mut parts = value.split(':');
    if parts.next() != Some(pass.version) {
        return None;
    }
    let mut numbers = parts.map(|p| p.parse::<u64>().unwrap_or(0));
    let total = numbers.next().unwrap_or(0);
    let undecryptable = numbers.next().unwrap_or(0);
    Some(KindBackfillProgress { done: total, total, finished: true, undecryptable })
}

fn progress(walks: &[Walk], undecryptable: u64, finished: bool) -> KindBackfillProgress {
    let (mut done, mut total) = (0u64, 0u64);
    for w in walks {
        let end = w.end.max(0) as u64;
        total += end;
        done += (w.after.clamp(0, w.end.max(0))) as u64;
    }
    KindBackfillProgress { done, total, finished, undecryptable }
}

async fn peer_end(conn: &Connection, peer: &str) -> Result<i64, SyncEngineError> {
    let mut rows = query(conn, PEER_END_SQL, params![peer]).await?;
    Ok(match rows.next().await.map_err(relay_err)? {
        Some(row) => row.get::<Option<i64>>(0).map_err(relay_err)?.unwrap_or(0),
        None => 0,
    })
}

/// Whether a device other than this one claimed the backfill less than [`CLAIM_FRESH_MS`] ago (and not more than
/// [`CLAIM_SKEW_MS`] in the future).
async fn another_fresh_claim(pass: &Pass, conn: &Connection, walks: &[Walk], own_device_id: &str) -> Result<bool, SyncEngineError> {
    let now = now_unix_ms();
    for walk in walks.iter().filter(|w| w.peer != own_device_id) {
        let mut rows = query(conn, CLAIM_SQL, params![walk.peer.as_str(), pass.claim_row_type]).await?;
        if let Some(row) = rows.next().await.map_err(relay_err)? {
            let at: i64 = row.get(0).map_err(relay_err)?;
            if claim_is_fresh(now.saturating_sub(at)) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The tag for one `items` row, or `None` when it does not open or decode.
fn row_tag(pass: &Pass, group_key: &GroupKey, peer: &str, local_seq: i64, ciphertext: &str) -> Option<String> {
    let sealed = STANDARD.decode(ciphertext).ok()?;
    let plaintext = sync::decrypt_payload(group_key, &sealed, &content_row_aad(peer, local_seq, ROW_TYPE_ITEMS)).ok()?;
    (pass.tag)(group_key, &plaintext)
}

/// One peer's walk, chunk by chunk from its saved progress, saved after every chunk is written (and at the end of
/// every window). `Ok(true)`: done; `Ok(false)`: `deadline` passed (after at least one chunk or window this step).
#[allow(clippy::too_many_arguments)]
async fn walk_peer(
    pass: &Pass,
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    walk: &mut Walk,
    deadline: Instant,
    moved: &mut bool,
    undecryptable: &mut u64,
) -> Result<bool, SyncEngineError> {
    let (chunk_rows, span, pause) = pace();
    let key = pass.peer_key(&walk.peer);
    loop {
        if walk.after >= walk.end {
            store.set_meta(&key, "done").map_err(local_err)?;
            return Ok(true);
        }
        if *moved && Instant::now() >= deadline {
            return Ok(false);
        }
        let window_end = walk.after.saturating_add(span.max(1)).min(walk.end);
        let chunk: Vec<(i64, String)> = {
            let mut rows =
                query(conn, pass.chunk_sql, params![walk.peer.as_str(), walk.after, window_end, chunk_rows]).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await.map_err(relay_err)? {
                out.push((row.get(0).map_err(relay_err)?, row.get(1).map_err(relay_err)?));
            }
            out
        };
        // Tests: let a concurrent step read the same chunk before this one writes it (test f).
        #[cfg(test)]
        tokio::task::yield_now().await;
        let full = chunk.len() as i64 >= chunk_rows;
        let had_rows = !chunk.is_empty();
        let last_seq = chunk.last().map(|(seq, _)| *seq);
        // Group the chunk by tag: one UPDATE per kind present, over that kind's rows.
        let mut by_tag: std::collections::BTreeMap<String, Vec<i64>> = std::collections::BTreeMap::new();
        let mut chunk_undecryptable = 0u64;
        for (local_seq, ciphertext) in &chunk {
            match row_tag(pass, group_key, &walk.peer, *local_seq, ciphertext) {
                Some(tag) => by_tag.entry(tag).or_default().push(*local_seq),
                // Left NULL for good (the pull still fetches it), and counted.
                None => chunk_undecryptable += 1,
            }
        }
        for (tag, seqs) in by_tag {
            let mut values: Vec<libsql::Value> = Vec::with_capacity(seqs.len() + 2);
            values.push(tag.into());
            values.push(walk.peer.as_str().into());
            values.extend(seqs.into_iter().map(libsql::Value::Integer));
            execute(conn, &pass.update_sql(values.len() - 2), libsql::params::Params::Positional(values)).await?;
        }
        // The chunk is written: its progress is saved before the next one is read, so a step cut short (or a
        // relay error mid-chunk) re-reads at most this chunk, whose tagged rows the `IS NULL` read skips.
        if chunk_undecryptable > 0 {
            *undecryptable += chunk_undecryptable;
            store
                .set_meta(&pass.undecryptable_key(), &undecryptable.to_string())
                .map_err(local_err)?;
        }
        walk.after = if full { last_seq.unwrap_or(window_end) } else { window_end };
        store.set_meta(&key, &walk.after.to_string()).map_err(local_err)?;
        *moved = true;
        if had_rows && walk.after < walk.end && !pause.is_zero() {
            tokio::time::sleep(pause.min(deadline.saturating_duration_since(Instant::now()))).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        content_row_aad, ensure_content_schema, kind_tag_for, push_marker, ItemPayload, ROW_TYPE_ITEMS,
        ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_KIND_BACKFILL_CLAIM, ROW_TYPE_MEETING,
    };

    const KINDS: [&str; 3] = ["capture", "transcript", "calendar_event"];
    const DEVICES: [&str; 3] = ["dev-a", "dev-b", "dev-c"];
    const ROWS_PER_DEVICE: i64 = 30;
    const LONG: Duration = Duration::from_secs(60);

    /// Small chunks and windows, no pause: every test walks several windows and chunks per peer.
    fn small_pace() {
        TEST_PACE.with(|c| c.set(Some((4, 7, Duration::ZERO))));
    }

    async fn relay() -> (tempfile::TempDir, libsql::Database, Connection, GroupKey) {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db")).build().await.unwrap();
        let conn = db.connect().unwrap();
        ensure_content_schema(&conn).await.unwrap();
        (dir, db, conn, GroupKey::generate())
    }

    fn item(kind: &str, body: &str) -> memory::AppliedItem {
        let calendar = kind == "calendar_event";
        memory::AppliedItem {
            kind: kind.to_string(),
            source: calendar.then(|| "outlook".to_string()),
            ts_unix_ms: 1_700_000_000_000,
            actors: None,
            title: calendar.then(|| body.to_string()),
            body_text: body.to_string(),
            refs: None,
            flags: None,
            hash: None,
            external_id: calendar.then(|| format!("{body}#1")),
            app_bundle_id: None,
            url: None,
        }
    }

    fn item_json(kind: &str, body: &str) -> String {
        serde_json::to_string(&ItemPayload { source_meeting_id: None, meeting_local_seq: None, item: item(kind, body) }).unwrap()
    }

    async fn seed(conn: &Connection, key: &GroupKey, device: &str, seq: i64, row_type: &str, json: &str, tag: Option<String>) {
        let aad = content_row_aad(device, seq, row_type);
        let ciphertext = STANDARD.encode(sync::encrypt_payload(key, json.as_bytes(), &aad));
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms, kind_tag) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![device, seq, row_type, ciphertext.as_str(), tag],
        )
        .await
        .unwrap();
    }

    fn kind_of(device_index: usize, seq: i64) -> &'static str {
        KINDS[(seq as usize + device_index) % KINDS.len()]
    }

    /// Three devices, each with capability markers, `items` rows of every kind (NULL), a `meeting` row every
    /// fifth seq (NULL, must stay NULL), and one `items` row a new client already tagged on push.
    async fn fixture(conn: &Connection, key: &GroupKey) {
        for (d, device) in DEVICES.iter().enumerate() {
            push_marker(conn, key, device, ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
            for seq in 1..=ROWS_PER_DEVICE {
                if seq % 5 == 0 {
                    seed(conn, key, device, seq, ROW_TYPE_MEETING, "{}", None).await;
                } else if seq == ROWS_PER_DEVICE - 1 {
                    let kind = kind_of(d, seq);
                    seed(conn, key, device, seq, ROW_TYPE_ITEMS, &item_json(kind, "pushed"), Some(sync::kind_tag(key, kind))).await;
                } else {
                    seed(conn, key, device, seq, ROW_TYPE_ITEMS, &item_json(kind_of(d, seq), &format!("{device} {seq}")), None).await;
                }
            }
        }
    }

    /// Every row as `(device, local_seq, row_type, kind_tag)`, in key order.
    async fn snapshot(conn: &Connection) -> Vec<(String, i64, String, Option<String>)> {
        let mut rows = conn
            .query("SELECT device_id, local_seq, row_type, kind_tag FROM sync_row ORDER BY device_id, local_seq, row_type", ())
            .await
            .unwrap();
        let mut out = Vec::new();
        while let Some(r) = rows.next().await.unwrap() {
            out.push((r.get(0).unwrap(), r.get(1).unwrap(), r.get(2).unwrap(), r.get(3).unwrap()));
        }
        out
    }

    async fn count(conn: &Connection, sql: &str) -> i64 {
        let mut rows = conn.query(sql, ()).await.unwrap();
        rows.next().await.unwrap().unwrap().get(0).unwrap()
    }

    /// Asserts the fixture is fully and correctly tagged: every `items` row carries its own kind's tag, every other
    /// row (meetings, markers, the claim) none.
    async fn assert_tagged(conn: &Connection, key: &GroupKey) {
        for (device, seq, row_type, tag) in snapshot(conn).await {
            if row_type == ROW_TYPE_ITEMS {
                let d = DEVICES.iter().position(|x| *x == device).unwrap();
                assert_eq!(tag, Some(sync::kind_tag(key, kind_of(d, seq))), "{device}/{seq}");
            } else {
                assert_eq!(tag, None, "{device}/{seq}/{row_type} must stay untagged");
            }
        }
    }

    async fn run_to_end(conn: &Connection, key: &GroupKey, store: &Store, own: &str, budget: Duration) -> (KindBackfillProgress, usize) {
        for steps in 1..=10_000 {
            let p = kind_backfill_step(conn, key, store, own, budget).await.unwrap();
            if p.finished {
                return (p, steps);
            }
        }
        panic!("the backfill never finished");
    }

    // (a)
    #[tokio::test]
    async fn tags_exactly_the_null_items_rows_of_every_device() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();

        let (p, _) = run_to_end(&conn, &key, &store, "dev-a", LONG).await;

        assert!(p.finished);
        assert_eq!(p.undecryptable, 0);
        assert_eq!(p.total, 3 * ROWS_PER_DEVICE as u64);
        assert_eq!(p.done, p.total);
        assert_tagged(&conn, &key).await;
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL").await, 0);
    }

    /// The backfilled tag is the push path's tag for the same item: one function of the kind.
    #[tokio::test]
    async fn the_backfilled_tag_equals_the_push_tag() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        push_marker(&conn, &key, "dev-a", ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        for (i, kind) in KINDS.iter().enumerate() {
            seed(&conn, &key, "dev-b", i as i64 + 1, ROW_TYPE_ITEMS, &item_json(kind, "x"), None).await;
        }
        let store = Store::open_in_memory().unwrap();
        run_to_end(&conn, &key, &store, "dev-a", LONG).await;

        for (i, kind) in KINDS.iter().enumerate() {
            let pushed = memory::SyncableRow::Item { local_seq: 1, source_meeting_id: None, meeting_local_seq: None, item: item(kind, "x") };
            let tag: Option<String> = {
                let mut rows = conn
                    .query("SELECT kind_tag FROM sync_row WHERE device_id = 'dev-b' AND local_seq = ?1", params![i as i64 + 1])
                    .await
                    .unwrap();
                rows.next().await.unwrap().unwrap().get(0).unwrap()
            };
            assert_eq!(tag, kind_tag_for(&key, &pushed), "{kind}");
        }
    }

    // (b)
    #[tokio::test]
    async fn running_to_completion_twice_changes_nothing() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        run_to_end(&conn, &key, &Store::open_in_memory().unwrap(), "dev-a", LONG).await;
        let first = snapshot(&conn).await;

        // A fresh database (no progress saved) walks everything again: nothing is left to tag.
        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));
        let (p, _) = run_to_end(&conn, &key, &Store::open_in_memory().unwrap(), "dev-a", LONG).await;
        let sql = CAPTURED_SQL.with(|c| c.borrow_mut().take().unwrap());

        assert!(p.finished);
        assert!(!sql.iter().any(|s| s.starts_with("UPDATE")), "nothing left to update");
        let second: Vec<_> = snapshot(&conn).await.into_iter().filter(|r| r.2 != ROW_TYPE_KIND_BACKFILL_CLAIM).collect();
        let first: Vec<_> = first.into_iter().filter(|r| r.2 != ROW_TYPE_KIND_BACKFILL_CLAIM).collect();
        assert_eq!(first, second);
    }

    // (c)
    #[tokio::test]
    async fn a_run_cut_into_tiny_steps_ends_like_an_uninterrupted_one() {
        small_pace();
        let key = GroupKey::generate();
        let (_d1, _db1, interrupted, _) = relay().await;
        let (_d2, _db2, whole, _) = relay().await;
        fixture(&interrupted, &key).await;
        fixture(&whole, &key).await;

        let store = Store::open_in_memory().unwrap();
        let first = kind_backfill_step(&interrupted, &key, &store, "dev-a", Duration::ZERO).await.unwrap();
        assert!(!first.finished);
        assert!(first.done > 0, "a step always moves forward");
        assert!(first.done < first.total);
        let tagged = count(&interrupted, "SELECT COUNT(*) FROM sync_row WHERE kind_tag IS NOT NULL").await;
        assert!(tagged > 3 && tagged < 3 * 23, "stopped mid-way: {tagged}");
        let (_, steps) = run_to_end(&interrupted, &key, &store, "dev-a", Duration::ZERO).await;
        assert!(steps > 3, "the run was spread over steps: {steps}");

        run_to_end(&whole, &key, &Store::open_in_memory().unwrap(), "dev-a", LONG).await;
        assert_eq!(snapshot(&interrupted).await, snapshot(&whole).await);
        assert_tagged(&interrupted, &key).await;
    }

    // (d)
    #[tokio::test]
    async fn an_undecryptable_row_stays_null_and_is_counted() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        // Not base64; another group's ciphertext; a payload that is not an item.
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES ('dev-b', 101, 'items', '!!not base64', 0)",
            (),
        )
        .await
        .unwrap();
        seed(&conn, &GroupKey::generate(), "dev-b", 102, ROW_TYPE_ITEMS, &item_json("capture", "other group"), None).await;
        seed(&conn, &key, "dev-c", 103, ROW_TYPE_ITEMS, "{\"kind\": 7}", None).await;
        seed(&conn, &key, "dev-c", 104, ROW_TYPE_ITEMS, &item_json("capture", "after the bad ones"), None).await;
        let store = Store::open_in_memory().unwrap();

        let (p, _) = run_to_end(&conn, &key, &store, "dev-a", LONG).await;

        assert!(p.finished);
        assert_eq!(p.undecryptable, 3);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL").await, 3);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM sync_row WHERE local_seq IN (101, 102, 103) AND kind_tag IS NULL").await,
            3
        );
        let tag: String = {
            let mut rows = conn.query("SELECT kind_tag FROM sync_row WHERE local_seq = 104", ()).await.unwrap();
            rows.next().await.unwrap().unwrap().get(0).unwrap()
        };
        assert_eq!(tag, sync::kind_tag(&key, "capture"));
        // Finished reports the same count later.
        let again = kind_backfill_step(&conn, &key, &store, "dev-a", LONG).await.unwrap();
        assert_eq!(again, p);
    }

    async fn claim_at(conn: &Connection, key: &GroupKey, device: &str, at_ms: i64) {
        push_marker(conn, key, device, ROW_TYPE_KIND_BACKFILL_CLAIM).await.unwrap();
        conn.execute(
            "UPDATE sync_row SET created_at_unix_ms = ?2 WHERE device_id = ?1 AND local_seq = 0 AND row_type = ?3",
            params![device, at_ms, ROW_TYPE_KIND_BACKFILL_CLAIM],
        )
        .await
        .unwrap();
    }

    async fn claim_of(conn: &Connection, device: &str) -> Option<i64> {
        let mut rows = conn
            .query(
                "SELECT created_at_unix_ms FROM sync_row WHERE device_id = ?1 AND local_seq = 0 AND row_type = ?2",
                params![device, ROW_TYPE_KIND_BACKFILL_CLAIM],
            )
            .await
            .unwrap();
        rows.next().await.unwrap().map(|r| r.get(0).unwrap())
    }

    // (e)
    #[tokio::test]
    async fn a_fresh_claim_by_another_device_holds_the_step_and_a_stale_one_does_not() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();
        claim_at(&conn, &key, "dev-b", now_unix_ms() - 60_000).await;
        let before = snapshot(&conn).await;

        let held = kind_backfill_step(&conn, &key, &store, "dev-a", LONG).await.unwrap();

        assert!(!held.finished);
        assert_eq!(held.done, 0);
        assert_eq!(held.total, 3 * ROWS_PER_DEVICE as u64);
        assert_eq!(snapshot(&conn).await, before, "nothing written, not even our own claim");
        assert_eq!(claim_of(&conn, "dev-a").await, None);

        // Older than 10 minutes: its claimer is gone.
        claim_at(&conn, &key, "dev-b", now_unix_ms() - 601_000).await;
        let started = now_unix_ms();
        let (p, _) = run_to_end(&conn, &key, &store, "dev-a", LONG).await;
        assert!(p.finished);
        assert_tagged(&conn, &key).await;
        assert!(claim_of(&conn, "dev-a").await.unwrap() >= started, "our claim, refreshed");
    }

    /// Clock skew: a claim dated an hour ahead is ignored, one 30 s ahead still holds, one exactly 600 s old is ignored.
    #[tokio::test]
    async fn a_claim_dated_far_in_the_future_is_ignored() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        let before = snapshot(&conn).await;

        claim_at(&conn, &key, "dev-b", now_unix_ms() + 30_000).await;
        let held = kind_backfill_step(&conn, &key, &Store::open_in_memory().unwrap(), "dev-a", LONG).await.unwrap();
        assert!(!held.finished);
        let after_held: Vec<_> = snapshot(&conn).await.into_iter().filter(|r| r.2 != ROW_TYPE_KIND_BACKFILL_CLAIM).collect();
        let before_rows: Vec<_> = before.iter().filter(|r| r.2 != ROW_TYPE_KIND_BACKFILL_CLAIM).cloned().collect();
        assert_eq!(after_held, before_rows, "30 s ahead is within the skew: held, nothing written");
        assert_eq!(claim_of(&conn, "dev-a").await, None);

        claim_at(&conn, &key, "dev-b", now_unix_ms() + 3_600_000).await;
        let (p, _) = run_to_end(&conn, &key, &Store::open_in_memory().unwrap(), "dev-a", LONG).await;
        assert!(p.finished, "an hour ahead is a broken clock, not a claim");
        assert_tagged(&conn, &key).await;
    }

    #[tokio::test]
    async fn a_claim_exactly_600_s_old_is_ignored() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        claim_at(&conn, &key, "dev-b", now_unix_ms() - CLAIM_FRESH_MS).await;

        let first = kind_backfill_step(&conn, &key, &Store::open_in_memory().unwrap(), "dev-a", LONG).await.unwrap();

        assert!(first.finished);
        assert_tagged(&conn, &key).await;
    }

    #[test]
    fn claim_freshness_bounds() {
        assert!(claim_is_fresh(0));
        assert!(claim_is_fresh(CLAIM_FRESH_MS - 1));
        assert!(!claim_is_fresh(CLAIM_FRESH_MS));
        assert!(claim_is_fresh(-CLAIM_SKEW_MS));
        assert!(!claim_is_fresh(-CLAIM_SKEW_MS - 1));
        assert!(!claim_is_fresh(-3_600_000));
    }

    /// Once this process has seen the column, a step trusts the latch and runs no `ensure_kind_tag_column`: on a
    /// relay where that check would fail (and return unfinished at once), the step goes on to its relay reads.
    #[tokio::test]
    async fn a_step_trusts_the_column_latch() {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db")).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE VIEW sync_row AS SELECT 1 AS device_id", ()).await.unwrap();
        KIND_TAG_COLUMN_SEEN.set();

        let result = kind_backfill_step(&conn, &GroupKey::generate(), &Store::open_in_memory().unwrap(), "dev-a", LONG).await;

        KIND_TAG_COLUMN_SEEN.reset();
        assert!(result.is_err(), "no column check ran, so the walk's own read met the broken table: {result:?}");
    }

    /// Our own claim never holds us, however fresh.
    #[tokio::test]
    async fn our_own_claim_does_not_hold_the_step() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        claim_at(&conn, &key, "dev-a", now_unix_ms()).await;
        let (p, _) = run_to_end(&conn, &key, &Store::open_in_memory().unwrap(), "dev-a", LONG).await;
        assert!(p.finished);
        assert_tagged(&conn, &key).await;
    }

    // (f)
    #[tokio::test]
    async fn two_concurrent_backfills_that_missed_each_others_claim_tag_every_row_correctly() {
        TEST_PACE.with(|c| c.set(Some((3, 5, Duration::from_millis(1)))));
        TEST_IGNORE_CLAIMS.with(|c| c.set(true));
        let (_dir, db, conn_a, key) = relay().await;
        let conn_b = db.connect().unwrap();
        fixture(&conn_a, &key).await;
        let (store_a, store_b) = (Store::open_in_memory().unwrap(), Store::open_in_memory().unwrap());
        let null_items = count(&conn_a, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL").await;
        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));

        let mut first_round = true;
        for _ in 0..10_000 {
            let (a, b) = tokio::join!(
                kind_backfill_step(&conn_a, &key, &store_a, "dev-a", Duration::from_millis(5)),
                kind_backfill_step(&conn_b, &key, &store_b, "dev-b", Duration::from_millis(5)),
            );
            let (a, b) = (a.unwrap(), b.unwrap());
            if first_round {
                assert!(a.done > 0 && b.done > 0, "both ran: {a:?} {b:?}");
                first_round = false;
            }
            if a.finished && b.finished {
                assert_eq!(a.undecryptable + b.undecryptable, 0);
                // Row keys the UPDATEs named (each names its chunk's seqs after `?1`, `?2`).
                let written: usize = CAPTURED_SQL
                    .with(|c| c.borrow_mut().take().unwrap())
                    .iter()
                    .filter(|s| s.starts_with("UPDATE"))
                    .map(|s| s.matches('?').count() - 2)
                    .sum();
                assert!(written as i64 > null_items, "the two steps wrote some of the same rows: {written} vs {null_items}");
                assert_tagged(&conn_a, &key).await;
                return;
            }
        }
        panic!("the concurrent backfills never finished");
    }

    // (g)
    #[tokio::test]
    async fn every_kind_tag_is_null_statement_is_bounded_by_the_primary_key() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));

        run_to_end(&conn, &key, &Store::open_in_memory().unwrap(), "dev-a", Duration::ZERO).await;

        let sql = CAPTURED_SQL.with(|c| c.borrow_mut().take().unwrap());
        let null_scans: Vec<&String> = sql.iter().filter(|s| s.contains("kind_tag IS NULL")).collect();
        assert!(null_scans.iter().any(|s| s.starts_with("SELECT")), "{sql:?}");
        assert!(null_scans.iter().any(|s| s.starts_with("UPDATE")), "{sql:?}");
        for s in &null_scans {
            assert!(s.contains("device_id = ?"), "no device bound: {s}");
            let ranged = s.contains("local_seq > ?") && s.contains("local_seq <= ?");
            let keys = s.contains("local_seq IN (?");
            assert!(ranged || keys, "no local_seq bound: {s}");
        }
        // Nothing reads the whole table: every statement names a device.
        for s in &sql {
            assert!(s.contains("device_id"), "unbounded: {s}");
        }
    }

    /// One device with 1,200 `items` rows of four kinds (and meetings between them): three chunks of 500, each
    /// written with one UPDATE per kind it holds, never one per row. A row already tagged inside a chunk (here with a
    /// tag no kind produces, as if another device had tagged it) keeps its tag.
    #[tokio::test]
    async fn a_chunk_is_written_with_one_update_per_kind_and_tagged_rows_keep_their_tag() {
        const BIG_KINDS: [&str; 4] = ["capture", "transcript", "calendar_event", "note"];
        TEST_PACE.with(|c| c.set(Some((CHUNK_ROWS, SPAN, Duration::ZERO))));
        let (_dir, _db, conn, key) = relay().await;
        push_marker(&conn, &key, "dev-a", ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        let mut expected = Vec::new();
        let (mut seq, mut items) = (0i64, 0);
        while items < 1_200 {
            seq += 1;
            if seq % 13 == 0 {
                seed(&conn, &key, "dev-big", seq, ROW_TYPE_MEETING, "{}", None).await;
                continue;
            }
            items += 1;
            let kind = BIG_KINDS[seq as usize % BIG_KINDS.len()];
            let pre = (items % 97 == 0).then(|| "foreign-tag".to_string());
            seed(&conn, &key, "dev-big", seq, ROW_TYPE_ITEMS, &item_json(kind, &format!("row {seq}")), pre.clone()).await;
            expected.push((seq, pre.unwrap_or_else(|| sync::kind_tag(&key, kind))));
        }
        let null_items = count(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL").await;
        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));

        run_to_end(&conn, &key, &Store::open_in_memory().unwrap(), "dev-a", LONG).await;

        let sql = CAPTURED_SQL.with(|c| c.borrow_mut().take().unwrap());
        // Split into chunks: each chunk read, then its writes.
        let mut chunks: Vec<Vec<&String>> = Vec::new();
        for s in &sql {
            if s.starts_with("SELECT local_seq, ciphertext") {
                chunks.push(Vec::new());
            } else if s.starts_with("UPDATE") {
                chunks.last_mut().expect("an UPDATE follows a chunk read").push(s);
            }
        }
        let written: Vec<&Vec<&String>> = chunks.iter().filter(|c| !c.is_empty()).collect();
        assert!(written.len() >= 3, "1,200 rows take three chunks of 500: {}", written.len());
        for c in &written {
            assert!(c.len() <= BIG_KINDS.len(), "one UPDATE per kind in a chunk, not per row: {}", c.len());
        }
        let seqs_written: usize = written.iter().flat_map(|c| c.iter()).map(|s| s.matches('?').count() - 2).sum();
        assert_eq!(seqs_written as i64, null_items, "every NULL row written once, no other");

        for (seq, tag) in expected {
            let mut rows = conn
                .query("SELECT kind_tag FROM sync_row WHERE device_id = 'dev-big' AND local_seq = ?1", params![seq])
                .await
                .unwrap();
            let got: Option<String> = rows.next().await.unwrap().unwrap().get(0).unwrap();
            assert_eq!(got.as_deref(), Some(tag.as_str()), "row {seq}");
        }
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'meeting' AND kind_tag IS NOT NULL").await, 0);
    }

    /// A row tagged by someone else after the chunk was read (NULL when read) keeps that tag: the UPDATE's own
    /// `kind_tag IS NULL` guard, not only the read's filter.
    #[tokio::test]
    async fn a_row_tagged_between_the_read_and_the_write_keeps_its_tag() {
        small_pace();
        let (_dir, db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        let other = db.connect().unwrap();
        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));
        let store = Store::open_in_memory().unwrap();

        let racer = async {
            // Wait for the step's first chunk read (it yields right after), then tag a row of that chunk.
            while !CAPTURED_SQL.with(|c| c.borrow().as_ref().unwrap().iter().any(|s| s.starts_with("SELECT local_seq, ciphertext"))) {
                tokio::task::yield_now().await;
            }
            other
                .execute("UPDATE sync_row SET kind_tag = 'foreign-tag' WHERE device_id = 'dev-a' AND local_seq = 2", ())
                .await
                .unwrap();
        };
        let (first, ()) = tokio::join!(kind_backfill_step(&conn, &key, &store, "dev-a", Duration::ZERO), racer);
        first.unwrap();
        run_to_end(&conn, &key, &store, "dev-a", LONG).await;

        let mut rows = conn.query("SELECT kind_tag FROM sync_row WHERE device_id = 'dev-a' AND local_seq = 2", ()).await.unwrap();
        let tag: Option<String> = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(tag.as_deref(), Some("foreign-tag"));
        let mut rows = conn.query("SELECT kind_tag FROM sync_row WHERE device_id = 'dev-a' AND local_seq = 1", ()).await.unwrap();
        let tag: Option<String> = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(tag, Some(sync::kind_tag(&key, kind_of(0, 1))), "its chunk-mate is tagged");
    }

    // (h)
    #[tokio::test]
    async fn finished_is_persisted_and_later_steps_read_nothing() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();
        let (p, _) = run_to_end(&conn, &key, &store, "dev-a", LONG).await;
        assert!(store.get_meta(KIND_BACKFILL_DONE_KEY).unwrap().is_some());

        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));
        conn.execute("DROP TABLE sync_row", ()).await.unwrap();
        let later = kind_backfill_step(&conn, &key, &store, "dev-a", LONG).await.unwrap();

        assert_eq!(later, p);
        assert!(later.finished);
        assert_eq!(CAPTURED_SQL.with(|c| c.borrow_mut().take().unwrap()), Vec::<String>::new());
    }

    /// A done marker of another backfill version walks again.
    #[tokio::test]
    async fn a_done_marker_of_another_version_does_not_count() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();
        store.set_meta(KIND_BACKFILL_DONE_KEY, "0:5:0").unwrap();

        let (p, _) = run_to_end(&conn, &key, &store, "dev-a", LONG).await;

        assert_eq!(p.total, 3 * ROWS_PER_DEVICE as u64);
        assert_tagged(&conn, &key).await;
    }

    #[tokio::test]
    async fn a_relay_whose_kind_tag_column_cannot_be_confirmed_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db")).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE VIEW sync_row AS SELECT 1 AS device_id", ()).await.unwrap();
        KIND_TAG_COLUMN_SEEN.reset();
        let store = Store::open_in_memory().unwrap();

        let p = kind_backfill_step(&conn, &GroupKey::generate(), &store, "dev-a", LONG).await.unwrap();

        assert!(!p.finished);
        assert_eq!(store.get_meta(KIND_BACKFILL_DONE_KEY).unwrap(), None);
    }

    /// A device that has not announced itself on the relay yet does not claim: a claim row alone would make it a
    /// peer without capability markers, and hold every other device's marker gates.
    #[tokio::test]
    async fn a_device_without_its_capability_marker_neither_claims_nor_tags() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        let before = snapshot(&conn).await;

        let p = kind_backfill_step(&conn, &key, &Store::open_in_memory().unwrap(), "dev-new", LONG).await.unwrap();

        assert!(!p.finished);
        assert_eq!(snapshot(&conn).await, before);
    }

    // ---- Task 5: the sync round runs it, Settings reads it ----

    const OWN: &str = "dev-own";

    /// Like [`fixture`] but only rows a pull can apply (real `items` rows, no placeholder meetings).
    async fn round_fixture(conn: &Connection, key: &GroupKey) {
        for (d, device) in DEVICES.iter().enumerate() {
            push_marker(conn, key, device, ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
            for seq in 1..=ROWS_PER_DEVICE {
                let kind = kind_of(d, seq);
                let tag = (seq == ROWS_PER_DEVICE - 1).then(|| sync::kind_tag(key, kind));
                seed(conn, key, device, seq, ROW_TYPE_ITEMS, &item_json(kind, &format!("{device} {seq}")), tag).await;
            }
        }
    }

    /// Also pins "nothing waiting to pull" for this test's thread: the real flag is process-wide, and another
    /// test's pull can leave it true.
    fn round_budget(b: Option<Duration>) {
        crate::TEST_BACKFILL_BUDGET.with(|c| c.set(b));
        crate::TEST_MORE_TO_PULL.with(|c| c.set(Some(false)));
    }

    async fn desktop_round(conn: &Connection, key: &GroupKey, store: &Store) {
        crate::content_round(conn, key, store, OWN, &|_| {}).await.unwrap();
    }

    fn backfill_keys(store: &Store) -> Vec<String> {
        store.list_meta_with_prefix("kind_backfill").unwrap().into_iter().map(|(k, _)| k).collect()
    }

    #[tokio::test]
    async fn percent_is_none_without_progress_and_capped_below_100_while_unfinished() {
        let _relay = relay().await; // libsql before rusqlite, see outbox.rs
        let store = Store::open_in_memory().unwrap();
        assert_eq!(kind_backfill_percent(&store), None, "nothing ran yet");
        record_progress(&store, &KindBackfillProgress { done: 61, total: 100, finished: false, undecryptable: 0 }).unwrap();
        assert_eq!(kind_backfill_percent(&store), Some(61));
        record_progress(&store, &KindBackfillProgress { done: 1999, total: 2000, finished: false, undecryptable: 0 }).unwrap();
        assert_eq!(kind_backfill_percent(&store), Some(99), "never 100 until finished");
        record_progress(&store, &KindBackfillProgress { done: 7, total: 5, finished: false, undecryptable: 0 }).unwrap();
        assert_eq!(kind_backfill_percent(&store), Some(99));
        // The step could not confirm the column, or the relay is empty: 0/0 unfinished keeps what was known.
        record_progress(&store, &KindBackfillProgress::default()).unwrap();
        assert_eq!(kind_backfill_percent(&store), Some(99), "0/0 is not progress and does not erase it");
        record_progress(&store, &KindBackfillProgress { done: 0, total: 50, finished: false, undecryptable: 0 }).unwrap();
        assert_eq!(kind_backfill_percent(&store), Some(99), "nothing walked is not progress either");
        record_progress(&store, &KindBackfillProgress { done: 100, total: 100, finished: true, undecryptable: 0 }).unwrap();
        assert_eq!(kind_backfill_percent(&store), None, "finished shows nothing");
    }

    #[tokio::test]
    async fn percent_is_none_for_an_empty_relay() {
        let _relay = relay().await; // libsql before rusqlite, see outbox.rs
        let store = Store::open_in_memory().unwrap();
        record_progress(&store, &KindBackfillProgress::default()).unwrap();
        assert_eq!(kind_backfill_percent(&store), None, "0/0 is not shown");
        record_progress(&store, &KindBackfillProgress { done: 0, total: 50, finished: false, undecryptable: 0 }).unwrap();
        assert_eq!(kind_backfill_percent(&store), None, "0 of 50 is not shown: this device is not indexing");
        record_progress(&store, &KindBackfillProgress { done: 0, total: 0, finished: true, undecryptable: 0 }).unwrap();
        assert_eq!(kind_backfill_percent(&store), None);
    }

    #[test]
    fn a_round_leaves_room_for_the_step_before_the_sync_timeout() {
        let budget = crate::KIND_BACKFILL_BUDGET;
        assert_eq!(budget, Duration::from_secs(2));
        assert!(crate::kind_backfill_fits(Duration::ZERO, budget));
        assert!(crate::kind_backfill_fits(Duration::from_secs(23), budget));
        assert!(!crate::kind_backfill_fits(Duration::from_secs(24), budget), "2 + 3 + 2 s spare must fit in 30 s");
        assert!(!crate::kind_backfill_fits(Duration::from_secs(29), budget));
    }

    /// A desktop round runs one bounded step after the pull, reports progress, and after enough rounds the
    /// backfill is finished, every row tagged, and the percentage gone.
    #[tokio::test]
    async fn a_desktop_round_runs_a_bounded_step_and_reports_progress() {
        small_pace();
        round_budget(Some(Duration::ZERO));
        let (_dir, _db, conn, key) = relay().await;
        round_fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();
        let null_before = count(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL").await;

        desktop_round(&conn, &key, &store).await;
        let null_after = count(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL").await;
        assert!(null_after < null_before, "the first round tagged something");
        assert!(null_after > 0, "a zero-budget step stops after one chunk");
        let p = kind_backfill_percent(&store).expect("a percentage while unfinished");
        assert!(p < 100);

        let mut rounds = 1;
        while kind_backfill_percent(&store).is_some() {
            desktop_round(&conn, &key, &store).await;
            rounds += 1;
            assert!(rounds < 500, "the backfill never finished");
        }
        assert!(store.get_meta(KIND_BACKFILL_DONE_KEY).unwrap().is_some(), "finished is persisted");
        assert!(rounds > 1);
        assert_tagged(&conn, &key).await;
        // Finished: another round does not touch the relay for it.
        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));
        desktop_round(&conn, &key, &store).await;
        let sql = CAPTURED_SQL.with(|c| c.borrow_mut().take().unwrap());
        assert!(sql.is_empty(), "{sql:?}");
        round_budget(None);
    }

    /// The step never fails the round: a relay it cannot read leaves the round Ok.
    #[tokio::test]
    async fn a_failing_step_does_not_fail_the_round() {
        round_budget(Some(Duration::ZERO));
        let (_dir, _db, conn, key) = relay().await;
        let store = Store::open_in_memory().unwrap();
        // No relay table at all: the step errors (logged), the call returns, and nothing is recorded.
        conn.execute("DROP TABLE sync_row", ()).await.unwrap();
        crate::run_kind_backfill(&conn, &key, &store, OWN, std::time::Instant::now()).await;
        assert!(backfill_keys(&store).is_empty(), "{:?}", backfill_keys(&store));
        assert_eq!(kind_backfill_percent(&store), None);
        assert!(store.get_meta(KIND_BACKFILL_DONE_KEY).unwrap().is_none());
        round_budget(None);
    }

    /// Another device's fresh claim holds the step: nothing is tagged, and Settings shows no "0%".
    #[tokio::test]
    async fn a_round_held_by_another_devices_claim_shows_no_percentage() {
        small_pace();
        round_budget(Some(Duration::from_secs(5)));
        let (_dir, _db, conn, key) = relay().await;
        round_fixture(&conn, &key).await;
        refresh_marker(&conn, &key, "dev-a", ROW_TYPE_KIND_BACKFILL_CLAIM).await.unwrap();
        let store = Store::open_in_memory().unwrap();
        let null_before = count_untagged(&conn).await;

        desktop_round(&conn, &key, &store).await;

        assert_eq!(count_untagged(&conn).await, null_before, "the claim held the step");
        assert_eq!(kind_backfill_percent(&store), None);
        assert!(backfill_keys(&store).iter().all(|k| k != "kind_backfill_progress"), "{:?}", backfill_keys(&store));
        round_budget(None);
    }

    /// A round whose own pull left rows waiting does not run the step.
    #[tokio::test]
    async fn a_round_with_more_to_pull_skips_the_step() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        round_fixture(&conn, &key).await;
        push_marker(&conn, &key, OWN, ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        let store = Store::open_in_memory().unwrap();
        let before = snapshot(&conn).await;
        crate::TEST_MORE_TO_PULL.with(|c| c.set(Some(true)));
        crate::run_kind_backfill(&conn, &key, &store, OWN, std::time::Instant::now()).await;
        assert_eq!(snapshot(&conn).await, before);
        assert!(backfill_keys(&store).is_empty());
        crate::TEST_MORE_TO_PULL.with(|c| c.set(Some(false)));
        crate::run_kind_backfill(&conn, &key, &store, OWN, std::time::Instant::now()).await;
        assert_ne!(snapshot(&conn).await, before, "nothing waiting: it runs");
        crate::TEST_MORE_TO_PULL.with(|c| c.set(None));
    }

    /// A whole round whose clock started 25 s ago (a slow connect) skips the step.
    #[tokio::test]
    async fn a_round_that_started_long_ago_skips_the_step() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        round_fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();
        let before = count_untagged(&conn).await;
        let late = std::time::Instant::now().checked_sub(Duration::from_secs(25)).unwrap();
        crate::content_round_from(&conn, &key, &store, OWN, &|_| {}, late).await.unwrap();
        assert_eq!(count_untagged(&conn).await, before);
        assert!(backfill_keys(&store).is_empty());
    }

    /// A round already deep into its timeout does not start a step.
    #[tokio::test]
    async fn a_late_round_skips_the_step() {
        small_pace();
        round_budget(None);
        let (_dir, _db, conn, key) = relay().await;
        round_fixture(&conn, &key).await;
        push_marker(&conn, &key, OWN, ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        let store = Store::open_in_memory().unwrap();
        let before = snapshot(&conn).await;
        let late = std::time::Instant::now().checked_sub(Duration::from_secs(25)).unwrap();
        crate::run_kind_backfill(&conn, &key, &store, OWN, late).await;
        assert_eq!(snapshot(&conn).await, before);
        assert!(backfill_keys(&store).is_empty());
        // On time it runs.
        crate::run_kind_backfill(&conn, &key, &store, OWN, std::time::Instant::now()).await;
        assert_ne!(snapshot(&conn).await, before);
    }

    /// The phone pulls only, and the cloud runner has its own entry points: neither runs the backfill, so a
    /// relay full of untagged rows is left exactly as it was and no progress is kept.
    #[tokio::test]
    async fn phone_and_runner_rounds_never_run_it() {
        small_pace();
        round_budget(Some(Duration::from_secs(5)));
        let (_dir, _db, conn, key) = relay().await;
        round_fixture(&conn, &key).await;

        let phone = Store::open_in_memory().unwrap();
        crate::pull_only_with_connection(&conn, &key, &phone, "phone-1", &crate::PullOptions::default()).await.unwrap();
        assert!(backfill_keys(&phone).is_empty(), "{:?}", backfill_keys(&phone));
        let after_phone = count_untagged(&conn).await;
        assert!(after_phone > 0, "the phone round left the rows untagged");

        let runner = Store::open_in_memory().unwrap();
        crate::runner_pull_with_connection(&conn, &key, &runner, "runner-1", &crate::PullOptions::default()).await.unwrap();
        crate::runner_dispatch_with_connection(&conn, &key, &runner, "runner-1").await.unwrap();
        assert!(backfill_keys(&runner).is_empty(), "{:?}", backfill_keys(&runner));
        assert_eq!(count_untagged(&conn).await, after_phone, "no runner round tagged anything");
        assert_eq!(count(&conn, &format!("SELECT COUNT(*) FROM sync_row WHERE row_type = '{ROW_TYPE_KIND_BACKFILL_CLAIM}'")).await, 0);

        // The desktop round on the same relay does.
        let desktop = Store::open_in_memory().unwrap();
        desktop_round(&conn, &key, &desktop).await;
        assert!(count_untagged(&conn).await < after_phone);
        round_budget(None);
    }

    async fn count_untagged(conn: &Connection) -> i64 {
        count(conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL").await
    }
    // ---- The cloud runner's one-off, long run ----

    const RUNNER: &str = "dev-runner";

    /// The fixture plus the runner's own `items` rows (NULL), as a runner that has pushed before has.
    async fn runner_fixture(conn: &Connection, key: &GroupKey) {
        fixture(conn, key).await;
        for seq in 1..=10 {
            seed(conn, key, RUNNER, seq, ROW_TYPE_ITEMS, &item_json("capture", &format!("runner {seq}")), None).await;
        }
    }

    async fn runner_run(
        conn: &Connection,
        key: &GroupKey,
        store: &Store,
        total: Duration,
        step: Duration,
    ) -> (KindBackfillProgress, usize) {
        let mut steps = 0;
        let (p, _) = crate::runner_kind_backfill_observed_with_connection(conn, key, store, RUNNER, total, step, &mut |_, n| steps = n)
            .await
            .unwrap();
        (p, steps)
    }

    // (a)
    #[tokio::test]
    async fn the_runner_backfill_tags_every_row_of_other_devices_and_its_own() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();

        let p = crate::runner_kind_backfill_with_connection(&conn, &key, &store, RUNNER, LONG, LONG).await.unwrap();

        assert!(p.finished);
        assert_eq!(p.undecryptable, 0);
        assert_eq!(p.total, (3 * ROWS_PER_DEVICE + 10) as u64);
        assert_eq!(p.done, p.total);
        assert_eq!(count_untagged(&conn).await, 0);
        // Meetings stay untagged; the runner's rows carry the capture tag.
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'meeting' AND kind_tag IS NOT NULL").await, 0);
        let capture = sync::kind_tag(&key, "capture");
        assert_eq!(
            count(&conn, &format!("SELECT COUNT(*) FROM sync_row WHERE device_id = '{RUNNER}' AND row_type = 'items' AND kind_tag = '{capture}'")).await,
            10
        );
    }

    // (b)
    #[tokio::test]
    async fn a_fresh_claim_by_another_device_stops_the_runner_backfill_without_looping() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        claim_at(&conn, &key, "dev-b", now_unix_ms() - 60_000).await;
        let store = Store::open_in_memory().unwrap();
        let untagged = count_untagged(&conn).await;

        let (p, steps) = runner_run(&conn, &key, &store, LONG, LONG).await;

        assert!(!p.finished);
        assert_eq!(p.done, 0);
        assert_eq!(steps, 2, "two steps without progress, then it stops");
        assert_eq!(count_untagged(&conn).await, untagged);
        assert_eq!(claim_of(&conn, RUNNER).await, None, "it never took the claim");
    }

    // (c)
    #[tokio::test]
    async fn a_spent_total_budget_returns_unfinished_with_progress_and_a_second_call_completes() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;

        let (p, _) = runner_run(&conn, &key, &Store::open_in_memory().unwrap(), Duration::ZERO, Duration::ZERO).await;
        assert!(!p.finished);
        assert!(p.done > 0 && p.done < p.total, "one step moved, not all: {p:?}");
        let left = count_untagged(&conn).await;
        assert!(left > 0);

        // A new store (a new process): it walks again, skipping the rows already tagged.
        let second = crate::runner_kind_backfill_with_connection(&conn, &key, &Store::open_in_memory().unwrap(), RUNNER, LONG, LONG)
            .await
            .unwrap();
        assert!(second.finished);
        assert_eq!(count_untagged(&conn).await, 0);
    }

    // (d)
    #[tokio::test]
    async fn the_runners_marker_is_pushed_once_and_a_second_call_does_not_duplicate_it() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        let markers = format!("SELECT COUNT(*) FROM sync_row WHERE device_id = '{RUNNER}' AND row_type = '{ROW_TYPE_ITEMS_CAPABILITY}'");
        let stamp = format!("SELECT created_at_unix_ms FROM sync_row WHERE device_id = '{RUNNER}' AND row_type = '{ROW_TYPE_ITEMS_CAPABILITY}'");
        assert_eq!(count(&conn, &markers).await, 0);

        let _ = runner_run(&conn, &key, &Store::open_in_memory().unwrap(), Duration::ZERO, Duration::ZERO).await;
        assert_eq!(count(&conn, &markers).await, 1);
        let all_five = format!("SELECT COUNT(*) FROM sync_row WHERE device_id = '{RUNNER}' AND local_seq = 0 AND row_type IN ('items_capability', 'topics_capability', 'topic_page_capability', 'topic_snapshot_capability', 'client_controls_capability')");
        assert_eq!(count(&conn, &all_five).await, 5, "all five pull-only markers are announced");
        let first = count(&conn, &stamp).await;

        crate::runner_kind_backfill_with_connection(&conn, &key, &Store::open_in_memory().unwrap(), RUNNER, LONG, LONG).await.unwrap();
        assert_eq!(count(&conn, &markers).await, 1);
        // The unchanged timestamp is what proves DO NOTHING (a DO UPDATE would move it to now).
        assert_eq!(count(&conn, &stamp).await, first, "an existing marker is left as it is");
    }

    #[tokio::test]
    async fn a_step_that_fails_twice_then_succeeds_still_finishes() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        TEST_FAIL_STEPS.with(|c| c.set(2));
        let p = crate::runner_kind_backfill_with_connection(&conn, &key, &Store::open_in_memory().unwrap(), RUNNER, LONG, LONG).await.unwrap();
        assert!(p.finished);
        assert_eq!(count_untagged(&conn).await, 0);
    }

    #[tokio::test]
    async fn a_fourth_consecutive_failure_returns_the_error() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        TEST_FAIL_STEPS.with(|c| c.set(4));
        let r = crate::runner_kind_backfill_with_connection(&conn, &key, &Store::open_in_memory().unwrap(), RUNNER, LONG, LONG).await;
        assert!(r.is_err());
        TEST_FAIL_STEPS.with(|c| c.set(0));
    }

    #[tokio::test]
    async fn a_success_resets_the_failure_count() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();
        // 3 failures, a success, then 3 more would exceed 3 only if the count were not reset.
        let mut seen = 0;
        TEST_FAIL_STEPS.with(|c| c.set(3));
        let r = crate::runner_kind_backfill_observed_with_connection(&conn, &key, &store, RUNNER, LONG, Duration::ZERO, &mut |_, _| {
            seen += 1;
            if seen == 1 {
                TEST_FAIL_STEPS.with(|c| c.set(3));
            }
        })
        .await;
        TEST_FAIL_STEPS.with(|c| c.set(0));
        assert!(r.unwrap().0.finished);
    }

    // I2
    #[tokio::test]
    async fn a_zero_step_budget_still_walks_to_the_end_one_chunk_per_step() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        let (p, steps) = runner_run(&conn, &key, &Store::open_in_memory().unwrap(), LONG, Duration::ZERO).await;
        assert!(p.finished);
        assert!(steps > 2, "steps {steps}");
        assert_eq!(count_untagged(&conn).await, 0);
    }

    #[tokio::test]
    async fn the_stop_reason_says_finished_budget_or_claim_held() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        let budget = crate::runner_kind_backfill_observed_with_connection(&conn, &key, &Store::open_in_memory().unwrap(), RUNNER, Duration::ZERO, Duration::ZERO, &mut |_, _| {})
            .await
            .unwrap()
            .1;
        assert_eq!(budget, BackfillStop::Budget);
        let finished = crate::runner_kind_backfill_observed_with_connection(&conn, &key, &Store::open_in_memory().unwrap(), RUNNER, LONG, Duration::ZERO, &mut |_, _| {})
            .await
            .unwrap()
            .1;
        assert_eq!(finished, BackfillStop::Finished);

    }

    #[tokio::test]
    async fn a_held_claim_reports_claim_held() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        claim_at(&conn, &key, "dev-b", now_unix_ms() - 60_000).await;
        let mut steps = 0;
        let r = crate::runner_kind_backfill_observed_with_connection(&conn, &key, &Store::open_in_memory().unwrap(), RUNNER, LONG, LONG, &mut |_, n| steps = n)
            .await
            .unwrap();
        assert_eq!(r.1, BackfillStop::ClaimHeld);
        assert_eq!(steps, 2);
    }

    #[tokio::test]
    async fn no_step_starts_with_under_10_s_of_the_total_budget_left() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        let mut steps = 0;
        let (p, stop) = crate::runner_kind_backfill_observed_with_connection(
            &conn, &key, &Store::open_in_memory().unwrap(), RUNNER, Duration::from_secs(5), Duration::ZERO, &mut |_, n| steps = n,
        )
        .await
        .unwrap();
        assert!(!p.finished);
        assert_eq!(steps, 1);
        assert_eq!(stop, BackfillStop::Budget);
    }
}
