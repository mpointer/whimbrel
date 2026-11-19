//! Client controls (X6 Task 9): inventory, delete, export, settings, and the keep-for-N-months review. Thin
//! glue over `api::client_controls`, which holds the logic and its tests; the Mac calls the same functions
//! through core/ffi.
//!
//! Each command resolves to core's envelope as JSON, exactly what the Mac decodes: `{"ok":true,"data":..}` or
//! `{"ok":false,"error":{"kind":"not_found"|"no_device_id"|"invalid_input"|"selection_changed"|"busy"|
//! "cancelled"|"folder_not_allowed"|"failed"}}` (never a client's
//! name). A `CommandError` means the shell itself could not run the call (the database is not open, the
//! account key could not be read). The UI's types for these shapes are in `ui/src/clientControls.ts`.
//!
//! The media folders, the vault folder, this PC's device id and the sync key come from the shell, never from
//! the webview. A delete or expiry runs on a blocking thread on its own connection, so neither the UI nor the
//! store lanes wait for it; the job loops pause while it runs (`api::client_controls::JobPause`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use api::client_controls as cc;
use serde::Deserialize;
use serde_json::Value;
use tauri::State;

use crate::error::CommandError;
use crate::state::AppState;
use crate::syncing::keys;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// core's envelope text as JSON for the UI.
fn envelope<T: serde::Serialize>(r: &cc::OpResult<T>) -> Result<Value, CommandError> {
    serde_json::from_str(&cc::envelope(r)).map_err(|e| CommandError::store(e.to_string()))
}

/// The folders a client's meeting files may be deleted from or copied from: the recordings folder first (named
/// under "left for you"), then every folder slide snapshots go to (the one chosen in Settings, the default
/// under Pictures, the fallback under the data folder; core removes a slide folder only when it carries
/// Whimbrel's marker). And the Obsidian vault folder, only while "Keep an Obsidian vault up to date" is on (Mike:
/// vault notes are deleted only while vault sync is on); off with a folder chosen before, no vault is sent and
/// `vault_sync_off` makes "left for you" say the vault was not touched.
fn folders(state: &AppState) -> cc::Folders {
    use crate::meetings::snapshots::storage;
    let (vault, vault_enabled, chosen) = {
        let file = state.settings.lock().unwrap();
        (file.vault_path(), file.vault_enabled(), file.meeting_snapshots_root())
    };
    let mut media_roots = vec![state.paths.recordings.clone()];
    for root in [chosen, storage::default_root(), Some(state.paths.root.join(storage::ROOT_NAME))].into_iter().flatten() {
        if !media_roots.contains(&root) {
            media_roots.push(root);
        }
    }
    let has_vault = !vault.trim().is_empty();
    cc::Folders {
        media_roots,
        vault: (has_vault && vault_enabled).then(|| PathBuf::from(vault)),
        vault_sync_off: has_vault && !vault_enabled,
    }
}

/// The account key when sync is set up on this PC.
fn group_key(state: &AppState) -> Result<Option<sync::GroupKey>, CommandError> {
    Ok(keys::load_group_key(&*state.secrets)?)
}

/// Runs `f` on a blocking thread with its own connection to the database (sharing the main store's embedder),
/// so a slow delete holds neither the UI nor a store lane. The connection opens like a store lane: with the
/// key the main store opened with when the database is encrypted (X4), plain otherwise.
async fn on_own_connection<R, F>(state: &AppState, f: F) -> Result<R, CommandError>
where
    R: Send + 'static,
    F: FnOnce(&memory::Store) -> R + Send + 'static,
{
    let main = state.store()?;
    let embedder = main.call(|s| s.embedder()).await?;
    let key = main.db_key();
    let db = state.paths.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = crate::db_open::open_lane(&db, embedder, key.as_deref()).map_err(CommandError::store)?;
        Ok::<R, CommandError>(f(&store))
    })
    .await
    .map_err(|e| CommandError::store(e.to_string()))?
}

/// data: the client's inventory (`linked`, `possible`, `possible_truncated`, `counts`, `left_for_you`).
#[tauri::command]
pub async fn client_inventory(state: State<'_, AppState>, key: String) -> Result<Value, CommandError> {
    let f = folders(&state);
    let r = state.store()?.call(move |s| cc::inventory(s, &key, &f)).await?;
    envelope(&r)
}

/// What the user confirmed on the review screen. Folders, device id and key are the shell's.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDeleteSelection {
    pub key: String,
    #[serde(default)]
    pub include_possible: Vec<cc::ItemRef>,
    #[serde(default)]
    pub delete_exports: bool,
    /// The linked items the review screen showed: when the linked set differs once the wait is over (an item
    /// filed under the client meanwhile), the answer is `selection_changed` and nothing is touched.
    #[serde(default)]
    pub reviewed_linked: Option<Vec<cc::ItemRef>>,
    /// The page's number for this delete, for `client_cancel_wait` (below 2^53). Absent: it cannot be cancelled.
    #[serde(default)]
    pub token: Option<u64>,
}

/// A delete waiting for background work, by its token: Cancel sets `cancel`; once `started` it is too late.
struct Wait {
    cancel: Arc<AtomicBool>,
    started: bool,
}

static WAITS: Mutex<Vec<(u64, Wait)>> = Mutex::new(Vec::new());

/// Tokens for deletes the page sent without one: above any page token.
static NEXT_SHELL_TOKEN: AtomicU64 = AtomicU64::new(1 << 60);

fn waits() -> MutexGuard<'static, Vec<(u64, Wait)>> {
    WAITS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The token's entry, made when absent (a Cancel can arrive before the delete's command has registered it).
fn wait_entry(v: &mut Vec<(u64, Wait)>, token: u64) -> &mut Wait {
    match v.iter().position(|(t, _)| *t == token) {
        Some(i) => &mut v[i].1,
        None => {
            v.push((token, Wait { cancel: Arc::default(), started: false }));
            &mut v.last_mut().expect("just pushed").1
        }
    }
}

/// Finished tokens kept (as started) so a Cancel that arrives after the delete ended answers "too late" rather than
/// leaving an entry behind.
const FINISHED_KEPT: usize = 64;

/// On every way out of `client_delete`: the token is marked finished, and the oldest finished ones are forgotten.
struct ForgetWait(u64);

impl Drop for ForgetWait {
    fn drop(&mut self) {
        let mut v = waits();
        wait_entry(&mut v, self.0).started = true;
        while v.iter().filter(|(_, w)| w.started).count() > FINISHED_KEPT {
            let Some(i) = v.iter().position(|(_, w)| w.started) else { break };
            v.remove(i);
        }
    }
}

/// Step 2's guard: a Cancel that came before this point wins (`cancelled`, nothing touched); after it, the delete
/// has started and Cancel answers "too late". One lock with `client_cancel_wait`, so the two never cross.
fn start_unless_cancelled(token: u64) -> bool {
    let mut v = waits();
    let w = wait_entry(&mut v, token);
    if w.cancel.load(Ordering::SeqCst) {
        return false;
    }
    w.started = true;
    true
}

/// Deletes a client and its material ("Delete this client"). data: the receipt plus `complete` and `retry_helps`. With no device
/// identity on this PC the call is `no_device_id` and nothing is touched.
///
/// Waits for running background work as long as it takes (PR 875: a tick making slow local-model calls can outlast
/// any fixed limit), on a thread that holds no store; no new tick starts meanwhile. `client_cancel_wait` ends the
/// wait (`cancelled`, nothing touched) until the delete itself starts.
#[tauri::command]
pub async fn client_delete(state: State<'_, AppState>, selection: ClientDeleteSelection) -> Result<Value, CommandError> {
    let held = cc::HeldDeleteRequest {
        request: cc::DeleteRequest {
            key: selection.key,
            include_possible: selection.include_possible,
            delete_exports: selection.delete_exports,
            folders: folders(&state),
        },
        reviewed_linked: selection.reviewed_linked,
    };
    let device = state.identity.as_ref().map(|i| i.device_id.clone());
    let key = group_key(&state)?;
    let token = selection.token.unwrap_or_else(|| NEXT_SHELL_TOKEN.fetch_add(1, Ordering::Relaxed));
    let _forget = ForgetWait(token);
    let cancel = wait_entry(&mut waits(), token).cancel.clone();
    // Step 1: wait for the running ticks, holding no store and no lane.
    let paused = tauri::async_runtime::spawn_blocking(move || cc::pause_jobs_wait(cc::PauseWait::Until(&cancel)))
        .await
        .map_err(|e| CommandError::store(e.to_string()))?;
    let pause = match paused {
        Ok(p) => p,
        Err(e) => return envelope(&Err::<(), _>(e)),
    };
    // Step 2 on its own connection. Should the connection not open, the closure is dropped with the pause in it,
    // which releases it.
    let r = on_own_connection(&state, move |s| {
        if !start_unless_cancelled(token) {
            drop(pause);
            return Err(cc::ClientOpError::Cancelled);
        }
        cc::delete_with_pause(s, &held, device.as_deref(), key.as_ref(), pause)
    })
    .await?;
    envelope(&r)
}

/// Cancels the delete with `token` while it waits for background work: nothing is deleted. `true` = cancelled;
/// `false` = too late, the delete has started and will finish (its receipt follows).
#[tauri::command]
pub fn client_cancel_wait(token: u64) -> bool {
    let mut v = waits();
    let w = wait_entry(&mut v, token);
    if w.started {
        return false;
    }
    w.cancel.store(true, Ordering::SeqCst);
    true
}

/// Background ticks running now: what a waiting delete is waiting for.
#[tauri::command]
pub fn client_jobs_running() -> usize {
    cc::jobs_running()
}

/// `{"pending_devices": n | null, "topic_rows_held_devices": n | null}`: devices that still need to update before
/// they apply client deletes, and devices holding back this PC's client records (with the exclude setting).
#[tauri::command]
pub async fn client_pending_devices(state: State<'_, AppState>) -> Result<Value, CommandError> {
    let p = state.store()?.call(cc::pending_devices).await?;
    serde_json::to_value(p).map_err(|e| CommandError::store(e.to_string()))
}

/// `{"devices": [id] | null}`: the sync device ids (public keys) of the devices `client_pending_devices` counts.
#[tauri::command]
pub async fn client_pending_device_ids(state: State<'_, AppState>) -> Result<Value, CommandError> {
    let p = state.store()?.call(cc::pending_device_ids).await?;
    serde_json::to_value(p).map_err(|e| CommandError::store(e.to_string()))
}

/// "Remove this device": stops counting `device_id` (one of `client_pending_device_ids`) in the devices still to
/// update. data: `{"pending_devices": n | null}`; `not_found` for this PC's own id or one not listed. Releases
/// nothing. Desktop only: the phones have no such command.
#[tauri::command]
pub async fn client_retire_device(state: State<'_, AppState>, device_id: String) -> Result<Value, CommandError> {
    let r = state.store()?.call(move |s| cc::retire_device(s, &device_id)).await?;
    envelope(&r)
}

/// data: `{"exclude_hosted","keep_months"}`.
#[tauri::command]
pub async fn client_control_get(state: State<'_, AppState>, key: String) -> Result<Value, CommandError> {
    let r = state.store()?.call(move |s| cc::control_get(s, &key)).await?;
    envelope(&r)
}

/// Changes one setting (`{"exclude_hosted": bool}` or `{"keep_months": n | null}`) on the value stored now: the
/// read, the change and the write are one job on the store, where sync rounds also run, so a value another device
/// synced since the page loaded is kept. Republishes the hosted-model gate. data: `{"before","after"}`.
#[tauri::command]
pub async fn client_control_update(state: State<'_, AppState>, key: String, change: cc::ControlChange) -> Result<Value, CommandError> {
    let r = state.store()?.call(move |s| cc::control_update(s, &key, &change)).await?;
    envelope(&r)
}

/// data: `{"meetings":[id],"already_left_note"}` for turning exclusion on.
#[tauri::command]
pub async fn client_resummary_offer(state: State<'_, AppState>, key: String) -> Result<Value, CommandError> {
    let r = state.store()?.call(move |s| cc::resummary_offer(s, &key)).await?;
    envelope(&r)
}

/// data: `[{"key","items"}]`, or one client's `[ClientItem]` when `key` is given.
#[tauri::command]
pub async fn client_due_items(state: State<'_, AppState>, key: Option<String>) -> Result<Value, CommandError> {
    let now = now_ms();
    match key {
        Some(key) => envelope(&state.store()?.call(move |s| cc::due_items_for(s, &key, now)).await?),
        None => envelope(&state.store()?.call(move |s| cc::due_items(s, now)).await?),
    }
}

/// Keep or snooze one item, on this PC only. data: null.
#[tauri::command]
pub async fn client_retention_decide(
    state: State<'_, AppState>,
    key: String,
    item: cc::ItemRef,
    decision: api::client_retention::Decision,
) -> Result<Value, CommandError> {
    let now = now_ms();
    let r = state.store()?.call(move |s| cc::retention_decide(s, &key, &item, decision, now)).await?;
    envelope(&r)
}

/// data: the weekly review flag `{"clients","items","raised_at_unix_ms"}`, or null.
#[tauri::command]
pub async fn client_review_flag(state: State<'_, AppState>) -> Result<Value, CommandError> {
    let flag = state.store()?.call(cc::review_flag).await?;
    envelope(&Ok(flag))
}

/// "Later": puts the review prompt off for a week. data: null.
#[tauri::command]
pub async fn client_review_flag_dismiss(state: State<'_, AppState>) -> Result<Value, CommandError> {
    let now = now_ms();
    let r = state.store()?.call(move |s| cc::dismiss_review_flag(s, now)).await?;
    envelope(&r)
}

/// Deletes confirmed due items on this PC only, for at most `budget_ms` (data `more`: call again).
#[tauri::command]
pub async fn client_expire(state: State<'_, AppState>, key: String, items: Vec<cc::ItemRef>, budget_ms: u64) -> Result<Value, CommandError> {
    let req = cc::ExpireRequest { key, items, folders: folders(&state) };
    let sync_on = group_key(&state)?.is_some();
    let now = now_ms();
    let budget = Duration::from_millis(budget_ms);
    // The fresh connection does not know this PC's id: without it, an expiry with sync on is `no_device_id`
    // (it could not tell this PC's unsent rows from another device's).
    let device = state.identity.as_ref().map(|i| i.device_id.clone());
    let r = on_own_connection(&state, move |s| {
        if let Some(d) = device.as_deref() {
            s.set_own_device_id(d);
        }
        cc::expire(s, &req, now, sync_on, budget)
    })
    .await?;
    envelope(&r)
}

/// Exports everything about a client into a new folder inside a folder the user picks here, in the shell (never
/// a path from the page). The text is written with the store; the media (when asked for) is copied after, with
/// the store released. data: `{"folder","media":{"files","skipped","failed"},"partial"}`, or null when the pick
/// was cancelled. A network share is `folder_not_allowed`.
#[tauri::command]
pub async fn client_export(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    key: String,
    include_possible: Vec<cc::ItemRef>,
    include_media: bool,
) -> Result<Value, CommandError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        super::parented_dialog(&app, &crate::l10n::tr("Choose folder…", &[])).blocking_pick_folder()
    })
    .await
    .map_err(|e| CommandError::store(e.to_string()))?;
    let Some(dest_parent) = picked.and_then(|p| p.into_path().ok()) else {
        return envelope(&Ok::<Option<cc::ExportDone>, cc::ClientOpError>(None));
    };
    let req = cc::ExportRequest { key, dest_parent, include_possible, include_media, media_roots: folders(&state).media_roots };
    let now = now_ms();
    let written = state.slow_store()?.call(move |s| cc::export_write(s, &req, now)).await?;
    let done = match written {
        Ok(w) => tauri::async_runtime::spawn_blocking(move || cc::export_copy_media(&w))
            .await
            .map_err(|e| CommandError::store(e.to_string()))?,
        Err(e) => Err(e),
    };
    envelope(&done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_before_the_delete_starts_wins_and_one_after_is_too_late() {
        // Cancelled while waiting: step 2 does not start.
        let t1 = 0x875_0001;
        let f1 = ForgetWait(t1);
        assert!(client_cancel_wait(t1));
        assert!(!start_unless_cancelled(t1));
        drop(f1);
        // Started: Cancel is too late, during the delete and after it ended.
        let t2 = 0x875_0002;
        let f2 = ForgetWait(t2);
        assert!(start_unless_cancelled(t2));
        assert!(!client_cancel_wait(t2));
        drop(f2);
        assert!(!client_cancel_wait(t2), "a cancel after the delete ended is too late");
        // A Cancel that arrives before the command registered its token still counts.
        let t3 = 0x875_0003;
        assert!(client_cancel_wait(t3));
        let f3 = ForgetWait(t3);
        assert!(!start_unless_cancelled(t3));
        drop(f3);
    }

    #[test]
    fn finished_tokens_are_bounded() {
        for t in 0..(FINISHED_KEPT as u64 + 20) {
            drop(ForgetWait(0x876_0000 + t));
        }
        assert!(waits().iter().filter(|(_, w)| w.started).count() <= FINISHED_KEPT);
    }
}
