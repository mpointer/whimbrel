//! Meeting snapshots in the UI (plan Task 7): the workspace's Slides strip and the
//! snapshot rows in Settings → Meetings.
//!
//! Every command takes a meeting and a snapshot id, never a path: the path comes from
//! that meeting's own rows, so the UI can only open, copy or show files the snapshot
//! service wrote. File work runs on a blocking thread, not inside the store actor.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use base64::Engine;
use memory::Store;
use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::error::CommandError;
use crate::meetings::snapshots::storage;
use crate::state::AppState;

/// The longest side of a strip thumbnail, in pixels: the Mac's size, sharp at 2x for the
/// strip's 120 px row.
const THUMBNAIL_PX: u32 = 480;
/// How much slide text each tile carries for its tooltip. The list is fetched every 2 s.
const TEXT_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotView {
    pub id: i64,
    pub captured_unix_ms: i64,
    pub file_name: String,
    pub width: i64,
    pub height: i64,
    /// The file's modification time: a build that replaces a slide can reuse an earlier
    /// file name, so the strip keys its thumbnails on this too.
    pub modified_unix_ms: i64,
    /// The start of the slide's text.
    pub text: String,
}

/// One meeting's snapshots, oldest first, leaving out any whose file is gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotsView {
    pub snapshots: Vec<SnapshotView>,
    /// "Keep snapshots": retention leaves this meeting's folder alone.
    pub keep: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Thumbnail {
    /// A `data:image/png;base64,` URL, which the CSP's `img-src data:` allows.
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

/// Settings → Meetings' snapshot rows. Local to this PC, never synced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSettings {
    /// "Save slides and shared screens". On by default.
    pub enabled: bool,
    /// Where new meetings' folders go.
    pub folder: String,
    /// No folder has been chosen, so `folder` is the default.
    pub is_default: bool,
}

fn modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
}

fn snippet(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.char_indices().nth(TEXT_CHARS) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text,
    }
}

/// The strip's view of a meeting. Pure over the store, apart from a `stat` per file.
pub fn snapshots_view(s: &Store, meeting_id: i64) -> Result<SnapshotsView, String> {
    let keep = s.meeting_snapshots_info(meeting_id).map_err(|e| e.to_string())?.is_some_and(|(_, keep)| keep);
    let snapshots = s
        .list_meeting_snapshots(meeting_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|snap| {
            let path = Path::new(&snap.file_path);
            Some(SnapshotView {
                id: snap.id,
                captured_unix_ms: snap.captured_unix_ms,
                file_name: path.file_name()?.to_string_lossy().into_owned(),
                width: snap.width,
                height: snap.height,
                modified_unix_ms: modified_ms(path)?,
                text: snippet(&snap.ocr_text),
            })
        })
        .collect();
    Ok(SnapshotsView { snapshots, keep })
}

/// The file of snapshot `id`, if it belongs to `meeting_id`.
fn snapshot_path(s: &Store, meeting_id: i64, id: i64) -> Result<PathBuf, String> {
    s.list_meeting_snapshots(meeting_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|snap| snap.id == id)
        .map(|snap| PathBuf::from(snap.file_path))
        .ok_or_else(|| "That slide is no longer saved.".to_string())
}

/// Deletes snapshot `id` of `meeting_id`: the row first, then its file, so a failed
/// delete never leaves a row pointing at a file that isn't there.
pub fn delete_snapshot(s: &Store, meeting_id: i64, id: i64) -> Result<(), String> {
    snapshot_path(s, meeting_id, id)?;
    if let Some(path) = s.delete_meeting_snapshot(id).map_err(|e| e.to_string())? {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// A PNG thumbnail of `path` no larger than `max` on either side.
pub fn thumbnail(path: &Path, max: u32) -> Result<Thumbnail, String> {
    let image = image::open(path).map_err(|e| format!("couldn't read the slide: {e}"))?;
    // `thumbnail` also scales up, which a small slide doesn't need.
    let small = if image.width() <= max && image.height() <= max { image.to_rgb8() } else { image.thumbnail(max, max).to_rgb8() };
    let mut png = Vec::new();
    small.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(Thumbnail {
        data_url: format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png)),
        width: small.width(),
        height: small.height(),
    })
}

/// A packed CF_DIB: a `BITMAPINFOHEADER`, then 32-bit BGRX rows bottom-up (what every
/// app pastes; 4 bytes a pixel needs no row padding). The spare byte is 255, since some
/// apps read it as alpha.
pub fn dib(rgb: &image::RgbImage) -> Vec<u8> {
    let (width, height) = rgb.dimensions();
    let pixels = width as usize * height as usize * 4;
    let mut out = Vec::with_capacity(40 + pixels);
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes()); // positive: bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(pixels as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]); // resolution and palette: unused
    for row in rgb.rows().rev() {
        for p in row {
            out.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    out
}

fn main_hwnd(app: &AppHandle) -> Option<windows::Win32::Foundation::HWND> {
    let window = app.get_webview_window(crate::app::MAIN_WINDOW)?;
    Some(windows::Win32::Foundation::HWND(window.hwnd().ok()?.0))
}

/// Puts a CF_DIB on the clipboard, owned by `owner`. Another app can hold the clipboard
/// for a moment, so opening it is retried briefly.
fn set_clipboard_dib(owner: Option<windows::Win32::Foundation::HWND>, dib: &[u8]) -> Result<(), String> {
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_DIB;

    if !(0..5).any(|attempt| {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
        unsafe { OpenClipboard(owner).is_ok() }
    }) {
        return Err("Another app is using the clipboard. Try again in a moment.".into());
    }
    let result = unsafe {
        (|| {
            EmptyClipboard().map_err(|e| e.to_string())?;
            let memory = GlobalAlloc(GMEM_MOVEABLE, dib.len()).map_err(|e| e.to_string())?;
            let target = GlobalLock(memory);
            if target.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err("couldn't lock the clipboard memory".to_string());
            }
            std::ptr::copy_nonoverlapping(dib.as_ptr(), target.cast::<u8>(), dib.len());
            let _ = GlobalUnlock(memory);
            // Once this succeeds the clipboard owns the memory, so it's never freed here.
            if let Err(e) = SetClipboardData(CF_DIB.0.into(), Some(HANDLE(memory.0))) {
                let _ = GlobalFree(Some(memory));
                return Err(e.to_string());
            }
            Ok(())
        })()
    };
    unsafe {
        let _ = CloseClipboard();
    }
    result
}

/// `ShellExecuteW("open", file, params)`: the default viewer for a PNG, Explorer for a
/// folder or with `/select`.
fn shell_open(file: &str, params: Option<&str>) -> Result<(), String> {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let params = params.map(HSTRING::from);
    let params = params.as_ref().map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr()));
    let result = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(file), params, PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("Windows couldn't open it (ShellExecute returned {})", result.0 as isize))
    }
}

/// Opens Explorer with `file` selected. The path is quoted: Pictures paths have spaces,
/// and a Windows path can't contain a `"`.
fn show_in_explorer(file: &Path) -> Result<(), String> {
    shell_open("explorer.exe", Some(&format!("/select,\"{}\"", file.display())))
}

/// The native folder picker (`IFileOpenDialog` with `FOS_PICKFOLDERS`), owned by the main
/// window so it opens in front of it. `None` when the user cancels. Runs on a thread of
/// its own, initialised for COM's single-threaded apartment as the dialog needs.
fn pick_folder(owner: Option<windows::Win32::Foundation::HWND>, start: PathBuf) -> Result<Option<PathBuf>, String> {
    let owner = owner.map(|h| h.0 as isize);
    std::thread::Builder::new()
        .name("whimbrel-folder-picker".into())
        .spawn(move || unsafe { pick_folder_on_this_thread(owner.map(|h| windows::Win32::Foundation::HWND(h as _)), &start) })
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| "the folder picker stopped unexpectedly".to_string())?
}

unsafe fn pick_folder_on_this_thread(owner: Option<windows::Win32::Foundation::HWND>, start: &Path) -> Result<Option<PathBuf>, String> {
    use windows::core::{w, HSTRING};
    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
        FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    let initialised = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
    let picked = (|| {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let options = dialog.GetOptions().map_err(|e| e.to_string())?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
            .map_err(|e| e.to_string())?;
        let _ = dialog.SetTitle(w!("Choose where meeting snapshots go"));
        // Open where snapshots go now, or the nearest folder of it that exists.
        if let Some(existing) = start.ancestors().find(|p| p.is_dir()) {
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(existing.as_os_str()), None) {
                let _ = dialog.SetFolder(&item);
            }
        }
        match dialog.Show(owner) {
            Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => return Ok(None),
            Err(e) => return Err(e.to_string()),
            Ok(()) => {}
        }
        let name = dialog.GetResult().and_then(|item| item.GetDisplayName(SIGDN_FILESYSPATH)).map_err(|e| e.to_string())?;
        let path = name.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(Some(PathBuf::from(path?)))
    })();
    if initialised {
        CoUninitialize();
    }
    picked
}

fn fallback_root(state: &AppState) -> PathBuf {
    state.paths.root.join(storage::ROOT_NAME)
}

fn read_settings(file: &crate::settings::SettingsFile, fallback: &Path) -> SnapshotSettings {
    let chosen = file.meeting_snapshots_root();
    SnapshotSettings {
        enabled: file.meeting_snapshots_enabled(),
        is_default: chosen.is_none(),
        folder: storage::effective_root(chosen, fallback).display().to_string(),
    }
}

fn save(state: &AppState, change: impl FnOnce(&mut crate::settings::SettingsFile)) -> Result<SnapshotSettings, CommandError> {
    let mut file = state.settings.lock().unwrap();
    change(&mut file);
    file.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    Ok(read_settings(&file, &fallback_root(state)))
}

async fn blocking<R: Send + 'static>(work: impl FnOnce() -> Result<R, String> + Send + 'static) -> Result<R, CommandError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| CommandError::store(e.to_string()))?
        .map_err(CommandError::invalid_input)
}

async fn path_of(state: &State<'_, AppState>, meeting_id: i64, id: i64) -> Result<PathBuf, CommandError> {
    state
        .store()?
        .call(move |s| snapshot_path(s, meeting_id, id))
        .await?
        .map_err(CommandError::invalid_input)
}

#[tauri::command]
pub async fn meeting_snapshots(state: State<'_, AppState>, meeting_id: i64) -> Result<SnapshotsView, CommandError> {
    state
        .store()?
        .call(move |s| snapshots_view(s, meeting_id))
        .await?
        .map_err(CommandError::store)
}

#[tauri::command]
pub async fn meeting_snapshot_thumbnail(state: State<'_, AppState>, meeting_id: i64, id: i64) -> Result<Thumbnail, CommandError> {
    let path = path_of(&state, meeting_id, id).await?;
    blocking(move || thumbnail(&path, THUMBNAIL_PX)).await
}

#[tauri::command]
pub async fn meeting_snapshot_delete(state: State<'_, AppState>, meeting_id: i64, id: i64) -> Result<(), CommandError> {
    state
        .store()?
        .call(move |s| delete_snapshot(s, meeting_id, id))
        .await?
        .map_err(CommandError::invalid_input)
}

#[tauri::command]
pub async fn meeting_set_keep_snapshots(state: State<'_, AppState>, meeting_id: i64, keep: bool) -> Result<(), CommandError> {
    state
        .store()?
        .call(move |s| s.set_keep_snapshots(meeting_id, keep).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)
}

/// Opens the slide in the default image viewer.
#[tauri::command]
pub async fn meeting_snapshot_open(state: State<'_, AppState>, meeting_id: i64, id: i64) -> Result<(), CommandError> {
    let path = path_of(&state, meeting_id, id).await?;
    blocking(move || shell_open(&path.display().to_string(), None)).await
}

/// Copies the full-size slide to the clipboard.
#[tauri::command]
pub async fn meeting_snapshot_copy(app: AppHandle, state: State<'_, AppState>, meeting_id: i64, id: i64) -> Result<(), CommandError> {
    let path = path_of(&state, meeting_id, id).await?;
    let owner = main_hwnd(&app).map(|h| h.0 as isize);
    blocking(move || {
        let image = image::open(&path).map_err(|e| format!("couldn't read the slide: {e}"))?;
        set_clipboard_dib(owner.map(|h| windows::Win32::Foundation::HWND(h as _)), &dib(&image.to_rgb8()))
    })
    .await
}

/// What "Show in Explorer" opens.
enum Shown {
    /// Explorer with this file selected.
    File(PathBuf),
    /// This folder.
    Folder(PathBuf),
}

/// Explorer with the slide selected, or with no `id`, the meeting's folder.
#[tauri::command]
pub async fn meeting_snapshots_show(state: State<'_, AppState>, meeting_id: i64, id: Option<i64>) -> Result<(), CommandError> {
    let shown = state
        .store()?
        .call(move |s| {
            if let Some(id) = id {
                return snapshot_path(s, meeting_id, id).map(Shown::File);
            }
            let dir = s.meeting_snapshots_info(meeting_id).map_err(|e| e.to_string())?.and_then(|(dir, _)| dir);
            if let Some(dir) = dir.map(PathBuf::from).filter(|d| d.is_dir()) {
                return Ok(Shown::Folder(dir));
            }
            // Belt and braces: a row is only written after its folder is recorded.
            s.list_meeting_snapshots(meeting_id)
                .map_err(|e| e.to_string())?
                .first()
                .map(|snap| Shown::File(PathBuf::from(&snap.file_path)))
                .ok_or_else(|| "This meeting has no saved slides.".to_string())
        })
        .await?
        .map_err(CommandError::invalid_input)?;
    blocking(move || match shown {
        Shown::File(file) => show_in_explorer(&file),
        Shown::Folder(folder) => shell_open(&folder.display().to_string(), None),
    })
    .await
}

#[tauri::command]
pub fn meeting_snapshot_settings(state: State<'_, AppState>) -> SnapshotSettings {
    read_settings(&state.settings.lock().unwrap(), &fallback_root(&state))
}

#[tauri::command]
pub fn set_meeting_snapshots_enabled(state: State<'_, AppState>, enabled: bool) -> Result<SnapshotSettings, CommandError> {
    save(&state, |file| file.set_meeting_snapshots_enabled(enabled))
}

/// The folder picker, then saves the choice. Applies to meetings recorded from now on;
/// existing meetings keep their folders.
#[tauri::command]
pub async fn choose_meeting_snapshots_folder(app: AppHandle, state: State<'_, AppState>) -> Result<SnapshotSettings, CommandError> {
    let start = PathBuf::from(meeting_snapshot_settings(state.clone()).folder);
    let owner = main_hwnd(&app).map(|h| h.0 as isize);
    let picked = blocking(move || pick_folder(owner.map(|h| windows::Win32::Foundation::HWND(h as _)), start)).await?;
    match picked {
        Some(folder) => save(&state, |file| file.set_meeting_snapshots_root(Some(&folder))),
        None => Ok(meeting_snapshot_settings(state)),
    }
}

#[tauri::command]
pub fn reset_meeting_snapshots_folder(state: State<'_, AppState>) -> Result<SnapshotSettings, CommandError> {
    save(&state, |file| file.set_meeting_snapshots_root(None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::{MeetingStatus, MeetingTrigger, NewMeeting, NewMeetingSnapshot};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("whimbrel-snapcmd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn meeting(store: &Store) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "this-pc".into(),
                start_unix_ms: 1_000,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap()
    }

    fn slide(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.join(name);
        image::RgbImage::from_fn(width, height, |x, _| image::Rgb([(x % 256) as u8, 40, 200])).save(&path).unwrap();
        path
    }

    fn snapshot(store: &Store, meeting_id: i64, path: &Path, at: i64, text: &str) -> i64 {
        store
            .insert_meeting_snapshot(&NewMeetingSnapshot {
                meeting_id,
                captured_unix_ms: at,
                file_path: path.display().to_string(),
                width: 64,
                height: 36,
                ocr_text: text.into(),
            })
            .unwrap()
            .unwrap()
    }

    #[test]
    fn the_strip_lists_slides_oldest_first_and_drops_missing_files() {
        let dir = temp("list");
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store);
        let second = snapshot(&store, id, &slide(&dir, "10.00.05.png", 64, 36), 2_000, "Q3 roadmap");
        let first = snapshot(&store, id, &slide(&dir, "10.00.00.png", 64, 36), 1_000, "Agenda");
        snapshot(&store, id, &dir.join("gone.png"), 3_000, "deleted in Explorer");

        let view = snapshots_view(&store, id).unwrap();
        assert_eq!(view.snapshots.iter().map(|s| s.id).collect::<Vec<_>>(), vec![first, second]);
        assert_eq!(view.snapshots[0].file_name, "10.00.00.png");
        assert_eq!(view.snapshots[0].text, "Agenda");
        assert!(view.snapshots[0].modified_unix_ms > 0);
        assert!(!view.keep);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keep_round_trips() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store);
        store.set_keep_snapshots(id, true).unwrap();
        assert!(snapshots_view(&store, id).unwrap().keep);
        store.set_keep_snapshots(id, false).unwrap();
        assert!(!snapshots_view(&store, id).unwrap().keep);
    }

    #[test]
    fn a_slide_text_tooltip_is_one_line_and_capped() {
        assert_eq!(snippet("Q3\n  roadmap\tdraft"), "Q3 roadmap draft");
        let long = "é".repeat(TEXT_CHARS + 50);
        let capped = snippet(&long);
        assert_eq!(capped.chars().count(), TEXT_CHARS + 1);
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn delete_removes_the_row_and_the_file() {
        let dir = temp("delete");
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store);
        let path = slide(&dir, "10.00.00.png", 64, 36);
        let snap = snapshot(&store, id, &path, 1_000, "Agenda");

        delete_snapshot(&store, id, snap).unwrap();
        assert!(!path.exists());
        assert!(store.list_meeting_snapshots(id).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_snapshot_is_only_reachable_through_its_own_meeting() {
        let dir = temp("owner");
        let store = Store::open_in_memory().unwrap();
        let (mine, other) = (meeting(&store), meeting(&store));
        let path = slide(&dir, "10.00.00.png", 64, 36);
        let snap = snapshot(&store, mine, &path, 1_000, "Agenda");

        assert!(snapshot_path(&store, other, snap).is_err());
        assert!(delete_snapshot(&store, other, snap).is_err());
        assert!(path.exists(), "a wrong meeting id deletes nothing");
        assert_eq!(snapshot_path(&store, mine, snap).unwrap(), path);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_thumbnail_is_a_png_data_url_that_fits() {
        let dir = temp("thumb");
        let wide = thumbnail(&slide(&dir, "wide.png", 1920, 1080), THUMBNAIL_PX).unwrap();
        assert!(wide.data_url.starts_with("data:image/png;base64,"));
        assert_eq!((wide.width, wide.height), (480, 270));
        let small = thumbnail(&slide(&dir, "small.png", 200, 100), THUMBNAIL_PX).unwrap();
        assert_eq!((small.width, small.height), (200, 100), "never scaled up");
        assert!(thumbnail(&dir.join("gone.png"), THUMBNAIL_PX).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_clipboard_bitmap_is_bottom_up_bgrx() {
        let mut rgb = image::RgbImage::new(2, 2);
        rgb.put_pixel(0, 0, image::Rgb([1, 2, 3])); // top left
        rgb.put_pixel(1, 1, image::Rgb([7, 8, 9])); // bottom right
        let bytes = dib(&rgb);
        assert_eq!(bytes.len(), 40 + 2 * 2 * 4);
        assert_eq!(&bytes[0..4], &40u32.to_le_bytes());
        assert_eq!(&bytes[4..12], &[2, 0, 0, 0, 2, 0, 0, 0], "width, then a positive height");
        assert_eq!(&bytes[14..16], &32u16.to_le_bytes());
        let pixels = &bytes[40..];
        assert_eq!(&pixels[4..8], &[9, 8, 7, 255], "the first row stored is the bottom one");
        assert_eq!(&pixels[8..12], &[3, 2, 1, 255], "then the top one");
    }

    #[test]
    fn the_settings_show_the_chosen_folder_or_the_default() {
        let dir = temp("settings");
        let mut file = crate::settings::SettingsFile::load(&dir.join("settings.json"));
        let fallback = dir.join("fallback");
        let shown = read_settings(&file, &fallback);
        assert!(shown.enabled && shown.is_default);
        assert!(shown.folder.ends_with(storage::ROOT_NAME));

        file.set_meeting_snapshots_root(Some(Path::new(r"D:\Slides")));
        file.set_meeting_snapshots_enabled(false);
        let shown = read_settings(&file, &fallback);
        assert_eq!((shown.enabled, shown.is_default, shown.folder.as_str()), (false, false, r"D:\Slides"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
