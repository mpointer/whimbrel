//! Meeting-parts C-ABI surface (meeting continue): thin wrappers over the `Store` methods in
//! `memory::meeting_parts`. JSON out, null on a null handle or error; unit-returning writes
//! return `bool`, like `whimbrel_profile_dismiss_suggestion`.

use std::ffi::c_char;
use std::path::Path;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

fn json_out<T: serde::Serialize, E>(value: Result<T, E>) -> *mut c_char {
    match value.ok().and_then(|v| serde_json::to_string(&v).ok()) {
        Some(json) => json_to_c_string(json),
        None => ptr::null_mut(),
    }
}

/// `{"part_index":N}` for the part just added.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_begin_part(
    handle: *mut StoreHandle,
    meeting_id: i64,
    start_unix_ms: i64,
    continue_key: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let key = cstr_to_string(continue_key);
    json_out(
        store
            .begin_meeting_part(meeting_id, start_unix_ms, key.as_deref())
            .map(|i| serde_json::json!({ "part_index": i })),
    )
}

/// `{"id":N}` or `{"id":null}` when nothing continues.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_find_continuable(
    handle: *mut StoreHandle,
    continue_key: *const c_char,
    now_unix_ms: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Some(key) = cstr_to_string(continue_key) else {
        return ptr::null_mut();
    };
    json_out(
        store
            .find_continuable_meeting(&key, now_unix_ms)
            .map(|id| serde_json::json!({ "id": id })),
    )
}

/// [`whimbrel_meeting_find_continuable`] with the caller's window in ms.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_find_continuable_within(
    handle: *mut StoreHandle,
    continue_key: *const c_char,
    now_unix_ms: i64,
    window_ms: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Some(key) = cstr_to_string(continue_key) else {
        return ptr::null_mut();
    };
    json_out(
        store
            .find_continuable_meeting_within(&key, now_unix_ms, window_ms)
            .map(|id| serde_json::json!({ "id": id })),
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_reopen(handle: *mut StoreHandle, id: i64) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe { &(*handle).0 }.reopen_meeting(id).is_ok()
}

/// Drops a just-begun part (never part 1) and puts the meeting back to completed.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_undo_reopen(handle: *mut StoreHandle, meeting_id: i64, part_index: i64) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe { &(*handle).0 }
        .undo_meeting_reopen(meeting_id, part_index)
        .is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_complete_part(
    handle: *mut StoreHandle,
    meeting_id: i64,
    part_index: i64,
    end_unix_ms: i64,
    audio_path: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let audio = cstr_to_string(audio_path);
    unsafe { &(*handle).0 }
        .complete_meeting_part(meeting_id, part_index, end_unix_ms, audio.as_deref())
        .is_ok()
}

/// Writes only the meeting's notes; null clears them.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_set_notes(
    handle: *mut StoreHandle,
    meeting_id: i64,
    notes: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let notes = cstr_to_string(notes);
    unsafe { &(*handle).0 }
        .set_meeting_notes(meeting_id, notes.as_deref())
        .is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_set_part_transcript(
    handle: *mut StoreHandle,
    meeting_id: i64,
    part_index: i64,
    transcript_path: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(path) = cstr_to_string(transcript_path) else {
        return false;
    };
    unsafe { &(*handle).0 }
        .set_meeting_part_transcript(meeting_id, part_index, &path)
        .is_ok()
}

/// JSON array of `MeetingPart`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_parts(handle: *mut StoreHandle, meeting_id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    json_out(unsafe { &(*handle).0 }.meeting_parts(meeting_id))
}

/// `segments_json` is `[{"start_cs","speaker_label","text"}]` with part-relative `start_cs`.
/// Returns `{"inserted":N}`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_replace_part_segments(
    handle: *mut StoreHandle,
    meeting_id: i64,
    part_index: i64,
    segments_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(json) = cstr_to_string(segments_json) else {
        return ptr::null_mut();
    };
    let Ok(raw) = serde_json::from_str::<Vec<serde_json::Value>>(&json) else {
        return ptr::null_mut();
    };
    let mut segments = Vec::with_capacity(raw.len());
    for v in &raw {
        let (Some(start_cs), Some(speaker), Some(text)) = (
            v.get("start_cs").and_then(|x| x.as_i64()),
            v.get("speaker_label").and_then(|x| x.as_str()),
            v.get("text").and_then(|x| x.as_str()),
        ) else {
            return ptr::null_mut();
        };
        segments.push(memory::NewTranscriptSegment {
            meeting_id,
            start_cs,
            speaker_label: speaker.to_string(),
            text: text.to_string(),
        });
    }
    let store = unsafe { &(*handle).0 };
    json_out(
        store
            .replace_part_transcript_segments(meeting_id, part_index, &segments)
            .map(|n| serde_json::json!({ "inserted": n })),
    )
}

/// The whole-meeting transcript as plain text (not JSON).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_transcript_text(
    handle: *mut StoreHandle,
    meeting_id: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    match unsafe { &(*handle).0 }.meeting_transcript_text(meeting_id) {
        Ok(text) => json_to_c_string(text),
        Err(_) => ptr::null_mut(),
    }
}

/// `SplitResult` JSON: `{"new_meeting_id","moved_snapshot_paths"}`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_split(
    handle: *mut StoreHandle,
    meeting_id: i64,
    from_part_index: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    json_out(unsafe { &(*handle).0 }.split_meeting(meeting_id, from_part_index))
}

/// `moves_json` is `[[old_path,new_path],...]`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_set_snapshot_paths(
    handle: *mut StoreHandle,
    meeting_id: i64,
    moves_json: *const c_char,
    snapshots_dir: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let (Some(moves), Some(dir)) = (cstr_to_string(moves_json), cstr_to_string(snapshots_dir))
    else {
        return false;
    };
    let Ok(moves) = serde_json::from_str::<Vec<(String, String)>>(&moves) else {
        return false;
    };
    unsafe { &(*handle).0 }
        .set_meeting_snapshot_paths(meeting_id, &moves, &dir)
        .is_ok()
}

/// Deletes the meeting row and returns `DeletedMeetingFiles` JSON (every part's files, and
/// `snapshots_dir` only when no other meeting still has snapshots in it); null if there was
/// no such meeting. The caller deletes the files with `whimbrel_retention_delete_all_meeting_files`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_delete_returning_files(
    handle: *mut StoreHandle,
    id: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    match unsafe { &(*handle).0 }.delete_meeting_returning_files(id) {
        Ok(Some(files)) => json_out::<_, ()>(Ok(files)),
        _ => ptr::null_mut(),
    }
}

/// Best-effort file deletion for a `DeletedMeetingFiles` JSON (all parts, plus the guarded
/// snapshot folder). Same tolerance as `whimbrel_retention_delete_meeting_files`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_retention_delete_all_meeting_files(
    files_json: *const c_char,
    recordings_dir: *const c_char,
) {
    let (Some(json), Some(dir)) = (cstr_to_string(files_json), cstr_to_string(recordings_dir))
    else {
        return;
    };
    if let Ok(files) = serde_json::from_str::<memory::DeletedMeetingFiles>(&json) {
        api::retention::delete_all_meeting_files(&files, Path::new(&dir));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{whimbrel_core_free_string, whimbrel_memory_close, whimbrel_memory_open};
    use std::ffi::{CStr, CString};

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }
    fn take(p: *mut c_char) -> String {
        assert!(!p.is_null());
        let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(p);
        s
    }

    fn meeting(handle: *mut StoreHandle, start: i64) -> i64 {
        unsafe { &(*handle).0 }
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: start,
                end_unix_ms: None,
                title: "T".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap()
    }

    #[test]
    fn meeting_parts_round_trip_through_the_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db = c(dir.path().join("t.sqlite").to_str().unwrap());
        let h = whimbrel_memory_open(db.as_ptr(), ptr::null());
        let id = meeting(h, 1_000);
        let key = c("cal:1");

        assert_eq!(
            take(whimbrel_meeting_begin_part(h, id, 1_000, key.as_ptr())),
            r#"{"part_index":1}"#
        );
        assert!(whimbrel_meeting_complete_part(
            h,
            id,
            1,
            60_000,
            c("/r/system-audio-A.caf").as_ptr()
        ));
        unsafe { &(*h).0 }
            .complete_meeting(id, 60_000, Some("/r/system-audio-A.caf"))
            .unwrap();
        assert_eq!(
            take(whimbrel_meeting_find_continuable(h, key.as_ptr(), 120_000)),
            format!(r#"{{"id":{id}}}"#)
        );
        assert_eq!(
            take(whimbrel_meeting_find_continuable(
                h,
                c("other").as_ptr(),
                120_000
            )),
            r#"{"id":null}"#
        );

        // 60_000 ended, 120_000 now: inside a 2-minute window, outside a 30-second one.
        assert_eq!(
            take(whimbrel_meeting_find_continuable_within(
                h,
                key.as_ptr(),
                120_000,
                120_000
            )),
            format!(r#"{{"id":{id}}}"#)
        );
        assert_eq!(
            take(whimbrel_meeting_find_continuable_within(
                h,
                key.as_ptr(),
                120_000,
                30_000
            )),
            r#"{"id":null}"#
        );

        assert!(whimbrel_meeting_reopen(h, id));
        assert_eq!(
            take(whimbrel_meeting_begin_part(h, id, 100_000, ptr::null())),
            r#"{"part_index":2}"#
        );
        assert!(whimbrel_meeting_set_part_transcript(
            h,
            id,
            2,
            c("/r/transcript-B.txt").as_ptr()
        ));
        let parts = take(whimbrel_meeting_parts(h, id));
        assert!(
            parts.contains(r#""part_index":2"#) && parts.contains("transcript-B.txt"),
            "{parts}"
        );
        assert!(whimbrel_meeting_set_notes(h, id, c("typed").as_ptr()));
        assert!(whimbrel_meeting_set_notes(h, id, ptr::null()));
        assert!(!whimbrel_meeting_set_notes(ptr::null_mut(), id, ptr::null()));

        let segs = c(r#"[{"start_cs":10,"speaker_label":"A","text":"hello again"}]"#);
        assert_eq!(
            take(whimbrel_meeting_replace_part_segments(
                h,
                id,
                2,
                segs.as_ptr()
            )),
            r#"{"inserted":1}"#
        );
        assert!(take(whimbrel_meeting_transcript_text(h, id)).contains("hello again"));
        assert!(whimbrel_meeting_replace_part_segments(h, id, 2, c("not json").as_ptr()).is_null());

        unsafe { &(*h).0 }
            .complete_meeting(id, 200_000, None)
            .unwrap();
        let split = take(whimbrel_meeting_split(h, id, 2));
        assert!(
            split.contains("new_meeting_id") && split.contains("moved_snapshot_paths"),
            "{split}"
        );
        assert!(whimbrel_meeting_split(h, id, 9).is_null());
        assert!(whimbrel_meeting_set_snapshot_paths(
            h,
            id,
            c("[[\"/a\",\"/b\"]]").as_ptr(),
            c("/d").as_ptr()
        ));
        assert!(!whimbrel_meeting_set_snapshot_paths(
            h,
            id,
            c("nope").as_ptr(),
            c("/d").as_ptr()
        ));
        whimbrel_memory_close(h);
    }

    #[test]
    fn delete_returning_files_covers_every_part() {
        let dir = tempfile::tempdir().unwrap();
        let db = c(dir.path().join("t.sqlite").to_str().unwrap());
        let h = whimbrel_memory_open(db.as_ptr(), ptr::null());
        let id = meeting(h, 1_000);
        let rec = dir.path();
        let touch = |n: &str| {
            let p = rec.join(n);
            std::fs::write(&p, n).unwrap();
            p
        };
        let (s1, m1, t1) = (
            touch("system-audio-A.caf"),
            touch("mic-audio-A.caf"),
            touch("transcript-A.txt"),
        );
        let (s2, m2, t2) = (
            touch("system-audio-B.caf"),
            touch("mic-audio-B.caf"),
            touch("transcript-B.txt"),
        );
        whimbrel_meeting_begin_part(h, id, 1_000, ptr::null());
        whimbrel_meeting_complete_part(h, id, 1, 2_000, c(s1.to_str().unwrap()).as_ptr());
        whimbrel_meeting_set_part_transcript(h, id, 1, c(t1.to_str().unwrap()).as_ptr());
        whimbrel_meeting_begin_part(h, id, 3_000, ptr::null());
        whimbrel_meeting_complete_part(h, id, 2, 4_000, c(s2.to_str().unwrap()).as_ptr());
        whimbrel_meeting_set_part_transcript(h, id, 2, c(t2.to_str().unwrap()).as_ptr());

        let files = whimbrel_meeting_delete_returning_files(h, id);
        assert!(!files.is_null());
        whimbrel_retention_delete_all_meeting_files(files, c(rec.to_str().unwrap()).as_ptr());
        whimbrel_core_free_string(files);
        for p in [s1, m1, t1, s2, m2, t2] {
            assert!(!p.exists(), "{p:?}");
        }
        assert!(
            whimbrel_meeting_delete_returning_files(h, id).is_null(),
            "already gone"
        );
        whimbrel_memory_close(h);
    }

    #[test]
    fn null_handle_returns_null() {
        assert!(whimbrel_meeting_begin_part(ptr::null_mut(), 1, 1, ptr::null()).is_null());
        assert!(whimbrel_meeting_parts(ptr::null_mut(), 1).is_null());
        assert!(whimbrel_meeting_transcript_text(ptr::null_mut(), 1).is_null());
        assert!(whimbrel_meeting_split(ptr::null_mut(), 1, 2).is_null());
        assert!(whimbrel_meeting_delete_returning_files(ptr::null_mut(), 1).is_null());
        assert!(!whimbrel_meeting_reopen(ptr::null_mut(), 1));
        assert!(!whimbrel_meeting_undo_reopen(ptr::null_mut(), 1, 2));
    }

    #[test]
    fn undo_reopen_through_the_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db = c(dir.path().join("t.sqlite").to_str().unwrap());
        let h = whimbrel_memory_open(db.as_ptr(), ptr::null());
        let id = meeting(h, 1_000);
        let store = unsafe { &(*h).0 };
        store.begin_meeting_part(id, 1_000, Some("cal:1")).unwrap();
        store.complete_meeting(id, 5_000, None).unwrap();
        assert!(whimbrel_meeting_reopen(h, id));
        let part = store.begin_meeting_part(id, 6_000, None).unwrap();
        assert!(whimbrel_meeting_undo_reopen(h, id, part));
        let m = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(m.status, memory::MeetingStatus::Completed);
        assert_eq!(m.end_unix_ms, Some(5_000));
        assert_eq!(store.meeting_parts(id).unwrap().len(), 1);
        whimbrel_memory_close(h);
    }
}
