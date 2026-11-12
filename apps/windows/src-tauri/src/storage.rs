//! Storage (plan 2026-11-12-storage-visibility, Windows half): what Whimbrel keeps on this PC,
//! compacting the database, the low-disk warning, and Free up space.
//!
//! The rules are core's (`memory::Store::{db_space, compact, captures_older_than}`,
//! `api::storage::decide`, `api::media_retention::plan`, `api::retention`); this measures the
//! folders, asks Windows how much room the disk has, and runs the one-off cleanups. Windows
//! writes captures straight into the store, so there is no capture queue file to measure or trim.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Serialize;

use crate::paths::Paths;

/// Below this much free space on the data folder's disk, Whimbrel warns (the Mac's 10 GB).
pub const LOW_DISK_BYTES: i64 = 10_000_000_000;

/// The day choices Free up space offers, and its default.
pub const FREE_UP_DAYS: [u32; 3] = [30, 90, 180];
pub const FREE_UP_DEFAULT_DAYS: u32 = 90;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// What Whimbrel keeps on this PC, by part, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Footprint {
    /// The database file with its write-ahead log.
    pub database: i64,
    /// Meeting recordings and transcripts.
    pub recordings: i64,
    /// Speech and embedding models.
    pub models: i64,
    /// Everything else in the data folder: logs, skins, snapshots, diagnostics.
    pub other: i64,
    pub total: i64,
}

/// A file's size, or 0 when it isn't there.
fn file_size(path: &Path) -> i64 {
    std::fs::symlink_metadata(path).ok().filter(|m| m.is_file()).map_or(0, |m| m.len() as i64)
}

/// A folder's size, counting regular files only and never following a link out of it.
pub fn dir_size(path: &Path) -> i64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len() as i64),
            _ => 0,
        })
        .sum()
}

/// The database, its `-wal` and `-shm`.
pub fn database_bytes(db: &Path) -> i64 {
    let with = |suffix: &str| {
        let mut name = db.as_os_str().to_owned();
        name.push(suffix);
        file_size(Path::new(&name))
    };
    file_size(db) + with("-wal") + with("-shm")
}

/// Measures every part. Reads the whole data folder, so it runs off the UI thread.
pub fn measure(paths: &Paths) -> Footprint {
    let database = database_bytes(&paths.db);
    let recordings = dir_size(&paths.recordings);
    let models = dir_size(&paths.models);
    let all = dir_size(&paths.root);
    // A part outside the data folder (none today) still counts once.
    let inside = |p: &Path, bytes: i64| if p.starts_with(&paths.root) { bytes } else { 0 };
    let other = (all - inside(&paths.db, database) - inside(&paths.recordings, recordings) - inside(&paths.models, models)).max(0);
    let total = other + database + recordings + models;
    Footprint { database, recordings, models, other, total }
}

/// Free bytes on the disk holding `path` (for this user, so quotas count), or `None` when Windows
/// can't say.
#[cfg(windows)]
pub fn volume_free_bytes(path: &Path) -> Option<i64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    // The nearest folder that exists: a data folder not made yet still has a disk.
    let mut dir = path.to_path_buf();
    while !dir.exists() {
        dir = dir.parent()?.to_path_buf();
    }
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut available = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut available), None, None) }.ok()?;
    i64::try_from(available).ok()
}

#[cfg(not(windows))]
pub fn volume_free_bytes(_path: &Path) -> Option<i64> {
    None
}

/// A disk under the warning line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LowDisk {
    /// The drive, such as `C:\`.
    pub volume: String,
    pub free_bytes: i64,
}

/// The drive a path is on (its prefix and root), for telling two folders' disks apart.
fn volume_of(path: &Path) -> String {
    path.components()
        .take_while(|c| matches!(c, std::path::Component::Prefix(_) | std::path::Component::RootDir))
        .collect::<PathBuf>()
        .to_string_lossy()
        .into_owned()
}

/// Each distinct disk among `folders` with less than `threshold` free. Pure over `free`.
pub fn low_disks(folders: &[&Path], free: impl Fn(&Path) -> Option<i64>, threshold: i64) -> Vec<LowDisk> {
    let mut out: Vec<LowDisk> = Vec::new();
    for folder in folders {
        let volume = volume_of(folder);
        if out.iter().any(|l| l.volume == volume) {
            continue;
        }
        if let Some(bytes) = free(folder).filter(|b| *b < threshold) {
            out.push(LowDisk { volume, free_bytes: bytes });
        }
    }
    out
}

/// What a compaction did, for Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompactView {
    pub ran: bool,
    /// `"ran"`, `"not_worth_it"` or `"not_enough_disk"`.
    pub reason: &'static str,
    /// For `"not_enough_disk"`: the free space the rewrite needs.
    pub need_bytes: Option<i64>,
    pub file_bytes_before: i64,
    pub file_bytes_after: i64,
    /// It ran, but another reader kept the last checkpoint from finishing, so the freed space
    /// reaches the disk a little later.
    pub space_pending: bool,
}

/// Compacts when it pays off (or when `forced`, the user asked) and the disk can hold the rewrite.
/// Holds the store for the whole `VACUUM`, so it runs on the store's own lane.
pub fn compact(s: &memory::Store, volume_free_bytes: i64, forced: bool) -> Result<CompactView, String> {
    let before = s.db_space().map_err(|e| e.to_string())?;
    let view = |ran, reason, need_bytes, after: i64, space_pending| CompactView { ran, reason, need_bytes, file_bytes_before: before.file_bytes(), file_bytes_after: after, space_pending };
    Ok(match api::storage::decide(&before, volume_free_bytes, forced) {
        api::storage::CompactDecision::Run => {
            let started = std::time::Instant::now();
            let outcome = s.compact().map_err(|e| e.to_string())?;
            let after = outcome.space.file_bytes();
            tracing::info!(before = before.file_bytes(), after, pending = outcome.space_pending, ms = started.elapsed().as_millis() as u64, forced, "database compacted");
            view(true, "ran", None, after, outcome.space_pending)
        }
        api::storage::CompactDecision::NotWorthIt => view(false, "not_worth_it", None, before.file_bytes(), false),
        api::storage::CompactDecision::NotEnoughDisk { need_bytes } => {
            tracing::warn!(need_bytes, volume_free_bytes, "not enough free disk to compact the database");
            view(false, "not_enough_disk", Some(need_bytes), before.file_bytes(), false)
        }
    })
}

/// What each Free up space row would free at `days`, for the dialog. `None` hides a row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FreeUpOptions {
    pub days: u32,
    /// Free space inside the database file.
    pub compact_bytes: Option<i64>,
    /// Audio of meetings older than `days` that can go (every part transcribed).
    pub recordings_bytes: Option<i64>,
    pub captures_count: Option<i64>,
    /// The captures' text, about what deleting them frees.
    pub captures_bytes: Option<i64>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The media plan that deletes the audio of meetings older than `days`, and nothing else.
fn delete_audio_policy(days: u32) -> api::media_retention::MediaPolicy {
    api::media_retention::MediaPolicy {
        compress_after_days: None,
        delete_audio_after_days: Some(days),
        shrink_images_after_days: None,
        delete_audio_preview_after_days: Some(days),
    }
}

/// The meetings a delete would touch: planned for `DeleteAudio`, not `busy`, every part transcribed.
fn deletable_recordings(s: &memory::Store, days: u32, recordings: &Path, busy: &[i64]) -> Result<(Vec<api::media_retention::MeetingWork>, i64), String> {
    let plan = api::media_retention::plan(s, &delete_audio_policy(days), now_ms(), recordings, busy).map_err(|e| e.to_string())?;
    let mut work = Vec::new();
    for w in plan.work {
        if !w.steps.contains(&api::media_retention::MediaStep::DeleteAudio) {
            continue;
        }
        let parts = s.meeting_parts(w.meeting_id).map_err(|e| e.to_string())?;
        // A part still waiting for its transcript keeps its audio: deleting it would lose the meeting.
        if !parts.is_empty() && parts.iter().all(|p| p.transcribed) {
            work.push(w);
        }
    }
    Ok((work, plan.usage.audio_deletable_bytes))
}

/// The dialog's numbers at `days`.
pub fn options(s: &memory::Store, days: u32, recordings: &Path, busy: &[i64]) -> Result<FreeUpOptions, String> {
    let space = s.db_space().map_err(|e| e.to_string())?;
    let (work, deletable) = deletable_recordings(s, days, recordings, busy)?;
    let (count, text) = s.captures_older_than(now_ms() - i64::from(days) * DAY_MS).map_err(|e| e.to_string())?;
    Ok(FreeUpOptions {
        days,
        compact_bytes: Some(space.free_bytes()).filter(|b| *b > 0),
        recordings_bytes: Some(deletable).filter(|b| *b > 0 && !work.is_empty()),
        captures_count: Some(count).filter(|c| *c > 0),
        captures_bytes: Some(text).filter(|_| count > 0),
    })
}

/// Deletes the audio of meetings older than `days` (transcripts, notes and summaries stay). Returns
/// how many meetings lost their audio. Only Whimbrel's own files inside the recordings folder go.
pub fn delete_old_recordings(s: &memory::Store, days: u32, recordings: &Path, busy: &[i64]) -> Result<usize, String> {
    let (work, _) = deletable_recordings(s, days, recordings, busy)?;
    for w in &work {
        for path in &w.audio_paths {
            api::retention::delete_meeting_files(Some(path), None, None, &[recordings]);
        }
    }
    tracing::info!(meetings = work.len(), days, "free up space: deleted old recordings");
    Ok(work.len())
}

/// Deletes screen captures older than `days` through retention with only the capture window set,
/// so the deletion reaches the other devices too. The retention settings don't change.
pub fn delete_old_captures(s: &memory::Store, days: u32, recordings: &Path) -> Result<usize, String> {
    let policy = memory::RetentionPolicy { capture_item_days: Some(days), meeting_days: None, conversation_days: None };
    let result = api::retention::apply(s, &policy, now_ms(), recordings).map_err(|e| e.to_string())?;
    tracing::info!(captures = result.capture_items_deleted, days, "free up space: deleted old screen captures");
    Ok(result.capture_items_deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whimbrel-storage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn measure_splits_the_parts() {
        let root = scratch("measure");
        let paths = Paths::under(&root);
        write(&paths.db, 1000);
        let mut wal = paths.db.as_os_str().to_owned();
        wal.push("-wal");
        write(Path::new(&wal), 200);
        write(&paths.recordings.join("system-audio-1.wav"), 3000);
        write(&paths.models.join("ggml.bin"), 5000);
        write(&paths.logs.join("whimbrel.log"), 40);
        let f = measure(&paths);
        assert_eq!((f.database, f.recordings, f.models, f.other), (1200, 3000, 5000, 40));
        assert_eq!(f.total, 9240);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn low_disks_reports_each_low_drive_once() {
        let c = Path::new(r"C:\Users\u\AppData\Local\Whimbrel\data");
        let c2 = Path::new(r"C:\Users\u\AppData\Local\Whimbrel\data\recordings");
        let d = Path::new(r"D:\Whimbrel\recordings");
        let free = |p: &Path| Some(if p.starts_with("C:\\") { 2_000_000_000 } else { 50_000_000_000 });
        assert_eq!(low_disks(&[c, c2, d], free, LOW_DISK_BYTES), vec![LowDisk { volume: r"C:\".into(), free_bytes: 2_000_000_000 }]);
        // Only the other drive is low.
        let free = |p: &Path| Some(if p.starts_with("D:\\") { 1 } else { 50_000_000_000 });
        assert_eq!(low_disks(&[c, d], free, LOW_DISK_BYTES).len(), 1);
        // At the line is not low, and an unknown answer never warns.
        assert!(low_disks(&[c], |_| Some(LOW_DISK_BYTES), LOW_DISK_BYTES).is_empty());
        assert!(low_disks(&[c], |_| None, LOW_DISK_BYTES).is_empty());
    }

    #[test]
    fn volume_free_bytes_answers_for_a_real_folder_and_one_not_made_yet() {
        let dir = std::env::temp_dir();
        assert!(volume_free_bytes(&dir).is_some_and(|b| b > 0));
        assert!(volume_free_bytes(&dir.join("not-made-yet").join("deeper")).is_some_and(|b| b > 0));
    }

    #[test]
    fn compact_refuses_without_disk_and_skips_when_not_worth_it() {
        let s = memory::Store::open_in_memory().unwrap();
        let none = compact(&s, 0, true).unwrap();
        assert_eq!((none.ran, none.reason), (false, "not_enough_disk"));
        assert!(none.need_bytes.is_some());
        let small = compact(&s, i64::MAX, false).unwrap();
        assert_eq!((small.ran, small.reason), (false, "not_worth_it"));
        let forced = compact(&s, i64::MAX, true).unwrap();
        assert_eq!((forced.ran, forced.reason), (true, "ran"));
    }

    #[test]
    fn options_hide_what_frees_nothing() {
        let s = memory::Store::open_in_memory().unwrap();
        let root = scratch("options");
        let o = options(&s, FREE_UP_DEFAULT_DAYS, &root, &[]).unwrap();
        assert_eq!(o, FreeUpOptions { days: 90, ..FreeUpOptions::default() });
        let _ = std::fs::remove_dir_all(&root);
    }
}
