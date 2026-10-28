//! Auto-update (M3.5; docs/12 §7), the Windows counterpart of the Mac's `UpdateChecker`.
//!
//! A release build checks `windows/latest.json` on the updates Worker at launch and then
//! daily, downloads a newer installer in the background (`tauri-plugin-updater` verifies
//! its minisign signature against the `pubkey` in `tauri.conf.json`), and holds it until
//! the user picks "Restart now" in the UI's banner. It keeps checking while one is waiting:
//! a release newer than the staged one replaces it, so a user who leaves the banner for a
//! few days installs the latest, not the first they were offered, and an update that is
//! already staged is never downloaded twice. Installing runs the NSIS installer in
//! passive mode and exits with 0, so the crash supervisor ends too; the installer closes
//! anything still running from the old exe and relaunches the new one, which starts a
//! supervisor of its own.

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::CommandError;
use crate::state::AppState;

const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long the installer waits for the MCP server to stop before going ahead without it.
const STOP_SERVER_WITHIN: Duration = Duration::from_secs(5);

/// Runs `work` on its own thread and waits up to `limit` for it; `false` if it did not finish.
/// A hung job is left behind (it holds nothing the installer needs) rather than blocking the caller.
fn run_bounded(limit: Duration, work: impl FnOnce() + Send + 'static) -> bool {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        work();
        let _ = done.send(());
    });
    finished.recv_timeout(limit).is_ok()
}

/// Makes `new` the staged update **only if the download succeeded**, returning what it replaced.
/// On failure the slot is untouched, so a partial or failed download leaves the old update waiting.
fn replace_if_ok<T, E>(slot: &mut Option<T>, new: Result<T, E>) -> Result<Option<T>, E> {
    let new = new?;
    Ok(slot.replace(new))
}

/// A downloaded, signature-checked update waiting for "Restart now".
// ponytail: the installer bytes stay in memory until then (tens of MB); write them to a
// file if that ever matters.
#[derive(Default)]
pub struct StagedUpdate(Mutex<Option<(Update, Vec<u8>)>>, Mutex<Option<String>>);

impl StagedUpdate {
    /// The version waiting for "Restart now", if any.
    fn version(&self) -> Option<String> {
        self.0.lock().unwrap().as_ref().map(|(update, _)| update.version.clone())
    }
}

pub fn start(app: &AppHandle) {
    app.manage(StagedUpdate::default());
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("whimbrel-update".into()).spawn(move || loop {
        match tauri::async_runtime::block_on(check_and_stage(&app)) {
            Ok(Checked::NothingNewer) => tracing::debug!("no update available"),
            Ok(Checked::AlreadyStaged(version)) => tracing::debug!(%version, "the latest update is already downloaded"),
            Ok(Checked::Staged { version, replaced: None }) => {
                tracing::info!(%version, "update downloaded; waiting for Restart now");
            }
            Ok(Checked::Staged { version, replaced: Some(old) }) => {
                tracing::info!(%version, replaced = %old, "a newer update was downloaded and replaced the one waiting");
            }
            Err(e) => tracing::warn!(error = %e, "update check failed"),
        }
        std::thread::sleep(CHECK_EVERY);
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the update checker");
    }
}

/// What one check found and did.
#[derive(Debug, PartialEq, Eq)]
enum Checked {
    /// The running version is the latest.
    NothingNewer,
    /// The latest is the one already waiting; nothing was downloaded.
    AlreadyStaged(String),
    /// A download finished and is now the staged update, in place of `replaced` if there was one.
    Staged { version: String, replaced: Option<String> },
}

/// What to do with the newest release a check found, given the update already waiting.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Download it and make it the staged update.
    Stage,
    /// The staged update is already this one or newer.
    Keep,
}

/// `major.minor.patch` of a version string, ignoring any `-pre` or `+build` suffix.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let triple = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(triple)
}

fn decide(staged: Option<&str>, candidate: &str) -> Decision {
    let Some(staged) = staged else { return Decision::Stage };
    if staged == candidate {
        return Decision::Keep;
    }
    match (parse_version(staged), parse_version(candidate)) {
        (Some(have), Some(found)) if found <= have => Decision::Keep,
        // Newer, or a version this cannot order: the server's latest wins.
        _ => Decision::Stage,
    }
}

/// A line for the log when what the banner would show changes, so a missing banner can be told
/// apart from an update that never arrived. `last` is what the UI was told last time.
fn banner_change(last: &mut Option<String>, now: Option<String>) -> Option<String> {
    if *last == now {
        return None;
    }
    let line = match (&*last, &now) {
        (None, Some(v)) => format!("the update banner now offers {v}"),
        (Some(old), Some(v)) => format!("the update banner changed from {old} to {v}"),
        (Some(old), None) => format!("the update banner no longer offers {old}"),
        (None, None) => return None,
    };
    *last = now;
    Some(line)
}

async fn check_and_stage(app: &AppHandle) -> Result<Checked, tauri_plugin_updater::Error> {
    let Some(update) = app.updater()?.check().await? else { return Ok(Checked::NothingNewer) };
    let staged = app.state::<StagedUpdate>().version();
    if decide(staged.as_deref(), &update.version) == Decision::Keep {
        return Ok(Checked::AlreadyStaged(staged.unwrap_or_default()));
    }
    // `download` verifies the minisign signature against the configured key before it returns the bytes.
    let downloaded = update.download(|_, _| {}, || {}).await.map(|bytes| (update.clone(), bytes));
    let version = update.version.clone();
    // The old update (and its installer bytes) is dropped here; a failed download changes nothing.
    replace_if_ok(&mut app.state::<StagedUpdate>().0.lock().unwrap(), downloaded)?;
    Ok(Checked::Staged { version, replaced: staged })
}

/// The version of a downloaded update, if one is ready to install.
#[tauri::command]
pub fn update_ready(staged: State<'_, StagedUpdate>) -> Option<String> {
    let now = staged.version();
    if let Some(line) = banner_change(&mut staged.1.lock().unwrap(), now.clone()) {
        tracing::info!("{line}");
    }
    now
}

/// Installs the downloaded update. On success the process exits inside this call.
#[tauri::command]
pub fn install_update(staged: State<'_, StagedUpdate>, state: State<'_, AppState>) -> Result<(), CommandError> {
    let Some((update, bytes)) = staged.0.lock().unwrap().take() else {
        return Err(CommandError::invalid_input("no update has been downloaded"));
    };
    // `install` exits the process directly, skipping `RunEvent::Exit`, so a recording is
    // stopped here the way Quit stops it.
    if let Some(meetings) = &state.meetings {
        if meetings.is_recording() {
            let _ = meetings.stop();
        }
    }
    // The installer overwrites whimbrel-mcp-server.exe; stopping it here means that file is not
    // still locked by a server that is slow to notice the app is going away.
    // Bounded: if stopping it hangs for any reason, the installer still runs.
    if let Some(server) = state.mcp_server.clone() {
        if !run_bounded(STOP_SERVER_WITHIN, move || {
            server.stop();
        }) {
            tracing::warn!("the MCP server did not stop in time; installing anyway");
        }
    }
    tracing::info!(version = %update.version, "installing update");
    // The installer inherits this process's environment and relaunches the app with it.
    let supervised = crate::restart::leave_supervision();
    let result = update.install(bytes);
    // Still here: the installer didn't start, and this app carries on under its supervisor.
    if supervised {
        crate::restart::rejoin_supervision();
    }
    result.map_err(|e| CommandError::network(format!("could not start the installer: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_nothing_staged_the_latest_is_downloaded() {
        assert_eq!(decide(None, "0.9.18"), Decision::Stage);
    }

    #[test]
    fn the_update_already_waiting_is_not_downloaded_again() {
        assert_eq!(decide(Some("0.9.18"), "0.9.18"), Decision::Keep);
    }

    #[test]
    fn a_newer_release_replaces_the_one_waiting() {
        assert_eq!(decide(Some("0.9.18"), "0.9.19"), Decision::Stage);
        assert_eq!(decide(Some("0.9.18"), "0.10.0"), Decision::Stage, "0.10 is newer than 0.9, not older");
        assert_eq!(decide(Some("0.9.9"), "0.9.10"), Decision::Stage, "numbers, not text");
        assert_eq!(decide(Some("0.9.18"), "1.0.0"), Decision::Stage);
    }

    #[test]
    fn an_older_release_never_replaces_a_newer_one_waiting() {
        assert_eq!(decide(Some("0.9.19"), "0.9.18"), Decision::Keep);
        assert_eq!(decide(Some("0.10.0"), "0.9.99"), Decision::Keep);
    }

    #[test]
    fn a_version_that_cannot_be_ordered_defers_to_the_server() {
        assert_eq!(decide(Some("0.9.18"), "nightly"), Decision::Stage);
        assert_eq!(decide(Some("nightly"), "0.9.18"), Decision::Stage);
        assert_eq!(decide(Some("nightly"), "nightly"), Decision::Keep, "the same label is the same update");
    }

    #[test]
    fn versions_are_read_ignoring_a_leading_v_and_suffixes() {
        assert_eq!(parse_version("0.9.21"), Some((0, 9, 21)));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3+build5"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("x.y.z"), None);
    }

    #[test]
    fn the_log_says_when_the_banner_starts_changes_and_stops_offering_an_update() {
        let mut last = None;
        assert_eq!(banner_change(&mut last, None), None, "nothing to say while nothing is offered");
        assert_eq!(banner_change(&mut last, Some("0.9.18".into())).as_deref(), Some("the update banner now offers 0.9.18"));
        assert_eq!(banner_change(&mut last, Some("0.9.18".into())), None, "polling the same value is silent");
        assert_eq!(
            banner_change(&mut last, Some("0.9.19".into())).as_deref(),
            Some("the update banner changed from 0.9.18 to 0.9.19")
        );
        assert_eq!(banner_change(&mut last, None).as_deref(), Some("the update banner no longer offers 0.9.19"));
        assert_eq!(banner_change(&mut last, None), None);
    }

    // --- replacing the staged update ---

    #[test]
    fn a_whole_download_replaces_the_staged_one_and_hands_back_the_old() {
        let mut slot = Some("0.9.18");
        let old = replace_if_ok::<_, String>(&mut slot, Ok("0.9.19")).unwrap();
        assert_eq!(old, Some("0.9.18"));
        assert_eq!(slot, Some("0.9.19"));
    }

    #[test]
    fn a_download_that_fails_or_comes_up_short_leaves_the_staged_one_waiting() {
        let mut slot = Some("0.9.18");
        let err = replace_if_ok(&mut slot, Err::<&str, _>("signature did not verify")).unwrap_err();
        assert_eq!(err, "signature did not verify");
        assert_eq!(slot, Some("0.9.18"), "the update the user can install is still there");
    }

    #[test]
    fn staging_into_an_empty_slot_replaces_nothing() {
        let mut slot: Option<&str> = None;
        assert_eq!(replace_if_ok::<_, String>(&mut slot, Ok("0.9.19")).unwrap(), None);
        assert_eq!(slot, Some("0.9.19"));
    }

    // --- stopping the server before the installer ---

    #[test]
    fn work_that_finishes_in_time_is_reported_done() {
        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ran.clone();
        assert!(run_bounded(Duration::from_secs(5), move || flag.store(true, std::sync::atomic::Ordering::SeqCst)));
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn work_that_hangs_does_not_hold_the_installer_up() {
        let started = std::time::Instant::now();
        let finished = run_bounded(Duration::from_millis(200), || std::thread::sleep(Duration::from_secs(30)));
        assert!(!finished, "a hung stop is reported as not finished");
        assert!(started.elapsed() < Duration::from_secs(5), "and the caller got control back after the limit");
    }
}
