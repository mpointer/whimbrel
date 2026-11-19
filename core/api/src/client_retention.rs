//! "Keep this client's material for N months" (X6 client controls, spec section 5).
//!
//! A client (seed topic) with `keep_months` set has its linked material checked by age, **per item**,
//! from the item's own date: a meeting's end, an email's date, a calendar event's date, a screen
//! activity episode's last capture. Nothing is ever deleted unasked:
//! - [`due_items`] lists what has passed the period (read-only). Items the user said to keep are left
//!   out for good; snoozed ones until the snooze ends ([`set_retention_decision`]).
//! - The scheduler that runs global retention calls [`refresh_review_flag`], which at most once a week
//!   raises a count-only "N items ready to review" flag ([`review_flag`]); the UIs show the section 4
//!   review screen over [`due_items`].
//! - Only the user's confirmation runs [`expire_items`], which deletes exactly those items that are
//!   still due, **on this device only** (spec 4.3: "Retention remains per device and stays local"):
//!   no tombstone, no forgotten record, no relay clean-up (memory's `expire_*` entry points). The client's
//!   own record (seed, name, aliases, settings) is never deleted here.
//!
//! The client's period overrides global retention for its material (`memory::Store::apply_retention`
//! leaves it alone); `keep_months: None` means global retention applies and nothing is due here.
//!
//! Nothing here holds or stores a client's name: decisions are kept under an opaque prefix
//! (`Store::client_retention_prefix`), the review flag and the stored receipt are counts only.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Months};
use serde::{Deserialize, Serialize};

use crate::client_delete::{bump, DeleteStage};
use crate::client_inventory::{ClientItem, ItemKind, LeftForYou};
use crate::envelope::{ApiError, ApiResult};
use memory::ClientItemKind;

/// How often the review prompt may come back after it was shown or set aside (spec 5: weekly).
pub const PROMPT_INTERVAL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// "Snooze 30 days".
pub const SNOOZE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// kv: the count-only review flag the UIs read.
const FLAG_KEY: &str = "client_retention_review";
/// kv: when the prompt was last raised or set aside.
const PROMPTED_KEY: &str = "client_retention_prompted_at";
/// kv prefix of the stored expiry receipts (counts only).
pub const EXPIRY_RECEIPT_PREFIX: &str = "client_expiry_receipt:";

/// One client's items past its period.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DueClient {
    pub key: String,
    pub items: Vec<ClientItem>,
}

/// What the user chose for an item on the review screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// "Keep these": never ask again for this item.
    Keep,
    /// "Snooze 30 days" (or any time): not due again before `until_ms`.
    Snooze { until_ms: i64 },
}

/// The weekly "items ready to review" flag. Counts only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFlag {
    pub clients: usize,
    pub items: usize,
    pub raised_at_unix_ms: i64,
}

/// What the user confirmed on the review screen for one client.
#[derive(Debug, Clone, Default)]
pub struct ExpirySelection {
    pub key: String,
    /// The items to delete: those [`due_items`] listed. Anything not due when the delete runs is left.
    pub items: Vec<ClientItem>,
    /// The Obsidian vault folder, when the user has one and vault sync is on: notes of deleted meetings go too.
    pub vault: Option<PathBuf>,
    /// Vault sync is off: the vault is never touched (even if `vault` is set), and "left for you" says so.
    pub vault_off: bool,
}

/// After an expiry. Counts only: no name, no key, no text, no path in the stored copy.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExpiryReceipt {
    pub date_unix_ms: i64,
    /// Deleted on this device, per kind. Other devices keep their copies (retention is per device).
    pub counts: Vec<(ItemKind, usize)>,
    /// Not deleted: no longer due (the period changed, a newer item, kept, snoozed, shared with a client
    /// that keeps it longer) or not a client item. They stay.
    pub not_due: Vec<(ItemKind, usize)>,
    /// Not deleted yet: this device has not sent them to the other devices. A later run takes them.
    pub waiting_to_sync: Vec<(ItemKind, usize)>,
    /// The run stopped at its time limit; the rest is still due and the next run continues.
    pub more: bool,
    /// Kinds with an item that could not be deleted; still here.
    pub failed: Vec<ItemKind>,
    pub failed_stages: Vec<DeleteStage>,
    /// Always includes `CopiesOnOtherDevices`: an expiry never reaches other devices.
    pub left_for_you: Vec<LeftForYou>,
}

impl ExpiryReceipt {
    /// Everything confirmed and due was deleted here.
    pub fn complete(&self) -> bool {
        self.failed.is_empty() && self.failed_stages.is_empty() && !self.more && self.waiting_to_sync.is_empty()
    }
}

fn db(_: rusqlite::Error) -> ApiError {
    ApiError::ProviderFailure
}

/// The instant `months` calendar months before `now_ms` (UTC). `None` for 0 months or a date out of
/// range: nothing is due then, never everything.
pub fn cutoff_ms(now_ms: i64, months: u32) -> Option<i64> {
    if months == 0 {
        return None;
    }
    DateTime::from_timestamp_millis(now_ms)?
        .checked_sub_months(Months::new(months))
        .map(|d| d.timestamp_millis())
}

fn decision_key(key: &str, kind: ItemKind, id: &str) -> String {
    let kind = serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    format!("{}{kind}:{id}", memory::Store::client_retention_prefix(key))
}

/// Records the user's choice for `item` of client `key`, on this device only (kv, never synced).
pub fn set_retention_decision(store: &memory::Store, key: &str, item: &ClientItem, d: Decision) -> ApiResult<()> {
    set_retention_decision_at(store, key, item, d, crate::util::now_unix_ms())
}

/// [`set_retention_decision`] at `now_ms`, which also brings a standing review flag up to date.
pub fn set_retention_decision_at(
    store: &memory::Store,
    key: &str,
    item: &ClientItem,
    d: Decision,
    now_ms: i64,
) -> ApiResult<()> {
    live_client(store, key)?;
    let json = serde_json::to_string(&d).map_err(|_| ApiError::ProviderFailure)?;
    store.set_meta(&decision_key(key, item.kind, &item.id), &json).map_err(db)?;
    // The flag is a hint: failing to update it never fails the decision.
    let _ = update_standing_flag(store, now_ms);
    Ok(())
}

fn decisions(store: &memory::Store, key: &str) -> ApiResult<HashMap<String, Decision>> {
    let prefix = memory::Store::client_retention_prefix(key);
    let mut out = HashMap::new();
    for (k, v) in store.list_meta_with_prefix(&prefix).map_err(db)? {
        // An unreadable decision is no decision: the item is asked about again, never deleted unasked.
        if let Ok(d) = serde_json::from_str::<Decision>(&v) {
            out.insert(k, d);
        }
    }
    Ok(out)
}

fn live_client(store: &memory::Store, key: &str) -> ApiResult<()> {
    match store.list_seed_topics().map_err(db)?.iter().any(|s| s.key == key) {
        true => Ok(()),
        false => Err(ApiError::NotFound),
    }
}

/// A linked item with the source it is filed as (for who else holds it) and its age date.
struct Candidate {
    item: ClientItem,
    /// `(topic_source.kind, ref_id)` deciding who else holds it; captures take their episode's.
    source: (&'static str, i64),
    /// The item's own date; `None` when undated (never due).
    date: Option<i64>,
}

/// Client `key`'s linked material with the dates its age is counted from. Steps are not listed: they go
/// with the email, event, episode or meeting they were filed from.
fn candidates(store: &memory::Store, key: &str) -> ApiResult<Vec<Candidate>> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: HashSet<(ItemKind, String)> = HashSet::new();
    let dated = |d: i64| (d > 0).then_some(d);
    let item = |kind, id: String, date: i64| ClientItem { kind, id, also_client: None, date_unix_ms: date, removed: false, label: None, excerpt: None, app: None };
    for m in store.client_members(key).map_err(db)? {
        let Some(r) = m.ref_id else { continue };
        let (kind, id, source) = match m.kind.as_str() {
            "meeting" => (ItemKind::Meeting, r, ("meeting", r)),
            "snapshot" => match store.snapshot_meeting(r).map_err(db)? {
                Some((meeting, _)) => (ItemKind::Meeting, meeting, ("meeting", meeting)),
                None => continue,
            },
            "email" => (ItemKind::Email, r, ("email", r)),
            "calendar_event" => (ItemKind::CalendarEvent, r, ("calendar_event", r)),
            "episode" => (ItemKind::Episode, r, ("episode", r)),
            _ => continue,
        };
        if !seen.insert((kind, id.to_string())) {
            continue;
        }
        match kind {
            ItemKind::Meeting => {
                // Its end (spec 5); a meeting still recording is never due.
                let Some(meeting) = store.get_meeting(id).map_err(db)? else { continue };
                if meeting.status == memory::MeetingStatus::Recording {
                    continue;
                }
                let date = dated(meeting.end_unix_ms.unwrap_or(meeting.start_unix_ms));
                out.push(Candidate { item: item(kind, id.to_string(), date.unwrap_or(0)), source, date });
            }
            ItemKind::Episode => {
                // Its last capture: the episode, with every capture under it, is due once that one is.
                let captures = store.episode_captures(id).map_err(db)?;
                let last = captures.iter().map(|c| c.2).max().unwrap_or(m.ts_unix_ms).max(m.ts_unix_ms);
                // One item with its captures: one decision, one confirmation (they go together).
                let date = dated(last);
                out.push(Candidate { item: item(kind, id.to_string(), last), source, date });
            }
            _ => {
                let date = dated(m.ts_unix_ms);
                out.push(Candidate { item: item(kind, id.to_string(), m.ts_unix_ms), source, date });
            }
        }
    }
    Ok(out)
}

/// Client `key`'s items past its period at `now_ms`, oldest first. `periods` maps every client with a
/// keep period to it; an item another client holds is due only under the longest period of the clients
/// holding it, and never while a client without a period (keep forever) holds it.
fn due_for(
    store: &memory::Store,
    key: &str,
    months: u32,
    periods: &HashMap<String, u32>,
    now_ms: i64,
) -> ApiResult<Vec<ClientItem>> {
    if cutoff_ms(now_ms, months).is_none() {
        return Ok(Vec::new());
    }
    let decided = decisions(store, key)?;
    let mut holders: HashMap<(&'static str, i64), Option<u32>> = HashMap::new();
    let mut out = Vec::new();
    for c in candidates(store, key)? {
        let Some(date) = c.date else { continue };
        let longest = match holders.get(&c.source) {
            Some(l) => *l,
            None => {
                let mut longest = Some(months);
                for other in store.source_clients(c.source.0, c.source.1).map_err(db)? {
                    longest = match (longest, periods.get(&other)) {
                        (Some(a), Some(b)) => Some(a.max(*b)),
                        _ => None,
                    };
                }
                holders.insert(c.source, longest);
                longest
            }
        };
        let Some(cutoff) = longest.and_then(|m| cutoff_ms(now_ms, m)) else { continue };
        if date >= cutoff {
            continue;
        }
        match decided.get(&decision_key(key, c.item.kind, &c.item.id)) {
            Some(Decision::Keep) => continue,
            Some(Decision::Snooze { until_ms }) if *until_ms > now_ms => continue,
            _ => {}
        }
        out.push(c.item);
    }
    out.sort_by(|a, b| a.date_unix_ms.cmp(&b.date_unix_ms).then(a.id.cmp(&b.id)));
    // Display only: a short name for each row on the review screen (never in the expiry receipt).
    for item in out.iter_mut() {
        item.label = crate::client_inventory::item_label(store, item.kind, &item.id);
    }
    Ok(out)
}

fn periods(store: &memory::Store) -> ApiResult<HashMap<String, u32>> {
    // Unreadable settings: an error, so nothing is listed and nothing is deleted.
    Ok(store.clients_with_keep_period().map_err(db)?.into_iter().collect())
}

/// Every client's items past its keep period at `now_ms` (see the module docs). Clients with no period,
/// or with nothing due, are left out. Read-only: the store is unchanged.
pub fn due_items(store: &memory::Store, now_ms: i64) -> ApiResult<Vec<DueClient>> {
    let periods = periods(store)?;
    let mut keys: Vec<(&String, &u32)> = periods.iter().collect();
    keys.sort();
    let mut out = Vec::new();
    for (key, months) in keys {
        let items = due_for(store, key, *months, &periods, now_ms)?;
        if !items.is_empty() {
            out.push(DueClient { key: key.clone(), items });
        }
    }
    Ok(out)
}

/// [`due_items`] for client `key` alone (its review screen). Empty when it has no period.
/// `Err(NotFound)` when `key` is not a live client.
pub fn due_items_for(store: &memory::Store, key: &str, now_ms: i64) -> ApiResult<Vec<ClientItem>> {
    live_client(store, key)?;
    let periods = periods(store)?;
    match periods.get(key) {
        Some(months) => due_for(store, key, *months, &periods, now_ms),
        None => Ok(Vec::new()),
    }
}

/// The scheduler's weekly check, run with global retention: raises the count-only review flag when
/// items are due and the prompt has not been shown or set aside in the last week; clears it when nothing
/// is due. Never deletes. Returns the flag now standing.
pub fn refresh_review_flag(store: &memory::Store, now_ms: i64) -> ApiResult<Option<ReviewFlag>> {
    let due = due_items(store, now_ms)?;
    let items: usize = due.iter().map(|c| c.items.len()).sum();
    if items == 0 {
        store.delete_meta(FLAG_KEY).map_err(db)?;
        return Ok(None);
    }
    let last = store.get_meta(PROMPTED_KEY).map_err(db)?.and_then(|v| v.parse::<i64>().ok());
    if let Some(t) = last {
        if t > now_ms {
            // The clock was set back: count the prompt as just shown (clamped), never as long ago.
            store.set_meta(PROMPTED_KEY, &now_ms.to_string()).map_err(db)?;
            return update_standing_flag(store, now_ms);
        }
        if now_ms - t < PROMPT_INTERVAL_MS {
            return update_standing_flag(store, now_ms);
        }
    }
    let flag = ReviewFlag { clients: due.len(), items, raised_at_unix_ms: now_ms };
    let json = serde_json::to_string(&flag).map_err(|_| ApiError::ProviderFailure)?;
    store.set_meta(FLAG_KEY, &json).map_err(db)?;
    store.set_meta(PROMPTED_KEY, &now_ms.to_string()).map_err(db)?;
    Ok(Some(flag))
}

/// Brings a standing flag's counts up to date (cleared when nothing is due); never raises one.
fn update_standing_flag(store: &memory::Store, now_ms: i64) -> ApiResult<Option<ReviewFlag>> {
    let Some(flag) = review_flag(store) else { return Ok(None) };
    let due = due_items(store, now_ms)?;
    let items: usize = due.iter().map(|c| c.items.len()).sum();
    if items == 0 {
        store.delete_meta(FLAG_KEY).map_err(db)?;
        return Ok(None);
    }
    let flag = ReviewFlag { clients: due.len(), items, ..flag };
    let json = serde_json::to_string(&flag).map_err(|_| ApiError::ProviderFailure)?;
    store.set_meta(FLAG_KEY, &json).map_err(db)?;
    Ok(Some(flag))
}

/// The "N items ready to review" flag, if raised. Unreadable reads as none.
pub fn review_flag(store: &memory::Store) -> Option<ReviewFlag> {
    store.get_meta(FLAG_KEY).ok().flatten().and_then(|v| serde_json::from_str(&v).ok())
}

/// The user set the prompt aside (or opened the review): the flag goes, and comes back in a week if
/// items are still due.
pub fn dismiss_review_flag(store: &memory::Store, now_ms: i64) -> ApiResult<()> {
    store.delete_meta(FLAG_KEY).map_err(db)?;
    store.set_meta(PROMPTED_KEY, &now_ms.to_string()).map_err(db)
}

#[derive(Default)]
struct Tally {
    counts: Vec<(ItemKind, usize)>,
    not_due: Vec<(ItemKind, usize)>,
    waiting: Vec<(ItemKind, usize)>,
    failed: Vec<ItemKind>,
}

impl Tally {
    fn outcome(&mut self, kind: ItemKind, done: rusqlite::Result<bool>) -> bool {
        match done {
            Ok(deleted) => {
                if deleted {
                    bump(&mut self.counts, kind, 1);
                }
                true
            }
            Err(e) if memory::is_unsent_ops(&e) => {
                bump(&mut self.waiting, kind, 1);
                false
            }
            Err(_) => {
                if !self.failed.contains(&kind) {
                    self.failed.push(kind);
                    self.failed.sort();
                }
                false
            }
        }
    }
}

/// Deletes, on this device only, the items of `sel` that are still due at `now_ms` (the user confirmed
/// them on the review screen). See the module docs: nothing reaches other devices, and the client's own
/// record stays.
///
/// - `media_roots`: the only folders a meeting's audio, transcript and slide files are removed from.
/// - `sync_on`: sync is set up on this device. An item it has not sent to the other devices yet is then
///   left (`waiting_to_sync`), so expiring it here never loses the only copy.
/// - `budget`: the run stops starting new items after this long (`more`); a later run continues. The
///   final index clean-up runs after it.
///
/// A run that deleted something also removes the encryption upgrade's plaintext rollback copy
/// (`<db>.plain.bak`) while it exists, as a client delete does; if that fails, `failed_stages` has `Files`
/// and the next run retries it.
///
/// `Err` only when nothing could be read (an unknown client is `NotFound`; unreadable settings or
/// decisions): then nothing is deleted. Safe to re-run: what is gone is skipped.
pub fn expire_items(
    store: &memory::Store,
    sel: &ExpirySelection,
    media_roots: &[PathBuf],
    now_ms: i64,
    sync_on: bool,
    budget: Duration,
) -> ApiResult<ExpiryReceipt> {
    let started = Instant::now();
    expire_items_until(store, sel, media_roots, now_ms, sync_on, &|_| started.elapsed() >= budget)
}

/// [`expire_items`] with the stop rule given: `stop(n)` is asked before starting the item after `n`
/// started ones (tests stop after a count).
fn expire_items_until(
    store: &memory::Store,
    sel: &ExpirySelection,
    media_roots: &[PathBuf],
    now_ms: i64,
    sync_on: bool,
    stop: &dyn Fn(usize) -> bool,
) -> ApiResult<ExpiryReceipt> {
    use memory::client_delete::PENDING_CAPTURE;
    let mut started_items = 0usize;
    live_client(store, &sel.key)?;
    let due = due_items_for(store, &sel.key, now_ms)?;
    let due_ids: HashSet<(ItemKind, &str)> = due.iter().map(|i| (i.kind, i.id.as_str())).collect();
    // Retention is per device (spec 4.3): every delete below goes through memory's `expire_*` entry
    // points, which build the local-only reach themselves and fail if a tombstone was ever queued.
    let mut tally = Tally::default();
    let mut targets: Vec<&ClientItem> = Vec::new();
    let mut picked: HashSet<(ItemKind, &str)> = HashSet::new();
    for item in &sel.items {
        if due_ids.contains(&(item.kind, item.id.as_str())) {
            if picked.insert((item.kind, item.id.as_str())) {
                targets.push(item);
            }
        } else {
            bump(&mut tally.not_due, item.kind, 1);
        }
    }
    targets.sort_by_key(|i| i.date_unix_ms);

    // The expiry's own pending record, never the client delete's (that one owes deletes everywhere). A later
    // client delete adopts what is left of it.
    let run = memory::Store::client_expiry_run_id(&sel.key);
    let secure_before = store.set_secure_delete(true).ok();
    let mut stages: Vec<DeleteStage> = Vec::new();
    let mut more = false;
    let mut gone: Vec<&ClientItem> = Vec::new();
    for item in targets {
        if stop(started_items) {
            more = true;
            break;
        }
        started_items += 1;
        let row_id = || item.id.parse::<i64>().map_err(|_| rusqlite::Error::InvalidQuery);
        let done: rusqlite::Result<bool> = match item.kind {
            ItemKind::Meeting => row_id().and_then(|id| expire_meeting(store, &run, id, sync_on)),
            ItemKind::Episode => row_id()
                .and_then(|_| store.expire_client_episode(&run, &item.id, sync_on))
                .map(|o| o.is_some()),
            ItemKind::Email => item_in(store, ClientItemKind::Email, &item.id, sync_on),
            ItemKind::CalendarEvent => item_in(store, ClientItemKind::CalendarEvent, &item.id, sync_on),
            // Never listed as due (captures go with their episode, steps with their source; the rest are
            // not linked material).
            ItemKind::Capture
            | ItemKind::Thread
            | ItemKind::Conversation
            | ItemKind::Step
            | ItemKind::VaultNote
            | ItemKind::ExportFolder => continue,
        };
        if tally.outcome(item.kind, done) {
            gone.push(item);
        }
    }

    // Captures of episodes this run (or an earlier one) deleted.
    match store.client_delete_pending(&run) {
        Ok(pending) => {
            let mut done = Vec::new();
            for (kind, value) in pending.into_iter().filter(|(k, _)| k == PENDING_CAPTURE) {
                if stop(started_items) {
                    more = true;
                    break;
                }
                started_items += 1;
                let r = store.expire_client_item(ClientItemKind::Capture, &value, sync_on).map(|o| o.is_some());
                if tally.outcome(ItemKind::Capture, r) {
                    done.push((kind, value));
                }
            }
            if store.clear_client_delete_pending(&run, &done).is_err() {
                stages.push(DeleteStage::Files);
            }
        }
        Err(_) => stages.push(DeleteStage::Files),
    }

    // Meeting files, then vault notes of deleted meetings (never the client's own topic note).
    let roots: Vec<&Path> = media_roots.iter().map(PathBuf::as_path).collect();
    let mut left_files: Vec<PathBuf> = Vec::new();
    match store
        .client_delete_pending(&run)
        .and_then(|p| crate::client_delete::finish_meeting_files(store, &p, &roots))
    {
        Ok(files) => {
            left_files.extend(files.left);
            if files.stuck || store.clear_client_delete_pending(&run, &files.done).is_err() {
                stages.push(DeleteStage::Files);
            }
        }
        Err(_) => stages.push(DeleteStage::Files),
    }
    match store
        .client_delete_pending(&run)
        .and_then(|p| crate::client_delete::vault_owners(store, &p, false))
    {
        Ok((owners, used)) => {
            // Vault sync off: never touched, even if a path came along.
            let removed = match sel.vault.as_ref().filter(|_| !sel.vault_off) {
                Some(vault) => crate::topics_export::delete_vault_notes(vault, &owners).map(|r| {
                    left_files.extend(r.unchecked);
                    r.removed
                }),
                None => Ok(0),
            };
            match removed {
                Ok(n) => {
                    bump(&mut tally.counts, ItemKind::VaultNote, n);
                    if store.clear_client_delete_pending(&run, &used).is_err() {
                        stages.push(DeleteStage::Files);
                    }
                }
                Err(_) => {
                    if !tally.failed.contains(&ItemKind::VaultNote) {
                        tally.failed.push(ItemKind::VaultNote);
                    }
                }
            }
        }
        Err(_) => stages.push(DeleteStage::Files),
    }

    // Derived text is not swept by name here: only what was built from the deleted items goes with them
    // (review finding 1: nothing the user did not confirm). The index rebuild runs once, on the slice that
    // finishes (a slice that stops early leaves it to the last one).
    // "Cleanup owed" is remembered across slices and runs: any slice that deleted something sets it, and it
    // clears only when the cleanup succeeds, so an empty final slice still finishes what earlier ones began.
    const CLEANUP_OWED: &str = "client_expiry_cleanup_owed";
    if !gone.is_empty() && store.set_meta(CLEANUP_OWED, "1").is_err() {
        stages.push(DeleteStage::Cleanup);
    }
    if !more && matches!(store.get_meta(CLEANUP_OWED), Ok(Some(_)) | Err(_)) {
        if store.finish_client_delete().is_err() {
            stages.push(DeleteStage::Cleanup);
        } else {
            let _ = store.delete_meta(CLEANUP_OWED);
        }
    }
    // X4 design 3.5 step 8, as for a client delete: the encryption upgrade's plaintext rollback copy still holds
    // what this run deleted, so it goes too. Owed until it is gone: a run that could not remove it fails, and the
    // next run retries even with nothing left to delete.
    const ROLLBACK_COPY_OWED: &str = "client_expiry_rollback_copy_owed";
    if !gone.is_empty() || matches!(store.get_meta(ROLLBACK_COPY_OWED), Ok(Some(_))) {
        if let Some(db) = store.db_path() {
            let paths = crate::encrypt_migration::MigrationPaths::beside(&db);
            match crate::encrypt_migration::remove_plain_backup_for_client_delete(&paths) {
                Ok(_) => {
                    let _ = store.delete_meta(ROLLBACK_COPY_OWED);
                }
                Err(_) => {
                    let _ = store.set_meta(ROLLBACK_COPY_OWED, "1");
                    stages.push(DeleteStage::Files);
                }
            }
        }
    }
    if let Some(before) = secure_before {
        let _ = store.set_secure_delete(before);
    }
    // Decisions about items that are gone are moot.
    for item in &gone {
        let _ = store.delete_meta(&decision_key(&sel.key, item.kind, &item.id));
    }
    // A count only, so a later client delete can say these copies stay on the other devices.
    let expired: usize =
        tally.counts.iter().filter(|(k, _)| *k != ItemKind::VaultNote).map(|(_, n)| n).sum();
    if store.add_client_expired_count(&sel.key, expired).is_err() {
        stages.push(DeleteStage::Files);
    }

    let mut left_for_you = Vec::new();
    if let Some(p) = media_roots.first().cloned() {
        left_for_you.push(LeftForYou::MediaFolder(p));
    }
    if sel.vault_off {
        left_for_you.push(LeftForYou::VaultSyncOff);
    } else if let Some(p) = sel.vault.clone() {
        left_for_you.push(LeftForYou::VaultFolder(p));
    }
    left_for_you.push(LeftForYou::MailboxOriginals);
    left_for_you.push(LeftForYou::BackupsAndCloud);
    left_for_you.push(LeftForYou::CopiesOnOtherDevices);
    if !left_files.is_empty() {
        left_files.sort();
        left_files.dedup();
        left_for_you.push(LeftForYou::FilesNotRemoved(left_files));
    }
    let receipt = ExpiryReceipt {
        date_unix_ms: now_ms,
        counts: tally.counts,
        not_due: tally.not_due,
        waiting_to_sync: tally.waiting,
        more,
        failed: tally.failed,
        failed_stages: stages,
        left_for_you,
    };
    if let Ok(json) = serde_json::to_string(&StoredExpiry::from(&receipt)) {
        // One receipt per slice, never overwritten: `<now>:<n>`, n the first free number for that time.
        let taken = store
            .list_meta_with_prefix(&format!("{EXPIRY_RECEIPT_PREFIX}{now_ms}:"))
            .map(|v| v.len())
            .unwrap_or(0);
        let _ = store.set_meta(&format!("{EXPIRY_RECEIPT_PREFIX}{now_ms}:{taken:06}"), &json);
    }
    let _ = update_standing_flag(store, now_ms);
    Ok(receipt)
}

fn item_in(store: &memory::Store, kind: ClientItemKind, id: &str, sync_on: bool) -> rusqlite::Result<bool> {
    id.parse::<i64>().map_err(|_| rusqlite::Error::InvalidQuery)?;
    store.expire_client_item(kind, id, sync_on).map(|o| o.is_some())
}

/// Meeting `id`, here only: its files recorded as pending first (finished by this run or a later one, or by
/// a client delete), then its rows. A meeting that waits for sync keeps nothing pending.
fn expire_meeting(store: &memory::Store, run: &str, id: i64, sync_on: bool) -> rusqlite::Result<bool> {
    let rows = crate::client_delete::meeting_pending_rows(store, run, id)?;
    if rows.is_empty() {
        return Ok(false);
    }
    match store.expire_meeting_here(id, sync_on) {
        Ok(files) => Ok(files.is_some()),
        Err(e) => {
            let _ = store.clear_client_delete_pending(run, &rows);
            Err(e)
        }
    }
}

/// The stored copy of an [`ExpiryReceipt`]: counts and categories, no paths.
#[derive(Serialize)]
struct StoredExpiry<'a> {
    date_unix_ms: i64,
    counts: &'a [(ItemKind, usize)],
    not_due: &'a [(ItemKind, usize)],
    waiting_to_sync: &'a [(ItemKind, usize)],
    more: bool,
    failed: &'a [ItemKind],
    failed_stages: &'a [DeleteStage],
}

impl<'a> From<&'a ExpiryReceipt> for StoredExpiry<'a> {
    fn from(r: &'a ExpiryReceipt) -> Self {
        StoredExpiry {
            date_unix_ms: r.date_unix_ms,
            counts: &r.counts,
            not_due: &r.not_due,
            waiting_to_sync: &r.waiting_to_sync,
            more: r.more,
            failed: &r.failed,
            failed_stages: &r.failed_stages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    const NAME: &str = "Zorblax";
    /// 2026-11-12T00:00:00Z.
    const NOW: i64 = 1_794_441_600_000;
    const DAY: i64 = 24 * 60 * 60 * 1000;

    struct Env {
        dir: tempfile::TempDir,
        store: memory::Store,
        c: Connection,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let store = crate::test_store::open(&path, dir.path());
        store.set_own_device_id("dev-a");
        let c = crate::test_store::raw(&path);
        Env { dir, store, c }
    }

    fn count(e: &Env, sql: &str) -> i64 {
        e.c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// `months` calendar months before NOW, less a day so it is clearly past.
    fn ago(months: u32) -> i64 {
        cutoff_ms(NOW, months).unwrap() - DAY
    }

    fn file(e: &Env, topic: i64, kind: &str, r: i64, ts: i64) {
        let sid: i64 = match e.c.query_row(
            "SELECT id FROM topic_source WHERE kind = ?1 AND ref_id = ?2",
            params![kind, r],
            |x| x.get(0),
        ) {
            Ok(id) => id,
            Err(_) => {
                e.c.execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, ?3, 1)",
                    params![kind, r, ts],
                )
                .unwrap();
                e.c.last_insert_rowid()
            }
        };
        e.c.execute(
            "INSERT OR IGNORE INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')",
            params![topic, sid],
        )
        .unwrap();
    }

    fn email(e: &Env, ext: &str, ts: i64, thread: &str) -> i64 {
        e.store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "dev-a".into(),
                external_id: ext.into(),
                ts_unix_ms: ts,
                from: "pat@x.com".into(),
                to: "me@x.com".into(),
                subject: format!("{NAME} {ext}"),
                body_text: format!("{NAME} wrote {ext}"),
                gmail_thread_id: thread.into(),
                account_email: "me@x.com".into(),
                gmail_labels: vec![],
                list_unsubscribe: false,
            }])
            .unwrap();
        e.c.query_row("SELECT id FROM items WHERE kind = 'email' AND external_id = ?1", [ext], |r| r.get(0))
            .unwrap()
    }

    fn meeting(e: &Env, start: i64) -> i64 {
        let id = e
            .store
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev-a".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 3_600_000),
                title: format!("{NAME} review"),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        e.store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id: id,
                start_cs: 0,
                speaker_label: "A".into(),
                text: format!("{NAME} said hello"),
            }])
            .unwrap();
        id
    }

    /// Everything this device has sent: the other devices have it.
    fn synced(e: &Env) {
        e.store.set_sync_push_cursor(i64::MAX).unwrap();
        e.c.execute(
            "UPDATE sync_outbox SET state = 'delivered', delivered_at_unix_ms = 1, remote_receipt = 'sync_row:' || local_seq",
            [],
        )
        .unwrap();
    }

    struct World {
        a: memory::topics::seeds::SeedTopic,
        old_email: i64,
        new_email: i64,
        old_meeting: i64,
        new_meeting: i64,
    }

    /// Client A keeps 12 months: an email and a meeting 13 months old, and one of each from last month.
    fn world(e: &Env) -> World {
        let a = e.store.create_seed_topic(NAME, &[], None).unwrap();
        e.store
            .set_client_control(&a.key, &memory::topics::client_controls::ClientControl { exclude_hosted: false, keep_months: Some(12) })
            .unwrap();
        let old_email = email(e, "old", ago(13), "t-old");
        let new_email = email(e, "new", ago(1), "t-new");
        let old_meeting = meeting(e, ago(13));
        let new_meeting = meeting(e, ago(1));
        file(e, a.topic_id, "email", old_email, ago(13));
        file(e, a.topic_id, "email", new_email, ago(1));
        file(e, a.topic_id, "meeting", old_meeting, ago(13));
        file(e, a.topic_id, "meeting", new_meeting, ago(1));
        World { a, old_email, new_email, old_meeting, new_meeting }
    }

    fn ids(items: &[ClientItem]) -> Vec<(ItemKind, String)> {
        let mut v: Vec<_> = items.iter().map(|i| (i.kind, i.id.clone())).collect();
        v.sort();
        v
    }

    fn due_of(e: &Env, key: &str, now: i64) -> Vec<(ItemKind, String)> {
        ids(&due_items_for(&e.store, key, now).unwrap())
    }

    fn set_keep(e: &Env, key: &str, months: Option<u32>) {
        e.store
            .set_client_control(key, &memory::topics::client_controls::ClientControl { exclude_hosted: false, keep_months: months })
            .unwrap();
    }

    /// Vault sync off: an expired meeting's vault note stays, even when a vault path comes along; on, it goes.
    #[test]
    fn an_expiry_touches_the_vault_only_while_vault_sync_is_on() {
        for off in [true, false] {
            let e = env();
            let w = world(&e);
            let vault = e.dir.path().join("vault");
            std::fs::create_dir_all(vault.join("Whimbrel/Meetings")).unwrap();
            let note = vault.join("Whimbrel/Meetings/Old.md");
            std::fs::write(&note, format!("---\nwhimbrel_id: meeting-{}\n---\n# Old\n", w.old_meeting)).unwrap();
            let items = due_items_for(&e.store, &w.a.key, NOW).unwrap().into_iter().filter(|i| i.kind == ItemKind::Meeting).collect();
            let sel = ExpirySelection { key: w.a.key.clone(), items, vault: Some(vault.clone()), vault_off: off };
            let r = expire_items(&e.store, &sel, &[], NOW, false, LONG).unwrap();
            assert_eq!(r.counts.iter().find(|(k, _)| *k == ItemKind::Meeting).map(|x| x.1), Some(1), "{r:?}");
            assert_eq!(note.exists(), off, "off: the note stays; on: it goes");
            assert_eq!(r.left_for_you.contains(&LeftForYou::VaultSyncOff), off);
            assert_eq!(r.left_for_you.contains(&LeftForYou::VaultFolder(vault)), !off);
        }
    }

    #[test]
    fn old_item_of_active_client_is_due_new_one_is_not() {
        let e = env();
        let w = world(&e);
        let due = due_items(&e.store, NOW).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].key, w.a.key);
        assert_eq!(
            ids(&due[0].items),
            vec![(ItemKind::Meeting, w.old_meeting.to_string()), (ItemKind::Email, w.old_email.to_string())]
        );
        // A meeting's age counts from its end: one that ended a day inside the period is not due.
        let edge = meeting(&e, cutoff_ms(NOW, 12).unwrap() - 3_600_000 + DAY);
        file(&e, w.a.topic_id, "meeting", edge, 0);
        assert!(!due_of(&e, &w.a.key, NOW).contains(&(ItemKind::Meeting, edge.to_string())));
    }

    #[test]
    fn keep_decision_suppresses_item() {
        let e = env();
        let w = world(&e);
        let due = due_items_for(&e.store, &w.a.key, NOW).unwrap();
        let email = due.iter().find(|i| i.kind == ItemKind::Email).unwrap();
        set_retention_decision(&e.store, &w.a.key, email, Decision::Keep).unwrap();
        assert_eq!(due_of(&e, &w.a.key, NOW), vec![(ItemKind::Meeting, w.old_meeting.to_string())]);
        assert!(!due_of(&e, &w.a.key, NOW + 400 * DAY).contains(&(ItemKind::Email, w.old_email.to_string())));
        // Stored under an opaque prefix: no key, no name.
        let stored: Vec<String> = e
            .c
            .prepare("SELECT key || value FROM kv_meta WHERE key LIKE 'client_retention:%'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert!(!stored[0].contains(&w.a.key) && !stored[0].contains(NAME));
    }

    #[test]
    fn snooze_30_days_hides_then_returns() {
        let e = env();
        let w = world(&e);
        let due = due_items_for(&e.store, &w.a.key, NOW).unwrap();
        for item in &due {
            set_retention_decision(&e.store, &w.a.key, item, Decision::Snooze { until_ms: NOW + SNOOZE_MS }).unwrap();
        }
        assert!(due_items(&e.store, NOW).unwrap().is_empty());
        assert!(due_items(&e.store, NOW + SNOOZE_MS - 1).unwrap().is_empty());
        assert_eq!(due_of(&e, &w.a.key, NOW + SNOOZE_MS + 1), ids(&due));
    }

    #[test]
    fn no_setting_means_nothing_due() {
        let e = env();
        let w = world(&e);
        set_keep(&e, &w.a.key, None);
        assert!(due_items(&e.store, NOW).unwrap().is_empty());
        assert!(due_items_for(&e.store, &w.a.key, NOW).unwrap().is_empty());
        // Zero months is not "everything": nothing is due.
        set_keep(&e, &w.a.key, Some(0));
        assert!(due_items(&e.store, NOW).unwrap().is_empty());
        assert!(matches!(due_items_for(&e.store, "no-such-client", NOW), Err(ApiError::NotFound)));
    }

    /// Every row of every table, as text.
    fn dump(e: &Env) -> Vec<String> {
        let tables: Vec<String> = e
            .c
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let mut out = Vec::new();
        for t in tables {
            let Ok(mut stmt) = e.c.prepare(&format!("SELECT * FROM \"{t}\"")) else { continue };
            let n = stmt.column_count();
            let rows = stmt
                .query_map([], |r| {
                    let mut s = t.clone();
                    for i in 0..n {
                        s.push('|');
                        s.push_str(&format!("{:?}", r.get_ref(i)?));
                    }
                    Ok(s)
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            out.extend(rows);
        }
        out
    }

    #[test]
    fn retention_never_deletes_without_confirmation() {
        let e = env();
        let w = world(&e);
        let before = dump(&e);
        assert!(!due_items(&e.store, NOW).unwrap().is_empty());
        due_items_for(&e.store, &w.a.key, NOW).unwrap();
        assert_eq!(dump(&e), before, "listing what is due changes nothing");
        // The scheduler's weekly check raises a count-only flag and deletes nothing.
        let flag = refresh_review_flag(&e.store, NOW).unwrap().unwrap();
        assert_eq!((flag.clients, flag.items), (1, 2));
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.old_email)), 1);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.old_meeting)), 1);
        // So does the global retention sweep it rides on, and the client's meeting outlives a short
        // global window (the client's period overrides it).
        let policy = memory::RetentionPolicy { capture_item_days: None, meeting_days: Some(1), conversation_days: None };
        crate::retention::apply_within(&e.store, &policy, NOW, &[e.dir.path()]).unwrap();
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.old_meeting)), 1);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.new_meeting)), 1);
    }

    #[test]
    fn the_review_flag_comes_back_weekly_and_holds_counts_only() {
        let e = env();
        let w = world(&e);
        assert!(review_flag(&e.store).is_none());
        refresh_review_flag(&e.store, NOW).unwrap();
        let raised = review_flag(&e.store).unwrap();
        assert_eq!(raised.items, 2);
        let stored = e.store.get_meta(FLAG_KEY).unwrap().unwrap();
        assert!(!stored.contains(&w.a.key) && !stored.contains(NAME));
        dismiss_review_flag(&e.store, NOW + DAY).unwrap();
        assert!(review_flag(&e.store).is_none());
        refresh_review_flag(&e.store, NOW + 2 * DAY).unwrap();
        assert!(review_flag(&e.store).is_none(), "not again within the week");
        refresh_review_flag(&e.store, NOW + DAY + PROMPT_INTERVAL_MS).unwrap();
        assert!(review_flag(&e.store).is_some(), "back a week later while items are still due");
        set_keep(&e, &w.a.key, None);
        refresh_review_flag(&e.store, NOW + 30 * DAY).unwrap();
        assert!(review_flag(&e.store).is_none(), "nothing due: no flag");
    }

    fn select_all(e: &Env, key: &str) -> ExpirySelection {
        ExpirySelection { key: key.into(), items: due_items_for(&e.store, key, NOW).unwrap(), vault: None, vault_off: false }
    }

    const LONG: Duration = Duration::from_secs(600);

    #[test]
    fn a_confirmed_expiry_deletes_here_only_and_keeps_the_client() {
        let e = env();
        let w = world(&e);
        synced(&e);
        let outbox_before = count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE state != 'delivered'");
        // A failed summary of the old meeting quoted in a model error note (X6 final review, Major 1).
        e.store.record_summarize_error("episode_summarize", "no JSON object: \"old kickoff notes\"", 1);
        let r = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(r.counts, vec![(ItemKind::Meeting, 1), (ItemKind::Email, 1)]);
        assert_eq!(count(&e, "SELECT COUNT(*) FROM kv_meta WHERE key LIKE 'last_error:%'"), 0, "error notes go");
        assert!(r.left_for_you.contains(&LeftForYou::CopiesOnOtherDevices));

        // Old material gone, new material and the client stay.
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.old_email)), 0);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.old_meeting)), 0);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.new_email)), 1);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.new_meeting)), 1);
        let seeds = e.store.list_seed_topics().unwrap();
        assert!(seeds.iter().any(|s| s.key == w.a.key && s.name == NAME));
        assert_eq!(e.store.client_control(&w.a.key).unwrap().keep_months, Some(12));
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM transcript_segment WHERE meeting_id = {}", w.old_meeting)), 0);
        assert_eq!(count(&e, "SELECT COUNT(*) FROM items_fts WHERE items_fts MATCH 'old'"), 0);

        // Nothing reaches other devices: no tombstone, no forgotten record, no relay clean-up, no new op.
        for (sql, what) in [
            ("SELECT COUNT(*) FROM sync_outbox WHERE operation_kind = 'tombstone'", "tombstones"),
            ("SELECT COUNT(*) FROM forgotten_client_item", "forgotten items"),
            ("SELECT COUNT(*) FROM forgotten_meeting", "forgotten meetings"),
            ("SELECT COUNT(*) FROM client_relay_cleanup", "relay clean-up"),
            ("SELECT COUNT(*) FROM meeting_server_cleanup", "meeting server clean-up"),
        ] {
            assert_eq!(count(&e, sql), 0, "{what}");
        }
        assert_eq!(count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE state != 'delivered'"), outbox_before);

        // The receipt and its stored copy name nothing.
        let shown = serde_json::to_string(&r).unwrap();
        let stored = e.store.list_meta_with_prefix(EXPIRY_RECEIPT_PREFIX).unwrap();
        assert_eq!(stored.len(), 1);
        for text in [shown.as_str(), stored[0].1.as_str(), &format!("{r:?}")] {
            assert!(!text.contains(NAME) && !text.contains(&w.a.key), "{text}");
        }

        // A re-run has nothing left to do.
        let again = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert!(again.counts.is_empty() && again.complete());
    }

    /// Security review: every kind an expiry deletes (meeting, email, calendar event, episode and its
    /// captures) queues no tombstone and records nothing forgotten or for relay clean-up.
    #[test]
    fn expiry_queues_no_tombstone_and_records_nothing_forgotten() {
        let e = env();
        let w = world(&e);
        e.store
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: "dev-a".into(),
                source: "google_calendar".into(),
                external_id: "ev-old#1".into(),
                ts_unix_ms: ago(13),
                title: format!("{NAME} kickoff"),
                actors: vec![],
            }])
            .unwrap();
        let cal: i64 = e.c.query_row("SELECT id FROM items WHERE kind = 'calendar_event'", [], |r| r.get(0)).unwrap();
        file(&e, w.a.topic_id, "calendar_event", cal, ago(13));
        let cap = e
            .store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "dev-a".into(),
                ts_unix_ms: ago(13),
                app_bundle_id: "app".into(),
                window_title: "w".into(),
                url: None,
                text: format!("{NAME} draft"),
                hash: 1,
                source: "ax".into(),
            })
            .unwrap()
            .unwrap();
        let item: i64 = e
            .c
            .query_row("SELECT id FROM items WHERE source_table = 'capture_item' AND source_id = ?1", [cap], |r| r.get(0))
            .unwrap();
        e.c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, \
             start_item_id, end_item_id, item_count, summary, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('dev-a', 'app', 'w', ?1, ?1, ?2, ?2, 1, 's', 1, 1)",
            params![ago(13), item],
        )
        .unwrap();
        file(&e, w.a.topic_id, "episode", e.c.last_insert_rowid(), ago(13));
        synced(&e);
        let r = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(
            r.counts,
            vec![(ItemKind::Meeting, 1), (ItemKind::Email, 1), (ItemKind::CalendarEvent, 1), (ItemKind::Capture, 1), (ItemKind::Episode, 1)]
        );
        for table in ["forgotten_client_item", "forgotten_meeting", "client_relay_cleanup", "meeting_server_cleanup"] {
            assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM {table}")), 0, "{table}");
        }
        assert_eq!(count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE operation_kind = 'tombstone'"), 0);
        assert_eq!(count(&e, "SELECT COUNT(*) FROM sync_outbox WHERE state != 'delivered'"), 0, "nothing new to send");
    }

    #[test]
    fn only_what_is_still_due_is_deleted_and_a_shorter_period_applies_next_run() {
        let e = env();
        let w = world(&e);
        synced(&e);
        let confirmed = select_all(&e, &w.a.key);
        // The period was lengthened after the review: nothing is due, nothing goes.
        set_keep(&e, &w.a.key, Some(24));
        let r = expire_items(&e.store, &confirmed, &[], NOW, true, LONG).unwrap();
        assert!(r.counts.is_empty());
        assert_eq!(r.not_due, vec![(ItemKind::Meeting, 1), (ItemKind::Email, 1)]);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.old_email)), 1);
        // A newer item slipped into the selection is never deleted.
        let mut forged = select_all(&e, &w.a.key);
        forged.items.push(ClientItem {
            kind: ItemKind::Email,
            id: w.new_email.to_string(),
            also_client: None,
            date_unix_ms: 0,
            removed: false,
            label: None,
            excerpt: None,
            app: None,
        });
        let r = expire_items(&e.store, &forged, &[], NOW, true, LONG).unwrap();
        assert_eq!(r.not_due, vec![(ItemKind::Email, 1)]);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.new_email)), 1);
        // Shortened to 1 month: last month's items are due on the next run too.
        set_keep(&e, &w.a.key, Some(1));
        let due = due_of(&e, &w.a.key, NOW);
        assert!(due.contains(&(ItemKind::Meeting, w.new_meeting.to_string())));
        assert!(due.contains(&(ItemKind::Email, w.new_email.to_string())));
    }

    #[test]
    fn undated_and_shared_items_are_handled_safely() {
        let e = env();
        let w = world(&e);
        let b = e.store.create_seed_topic("Globex", &[], None).unwrap();
        // An undated email is never due.
        let undated = email(&e, "undated", 0, "t-u");
        file(&e, w.a.topic_id, "email", undated, 0);
        assert!(!due_of(&e, &w.a.key, NOW).contains(&(ItemKind::Email, undated.to_string())));
        // The old email is also B's, and B has no period (keep forever): not due for A.
        file(&e, b.topic_id, "email", w.old_email, ago(13));
        assert_eq!(due_of(&e, &w.a.key, NOW), vec![(ItemKind::Meeting, w.old_meeting.to_string())]);
        // B keeps 24 months: the longer period wins, still not due.
        set_keep(&e, &b.key, Some(24));
        assert!(!due_of(&e, &w.a.key, NOW).contains(&(ItemKind::Email, w.old_email.to_string())));
        // B keeps 6 months: due under A's 12, and listed for both.
        set_keep(&e, &b.key, Some(6));
        assert!(due_of(&e, &w.a.key, NOW).contains(&(ItemKind::Email, w.old_email.to_string())));
        assert!(due_of(&e, &b.key, NOW).contains(&(ItemKind::Email, w.old_email.to_string())));
    }

    #[test]
    fn a_run_stops_at_its_time_limit_and_the_next_continues() {
        let e = env();
        let w = world(&e);
        synced(&e);
        let r = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, Duration::ZERO).unwrap();
        assert!(r.more && r.counts.is_empty() && !r.complete());
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.old_email)), 1);
        let r = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert!(r.complete());
        assert_eq!(r.counts, vec![(ItemKind::Meeting, 1), (ItemKind::Email, 1)]);
    }

    #[test]
    fn material_not_sent_yet_waits_while_sync_is_on() {
        let e = env();
        let w = world(&e);
        // Nothing pushed: the meeting is this device's and the other devices have not got it.
        let r = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert_eq!(r.waiting_to_sync, vec![(ItemKind::Meeting, 1)]);
        assert!(!r.complete());
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.old_meeting)), 1);
        assert_eq!(count(&e, "SELECT COUNT(*) FROM client_delete_pending"), 0, "nothing owed for a waiting meeting");
        // Still due, so the review offers it again; once sent it goes.
        assert!(due_of(&e, &w.a.key, NOW).contains(&(ItemKind::Meeting, w.old_meeting.to_string())));
        synced(&e);
        let r = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.old_meeting)), 0);
    }

    #[test]
    fn unreadable_settings_delete_nothing() {
        let e = env();
        let w = world(&e);
        synced(&e);
        let sel = select_all(&e, &w.a.key);
        e.c.execute("ALTER TABLE client_control RENAME TO client_control_broken", []).unwrap();
        assert!(due_items(&e.store, NOW).is_err());
        assert!(expire_items(&e.store, &sel, &[], NOW, true, LONG).is_err());
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.old_email)), 1);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.old_meeting)), 1);
        // Global retention does not run over the client's material either.
        let policy = memory::RetentionPolicy { capture_item_days: None, meeting_days: Some(1), conversation_days: None };
        assert!(crate::retention::apply_within(&e.store, &policy, NOW, &[e.dir.path()]).is_err());
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.old_meeting)), 1);
    }

    fn routine_and_digest(e: &Env) {
        e.c.execute_batch(&format!(
            "INSERT INTO routine (id, name, template, schedule_hhmm, created_at_unix_ms) VALUES (1, 'r', 't', '09:00', 1);
             INSERT INTO routine_run (routine_id, ran_at_unix_ms, status, output_summary, output_json) VALUES (1, {old}, 'ok', 'Call {NAME}', '{{}}');
             INSERT INTO mcp_daily_digest_cache (day, digest_json, generated_at_unix_ms) VALUES ('2020-01-01', '{{\"about\":\"Globex\"}}', 1);
             INSERT INTO person (person_key, name, updated_at_unix_ms) VALUES ('p1', '{NAME} contact', 1);",
            old = ago(13),
        ))
        .unwrap();
    }

    /// Review finding 1: an expiry touches only what the user confirmed. Derived text the review never
    /// listed (routine runs naming the client, digest days, people) stays, whatever the selection.
    #[test]
    fn an_empty_or_all_kept_selection_deletes_nothing_at_all() {
        let e = env();
        let w = world(&e);
        synced(&e);
        routine_and_digest(&e);
        let empty = ExpirySelection { key: w.a.key.clone(), items: vec![], vault: None, vault_off: false };
        let r = expire_items(&e.store, &empty, &[], NOW, true, LONG).unwrap();
        assert!(r.counts.is_empty());
        for item in due_items_for(&e.store, &w.a.key, NOW).unwrap() {
            set_retention_decision(&e.store, &w.a.key, &item, Decision::Keep).unwrap();
        }
        let kept = select_all(&e, &w.a.key);
        assert!(kept.items.is_empty());
        expire_items(&e.store, &kept, &[], NOW, true, LONG).unwrap();
        // Even a confirmed expiry leaves derived text it did not list.
        set_keep(&e, &w.a.key, Some(6));
        let other = e.store.create_seed_topic("Wombat", &[], None).unwrap();
        set_keep(&e, &other.key, Some(6));
        let m = meeting(&e, ago(13));
        file(&e, other.topic_id, "meeting", m, ago(13));
        synced(&e);
        expire_items(&e.store, &select_all(&e, &other.key), &[], NOW, true, LONG).unwrap();
        assert_eq!(count(&e, "SELECT COUNT(*) FROM routine_run"), 1);
        assert_eq!(count(&e, "SELECT COUNT(*) FROM mcp_daily_digest_cache"), 1);
        assert_eq!(count(&e, "SELECT COUNT(*) FROM person"), 1);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", w.old_email)), 1);
    }

    /// `(episode id, [capture items.id])` for an episode of `app` over captures at `ts`.
    fn episode(e: &Env, topic: i64, app: &str, ts: &[i64]) -> (i64, Vec<i64>) {
        let mut items = Vec::new();
        for (n, t) in ts.iter().enumerate() {
            let cap = e
                .store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "dev-a".into(),
                    ts_unix_ms: *t,
                    app_bundle_id: app.into(),
                    window_title: "w".into(),
                    url: None,
                    text: format!("{NAME} draft {app} {n} {t}"),
                    hash: *t + n as i64,
                    source: "ax".into(),
                })
                .unwrap()
                .unwrap();
            items.push(
                e.c.query_row("SELECT id FROM items WHERE source_table = 'capture_item' AND source_id = ?1", [cap], |r| r.get(0))
                    .unwrap(),
            );
        }
        e.c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, \
             start_item_id, end_item_id, item_count, summary, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('dev-a', ?1, 'w', ?2, ?3, ?4, ?5, ?6, 's', 1, 1)",
            params![app, ts[0], ts[ts.len() - 1], items[0], items[items.len() - 1], items.len() as i64],
        )
        .unwrap();
        let id = e.c.last_insert_rowid();
        file(e, topic, "episode", id, ts[0]);
        (id, items)
    }

    /// Review finding 2: an episode and its captures are one item on the review (one decision, one
    /// confirmation), and deleting its captures never takes another episode over the same time.
    #[test]
    fn an_episode_and_its_captures_are_one_unit() {
        let e = env();
        let w = world(&e);
        let b = e.store.create_seed_topic("Globex", &[], None).unwrap();
        let (ep, caps) = episode(&e, w.a.topic_id, "app", &[ago(14), ago(13)]);
        // B's episode (no period: kept) covers the same app and time.
        e.c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, \
             start_item_id, end_item_id, item_count, summary, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('dev-a', 'app', 'w', ?1, ?2, ?3, ?3, 1, 'b', 1, 1)",
            params![ago(14), ago(13), caps[0]],
        )
        .unwrap();
        let b_ep = e.c.last_insert_rowid();
        file(&e, b.topic_id, "episode", b_ep, ago(14));
        synced(&e);
        let due = due_of(&e, &w.a.key, NOW);
        assert!(due.contains(&(ItemKind::Episode, ep.to_string())));
        assert!(!due.iter().any(|(k, _)| *k == ItemKind::Capture), "captures are not offered on their own: {due:?}");
        // A capture confirmed on its own is not due: nothing goes.
        let lone = ExpirySelection {
            key: w.a.key.clone(),
            items: vec![ClientItem { kind: ItemKind::Capture, id: format!("i:{}", caps[0]), also_client: None, date_unix_ms: ago(14), removed: false, label: None, excerpt: None, app: None }],
            vault: None,
            vault_off: false,
        };
        let r = expire_items(&e.store, &lone, &[], NOW, true, LONG).unwrap();
        assert_eq!(r.not_due, vec![(ItemKind::Capture, 1)]);
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM episodes WHERE id = {ep}")), 1);
        // The episode confirmed: it and its captures go; B's episode over the same time stays.
        let sel = ExpirySelection {
            key: w.a.key.clone(),
            items: due_items_for(&e.store, &w.a.key, NOW).unwrap().into_iter().filter(|i| i.kind == ItemKind::Episode).collect(),
            vault: None,
            vault_off: false,
        };
        let r = expire_items(&e.store, &sel, &[], NOW, true, LONG).unwrap();
        assert!(r.complete(), "{r:?}");
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM episodes WHERE id = {ep}")), 0);
        for c in &caps {
            assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {c}")), 0);
        }
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM episodes WHERE id = {b_ep}")), 1);
    }

    /// Review finding 3: an expiry never touches what a failed client delete still owes.
    #[test]
    fn an_expiry_leaves_a_failed_client_deletes_owed_captures() {
        let e = env();
        let w = world(&e);
        synced(&e);
        let (_, caps) = episode(&e, w.a.topic_id, "owed", &[ago(1)]);
        let run = memory::Store::client_delete_run_id(&w.a.key);
        e.store.add_client_delete_pending(&run, memory::client_delete::PENDING_CAPTURE, &[caps[0].to_string()]).unwrap();
        expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM items WHERE id = {}", caps[0])), 1);
        assert_eq!(e.store.client_delete_pending(&run).unwrap().len(), 1, "still owed to the client delete");
    }

    /// Review finding 6: a period of 0 is no period: it never shields material from global retention.
    #[test]
    fn a_zero_period_does_not_shield_from_global_retention() {
        let e = env();
        let w = world(&e);
        set_keep(&e, &w.a.key, Some(0));
        let policy = memory::RetentionPolicy { capture_item_days: None, meeting_days: Some(1), conversation_days: None };
        crate::retention::apply_within(&e.store, &policy, NOW, &[e.dir.path()]).unwrap();
        assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM meeting WHERE id = {}", w.old_meeting)), 0);
    }

    /// Review finding 11: a clock set back does not re-raise the prompt on every sweep, and the flag
    /// follows decisions and expiries at once.
    #[test]
    fn a_clock_set_back_does_not_nag_and_the_flag_follows_decisions() {
        let e = env();
        let w = world(&e);
        synced(&e);
        refresh_review_flag(&e.store, NOW).unwrap();
        dismiss_review_flag(&e.store, NOW).unwrap();
        refresh_review_flag(&e.store, NOW - 10 * DAY).unwrap();
        assert!(review_flag(&e.store).is_none(), "clock set back 10 days");
        refresh_review_flag(&e.store, NOW - 9 * DAY).unwrap();
        assert!(review_flag(&e.store).is_none(), "the next sweep, still set back");
        refresh_review_flag(&e.store, NOW + PROMPT_INTERVAL_MS).unwrap();
        assert_eq!(review_flag(&e.store).unwrap().items, 2);
        let due = due_items_for(&e.store, &w.a.key, NOW + PROMPT_INTERVAL_MS).unwrap();
        set_retention_decision_at(&e.store, &w.a.key, &due[0], Decision::Keep, NOW + PROMPT_INTERVAL_MS).unwrap();
        assert_eq!(review_flag(&e.store).unwrap().items, 1, "a decision updates the standing flag");
        let sel = ExpirySelection { key: w.a.key.clone(), items: due[1..].to_vec(), vault: None, vault_off: false };
        expire_items(&e.store, &sel, &[], NOW, true, LONG).unwrap();
        assert!(review_flag(&e.store).is_none(), "nothing left to review");
    }

    /// Review findings 9 and 10: a slice that stops early leaves the index rebuild to the last slice, and
    /// every slice keeps its own receipt.
    #[test]
    fn slices_leave_the_rebuild_to_the_last_and_keep_their_receipts() {
        let e = env();
        let w = world(&e);
        let marker = email(&e, "plimsollword", ago(14), "t-p");
        file(&e, w.a.topic_id, "email", marker, ago(14));
        synced(&e);
        let in_pages = |e: &Env| {
            count(e, "SELECT COUNT(*) FROM items_fts_data WHERE instr(block, CAST('plimsollword' AS BLOB)) > 0")
        };
        let sel = select_all(&e, &w.a.key);
        let r = expire_items_until(&e.store, &sel, &[], NOW, true, &|n| n >= 1).unwrap();
        assert!(r.more);
        assert_eq!(r.counts, vec![(ItemKind::Email, 1)]);
        assert!(in_pages(&e) > 0, "no rebuild on a slice that stops early");
        let r = expire_items_until(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, &|_| false).unwrap();
        assert!(r.complete());
        assert_eq!(in_pages(&e), 0, "the last slice rebuilds");
        assert_eq!(e.store.list_meta_with_prefix(EXPIRY_RECEIPT_PREFIX).unwrap().len(), 2);
    }

    /// Review finding 4 (minimal): a later client delete says earlier expired items stay on other devices.
    #[test]
    fn a_client_delete_after_an_expiry_lists_the_expired_copies() {
        let e = env();
        let w = world(&e);
        synced(&e);
        let r = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert!(r.complete());
        let sel = crate::client_delete::DeleteSelection { key: w.a.key.clone(), ..Default::default() };
        let receipt = crate::client_delete::delete_client(&e.store, &sel, &[], None).unwrap();
        assert!(receipt.left_for_you.contains(&LeftForYou::ExpiredHereOnly(2)), "{:?}", receipt.left_for_you);
        // The count goes with the client.
        assert_eq!(count(&e, "SELECT COUNT(*) FROM kv_meta WHERE key LIKE 'client_expired:%'"), 0);
    }

    /// X4 design 3.5 step 8, as for a client delete: the encryption upgrade's plaintext rollback copy
    /// `<db>.plain.bak` still holds what an expiry deletes, so an expiry that deleted something removes it (and
    /// its sidecars). One that deleted nothing leaves it.
    #[test]
    fn an_expiry_that_deletes_rows_removes_the_plain_backup() {
        let e = env();
        let w = world(&e);
        synced(&e);
        let bak = crate::encrypt_migration::MigrationPaths::beside(&e.dir.path().join("m.db")).plain_backup();
        let bak_wal = std::path::PathBuf::from(format!("{}-wal", bak.display()));
        std::fs::write(&bak, format!("SQLite format 3\0 {NAME} old kickoff notes")).unwrap();
        std::fs::write(&bak_wal, NAME).unwrap();
        let empty = ExpirySelection { key: w.a.key.clone(), items: vec![], vault: None, vault_off: false };
        expire_items(&e.store, &empty, &[], NOW, true, LONG).unwrap();
        assert!(bak.exists(), "an expiry that deleted nothing leaves the copy");
        let r = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert!(r.complete(), "{r:?}");
        assert!(!r.counts.is_empty());
        assert!(!bak.exists() && !bak_wal.exists());
        let shown = format!("{r:?}{}", serde_json::to_string(&r).unwrap());
        assert!(!shown.contains("plain.bak"), "{shown}");
    }

    /// A rollback copy that would not go fails the expiry (never reads as complete), and the next run removes it
    /// even with nothing left to delete.
    #[cfg(unix)]
    #[test]
    fn a_plain_backup_that_will_not_go_is_removed_by_the_next_expiry() {
        use std::os::unix::fs::PermissionsExt;
        let e = env();
        let w = world(&e);
        synced(&e);
        let bak = crate::encrypt_migration::MigrationPaths::beside(&e.dir.path().join("m.db")).plain_backup();
        // A non-empty folder at the copy's name: removing a file there fails.
        std::fs::create_dir(&bak).unwrap();
        std::fs::write(bak.join("x"), b"x").unwrap();
        std::fs::set_permissions(&bak, std::fs::Permissions::from_mode(0o500)).unwrap();
        let r = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        std::fs::set_permissions(&bak, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!r.counts.is_empty(), "{r:?}");
        assert!(!r.complete() && r.failed_stages.contains(&DeleteStage::Files), "{r:?}");
        std::fs::remove_dir_all(&bak).unwrap();
        std::fs::write(&bak, NAME).unwrap();
        let again = expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert!(again.counts.is_empty() && again.complete(), "{again:?}");
        assert!(!bak.exists(), "the owed removal is retried");
        // Done: a later run that deletes nothing leaves a new copy alone.
        std::fs::write(&bak, NAME).unwrap();
        expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        assert!(bak.exists());
    }

    #[test]
    fn a_later_client_delete_still_finds_the_client() {
        let e = env();
        let w = world(&e);
        synced(&e);
        let item = due_items_for(&e.store, &w.a.key, NOW).unwrap().remove(0);
        set_retention_decision(&e.store, &w.a.key, &item, Decision::Keep).unwrap();
        expire_items(&e.store, &select_all(&e, &w.a.key), &[], NOW, true, LONG).unwrap();
        let sel = crate::client_delete::DeleteSelection { key: w.a.key.clone(), ..Default::default() };
        let receipt = crate::client_delete::delete_client(&e.store, &sel, &[], None).unwrap();
        assert!(receipt.complete(), "{receipt:?}");
        assert_eq!(count(&e, "SELECT COUNT(*) FROM kv_meta WHERE key LIKE 'client_retention:%'"), 0, "decisions go with the client");
    }
}
