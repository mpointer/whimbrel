//! Retention sweep and recording-file cleanup (M3.0 part 2, Task 2), moved from
//! apps/macos `RetentionSettings.applyNow`, `RecordingFileOrphanScanner` and
//! `MeetingFileCleanup`. File deletion is I/O, not network, so it may run while the
//! shell holds its store queue (ADR 0001 only forbids network there). `recordings_dir`
//! is the shell's recordings folder (macOS: `SpikePaths.root`); nothing here computes a
//! platform path.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use memory::{RetentionCounts, RetentionPolicy, RetentionResult, Store};

/// The recording extensions: `.caf` is what the macOS shell writes, `.wav` the Windows
/// shell (16 kHz mono PCM straight from WASAPI, so no conversion step), `.m4a` a
/// recording compressed by the storage-retention pass.
const RECORDING_EXTENSIONS: [&str; 3] = ["caf", "wav", "m4a"];

/// Result of [`scan_orphans`]: full paths of recording files in the recordings folder that
/// no `meeting.audio_path` claims (directly or via the mic/system sibling rule), and
/// their summed on-disk size.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OrphanScan {
    pub orphaned: Vec<String>,
    pub total_bytes: i64,
}

/// Runs the per-data-class retention sweep, then deletes the audio/transcript files of
/// every meeting row it removed (best-effort: a missing file is not an error).
pub fn apply(
    store: &Store,
    policy: &RetentionPolicy,
    now_unix_ms: i64,
    recordings_dir: &Path,
) -> rusqlite::Result<RetentionResult> {
    let result = store.apply_retention(policy, now_unix_ms)?;
    for deleted in &result.deleted_meeting_files {
        delete_all_meeting_files(deleted, recordings_dir);
    }
    Ok(result)
}

/// What one [`apply`] plus orphan sweep would remove right now, counted without removing
/// anything — for a shell that shows the user the damage before doing it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetentionPreview {
    pub counts: RetentionCounts,
    /// Recordings no meeting claims, excluding whatever the caller is writing now.
    pub orphan_files: usize,
    pub orphan_bytes: i64,
}

/// The read-only twin of [`apply`]: the same predicates, the same orphan scan, no
/// deletions. `excluded` is what [`scan_orphans`] takes — the files a live recording is
/// writing — so the orphan count matches what a sweep at this moment would remove.
pub fn preview(
    store: &Store,
    policy: &RetentionPolicy,
    now_unix_ms: i64,
    recordings_dir: &Path,
    excluded: &[String],
) -> rusqlite::Result<RetentionPreview> {
    let counts = store.retention_preview(policy, now_unix_ms)?;
    let orphans = scan_orphans(store, recordings_dir, excluded)?;
    Ok(RetentionPreview {
        counts,
        orphan_files: orphans.orphaned.len(),
        orphan_bytes: orphans.total_bytes,
    })
}

/// Best-effort deletion of a meeting's on-disk files. `audio_path`'s filename is either
/// `system-audio-<stamp>.<ext>` or `mic-audio-<stamp>.<ext>`; both siblings are deleted
/// from `audio_path`'s own folder (a moved recording lives outside the recordings
/// folder) since a completed session has both and the row only stores one. An
/// unrecognized filename is deleted at its own path. A missing file is never an error:
/// it may already be gone, or never existed for a mic-only session.
/// `_recordings_dir` is unused; it stays so the Windows shell's call keeps compiling.
pub fn delete_meeting_files(
    audio_path: Option<&str>,
    transcript_path: Option<&str>,
    snapshots_dir: Option<&str>,
    _recordings_dir: &Path,
) {
    if let Some(audio_path) = audio_path {
        match sibling_audio_paths(audio_path) {
            Some((system, mic)) => {
                let _ = std::fs::remove_file(system);
                let _ = std::fs::remove_file(mic);
            }
            None => {
                let _ = std::fs::remove_file(audio_path);
            }
        }
    }
    if let Some(transcript_path) = transcript_path {
        let _ = std::fs::remove_file(transcript_path);
    }
    if let Some(dir) = snapshots_dir {
        delete_snapshots_dir(Path::new(dir));
    }
}

/// [`delete_meeting_files`] for every part of a deleted meeting.
pub fn delete_all_meeting_files(deleted: &memory::DeletedMeetingFiles, recordings_dir: &Path) {
    for a in &deleted.audio_paths {
        delete_meeting_files(Some(a), None, None, recordings_dir);
    }
    for t in &deleted.transcript_paths {
        delete_meeting_files(None, Some(t), None, recordings_dir);
    }
    delete_meeting_files(None, None, deleted.snapshots_dir.as_deref(), recordings_dir);
}

/// Marker file Whimbrel drops in every snapshot folder it creates; proves the folder is ours.
pub const SNAPSHOT_MARKER: &str = ".kestrel-snapshots";

/// A snapshot name Whimbrel writes: `HH.mm.ss` (or `HH.mm.ss-N` on a same-second
/// collision) followed by `.png`, `.heic`, or `.thumb.jpg` (a shrunk image's
/// thumbnail). Any other file is the user's.
fn is_snapshot_file(name: &str) -> bool {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            regex::Regex::new(r"^[0-9]{2}\.[0-9]{2}\.[0-9]{2}(-[0-9]+)?(\.png|\.heic|\.thumb\.jpg)$").unwrap()
        })
        .is_match(name)
}

/// Removes a meeting's snapshot folder, but only if it holds the [`SNAPSHOT_MARKER`] and
/// nothing else except Whimbrel-named snapshot files ([`is_snapshot_file`]) and `.DS_Store`
/// (the folder is user-visible, so anything foreign means the user put it there). Otherwise removes nothing and logs
/// one warning. Returns true if the folder was removed.
pub fn delete_snapshots_dir(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false; // already gone
    };
    let mut files = Vec::new();
    let mut marked = false;
    let mut foreign = false;
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        if name == SNAPSHOT_MARKER && is_file {
            marked = true;
        } else if is_file && (name == ".DS_Store" || is_snapshot_file(&name)) {
            files.push(entry.path());
        } else {
            foreign = true;
        }
    }
    if !marked || foreign {
        eprintln!("[retention] not deleting snapshot folder {}: missing marker or foreign contents", dir.display());
        return false;
    }
    for f in files {
        let _ = std::fs::remove_file(f);
    }
    let _ = std::fs::remove_file(dir.join(SNAPSHOT_MARKER));
    std::fs::remove_dir(dir).is_ok()
}

/// Recovers `<stamp>` from `audio_path`'s filename and returns the
/// `(system-audio, mic-audio)` sibling paths in `audio_path`'s own folder, or `None` if
/// the filename matches neither prefix. The siblings keep `audio_path`'s own extension:
/// both files of a recording are written, moved and compressed together. Shared by [`delete_meeting_files`] and
/// [`orphaned_filenames`] so the two can never disagree about what counts as "the same
/// recording."
pub fn sibling_audio_paths(audio_path: &str) -> Option<(PathBuf, PathBuf)> {
    let path = Path::new(audio_path);
    let dir = path.parent().unwrap_or(Path::new(""));
    let stem = path.file_stem()?.to_str()?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("caf");
    let stamp = stem
        .strip_prefix("system-audio-")
        .or_else(|| stem.strip_prefix("mic-audio-"))?;
    Some((
        dir.join(format!("system-audio-{stamp}.{ext}")),
        dir.join(format!("mic-audio-{stamp}.{ext}")),
    ))
}

/// Pure: which of the bare filenames in `on_disk` are claimed by no path in
/// `referenced_paths` (every `meeting.audio_path`, regardless of that meeting's own
/// retention window). A file counts as referenced if its own filename or its
/// mic/system sibling's filename is the last path component of any referenced path.
/// Only `system-audio-<stamp>` and `mic-audio-<stamp>` names are ever orphans: any other
/// name is a file the user put there.
pub fn orphaned_filenames(on_disk: &[String], referenced_paths: &[String]) -> Vec<String> {
    let referenced: std::collections::HashSet<String> = referenced_paths
        .iter()
        .filter_map(|p| Path::new(p).file_name())
        .filter_map(|n| n.to_str())
        .map(str::to_string)
        .collect();
    on_disk
        .iter()
        .filter(|filename| {
            if referenced.contains(filename.as_str()) {
                return false;
            }
            match sibling_audio_paths(filename) {
                Some((system, mic)) => {
                    !referenced.contains(file_name(&system)) && !referenced.contains(file_name(&mic))
                }
                // Not a Whimbrel recording name: the user's file, never an orphan.
                None => false,
            }
        })
        .cloned()
        .collect()
}

/// A recording modified within this window is never an orphan: it may be a live or just-
/// stranded recording the DB doesn't reference yet, and deleting it is unrecoverable.
pub const ORPHAN_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

fn file_name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// Lists the recording files (`.caf`, `.wav`, `.m4a`) in `recordings_dir`, computes the orphans against every
/// referenced `meeting.audio_path` in `store`, drops anything in `excluded` (the files
/// a live recording is writing right now, snapshotted by the shell before calling),
/// and sums the remaining files' sizes. An unreadable directory yields an empty scan.
/// No age guard (the Windows shell's contract); see [`scan_orphans_with_min_age`].
pub fn scan_orphans(
    store: &Store,
    recordings_dir: &Path,
    excluded: &[String],
) -> rusqlite::Result<OrphanScan> {
    scan_orphans_with_min_age(store, recordings_dir, excluded, std::time::Duration::ZERO)
}

/// [`scan_orphans`] that also keeps any file modified within `min_age` of now (a live or
/// just-stranded recording the DB doesn't reference yet). A file whose mtime can't be
/// read, or is in the future, is kept. `Duration::ZERO` disables the guard entirely
/// (an unreadable file is then listed with 0 bytes, as plain `scan_orphans` always did).
pub fn scan_orphans_with_min_age(
    store: &Store,
    recordings_dir: &Path,
    excluded: &[String],
    min_age: std::time::Duration,
) -> rusqlite::Result<OrphanScan> {
    scan_orphans_in(store, &[recordings_dir], excluded, min_age)
}

/// [`scan_orphans_with_min_age`] over several folders (the recordings folder and the
/// media folder's `Audio`), in order. A folder that can't be read (an unplugged drive)
/// is skipped; the others are still scanned. Names starting with `.` (a compression
/// pass's hidden temp files) are never listed. Each orphan is joined with the folder it
/// was found in. Every folder is matched against the same referenced paths, by filename.
pub fn scan_orphans_in(
    store: &Store,
    dirs: &[&Path],
    excluded: &[String],
    min_age: std::time::Duration,
) -> rusqlite::Result<OrphanScan> {
    let referenced = store.all_referenced_meeting_audio_paths()?;
    let excluded: Vec<PathBuf> = excluded.iter().map(PathBuf::from).collect();
    let mut total_bytes = 0i64;
    let mut orphaned = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let on_disk: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| !n.starts_with('.'))
            .filter(|n| {
                Path::new(n)
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| RECORDING_EXTENSIONS.contains(&e))
            })
            .collect();
        for name in orphaned_filenames(&on_disk, &referenced) {
            let path = dir.join(&name);
            if excluded.contains(&path) {
                continue;
            }
            let meta = std::fs::metadata(&path).ok();
            if !min_age.is_zero() {
                let old_enough = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|m| m.elapsed().ok())
                    .is_some_and(|age| age >= min_age);
                if !old_enough {
                    continue;
                }
            }
            // Unreadable metadata without the guard: listed with 0 bytes, as before it.
            total_bytes += meta.map_or(0, |m| m.len() as i64);
            orphaned.push(path.to_string_lossy().into_owned());
        }
    }
    Ok(OrphanScan { orphaned, total_bytes })
}

/// The permanent deletion pass over a [`scan_orphans`] result; the shell confirms with
/// the user first where it wants to. Returns how many files were actually removed.
pub fn delete_orphans(paths: &[String]) -> usize {
    paths
        .iter()
        .filter(|p| std::fs::remove_file(p).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    // ---- orphaned_filenames (ported from RecordingFileOrphanScannerTests.swift) ----

    #[test]
    fn unreferenced_file_with_no_sibling_in_either_list_is_orphaned() {
        assert_eq!(orphaned_filenames(&s(&["system-audio-A.caf"]), &[]), s(&["system-audio-A.caf"]));
    }

    #[test]
    fn directly_referenced_file_is_not_orphaned() {
        assert_eq!(
            orphaned_filenames(&s(&["system-audio-A.caf"]), &s(&["/spike/system-audio-A.caf"])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn referenced_system_audio_protects_its_mic_sibling() {
        assert_eq!(
            orphaned_filenames(&s(&["mic-audio-B.caf"]), &s(&["/spike/system-audio-B.caf"])),
            Vec::<String>::new(),
            "the DB only ever stores one of the two sibling paths"
        );
    }

    #[test]
    fn referenced_mic_audio_protects_its_system_sibling() {
        assert_eq!(
            orphaned_filenames(&s(&["system-audio-C.caf"]), &s(&["/spike/mic-audio-C.caf"])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn mixed_backlog_returns_only_the_genuinely_unreferenced_files() {
        let orphans = orphaned_filenames(
            &s(&["system-audio-A.caf", "mic-audio-A.caf", "system-audio-ORPHAN.caf", "mic-audio-ORPHAN2.caf"]),
            &s(&["/spike/system-audio-A.caf"]),
        );
        let set: std::collections::HashSet<String> = orphans.into_iter().collect();
        assert_eq!(set, s(&["system-audio-ORPHAN.caf", "mic-audio-ORPHAN2.caf"]).into_iter().collect());
    }

    #[test]
    fn unrecognized_filename_is_never_an_orphan() {
        assert!(orphaned_filenames(&s(&["some-other-file.caf"]), &[]).is_empty());
    }

    // ---- delete_meeting_files / sibling_audio_paths (ported from MeetingFileCleanupTests.swift) ----

    #[test]
    fn delete_files_removes_both_sibling_audio_files_and_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("system-audio-2026-08-01T00-00-00.000Z.caf");
        let mic = dir.path().join("mic-audio-2026-08-01T00-00-00.000Z.caf");
        let transcript = dir.path().join("transcript-2026-08-01T00-00-00.000Z.txt");
        std::fs::write(&system, "system").unwrap();
        std::fs::write(&mic, "mic").unwrap();
        std::fs::write(&transcript, "transcript").unwrap();

        delete_meeting_files(system.to_str(), transcript.to_str(), None, dir.path());

        assert!(!system.exists());
        assert!(!mic.exists());
        assert!(!transcript.exists());
    }

    #[test]
    fn delete_files_tolerates_already_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("system-audio-2026-08-01T01-00-00.000Z.caf");
        delete_meeting_files(missing.to_str(), None, None, dir.path());
    }

    #[test]
    fn delete_files_handles_no_audio_path() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("transcript-2026-08-01T02-00-00.000Z.txt");
        std::fs::write(&transcript, "transcript").unwrap();
        delete_meeting_files(None, transcript.to_str(), None, dir.path());
        assert!(!transcript.exists());
    }

    #[test]
    fn delete_files_removes_an_unrecognized_filename_at_its_own_path() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("elsewhere").join("not-a-recording.caf");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, "x").unwrap();
        delete_meeting_files(other.to_str(), None, None, dir.path());
        assert!(!other.exists());
    }

    #[test]
    fn sibling_audio_paths_from_system_audio_path() {
        let dir = Path::new("/some/dir");
        let (system, mic) = sibling_audio_paths("/some/dir/system-audio-STAMP.caf").unwrap();
        assert_eq!(system, dir.join("system-audio-STAMP.caf"));
        assert_eq!(mic, dir.join("mic-audio-STAMP.caf"));
    }

    #[test]
    fn sibling_audio_paths_from_mic_audio_path() {
        let dir = Path::new("/some/dir");
        let (system, mic) = sibling_audio_paths("/some/dir/mic-audio-STAMP.caf").unwrap();
        assert_eq!(system, dir.join("system-audio-STAMP.caf"));
        assert_eq!(mic, dir.join("mic-audio-STAMP.caf"));
    }

    #[test]
    fn sibling_audio_paths_keep_a_wav_extension() {
        let dir = Path::new("/some/dir");
        let (system, mic) = sibling_audio_paths("/some/dir/mic-audio-STAMP.wav").unwrap();
        assert_eq!(system, dir.join("system-audio-STAMP.wav"));
        assert_eq!(mic, dir.join("mic-audio-STAMP.wav"));
    }

    #[test]
    fn sibling_audio_paths_follow_a_moved_m4a() {
        let (sys, mic) = sibling_audio_paths("/Volumes/Scratch/Whimbrel/Audio/mic-audio-S.m4a").unwrap();
        assert_eq!(sys, Path::new("/Volumes/Scratch/Whimbrel/Audio/system-audio-S.m4a"));
        assert_eq!(mic, Path::new("/Volumes/Scratch/Whimbrel/Audio/mic-audio-S.m4a"));
    }

    #[test]
    fn delete_files_removes_a_moved_compressed_pair() {
        let media = tempfile::tempdir().unwrap();
        let unrelated = tempfile::tempdir().unwrap();
        let audio = media.path().join("media").join("Audio");
        std::fs::create_dir_all(&audio).unwrap();
        let system = audio.join("system-audio-2026-08-01T00-00-00.000Z.m4a");
        let mic = audio.join("mic-audio-2026-08-01T00-00-00.000Z.m4a");
        std::fs::write(&system, "s").unwrap();
        std::fs::write(&mic, "m").unwrap();

        delete_meeting_files(system.to_str(), None, None, unrelated.path());

        assert!(!system.exists());
        assert!(!mic.exists());
    }

    #[test]
    fn referenced_wav_protects_its_wav_sibling_but_not_a_caf_of_the_same_stamp() {
        assert_eq!(
            orphaned_filenames(
                &s(&["mic-audio-W.wav", "mic-audio-W.caf"]),
                &s(&["/whimbrel/recordings/system-audio-W.wav"]),
            ),
            s(&["mic-audio-W.caf"])
        );
    }

    #[test]
    fn delete_files_removes_both_wav_siblings_and_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("system-audio-2026-09-17T09-00-00.000Z.wav");
        let mic = dir.path().join("mic-audio-2026-09-17T09-00-00.000Z.wav");
        let transcript = dir.path().join("transcript-2026-09-17T09-00-00.000Z.txt");
        for p in [&system, &mic, &transcript] {
            std::fs::write(p, "x").unwrap();
        }

        delete_meeting_files(mic.to_str(), transcript.to_str(), None, dir.path());

        assert!(!system.exists());
        assert!(!mic.exists());
        assert!(!transcript.exists());
    }

    #[test]
    fn sibling_audio_paths_returns_none_for_unrecognized_filename() {
        assert!(sibling_audio_paths("/some/dir/not-a-recording.caf").is_none());
    }

    // ---- scan_orphans / delete_orphans / apply (against a real store and temp dir) ----

    fn insert_meeting(store: &Store, start_unix_ms: i64, audio_path: Option<&str>) -> i64 {
        store
            .insert_meeting(&memory::NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms,
                end_unix_ms: None,
                title: "M".to_string(),
                attendees: Vec::new(),
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: audio_path.map(str::to_string),
                status: memory::MeetingStatus::Completed,
            })
            .unwrap()
    }

    /// Sets a file's mtime 8 days back, past the orphan age guard.
    fn backdate(path: &Path) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86_400))
            .unwrap();
    }

    #[test]
    fn scan_orphans_skips_recently_modified_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        std::fs::create_dir(&rec).unwrap();
        std::fs::write(rec.join("system-audio-FRESH.caf"), "a").unwrap();
        std::fs::write(rec.join("system-audio-OLD.caf"), "b").unwrap();
        backdate(&rec.join("system-audio-OLD.caf"));

        let scan = scan_orphans_with_min_age(&store, &rec, &[], ORPHAN_MIN_AGE).unwrap();
        assert_eq!(scan.orphaned, vec![rec.join("system-audio-OLD.caf").to_string_lossy().into_owned()]);
        assert_eq!(scan.total_bytes, 1);
    }

    /// A dangling symlink: listed by read_dir, but `metadata` (which follows it) fails.
    /// The plain scan lists it with 0 bytes, as before the age guard (Windows' contract);
    /// the guarded scan keeps it, since its age is unknown.
    #[cfg(unix)]
    #[test]
    fn unreadable_metadata_listed_without_guard_kept_with_guard() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        std::fs::create_dir(&rec).unwrap();
        let link = rec.join("system-audio-GONE.caf");
        std::os::unix::fs::symlink(dir.path().join("missing"), &link).unwrap();

        let scan = scan_orphans(&store, &rec, &[]).unwrap();
        assert_eq!(scan.orphaned, vec![link.to_string_lossy().into_owned()]);
        assert_eq!(scan.total_bytes, 0);
        let guarded = scan_orphans_with_min_age(&store, &rec, &[], ORPHAN_MIN_AGE).unwrap();
        assert!(guarded.orphaned.is_empty());
    }

    #[test]
    fn plain_scan_orphans_has_no_age_guard() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        std::fs::create_dir(&rec).unwrap();
        std::fs::write(rec.join("system-audio-FRESH.caf"), "a").unwrap();
        let scan = scan_orphans(&store, &rec, &[]).unwrap();
        assert_eq!(scan.orphaned, vec![rec.join("system-audio-FRESH.caf").to_string_lossy().into_owned()]);
    }

    #[test]
    fn scan_orphans_skips_referenced_siblings_and_live_files_and_sums_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        std::fs::create_dir(&rec).unwrap();
        for (name, body) in [
            ("system-audio-A.caf", "aaaa"),
            ("mic-audio-A.caf", "bb"),
            ("system-audio-ORPHAN.caf", "ccccc"),
            ("mic-audio-LIVE.caf", "d"),
            ("notes.txt", "eee"),
        ] {
            std::fs::write(rec.join(name), body).unwrap();
            backdate(&rec.join(name));
        }
        insert_meeting(&store, 1, Some(rec.join("system-audio-A.caf").to_str().unwrap()));
        let live = rec.join("mic-audio-LIVE.caf").to_string_lossy().into_owned();

        let scan = scan_orphans(&store, &rec, &[live]).unwrap();
        assert_eq!(scan.orphaned, vec![rec.join("system-audio-ORPHAN.caf").to_string_lossy().into_owned()]);
        assert_eq!(scan.total_bytes, 5);

        assert_eq!(delete_orphans(&scan.orphaned), 1);
        assert!(!rec.join("system-audio-ORPHAN.caf").exists());
        assert!(rec.join("system-audio-A.caf").exists());
        assert!(rec.join("mic-audio-LIVE.caf").exists());
        assert_eq!(delete_orphans(&scan.orphaned), 0, "already gone");
    }

    #[test]
    fn scan_orphans_finds_wav_orphans_and_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        std::fs::create_dir(&rec).unwrap();
        for name in ["system-audio-K.wav", "mic-audio-K.wav", "mic-audio-ORPHAN.wav", "transcript-K.txt"] {
            std::fs::write(rec.join(name), "xy").unwrap();
            backdate(&rec.join(name));
        }
        insert_meeting(&store, 1, Some(rec.join("system-audio-K.wav").to_str().unwrap()));

        let scan = scan_orphans(&store, &rec, &[]).unwrap();
        assert_eq!(scan.orphaned, vec![rec.join("mic-audio-ORPHAN.wav").to_string_lossy().into_owned()]);
        assert_eq!(scan.total_bytes, 2);
    }

    #[test]
    fn scan_orphans_in_keeps_moved_and_compressed_pairs_and_user_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        let media_audio = dir.path().join("media").join("Audio");
        std::fs::create_dir_all(&rec).unwrap();
        std::fs::create_dir_all(&media_audio).unwrap();
        let files = [
            media_audio.join("system-audio-A.m4a"),
            media_audio.join("mic-audio-A.m4a"),
            media_audio.join("interview.m4a"),
            media_audio.join(".system-audio-B.m4a"),
            rec.join("system-audio-A.caf"),
            rec.join("mic-audio-ORPHAN.caf"),
        ];
        for f in &files {
            std::fs::write(f, "x").unwrap();
            backdate(f);
        }
        insert_meeting(&store, 1, Some(media_audio.join("system-audio-A.m4a").to_str().unwrap()));

        let scan = scan_orphans_in(&store, &[&rec, &media_audio], &[], ORPHAN_MIN_AGE).unwrap();
        let got: std::collections::HashSet<String> = scan.orphaned.into_iter().collect();
        let want: std::collections::HashSet<String> = [rec.join("system-audio-A.caf"), rec.join("mic-audio-ORPHAN.caf")]
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(got, want);
        assert_eq!(scan.total_bytes, 2);
    }

    #[test]
    fn scan_orphans_in_skips_a_missing_dir_and_scans_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        std::fs::create_dir(&rec).unwrap();
        std::fs::write(rec.join("mic-audio-ORPHAN.caf"), "x").unwrap();
        let missing = dir.path().join("unplugged").join("Audio");

        let scan = scan_orphans_in(&store, &[&missing, &rec], &[], std::time::Duration::ZERO).unwrap();
        assert_eq!(scan.orphaned, vec![rec.join("mic-audio-ORPHAN.caf").to_string_lossy().into_owned()]);
    }

    #[test]
    fn scan_orphans_on_a_missing_directory_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let scan = scan_orphans(&store, &dir.path().join("nope"), &[]).unwrap();
        assert_eq!(scan, OrphanScan { orphaned: Vec::new(), total_bytes: 0 });
    }

    #[test]
    fn preview_counts_what_apply_then_deletes_and_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        std::fs::create_dir(&rec).unwrap();
        let day = 86_400_000i64;
        let now = 100 * day;
        let old_audio = rec.join("system-audio-OLD.caf");
        let orphan = rec.join("system-audio-ORPHAN.caf");
        for p in [&old_audio, &orphan] {
            std::fs::write(p, "xx").unwrap();
            backdate(p);
        }
        insert_meeting(&store, now - 30 * day, old_audio.to_str());
        insert_meeting(&store, now - 1 * day, None);

        let policy = RetentionPolicy { capture_item_days: Some(30), meeting_days: Some(7), conversation_days: Some(30) };
        let preview = preview(&store, &policy, now, &rec, &[]).unwrap();
        assert_eq!(preview.counts.meetings, 1, "only the meeting past the window");
        assert_eq!((preview.orphan_files, preview.orphan_bytes), (1, 2));
        assert_eq!(store.recent_meetings(10).unwrap().len(), 2, "a preview deletes nothing");
        assert!(old_audio.exists() && orphan.exists());

        let result = apply(&store, &policy, now, &rec).unwrap();
        assert_eq!(result.meetings_deleted, preview.counts.meetings, "the count was the truth");
        assert_eq!(result.capture_items_deleted, preview.counts.capture_items);
        assert_eq!(result.conversations_deleted, preview.counts.conversations);
    }

    #[test]
    fn preview_counts_nothing_for_windows_kept_forever() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let day = 86_400_000i64;
        let now = 1_000 * day;
        insert_meeting(&store, 0, None);

        let policy = RetentionPolicy { capture_item_days: None, meeting_days: None, conversation_days: None };
        let preview = preview(&store, &policy, now, &dir.path().join("nope"), &[]).unwrap();
        assert_eq!(preview.counts, memory::RetentionCounts::default());
    }

    #[test]
    fn apply_deletes_expired_meeting_rows_and_their_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        std::fs::create_dir(&rec).unwrap();
        let day = 86_400_000i64;
        let now = 100 * day;
        let old_audio = rec.join("system-audio-OLD.caf");
        let old_mic = rec.join("mic-audio-OLD.caf");
        let new_audio = rec.join("system-audio-NEW.caf");
        for p in [&old_audio, &old_mic, &new_audio] {
            std::fs::write(p, "x").unwrap();
        }
        insert_meeting(&store, now - 30 * day, old_audio.to_str());
        insert_meeting(&store, now - 1 * day, new_audio.to_str());

        let policy = RetentionPolicy { capture_item_days: None, meeting_days: Some(7), conversation_days: None };
        let result = apply(&store, &policy, now, &rec).unwrap();
        assert_eq!(result.meetings_deleted, 1);
        assert_eq!(result.deleted_meeting_files.len(), 1);
        assert!(!old_audio.exists());
        assert!(!old_mic.exists(), "the sibling the row never stored is deleted too");
        assert!(new_audio.exists());
        assert_eq!(store.recent_meetings(10).unwrap().len(), 1);
    }

    // ---- snapshot folders ----

    fn snapshot_dir(with_marker: bool) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        if with_marker {
            std::fs::write(d.path().join(SNAPSHOT_MARKER), "").unwrap();
        }
        std::fs::write(d.path().join("10.14.03.png"), "x").unwrap();
        std::fs::write(d.path().join("10.14.03-2.png"), "x").unwrap();
        std::fs::write(d.path().join(".DS_Store"), "x").unwrap();
        d
    }

    #[test]
    fn delete_snapshots_dir_skips_folder_with_user_png() {
        let d = snapshot_dir(true);
        let user = "Screenshot 2026-10-22 at 10.14.03.png";
        std::fs::write(d.path().join(user), "x").unwrap();
        assert!(!delete_snapshots_dir(d.path()));
        for n in [user, "10.14.03.png", "10.14.03-2.png", ".DS_Store", SNAPSHOT_MARKER] {
            assert!(d.path().join(n).exists(), "{n} must survive");
        }
    }

    #[test]
    fn is_snapshot_file_matches_only_whimbrels_names() {
        for ok in ["00.00.00.png", "23.59.59.png", "10.14.03-2.png", "10.14.03-12.png",
                   "10.14.03.heic", "10.14.03-2.thumb.jpg"] {
            assert!(is_snapshot_file(ok), "{ok}");
        }
        for bad in ["slide-1.png", "1.png", "10.14.03.PNG", "10.14.03-.png", "10.14.3.png",
                    "10.14.03-2a.png", "x10.14.03.png", "Screenshot 2026-10-22 at 10.14.03.png", "10.14.03.png.txt",
                    "10.14.03.jpg", "10.14.03.thumb.png"] {
            assert!(!is_snapshot_file(bad), "{bad}");
        }
    }

    #[test]
    fn delete_snapshots_dir_removes_heic_and_thumbnails() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("snaps");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join(SNAPSHOT_MARKER), "").unwrap();
        std::fs::write(dir.join("10.14.03.heic"), "x").unwrap();
        std::fs::write(dir.join("10.14.03.thumb.jpg"), "x").unwrap();
        assert!(delete_snapshots_dir(&dir));
        assert!(!dir.exists());
    }

    #[test]
    fn delete_snapshots_dir_removes_marked_png_only_folder() {
        let d = snapshot_dir(true);
        assert!(delete_snapshots_dir(d.path()));
        assert!(!d.path().exists());
    }

    #[test]
    fn delete_snapshots_dir_skips_folder_without_marker() {
        let d = snapshot_dir(false);
        assert!(!delete_snapshots_dir(d.path()));
        assert!(d.path().join("10.14.03.png").exists());
    }

    #[test]
    fn delete_snapshots_dir_skips_folder_with_foreign_files() {
        let d = snapshot_dir(true);
        std::fs::write(d.path().join("notes.docx"), "x").unwrap();
        assert!(!delete_snapshots_dir(d.path()));
        for n in ["notes.docx", "10.14.03.png", ".DS_Store", SNAPSHOT_MARKER] {
            assert!(d.path().join(n).exists(), "{n} must survive");
        }
    }

    #[test]
    fn delete_snapshots_dir_skips_folder_with_subdirectory() {
        let d = snapshot_dir(true);
        std::fs::create_dir(d.path().join("sub")).unwrap();
        assert!(!delete_snapshots_dir(d.path()));
        assert!(d.path().join("10.14.03.png").exists());
    }

    #[test]
    fn apply_deletes_snapshot_dir_of_expired_unkept_meeting_and_spares_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let rec = dir.path().join("rec");
        std::fs::create_dir(&rec).unwrap();
        let day = 86_400_000i64;
        let now = 100 * day;
        let mut dirs = Vec::new();
        for (i, keep) in [false, true].into_iter().enumerate() {
            let d = rec.join(format!("snaps-{i}"));
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join(SNAPSHOT_MARKER), "").unwrap();
            std::fs::write(d.join("10.14.03.png"), "x").unwrap();
            let id = insert_meeting(&store, now - 30 * day, None);
            store.set_meeting_snapshots_dir(id, d.to_str().unwrap()).unwrap();
            store.set_keep_snapshots(id, keep).unwrap();
            dirs.push(d);
        }
        let policy = RetentionPolicy { capture_item_days: None, meeting_days: Some(7), conversation_days: None };
        apply(&store, &policy, now, &rec).unwrap();
        assert!(!dirs[0].exists());
        assert!(dirs[1].join("10.14.03.png").exists());
    }

    #[test]
    fn scan_orphans_ignores_snapshot_folders() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        let snaps = dir.path().join("snaps");
        std::fs::create_dir(&snaps).unwrap();
        std::fs::write(snaps.join(SNAPSHOT_MARKER), "").unwrap();
        std::fs::write(snaps.join("10.14.03.png"), "x").unwrap();
        assert!(scan_orphans(&store, dir.path(), &[]).unwrap().orphaned.is_empty());
    }
}
