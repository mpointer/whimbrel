//! Client controls C-ABI (X6 Task 9): inventory, delete, export, settings, keep-for-N-months review and
//! expiry. Thin glue over `api::client_controls`, which holds the logic and its tests.
//!
//! Every `*_json` function returns an envelope, `{"ok":true,"data":..}` or `{"ok":false,"error":{"kind":..}}`
//! with kind `not_found`, `no_device_id`, `invalid_input`, `selection_changed`, `busy`, `folder_not_allowed` or
//! `failed` (never a client's name), freed with
//! `whimbrel_core_free_string`. NULL only on a NULL handle or an argument that is not a C string.
//!
//! Slow calls (delete, expire, export write) must run off any queue the UI waits on: the Mac runs them on the
//! slow lane. The export's media copy needs no handle, so it runs with the store released.

use std::collections::HashMap;
use std::ffi::c_char;
use std::ptr;
use std::sync::Mutex;
use std::time::Duration;

use api::client_controls as cc;
use base64::Engine as _;
use memory::Store;
use serde::Serialize;

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

fn store<'a>(handle: *mut StoreHandle) -> Option<&'a Store> {
    (!handle.is_null()).then(|| unsafe { &(*handle).0 })
}

fn out<T: Serialize>(r: &cc::OpResult<T>) -> *mut c_char {
    json_to_c_string(cc::envelope(r))
}

fn now() -> i64 {
    api::util::now_unix_ms()
}

/// `folders_json` may be NULL (no folders known).
fn folders(json: *const c_char) -> cc::OpResult<cc::Folders> {
    match cstr_to_string(json) {
        None => Ok(cc::Folders::default()),
        Some(j) => cc::parse(&j),
    }
}

/// NULL: sync is not set up. Anything else must be a 32-byte base64 key, or the call is refused (never
/// downgraded to "no sync").
fn group_key(b64: *const c_char) -> cc::OpResult<Option<sync::GroupKey>> {
    if b64.is_null() {
        return Ok(None);
    }
    let text = cstr_to_string(b64).ok_or(cc::ClientOpError::InvalidInput)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|_| cc::ClientOpError::InvalidInput)?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| cc::ClientOpError::InvalidInput)?;
    Ok(Some(sync::GroupKey::from_bytes(bytes)))
}

/// data: the client's inventory `{"linked":[ClientItem],"possible":[ClientItem],"possible_truncated","counts":
/// [[kind,n]],"left_for_you":[..]}`. `folders_json` (may be NULL): `{"media_roots":[..],"vault":..,"vault_sync_off":bool}`. A ClientItem's
/// `label` is display text for the user's own screen only (see the header).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_inventory_json(
    handle: *mut StoreHandle,
    key: *const c_char,
    folders_json: *const c_char,
) -> *mut c_char {
    let (Some(s), Some(key)) = (store(handle), cstr_to_string(key)) else {
        return ptr::null_mut();
    };
    out(&folders(folders_json).and_then(|f| cc::inventory(s, &key, &f)))
}

/// Deletes a client and its material ("Delete this client"). `request_json`: `{"key","include_possible":[{"kind","id"}] (required),
/// "delete_exports","media_roots":[..],"vault"}`. `device_id`: this device's sync id, set on the store first
/// (NULL keeps the one set before; with none, or the legacy "spike-device", the call is `no_device_id` and
/// nothing is touched). A ticked item a fresh inventory does not list is `selection_changed`, and running
/// background work that does not finish in time is `busy`; both touch nothing.
/// `group_key_base64`: NULL when sync is not set up. data: the receipt plus `complete` and `retry_helps`.
/// Job loops are paused while it runs (`whimbrel_client_job_begin` answers false).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_delete_json(
    handle: *mut StoreHandle,
    request_json: *const c_char,
    device_id: *const c_char,
    group_key_base64: *const c_char,
) -> *mut c_char {
    let (Some(s), Some(req)) = (store(handle), cstr_to_string(request_json)) else {
        return ptr::null_mut();
    };
    let device = cstr_to_string(device_id);
    let r = cc::parse::<cc::DeleteRequest>(&req).and_then(|req| {
        let key = group_key(group_key_base64)?;
        let r = cc::delete(s, &req, device.as_deref(), key.as_ref());
        if r.is_ok() {
            forget_tokens(&req.key);
        }
        r
    });
    out(&r)
}

/// `{"pending_devices": n | null, "topic_rows_held_devices": n | null}`: devices that still need to update before
/// they apply client deletes, and devices holding back this device's client records (the exclude setting rides in
/// them: warn on the exclude toggle while > 0). null when not known yet (never show it as 0). NULL on a NULL
/// handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_pending_devices_json(handle: *mut StoreHandle) -> *mut c_char {
    let Some(s) = store(handle) else { return ptr::null_mut() };
    serde_json::to_string(&cc::pending_devices(s)).map_or(ptr::null_mut(), json_to_c_string)
}

/// `{"devices": [id] | null}` (not an envelope): the sync device ids (public keys) of the devices
/// `whimbrel_client_pending_devices_json` counts, for naming them in Settings. NULL on a NULL handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_pending_device_ids_json(handle: *mut StoreHandle) -> *mut c_char {
    let Some(s) = store(handle) else { return ptr::null_mut() };
    serde_json::to_string(&cc::pending_device_ids(s)).map_or(ptr::null_mut(), json_to_c_string)
}

/// "Remove this device" (desktop Settings only: the Mac here, Windows through the same `api` call in its Tauri
/// command; never call it from a phone): stops counting `device_id`, one of
/// `whimbrel_client_pending_device_ids_json`'s, in the devices still to update. data: `{"pending_devices": n|null}`
/// after the removal. "not_found" for this device's own id or one not listed. Releases nothing.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_retire_device_json(handle: *mut StoreHandle, device_id: *const c_char) -> *mut c_char {
    let (Some(s), Some(id)) = (store(handle), cstr_to_string(device_id)) else {
        return ptr::null_mut();
    };
    out(&cc::retire_device(s, &id))
}

/// data: `{"exclude_hosted","keep_months"}` (keep_months null: no period).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_control_get_json(handle: *mut StoreHandle, key: *const c_char) -> *mut c_char {
    let (Some(s), Some(key)) = (store(handle), cstr_to_string(key)) else {
        return ptr::null_mut();
    };
    out(&cc::control_get(s, &key))
}

/// Saves `{"exclude_hosted","keep_months"}` for a client (it syncs) and republishes the streaming gate.
/// data: the saved settings.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_control_set_json(
    handle: *mut StoreHandle,
    key: *const c_char,
    control_json: *const c_char,
) -> *mut c_char {
    let (Some(s), Some(key), Some(json)) = (store(handle), cstr_to_string(key), cstr_to_string(control_json)) else {
        return ptr::null_mut();
    };
    out(&cc::parse::<cc::ControlView>(&json).and_then(|c| cc::control_set(s, &key, &c)))
}

/// data: `{"meetings":[id],"already_left_note":".."}`: the client's meetings a hosted model summarised (offer
/// to summarise them again; `whimbrel_llm_summarize_meeting` keeps an excluded client's meeting on this
/// computer), and the sentence to show when exclusion is turned on.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_resummary_offer_json(handle: *mut StoreHandle, key: *const c_char) -> *mut c_char {
    let (Some(s), Some(key)) = (store(handle), cstr_to_string(key)) else {
        return ptr::null_mut();
    };
    out(&cc::resummary_offer(s, &key))
}

/// data: `[{"key","items":[ClientItem]}]`, every client's items past its keep period. Read only.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_due_items_json(handle: *mut StoreHandle) -> *mut c_char {
    let Some(s) = store(handle) else { return ptr::null_mut() };
    out(&cc::due_items(s, now()))
}

/// data: `[ClientItem]`, one client's due items. Read only.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_due_items_for_json(handle: *mut StoreHandle, key: *const c_char) -> *mut c_char {
    let (Some(s), Some(key)) = (store(handle), cstr_to_string(key)) else {
        return ptr::null_mut();
    };
    out(&cc::due_items_for(s, &key, now()))
}

/// Keep (`{"decision":"keep"}`) or snooze (`{"decision":"snooze","until_ms":..}`) one item `{"kind","id"}`,
/// on this device only. data: null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_retention_decide_json(
    handle: *mut StoreHandle,
    key: *const c_char,
    item_json: *const c_char,
    decision_json: *const c_char,
) -> *mut c_char {
    let (Some(s), Some(key), Some(item), Some(decision)) =
        (store(handle), cstr_to_string(key), cstr_to_string(item_json), cstr_to_string(decision_json))
    else {
        return ptr::null_mut();
    };
    let r = cc::parse::<cc::ItemRef>(&item)
        .and_then(|item| Ok((item, cc::parse::<api::client_retention::Decision>(&decision)?)))
        .and_then(|(item, d)| cc::retention_decide(s, &key, &item, d, now()));
    out(&r)
}

/// data: `{"clients","items","raised_at_unix_ms"}` when the weekly "items ready to review" flag is up, else null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_review_flag_json(handle: *mut StoreHandle) -> *mut c_char {
    let Some(s) = store(handle) else { return ptr::null_mut() };
    out(&Ok(cc::review_flag(s)))
}

/// "Later": puts the review prompt off for a week. data: null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_review_flag_dismiss_json(handle: *mut StoreHandle) -> *mut c_char {
    let Some(s) = store(handle) else { return ptr::null_mut() };
    out(&cc::dismiss_review_flag(s, now()))
}

/// One delete that waits for background work (or has the pause and is queued for the slow lane), by the token
/// the shell chose.
struct WaitEntry {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The job pause, held from the moment the running ticks are gone until the delete takes it.
    pause: Option<cc::JobPause>,
    /// The delete has started: Cancel is too late.
    started: bool,
}

static WAITS: Mutex<Vec<(u64, WaitEntry)>> = Mutex::new(Vec::new());

fn waits() -> std::sync::MutexGuard<'static, Vec<(u64, WaitEntry)>> {
    WAITS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Tokens whose delete has run (newest last, a few kept): a cancel that arrives after the delete finished is
/// "too late", not a fresh entry that would never be cleaned up.
static FINISHED: Mutex<std::collections::VecDeque<u64>> = Mutex::new(std::collections::VecDeque::new());
const FINISHED_KEPT: usize = 64;

fn finished() -> std::sync::MutexGuard<'static, std::collections::VecDeque<u64>> {
    FINISHED.lock().unwrap_or_else(|e| e.into_inner())
}

fn entry<'a>(v: &'a mut Vec<(u64, WaitEntry)>, token: u64) -> &'a mut WaitEntry {
    if let Some(i) = v.iter().position(|(t, _)| *t == token) {
        return &mut v[i].1;
    }
    let e = WaitEntry { cancel: Default::default(), pause: None, started: false };
    v.push((token, e));
    &mut v.last_mut().expect("just pushed").1
}

/// Step 1 of a delete that waits as long as it takes: pauses the job loops (no new tick starts) and blocks
/// until the running ticks are gone, then keeps the pause held under `token`. Needs no store, so the shell
/// calls it on a plain thread and the slow lane stays free. 0 = paused and held (go on with
/// `whimbrel_client_delete_held_json`); 1 = cancelled (`whimbrel_client_cancel_wait`), nothing held. `token`:
/// a number the shell has not used before. Every way out of the wait releases the pause.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_pause_wait(token: u64) -> i32 {
    let cancel = entry(&mut waits(), token).cancel.clone();
    match cc::pause_jobs_wait(cc::PauseWait::Until(&cancel)) {
        Ok(p) => {
            let mut v = waits();
            let e = entry(&mut v, token);
            // A cancel that came while the last tick ended wins: the pause is not kept.
            if e.cancel.load(std::sync::atomic::Ordering::SeqCst) {
                v.retain(|(t, _)| *t != token);
                drop(v);
                drop(p);
                return 1;
            }
            e.pause = Some(p);
            0
        }
        Err(_) => {
            waits().retain(|(t, _)| *t != token);
            1
        }
    }
}

/// Step 2: [`whimbrel_client_delete_json`] with the pause held under `token` (step 1 answered 0), run with the
/// slow-lane store. The request's optional `"reviewed_linked":[{"kind","id"}]` is the linked set the user saw;
/// when it differs now the answer is `selection_changed` and nothing is touched. A cancel that came first
/// answers `cancelled`. Every way out releases the pause and forgets the token.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_delete_held_json(
    handle: *mut StoreHandle,
    request_json: *const c_char,
    device_id: *const c_char,
    group_key_base64: *const c_char,
    token: u64,
) -> *mut c_char {
    let pause = {
        let mut v = waits();
        let e = entry(&mut v, token);
        let cancelled = e.cancel.load(std::sync::atomic::Ordering::SeqCst);
        let p = e.pause.take();
        if cancelled || p.is_none() {
            v.retain(|(t, _)| *t != token);
            drop(v);
            drop(p);
            return out::<()>(&Err(if cancelled { cc::ClientOpError::Cancelled } else { cc::ClientOpError::Busy }));
        }
        e.started = true;
        p
    };
    let (Some(s), Some(req), Some(pause)) = (store(handle), cstr_to_string(request_json), pause) else {
        waits().retain(|(t, _)| *t != token);
        return ptr::null_mut();
    };
    let device = cstr_to_string(device_id);
    let r = cc::parse::<cc::HeldDeleteRequest>(&req).and_then(|req| {
        let key = group_key(group_key_base64)?;
        let r = cc::delete_with_pause(s, &req, device.as_deref(), key.as_ref(), pause);
        if r.is_ok() {
            forget_tokens(&req.request.key);
        }
        r
    });
    {
        let mut f = finished();
        f.push_back(token);
        while f.len() > FINISHED_KEPT {
            f.pop_front();
        }
    }
    waits().retain(|(t, _)| *t != token);
    out(&r)
}

/// Cancels the delete that has `token` before it starts (while it waits for background work or is queued):
/// the pause is released and nothing is deleted. `true` = cancelled; `false` = too late, the delete has started
/// and will finish. A cancel that comes before `whimbrel_client_pause_wait` also counts.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_cancel_wait(token: u64) -> bool {
    let mut v = waits();
    if finished().contains(&token) {
        return false;
    }
    let e = entry(&mut v, token);
    if e.started {
        return false;
    }
    e.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    let held = e.pause.take();
    drop(v);
    drop(held);
    true
}

/// Drops the pause held under `token` and forgets it (the shell could not go on to step 2).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_release_pause(token: u64) {
    let held: Vec<_> = {
        let mut v = waits();
        let (gone, keep): (Vec<_>, Vec<_>) = v.drain(..).partition(|(t, _)| *t == token);
        *v = keep;
        gone
    };
    drop(held);
}

/// How many background ticks are running now: what a waiting delete is waiting for.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_jobs_running() -> u32 {
    u32::try_from(cc::jobs_running()).unwrap_or(u32::MAX)
}

/// Deletes the confirmed due items `{"key","items":[{"kind","id"}],"media_roots","vault"}` on this device
/// only. `sync_on`: sync is set up here; the store must then know this device's id (set before the call), else
/// `no_device_id` with nothing touched. Stops starting new items after `budget_ms` (data `more`: call again).
/// data: the expiry receipt plus `complete`. Job loops are paused while it runs.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_expire_json(
    handle: *mut StoreHandle,
    request_json: *const c_char,
    sync_on: bool,
    budget_ms: u64,
) -> *mut c_char {
    let (Some(s), Some(req)) = (store(handle), cstr_to_string(request_json)) else {
        return ptr::null_mut();
    };
    let r = cc::parse::<cc::ExpireRequest>(&req)
        .and_then(|req| cc::expire(s, &req, now(), sync_on, Duration::from_millis(budget_ms)));
    out(&r)
}

/// An export written but whose media is not copied yet: its client and when it was written.
struct Pending {
    key: String,
    at: std::time::Instant,
    written: cc::ExportWritten,
}

/// How long a media token stays usable.
const TOKEN_TTL: Duration = Duration::from_secs(60 * 60);

/// Exports written but whose media is not copied yet, by token. Taken by `whimbrel_client_export_copy_media_json`;
/// a client's tokens are dropped by its delete, and any token after [`TOKEN_TTL`].
static EXPORTS: Mutex<(u64, Option<HashMap<u64, Pending>>)> = Mutex::new((0, None));

fn exports() -> std::sync::MutexGuard<'static, (u64, Option<HashMap<u64, Pending>>)> {
    let mut g = EXPORTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = g.1.as_mut() {
        m.retain(|_, p| p.at.elapsed() < TOKEN_TTL);
    }
    g
}

/// Drops client `key`'s unused media tokens (its export folders may be gone).
fn forget_tokens(key: &str) {
    if let Some(m) = exports().1.as_mut() {
        m.retain(|_, p| p.key != key);
    }
}

#[derive(Serialize)]
struct Written {
    folder: std::path::PathBuf,
    /// Pass to `whimbrel_client_export_copy_media_json`; null when there is no media to copy.
    media_token: Option<u64>,
}

/// Writes a client export's text (needs the store). `request_json`: `{"key","dest_parent","include_possible":
/// [{"kind","id"}],"include_media","media_roots":[..]}`. data: `{"folder","media_token"}`. With a token, call
/// `whimbrel_client_export_copy_media_json(token)` next, off the store's queue, within an hour.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_export_write_json(handle: *mut StoreHandle, request_json: *const c_char) -> *mut c_char {
    let (Some(s), Some(req)) = (store(handle), cstr_to_string(request_json)) else {
        return ptr::null_mut();
    };
    let r = cc::parse::<cc::ExportRequest>(&req).and_then(|req| {
        let w = cc::export_write(s, &req, now())?;
        let folder = w.export.folder.clone();
        let media_token = (!w.export.media.is_empty()).then(|| {
            let mut g = exports();
            g.0 += 1;
            let token = g.0;
            g.1.get_or_insert_with(HashMap::new).insert(token, Pending { key: req.key.clone(), at: std::time::Instant::now(), written: w });
            token
        });
        Ok(Written { folder, media_token })
    });
    out(&r)
}

/// Copies the media of an export `whimbrel_client_export_write_json` wrote (no store needed). data:
/// `{"folder","media":{"files","skipped","failed"},"partial"}`; `partial`: the text is complete but some media
/// was not copied. An unknown, used, expired, or deleted client's token is `not_found`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_export_copy_media_json(media_token: u64) -> *mut c_char {
    let pending = exports().1.as_mut().and_then(|m| m.remove(&media_token));
    let r = pending.ok_or(cc::ClientOpError::NotFound).and_then(|p| cc::export_copy_media(&p.written));
    out(&r)
}

/// A job loop's tick may start: `true` = go, and call `whimbrel_client_job_end` exactly once when the tick
/// ends; `false` = a client delete or expiry is running, skip this tick.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_job_begin() -> bool {
    cc::job_begin()
}

/// Ends a tick `whimbrel_client_job_begin` allowed.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_client_job_end() {
    cc::job_end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    use crate::{whimbrel_core_free_string, whimbrel_memory_close, whimbrel_memory_open};

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn json(p: *mut c_char) -> serde_json::Value {
        assert!(!p.is_null());
        let v = serde_json::from_str(unsafe { CStr::from_ptr(p) }.to_str().unwrap()).unwrap();
        whimbrel_core_free_string(p);
        v
    }

    /// The job pause is process-wide: tests that delete or check it run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn open() -> (tempfile::TempDir, *mut StoreHandle, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let h = whimbrel_memory_open(c(path.to_str().unwrap()).as_ptr(), ptr::null());
        assert!(!h.is_null());
        (dir, h, path)
    }

    fn seed(h: *mut StoreHandle, name: &str) -> String {
        let p = crate::topics_ffi::whimbrel_topics_seed_save_json(
            h,
            c(&format!(r#"{{"key":null,"name":"{name}","aliases":[],"kind":"customer"}}"#)).as_ptr(),
        );
        json(p)["key"].as_str().unwrap().to_string()
    }

    /// An email filed under `topic`'s client, or only naming it.
    fn email(h: *mut StoreHandle, path: &std::path::Path, ext: &str, subject: &str, topic: Option<i64>) -> i64 {
        let s = unsafe { &(*h).0 };
        s.upsert_email_items(&[memory::NewEmailItem {
            device_id: "dev".into(),
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
        let conn = rusqlite::Connection::open(path).unwrap();
        let id: i64 =
            conn.query_row("SELECT id FROM items WHERE kind = 'email' AND external_id = ?1", [ext], |r| r.get(0)).unwrap();
        if let Some(t) = topic {
            conn.execute(
                "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES ('email', ?1, 1000, 1)",
                [id],
            )
            .unwrap();
            let sid = conn.last_insert_rowid();
            conn.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", [t, sid]).unwrap();
        }
        id
    }

    fn topic_id(h: *mut StoreHandle, key: &str) -> i64 {
        let list = json(crate::topics_ffi::whimbrel_topics_seeds_json(h));
        list.as_array().unwrap().iter().find(|s| s["key"] == key).unwrap()["topic_id"].as_i64().unwrap()
    }

    fn rows(path: &std::path::Path, id: i64) -> i64 {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.query_row("SELECT count(*) FROM items WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }


    /// A cancel after the delete has run is "too late" and leaves no entry behind (review of #875).
    #[test]
    fn a_cancel_after_the_delete_ran_is_too_late_and_leaves_no_entry() {
        let token = 0xF1_2E_5E_D0;
        finished().push_back(token);
        assert!(!whimbrel_client_cancel_wait(token), "too late");
        assert!(waits().iter().all(|(t, _)| *t != token), "no stray entry");
    }
    #[test]
    fn ffi_client_delete_roundtrip() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_d, h, path) = open();
        let key = seed(h, "Plumbrook");
        let t = topic_id(h, &key);
        let linked = email(h, &path, "e1", "pricing sheet", Some(t));
        let named = email(h, &path, "e2", "Plumbrook kickoff notes", None);
        let other = email(h, &path, "e3", "unrelated lunch", None);

        let inv = json(whimbrel_client_inventory_json(h, c(&key).as_ptr(), ptr::null()));
        assert_eq!(inv["ok"], true, "{inv}");
        assert_eq!(inv["data"]["linked"][0]["id"], linked.to_string());
        assert_eq!(inv["data"]["linked"][0]["kind"], "email");
        assert_eq!(inv["data"]["linked"][0]["removed"], false);
        assert!(inv["data"]["possible"].as_array().unwrap().iter().any(|p| p["id"] == named.to_string()));
        assert_eq!(inv["data"]["counts"], serde_json::json!([["email", 1]]));

        // No device id yet and none given, or only the legacy one: refused, nothing touched.
        let req = c(&format!(r#"{{"key":"{key}","include_possible":[{{"kind":"email","id":"{named}"}}]}}"#));
        let refused = json(whimbrel_client_delete_json(h, req.as_ptr(), ptr::null(), ptr::null()));
        assert_eq!(refused, serde_json::json!({"ok": false, "error": {"kind": "no_device_id"}}));
        unsafe { &(*h).0 }.set_own_device_id("spike-device");
        let refused = json(whimbrel_client_delete_json(h, req.as_ptr(), ptr::null(), ptr::null()));
        assert_eq!(refused["error"]["kind"], "no_device_id", "the legacy fallback id is no device id");
        let refused = json(whimbrel_client_delete_json(h, req.as_ptr(), c("spike-device").as_ptr(), ptr::null()));
        assert_eq!(refused["error"]["kind"], "no_device_id");
        assert_eq!(rows(&path, linked), 1);
        // A key that is not a sync key is refused, never treated as "no sync".
        let bad_key = json(whimbrel_client_delete_json(h, req.as_ptr(), c("mac-1").as_ptr(), c("not base64!").as_ptr()));
        assert_eq!(bad_key["error"]["kind"], "invalid_input");
        assert_eq!(rows(&path, linked), 1);
        // A ticked item no inventory lists: refused up front, nothing touched.
        let forged = c(&format!(r#"{{"key":"{key}","include_possible":[{{"kind":"email","id":"{named}"}},{{"kind":"email","id":"{other}"}}]}}"#));
        let changed = json(whimbrel_client_delete_json(h, forged.as_ptr(), c("mac-1").as_ptr(), ptr::null()));
        assert_eq!(changed, serde_json::json!({"ok": false, "error": {"kind": "selection_changed"}}));
        assert_eq!((rows(&path, linked), rows(&path, named), rows(&path, other)), (1, 1, 1));
        // include_possible is required.
        let missing = json(whimbrel_client_delete_json(h, c(&format!(r#"{{"key":"{key}"}}"#)).as_ptr(), c("mac-1").as_ptr(), ptr::null()));
        assert_eq!(missing["error"]["kind"], "invalid_input");

        let group = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let receipt = json(whimbrel_client_delete_json(h, req.as_ptr(), c("mac-1").as_ptr(), c(&group).as_ptr()));
        assert_eq!(receipt["ok"], true, "{receipt}");
        let r = &receipt["data"];
        assert_eq!(r["counts"], serde_json::json!([["email", 2]]));
        assert_eq!(r["failed"], serde_json::json!([]));
        assert_eq!(r["client"], "removed");
        assert_eq!((r["complete"].as_bool(), r["retry_helps"].as_bool()), (Some(true), Some(false)));
        assert!(r["pending_devices"].is_null() || r["pending_devices"].is_u64(), "null is unknown, never 0");
        assert!(r["left_for_you"].as_array().unwrap().contains(&serde_json::json!("copies_on_other_devices")));
        assert!(!receipt.to_string().contains("Plumbrook"), "no client text in the receipt");
        assert_eq!((rows(&path, linked), rows(&path, named), rows(&path, other)), (0, 0, 1));
        assert!(json(crate::topics_ffi::whimbrel_topics_seeds_json(h)).as_array().unwrap().is_empty());
        assert!(whimbrel_client_job_begin(), "the loops run again after the delete");
        whimbrel_client_job_end();

        let gone = json(whimbrel_client_delete_json(h, req.as_ptr(), c("mac-1").as_ptr(), ptr::null()));
        assert_eq!(gone["error"]["kind"], "not_found");
        assert_eq!(json(whimbrel_client_delete_json(h, c("{").as_ptr(), ptr::null(), ptr::null()))["error"]["kind"], "invalid_input");
        assert!(whimbrel_client_delete_json(ptr::null_mut(), req.as_ptr(), ptr::null(), ptr::null()).is_null());
        assert!(whimbrel_client_delete_json(h, ptr::null(), ptr::null(), ptr::null()).is_null());
        let pending = json(whimbrel_client_pending_devices_json(h));
        assert!(pending.get("pending_devices").is_some());
        assert_eq!(json(whimbrel_client_pending_device_ids_json(h)), serde_json::json!({"devices": null}));
        assert_eq!(json(whimbrel_client_retire_device_json(h, c("old").as_ptr()))["error"]["kind"], "not_found");
        assert!(whimbrel_client_retire_device_json(h, ptr::null()).is_null());
        whimbrel_memory_close(h);
    }

    #[test]
    fn ffi_client_control_roundtrip() {
        let (_d, h, _) = open();
        let key = seed(h, "Plumbrook");
        let got = json(whimbrel_client_control_get_json(h, c(&key).as_ptr()));
        assert_eq!(got, serde_json::json!({"ok": true, "data": {"exclude_hosted": false, "keep_months": null}}));
        let set = json(whimbrel_client_control_set_json(h, c(&key).as_ptr(), c(r#"{"exclude_hosted":true,"keep_months":12}"#).as_ptr()));
        assert_eq!(set["data"], serde_json::json!({"exclude_hosted": true, "keep_months": 12}));
        assert_eq!(json(whimbrel_client_control_get_json(h, c(&key).as_ptr()))["data"]["exclude_hosted"], true);
        assert_eq!(json(whimbrel_client_control_set_json(h, c(&key).as_ptr(), c("[]").as_ptr()))["error"]["kind"], "invalid_input");
        assert_eq!(json(whimbrel_client_control_get_json(h, c("nope").as_ptr()))["error"]["kind"], "not_found");
        let offer = json(whimbrel_client_resummary_offer_json(h, c(&key).as_ptr()));
        assert_eq!(offer["data"]["meetings"], serde_json::json!([]));
        assert_eq!(offer["data"]["already_left_note"], api::exclusion::ALREADY_LEFT_NOTE);
        assert!(whimbrel_client_control_get_json(ptr::null_mut(), c(&key).as_ptr()).is_null());
        whimbrel_memory_close(h);
    }

    #[test]
    fn window_ask_destination_says_local_only_for_an_excluded_client_and_names_ollama_cloud() {
        use crate::whimbrel_window_ask_destination as dest;
        let (_d, h, _) = open();
        let key = seed(h, "Plumbrook");
        let on = json(whimbrel_client_control_set_json(h, c(&key).as_ptr(), c(r#"{"exclude_hosted":true,"keep_months":null}"#).as_ptr()));
        assert_eq!(on["ok"], true);
        let hosted = c(r#"{"preferred_provider":"anthropic","preferred_model":"claude"}"#);
        let cloud = c(r#"{"preferred_provider":"ollama","preferred_model":"gpt-oss:120b-cloud","ollama_model":"gpt-oss:120b-cloud"}"#);
        let plain = c(r#"{"app_name":"Notes","title":"Plan","text":"ship it on Friday","is_selection":false}"#);
        let naming = c(r#"{"app_name":"Mail","title":"Re: Plumbrook renewal","text":"see below","is_selection":false}"#);
        let call = |w: &CString, q: &str, r: &CString| json(dest(h, w.as_ptr(), c(q).as_ptr(), r.as_ptr()));
        // Before Return: the window title alone carries the client.
        assert_eq!(call(&naming, "", &hosted), serde_json::json!("local_only"));
        // The question carries it.
        assert_eq!(call(&plain, "what did Plumbrook ask for?", &hosted), serde_json::json!("local_only"));
        assert_eq!(call(&plain, "what is due?", &hosted), serde_json::json!({"hosted": {"provider": "anthropic"}}));
        // An Ollama cloud model leaves this computer.
        assert_eq!(call(&plain, "what is due?", &cloud), serde_json::json!({"hosted": {"provider": "ollama"}}));
        assert!(dest(ptr::null_mut(), plain.as_ptr(), c("").as_ptr(), hosted.as_ptr()).is_null());
        assert!(dest(h, c("{nope").as_ptr(), c("").as_ptr(), hosted.as_ptr()).is_null());
        whimbrel_memory_close(h);
    }

    #[test]
    fn ffi_client_retention_roundtrip() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_d, h, _) = open();
        let key = seed(h, "Plumbrook");
        assert_eq!(json(whimbrel_client_due_items_json(h)), serde_json::json!({"ok": true, "data": []}));
        assert_eq!(json(whimbrel_client_due_items_for_json(h, c(&key).as_ptr())), serde_json::json!({"ok": true, "data": []}));
        let item = c(r#"{"kind":"meeting","id":"4"}"#);
        let keep = json(whimbrel_client_retention_decide_json(h, c(&key).as_ptr(), item.as_ptr(), c(r#"{"decision":"keep"}"#).as_ptr()));
        assert_eq!(keep, serde_json::json!({"ok": true, "data": null}));
        let snooze = c(r#"{"decision":"snooze","until_ms":1}"#);
        assert_eq!(json(whimbrel_client_retention_decide_json(h, c(&key).as_ptr(), item.as_ptr(), snooze.as_ptr()))["ok"], true);
        let bad = json(whimbrel_client_retention_decide_json(h, c(&key).as_ptr(), item.as_ptr(), c(r#"{"decision":"never"}"#).as_ptr()));
        assert_eq!(bad["error"]["kind"], "invalid_input");
        assert_eq!(json(whimbrel_client_review_flag_json(h)), serde_json::json!({"ok": true, "data": null}));
        assert_eq!(json(whimbrel_client_review_flag_dismiss_json(h))["ok"], true);
        let req = c(&format!(r#"{{"key":"{key}","items":[{{"kind":"meeting","id":"4"}}],"media_roots":[]}}"#));
        // With sync on, a store that does not know its own id is refused (X6 final review, Major 21).
        assert_eq!(json(whimbrel_client_expire_json(h, req.as_ptr(), true, 2_000))["error"]["kind"], "no_device_id");
        unsafe { &(*h).0 }.set_own_device_id("mac-1");
        let expired = json(whimbrel_client_expire_json(h, req.as_ptr(), true, 2_000));
        assert_eq!(expired["ok"], true, "{expired}");
        assert_eq!(expired["data"]["not_due"], serde_json::json!([["meeting", 1]]));
        assert_eq!(expired["data"]["more"], false);
        assert!(expired["data"]["waiting_to_sync"].is_array() && expired["data"]["complete"].is_boolean());
        assert_eq!(json(whimbrel_client_expire_json(h, c(r#"{"key":"nope","items":[]}"#).as_ptr(), false, 10))["error"]["kind"], "not_found");
        whimbrel_memory_close(h);
    }

    #[test]
    fn ffi_client_export_roundtrip() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (d, h, path) = open();
        let key = seed(h, "Plumbrook");
        let t = topic_id(h, &key);
        email(h, &path, "e1", "pricing sheet", Some(t));
        let dest = d.path().join("exports");
        std::fs::create_dir(&dest).unwrap();
        // The export reports the canonical folder; a temp dir under a symlink (macOS /var) must still match.
        let dest = dest.canonicalize().unwrap();
        let req = serde_json::json!({"key": key, "dest_parent": dest, "include_possible": [], "include_media": false}).to_string();
        let w = json(whimbrel_client_export_write_json(h, c(&req).as_ptr()));
        assert_eq!(w["ok"], true, "{w}");
        let folder = std::path::PathBuf::from(w["data"]["folder"].as_str().unwrap());
        // The export resolves the destination to its real path (macOS temp folders sit behind a symlink).
        assert!(folder.is_dir() && folder.starts_with(dest.canonicalize().unwrap()), "{}", folder.display());
        assert!(w["data"]["media_token"].is_null(), "no media asked for: nothing to copy");
        assert_eq!(json(whimbrel_client_export_copy_media_json(424242))["error"]["kind"], "not_found");

        // A meeting of the client with its audio in the media folder: the copy goes by token, once.
        let media = d.path().join("media");
        std::fs::create_dir(&media).unwrap();
        let audio = media.join("system-audio-1.m4a");
        std::fs::write(&audio, b"audio").unwrap();
        let s = unsafe { &(*h).0 };
        let m = s
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev".into(),
                start_unix_ms: 10_000,
                end_unix_ms: Some(20_000),
                title: "Kickoff".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: Some(audio.to_string_lossy().into()),
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES ('meeting', ?1, 10000, 1)", [m]).unwrap();
        conn.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", [t, conn.last_insert_rowid()]).unwrap();
        let with = |root: &std::path::Path| {
            serde_json::json!({"key": key, "dest_parent": dest, "include_possible": [], "include_media": true, "media_roots": [root]}).to_string()
        };
        let w = json(whimbrel_client_export_write_json(h, c(&with(&media)).as_ptr()));
        let token = w["data"]["media_token"].as_u64().expect("media to copy");
        let done = json(whimbrel_client_export_copy_media_json(token));
        assert_eq!(done["ok"], true, "{done}");
        assert_eq!((done["data"]["media"]["files"].as_u64(), done["data"]["partial"].as_bool()), (Some(1), Some(false)));
        assert_eq!(json(whimbrel_client_export_copy_media_json(token))["error"]["kind"], "not_found", "a token is used once");

        // A client delete drops its unused tokens.
        let w = json(whimbrel_client_export_write_json(h, c(&with(&media)).as_ptr()));
        let unused = w["data"]["media_token"].as_u64().unwrap();
        let del = c(&format!(r#"{{"key":"{key}","include_possible":[]}}"#));
        assert_eq!(json(whimbrel_client_delete_json(h, del.as_ptr(), c("mac-1").as_ptr(), ptr::null()))["ok"], true);
        assert_eq!(json(whimbrel_client_export_copy_media_json(unused))["error"]["kind"], "not_found");

        let bad = serde_json::json!({"key": "nope", "dest_parent": dest}).to_string();
        assert_eq!(json(whimbrel_client_export_write_json(h, c(&bad).as_ptr()))["error"]["kind"], "not_found");
        whimbrel_memory_close(h);
    }

    #[test]
    fn job_begin_and_end_follow_the_pause() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        assert!(whimbrel_client_job_begin());
        whimbrel_client_job_end();
        let p = cc::pause_jobs(Duration::ZERO);
        assert!(!whimbrel_client_job_begin(), "a delete is running: skip the tick");
        drop(p);
        assert!(whimbrel_client_job_begin());
        whimbrel_client_job_end();
    }
}
