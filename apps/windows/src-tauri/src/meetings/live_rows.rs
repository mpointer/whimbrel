//! The live transcript: what the live worker (`live`) has heard so far, stored as core's
//! local-only provisional rows (`memory::live`) for the meeting page's pane and the meeting
//! Ask bar (`api::ask` reads them through `transcript_for_meeting` until the real transcript
//! exists). Never synced. Plan:
//! `docs/superpowers/plans/2026-11-07-windows-live-transcript-and-meeting-ask.md`
//! Decisions 3 to 5.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use memory::live::NewLiveSegment;

use super::live::{Accumulated, Listener};
use super::pipeline::{ME, OTHERS};
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;

/// How many of each channel's segments are already stored.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    pub system: usize,
    pub mic: usize,
}

/// One store write: clear the meeting's rows first, or only append.
#[derive(Debug, Default)]
pub struct Update {
    pub clear: bool,
    pub rows: Vec<NewLiveSegment>,
}

/// The rows a snapshot adds. Rows are appended, never replaced: a live citation is a row's
/// id (`-rowid`), so an answer given in the call must still point at its line later. A
/// channel the worker gave up on shrinks; then everything is cleared and written again,
/// which is rare (a failed slice) and only renumbers rows.
pub fn plan(written: &mut Written, meeting_id: i64, state: &Accumulated) -> Update {
    let clear = state.system.len() < written.system || state.mic.len() < written.mic;
    if clear {
        *written = Written::default();
    }
    let mut rows = Vec::new();
    for (segments, done, channel, label) in [
        (&state.system, &mut written.system, "system", OTHERS),
        (&state.mic, &mut written.mic, "mic", ME),
    ] {
        for (i, s) in segments.iter().enumerate().skip(*done) {
            let text = s.text.trim();
            if text.is_empty() {
                continue;
            }
            rows.push(NewLiveSegment {
                meeting_id,
                channel: channel.into(),
                chunk_index: i as i64,
                start_cs: s.start_cs,
                end_cs: s.end_cs,
                speaker_label: label.into(),
                text: text.into(),
            });
        }
        *done = segments.len();
    }
    Update { clear, rows }
}

/// Applies one update, unless the live transcript was turned off (`enabled` reads the
/// setting here, serialized with the clear that turning it off runs) or the meeting already
/// has its real transcript, so a late write can't bring rows back. Returns whether it wrote.
pub fn apply(store: &memory::Store, meeting_id: i64, update: &Update, enabled: bool) -> Result<bool, String> {
    if !enabled {
        return Ok(false);
    }
    let (real, _) = store.get_transcript_segments_page(meeting_id, 0, 1).map_err(|e| e.to_string())?;
    if !real.is_empty() {
        return Ok(false);
    }
    if update.clear {
        store.clear_live_segments(meeting_id).map_err(|e| e.to_string())?;
    }
    if !update.rows.is_empty() {
        store.insert_live_segments(&update.rows).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

/// Starts a writer thread for one recording and returns the worker's listener. Snapshots go
/// over a channel, so the worker never waits on the store; the writer takes only the newest.
/// It ends when the worker drops the listener.
pub fn listener(store: StoreHandle, settings: Arc<Mutex<SettingsFile>>, meeting_id: i64) -> Option<Listener> {
    let (tx, rx) = mpsc::channel::<Accumulated>();
    std::thread::Builder::new()
        .name("whimbrel-live-rows".into())
        .spawn(move || write_all(&store, &settings, meeting_id, &rx))
        .map_err(|e| tracing::error!(error = %e, "could not start the live transcript writer"))
        .ok()?;
    Some(Listener(Arc::new(move |state: &Accumulated| {
        let _ = tx.send(state.clone());
    })))
}

fn write_all(store: &StoreHandle, settings: &Arc<Mutex<SettingsFile>>, meeting_id: i64, rx: &mpsc::Receiver<Accumulated>) {
    let mut written = Written::default();
    while let Ok(mut state) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            state = newer;
        }
        let mut next = written;
        let update = plan(&mut next, meeting_id, &state);
        let settings = settings.clone();
        let wrote = store.call_blocking(move |s| apply(s, meeting_id, &update, settings.lock().unwrap().live_transcript_enabled()));
        match wrote {
            Ok(Ok(true)) => written = next,
            // Off (its rows were deleted) or replaced by the real transcript: if it comes back
            // on, everything heard so far is written again.
            Ok(Ok(false)) => written = Written::default(),
            Ok(Err(e)) => tracing::warn!(error = %e, "could not store the live transcript"),
            Err(e) => tracing::warn!(error = %e, "could not store the live transcript"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transcribe::TranscriptSegment;

    fn seg(start_cs: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment { start_cs, end_cs: start_cs + 90, text: text.into() }
    }

    fn meeting(store: &memory::Store) -> i64 {
        store
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev".into(),
                start_unix_ms: 1_000,
                end_unix_ms: None,
                title: "Acme sync".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap()
    }

    fn texts(store: &memory::Store, id: i64) -> Vec<(String, String)> {
        store.list_live_segments(id).unwrap().into_iter().map(|s| (s.speaker_label, s.text)).collect()
    }

    #[test]
    fn each_snapshot_appends_only_what_is_new_and_rows_keep_their_ids() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting(&store);
        let mut written = Written::default();
        let mut state = Accumulated { system: vec![seg(0, " Hello there ")], mic: vec![seg(50, "Hi")], ..Accumulated::default() };
        assert!(apply(&store, id, &plan(&mut written, id, &state), true).unwrap());
        let first = store.list_live_segments(id).unwrap();

        state.system.push(seg(300, "How was the trip?"));
        state.mic.push(seg(400, "   "));
        let update = plan(&mut written, id, &state);
        assert!(!update.clear);
        assert_eq!(update.rows.len(), 1, "blank lines aren't stored");
        apply(&store, id, &update, true).unwrap();
        assert_eq!(
            texts(&store, id),
            [("Others".into(), "Hello there".into()), ("Me".into(), "Hi".into()), ("Others".into(), "How was the trip?".into())]
        );
        let again = store.list_live_segments(id).unwrap();
        assert_eq!(first.iter().map(|s| s.id).collect::<Vec<_>>(), again[..2].iter().map(|s| s.id).collect::<Vec<_>>(), "ids are stable");
        assert_eq!(written, Written { system: 2, mic: 2 });
    }

    #[test]
    fn a_channel_given_up_on_clears_and_rewrites() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting(&store);
        let mut written = Written::default();
        let state = Accumulated { system: vec![seg(0, "a"), seg(100, "b")], mic: vec![seg(50, "me")], ..Accumulated::default() };
        apply(&store, id, &plan(&mut written, id, &state), true).unwrap();
        // The worker dropped the system channel after a failed slice.
        let state = Accumulated { system: vec![], mic: vec![seg(50, "me"), seg(150, "still me")], ..Accumulated::default() };
        let update = plan(&mut written, id, &state);
        assert!(update.clear);
        apply(&store, id, &update, true).unwrap();
        assert_eq!(texts(&store, id), [("Me".into(), "me".into()), ("Me".into(), "still me".into())]);
    }

    #[test]
    fn nothing_is_written_when_off_or_once_the_real_transcript_exists() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = meeting(&store);
        let state = Accumulated { mic: vec![seg(0, "secret plans")], ..Accumulated::default() };
        assert!(!apply(&store, id, &plan(&mut Written::default(), id, &state), false).unwrap());
        assert!(texts(&store, id).is_empty(), "off writes nothing");

        store
            .replace_part_transcript_segments(
                id,
                1,
                &[memory::NewTranscriptSegment { meeting_id: id, start_cs: 0, speaker_label: "Me".into(), text: "secret plans".into() }],
            )
            .unwrap();
        assert!(!apply(&store, id, &plan(&mut Written::default(), id, &state), true).unwrap());
        assert!(texts(&store, id).is_empty(), "a late write after the pipeline is dropped");
    }

    #[test]
    fn the_writer_thread_stores_what_the_worker_hears_and_ends_with_it() {
        let store = StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap();
        let id = store.call_blocking(meeting).unwrap();
        let path = std::env::temp_dir().join(format!("whimbrel-live-rows-{}.json", std::process::id()));
        let settings = Arc::new(Mutex::new(SettingsFile::load(&path)));
        let listener = listener(store.clone(), settings, id).unwrap();
        (listener.0)(&Accumulated { mic: vec![seg(0, "first")], ..Accumulated::default() });
        (listener.0)(&Accumulated { mic: vec![seg(0, "first"), seg(100, "second")], ..Accumulated::default() });
        drop(listener);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let rows = store.call_blocking(move |s| s.list_live_segments(id).unwrap()).unwrap();
            if rows.len() == 2 {
                assert_eq!(rows[1].text, "second");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the writer stored {rows:?}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
