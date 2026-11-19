//! Settings > Capture > "Delete everything on this PC" (X4 Task 12; the Mac's
//! `YourDataSettingsPane.deleteEverything`). The confirmation lists what goes and what stays.
//!
//! On confirm, in this order (each step stops everything after it when it fails):
//! 1. The sync account, if the user asked (the Mac's order: if the server refuses, nothing on
//!    this PC has been touched yet).
//! 2. Activity capture off and the meeting in progress ended, so nothing writes a meeting back.
//! 3. The connected Google accounts (Calendar, Gmail) are noted in the data folder: they stay
//!    connected (Mike, 2026-11-17). Their sign-ins stay in Credential Manager, and the launch that
//!    opens the new database lists them again (`connectors::kept_accounts`).
//! 4. The meeting slide images outside the data folder go at once, under
//!    `delete_everything::check_media_root`'s checks.
//! 5. The note, the MCP server's stop and the restart: everything in the data folder goes at the
//!    next launch (`crate::delete_everything`), once this process and its MCP server have exited,
//!    because Windows refuses to delete a file that is open.
//!
//! The command checks the proof itself (the `main` window, `DELETE` typed): the typed word is
//! not only a UI gate, since every window's script can call any command.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::connectors::kept_accounts::{self, KeptAccount};
use crate::delete_everything::{self as de, MediaRefusal};
use crate::error::CommandError;
use crate::l10n::tr;
use crate::meetings::snapshots::storage;
use crate::state::AppState;

/// What the user types to confirm. The command checks it too.
pub const CONFIRM_WORD: &str = "DELETE";
/// The only window that may delete everything (Settings lives in it).
pub const MAIN_WINDOW: &str = "main";

/// What the confirmation shows.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteEverythingPreview {
    /// The data folder; everything in it goes, the logs too, except the models and the settings file.
    pub data_folder: String,
    pub data_bytes: i64,
    /// The slide-image folder, when it is outside the data folder and holds meetings Whimbrel made.
    pub snapshots_folder: Option<String>,
    pub snapshots_bytes: i64,
    /// Why the slide-image folder is left alone, if it is (a link, or a user folder).
    pub snapshots_refused: Option<String>,
    /// Sync is on: the dialog offers to delete the sync account too.
    pub sync_on: bool,
    /// A sync sign-in is stored in Credential Manager (on or not, Ready or not). It stays unless
    /// the sync account is deleted too.
    pub sync_signed_in: bool,
    /// The connected Google accounts (Calendar and Gmail), which stay connected; `None` when they
    /// can't be listed (the app runs without its database, or the database couldn't list them).
    pub google_accounts: Option<Vec<String>>,
    /// The memory database is open: an unlisted `google_accounts` means the list failed, not
    /// that Whimbrel runs without its database.
    pub store_open: bool,
    /// The meeting being recorded or processed, which Delete everything ends first, if any.
    pub ends_meeting: Option<String>,
}

/// What the dialog says about a meeting in progress: Delete everything ends it first (as the
/// Mac does), and a meeting still being processed is not finished.
pub fn meeting_note(recording: bool, processing: bool) -> Option<String> {
    if recording {
        Some(tr("A meeting is being recorded. Delete everything stops the recording first, and the meeting is deleted with the rest.", &[]))
    } else if processing {
        Some(tr("Whimbrel is still processing a meeting. Delete everything stops that, and the meeting is deleted with the rest.", &[]))
    } else {
        None
    }
}

/// The proof the command checks: called from the main window, with DELETE typed.
pub fn authorize(window: &str, confirm: &str) -> Result<(), String> {
    if window != MAIN_WINDOW {
        return Err(tr("Delete everything can only be confirmed in Settings.", &[]));
    }
    if confirm != CONFIRM_WORD {
        return Err(tr("Type DELETE to confirm.", &[]));
    }
    Ok(())
}

pub fn refusal_text(r: &MediaRefusal) -> String {
    tr(
        "Whimbrel will not delete the slide images because their folder is not safe to delete from (a link, a top-level folder such as Documents, or a folder it couldn't examine). Remove them by hand from: {folder}",
        &[("folder", &r.path().display().to_string())],
    )
}

fn snapshots_root(state: &AppState) -> PathBuf {
    let chosen = state.settings.lock().unwrap_or_else(|p| p.into_inner()).meeting_snapshots_root();
    storage::effective_root(chosen, &state.paths.root.join(storage::ROOT_NAME))
}

fn meeting_in_progress(state: &AppState) -> Option<String> {
    let recording = state.meetings.as_ref().is_some_and(|m| m.is_recording());
    let processing = state.pipeline.as_ref().is_some_and(|p| p.processing().is_some());
    meeting_note(recording, processing)
}

/// The connected Google accounts; `None` without the database (it holds the list).
fn google_accounts(state: &AppState) -> Option<Vec<KeptAccount>> {
    state.store().ok()?.call_blocking(|s| kept_accounts::list(s).ok()).ok()?
}

/// Every address once, in order, for the dialog (one account can be connected to both).
fn addresses(accounts: &[KeptAccount]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in accounts {
        if !out.contains(&a.email) {
            out.push(a.email.clone());
        }
    }
    out
}

fn entry_size(p: &Path) -> i64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => crate::storage::dir_size(p),
        Ok(m) if m.is_file() => m.len() as i64,
        _ => 0,
    }
}

pub fn preview(
    root: &Path,
    snaps: &Path,
    forbidden: &[PathBuf],
    sync_on: bool,
    sync_signed_in: bool,
    google_accounts: Option<Vec<String>>,
    store_open: bool,
    ends_meeting: Option<String>,
) -> DeleteEverythingPreview {
    let media = de::media_plan(root, snaps, forbidden);
    let folders = media.as_ref().map(Vec::clone).unwrap_or_default();
    DeleteEverythingPreview {
        data_folder: root.display().to_string(),
        // The logs go too, though not through the note (`de::clear_logs`).
        data_bytes: de::plan(root).iter().chain([&root.join(de::LOGS)]).map(|p| entry_size(p)).sum(),
        snapshots_folder: (!folders.is_empty()).then(|| snaps.display().to_string()),
        snapshots_bytes: folders.iter().map(|p| crate::storage::dir_size(p)).sum(),
        snapshots_refused: media.err().map(|r| refusal_text(&r)),
        sync_on,
        sync_signed_in,
        google_accounts,
        store_open,
        ends_meeting,
    }
}

/// Works in the no-store mode too: it reads only the folders, the settings file and, when the
/// database is open, the connected accounts.
#[tauri::command]
pub async fn delete_everything_preview(app: AppHandle) -> Result<DeleteEverythingPreview, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let sync_on = state.sync.as_ref().is_some_and(|s| s.ready_credentials().is_some());
        let signed_in = matches!(state.secrets.get_string(crate::secrets::GOOGLE_OAUTH_SERVICE, crate::secrets::accounts::GOOGLE_SYNC), Ok(Some(_)));
        let accounts = google_accounts(&state).map(|a| addresses(&a));
        let store_open = state.store().is_ok();
        preview(&state.paths.root, &snapshots_root(&state), &de::user_folders(), sync_on, signed_in, accounts, store_open, meeting_in_progress(&state))
    })
    .await
    .map_err(|e| CommandError::invalid_input(e.to_string()))
}

/// The confirmed delete's steps, injected so tests can record their order and fail one.
pub struct Steps<'a> {
    /// `Some` when the user asked to delete the sync account too: `Err` means it was not deleted.
    pub delete_sync_account: Option<Box<dyn FnOnce() -> Result<(), String> + 'a>>,
    pub stop_capture: Box<dyn FnOnce() + 'a>,
    /// Stops a recording in progress; `Err` (why) when it could not be stopped.
    pub end_meeting: Box<dyn FnOnce() -> Result<(), String> + 'a>,
    /// Notes the connected Google accounts for the launch after (they stay connected); `Err`
    /// (why) when the note could not be written.
    pub keep_google: Box<dyn FnOnce() -> Result<(), String> + 'a>,
    pub stop_mcp: Box<dyn FnOnce() + 'a>,
    pub restart: Box<dyn FnOnce() + 'a>,
}

/// The confirmed delete (see the module docs for the order). `slides_left_alone`: the dialog
/// already told the user the slide-image folder is left alone (the preview's refusal), so a
/// refusal goes ahead; one the user wasn't shown stops here with nothing changed.
pub fn confirm(root: &Path, snaps: &Path, forbidden: &[PathBuf], slides_left_alone: bool, steps: Steps<'_>) -> Result<(), String> {
    // Before anything changes: a refusal the user hasn't seen is shown now, not only logged.
    if let Err(r) = de::media_plan(root, snaps, forbidden) {
        if !slides_left_alone {
            return Err(format!("{}\n\n{}", refusal_text(&r), tr("Nothing has been deleted. Close this and open Delete everything again to see what it will do.", &[])));
        }
    }
    let account_deleted = steps.delete_sync_account.is_some();
    if let Some(delete) = steps.delete_sync_account {
        delete().map_err(|e| format!("{e}\n\n{}", tr("Nothing on this PC has been deleted.", &[])))?;
    }
    (steps.stop_capture)();
    let stopped = (steps.end_meeting)();
    // What already happened, said with any failure from here on.
    let so_far = |message: String| {
        // Only a restart stops a meeting being processed, so one carries on here.
        let meeting = match &stopped {
            Ok(()) => tr("Activity capture is off for now and any recording was stopped. A meeting Whimbrel was still processing carries on.", &[]),
            Err(e) => tr(
                "Activity capture is off for now, but the recording could not be stopped: {error}. A meeting Whimbrel was still processing carries on.",
                &[("error", e.as_str())],
            ),
        };
        let mut parts = vec![message, meeting];
        if account_deleted {
            parts.push(tr("Your sync account was deleted.", &[]));
        }
        parts.join("\n\n")
    };
    if let Err(e) = (steps.keep_google)() {
        // A write that failed part-way leaves nothing behind for a later launch.
        let _ = kept_accounts::discard(root);
        return Err(so_far(tr(
            "Whimbrel couldn't save the list of your connected Google accounts, so nothing in the data folder has been deleted. Try again. ({error})",
            &[("error", &e)],
        )));
    }
    // From here the list is on disk for the next launch; a stop before the restart removes it
    // again, so it never outlives this delete (`kept_accounts` relies on that).
    let rest = || -> Result<(), String> {
        let media = de::delete_media(root, snaps, forbidden);
        if let Some(r) = &media.refused {
            tracing::warn!(refusal = ?r, "Delete everything leaves the slide-image folder alone");
            if !slides_left_alone {
                return Err(so_far(format!("{}\n\n{}", refusal_text(r), tr("The data folder has not been deleted. Open Delete everything again to go on.", &[]))));
            }
        }
        if !media.not_removed.is_empty() {
            let list = media.not_removed.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
            return Err(so_far(tr("These slide image folders could not be removed, and the data folder was left as it is. Close any program using them and try again:\n{folders}", &[("folders", &list)])));
        }
        de::start(root, &de::plan(root), std::process::id(), steps.stop_mcp, steps.restart).map_err(|e| {
            so_far(tr("The slide images outside the data folder are already deleted, but the rest could not be started: {error}. Quit Whimbrel and remove the data folder by hand.", &[("error", &e.to_string())]))
        })
    };
    let result = rest();
    if result.is_err() {
        if let Err(e) = kept_accounts::discard(root) {
            tracing::warn!(error = %e.kind(), "could not remove the kept Google accounts file after the delete stopped");
        }
    }
    result
}

/// Confirmed in the dialog. Checks its own proof ([`authorize`]); on success the process exits
/// inside this call. Runs off the main thread, so the window keeps painting.
#[tauri::command]
pub async fn delete_everything_now(
    window: tauri::WebviewWindow,
    app: AppHandle,
    confirm: String,
    also_delete_sync_account: bool,
    slides_left_alone: bool,
) -> Result<(), CommandError> {
    authorize(window.label(), &confirm).map_err(CommandError::invalid_input)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        tracing::warn!(also_delete_sync_account, "Delete everything confirmed; the data folder goes at the next launch");
        run(&app, &state, also_delete_sync_account, slides_left_alone)
    })
    .await
    .map_err(|e| CommandError::invalid_input(e.to_string()))?
    .map_err(CommandError::invalid_input)
}

fn run(app: &AppHandle, state: &State<'_, AppState>, also_delete_sync_account: bool, slides_left_alone: bool) -> Result<(), String> {
    let sync = state.sync.clone();
    let delete_sync_account: Option<Box<dyn FnOnce() -> Result<(), String>>> = match (also_delete_sync_account, sync) {
        (false, _) => None,
        (true, Some(sync)) => Some(Box::new(move || sync.delete_account())),
        (true, None) => return Err(tr("Sync isn't on, so there is no sync account to delete from this PC.", &[])),
    };
    let accounts = google_accounts(state);
    let root = state.paths.root.clone();
    let mcp = state.mcp_server.clone();
    let app = app.clone();
    confirm(
        &state.paths.root,
        &snapshots_root(state),
        &de::user_folders(),
        slides_left_alone,
        Steps {
            delete_sync_account,
            stop_capture: Box::new(|| {
                if let Some(capture) = &state.capture {
                    capture.set_enabled(false);
                }
                // Off wins over a timed pause: no deadline is left behind.
                let mut settings = state.settings.lock().unwrap_or_else(|p| p.into_inner());
                settings.set_capture_resume_at(None);
                let _ = settings.save();
            }),
            end_meeting: Box::new(|| {
                if let Some(meetings) = &state.meetings {
                    if let Err(e) = meetings.stop() {
                        tracing::warn!(error = %e, "Delete everything could not stop the recording cleanly");
                        return Err(e.to_string());
                    }
                }
                Ok(())
            }),
            // Without the database there is no list: their sign-ins stay, unlisted, and an older
            // list goes (the dialog says they need connecting again).
            keep_google: Box::new(move || kept_accounts::keep(&root, accounts.as_deref()).map_err(|e| e.to_string())),
            stop_mcp: Box::new(move || {
                if let Some(mcp) = mcp {
                    mcp.stop();
                }
            }),
            restart: Box::new(move || crate::commands::models::restart_app(app)),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup(name: &str) -> (Tmp, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("whimbrel-delete-cmd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("data");
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::create_dir_all(root.join("recordings")).unwrap();
        std::fs::write(root.join("recordings").join("a.wav"), b"wav").unwrap();
        std::fs::write(root.join("kestrel.sqlite"), b"db").unwrap();
        let snaps = base.join("Pictures").join("Whimbrel Snapshots");
        let meeting = snaps.join("2026-11-15 09.30 Review");
        std::fs::create_dir_all(&meeting).unwrap();
        std::fs::write(meeting.join(api::retention::SNAPSHOT_MARKER), b"").unwrap();
        std::fs::write(meeting.join("09.30.01.png"), b"png").unwrap();
        (Tmp(base), root, snaps)
    }

    type Log = Rc<RefCell<Vec<String>>>;

    /// Steps that log, in order. `sync`: `None` = not asked; `Some(r)` = asked, answering `r`.
    fn steps(log: &Log, root: &Path, snaps: &Path, sync: Option<Result<(), String>>, google_fails: Option<String>) -> Steps<'static> {
        let (a, b, c, d, e, f) = (log.clone(), log.clone(), log.clone(), log.clone(), log.clone(), log.clone());
        let (root, slide) = (root.to_path_buf(), snaps.join("2026-11-15 09.30 Review"));
        Steps {
            delete_sync_account: sync.map(|r| {
                Box::new(move || {
                    a.borrow_mut().push("sync account".into());
                    r
                }) as Box<dyn FnOnce() -> Result<(), String>>
            }),
            stop_capture: Box::new(move || b.borrow_mut().push("capture off".into())),
            end_meeting: Box::new(move || {
                c.borrow_mut().push("end meeting".into());
                Ok(())
            }),
            keep_google: Box::new(move || {
                d.borrow_mut().push("google".into());
                google_fails.map_or(Ok(()), Err)
            }),
            stop_mcp: Box::new(move || {
                e.borrow_mut().push(format!("stop mcp (slides gone: {}, note: {})", !slide.exists(), root.join(de::NOTE_NAME).exists()))
            }),
            restart: Box::new(move || f.borrow_mut().push("restart".into())),
        }
    }

    #[test]
    fn only_the_main_window_with_delete_typed_may_confirm() {
        assert!(authorize("main", "DELETE").is_ok());
        assert!(authorize("tray-popup", "DELETE").is_err(), "another window's script");
        assert!(authorize("main", "").is_err(), "no proof");
        assert!(authorize("main", "delete").is_err(), "exactly DELETE");
        assert!(authorize("main", "DELETE ").is_err());
    }

    #[test]
    fn a_meeting_in_progress_is_ended_first_and_said_so() {
        assert!(meeting_note(true, false).is_some_and(|m| m.contains("stops the recording first")));
        assert!(meeting_note(true, true).is_some_and(|m| m.contains("being recorded")), "recording says so first");
        assert!(meeting_note(false, true).is_some_and(|m| m.contains("processing")));
        assert_eq!(meeting_note(false, false), None);
    }

    #[test]
    fn confirm_runs_account_capture_meeting_google_slides_note_mcp_restart_in_that_order() {
        let (_t, root, snaps) = setup("order");
        let log = Log::default();
        confirm(&root, &snaps, &[], false, steps(&log, &root, &snaps, Some(Ok(())), None)).unwrap();
        assert_eq!(
            *log.borrow(),
            ["sync account", "capture off", "end meeting", "google", "stop mcp (slides gone: true, note: true)", "restart"]
        );
        assert!(root.join("recordings").join("a.wav").is_file(), "the data folder waits for the next launch");
        let note = std::fs::read_to_string(root.join(de::NOTE_NAME)).unwrap();
        assert!(!note.contains("models") && !note.contains("Snapshots"), "{note}");
    }

    #[test]
    fn a_sync_account_the_server_did_not_delete_stops_everything_else() {
        let (_t, root, snaps) = setup("sync-refused");
        let log = Log::default();
        let e = confirm(&root, &snaps, &[], false, steps(&log, &root, &snaps, Some(Err("Your sync account wasn't deleted. Try again.".into())), None)).unwrap_err();
        assert!(e.contains("wasn't deleted") && e.contains("Nothing on this PC has been deleted"), "{e}");
        assert_eq!(*log.borrow(), ["sync account"]);
        assert!(snaps.join("2026-11-15 09.30 Review").is_dir() && !root.join(de::NOTE_NAME).exists());
    }

    #[test]
    fn without_the_account_option_the_sync_account_is_never_touched() {
        let (_t, root, snaps) = setup("no-sync");
        let log = Log::default();
        confirm(&root, &snaps, &[], false, steps(&log, &root, &snaps, None, None)).unwrap();
        assert!(!log.borrow().iter().any(|s| s == "sync account"));
    }

    #[test]
    fn an_account_list_that_could_not_be_saved_stops_before_any_delete() {
        let (_t, root, snaps) = setup("google");
        let log = Log::default();
        let e = confirm(&root, &snaps, &[], false, steps(&log, &root, &snaps, Some(Ok(())), Some("disk full".into()))).unwrap_err();
        assert!(e.contains("disk full") && e.contains("nothing in the data folder has been deleted"), "{e}");
        assert!(e.contains("any recording was stopped") && e.contains("Your sync account was deleted."), "says what already happened: {e}");
        assert!(snaps.join("2026-11-15 09.30 Review").is_dir() && !root.join(de::NOTE_NAME).exists());
        assert!(!log.borrow().iter().any(|s| s == "restart"));
    }

    #[test]
    fn a_refused_slide_folder_the_user_was_shown_is_left_alone_and_the_rest_goes_ahead() {
        let (_t, root, snaps) = setup("refused");
        let log = Log::default();
        confirm(&root, &snaps, &[snaps.clone()], true, steps(&log, &root, &snaps, None, None)).unwrap();
        assert!(log.borrow().iter().any(|s| s == "restart"));
        assert!(snaps.join("2026-11-15 09.30 Review").join("09.30.01.png").is_file());
        let p = preview(&root, &snaps, &[snaps.clone()], false, false, Some(vec![]), true, None);
        assert!(p.snapshots_folder.is_none() && p.snapshots_refused.is_some_and(|r| r.contains("not safe to delete from")));
    }

    #[test]
    fn a_refused_slide_folder_the_user_was_not_shown_is_shown_and_nothing_changes() {
        let (_t, root, snaps) = setup("refused-unseen");
        let log = Log::default();
        let e = confirm(&root, &snaps, &[snaps.clone()], false, steps(&log, &root, &snaps, Some(Ok(())), None)).unwrap_err();
        assert!(e.contains("not safe to delete from") && e.contains("Nothing has been deleted"), "{e}");
        assert!(log.borrow().is_empty(), "decided before the sync account or anything else: {:?}", log.borrow());
    }

    #[test]
    fn the_preview_counts_the_data_folder_without_the_models_and_lists_what_goes() {
        let (_t, root, snaps) = setup("preview");
        std::fs::write(root.join("models").join("big.bin"), vec![0u8; 1000]).unwrap();
        let p = preview(&root, &snaps, &[], true, true, Some(vec!["a@example.com".into()]), true, meeting_note(true, false));
        assert_eq!(p.data_bytes, 5, "a.wav + kestrel.sqlite, never the models");
        assert_eq!(p.snapshots_folder.as_deref(), Some(snaps.display().to_string().as_str()));
        assert_eq!(p.snapshots_bytes, 3);
        assert!(p.sync_on && p.snapshots_refused.is_none() && p.ends_meeting.is_some());
        assert_eq!(p.google_accounts, Some(vec!["a@example.com".to_string()]));
    }

    /// A failed start leaves the process running: a meeting it was processing carries on, and
    /// capture comes back after a restart. The message says only what is true.
    #[test]
    fn a_failed_start_does_not_claim_the_processing_meeting_was_ended() {
        let (_t, root, snaps) = setup("start-fails");
        std::fs::create_dir_all(root.join(de::NOTE_NAME)).unwrap(); // the note can't be written
        let log = Log::default();
        let e = confirm(&root, &snaps, &[], false, steps(&log, &root, &snaps, None, None)).unwrap_err();
        assert!(!e.contains("was ended"), "{e}");
        assert!(e.contains("still processing carries on"), "{e}");
        assert!(!log.borrow().iter().any(|s| s == "restart"));
    }

    /// The kept accounts' list is only for the launch after a restart: a delete that stops before
    /// it removes the list again, so a later launch never restores stale defaults.
    #[test]
    fn a_delete_that_stops_after_the_list_was_written_removes_it() {
        let (_t, root, snaps) = setup("keep-then-stop");
        let writes_list = |root: &Path| -> Box<dyn FnOnce() -> Result<(), String>> {
            let r = root.to_path_buf();
            Box::new(move || {
                kept_accounts::write(&r, &[KeptAccount { connector: "gmail".into(), email: "a@example.com".into(), default: true }]).map_err(|e| e.to_string())
            })
        };
        std::fs::create_dir_all(root.join(de::NOTE_NAME)).unwrap(); // the note can't be written
        let log = Log::default();
        let mut s = steps(&log, &root, &snaps, None, None);
        s.keep_google = writes_list(&root);
        assert!(confirm(&root, &snaps, &[], false, s).is_err());
        assert!(!root.join(kept_accounts::FILE_NAME).exists());
        // A delete that goes on to the restart leaves it for the next launch.
        std::fs::remove_dir_all(root.join(de::NOTE_NAME)).unwrap();
        let mut s = steps(&log, &root, &snaps, None, None);
        s.keep_google = writes_list(&root);
        confirm(&root, &snaps, &[], false, s).unwrap();
        assert!(root.join(kept_accounts::FILE_NAME).is_file());
    }

    /// A recording that could not be stopped is not claimed stopped in the messages after it.
    #[test]
    fn a_recording_that_could_not_be_stopped_is_not_claimed_stopped() {
        let (_t, root, snaps) = setup("stop-fails");
        let log = Log::default();
        let mut s = steps(&log, &root, &snaps, None, Some("disk full".into()));
        s.end_meeting = Box::new(|| Err("the microphone is stuck".into()));
        let e = confirm(&root, &snaps, &[], false, s).unwrap_err();
        assert!(!e.contains("any recording was stopped"), "{e}");
        assert!(e.contains("could not be stopped") && e.contains("the microphone is stuck"), "{e}");
        assert!(e.contains("still processing carries on"), "{e}");
    }

    /// The command checks its proof before anything else (a source check: the command needs a
    /// running Tauri app to call).
    #[test]
    fn the_command_authorizes_before_it_does_anything() {
        let src = include_str!("delete_everything.rs").replace("\r\n", "\n");
        let body = src.split("pub async fn delete_everything_now(").nth(1).unwrap();
        let body = body.split("\nfn run(").next().unwrap();
        let auth = body.find("authorize(window.label(), &confirm)").expect("authorize is called");
        let work = body.find("spawn_blocking").expect("the work runs off the main thread");
        assert!(auth < work, "authorize runs first");
    }
}
