//! The client controls as the shells call them (X6 client controls, Task 9): one function per operation,
//! JSON-shaped input and output, shared by the Mac's C ABI (`core/ffi`) and the Windows commands so the
//! logic is in one compiled, tested place.
//!
//! - Every operation returns [`OpResult`]; [`envelope`] turns it into `{"ok":true,"data":..}` or
//!   `{"ok":false,"error":{"kind":..}}` with a [`ClientOpError`] kind. No error carries a client's name.
//! - Items cross the boundary as [`ItemRef`] (`{"kind","id"}`) only. The engines match them against a fresh
//!   inventory (or a fresh list of due items) and act on those, never on what the caller sent: a forged or
//!   stale item is never deleted, and a still-stored one counts as failed in the receipt.
//! - A client delete or expiry holds [`JobPause`] while it runs: the shells' job loops (summaries, judging,
//!   steps, routines, retention, episodes, threads) take a [`JobTicket`] per tick and skip the tick while a
//!   pause is held. The pause waits a bounded time for ticks already running to finish.
//! - Anything that can change what is excluded from hosted models republishes the streaming gate
//!   ([`crate::exclusion::refresh_published_gate`]): the control write and the delete here; the shells' sync
//!   rounds call [`after_sync_round`] (a peer's control change or client purge arrives that way).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::client_delete::{delete_client, DeleteSelection, DeleteStage, Receipt};
use crate::client_export::{ClientExport, ExportOptions, MediaCopied};
use crate::client_inventory::{inventory_in, inventory_lean, ClientInventory, ClientItem, ItemKind, KnownFolders};
use crate::client_retention::{self, Decision, DueClient, ExpiryReceipt, ExpirySelection, ReviewFlag};
use crate::envelope::ApiError;

/// Why an operation did not run. Never names a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientOpError {
    /// The key is not a live client (unknown, deleted, or merged into another).
    NotFound,
    /// A client delete (or, with sync on, a keep-for expiry) was refused before it started: this device has no
    /// sync id, so it could not tell its own meeting files and unsent rows from another device's. Not retryable
    /// until the device has its id.
    NoDeviceId,
    /// The request could not be read (bad JSON, an unreadable sync key): a programming error on the caller's side.
    InvalidInput,
    /// A ticked item is no longer in the client's inventory (a peer's delete arrived between review and
    /// confirm, or the item was never the client's). Nothing was touched: reload the review screen.
    SelectionChanged,
    /// Background work did not finish in time for the delete to start safely. Nothing was touched: try again
    /// in a moment.
    Busy,
    /// The user cancelled while the delete (or expiry) waited for background work. Nothing was touched.
    Cancelled,
    /// An export may not go to a network share (a `\\server\share` path): a client's whole material would
    /// leave the computer. Nothing was written.
    FolderNotAllowed,
    /// The database or a folder could not be read or written. Running it again may help.
    Failed,
}

impl From<ApiError> for ClientOpError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::NotFound => ClientOpError::NotFound,
            // The client modules report a database or folder error as a provider failure; nothing here calls
            // a model, so it is never one.
            _ => ClientOpError::Failed,
        }
    }
}

impl From<rusqlite::Error> for ClientOpError {
    fn from(_: rusqlite::Error) -> Self {
        ClientOpError::Failed
    }
}

pub type OpResult<T> = Result<T, ClientOpError>;

/// `{"ok":true,"data":..}` or `{"ok":false,"error":{"kind":..}}`.
pub fn envelope<T: Serialize>(r: &OpResult<T>) -> String {
    let v = match r {
        Ok(data) => match serde_json::to_value(data) {
            Ok(data) => serde_json::json!({ "ok": true, "data": data }),
            Err(_) => serde_json::json!({ "ok": false, "error": ClientOpError::Failed }),
        },
        Err(e) => serde_json::json!({ "ok": false, "error": e }),
    };
    v.to_string()
}

/// Parses a request, mapping bad JSON to [`ClientOpError::InvalidInput`].
pub fn parse<T: serde::de::DeserializeOwned>(json: &str) -> OpResult<T> {
    serde_json::from_str(json).map_err(|_| ClientOpError::InvalidInput)
}

/// One item as the shell names it: an item's `kind` and `id` from the inventory (a capture's id may be
/// `i:<items.id>`). Anything else the inventory said about it is not read back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ItemRef {
    pub kind: ItemKind,
    pub id: String,
}

impl ItemRef {
    /// A bare item for the engines, which match it against their own fresh list by kind and id.
    fn bare(&self) -> ClientItem {
        ClientItem { kind: self.kind, id: self.id.clone(), also_client: None, date_unix_ms: 0, removed: false, label: None, excerpt: None, app: None }
    }
}

/// Folders only the shell knows.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Folders {
    /// The only folders meeting files are deleted from (or copied from, for an export). The first is the media
    /// folder the "left for you" list names. Shells pass the media folder setting, then the recordings folder.
    #[serde(default)]
    pub media_roots: Vec<PathBuf>,
    /// The Obsidian vault folder (it holds `Whimbrel/`), when the user has one.
    #[serde(default)]
    pub vault: Option<PathBuf>,
    /// The Obsidian vault integration is off on this device. Then the vault is never touched, even when `vault`
    /// is sent (shells should send none), and "left for you" says vault sync is off (Mike: vault notes are deleted
    /// only while vault sync is on).
    #[serde(default)]
    pub vault_sync_off: bool,
}

impl Folders {
    fn known(&self) -> KnownFolders {
        KnownFolders { media: self.media_roots.first().cloned(), vault: self.vault_to_touch(), vault_off: self.vault_sync_off }
    }

    /// The vault a delete or expiry may remove notes from: none while vault sync is off.
    pub fn vault_to_touch(&self) -> Option<PathBuf> {
        self.vault.clone().filter(|_| !self.vault_sync_off)
    }
}

// ---------------------------------------------------------------------------------------------------------
// Pausing the job loops while a client delete runs.

#[derive(Default)]
struct PauseState {
    /// Job ticks running now.
    running: usize,
    /// Deletes (or expiries) running now.
    pauses: usize,
}

static PAUSE: Mutex<PauseState> = Mutex::new(PauseState { running: 0, pauses: 0 });
static PAUSE_CHANGED: Condvar = Condvar::new();

fn pause_state() -> MutexGuard<'static, PauseState> {
    PAUSE.lock().unwrap_or_else(|e| e.into_inner())
}

/// How long a delete waits for job ticks already running; past it the delete answers `busy`.
pub const PAUSE_WAIT: Duration = Duration::from_secs(120);

/// Held by a job loop for one tick. Dropping it ends the tick.
#[must_use = "the tick runs while the ticket is held"]
pub struct JobTicket(());

impl Drop for JobTicket {
    fn drop(&mut self) {
        let mut s = pause_state();
        s.running = s.running.saturating_sub(1);
        PAUSE_CHANGED.notify_all();
    }
}

/// A ticket for one job tick, or `None` while a client delete runs (skip the tick and try later).
pub fn job_ticket() -> Option<JobTicket> {
    let mut s = pause_state();
    if s.pauses > 0 {
        return None;
    }
    s.running += 1;
    Some(JobTicket(()))
}

/// [`job_ticket`] for a shell that cannot hold the ticket across its call boundary (the Mac's C ABI): `true`
/// = the tick may run, and exactly one [`job_end`] must follow; `false` = skip the tick.
pub fn job_begin() -> bool {
    match job_ticket() {
        Some(t) => {
            std::mem::forget(t);
            true
        }
        None => false,
    }
}

/// Ends a tick [`job_begin`] allowed.
pub fn job_end() {
    drop(JobTicket(()));
}

/// Whether job loops are paused now.
pub fn jobs_paused() -> bool {
    pause_state().pauses > 0
}

/// Held while a client delete or expiry runs; dropping it (also when the delete panics) resumes the loops.
#[must_use = "the loops resume when the pause is dropped"]
pub struct JobPause(());

impl Drop for JobPause {
    fn drop(&mut self) {
        let mut s = pause_state();
        s.pauses = s.pauses.saturating_sub(1);
        PAUSE_CHANGED.notify_all();
    }
}

/// How a delete or expiry waits for the ticks already running.
#[derive(Clone, Copy)]
pub enum PauseWait<'a> {
    /// At most this long, then [`ClientOpError::Busy`].
    Limit(Duration),
    /// As long as it takes, until `cancel` is set ([`ClientOpError::Cancelled`]). The shell shows that it is
    /// waiting (see [`jobs_running`]) and offers Cancel.
    Until(&'a AtomicBool),
}

impl From<Duration> for PauseWait<'_> {
    fn from(d: Duration) -> Self {
        PauseWait::Limit(d)
    }
}

/// How often a cancellable wait looks at its flag.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Job ticks running now (what a delete that waits is waiting for).
pub fn jobs_running() -> usize {
    pause_state().running
}

/// [`pause_jobs`] with a choice of waiting. No new tick starts from the first moment, whichever way it waits.
/// The pause is a guard from the start, so every way out (busy, cancelled, a panic) releases it.
pub fn pause_jobs_wait(wait: PauseWait<'_>) -> OpResult<JobPause> {
    let deadline = match wait {
        PauseWait::Limit(d) => Some(Instant::now() + d),
        PauseWait::Until(_) => None,
    };
    let cancelled = || matches!(wait, PauseWait::Until(c) if c.load(Ordering::SeqCst));
    pause_state().pauses += 1;
    let guard = JobPause(());
    let mut s = pause_state();
    loop {
        if cancelled() {
            drop(s);
            return Err(ClientOpError::Cancelled);
        }
        if s.running == 0 {
            drop(s);
            return Ok(guard);
        }
        let slice = match deadline {
            Some(d) => {
                let left = d.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    drop(s);
                    return Err(ClientOpError::Busy);
                }
                left.min(CANCEL_POLL)
            }
            None => CANCEL_POLL,
        };
        s = PAUSE_CHANGED.wait_timeout(s, slice).unwrap_or_else(|e| e.into_inner()).0;
    }
}

/// Pauses the job loops: no new tick starts, and this waits up to `wait` for the ticks already running to
/// finish. `None` when they have not finished by then: the pause is dropped again (the loops go on) and the
/// caller must not start, since a tick finishing after a delete could write the client's text again.
pub fn pause_jobs(wait: Duration) -> Option<JobPause> {
    pause_jobs_wait(PauseWait::Limit(wait)).ok()
}

// ---------------------------------------------------------------------------------------------------------
// Inventory and delete.

/// What is linked to client `key`, what only names it, and the places Whimbrel cannot reach.
pub fn inventory(store: &memory::Store, key: &str, folders: &Folders) -> OpResult<ClientInventory> {
    Ok(inventory_in(store, key, &folders.known())?)
}

/// What the user confirmed on the review screen.
#[derive(Debug, Clone, Deserialize)]
pub struct DeleteRequest {
    pub key: String,
    /// The possible matches the user ticked; unticked ones are never deleted. Required (an empty list for none),
    /// so a misspelled key never reads as "none ticked".
    pub include_possible: Vec<ItemRef>,
    /// Also delete the export folders Whimbrel recorded for this client.
    #[serde(default)]
    pub delete_exports: bool,
    #[serde(flatten)]
    pub folders: Folders,
}

/// A [`DeleteRequest`] for [`delete_with_pause`], with the linked items the user saw on the review screen. When
/// given, the delete re-reads the linked set after it has the job pause and answers
/// [`ClientOpError::SelectionChanged`] if it is not exactly this (an item filed under the client during a long
/// wait would otherwise be deleted unseen). Absent: not checked. A separate type so [`DeleteRequest`] keeps its
/// fields for the shells that build it directly.
#[derive(Debug, Clone, Deserialize)]
pub struct HeldDeleteRequest {
    #[serde(flatten)]
    pub request: DeleteRequest,
    #[serde(default)]
    pub reviewed_linked: Option<Vec<ItemRef>>,
}

/// The receipt, with what the screen needs to decide on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeleteDone {
    #[serde(flatten)]
    pub receipt: Receipt,
    /// Nothing failed: every selected item, the derived text and the client record are gone.
    pub complete: bool,
    /// Running the same selection again can finish it (`false` when only a missing device id is in the way).
    pub retry_helps: bool,
}

/// Deletes client `req.key` and its material on this device and, through tombstones, on the others.
///
/// Refused with nothing touched:
/// - [`ClientOpError::NoDeviceId`]: no device id (none given and none set before, or the legacy
///   [`memory::LEGACY_SPIKE_DEVICE`]). `device_id` is set on `store` first when it is a real one.
/// - [`ClientOpError::NotFound`]: not a live client.
/// - [`ClientOpError::SelectionChanged`]: a ticked item a fresh inventory does not list.
/// - [`ClientOpError::Busy`]: background ticks did not finish within [`PAUSE_WAIT`].
///
/// `group_key`: the sync group key when sync is set up (relay clean-up targets are sealed at once), else
/// `None`. Holds [`JobPause`] for the whole run. Slow (it rebuilds the search indexes): never call it on a
/// thread the UI waits on.
pub fn delete(
    store: &memory::Store,
    req: &DeleteRequest,
    device_id: Option<&str>,
    group_key: Option<&sync::GroupKey>,
) -> OpResult<DeleteDone> {
    delete_waiting(store, req, device_id, group_key, PAUSE_WAIT)
}

/// [`delete`] that waits for running ticks as long as it takes, until `cancel` is set (then
/// [`ClientOpError::Cancelled`], nothing touched). While it waits no new tick starts and [`jobs_running`] says
/// how many are left. Every way out releases the pause.
pub fn delete_cancellable(
    store: &memory::Store,
    req: &DeleteRequest,
    device_id: Option<&str>,
    group_key: Option<&sync::GroupKey>,
    cancel: &AtomicBool,
) -> OpResult<DeleteDone> {
    delete_waiting(store, req, device_id, group_key, PauseWait::Until(cancel))
}

fn real_device_id(id: &str) -> Option<&str> {
    let id = id.trim();
    (!id.is_empty() && id != memory::LEGACY_SPIKE_DEVICE).then_some(id)
}

/// [`delete`] waiting at most `wait` for running ticks.
pub(crate) fn delete_waiting<'a>(
    store: &memory::Store,
    req: &DeleteRequest,
    device_id: Option<&str>,
    group_key: Option<&sync::GroupKey>,
    wait: impl Into<PauseWait<'a>>,
) -> OpResult<DeleteDone> {
    if let Some(id) = device_id.and_then(real_device_id) {
        store.set_own_device_id(id);
    }
    if store.own_device_id().filter(|id| real_device_id(id).is_some()).is_none() {
        return Err(ClientOpError::NoDeviceId);
    }
    let pause = pause_jobs_wait(wait.into())?;
    let held = HeldDeleteRequest { request: req.clone(), reviewed_linked: None };
    delete_with_pause(store, &held, device_id, group_key, pause)
}

/// The delete itself, with the job pause already held (see [`pause_jobs_wait`]): a shell can wait for the
/// running ticks on a thread that holds no store, then run this on the slow lane. The pause is released when
/// this returns, on every path. After the pause is held the selection is checked against a fresh lean
/// inventory: every ticked item must be a possible match it lists, and when `held.reviewed_linked` is given the
/// linked set must be exactly that ([`ClientOpError::SelectionChanged`] otherwise, nothing touched).
pub fn delete_with_pause(
    store: &memory::Store,
    held: &HeldDeleteRequest,
    device_id: Option<&str>,
    group_key: Option<&sync::GroupKey>,
    pause: JobPause,
) -> OpResult<DeleteDone> {
    let req = &held.request;
    if let Some(id) = device_id.and_then(real_device_id) {
        store.set_own_device_id(id);
    }
    let Some(own) = store.own_device_id().filter(|id| real_device_id(id).is_some()) else {
        return Err(ClientOpError::NoDeviceId);
    };
    // Every ticked item must be one a fresh inventory lists. The engine intersects again (defence in depth, for
    // anything that changes between here and there).
    let inv = inventory_lean(store, &req.key, &req.folders.known())?;
    let listed = |r: &ItemRef| inv.possible.iter().chain(inv.linked.iter()).any(|i| i.kind == r.kind && i.id == r.id);
    if !req.include_possible.iter().all(listed) {
        return Err(ClientOpError::SelectionChanged);
    }
    if let Some(seen) = &held.reviewed_linked {
        let now: std::collections::HashSet<(ItemKind, &str)> = inv.linked.iter().map(|i| (i.kind, i.id.as_str())).collect();
        let before: std::collections::HashSet<(ItemKind, &str)> = seen.iter().map(|r| (r.kind, r.id.as_str())).collect();
        if now != before {
            return Err(ClientOpError::SelectionChanged);
        }
    }
    let sel = DeleteSelection {
        key: req.key.clone(),
        include_possible: req.include_possible.iter().map(ItemRef::bare).collect(),
        delete_exports: req.delete_exports,
        vault: req.folders.vault_to_touch(),
        vault_off: req.folders.vault_sync_off,
    };
    let receipt = {
        let _pause = pause;
        delete_client(store, &sel, &req.folders.media_roots, group_key.map(|k| (k, own.as_str())))
    };
    // The client's names may have been in the excluded set: the streaming gate is read again either way.
    crate::exclusion::refresh_published_gate(store);
    let receipt = lossy_paths(receipt?);
    let complete = receipt.complete();
    let retry_helps = !receipt.failed.is_empty() || receipt.failed_stages.iter().any(|s| *s != DeleteStage::NoDeviceId);
    Ok(DeleteDone { receipt, complete, retry_helps })
}

/// Paths as text the receipt can always be written as (a path that is not Unicode would otherwise make a
/// finished delete read as failed).
fn lossy_paths(mut r: Receipt) -> Receipt {
    use crate::client_inventory::LeftForYou as L;
    let fix = |p: &mut PathBuf| *p = PathBuf::from(p.to_string_lossy().into_owned());
    for l in r.left_for_you.iter_mut() {
        match l {
            L::MediaFolder(p) | L::VaultFolder(p) => fix(p),
            L::ExportFolders(ps) | L::FilesNotRemoved(ps) => ps.iter_mut().for_each(fix),
            L::MailboxOriginals
            | L::BackupsAndCloud
            | L::RelayCopiesBeforeThisRelease
            | L::CopiesOnOtherDevices
            | L::ExpiredHereOnly(_)
            | L::VaultSyncOff => {}
        }
    }
    r
}

/// Devices on the sync relay that still need to update before they apply client deletes; `None` when not
/// known yet (never show it as 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PendingDevices {
    pub pending_devices: Option<usize>,
    /// Devices on the relay that hold back this device's client records (no topics support yet; a removed device
    /// too), and with them the exclude setting: while more than 0, another device may not learn that a client is
    /// excluded and can still send its material to a hosted model. For a warning on the exclude toggle. `None`
    /// when not known yet (no outbox pass measured it, or unreadable): never shown as 0.
    pub topic_rows_held_devices: Option<usize>,
}

pub fn pending_devices(store: &memory::Store) -> PendingDevices {
    PendingDevices {
        pending_devices: store.pending_tombstone_devices().ok().flatten(),
        topic_rows_held_devices: store.topic_rows_held_devices().ok().flatten(),
    }
}

/// The devices [`pending_devices`] counts, by sync device id (the device's public key; the shells name them from
/// their device list). `None` when not known yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingDeviceIds {
    pub devices: Option<Vec<String>>,
}

pub fn pending_device_ids(store: &memory::Store) -> PendingDeviceIds {
    PendingDeviceIds { devices: store.pending_tombstone_device_ids().ok().flatten() }
}

/// "Remove this device": stops counting `device_id` (a retired device that will never update) in the devices still
/// to update, here at once and on every device after this desktop's next sync round. It releases nothing: client
/// deletes are never held for a device. [`ClientOpError::NotFound`] when it is this device or not one of those
/// listed. For the desktops' Settings only: phones never call it, and their pull-only round never publishes it.
pub fn retire_device(store: &memory::Store, device_id: &str) -> OpResult<PendingDevices> {
    match store.queue_retired_device(device_id)? {
        true => Ok(pending_devices(store)),
        false => Err(ClientOpError::NotFound),
    }
}

// ---------------------------------------------------------------------------------------------------------
// Settings (exclude from hosted models, keep for N months).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlView {
    pub exclude_hosted: bool,
    /// Months to keep the client's material; `None` (or 0) keeps it until the global settings remove it.
    #[serde(default)]
    pub keep_months: Option<u32>,
}

fn live_client(store: &memory::Store, key: &str) -> OpResult<()> {
    match store.list_seed_topics()?.iter().any(|s| s.key == key) {
        true => Ok(()),
        false => Err(ClientOpError::NotFound),
    }
}

pub fn control_get(store: &memory::Store, key: &str) -> OpResult<ControlView> {
    live_client(store, key)?;
    let c = store.client_control(key)?;
    Ok(ControlView { exclude_hosted: c.exclude_hosted, keep_months: c.keep_months })
}

/// Saves client `key`'s settings (they sync with the client's record) and republishes the streaming gate.
pub fn control_set(store: &memory::Store, key: &str, v: &ControlView) -> OpResult<ControlView> {
    live_client(store, key)?;
    let saved = store.set_client_control(
        key,
        &memory::topics::client_controls::ClientControl { exclude_hosted: v.exclude_hosted, keep_months: v.keep_months },
    );
    crate::exclusion::refresh_published_gate(store);
    saved?;
    control_get(store, key)
}

/// One setting to change, as `{"exclude_hosted": bool}` or `{"keep_months": n | null}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlChange {
    ExcludeHosted(bool),
    KeepMonths(Option<u32>),
}

/// A settings write: the value it started from (read in the same call) and the value saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ControlUpdate {
    pub before: ControlView,
    pub after: ControlView,
}

/// Reads client `key`'s settings and saves `change` applied to them, in one call, so a value another device synced
/// since the page loaded is never overwritten with a stale one (the shell runs it as one job on the store, where
/// sync rounds also run). Unchanged values are not written.
pub fn control_update(store: &memory::Store, key: &str, change: &ControlChange) -> OpResult<ControlUpdate> {
    let before = control_get(store, key)?;
    let mut next = before;
    match *change {
        ControlChange::ExcludeHosted(on) => next.exclude_hosted = on,
        ControlChange::KeepMonths(m) => next.keep_months = m.filter(|m| *m > 0),
    }
    if next == before {
        return Ok(ControlUpdate { before, after: before });
    }
    Ok(ControlUpdate { before, after: control_set(store, key, &next)? })
}

/// What turning exclusion on offers: the client's meetings a hosted model summarised, which can be summarised
/// again on this computer (`summarize_meeting` keeps an excluded client's meeting local), and the sentence that
/// material which already left cannot be recalled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResummaryOffer {
    pub meetings: Vec<i64>,
    pub already_left_note: &'static str,
}

pub fn resummary_offer(store: &memory::Store, key: &str) -> OpResult<ResummaryOffer> {
    live_client(store, key)?;
    Ok(ResummaryOffer {
        meetings: crate::exclusion::hosted_summaries_to_redo(store, key)?,
        already_left_note: crate::exclusion::ALREADY_LEFT_NOTE,
    })
}

/// After every content-sync round, successful or not (a partial pull may have applied rows): a peer's control
/// change or client purge arrives through it, so the streaming gate is read again.
pub fn after_sync_round(store: &memory::Store) {
    crate::exclusion::refresh_published_gate(store);
}

// ---------------------------------------------------------------------------------------------------------
// Keep for N months.

/// Every client's items past its keep period that wait for the user's review. Read only.
pub fn due_items(store: &memory::Store, now_ms: i64) -> OpResult<Vec<DueClient>> {
    Ok(client_retention::due_items(store, now_ms)?)
}

/// One client's due items. Read only.
pub fn due_items_for(store: &memory::Store, key: &str, now_ms: i64) -> OpResult<Vec<ClientItem>> {
    Ok(client_retention::due_items_for(store, key, now_ms)?)
}

/// "Keep these" (`{"decision":"keep"}`) or "Snooze" (`{"decision":"snooze","until_ms":..}`) for one item, on this
/// device only.
pub fn retention_decide(store: &memory::Store, key: &str, item: &ItemRef, d: Decision, now_ms: i64) -> OpResult<()> {
    Ok(client_retention::set_retention_decision_at(store, key, &item.bare(), d, now_ms)?)
}

/// The weekly "items ready to review" flag (counts only), if raised.
pub fn review_flag(store: &memory::Store) -> Option<ReviewFlag> {
    client_retention::review_flag(store)
}

/// "Later": puts the review prompt off for a week.
pub fn dismiss_review_flag(store: &memory::Store, now_ms: i64) -> OpResult<()> {
    Ok(client_retention::dismiss_review_flag(store, now_ms)?)
}

/// The items the user confirmed for deletion on the review screen.
#[derive(Debug, Clone, Deserialize)]
pub struct ExpireRequest {
    pub key: String,
    pub items: Vec<ItemRef>,
    #[serde(flatten)]
    pub folders: Folders,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExpireDone {
    #[serde(flatten)]
    pub receipt: ExpiryReceipt,
    pub complete: bool,
}

/// Deletes the confirmed items that are still due, on this device only (retention is per device). Stops
/// starting new items after `budget` (`more` in the receipt: call again). `sync_on`: sync is set up here, so
/// material not sent to other devices yet waits; then the store must know this device's id (set it first with
/// `Store::set_own_device_id`), else [`ClientOpError::NoDeviceId`] with nothing touched. Holds [`JobPause`]
/// while it runs; slow, like the delete.
pub fn expire(
    store: &memory::Store,
    req: &ExpireRequest,
    now_ms: i64,
    sync_on: bool,
    budget: Duration,
) -> OpResult<ExpireDone> {
    expire_waiting(store, req, now_ms, sync_on, budget, PAUSE_WAIT)
}

/// [`expire`] that waits for running ticks until `cancel` is set, like [`delete_cancellable`].
pub fn expire_cancellable(
    store: &memory::Store,
    req: &ExpireRequest,
    now_ms: i64,
    sync_on: bool,
    budget: Duration,
    cancel: &AtomicBool,
) -> OpResult<ExpireDone> {
    expire_waiting(store, req, now_ms, sync_on, budget, PauseWait::Until(cancel))
}

/// [`expire`] waiting at most `wait` for running ticks ([`ClientOpError::Busy`] past it, nothing touched).
pub(crate) fn expire_waiting<'a>(
    store: &memory::Store,
    req: &ExpireRequest,
    now_ms: i64,
    sync_on: bool,
    budget: Duration,
    wait: impl Into<PauseWait<'a>>,
) -> OpResult<ExpireDone> {
    // With sync on, an item this device has not sent yet must wait, and only the device id tells this device's
    // rows from a peer's: without one, refused like the delete (X6 final review, Major 21). The shell sets the id
    // on the store first (`Store::set_own_device_id`).
    if sync_on && store.own_device_id().filter(|id| real_device_id(id).is_some()).is_none() {
        return Err(ClientOpError::NoDeviceId);
    }
    // The engine orders by date: hand it the due list's own items (with their dates). One it no longer lists
    // is passed bare and counted as no longer due.
    let due = client_retention::due_items_for(store, &req.key, now_ms)?;
    let items = req
        .items
        .iter()
        .map(|r| due.iter().find(|d| d.kind == r.kind && d.id == r.id).cloned().unwrap_or_else(|| r.bare()))
        .collect();
    let sel = ExpirySelection {
        key: req.key.clone(),
        items,
        vault: req.folders.vault_to_touch(),
        vault_off: req.folders.vault_sync_off,
    };
    let receipt = {
        let _pause = pause_jobs_wait(wait.into())?;
        client_retention::expire_items(store, &sel, &req.folders.media_roots, now_ms, sync_on, budget)?
    };
    let complete = receipt.complete();
    Ok(ExpireDone { receipt, complete })
}

// ---------------------------------------------------------------------------------------------------------
// Export.

#[derive(Debug, Clone, Deserialize)]
pub struct ExportRequest {
    pub key: String,
    /// The folder the user chose; a new `Whimbrel client export <date>` folder is made inside it.
    pub dest_parent: PathBuf,
    #[serde(default)]
    pub include_possible: Vec<ItemRef>,
    /// Copy meeting audio and slides too (only from `media_roots`).
    #[serde(default)]
    pub include_media: bool,
    #[serde(default)]
    pub media_roots: Vec<PathBuf>,
}

/// The text part is written: the folder exists. `media` is what is still to copy (empty: nothing).
pub struct ExportWritten {
    pub export: ClientExport,
    pub media_roots: Vec<PathBuf>,
}

/// The part of an export that needs the store: makes and records the folder and writes the text files. Copy
/// the media afterwards with [`export_copy_media`], with the store released.
pub fn export_write(store: &memory::Store, req: &ExportRequest, now_ms: i64) -> OpResult<ExportWritten> {
    if is_network_path(&req.dest_parent) {
        return Err(ClientOpError::FolderNotAllowed);
    }
    let opts = ExportOptions {
        include_possible: req.include_possible.iter().map(ItemRef::bare).collect(),
        include_media: req.include_media,
        media_roots: req.media_roots.clone(),
    };
    let export = crate::client_export::write_client_export(store, &req.key, &req.dest_parent, &opts, now_ms)?;
    Ok(ExportWritten { export, media_roots: req.media_roots.clone() })
}

/// A UNC path (`\\server\share`, `//server/share`, `\\?\UNC\..`). A drive letter mapped to a share is not
/// detected here.
fn is_network_path(p: &std::path::Path) -> bool {
    let s = p.to_string_lossy();
    let lower = s.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") || lower.starts_with("//?/unc/") {
        return true;
    }
    if lower.starts_with(r"\\?\") || lower.starts_with(r"\\.\") {
        return false; // a local device path such as \\?\C:\
    }
    (s.starts_with(r"\\") || s.starts_with("//")) && s.len() > 2
}

/// How an export ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportDone {
    pub folder: PathBuf,
    pub media: MediaCopiedView,
    /// The text is complete, but some media could not be copied.
    pub partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MediaCopiedView {
    pub files: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl From<&MediaCopied> for MediaCopiedView {
    fn from(m: &MediaCopied) -> Self {
        MediaCopiedView { files: m.files, skipped: m.skipped, failed: m.failed }
    }
}

/// Copies an export's media; needs no store. Media that would not copy does not fail the export: the text is
/// complete and `partial` says so.
pub fn export_copy_media(written: &ExportWritten) -> OpResult<ExportDone> {
    let media = if written.export.media.is_empty() {
        MediaCopied::default()
    } else {
        crate::client_export::copy_client_media(&written.export, &written.media_roots)?
    };
    let done = crate::client_export::ExportDone { folder: written.export.folder.clone(), media };
    Ok(ExportDone { partial: done.partial(), media: (&done.media).into(), folder: done.folder })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    const NAME: &str = "Quuxwell";

    /// Tests that take or check the job pause run one at a time: it is process-wide.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct Env {
        _dir: tempfile::TempDir,
        root: PathBuf,
        store: memory::Store,
        c: Connection,
        key: String,
        topic: i64,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let store = crate::test_store::open(&path, dir.path());
        let c = crate::test_store::raw(&path);
        let seed = store.create_seed_topic(NAME, &[], None).unwrap();
        Env { root: dir.path().to_path_buf(), _dir: dir, store, c, key: seed.key, topic: seed.topic_id }
    }

    fn email(e: &Env, ext: &str, subject: &str) -> i64 {
        e.store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "dev-a".into(),
                external_id: ext.into(),
                ts_unix_ms: 1_000,
                from: "pat@x.com".into(),
                to: "me@x.com".into(),
                subject: subject.into(),
                body_text: subject.into(),
                gmail_thread_id: format!("t-{ext}"),
                account_email: "me@x.com".into(),
                gmail_labels: vec![],
                list_unsubscribe: false,
            }])
            .unwrap();
        e.c.query_row("SELECT id FROM items WHERE kind = 'email' AND external_id = ?1", [ext], |r| r.get(0)).unwrap()
    }

    fn file_under_client(e: &Env, kind: &str, r: i64) {
        e.c.execute(
            "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, 1000, 1)",
            params![kind, r],
        )
        .unwrap();
        let sid = e.c.last_insert_rowid();
        e.c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![e.topic, sid])
            .unwrap();
    }

    fn exists(e: &Env, id: i64) -> bool {
        e.c.query_row("SELECT count(*) FROM items WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).unwrap() > 0
    }

    fn json<T: Serialize>(r: &OpResult<T>) -> serde_json::Value {
        serde_json::from_str(&envelope(r)).unwrap()
    }

    /// X6 final review, Major 21: with sync on, an expiry on a store with no device id (none set, or the legacy
    /// placeholder) is refused like the delete, nothing touched. With sync off it runs.
    #[test]
    fn an_expiry_with_sync_on_and_no_device_id_is_refused() {
        let _s = serial();
        for placeholder in [false, true] {
            let e = env();
            if placeholder {
                e.store.set_own_device_id(memory::LEGACY_SPIKE_DEVICE);
            }
            let linked = email(&e, "m1", "pricing");
            file_under_client(&e, "email", linked);
            control_set(&e.store, &e.key, &ControlView { exclude_hosted: false, keep_months: Some(1) }).unwrap();
            let now = 100 * 86_400_000;
            let x: ExpireRequest =
                parse(&format!(r#"{{"key":"{}","items":[{{"kind":"email","id":"{linked}"}}]}}"#, e.key)).unwrap();
            let r = expire(&e.store, &x, now, true, Duration::from_secs(5));
            assert_eq!(json(&r), serde_json::json!({"ok": false, "error": {"kind": "no_device_id"}}));
            assert!(exists(&e, linked), "nothing deleted");
            assert_eq!(json(&expire(&e.store, &x, now, false, Duration::from_secs(5)))["ok"], true);
            assert!(!exists(&e, linked));
        }
    }

    #[test]
    fn a_delete_without_a_device_id_is_refused_and_touches_nothing() {
        let _s = serial();
        let e = env();
        let linked = email(&e, "m1", "pricing");
        file_under_client(&e, "email", linked);
        let req: DeleteRequest = parse(&format!(r#"{{"key":"{}","include_possible":[]}}"#, e.key)).unwrap();
        for id in [None, Some(""), Some("   ")] {
            let r = delete(&e.store, &req, id, None);
            assert_eq!(r.as_ref().err(), Some(&ClientOpError::NoDeviceId));
            assert_eq!(json(&r), serde_json::json!({"ok": false, "error": {"kind": "no_device_id"}}));
        }
        assert!(exists(&e, linked), "nothing deleted");
        assert!(e.store.list_seed_topics().unwrap().iter().any(|s| s.key == e.key), "client kept");
    }

    #[test]
    fn delete_sets_the_device_id_deletes_what_is_listed_and_never_a_forged_item() {
        let _s = serial();
        let e = env();
        let linked = email(&e, "m1", "pricing");
        file_under_client(&e, "email", linked);
        let possible = email(&e, "m2", &format!("{NAME} kickoff"));
        let unticked = email(&e, "m3", &format!("lunch with {NAME}"));
        let forged = email(&e, "m4", "someone else entirely");
        let inv = inventory(&e.store, &e.key, &Folders::default()).unwrap();
        assert!(inv.possible.iter().any(|p| p.id == possible.to_string()));
        assert!(inv.possible.iter().any(|p| p.id == unticked.to_string()));

        // An item no inventory lists: refused up front, nothing touched.
        let forged_req: DeleteRequest = parse(&format!(
            r#"{{"key":"{}","include_possible":[{{"kind":"email","id":"{possible}"}},{{"kind":"email","id":"{forged}"}}],"delete_exports":false,"media_roots":[],"vault":null}}"#,
            e.key
        ))
        .unwrap();
        let r = delete(&e.store, &forged_req, Some("dev-a"), None);
        assert_eq!(r.as_ref().err(), Some(&ClientOpError::SelectionChanged));
        assert_eq!(json(&r)["error"]["kind"], "selection_changed");
        assert!(exists(&e, linked) && exists(&e, possible) && exists(&e, forged), "nothing touched");
        assert!(!jobs_paused());

        let req: DeleteRequest = parse(&format!(
            r#"{{"key":"{}","include_possible":[{{"kind":"email","id":"{possible}"}}],"delete_exports":false,"media_roots":[],"vault":null}}"#,
            e.key
        ))
        .unwrap();
        let done = delete(&e.store, &req, Some("dev-a"), None).unwrap();
        assert_eq!(e.store.own_device_id().as_deref(), Some("dev-a"));
        assert!(!exists(&e, linked) && !exists(&e, possible), "linked and ticked go");
        assert!(exists(&e, unticked), "an unticked possible match stays");
        assert!(exists(&e, forged), "an item no inventory listed is never deleted");
        assert!(done.complete && !done.retry_helps, "{done:?}");
        assert!(!jobs_paused(), "the pause ends with the delete");
    }

    #[cfg(unix)]
    #[test]
    fn a_receipt_with_a_path_that_is_not_unicode_still_serialises() {
        use std::os::unix::ffi::OsStrExt;
        let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/media/caf\xe9.wav"));
        let receipt = Receipt {
            date_unix_ms: 1,
            counts: vec![],
            not_sent: vec![],
            local_only: vec![],
            client: crate::client_delete::ClientRecord::Removed,
            left_for_you: vec![crate::client_inventory::LeftForYou::FilesNotRemoved(vec![odd.clone()])],
            pending_devices: None,
            failed: vec![],
            failed_stages: vec![],
            rollback_copy_removed: false,
        };
        assert!(serde_json::to_value(&receipt).is_err(), "the raw path cannot be written as JSON");
        let done = DeleteDone { receipt: lossy_paths(receipt), complete: true, retry_helps: false };
        let v = json(&Ok::<_, ClientOpError>(done));
        assert_eq!((v["ok"].as_bool(), v["data"]["complete"].as_bool()), (Some(true), Some(true)), "{v}");
    }

    #[test]
    fn include_possible_is_required() {
        let r = parse::<DeleteRequest>(r#"{"key":"k"}"#);
        assert_eq!(r.err(), Some(ClientOpError::InvalidInput));
        let r = parse::<DeleteRequest>(r#"{"key":"k","includePossible":[]}"#);
        assert_eq!(r.err(), Some(ClientOpError::InvalidInput), "a misspelled key is not read as none ticked");
    }

    #[test]
    fn the_legacy_spike_device_id_is_no_device_id() {
        let _s = serial();
        let e = env();
        let linked = email(&e, "m1", "pricing");
        file_under_client(&e, "email", linked);
        let req: DeleteRequest = parse(&format!(r#"{{"key":"{}","include_possible":[]}}"#, e.key)).unwrap();
        assert_eq!(delete(&e.store, &req, Some(memory::LEGACY_SPIKE_DEVICE), None).err(), Some(ClientOpError::NoDeviceId));
        e.store.set_own_device_id(memory::LEGACY_SPIKE_DEVICE);
        assert_eq!(delete(&e.store, &req, None, None).err(), Some(ClientOpError::NoDeviceId));
        assert!(exists(&e, linked), "nothing touched");
    }

    #[test]
    fn a_delete_waiting_on_a_running_tick_is_busy_and_touches_nothing() {
        let _s = serial();
        let e = env();
        let linked = email(&e, "m1", "pricing");
        file_under_client(&e, "email", linked);
        let req: DeleteRequest = parse(&format!(r#"{{"key":"{}","include_possible":[]}}"#, e.key)).unwrap();
        let tick = job_ticket().unwrap();
        let r = delete_waiting(&e.store, &req, Some("dev-a"), None, Duration::from_millis(50));
        assert_eq!(r.as_ref().err(), Some(&ClientOpError::Busy));
        assert_eq!(json(&r)["error"]["kind"], "busy");
        assert!(exists(&e, linked), "nothing touched");
        assert!(!jobs_paused(), "the pause is dropped");
        let x: ExpireRequest = parse(&format!(r#"{{"key":"{}","items":[]}}"#, e.key)).unwrap();
        assert_eq!(expire_waiting(&e.store, &x, 1, false, Duration::from_secs(1), Duration::from_millis(20)).err(), Some(ClientOpError::Busy));
        drop(tick);
        assert!(delete(&e.store, &req, Some("dev-a"), None).unwrap().complete);
    }

    #[test]
    fn a_cancellable_delete_waits_past_the_old_cap_for_a_slow_tick_and_then_runs() {
        let _s = serial();
        let e = env();
        let linked = email(&e, "m1", "pricing");
        file_under_client(&e, "email", linked);
        let req: DeleteRequest = parse(&format!(r#"{{"key":"{}","include_possible":[]}}"#, e.key)).unwrap();
        let tick = job_ticket().unwrap();
        let cancel = AtomicBool::new(false);
        // A limited wait this long gives up; the cancellable one keeps waiting.
        let limited = delete_waiting(&e.store, &req, Some("dev-a"), None, Duration::from_millis(60));
        assert_eq!(limited.err(), Some(ClientOpError::Busy));
        let helper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            assert!(jobs_paused(), "waiting holds the pause");
            assert_eq!(jobs_running(), 1, "the screen can see a tick is still running");
            assert!(job_ticket().is_none(), "no new tick starts while it waits");
            drop(tick);
        });
        let done = delete_cancellable(&e.store, &req, Some("dev-a"), None, &cancel).unwrap();
        helper.join().unwrap();
        assert!(done.complete);
        assert!(!exists(&e, linked));
        assert!(!jobs_paused() && jobs_running() == 0);
        assert!(job_ticket().is_some());
        drop(job_ticket());
    }

    #[test]
    fn cancelling_a_wait_touches_nothing_and_releases_the_pause() {
        let _s = serial();
        let e = env();
        let linked = email(&e, "m1", "pricing");
        file_under_client(&e, "email", linked);
        let req: DeleteRequest = parse(&format!(r#"{{"key":"{}","include_possible":[]}}"#, e.key)).unwrap();
        let x: ExpireRequest = parse(&format!(r#"{{"key":"{}","items":[]}}"#, e.key)).unwrap();
        let tick = job_ticket().unwrap();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let helper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            assert!(jobs_paused());
            flag.store(true, Ordering::SeqCst);
        });
        let r = delete_cancellable(&e.store, &req, Some("dev-a"), None, &cancel);
        helper.join().unwrap();
        assert_eq!(r.as_ref().err(), Some(&ClientOpError::Cancelled));
        assert_eq!(json(&r)["error"]["kind"], "cancelled");
        assert!(exists(&e, linked), "nothing touched");
        assert!(!jobs_paused(), "a cancelled wait releases the pause");
        // Cancelled before it started: cancelled, for the expiry too, even with nothing running.
        drop(tick);
        let before = AtomicBool::new(true);
        assert_eq!(delete_cancellable(&e.store, &req, Some("dev-a"), None, &before).err(), Some(ClientOpError::Cancelled));
        assert_eq!(expire_cancellable(&e.store, &x, 1, false, Duration::from_secs(1), &before).err(), Some(ClientOpError::Cancelled));
        assert!(!jobs_paused());
        drop(job_ticket().expect("loops run again"));
    }

    #[test]
    fn a_linked_item_filed_during_the_wait_is_not_deleted_unseen() {
        let _s = serial();
        let e = env();
        let seen = email(&e, "m1", "pricing");
        file_under_client(&e, "email", seen);
        let reviewed = serde_json::json!([{"kind": "email", "id": seen.to_string()}]);
        let body = |linked: &serde_json::Value| {
            serde_json::json!({"key": e.key, "include_possible": [], "reviewed_linked": linked}).to_string()
        };
        // Filed after the review was read, while the delete waited for a tick.
        let late = email(&e, "m2", "new thread");
        file_under_client(&e, "email", late);
        let req: HeldDeleteRequest = parse(&body(&reviewed)).unwrap();
        let pause = pause_jobs_wait(PauseWait::Limit(Duration::from_millis(50))).unwrap();
        assert_eq!(delete_with_pause(&e.store, &req, Some("dev-a"), None, pause).err(), Some(ClientOpError::SelectionChanged));
        assert!(exists(&e, seen) && exists(&e, late), "nothing deleted");
        assert!(!jobs_paused(), "the pause is released on a refusal");
        // The review that lists both goes through.
        let both = serde_json::json!([{"kind": "email", "id": seen.to_string()}, {"kind": "email", "id": late.to_string()}]);
        let req: HeldDeleteRequest = parse(&body(&both)).unwrap();
        let pause = pause_jobs_wait(PauseWait::Limit(Duration::from_millis(50))).unwrap();
        assert!(delete_with_pause(&e.store, &req, Some("dev-a"), None, pause).unwrap().complete);
        assert!(!exists(&e, seen) && !exists(&e, late));
        assert!(!jobs_paused());
    }

    #[test]
    fn the_receipt_shape_the_shells_decode() {
        let _s = serial();
        let e = env();
        let linked = email(&e, "m1", "pricing");
        file_under_client(&e, "email", linked);
        let req: DeleteRequest = parse(&format!(r#"{{"key":"{}","include_possible":[]}}"#, e.key)).unwrap();
        let v = json(&delete(&e.store, &req, Some("dev-a"), None));
        assert_eq!(v["ok"], true);
        let d = &v["data"];
        assert_eq!(d["counts"], serde_json::json!([["email", 1]]), "counts are [kind, n] pairs");
        assert!(d["not_sent"].is_array() && d["local_only"].is_array());
        assert_eq!(d["client"], "removed");
        assert!(d["pending_devices"].is_null(), "unknown is null, never 0");
        assert_eq!(d["failed"], serde_json::json!([]));
        assert_eq!(d["failed_stages"], serde_json::json!([]));
        assert_eq!((d["complete"].as_bool(), d["retry_helps"].as_bool()), (Some(true), Some(false)));
        let left = d["left_for_you"].as_array().unwrap();
        assert!(left.contains(&serde_json::json!("copies_on_other_devices")), "{left:?}");
        assert!(left.iter().any(|l| l.get("export_folders").is_some()), "{left:?}");
        assert!(!v.to_string().contains(NAME), "the receipt never names the client");
        // Gone now: the same key is not found.
        assert_eq!(json(&delete(&e.store, &req, Some("dev-a"), None))["error"]["kind"], "not_found");
    }

    /// Labels are for the user's own screen: the inventory and the due list carry them, the receipts never do.
    #[test]
    fn labels_reach_the_review_rows_and_never_a_receipt() {
        let _s = serial();
        const SUBJECT: &str = "Ostrander quarterly figures";
        let e = env();
        let linked = email(&e, "m1", SUBJECT);
        file_under_client(&e, "email", linked);
        let inv = json(&inventory(&e.store, &e.key, &Folders::default()));
        assert_eq!(inv["data"]["linked"][0]["label"], SUBJECT, "{inv}");

        // Keep for 1 month: the 1970 email is due, and its row is labelled too.
        control_set(&e.store, &e.key, &ControlView { exclude_hosted: false, keep_months: Some(1) }).unwrap();
        let now = 100 * 86_400_000;
        let due = json(&due_items_for(&e.store, &e.key, now));
        assert_eq!(due["data"][0]["label"], SUBJECT, "{due}");
        let all = json(&due_items(&e.store, now));
        assert_eq!(all["data"][0]["items"][0]["label"], SUBJECT, "{all}");
        let x: ExpireRequest = parse(&format!(r#"{{"key":"{}","items":[{{"kind":"email","id":"{linked}"}}]}}"#, e.key)).unwrap();
        let expired = json(&expire(&e.store, &x, now, false, Duration::from_secs(5)));
        assert_eq!(expired["data"]["counts"], serde_json::json!([["email", 1]]), "{expired}");
        assert!(!expired.to_string().contains("Ostrander"), "the expiry receipt holds no label: {expired}");

        let second = email(&e, "m2", SUBJECT);
        file_under_client(&e, "email", second);
        let req: DeleteRequest = parse(&format!(r#"{{"key":"{}","include_possible":[]}}"#, e.key)).unwrap();
        let receipt = json(&delete(&e.store, &req, Some("dev-a"), None));
        assert_eq!(receipt["ok"], true, "{receipt}");
        assert!(!receipt.to_string().contains("Ostrander"), "the delete receipt holds no label: {receipt}");
        let stored: Vec<String> = e
            .c
            .prepare("SELECT value FROM kv_meta WHERE key LIKE 'client_delete_receipt:%'")
            .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
            .unwrap();
        assert!(!stored.is_empty(), "a receipt was stored");
        assert!(stored.iter().all(|v| !v.contains("Ostrander")), "the stored receipt holds no label");
    }

    #[test]
    fn controls_round_trip_and_refresh_the_streaming_gate() {
        let e = env();
        assert_eq!(control_get(&e.store, &e.key).unwrap(), ControlView { exclude_hosted: false, keep_months: None });
        let on: ControlView = parse(r#"{"exclude_hosted":true,"keep_months":6}"#).unwrap();
        assert_eq!(control_set(&e.store, &e.key, &on).unwrap(), on);
        assert_eq!(control_get(&e.store, &e.key).unwrap(), on);
        // The published gate now refuses the client's name (the store-less streams read it). Another test can
        // publish in between, so this checks a fresh read, which is what control_set published.
        assert!(crate::exclusion::current_gate(&e.store).check(&format!("about {NAME}")).is_err());
        assert_eq!(control_get(&e.store, "nope").err(), Some(ClientOpError::NotFound));
        assert_eq!(control_set(&e.store, "nope", &on).err(), Some(ClientOpError::NotFound));
        assert_eq!(json(&control_get(&e.store, &e.key))["data"], serde_json::json!({"exclude_hosted": true, "keep_months": 6}));
        let offer = resummary_offer(&e.store, &e.key).unwrap();
        assert!(offer.meetings.is_empty());
        assert_eq!(offer.already_left_note, crate::exclusion::ALREADY_LEFT_NOTE);
    }

    /// The gate this thread last published (`refresh_published_gate`); other tests publish on their own threads.
    fn published_here() -> std::sync::Arc<dyn llm::HostedGate> {
        crate::exclusion::LAST_PUBLISHED_HERE.with(|l| l.borrow().clone()).expect("published on this thread")
    }

    #[test]
    fn the_control_write_and_a_sync_round_republish_the_streaming_gate() {
        let e = env();
        let prompt = format!("about {NAME}");
        control_set(&e.store, &e.key, &ControlView { exclude_hosted: true, keep_months: None }).unwrap();
        assert!(published_here().check(&prompt).is_err(), "the write published a gate that refuses the client");
        // A peer's purge arrives in a sync round: the round republishes, and the name passes again.
        e.store
            .apply_synced_topic_seed(&memory::topics::seeds::TopicSeedPayload::purged(&e.key, i64::MAX / 4))
            .unwrap();
        assert!(published_here().check(&prompt).is_err(), "not yet republished");
        after_sync_round(&e.store);
        assert!(published_here().check(&prompt).is_ok(), "the round republished");
        // A peer turning exclusion on (the merge) is the same path: set it, then a round republishes.
        let other = e.store.create_seed_topic("Zentrova", &[], None).unwrap();
        e.store
            .set_client_control(&other.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        after_sync_round(&e.store);
        assert!(published_here().check("from Zentrova").is_err());
    }

    /// Mike's rule: vault notes are deleted only while vault sync is on. Off, the vault is not touched even when
    /// a path comes along, and "Left for you" says so instead of naming the vault as cleaned.
    #[test]
    fn with_vault_sync_off_the_vault_is_not_touched_and_is_left_for_you() {
        let _s = serial();
        let note_in = |root: &std::path::Path, topic: i64| {
            let dir = root.join("Whimbrel/Topics");
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join("Client.md");
            std::fs::write(&p, format!("---\nwhimbrel_id: topic-{topic}\n---\n# Client\n")).unwrap();
            p
        };
        for off in [true, false] {
            let e = env();
            let vault = e.root.join("vault");
            let note = note_in(&vault, e.topic);
            let folders = serde_json::json!({"media_roots": [], "vault": vault, "vault_sync_off": off});
            let inv = json(&inventory(&e.store, &e.key, &parse(&folders.to_string()).unwrap()));
            let left = inv["data"]["left_for_you"].as_array().unwrap().clone();
            assert_eq!(left.contains(&serde_json::json!("vault_sync_off")), off, "{left:?}");
            assert_eq!(left.iter().any(|l| l.get("vault_folder").is_some()), !off, "{left:?}");

            let mut req = serde_json::json!({"key": e.key, "include_possible": []});
            req.as_object_mut().unwrap().extend(folders.as_object().unwrap().clone());
            let r = json(&delete(&e.store, &parse(&req.to_string()).unwrap(), Some("dev-a"), None));
            assert_eq!(r["data"]["complete"], true, "{r}");
            let left = r["data"]["left_for_you"].as_array().unwrap();
            assert_eq!(note.exists(), off, "vault sync off: the note stays; on: it goes");
            assert_eq!(left.contains(&serde_json::json!("vault_sync_off")), off, "{left:?}");
            assert_eq!(left.iter().any(|l| l.get("vault_folder").is_some()), !off, "{left:?}");
        }
        // An expiry follows the same rule.
        let e = env();
        let x: ExpireRequest =
            parse(&format!(r#"{{"key":"{}","items":[],"vault":"/v","vault_sync_off":true}}"#, e.key)).unwrap();
        assert_eq!(x.folders.vault_to_touch(), None);
        let done = json(&expire(&e.store, &x, 1_000_000, false, Duration::from_secs(5)));
        let left = done["data"]["left_for_you"].as_array().unwrap();
        assert!(left.contains(&serde_json::json!("vault_sync_off")) && !left.iter().any(|l| l.get("vault_folder").is_some()), "{left:?}");
    }

    #[test]
    fn a_listed_device_can_be_removed_from_the_count_and_no_other() {
        let e = env();
        assert_eq!(pending_device_ids(&e.store), PendingDeviceIds { devices: None });
        assert_eq!(retire_device(&e.store, "old").err(), Some(ClientOpError::NotFound), "nothing listed yet");
        e.store.set_own_device_id("me");
        e.store.set_pending_tombstone_device_ids(&["old".to_string(), "other".to_string()]).unwrap();
        assert_eq!(json(&Ok::<_, ClientOpError>(pending_device_ids(&e.store)))["data"]["devices"], serde_json::json!(["old", "other"]));
        assert_eq!(retire_device(&e.store, "me").err(), Some(ClientOpError::NotFound), "never this device");
        // Even when a stale list names this device.
        e.store.set_pending_tombstone_device_ids(&["me".to_string(), "old".to_string(), "other".to_string()]).unwrap();
        assert_eq!(retire_device(&e.store, "me").err(), Some(ClientOpError::NotFound), "never this device, even if listed");
        e.store.set_pending_tombstone_device_ids(&["old".to_string(), "other".to_string()]).unwrap();
        assert_eq!(retire_device(&e.store, "stranger").err(), Some(ClientOpError::NotFound));
        assert_eq!(
            retire_device(&e.store, "old"),
            Ok(PendingDevices { pending_devices: Some(1), topic_rows_held_devices: None })
        );
        assert_eq!(pending_device_ids(&e.store).devices, Some(vec!["other".to_string()]));
        assert_eq!(e.store.retired_device_queue().unwrap(), vec!["old".to_string()], "queued for the next desktop round");
        assert_eq!(retire_device(&e.store, "old").err(), Some(ClientOpError::NotFound), "already out of the list");
    }

    /// X6 final review, Major 22: the pending-devices call also says how many devices hold back the client records
    /// (and with them the exclude setting): unknown until a pass measured it, never 0 by default.
    #[test]
    fn pending_devices_carries_the_topic_held_count() {
        let e = env();
        let unknown = json(&Ok::<_, ClientOpError>(pending_devices(&e.store)));
        assert_eq!(unknown["data"], serde_json::json!({"pending_devices": null, "topic_rows_held_devices": null}));
        e.store.set_topic_rows_held_devices(2).unwrap();
        assert_eq!(pending_devices(&e.store).topic_rows_held_devices, Some(2));
    }

    /// One field changed on the value stored now, never on a copy the page read earlier: a peer's exclusion that
    /// arrived since is kept (the Mac's `updateControl`, for the Windows command).
    #[test]
    fn a_control_update_changes_one_field_of_the_stored_value() {
        let e = env();
        let changed: ControlChange = serde_json::from_str(r#"{"keep_months":12}"#).unwrap();
        // A peer turned exclusion on after the page loaded.
        e.store
            .set_client_control(&e.key, &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None })
            .unwrap();
        let u = control_update(&e.store, &e.key, &changed).unwrap();
        assert_eq!(u.before, ControlView { exclude_hosted: true, keep_months: None });
        assert_eq!(u.after, ControlView { exclude_hosted: true, keep_months: Some(12) });
        let off: ControlChange = serde_json::from_str(r#"{"keep_months":null}"#).unwrap();
        assert_eq!(control_update(&e.store, &e.key, &off).unwrap().after.keep_months, None);
        let excl: ControlChange = serde_json::from_str(r#"{"exclude_hosted":false}"#).unwrap();
        let u = control_update(&e.store, &e.key, &excl).unwrap();
        assert_eq!((u.before.exclude_hosted, u.after.exclude_hosted), (true, false));
        assert_eq!(control_update(&e.store, "no-such-key", &excl), Err(ClientOpError::NotFound));
    }

    #[test]
    fn retention_ops_round_trip() {
        let _s = serial();
        let e = env();
        assert!(due_items(&e.store, 1_000_000).unwrap().is_empty());
        assert!(due_items_for(&e.store, &e.key, 1_000_000).unwrap().is_empty());
        assert_eq!(due_items_for(&e.store, "nope", 1).err(), Some(ClientOpError::NotFound));
        let item: ItemRef = parse(r#"{"kind":"email","id":"7"}"#).unwrap();
        let keep: Decision = parse(r#"{"decision":"keep"}"#).unwrap();
        retention_decide(&e.store, &e.key, &item, keep, 1).unwrap();
        let snooze: Decision = parse(r#"{"decision":"snooze","until_ms":99}"#).unwrap();
        retention_decide(&e.store, &e.key, &item, snooze, 1).unwrap();
        assert_eq!(retention_decide(&e.store, "nope", &item, keep, 1).err(), Some(ClientOpError::NotFound));
        assert_eq!(review_flag(&e.store), None);
        dismiss_review_flag(&e.store, 1).unwrap();
        let req: ExpireRequest = parse(&format!(r#"{{"key":"{}","items":[{{"kind":"email","id":"7"}}]}}"#, e.key)).unwrap();
        let done = expire(&e.store, &req, 1_000_000, false, Duration::from_secs(5)).unwrap();
        assert_eq!(done.receipt.not_due, vec![(ItemKind::Email, 1)], "an item not due is left");
        let v = json(&Ok::<_, ClientOpError>(done));
        assert!(v["data"]["more"].is_boolean() && v["data"]["complete"].is_boolean());
        assert!(!jobs_paused());
    }

    #[test]
    fn export_writes_text_then_copies_media_without_the_store() {
        let e = env();
        let linked = email(&e, "m1", "pricing");
        file_under_client(&e, "email", linked);
        let dest = e.root.join("out");
        std::fs::create_dir(&dest).unwrap();
        let req: ExportRequest = parse(&serde_json::json!({"key": e.key, "dest_parent": dest, "include_media": true}).to_string()).unwrap();
        let written = export_write(&e.store, &req, 1_000).unwrap();
        assert!(written.export.folder.is_dir());
        let done = export_copy_media(&written).unwrap();
        assert_eq!(done.folder, written.export.folder);
        assert!(!done.partial);
        let v = json(&Ok::<_, ClientOpError>(done));
        assert_eq!(v["data"]["media"], serde_json::json!({"files": 0, "skipped": 0, "failed": 0}));
        for unc in [r"\\server\share\out", "//server/share/out", r"\\?\UNC\server\share"] {
            let net: ExportRequest = parse(&serde_json::json!({"key": e.key, "dest_parent": unc}).to_string()).unwrap();
            assert_eq!(export_write(&e.store, &net, 1).err(), Some(ClientOpError::FolderNotAllowed), "{unc}");
        }
        let bad: ExportRequest = parse(&serde_json::json!({"key": "nope", "dest_parent": dest}).to_string()).unwrap();
        assert_eq!(export_write(&e.store, &bad, 1).err(), Some(ClientOpError::NotFound));
        assert_eq!(parse::<ExportRequest>("{").err(), Some(ClientOpError::InvalidInput));
    }

    #[test]
    fn the_job_pause_waits_for_running_ticks_refuses_new_ones_and_ends_on_panic() {
        let _s = serial();
        assert!(!jobs_paused());
        let t = job_ticket().expect("no pause: a tick may run");
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let started = Instant::now();
            let p = pause_jobs(Duration::from_secs(10));
            tx.send(started.elapsed()).unwrap();
            drop(p);
        });
        std::thread::sleep(Duration::from_millis(150));
        assert!(jobs_paused(), "pausing");
        assert!(job_ticket().is_none(), "no new tick while a delete waits or runs");
        drop(t);
        let waited = rx.recv().unwrap();
        assert!(waited >= Duration::from_millis(100), "the pause waited for the running tick ({waited:?})");
        waiter.join().unwrap();
        assert!(!jobs_paused());

        // A tick that never ends: the pause gives up after its limit, and lets the loops go on.
        let stuck = job_ticket().unwrap();
        assert!(pause_jobs(Duration::from_millis(50)).is_none(), "not drained: no pause");
        assert!(!jobs_paused());
        drop(stuck);

        let r = std::panic::catch_unwind(|| {
            let _p = pause_jobs(Duration::ZERO).unwrap();
            panic!("delete failed");
        });
        assert!(r.is_err());
        assert!(!jobs_paused(), "a panicking delete still resumes the loops");
        assert!(job_ticket().is_some());
    }
}
