//! Speaker names (docs/superpowers/specs/2026-11-05-speaker-names-design.md): naming a
//! meeting's speakers after diarization, and the C-ABI the Mac calls for names and voices.
//! Every function returns `{"ok":true,"data":…}`, `{"ok":false,"error":{"kind":"store"}}` on a
//! database error, or NULL on a null handle or unreadable argument. Free with
//! `whimbrel_core_free_string`.

use std::ffi::c_char;
use std::ptr;

use api::speaker_names::{assign_meeting_speakers, confirm_speaker};
use memory::speaker_names::{display_label, labelled_speaker, Attendee};
use memory::Store;
use serde::Deserialize;

use crate::{cstr_to_string, json_to_c_string, parse_routing_config, StoreHandle};

/// One entry of `Diarization.voices`; `speech_cs` is ignored.
#[derive(Deserialize)]
struct Voice {
    speaker_id: i32,
    embedding: Vec<f32>,
}

pub(crate) fn envelope<T: serde::Serialize, E>(r: Result<T, E>) -> *mut c_char {
    let json = match r.map(|v| serde_json::to_string(&v)) {
        Ok(Ok(data)) => format!(r#"{{"ok":true,"data":{data}}}"#),
        Ok(Err(_)) => return ptr::null_mut(),
        Err(_) => r#"{"ok":false,"error":{"kind":"store"}}"#.to_string(),
    };
    json_to_c_string(json)
}

fn store<'a>(handle: *mut StoreHandle) -> Option<&'a Store> {
    (!handle.is_null()).then(|| unsafe { &(*handle).0 })
}

/// Diarizes `wav_path` with one voice embedding per speaker: `{"segments":[SpeakerSegment],
/// "voices":[{"speaker_id","embedding","speech_cs"}]}`, or NULL on any failure (as
/// `whimbrel_diarize_wav`).
#[cfg(feature = "diarize")]
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_diarize_wav_voices(
    segmentation_model_path: *const c_char,
    embedding_model_path: *const c_char,
    wav_path: *const c_char,
    expected_speaker_count: i32,
) -> *mut c_char {
    let (Some(seg), Some(emb), Some(wav)) =
        (cstr_to_string(segmentation_model_path), cstr_to_string(embedding_model_path), cstr_to_string(wav_path))
    else {
        return ptr::null_mut();
    };
    let models = diarize::DiarizationModelPaths { segmentation_model_path: seg.into(), embedding_model_path: emb.into() };
    let expected = (expected_speaker_count > 0).then_some(expected_speaker_count);
    let Ok(d) = diarize::diarize_with_voices(std::path::Path::new(&wav), &models, expected, None) else {
        return ptr::null_mut();
    };
    json_to_c_string(serde_json::json!({ "segments": d.segments, "voices": d.voices }).to_string())
}

/// Names the meeting's speakers from `voices_json` (`Diarization.voices`) and returns
/// `[MeetingSpeaker]`. `model` and `routing_config_json` are the summary's. `remember` off
/// stores no embeddings. A model failure or spend cap leaves the rest unnamed, still ok.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_assign_meeting_speakers(
    handle: *mut StoreHandle,
    meeting_id: i64,
    voices_json: *const c_char,
    model: *const c_char,
    routing_config_json: *const c_char,
    remember: bool,
) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    let (Some(voices), Some(model)) = (cstr_to_string(voices_json), cstr_to_string(model)) else {
        return ptr::null_mut();
    };
    let Ok(voices) = serde_json::from_str::<Vec<Voice>>(&voices) else {
        return ptr::null_mut();
    };
    let voices: Vec<(i32, Vec<f32>)> = voices.into_iter().map(|v| (v.speaker_id, v.embedding)).collect();
    let routing = parse_routing_config(routing_config_json);
    envelope(assign_meeting_speakers(store, meeting_id, &voices, &model, &routing, remember))
}

/// Stores the meeting's invitees with their emails (`attendees_json`: `[{"name","email"}]`,
/// the user left out). data: null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_set_meeting_attendee_details(handle: *mut StoreHandle, meeting_id: i64, attendees_json: *const c_char) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    let Some(attendees) = cstr_to_string(attendees_json).and_then(|j| serde_json::from_str::<Vec<Attendee>>(&j).ok()) else {
        return ptr::null_mut();
    };
    envelope(store.set_meeting_attendee_details(meeting_id, &attendees))
}

/// The meeting's invitees `[{"name","email"}]` (names only, email null, when none were stored).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_attendee_details(handle: *mut StoreHandle, meeting_id: i64) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    envelope(store.meeting_attendee_details(meeting_id))
}

/// `[MeetingSpeaker]` for the meeting.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_speakers(handle: *mut StoreHandle, meeting_id: i64) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    envelope(store.meeting_speakers(meeting_id))
}

/// Mike names a speaker (confirmed); returns the meeting's `[MeetingSpeaker]`. `email` may be NULL.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_set_meeting_speaker(
    handle: *mut StoreHandle,
    meeting_id: i64,
    speaker_id: i32,
    name: *const c_char,
    email: *const c_char,
) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    let Some(name) = cstr_to_string(name).filter(|n| !n.trim().is_empty()) else {
        return ptr::null_mut();
    };
    let email = cstr_to_string(email);
    envelope(confirm_speaker(store, meeting_id, speaker_id, &name, email.as_deref()).and_then(|()| store.meeting_speakers(meeting_id)))
}

/// Mike's "Yes, that's X": confirms the speaker as `person_key`, the person it was shown as,
/// never re-resolved by name. A speaker no longer that person is left as is. Returns the
/// meeting's `[MeetingSpeaker]`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_confirm_meeting_speaker(
    handle: *mut StoreHandle,
    meeting_id: i64,
    speaker_id: i32,
    person_key: *const c_char,
) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    let Some(key) = cstr_to_string(person_key) else { return ptr::null_mut() };
    envelope(store.confirm_meeting_speaker(meeting_id, speaker_id, &key).and_then(|_| store.meeting_speakers(meeting_id)))
}

/// `[{"label","guess","person_key"}]` for each raw label in `labels_json` (`["Speaker 1","Me",…]`);
/// `person_key` is the named speaker's, else null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_display_labels(handle: *mut StoreHandle, meeting_id: i64, labels_json: *const c_char) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    let Some(labels) = cstr_to_string(labels_json).and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok()) else {
        return ptr::null_mut();
    };
    envelope(store.meeting_speakers(meeting_id).map(|speakers| {
        labels
            .iter()
            .map(|raw| {
                let (label, guess) = display_label(raw, &speakers);
                let key = labelled_speaker(raw, &speakers).filter(|s| s.name.is_some()).and_then(|s| s.person_key.clone());
                serde_json::json!({ "label": label, "guess": guess, "person_key": key })
            })
            .collect::<Vec<_>>()
    }))
}

/// `[VoicePerson]`: people with a remembered voice.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_voice_people(handle: *mut StoreHandle) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    envelope(store.voice_people())
}

/// Forgets one person's voice; returns the remaining `[VoicePerson]`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_forget_voice(handle: *mut StoreHandle, person_key: *const c_char) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    let Some(key) = cstr_to_string(person_key) else { return ptr::null_mut() };
    envelope(store.forget_voice(&key).and_then(|()| store.voice_people()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::speaker_match::NameStatus;
    use std::ffi::{CStr, CString};

    fn meeting(store: &Store, invitees: &[(&str, Option<&str>)]) -> i64 {
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Sync".into(),
                attendees: invitees.iter().map(|(n, _)| n.to_string()).collect(),
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        let details: Vec<Attendee> =
            invitees.iter().map(|(n, e)| Attendee { name: n.to_string(), email: e.map(str::to_string) }).collect();
        store.set_meeting_attendee_details(id, &details).unwrap();
        id
    }

    #[test]
    fn confirm_keeps_the_guessed_person_when_invitees_share_a_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = CString::new(dir.path().join("w.db").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        let s = unsafe { &(*handle).0 };
        let take = |p: *mut c_char| -> serde_json::Value {
            assert!(!p.is_null());
            let v = serde_json::from_str(unsafe { CStr::from_ptr(p) }.to_str().unwrap()).unwrap();
            crate::whimbrel_core_free_string(p);
            v
        };
        let m = meeting(s, &[("Sam", Some("sam@a.com")), ("Sam", Some("sam@b.com"))]);
        s.upsert_person("sam@b.com", "Sam", Some("sam@b.com")).unwrap();
        s.set_meeting_speaker(m, 0, Some("sam@b.com"), Some(NameStatus::Guess)).unwrap();

        let labels = CString::new(r#"["Speaker 1","Speaker 2"]"#).unwrap();
        let v = take(whimbrel_display_labels(handle, m, labels.as_ptr()));
        assert_eq!(v["data"], serde_json::json!([
            {"label":"Sam","guess":true,"person_key":"sam@b.com"},{"label":"Speaker 2","guess":false,"person_key":null}
        ]));
        let key = CString::new("sam@b.com").unwrap();
        let v = take(whimbrel_confirm_meeting_speaker(handle, m, 0, key.as_ptr()));
        assert_eq!(v["data"][0]["person_key"], "sam@b.com");
        assert_eq!(v["data"][0]["status"], "confirmed");
        assert!(whimbrel_confirm_meeting_speaker(handle, m, 0, ptr::null()).is_null());
        crate::whimbrel_memory_close(handle);
    }

    #[test]
    fn set_meeting_attendee_details_stores_emails() {
        let dir = tempfile::tempdir().unwrap();
        let path = CString::new(dir.path().join("w.db").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        let s = unsafe { &(*handle).0 };
        let m = meeting(s, &[]);
        let json = CString::new(r#"[{"name":"Ana","email":"ana@x.com"},{"name":"Bo","email":null}]"#).unwrap();
        let out = whimbrel_set_meeting_attendee_details(handle, m, json.as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(out) }.to_str().unwrap(), r#"{"ok":true,"data":null}"#);
        crate::whimbrel_core_free_string(out);
        assert_eq!(s.meeting_attendee_details(m).unwrap(), vec![
            Attendee { name: "Ana".into(), email: Some("ana@x.com".into()) },
            Attendee { name: "Bo".into(), email: None },
        ]);
        let out = whimbrel_meeting_attendee_details(handle, m);
        assert_eq!(
            unsafe { CStr::from_ptr(out) }.to_str().unwrap(),
            r#"{"ok":true,"data":[{"name":"Ana","email":"ana@x.com"},{"name":"Bo","email":null}]}"#
        );
        crate::whimbrel_core_free_string(out);
        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_set_meeting_attendee_details(handle, m, bad.as_ptr()).is_null());
        assert!(whimbrel_meeting_attendee_details(ptr::null_mut(), m).is_null());
        crate::whimbrel_memory_close(handle);
    }

    #[test]
    fn set_meeting_speaker_confirms_and_teaches() {
        let dir = tempfile::tempdir().unwrap();
        let path = CString::new(dir.path().join("w.db").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let s = unsafe { &(*handle).0 };
        let take = |p: *mut c_char| -> serde_json::Value {
            assert!(!p.is_null());
            let v = serde_json::from_str(unsafe { CStr::from_ptr(p) }.to_str().unwrap()).unwrap();
            crate::whimbrel_core_free_string(p);
            v
        };

        let m1 = meeting(s, &[("Ana", Some("Ana@X.com")), ("Bo", None)]);
        let voices = CString::new(r#"[{"speaker_id":0,"embedding":[1.0,0.0],"speech_cs":500},{"speaker_id":1,"embedding":[0.0,1.0],"speech_cs":9}]"#).unwrap();
        let model = CString::new("gemma4:12b").unwrap();
        // No transcript: the model is never asked, so nothing is named yet.
        let v = take(whimbrel_assign_meeting_speakers(handle, m1, voices.as_ptr(), model.as_ptr(), ptr::null(), true));
        assert_eq!(v["ok"], true);
        assert!(v["data"].as_array().unwrap().iter().all(|sp| sp["person_key"].is_null()));

        let name = CString::new(" ana ").unwrap();
        let v = take(whimbrel_set_meeting_speaker(handle, m1, 0, name.as_ptr(), ptr::null()));
        assert_eq!(v["data"][0]["person_key"], "ana@x.com");
        assert_eq!(v["data"][0]["status"], "confirmed");

        let labels = CString::new(r#"["Speaker 1","Speaker 2","Me"]"#).unwrap();
        let v = take(whimbrel_display_labels(handle, m1, labels.as_ptr()));
        assert_eq!(v["data"], serde_json::json!([
            {"label":"Ana","guess":false,"person_key":"ana@x.com"},{"label":"Speaker 2","guess":false,"person_key":null},
            {"label":"Me","guess":false,"person_key":null}
        ]));

        let m2 = meeting(s, &[("Ana", Some("ana@x.com")), ("Cy", None), ("Di", None)]);
        let near = CString::new(r#"[{"speaker_id":3,"embedding":[0.98,0.1]}]"#).unwrap();
        let v = take(whimbrel_assign_meeting_speakers(handle, m2, near.as_ptr(), model.as_ptr(), ptr::null(), true));
        assert_eq!(v["data"][0]["name"], "Ana");
        assert_eq!(v["data"][0]["status"], "confirmed");

        let v = take(whimbrel_voice_people(handle));
        assert_eq!(v["data"][0]["confirmed_count"], 2);
        let key = CString::new("ana@x.com").unwrap();
        let v = take(whimbrel_forget_voice(handle, key.as_ptr()));
        assert_eq!(v["data"], serde_json::json!([]));
        crate::whimbrel_memory_close(handle);
    }
}
