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
    raw.execute_batch(
        "BEGIN IMMEDIATE; UPDATE sync_local_seq_counter SET next_seq = next_seq WHERE id = 1;",
    )
    .unwrap();
    let committer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        raw.execute_batch("COMMIT").unwrap();
    });

    let started = std::time::Instant::now();
    let second = Store::open(&path, Path::new(""));
    committer.join().unwrap();
    assert!(second.is_ok(), "{:?}", second.err());
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(250),
        "the open waited for the commit"
    );
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
        .map(|i| {
            main_store
                .insert_meeting(&meeting(i, MeetingStatus::Recording))
                .unwrap()
        })
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
                store
                    .complete_meeting(*id, 2_000 + i, None)
                    .expect("complete_meeting");
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
                store
                    .set_meeting_title_if_placeholder(*id, "Generated")
                    .expect("retitle");
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
        Err(_) => {
            panic!("two writers on two connections did not finish within {WRITERS_DEADLINE:?}")
        }
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

    let outbox_seqs: Vec<i64> = store
        .list_outbox_operations()
        .unwrap()
        .into_iter()
        .map(|(seq, ..)| seq)
        .collect();
    let all: Vec<i64> = capture_seqs
        .iter()
        .chain(outbox_seqs.iter())
        .copied()
        .collect();
    let distinct: HashSet<i64> = all.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        all.len(),
        "a local_seq was handed out twice across the two connections"
    );

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

/// The app, its slow lane and the MCP server can open the same database at once.
/// Open recreates some triggers (drop, then create); if two opens interleave those
/// steps, the second create fails with "trigger ... already exists" and that open
/// fails. Repeated because the window is small.
#[test]
fn simultaneous_opens_of_one_database_all_succeed() {
    const OPENERS: usize = 4;
    for round in 0..25 {
        let (_dir, path) = temp_db(&format!("open-race-{round}.sqlite"));
        Store::open(&path, Path::new("")).unwrap();
        let barrier = Arc::new(Barrier::new(OPENERS));
        let handles: Vec<_> = (0..OPENERS)
            .map(|_| {
                let (path, barrier) = (path.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    Store::open(&path, Path::new(""))
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })
            })
            .collect();
        for h in handles {
            if let Err(e) = h.join().unwrap() {
                panic!("round {round}: an open failed: {e}");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// The Mac app's background lane (issue #850): a third connection runs the long jobs (content
// sync, screen-text ingest, retention) while the screens write on the main one. These tests
// pin what the single queue used to give for free.

fn capture(i: i64, text: &str) -> NewCaptureItem {
    NewCaptureItem {
        device_id: DEVICE.to_string(),
        ts_unix_ms: 1_000 + i,
        app_bundle_id: format!("app-{}", i % 5),
        window_title: "w".to_string(),
        url: None,
        text: text.to_string(),
        // Distinct per row so the consecutive-duplicate check never drops one.
        hash: i,
        source: "uia".to_string(),
    }
}

fn open_with_device(path: &Path) -> Store {
    let s = Store::open(path, Path::new("")).unwrap();
    s.set_own_device_id(DEVICE);
    s
}

/// Invariant (a): a `local_seq` is taken and its row written in one transaction, so across
/// connections no two rows share one, and rows commit in `local_seq` order (row id order here).
#[test]
fn local_seq_is_unique_and_in_commit_order_across_connections() {
    const PER_THREAD: i64 = 40;
    let (_dir, path) = temp_db("seq-order.sqlite");
    drop(open_with_device(&path));
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..3i64)
        .map(|t| {
            let (path, barrier) = (path.clone(), barrier.clone());
            std::thread::spawn(move || {
                let s = open_with_device(&path);
                barrier.wait();
                for i in 0..PER_THREAD {
                    let n = t * 1_000 + i;
                    match t {
                        0 => {
                            s.insert_capture_item(&capture(n, &format!("text {n}"))).unwrap();
                        }
                        1 => {
                            s.insert_meeting(&meeting(n, MeetingStatus::Recording)).unwrap();
                        }
                        _ => {
                            s.insert_capture_item(&capture(n, &format!("other {n}"))).unwrap();
                        }
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    let col = |sql: &str| -> Vec<i64> {
        conn.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
    };
    let captures = col("SELECT local_seq FROM capture_item ORDER BY id");
    let items = col("SELECT local_seq FROM items WHERE kind = 'capture' ORDER BY id");
    let meetings = col("SELECT local_seq FROM meeting ORDER BY id");
    assert_eq!(captures.len() as i64, 2 * PER_THREAD);
    assert_eq!(items, captures, "the items mirror carries the capture's own local_seq");
    for (name, seqs) in [("capture_item", &captures), ("meeting", &meetings)] {
        assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{name}: local_seq out of commit order: {seqs:?}");
    }
    let mut all: Vec<i64> = captures.iter().chain(meetings.iter()).copied().collect();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len() as i64, 3 * PER_THREAD, "a local_seq was handed out twice");
}

/// Invariant (b): a push round on the background connection, running while the main one writes,
/// never advances the cursor past a row that commits later. Every row ends up pushed.
#[test]
fn the_push_cursor_never_skips_a_row_another_connection_commits_late() {
    const ROWS: i64 = 120;
    let (_dir, path) = temp_db("push-gap.sqlite");
    drop(open_with_device(&path));
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Every connection is open, with the apps' lock wait, before any writer starts. An open takes
    // the write lock, and on a loaded Windows runner one that raced the tight insert loops below
    // waited past the default five seconds ("database is locked"). Opens under load are covered by
    // `simultaneous_opens_of_one_database_all_succeed`; this test is about the cursor.
    let ready = Arc::new(Barrier::new(3));

    let writer = |offset: i64| {
        let (path, done, ready) = (path.clone(), done.clone(), ready.clone());
        std::thread::spawn(move || {
            let s = open_with_device(&path);
            s.set_busy_timeout(Duration::from_secs(30)).unwrap();
            ready.wait();
            for i in 0..ROWS {
                let n = offset + i;
                s.insert_capture_item(&capture(n, &format!("row {n}"))).unwrap();
            }
            drop(done);
        })
    };
    let w1 = writer(0);
    let w2 = writer(10_000);

    // The background lane's round, as `sync_engine::content::push_content` runs it.
    let pusher = {
        let (path, done, ready) = (path.clone(), done.clone(), ready.clone());
        std::thread::spawn(move || {
            let s = open_with_device(&path);
            s.set_busy_timeout(Duration::from_secs(30)).unwrap();
            ready.wait();
            let mut pushed: HashSet<i64> = HashSet::new();
            let mut final_pass = false;
            loop {
                let cursor = s.get_sync_push_cursor().unwrap();
                let rows = s.with_read_snapshot(|s| s.unpushed_item_rows(DEVICE, cursor)).unwrap();
                let mut highest = cursor;
                for r in rows {
                    if let memory::SyncableRow::Item { local_seq, .. } = r {
                        pushed.insert(local_seq);
                        highest = highest.max(local_seq);
                    }
                }
                s.set_sync_push_cursor(highest).unwrap();
                if final_pass {
                    return pushed;
                }
                final_pass = done.load(std::sync::atomic::Ordering::SeqCst);
            }
        })
    };
    w1.join().unwrap();
    w2.join().unwrap();
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    let pushed = pusher.join().unwrap();

    let conn = rusqlite::Connection::open(&path).unwrap();
    let committed: Vec<i64> = conn
        .prepare("SELECT local_seq FROM items WHERE kind = 'capture'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(committed.len() as i64, 2 * ROWS);
    let missed: Vec<i64> = committed.iter().copied().filter(|s| !pushed.contains(s)).collect();
    assert!(missed.is_empty(), "rows never pushed (a silent sync gap): {missed:?}");
}

/// A read snapshot holds one view of the database: a row another connection commits while it is
/// open is not seen by a later read in the same snapshot, but is by the next one.
#[test]
fn a_read_snapshot_sees_one_commit_point() {
    let (_dir, path) = temp_db("snapshot.sqlite");
    let reader = open_with_device(&path);
    let writer = open_with_device(&path);
    writer.insert_capture_item(&capture(1, "first")).unwrap();
    let (before, after) = reader
        .with_read_snapshot(|r| {
            let before = r.unpushed_item_rows(DEVICE, 0)?.len();
            writer.insert_capture_item(&capture(2, "second")).unwrap();
            Ok((before, r.unpushed_item_rows(DEVICE, 0)?.len()))
        })
        .unwrap();
    assert_eq!((before, after), (1, 1), "the second read still sees the first commit point");
    assert_eq!(reader.unpushed_item_rows(DEVICE, 0).unwrap().len(), 2);
}

/// Invariant (c): the exclusion queue lives in the database, so a cached excluded-material
/// set on the main connection notices ingest on the background one (the capture triggers note
/// the row; the generation does not move).
#[test]
fn the_main_connections_exclusion_cache_notices_ingest_on_another_connection() {
    let (_dir, path) = temp_db("exclusion.sqlite");
    let main_store = open_with_device(&path);
    let background = open_with_device(&path);
    let acme = main_store.create_seed_topic("Acme", &[], None).unwrap();
    main_store
        .set_client_control(
            &acme.key,
            &memory::topics::client_controls::ClientControl { exclude_hosted: true, keep_months: None },
        )
        .unwrap();
    let names = |t: &str| t.to_lowercase().contains("acme");
    let first = main_store.excluded_material_with(&names).unwrap();
    let generation = main_store.exclusion_generation().unwrap();

    background.insert_capture_item(&capture(1, "Acme pricing notes")).unwrap();

    assert_eq!(main_store.exclusion_generation().unwrap(), generation, "ingest does not move the generation");
    let second = main_store.excluded_material_with(&names).unwrap();
    assert!(
        second.items.len() > first.items.len(),
        "the new capture naming Acme is excluded on the main connection ({} -> {})",
        first.items.len(),
        second.items.len()
    );
}

/// Every connection starts at rusqlite's 5 s; the shell can raise it, and the exclusion code's
/// best-effort write puts back whatever was set rather than a fixed 5 s.
#[test]
fn the_busy_timeout_defaults_to_five_seconds_and_can_be_raised() {
    let (_dir, path) = temp_db("busy.sqlite");
    let s = open_with_device(&path);
    assert_eq!(s.busy_timeout_ms().unwrap(), 5_000);
    s.set_busy_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(s.busy_timeout_ms().unwrap(), 30_000);
}

/// A write that must not be dropped waits out another connection's long transaction.
#[test]
fn retry_when_busy_outlasts_a_lock_held_past_the_busy_timeout() {
    let (_dir, path) = temp_db("retry.sqlite");
    let s = open_with_device(&path);
    s.set_busy_timeout(Duration::from_millis(100)).unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("BEGIN IMMEDIATE").unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        raw.execute_batch("COMMIT").unwrap();
    });
    assert!(s.insert_meeting(&meeting(1, MeetingStatus::Recording)).is_err(), "one try gives up after 100 ms");
    let id = memory::retry_when_busy(Duration::from_secs(10), || s.insert_meeting(&meeting(2, MeetingStatus::Recording)));
    releaser.join().unwrap();
    assert!(id.is_ok(), "{:?}", id.err());
}

/// Retention deletes captures in batches, so the write lock is free between them: another
/// connection's write lands while expired captures still remain (with one big DELETE the
/// remaining count only ever reads all or none), and the FTS and `items` mirror stay in step.
#[test]
fn a_large_retention_sweep_lets_another_connection_write_between_batches() {
    let (_dir, path) = temp_db("retention.sqlite");
    let sweeper = open_with_device(&path);
    // Both sides get the apps' 30 s (both apps set it on every connection).
    sweeper.set_busy_timeout(Duration::from_secs(30)).unwrap();
    let n = 5_000; // five batches
    for i in 0..n {
        sweeper.insert_capture_item(&capture(i, &format!("expired{i} alpha"))).unwrap();
    }
    let now = 10 * 24 * 3_600_000i64;
    let mut fresh = capture(n + 1, "fresh survivor beta");
    fresh.ts_unix_ms = now - 1_000;
    sweeper.insert_capture_item(&fresh).unwrap();
    let policy = memory::RetentionPolicy { capture_item_days: Some(1), meeting_days: None, conversation_days: None };
    let writer = open_with_device(&path);
    // The apps give every connection 30 s; with the 5 s default a slow runner (debug build, tests
    // in parallel) can take longer than that over one batch and the write fails instead of waiting.
    writer.set_busy_timeout(Duration::from_secs(30)).unwrap();
    let reader = rusqlite::Connection::open(&path).unwrap();
    let remaining = |c: &rusqlite::Connection| -> i64 {
        c.query_row("SELECT COUNT(*) FROM capture_item WHERE ts_unix_ms < ?1", [now - 24 * 3_600_000], |r| r.get(0)).unwrap()
    };
    // Timeline, printed only on failure: when each batch committed (a sampler on its own
    // connection, polling every 20 ms) and when each write started, how long it waited, and
    // what it then read.
    let t0 = std::time::Instant::now();
    let ms = move || t0.elapsed().as_millis();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sampler = {
        let (stop, path) = (stop.clone(), path.clone());
        std::thread::spawn(move || {
            let c = rusqlite::Connection::open(&path).unwrap();
            let mut seen = Vec::new();
            let mut last = -1;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let left: i64 = c
                    .query_row("SELECT COUNT(*) FROM capture_item WHERE ts_unix_ms < ?1", [now - 24 * 3_600_000], |r| r.get(0))
                    .unwrap_or(-1);
                if left != last {
                    seen.push(format!("{}ms left={left}", ms()));
                    last = left;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            seen
        })
    };
    let handle = std::thread::spawn(move || sweeper.apply_retention(&policy, now).unwrap());
    let mut landed_mid_sweep = 0;
    let mut writes = Vec::new();
    let mut i = 0;
    while !handle.is_finished() {
        let started = ms();
        writer.insert_meeting(&meeting(i, MeetingStatus::Recording)).unwrap();
        let waited = ms() - started;
        i += 1;
        let left = remaining(&reader);
        writes.push(format!("{started}ms waited={waited}ms left={left}"));
        if left > 0 && left < n {
            landed_mid_sweep += 1;
        }
        // App writes come apart, never back to back. Without a pause this loop takes the lock again
        // the moment it lets go, and the sweep's busy handler (which sleeps between tries) can miss
        // every gap: on the Windows runner the sweep itself then failed with "database is locked".
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = handle.join().unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let batches = sampler.join().unwrap();
    assert_eq!(result.capture_items_deleted as i64, n);
    assert!(
        landed_mid_sweep >= 1,
        "no write landed while the sweep was partly done: the delete held the lock throughout\n\
         sweep done at {}ms\nbatches: {batches:?}\nwrites: {writes:?}",
        ms()
    );
    assert_eq!(remaining(&reader), 0);
    let count = |sql: &str| -> i64 { reader.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM items WHERE kind = 'capture'"), 1, "only the fresh mirror is left");
    assert_eq!(count("SELECT COUNT(*) FROM capture_item_fts WHERE capture_item_fts MATCH 'alpha'"), 0, "no stale FTS rows");
    assert_eq!(count("SELECT COUNT(*) FROM capture_item_fts WHERE capture_item_fts MATCH 'survivor'"), 1);
}

/// A recording start on one connection (30 s busy timeout, as the apps set) lands while another
/// connection is still pruning a large backlog of kept word fingerprints (#850 review, I2). The
/// prune commits a batch at a time; without a pause between batches it takes the lock again the
/// moment it lets go, the waiting insert's busy handler (which sleeps between tries) misses every
/// gap, and the start only lands once the whole prune is done.
#[test]
fn a_recording_start_lands_while_another_connection_prunes_word_fingerprints() {
    let (_dir, path) = temp_db("prune-starve.sqlite");
    let pruner = open_with_device(&path);
    pruner.set_busy_timeout(Duration::from_secs(30)).unwrap();
    let starter = open_with_device(&path);
    starter.set_busy_timeout(Duration::from_secs(30)).unwrap();
    // Many batches: the prune runs well past the moment the start asks.
    let rows = 20_000i64;
    let words: Vec<(i64, Vec<u32>)> = (0..rows).map(|h| (h, vec![1, 2, 3])).collect();
    pruner.mark_clean_text("an answer", "stamp", &words).unwrap();
    let reader = rusqlite::Connection::open(&path).unwrap();
    // How far the prune had got when the start committed, recorded inside the insert's own
    // transaction: a test thread descheduled after the insert cannot skew it.
    reader
        .execute_batch(
            "CREATE TABLE prune_probe (left INTEGER NOT NULL); \
             CREATE TRIGGER prune_probe_on_start AFTER INSERT ON meeting \
             BEGIN INSERT INTO prune_probe SELECT COUNT(*) FROM input_words; END;",
        )
        .unwrap();
    let left = |c: &rusqlite::Connection| -> i64 { c.query_row("SELECT COUNT(*) FROM input_words", [], |r| r.get(0)).unwrap() };
    let t0 = std::time::Instant::now();
    let looper = std::thread::spawn(move || {
        let n = pruner.prune_input_words_before(i64::MAX).unwrap();
        (n, t0.elapsed())
    });
    // Ask once the prune is under way (its first batch has committed).
    while left(&reader) == rows && !looper.is_finished() {
        std::thread::sleep(Duration::from_millis(1));
    }
    let asked = t0.elapsed();
    let inserted = starter.insert_meeting(&meeting(1, MeetingStatus::Recording));
    let waited = t0.elapsed() - asked;
    let (pruned, prune_took) = looper.join().unwrap();
    assert!(inserted.is_ok(), "{:?}", inserted.err());
    let remaining: i64 = reader.query_row("SELECT left FROM prune_probe", [], |r| r.get(0)).unwrap();
    assert_eq!(pruned as i64, rows);
    assert!(
        remaining > 0,
        "the start only landed after the whole prune (asked at {asked:?}, waited {waited:?}, prune took {prune_took:?})"
    );
    // Generous for a slow runner: with the pause the insert gets in within a batch or two.
    assert!(waited < Duration::from_secs(8), "the recording start waited {waited:?} behind the prune");
}
