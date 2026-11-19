//! The user-confirmed recovery actions behind the launch screens (Task 11a fix round 1):
//! `restore_rollback_copy`, `reset_encryption_status`, `delete_plain_backup`. Files only;
//! the "encrypted" database is encrypted-looking bytes, so this runs with or without SQLCipher.

use std::path::{Path, PathBuf};

use api::encrypt_migration::{
    delete_plain_backup, reset_encryption_status, restore_rollback_copy, restore_rollback_copy_with_hooks, status,
    FailAt, FailureKind, Hooks, MigrationError, MigrationPaths, MigrationStatus, RestoreWarning,
};
use memory::{db_file_state, DbFileState};

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn plain_bytes(dir: &Path) -> Vec<u8> {
    let p = dir.join("scratch-plain.sqlite");
    drop(memory::Store::open(&p, Path::new("")).unwrap());
    let b = std::fs::read(&p).unwrap();
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", p.display()));
    }
    b
}

fn encrypted_bytes() -> Vec<u8> {
    (0..16384u32).map(|i| (i.wrapping_mul(31) % 251) as u8 | 0x80).collect()
}

fn state_json(phase: &str, attempts: u32) -> String {
    format!(
        r#"{{"version":2,"phase":{phase},"started_unix_ms":1,"updated_unix_ms":1,"swapped_len":null,"swapped_mtime_ms":null,"swapped_head_sha256":null,"failed_attempts":{attempts},"last_failure":"copy"}}"#
    )
}

fn setup() -> (tempfile::TempDir, MigrationPaths, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let paths = MigrationPaths::beside(&db);
    (dir, paths, db)
}

#[test]
fn restore_rollback_copy_puts_the_backup_back_and_keeps_the_encrypted_copy() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(paths.encrypted_tmp(), encrypted_bytes()).unwrap();
    std::fs::write(&paths.state_file, state_json("null", 1)).unwrap();
    restore_rollback_copy(&paths).unwrap();
    assert_eq!(std::fs::read(&db).unwrap(), plain);
    assert!(!paths.plain_backup().exists());
    assert_eq!(std::fs::read(paths.encrypted_tmp()).unwrap(), encrypted_bytes(), ".enc.tmp is never deleted");
    assert_eq!(status(&paths), MigrationStatus::Idle { failed_attempts: 1, last_failure: Some(FailureKind::Copy) });
    assert_eq!(db_file_state(&db).unwrap(), DbFileState::Plain);
}

#[test]
fn restore_rollback_copy_after_done_leaves_no_run_and_moves_an_unreadable_state_aside() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(&paths.state_file, state_json("\"done\"", 0)).unwrap();
    restore_rollback_copy(&paths).unwrap();
    assert_eq!(status(&paths), MigrationStatus::None);
    assert!(db.exists());

    let (dir, paths, db) = setup();
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(&paths.state_file, b"garbage").unwrap();
    restore_rollback_copy(&paths).unwrap();
    assert!(db.exists());
    assert_eq!(status(&paths), MigrationStatus::None);
    assert!(listing(dir.path()).iter().any(|n| n.starts_with("encryption-migration.json.unreadable-")), "kept aside");
}

#[test]
fn restore_rollback_copy_refuses_and_changes_nothing() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    // The database exists.
    std::fs::write(&db, encrypted_bytes()).unwrap();
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    let before = listing(dir.path());
    assert!(restore_rollback_copy(&paths).is_err());
    assert_eq!(listing(dir.path()), before);
    assert_eq!(std::fs::read(&db).unwrap(), encrypted_bytes());
    // No backup.
    let (dir, paths, _db) = setup();
    assert!(restore_rollback_copy(&paths).is_err());
    assert!(listing(dir.path()).is_empty());
    // A run is recorded as unfinished: resume decides, not this button.
    let (dir, paths, _db) = setup();
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(&paths.state_file, state_json("\"verified\"", 0)).unwrap();
    let before = listing(dir.path());
    assert!(restore_rollback_copy(&paths).is_err());
    assert_eq!(listing(dir.path()), before);
    // The backup has sidecars of its own.
    let (dir, paths, _db) = setup();
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(format!("{}-wal", paths.plain_backup().display()), b"wal").unwrap();
    let before = listing(dir.path());
    assert!(restore_rollback_copy(&paths).is_err());
    assert_eq!(listing(dir.path()), before);
}

/// The likely shape of this screen: the store had the encrypted file open in WAL mode and
/// the main file was removed outside the app, leaving `-wal`/`-shm` at the database path.
#[test]
fn restore_rollback_copy_moves_stray_sidecars_aside_and_keeps_them() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(format!("{}-wal", db.display()), b"stray wal").unwrap();
    std::fs::write(format!("{}-shm", db.display()), b"stray shm").unwrap();
    let out = restore_rollback_copy(&paths).unwrap();
    assert_eq!(std::fs::read(&db).unwrap(), plain);
    assert_eq!(out.warning, None);
    assert_eq!(out.moved_aside.len(), 2, "{out:?}");
    let names = listing(dir.path());
    let wal = names.iter().find(|n| n.starts_with("core-memory.sqlite.foreign-") && n.ends_with("-wal")).unwrap();
    let shm = names.iter().find(|n| n.starts_with("core-memory.sqlite.foreign-") && n.ends_with("-shm")).unwrap();
    assert_eq!(std::fs::read(dir.path().join(wal)).unwrap(), b"stray wal");
    assert_eq!(std::fs::read(dir.path().join(shm)).unwrap(), b"stray shm");
    assert!(!Path::new(&format!("{}-wal", db.display())).exists());
    // The restored file opens with all its rows (no foreign WAL replayed into it).
    drop(memory::Store::open(&db, Path::new("")).unwrap());
}

#[test]
fn restore_rollback_copy_refuses_when_a_stray_sidecar_cannot_be_moved() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(format!("{}-wal", db.display()), b"stray wal").unwrap();
    let before = listing(dir.path());
    let hooks = Hooks { fail_at: Some(FailAt::SidecarMoveError), ..Default::default() };
    assert!(matches!(restore_rollback_copy_with_hooks(&paths, &hooks), Err(MigrationError::Refused(_))));
    assert_eq!(listing(dir.path()), before);
    assert_eq!(std::fs::read(paths.plain_backup()).unwrap(), plain);
}

/// The migration runs at launch before any store opens. If this process has the database
/// open, `run` refuses with a typed reason before touching anything (its lock probe and
/// header reads must never close a descriptor of a file this process has open).
#[test]
fn run_refuses_while_this_process_has_the_database_open() {
    let (dir, paths, db) = setup();
    let store = memory::Store::open(&db, Path::new("")).unwrap();
    let before = listing(dir.path());
    let key = memory::DbKey::new([1; 32]);
    let r = api::encrypt_migration::run(&paths, false, &key, &|_, _| Ok(()));
    assert!(
        matches!(r, Err(MigrationError::Skipped(api::encrypt_migration::SkipReason::OpenInThisProcess))),
        "{r:?}"
    );
    assert_eq!(listing(dir.path()), before, "nothing changed");
    // The precheck (Settings) names the same reason, without touching the file.
    assert_eq!(api::encrypt_migration::precheck(&paths, false), Err(api::encrypt_migration::SkipReason::OpenInThisProcess));
    drop(store);
}

/// One stray sidecar was moved aside and the next move fails: something did change, so the
/// error must not be "Refused: nothing was changed". It carries the names moved aside.
#[test]
fn restore_rollback_copy_reports_a_partial_sidecar_move_as_partly_done() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(format!("{}-wal", db.display()), b"stray wal").unwrap();
    std::fs::write(format!("{}-shm", db.display()), b"stray shm").unwrap();
    let hooks = Hooks { fail_at: Some(FailAt::LaterSidecarMoveError), ..Default::default() };
    match restore_rollback_copy_with_hooks(&paths, &hooks) {
        Err(MigrationError::PartlyDone { moved_aside, .. }) => {
            assert_eq!(moved_aside.len(), 1, "{moved_aside:?}");
            assert!(moved_aside[0].starts_with("core-memory.sqlite.foreign-") && moved_aside[0].ends_with("-wal"));
            assert_eq!(std::fs::read(dir.path().join(&moved_aside[0])).unwrap(), b"stray wal", "kept, never deleted");
        }
        other => panic!("expected PartlyDone, got {other:?}"),
    }
    // The copy did not move; the second sidecar is still at the database path.
    assert!(!db.exists());
    assert_eq!(std::fs::read(paths.plain_backup()).unwrap(), plain);
    assert_eq!(std::fs::read(format!("{}-shm", db.display())).unwrap(), b"stray shm");
    // Pressing the button again finishes it.
    let out = restore_rollback_copy(&paths).unwrap();
    assert_eq!(out.moved_aside.len(), 1);
    assert_eq!(std::fs::read(&db).unwrap(), plain);
    // The first-move failure is still a plain refusal (nothing changed).
    let (dir, paths, db) = setup();
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(format!("{}-wal", db.display()), b"w").unwrap();
    std::fs::write(format!("{}-shm", db.display()), b"s").unwrap();
    let before = listing(dir.path());
    let hooks = Hooks { fail_at: Some(FailAt::SidecarMoveError), ..Default::default() };
    assert!(matches!(restore_rollback_copy_with_hooks(&paths, &hooks), Err(MigrationError::Refused(_))));
    assert_eq!(listing(dir.path()), before);
}

/// The copy is back but the state file could not be rewritten: a warning naming that, not
/// an error and not the "state not moved aside" warning.
#[test]
fn restore_rollback_copy_reports_a_failed_state_rewrite_as_a_warning() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(&paths.state_file, state_json("\"done\"", 0)).unwrap();
    let hooks = Hooks { fail_at: Some(FailAt::ClearRunError), ..Default::default() };
    let out = restore_rollback_copy_with_hooks(&paths, &hooks).unwrap();
    assert_eq!(std::fs::read(&db).unwrap(), plain);
    assert!(matches!(out.warning, Some(RestoreWarning::StateNotCleared(_))), "{out:?}");
    let _ = dir;
}

/// Once the copy is back the restore is done: a failure to move an unreadable state file
/// aside is a warning, not an error (a retry would be refused: the database exists).
#[test]
fn restore_rollback_copy_reports_a_late_state_failure_as_a_warning() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(&paths.state_file, b"garbage").unwrap();
    let hooks = Hooks { fail_at: Some(FailAt::StateAsideError), ..Default::default() };
    let out = restore_rollback_copy_with_hooks(&paths, &hooks).unwrap();
    assert_eq!(std::fs::read(&db).unwrap(), plain);
    assert!(matches!(out.warning, Some(RestoreWarning::StateNotMovedAside(_))), "{out:?}");
    assert_eq!(status(&paths), MigrationStatus::Unreadable);
    // The way out afterwards: reset the status.
    assert!(reset_encryption_status(&paths).is_ok());
    let _ = dir;
}

/// An unfinished run whose database and both copies are gone: real `resume` fails every
/// time, so the only way out is resetting the status (moved aside, never deleted).
#[test]
fn reset_encryption_status_clears_an_unfinished_run_with_nothing_left() {
    let (dir, paths, _db) = setup();
    std::fs::write(&paths.state_file, state_json("\"verified\"", 0)).unwrap();
    assert!(api::encrypt_migration::resume(&paths).is_err(), "resume cannot help");
    assert!(api::encrypt_migration::resume(&paths).is_err(), "not on a retry either");
    let aside = reset_encryption_status(&paths).unwrap();
    assert_eq!(status(&paths), MigrationStatus::None);
    assert!(dir.path().join(aside).exists());
    // Not while a copy exists.
    let (_dir, paths, _db) = setup();
    std::fs::write(&paths.state_file, state_json("\"verified\"", 0)).unwrap();
    std::fs::write(paths.encrypted_tmp(), b"x").unwrap();
    assert!(reset_encryption_status(&paths).is_err());
    let (_dir, paths, _db) = setup();
    std::fs::write(&paths.state_file, state_json("\"verified\"", 0)).unwrap();
    std::fs::write(paths.plain_backup(), b"x").unwrap();
    assert!(reset_encryption_status(&paths).is_err());
}

#[test]
fn delete_plain_backup_refuses_while_the_state_file_is_unreadable() {
    let (dir, paths, db) = setup();
    std::fs::write(&db, encrypted_bytes()).unwrap();
    std::fs::write(paths.plain_backup(), plain_bytes(dir.path())).unwrap();
    std::fs::write(&paths.state_file, b"garbage").unwrap();
    assert!(matches!(delete_plain_backup(&paths, true), Err(MigrationError::Refused(_))));
    assert!(paths.plain_backup().exists());
    reset_encryption_status(&paths).unwrap();
    assert!(delete_plain_backup(&paths, true).is_ok());
    assert!(!paths.plain_backup().exists());
}

#[test]
fn reset_encryption_status_moves_an_unreadable_state_aside_only_when_safe() {
    for db_bytes in [Some(encrypted_bytes()), None] {
        let (dir, paths, db) = setup();
        if let Some(b) = &db_bytes {
            std::fs::write(&db, b).unwrap();
        }
        std::fs::write(&paths.state_file, b"{\"version\":3}").unwrap();
        assert_eq!(status(&paths), MigrationStatus::Unreadable);
        let aside = reset_encryption_status(&paths).unwrap();
        assert!(aside.starts_with("encryption-migration.json.unreadable-"), "{aside}");
        assert_eq!(std::fs::read(dir.path().join(&aside)).unwrap(), b"{\"version\":3}");
        assert_eq!(status(&paths), MigrationStatus::None);
    }
    // Plain database too.
    let (dir, paths, db) = setup();
    std::fs::write(&db, plain_bytes(dir.path())).unwrap();
    std::fs::write(&paths.state_file, b"x").unwrap();
    assert!(reset_encryption_status(&paths).is_ok());

    // Refusals: an encrypted copy beside it; a missing database with its backup; a readable
    // or absent state file; a damaged database.
    let refuse = |prepare: &dyn Fn(&Path, &MigrationPaths)| {
        let (dir, paths, _db) = setup();
        prepare(dir.path(), &paths);
        let before = listing(dir.path());
        assert!(reset_encryption_status(&paths).is_err());
        assert_eq!(listing(dir.path()), before);
    };
    refuse(&|_, p| {
        std::fs::write(&p.db, encrypted_bytes()).unwrap();
        std::fs::write(p.encrypted_tmp(), b"x").unwrap();
        std::fs::write(&p.state_file, b"x").unwrap();
    });
    refuse(&|_, p| {
        std::fs::write(p.plain_backup(), b"x").unwrap();
        std::fs::write(&p.state_file, b"x").unwrap();
    });
    refuse(&|_, p| {
        std::fs::write(&p.db, encrypted_bytes()).unwrap();
        std::fs::write(&p.state_file, state_json("null", 1)).unwrap();
    });
    refuse(&|_, p| std::fs::write(&p.db, encrypted_bytes()).unwrap());
    refuse(&|_, p| {
        std::fs::write(&p.db, b"short").unwrap();
        std::fs::write(&p.state_file, b"x").unwrap();
    });
}

#[test]
fn delete_plain_backup_needs_an_encrypted_database_opened_with_its_key() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    std::fs::write(&db, encrypted_bytes()).unwrap();
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(format!("{}-wal", paths.plain_backup().display()), b"w").unwrap();
    // Not opened with its key this launch: refused.
    assert!(delete_plain_backup(&paths, false).is_err());
    assert!(paths.plain_backup().exists());
    // Opened keyed: removed, with no Done record needed.
    let removed = delete_plain_backup(&paths, true).unwrap();
    assert_eq!(removed, vec!["core-memory.sqlite.plain.bak".to_string(), "core-memory.sqlite.plain.bak-wal".to_string()]);
    assert_eq!(listing(dir.path()), vec!["core-memory.sqlite".to_string()]);
}

#[test]
fn delete_plain_backup_refuses_on_a_plain_database() {
    let (dir, paths, db) = setup();
    let plain = plain_bytes(dir.path());
    std::fs::write(&db, &plain).unwrap();
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    assert!(delete_plain_backup(&paths, true).is_err());
    assert!(paths.plain_backup().exists());
    // Nor while a run is recorded as unfinished, nor with the database missing.
    let (_dir, paths, db) = setup();
    std::fs::write(&db, encrypted_bytes()).unwrap();
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    std::fs::write(&paths.state_file, state_json("\"swapped\"", 0)).unwrap();
    assert!(delete_plain_backup(&paths, true).is_err());
    assert!(paths.plain_backup().exists());
    let (_dir, paths, _db) = setup();
    std::fs::write(paths.plain_backup(), &plain).unwrap();
    assert!(delete_plain_backup(&paths, true).is_err());
    assert!(paths.plain_backup().exists());
}
