//! Storage retention, media half: decides per meeting whether to move, compress, shrink
//! snapshots, or delete audio. Pure planning over the store and the file system; the shell
//! executes the plan (copy/verify/rewrite paths/delete) and reports back through
//! `Store::set_meeting_audio_paths` / `Store::set_meeting_snapshot_paths`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use memory::Store;
use serde::{Deserialize, Serialize};

use crate::retention::sibling_audio_paths;

const DAY_MS: i64 = 86_400_000;

/// The three ages; `None` is the "keep forever" / "never delete" choice.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaPolicy {
    pub compress_after_days: Option<u32>,
    pub delete_audio_after_days: Option<u32>,
    pub shrink_images_after_days: Option<u32>,
    /// The delete age Settings measures `MediaUsage::audio_deletable_bytes` at while
    /// deletion is off ("turning it on would free …"). Plans no step.
    #[serde(default)]
    pub delete_audio_preview_after_days: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaStep {
    MoveAudio,
    MoveSnapshots,
    Compress,
    DeleteAudio,
    ShrinkImages,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingWork {
    pub meeting_id: i64,
    /// Moves first, then `DeleteAudio` or `Compress`, then `ShrinkImages`.
    pub steps: Vec<MediaStep>,
    /// The meeting's stored audio paths (meeting row and parts) that still have a file.
    /// These are the paths as planned: after `MoveAudio`, the executor must carry its
    /// old→new mapping into `Compress` (and rewrite with `set_meeting_audio_paths`),
    /// or compression would look for the files at their pre-move paths.
    pub audio_paths: Vec<String>,
    pub shrink_snapshot_ids: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaUsage {
    pub audio_full_bytes: i64,
    pub audio_compressed_bytes: i64,
    pub image_bytes: i64,
    /// Audio bytes deletion would free now, at `delete_audio_after_days`, or at
    /// `delete_audio_preview_after_days` while deletion is off.
    pub audio_deletable_bytes: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaPlan {
    pub work: Vec<MeetingWork>,
    pub disconnected_meetings: usize,
    pub usage: MediaUsage,
}

/// The track files a stored audio path stands for: its system/mic pair, or the path alone
/// when the name carries neither track prefix.
fn track_files(stored: &str) -> Vec<PathBuf> {
    match sibling_audio_paths(stored) {
        Some((system, mic)) => vec![system, mic],
        None => vec![PathBuf::from(stored)],
    }
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// A file is unreachable when its folder is gone and so is the drive it lived on
/// (`/Volumes/<name>`, an unplugged drive). A file missing from an existing folder, or a
/// folder deleted on a mounted volume, is simply already gone.
fn reachable(path: &Path) -> bool {
    path.parent().is_some_and(Path::is_dir) || !volume_gone(path)
}

/// `path` is under `/Volumes/<name>` and that volume is not mounted. Mirrors
/// `MediaMover.volumeGone`.
fn volume_gone(path: &Path) -> bool {
    let mut c = path.components();
    match (c.next(), c.next(), c.next()) {
        (Some(std::path::Component::RootDir), Some(v), Some(name)) if v.as_os_str() == "Volumes" => {
            !Path::new("/Volumes").join(name).is_dir()
        }
        _ => false,
    }
}

/// Component-wise, on canonical paths, so `Audio2/` is outside `Audio/` and `/var` vs
/// `/private/var` never matters. `folder` is already canonical.
fn outside(file: &Path, folder: &Path) -> bool {
    std::fs::canonicalize(file).is_ok_and(|f| !f.starts_with(folder))
}

/// One pass over every completed meeting with media, oldest first. See the storage
/// retention plan (Task 4) for the rules; `busy` meetings and meetings with any
/// unreachable file still count in `usage` but get no work.
pub fn plan(
    store: &Store,
    policy: &MediaPolicy,
    now_unix_ms: i64,
    media_root: &Path,
    busy: &[i64],
) -> rusqlite::Result<MediaPlan> {
    let due = |days: Option<u32>, start: i64| {
        days.is_some_and(|d| start < now_unix_ms - i64::from(d) * DAY_MS)
    };
    let folders = std::fs::canonicalize(media_root)
        .ok()
        .filter(|r| r.is_dir())
        .map(|r| (r.join("Audio"), r.join("Snapshots")));
    let mut out = MediaPlan::default();
    let mut counted = BTreeSet::new();
    for m in store.media_meetings()? {
        let audio: Vec<(&String, Vec<PathBuf>)> =
            m.audio_paths.iter().map(|p| (p, track_files(p))).collect();
        let deletable = !m.keep_media
            && m.all_transcribed
            && due(
                policy.delete_audio_after_days.or(policy.delete_audio_preview_after_days),
                m.start_unix_ms,
            );
        for file in audio.iter().flat_map(|(_, files)| files) {
            let Ok(meta) = std::fs::metadata(file) else { continue };
            if !counted.insert(file.clone()) {
                continue;
            }
            let len = meta.len() as i64;
            if deletable {
                out.usage.audio_deletable_bytes += len;
            }
            match extension(file).as_str() {
                "caf" | "wav" => out.usage.audio_full_bytes += len,
                "m4a" => out.usage.audio_compressed_bytes += len,
                _ => {}
            }
        }
        for snap in &m.snapshots {
            let path = PathBuf::from(&snap.file_path);
            if let Ok(meta) = std::fs::metadata(&path) {
                if counted.insert(path) {
                    out.usage.image_bytes += meta.len() as i64;
                }
            }
        }

        if busy.contains(&m.id) {
            continue;
        }
        // Past its delete age, audio on a gone folder is treated as already deleted (it
        // only ever gets `DeleteAudio`, which needs the files to exist), so it can't keep
        // the meeting's snapshots stuck as "disconnected" forever.
        let audio_delete_due = due(policy.delete_audio_after_days, m.start_unix_ms);
        let all_reachable = (audio_delete_due
            || audio.iter().flat_map(|(_, files)| files).all(|f| reachable(f)))
            && m.snapshots.iter().all(|s| reachable(Path::new(&s.file_path)));
        if !all_reachable {
            out.disconnected_meetings += 1;
            continue;
        }

        let audio_files: Vec<&PathBuf> =
            audio.iter().flat_map(|(_, files)| files).filter(|f| f.exists()).collect();
        let audio_paths: Vec<String> = audio
            .iter()
            .filter(|(_, files)| files.iter().any(|f| f.exists()))
            .map(|(p, _)| (*p).clone())
            .collect();
        let snaps: Vec<_> =
            m.snapshots.iter().filter(|s| Path::new(&s.file_path).exists()).collect();

        let protected = m.keep_media;
        // ponytail: one untranscribed part holds back the whole meeting's delete (the
        // step is per meeting); per-part delete paths if that ever matters.
        let delete = !protected
            && audio_delete_due
            && m.all_transcribed
            && !audio_files.is_empty();
        let compress = !delete
            && !protected
            && due(policy.compress_after_days, m.start_unix_ms)
            && audio_files.iter().any(|f| extension(f) != "m4a");
        let shrink_snapshot_ids: Vec<i64> =
            if !protected && !m.keep_snapshots && due(policy.shrink_images_after_days, m.start_unix_ms) {
                snaps
                    .iter()
                    .filter(|s| !s.file_path.ends_with(".thumb.jpg"))
                    .map(|s| s.id)
                    .collect()
            } else {
                vec![]
            };

        let mut steps = Vec::new();
        if let Some((audio_dir, snapshots_dir)) = &folders {
            if !delete && audio_files.iter().any(|f| outside(f, audio_dir)) {
                steps.push(MediaStep::MoveAudio);
            }
            if snaps.iter().any(|s| outside(Path::new(&s.file_path), snapshots_dir)) {
                steps.push(MediaStep::MoveSnapshots);
            }
        }
        if delete {
            steps.push(MediaStep::DeleteAudio);
        } else if compress {
            steps.push(MediaStep::Compress);
        }
        if !shrink_snapshot_ids.is_empty() {
            steps.push(MediaStep::ShrinkImages);
        }
        if !steps.is_empty() {
            out.work.push(MeetingWork { meeting_id: m.id, steps, audio_paths, shrink_snapshot_ids });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::{MeetingStatus, MeetingTrigger, NewMeeting, NewMeetingSnapshot};

    const DAY: i64 = 86_400_000;
    const NOW: i64 = 1_000 * DAY;

    fn policy(c: u32, d: u32, s: u32) -> MediaPolicy {
        MediaPolicy {
            compress_after_days: Some(c),
            delete_audio_after_days: Some(d),
            shrink_images_after_days: Some(s),
            delete_audio_preview_after_days: None,
        }
    }

    fn open(dir: &Path) -> Store {
        Store::open(&dir.join("t.sqlite"), Path::new("")).unwrap()
    }

    /// A completed, transcribed meeting.
    fn meeting(store: &Store, start_unix_ms: i64, audio: Option<&Path>) -> i64 {
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms,
                end_unix_ms: None,
                title: "M".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: audio.map(|p| p.to_str().unwrap().to_string()),
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store.update_meeting_transcript_and_notes(id, "/t.txt", None).unwrap();
        id
    }

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; bytes]).unwrap();
    }

    /// Writes both tracks of a recording in `dir` and returns the stored (system) path.
    fn pair(dir: &Path, stamp: &str, ext: &str, sys: usize, mic: usize) -> PathBuf {
        let system = dir.join(format!("system-audio-{stamp}.{ext}"));
        write(&system, sys);
        write(&dir.join(format!("mic-audio-{stamp}.{ext}")), mic);
        system
    }

    fn snap(store: &Store, id: i64, path: &Path) -> i64 {
        store
            .insert_meeting_snapshot(&NewMeetingSnapshot {
                meeting_id: id,
                captured_unix_ms: 1,
                file_path: path.to_str().unwrap().into(),
                width: 10,
                height: 10,
                ocr_text: String::new(),
            })
            .unwrap()
            .unwrap()
    }

    fn s(p: &Path) -> String {
        p.to_str().unwrap().to_string()
    }

    fn steps(plan: &MediaPlan, id: i64) -> Vec<MediaStep> {
        plan.work.iter().find(|w| w.meeting_id == id).map(|w| w.steps.clone()).unwrap_or_default()
    }

    const NO_ROOT: &str = "/nonexistent/whimbrel-media";

    #[test]
    fn exactly_30_days_old_is_untouched_and_30_days_plus_1ms_compresses() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let a = meeting(&store, NOW - 30 * DAY, Some(&pair(&rec, "A", "caf", 1, 1)));
        let b_path = pair(&rec, "B", "caf", 1, 1);
        let b = meeting(&store, NOW - 30 * DAY - 1, Some(&b_path));
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert!(steps(&p, a).is_empty());
        assert_eq!(
            p.work,
            vec![MeetingWork {
                meeting_id: b,
                steps: vec![MediaStep::Compress],
                audio_paths: vec![s(&b_path)],
                shrink_snapshot_ids: vec![],
            }]
        );
    }

    #[test]
    fn exactly_90_days_deletes_nothing_and_past_90_deletes_audio_without_compressing() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let a = meeting(&store, NOW - 90 * DAY, Some(&pair(&rec, "A", "caf", 1, 1)));
        let b = meeting(&store, NOW - 90 * DAY - 1, Some(&pair(&rec, "B", "caf", 1, 1)));
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(steps(&p, a), [MediaStep::Compress]);
        assert_eq!(steps(&p, b), [MediaStep::DeleteAudio]);
    }

    #[test]
    fn delete_suppresses_the_audio_move_but_not_the_snapshot_move() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();
        let id = meeting(&store, NOW - 100 * DAY, Some(&pair(&rec, "A", "caf", 1, 1)));
        let png = rec.join("snaps/10.00.00.png");
        write(&png, 1);
        let snap_id = snap(&store, id, &png);
        let p = plan(&store, &policy(30, 90, 30), NOW, &root, &[]).unwrap();
        assert_eq!(
            steps(&p, id),
            [MediaStep::MoveSnapshots, MediaStep::DeleteAudio, MediaStep::ShrinkImages]
        );
        assert_eq!(p.work[0].shrink_snapshot_ids, [snap_id]);
    }

    #[test]
    fn keep_media_blocks_compress_delete_and_shrink_but_not_moves() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();
        let id = meeting(&store, NOW - 100 * DAY, Some(&pair(&rec, "A", "caf", 1, 1)));
        let png = rec.join("snaps/10.00.00.png");
        write(&png, 1);
        snap(&store, id, &png);
        store.set_keep_media(id, true).unwrap();
        let p = plan(&store, &policy(30, 90, 30), NOW, &root, &[]).unwrap();
        assert_eq!(steps(&p, id), [MediaStep::MoveAudio, MediaStep::MoveSnapshots]);
        assert!(p.work[0].shrink_snapshot_ids.is_empty());
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert!(p.work.is_empty());
    }

    #[test]
    fn busy_meeting_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let id = meeting(&store, NOW - 100 * DAY, Some(&pair(&rec, "A", "caf", 5, 6)));
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[id]).unwrap();
        assert!(p.work.is_empty());
        assert_eq!(p.usage.audio_full_bytes, 11, "a busy meeting still counts in usage");
    }

    #[test]
    fn missing_parent_folder_counts_as_disconnected_and_skips_the_whole_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let id = meeting(&store, NOW - 100 * DAY, Some(&pair(&rec, "A", "caf", 1, 1)));
        snap(&store, id, Path::new("/Volumes/Gone/Snapshots/m/10.00.00.png"));
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert!(p.work.is_empty());
        assert_eq!(p.disconnected_meetings, 1);
    }

    #[test]
    fn missing_file_in_existing_folder_is_already_gone() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        std::fs::create_dir_all(&rec).unwrap();
        let id = meeting(&store, NOW - 100 * DAY, Some(&rec.join("system-audio-A.caf")));
        snap(&store, id, &rec.join("10.00.00.png"));
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();
        let p = plan(&store, &policy(30, 90, 30), NOW, &root, &[]).unwrap();
        assert!(p.work.is_empty());
        assert_eq!(p.disconnected_meetings, 0);
    }

    #[test]
    fn already_m4a_is_not_compressed_again() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        meeting(&store, NOW - 40 * DAY, Some(&pair(&rec, "A", "m4a", 1, 1)));
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert!(p.work.is_empty());
    }

    #[test]
    fn continued_meeting_plans_every_part() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let one = s(&pair(&rec, "A", "m4a", 1, 1));
        let two = s(&pair(&rec, "B", "caf", 1, 1));
        let id = meeting(&store, NOW - 40 * DAY, None);
        store.begin_meeting_part(id, NOW - 40 * DAY, Some("k")).unwrap();
        store.complete_meeting_part(id, 1, NOW - 40 * DAY + 1, Some(&one)).unwrap();
        store.begin_meeting_part(id, NOW - 40 * DAY + 2, None).unwrap();
        store.complete_meeting_part(id, 2, NOW - 40 * DAY + 3, Some(&two)).unwrap();
        store.complete_meeting(id, NOW - 40 * DAY + 3, Some(&one)).unwrap();
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(steps(&p, id), [MediaStep::Compress]);
        assert_eq!(p.work[0].audio_paths, [one, two]);
    }

    #[test]
    fn thumbnails_are_not_shrunk_again() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let snaps = dir.path().join("snaps");
        let thumb = snaps.join("10.00.00.thumb.jpg");
        let png = snaps.join("10.01.00.png");
        write(&thumb, 1);
        write(&png, 1);
        let mixed = meeting(&store, NOW - 40 * DAY, None);
        snap(&store, mixed, &thumb);
        let png_id = snap(&store, mixed, &png);
        let done = meeting(&store, NOW - 40 * DAY, None);
        let only_thumb = snaps.join("11.00.00.thumb.jpg");
        write(&only_thumb, 1);
        snap(&store, done, &only_thumb);
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(steps(&p, mixed), [MediaStep::ShrinkImages]);
        assert_eq!(p.work[0].shrink_snapshot_ids, [png_id]);
        assert!(steps(&p, done).is_empty());
    }

    #[test]
    fn files_outside_media_root_get_move_steps_and_none_when_media_root_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let root = dir.path().join("media");
        let outside = meeting(&store, NOW, Some(&pair(&rec, "A", "caf", 1, 1)));
        let png = rec.join("snaps/10.00.00.png");
        write(&png, 1);
        snap(&store, outside, &png);
        let inside = meeting(&store, NOW, Some(&pair(&root.join("Audio"), "B", "caf", 1, 1)));
        let png = root.join("Snapshots/m/10.00.00.png");
        write(&png, 1);
        snap(&store, inside, &png);
        // A sibling folder whose name merely starts with "Audio" is still outside.
        let lookalike = meeting(&store, NOW, Some(&pair(&root.join("Audio2"), "C", "caf", 1, 1)));
        let p = plan(&store, &MediaPolicy::default(), NOW, &root, &[]).unwrap();
        assert_eq!(steps(&p, outside), [MediaStep::MoveAudio, MediaStep::MoveSnapshots]);
        assert!(steps(&p, inside).is_empty());
        assert_eq!(steps(&p, lookalike), [MediaStep::MoveAudio]);
        let p = plan(&store, &MediaPolicy::default(), NOW, &dir.path().join("gone"), &[]).unwrap();
        assert!(p.work.is_empty());
    }

    #[test]
    fn usage_counts_full_compressed_and_image_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let a = meeting(&store, NOW, Some(&pair(&rec, "A", "caf", 10, 20)));
        meeting(&store, NOW, Some(&pair(&rec, "B", "wav", 1, 2)));
        meeting(&store, NOW, Some(&pair(&rec, "C", "m4a", 3, 4)));
        let png = rec.join("snaps/1.png");
        let thumb = rec.join("snaps/2.thumb.jpg");
        write(&png, 100);
        write(&thumb, 7);
        snap(&store, a, &png);
        snap(&store, a, &thumb);
        let p = plan(&store, &MediaPolicy::default(), NOW, Path::new(NO_ROOT), &[a]).unwrap();
        assert_eq!(
            p.usage,
            MediaUsage {
                audio_full_bytes: 33,
                audio_compressed_bytes: 7,
                image_bytes: 107,
                audio_deletable_bytes: 0
            }
        );
    }

    #[test]
    fn off_policy_never_steps() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();
        let id = meeting(&store, 0, Some(&pair(&rec, "A", "caf", 1, 1)));
        let png = rec.join("snaps/1.png");
        write(&png, 1);
        snap(&store, id, &png);
        let p = plan(&store, &MediaPolicy::default(), NOW, &root, &[]).unwrap();
        assert_eq!(steps(&p, id), [MediaStep::MoveAudio, MediaStep::MoveSnapshots]);
        let p = plan(&store, &MediaPolicy::default(), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert!(p.work.is_empty());
    }

    #[test]
    fn shrink_at_exactly_30_days_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let png = dir.path().join("snaps/1.png");
        write(&png, 1);
        let at = meeting(&store, NOW - 30 * DAY, None);
        snap(&store, at, &png);
        let past = meeting(&store, NOW - 30 * DAY - 1, None);
        let png2 = dir.path().join("snaps/2.png");
        write(&png2, 1);
        snap(&store, past, &png2);
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert!(steps(&p, at).is_empty());
        assert_eq!(steps(&p, past), [MediaStep::ShrinkImages]);
    }

    #[test]
    fn delete_off_at_100_days_compresses() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let id = meeting(&store, NOW - 100 * DAY, Some(&pair(&dir.path().join("rec"), "A", "caf", 1, 1)));
        let policy = MediaPolicy { delete_audio_after_days: None, ..policy(30, 90, 30) };
        let p = plan(&store, &policy, NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(steps(&p, id), [MediaStep::Compress]);
    }

    #[test]
    fn compress_off_at_40_days_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let id = meeting(&store, NOW - 40 * DAY, Some(&pair(&dir.path().join("rec"), "A", "caf", 1, 1)));
        let policy = MediaPolicy { compress_after_days: None, ..policy(30, 90, 30) };
        let p = plan(&store, &policy, NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert!(steps(&p, id).is_empty());
    }

    #[test]
    fn untranscribed_audio_is_compressed_but_never_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let one = s(&pair(&rec, "A", "caf", 1, 1));
        let two = s(&pair(&rec, "B", "caf", 1, 1));
        let id = meeting(&store, NOW - 100 * DAY, None);
        store.begin_meeting_part(id, NOW - 100 * DAY, Some("k")).unwrap();
        store.complete_meeting_part(id, 1, NOW - 100 * DAY + 1, Some(&one)).unwrap();
        store.set_meeting_part_transcript(id, 1, "/a.txt").unwrap();
        store.begin_meeting_part(id, NOW - 100 * DAY + 2, None).unwrap();
        store.complete_meeting_part(id, 2, NOW - 100 * DAY + 3, Some(&two)).unwrap();
        store.complete_meeting(id, NOW - 100 * DAY + 3, Some(&one)).unwrap();
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(steps(&p, id), [MediaStep::Compress]);
        store.set_meeting_part_transcript(id, 2, "/b.txt").unwrap();
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(steps(&p, id), [MediaStep::DeleteAudio]);
    }

    #[test]
    fn audio_past_its_delete_age_on_a_gone_drive_does_not_block_the_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();
        let id = meeting(&store, NOW - 100 * DAY, Some(Path::new("/Volumes/Gone/Audio/system-audio-A.caf")));
        let png = dir.path().join("snaps/1.png");
        write(&png, 1);
        let snap_id = snap(&store, id, &png);
        let p = plan(&store, &policy(30, 90, 30), NOW, &root, &[]).unwrap();
        assert_eq!(p.disconnected_meetings, 0);
        assert_eq!(steps(&p, id), [MediaStep::MoveSnapshots, MediaStep::ShrinkImages]);
        assert_eq!(p.work[0].shrink_snapshot_ids, [snap_id]);
        assert!(p.work[0].audio_paths.is_empty());
        // Before the delete age, the same gone folder still means "disconnected".
        let young = MediaPolicy { delete_audio_after_days: Some(200), ..policy(30, 90, 30) };
        let p = plan(&store, &young, NOW, &root, &[]).unwrap();
        assert_eq!(p.disconnected_meetings, 1);
        assert!(p.work.is_empty());
    }

    #[test]
    fn keep_snapshots_blocks_shrink_but_not_moves_or_audio() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();
        let id = meeting(&store, NOW - 40 * DAY, Some(&pair(&rec, "A", "caf", 1, 1)));
        let png = rec.join("snaps/10.00.00.png");
        write(&png, 1);
        snap(&store, id, &png);
        store.set_keep_snapshots(id, true).unwrap();
        let p = plan(&store, &policy(30, 90, 30), NOW, &root, &[]).unwrap();
        assert_eq!(
            steps(&p, id),
            [
                MediaStep::MoveAudio,
                MediaStep::MoveSnapshots,
                MediaStep::Compress
            ]
        );
        assert!(p.work[0].shrink_snapshot_ids.is_empty());
    }

    #[test]
    fn a_folder_deleted_on_a_mounted_volume_is_already_gone_not_disconnected() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let id = meeting(&store, NOW - 40 * DAY, Some(&pair(&rec, "A", "caf", 1, 1)));
        // The user deleted the whole snapshot folder in Finder: the disk is still here.
        snap(&store, id, &rec.join("deleted-folder/10.00.00.png"));
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(p.disconnected_meetings, 0);
        assert_eq!(
            steps(&p, id),
            [MediaStep::Compress],
            "the meeting's other steps still run"
        );
        // The same folder under an unmounted /Volumes drive is still disconnected.
        let gone = meeting(&store, NOW - 40 * DAY, Some(&pair(&rec, "B", "caf", 1, 1)));
        snap(
            &store,
            gone,
            Path::new("/Volumes/Whimbrel-Test-Gone/Snapshots/m/10.00.00.png"),
        );
        let p = plan(&store, &policy(30, 90, 30), NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(p.disconnected_meetings, 1);
        assert!(steps(&p, gone).is_empty());
    }

    #[test]
    fn delete_off_by_default_plans_no_delete_and_reports_what_turning_it_on_frees() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        let rec = dir.path().join("rec");
        let old = meeting(&store, NOW - 100 * DAY, Some(&pair(&rec, "A", "caf", 5, 6)));
        let kept = meeting(
            &store,
            NOW - 100 * DAY,
            Some(&pair(&rec, "B", "caf", 100, 100)),
        );
        store.set_keep_media(kept, true).unwrap();
        meeting(
            &store,
            NOW - 10 * DAY,
            Some(&pair(&rec, "C", "caf", 1000, 1000)),
        );
        let off = MediaPolicy {
            delete_audio_after_days: None,
            delete_audio_preview_after_days: Some(90),
            ..policy(30, 90, 30)
        };
        let p = plan(&store, &off, NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(steps(&p, old), [MediaStep::Compress]);
        assert_eq!(p.usage.audio_deletable_bytes, 11);
        let on = MediaPolicy {
            delete_audio_after_days: Some(90),
            ..off
        };
        let p = plan(&store, &on, NOW, Path::new(NO_ROOT), &[]).unwrap();
        assert_eq!(steps(&p, old), [MediaStep::DeleteAudio]);
        assert_eq!(p.usage.audio_deletable_bytes, 11);
    }
}
