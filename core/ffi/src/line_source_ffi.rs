//! The finished note's line sources: a thin wrapper over `Store::meeting_note_sources`.
//! Device-local, never synced. JSON out, NULL on a null handle, an error or a missing meeting.

use std::ffi::c_char;
use std::ptr;

use crate::{json_to_c_string, StoreHandle};

/// The meeting's current decisions, open questions and action items, each with the source that
/// still holds for it on this device. A source is returned only when it was stored for exactly
/// this line (so an edited line, or a summary regenerated elsewhere, has none) and the transcript
/// line is still in the meeting. Shape:
///
/// `{"decisions":[{"text","source"?}],"open_questions":[{"text","source"?}],
///   "action_items":[{"item_id"|null,"text","owner"?,"due"?,"source"?}]}`
///
/// where `source` is `{"segment_id","start_cs","speaker_label","text"}`: `segment_id` is the `id`
/// of the matching line in `whimbrel_memory_get_transcript_segments_page`, `text` the line's own
/// stored words. The lines are the edited ones when the person edited them, else the generated
/// ones. `action_items` come from the meeting's action item rows once they are seeded (`item_id`
/// is the row id; the owner, due and source attach by the item's exact text, and a person's own
/// item gets none), and before that from the generated summary strings with `item_id` null.
/// Absent keys mean none.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_note_sources(handle: *mut StoreHandle, meeting_id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    match unsafe { &(*handle).0 }.meeting_note_sources(meeting_id) {
        Ok(Some(notes)) => serde_json::to_string(&notes).map_or(ptr::null_mut(), json_to_c_string),
        _ => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    fn take(p: *mut c_char) -> serde_json::Value {
        assert!(!p.is_null());
        let v = serde_json::from_str(unsafe { CStr::from_ptr(p) }.to_str().unwrap()).unwrap();
        crate::whimbrel_core_free_string(p);
        v
    }

    const START: i64 = 1_700_000_000_000;

    #[test]
    fn the_note_sources_json_has_the_documented_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = CString::new(dir.path().join("w.db").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        let store = unsafe { &(*handle).0 };
        let m = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: START,
                end_unix_ms: Some(START + 600_000),
                title: "Sync".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        // Lines go in through the C call, one with an end time.
        let segs = CString::new(r#"[{"start_cs":0,"speaker_label":"Speaker 1","text":"We ship on Friday, no matter what.","end_cs":480},
                                    {"start_cs":900,"speaker_label":"Speaker 2","text":"Fine, I'll send the deck on Monday."}]"#)
            .unwrap();
        assert!(crate::whimbrel_memory_insert_transcript_segments(handle, m, segs.as_ptr()));
        store
            .update_meeting_summary(m, "t", &["Ship on Friday".into(), "Plain".into()], &["Bo: send the deck (Monday)".into()], &["Who?".into()], "p", "mod")
            .unwrap();
        let row = |kind, line: &str, owner: Option<&str>, due: Option<&str>, cs: i64, spoken: &str| memory::line_source::NewLineSource {
            kind,
            line_text: line.into(),
            owner: owner.map(Into::into),
            due: due.map(Into::into),
            locator: Some(memory::line_source::SourceLocator {
                offset_secs: cs / 100,
                base: memory::line_source::base_fingerprint("We ship on Friday, no matter what."),
                anchor: memory::line_source::anchor_of(spoken),
            }),
        };
        store.begin_meeting_part(m, START, None).unwrap();
        store
            .replace_meeting_line_sources(
                m,
                &[
                    row(memory::line_source::LineKind::Decision, "Ship on Friday", None, None, 0, "We ship on Friday, no matter what."),
                    row(memory::line_source::LineKind::ActionItem, "Bo: send the deck (Monday)", Some("Bo"), Some("Monday"), 900, "Fine, I'll send the deck on Monday."),
                ],
                1,
            )
            .unwrap();
        let v = take(whimbrel_meeting_note_sources(handle, m));
        let page = take(crate::whimbrel_memory_get_transcript_segments_page(handle, m, 0, 10));
        let first_id = page["segments"][0]["id"].as_i64().unwrap();
        assert_eq!(page["segments"][0]["end_cs"], 480, "the end stored with the line");
        assert!(page["segments"][1].get("end_cs").is_none(), "a line stored without one has no key");
        assert_eq!(
            v,
            serde_json::json!({
                "decisions": [
                    {"text": "Ship on Friday", "source": {"segment_id": first_id, "start_cs": 0, "speaker_label": "Speaker 1", "text": "We ship on Friday, no matter what."}},
                    {"text": "Plain"}
                ],
                "open_questions": [{"text": "Who?"}],
                "action_items": [{
                    "item_id": null, "text": "Bo: send the deck (Monday)", "owner": "Bo", "due": "Monday",
                    "source": {"segment_id": page["segments"][1]["id"], "start_cs": 900, "speaker_label": "Speaker 2", "text": "Fine, I'll send the deck on Monday."}
                }]
            })
        );
        assert!(whimbrel_meeting_note_sources(handle, m + 99).is_null());
        assert!(whimbrel_meeting_note_sources(ptr::null_mut(), m).is_null());
        crate::whimbrel_memory_close(handle);
    }
}
