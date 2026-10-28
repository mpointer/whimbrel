//! NFR-REL-1: "crash -> auto-restart; no more than the in-flight batch of captures
//! lost (WAL journaling)." This doesn't test the auto-restart/supervision part (an
//! OS/launchd concern, out of scope for a pure Rust test) — it tests the storage
//! guarantee underneath it: a real, hard process kill (SIGKILL, not a clean close)
//! mid-write must not corrupt the database, and everything committed before the kill
//! must survive.
//!
//! An integration test (not a unit test inside src/lib.rs) specifically because
//! `CARGO_BIN_EXE_<name>` — the path to the `crash_writer` helper binary
//! (src/bin/crash_writer.rs) — is only populated for tests under tests/.

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use memory::Store;

#[test]
fn wal_survives_a_hard_kill_mid_write() {
    let dir = std::env::temp_dir().join(format!(
        "whimbrel-crash-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("crash.sqlite");

    let mut child = Command::new(env!("CARGO_BIN_EXE_crash_writer"))
        .arg(&db_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn crash_writer");

    // Wait for crash_writer's "ready" sentinel — proof that schema creation (FTS5/vec0
    // virtual tables have their own commit overhead) is done and real rows have
    // committed — rather than a fixed sleep. A fixed sleep raced system load: on a
    // contended machine (e.g. running the whole workspace's tests in parallel) it could
    // fire before any row committed, so `survived > 0` below would flake.
    let ready_path = db_path.with_extension("ready");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !ready_path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "crash_writer never became ready within 30s"
        );
        thread::sleep(Duration::from_millis(10));
    }

    // SIGKILL: no chance to run destructors, flush buffers, or close the DB cleanly —
    // this is the scenario WAL mode is supposed to survive.
    child.kill().expect("failed to kill crash_writer");
    child.wait().expect("failed to reap crash_writer");

    // Reopening must not fail, and integrity_check must not report corruption. No real
    // models_dir needed — this test only checks WAL durability, not embeddings.
    let store =
        Store::open(&db_path, Path::new("")).expect("store failed to reopen after hard kill");
    let integrity = store
        .integrity_check()
        .expect("integrity_check query failed");
    assert_eq!(
        integrity, "ok",
        "database corrupted after hard kill mid-write"
    );

    let survived = store.count().expect("count query failed after hard kill");
    assert!(
        survived > 0,
        "expected at least some committed rows to survive the crash"
    );

    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}
