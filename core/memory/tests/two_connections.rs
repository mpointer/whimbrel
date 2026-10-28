//! Two `Store`s on one database file, written from two threads at once. The Windows
//! shell runs a main store actor (capture, sync, the Library) and a "slow lane" actor
//! (meeting summaries, blocking asks) on separate connections (M3.3 plan, Decision 2);
//! `whimbrel-mcp-server` has always opened a second connection the same way. These tests
//! pin the properties that arrangement relies on: a second open waits out an in-flight
//! write rather than failing, concurrent writers never hit SQLITE_BUSY within the default
//! busy timeout, and the one `local_seq` counter stays unique across both connections.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use memory::{MeetingStatus, MeetingTrigger, NewCaptureItem, NewMeeting, Store};

const DEVICE: &str = "device-a";

fn temp_db(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    (dir, path)
}

fn meeting(start: i64, status: MeetingStatus) -> NewMeeting {
    NewMeeting {
        device_id: DEVICE.to_string(),
        start_unix_ms: start,
        end_unix_ms: None,
        title: "Meeting".to_string(),
        attendees: Vec::new(),
        trigger: MeetingTrigger::Manual,
        audio_path: None,
        status,
    }
}

/// `Store::open`'s schema setup takes the write lock even on an up-to-date database
/// (`INSERT OR IGNORE` into the counters), so a second open waits behind an in-flight
/// write for up to the connection's busy timeout (5 s, rusqlite's default). Shells open
/// their second connection at launch, before capture or sync start writing.
#[test]
fn a_second_open_waits_for_an_in_flight_write_to_commit() {
    let (_dir, path) = temp_db("open.sqlite");
    drop(Store::open(&path, Path::new("")).unwrap());

    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("BEGIN IMMEDIATE; UPDATE sync_local_seq_counter SET next_seq = next_seq WHERE id = 1;")
        .unwrap();
    let committer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        raw.execute_batch("COMMIT").unwrap();
    });

    let started = std::time::Instant::now();
    let second = Store::open(&path, Path::new(""));
    committer.join().unwrap();
    assert!(second.is_ok(), "{:?}", second.err());
    assert!(started.elapsed() >= std::time::Duration::from_millis(250), "the open waited for the commit");
}

/// How long the two writers get before the test calls it a deadlock. Generous for a
/// loaded CI runner, and far short of a job timeout: this test once hung for 40 minutes
/// on the self-hosted Windows runner while it was also building, and a hang that fails
/// fast is worth more than one that eats the job.
const WRITERS_DEADLINE: Duration = Duration::from_secs(120);

#[test]
fn concurrent_writers_on_two_connections_never_fail_and_local_seq_stays_unique() {
    // Each round is four write transactions plus their FTS and vector rows, and the two
    // connections contend for the same write lock, so this is deliberately more traffic
    // than the shell ever generates — but not so much that a busy runner can't finish it.
    const ROUNDS: i64 = 60;
    let (_dir, path) = temp_db("stress.sqlite");
    let main_store = Store::open(&path, Path::new("")).unwrap();
    main_store.set_own_device_id(DEVICE);
    let meeting_ids: Vec<i64> = (0..ROUNDS)
        .map(|i| main_store.insert_meeting(&meeting(i, MeetingStatus::Recording)).unwrap())
        .collect();
    drop(main_store);

    let barrier = Arc::new(Barrier::new(2));

    // The main actor's shape: capture inserts and meeting completion.
    let main = {
        let (path, barrier, ids) = (path.clone(), barrier.clone(), meeting_ids.clone());
        std::thread::spawn(move || {
            let store = Store::open(&path, Path::new("")).unwrap();
            store.set_own_device_id(DEVICE);
            barrier.wait();
            for (i, id) in ids.iter().enumerate() {
                let i = i as i64;
                store
                    .insert_capture_item(&NewCaptureItem {
                        device_id: DEVICE.to_string(),
                        ts_unix_ms: 1_000 + i,
                        app_bundle_id: format!("app-{}", i % 3),
                        window_title: "w".to_string(),
                        url: None,
                        text: format!("captured text number {i}"),
                        hash: i,
                        source: "uia".to_string(),
                    })
                    .expect("capture insert");
                store.complete_meeting(*id, 2_000 + i, None).expect("complete_meeting");
            }
        })
    };

    // The slow lane's shape: summary writes, which re-stamp local_seq and enqueue outbox rows.
    let slow = {
        let (path, barrier, ids) = (path.clone(), barrier.clone(), meeting_ids.clone());
        std::thread::spawn(move || {
            let store = Store::open(&path, Path::new("")).unwrap();
            store.set_own_device_id(DEVICE);
            barrier.wait();
            for id in ids.iter().rev() {
                store
                    .update_meeting_summary(*id, "tl;dr", &[], &[], &[], "ollama", "m")
                    .expect("update_meeting_summary");
                store.set_meeting_title_if_placeholder(*id, "Generated").expect("retitle");
            }
        })
    };

    // Joined off the test thread, so a writer stuck on a lock fails here instead of
    // hanging until CI gives up.
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = main.join().and_then(|()| slow.join());
        let _ = done.send(outcome);
    });
    match finished.recv_timeout(WRITERS_DEADLINE) {
        Ok(Ok(())) => {}
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(_) => panic!("two writers on two connections did not finish within {WRITERS_DEADLINE:?}"),
    }

    let store = Store::open(&path, Path::new("")).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let capture_seqs: Vec<i64> = conn
        .prepare("SELECT local_seq FROM capture_item")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(capture_seqs.len() as i64, ROUNDS);

    let outbox_seqs: Vec<i64> = store.list_outbox_operations().unwrap().into_iter().map(|(seq, ..)| seq).collect();
    let all: Vec<i64> = capture_seqs.iter().chain(outbox_seqs.iter()).copied().collect();
    let distinct: HashSet<i64> = all.iter().copied().collect();
    assert_eq!(distinct.len(), all.len(), "a local_seq was handed out twice across the two connections");

    for id in meeting_ids {
        let m = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(m.status, MeetingStatus::Completed);
        assert_eq!(m.summary_tldr.as_deref(), Some("tl;dr"));
    }
}

#[test]
fn a_second_store_can_share_the_first_stores_embedder() {
    let (_dir, path) = temp_db("embedder.sqlite");
    let first = Store::open(&path, Path::new("")).unwrap();
    let second = Store::open_with_embedder(&path, first.embedder()).unwrap();
    assert!(Arc::ptr_eq(&first.embedder(), &second.embedder()));
}
