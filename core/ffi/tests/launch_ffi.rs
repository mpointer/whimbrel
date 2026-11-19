//! The C ABI over the shared launch decision (X4, Task 11): pinned step codes (in Rust and
//! in the header), `whimbrel_launch_observe_files`, `whimbrel_launch_decision` and
//! `whimbrel_fast_search_available`. Pure: runs with or without SQLCipher.

use std::ffi::{CStr, CString};
use std::ptr;

use api::launch_decision::{DamageReason, KeySource, LaunchAction, RecoveryReason, SkipNote};
use serde_json::{json, Value};
use whimbrel_core::*;

fn take(s: *mut std::ffi::c_char) -> Option<Value> {
    if s.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(s) }.to_str().unwrap().to_string();
    whimbrel_core_free_string(s);
    Some(serde_json::from_str(&text).unwrap())
}

#[test]
fn launch_codes_are_pinned_in_rust_and_the_header() {
    use LaunchAction::*;
    let all: [(LaunchAction, i32, &str); 27] = [
        (OpenPlain, WHIMBREL_LAUNCH_OPEN_PLAIN, "WHIMBREL_LAUNCH_OPEN_PLAIN"),
        (OpenPlainRecordKeyUnavailable, WHIMBREL_LAUNCH_OPEN_PLAIN_RECORD_KEY_UNAVAILABLE, "WHIMBREL_LAUNCH_OPEN_PLAIN_RECORD_KEY_UNAVAILABLE"),
        (HardStopNoKeyStorage, WHIMBREL_LAUNCH_HARD_STOP_NO_KEY_STORAGE, "WHIMBREL_LAUNCH_HARD_STOP_NO_KEY_STORAGE"),
        (ShowKeyUnavailableScreen, WHIMBREL_LAUNCH_SHOW_KEY_UNAVAILABLE_SCREEN, "WHIMBREL_LAUNCH_SHOW_KEY_UNAVAILABLE_SCREEN"),
        (ShowRecoveryScreen { reason: RecoveryReason::NeedsKey }, WHIMBREL_LAUNCH_SHOW_RECOVERY_SCREEN, "WHIMBREL_LAUNCH_SHOW_RECOVERY_SCREEN"),
        (ShowMigrationInterrupted, WHIMBREL_LAUNCH_SHOW_MIGRATION_INTERRUPTED, "WHIMBREL_LAUNCH_SHOW_MIGRATION_INTERRUPTED"),
        (ShowDamaged { reason: DamageReason::Damaged }, WHIMBREL_LAUNCH_SHOW_DAMAGED, "WHIMBREL_LAUNCH_SHOW_DAMAGED"),
        (ShowFailedBefore { attempts: 2 }, WHIMBREL_LAUNCH_SHOW_FAILED_BEFORE, "WHIMBREL_LAUNCH_SHOW_FAILED_BEFORE"),
        (ShowMigrationSkipped { reason: SkipNote::Precheck }, WHIMBREL_LAUNCH_SHOW_MIGRATION_SKIPPED, "WHIMBREL_LAUNCH_SHOW_MIGRATION_SKIPPED"),
        (ShowMigrationFailed, WHIMBREL_LAUNCH_SHOW_MIGRATION_FAILED, "WHIMBREL_LAUNCH_SHOW_MIGRATION_FAILED"),
        (NewInstallOnboarding { reuse_pending_key: false }, WHIMBREL_LAUNCH_NEW_INSTALL_ONBOARDING, "WHIMBREL_LAUNCH_NEW_INSTALL_ONBOARDING"),
        (FinishUndoViaResume, WHIMBREL_LAUNCH_FINISH_UNDO_VIA_RESUME, "WHIMBREL_LAUNCH_FINISH_UNDO_VIA_RESUME"),
        (PrecheckMigration { recording: false }, WHIMBREL_LAUNCH_PRECHECK_MIGRATION, "WHIMBREL_LAUNCH_PRECHECK_MIGRATION"),
        (RunMigration { key_source: KeySource::DbKey, recording: false }, WHIMBREL_LAUNCH_RUN_MIGRATION, "WHIMBREL_LAUNCH_RUN_MIGRATION"),
        (OpenKeyed { key_source: KeySource::DbKey, cache_kib: None }, WHIMBREL_LAUNCH_OPEN_KEYED, "WHIMBREL_LAUNCH_OPEN_KEYED"),
        (PromotePendingKey, WHIMBREL_LAUNCH_PROMOTE_PENDING_KEY, "WHIMBREL_LAUNCH_PROMOTE_PENDING_KEY"),
        (DeletePendingKey, WHIMBREL_LAUNCH_DELETE_PENDING_KEY, "WHIMBREL_LAUNCH_DELETE_PENDING_KEY"),
        (ClearMigrateFlag, WHIMBREL_LAUNCH_CLEAR_MIGRATE_FLAG, "WHIMBREL_LAUNCH_CLEAR_MIGRATE_FLAG"),
        (StartCleanupCounter, WHIMBREL_LAUNCH_START_CLEANUP_COUNTER, "WHIMBREL_LAUNCH_START_CLEANUP_COUNTER"),
        (RecordGoodLaunch { count: 2 }, WHIMBREL_LAUNCH_RECORD_GOOD_LAUNCH, "WHIMBREL_LAUNCH_RECORD_GOOD_LAUNCH"),
        (CleanupPlainBackup { good_launches: 3 }, WHIMBREL_LAUNCH_CLEANUP_PLAIN_BACKUP, "WHIMBREL_LAUNCH_CLEANUP_PLAIN_BACKUP"),
        (ResetCleanupCounter, WHIMBREL_LAUNCH_RESET_CLEANUP_COUNTER, "WHIMBREL_LAUNCH_RESET_CLEANUP_COUNTER"),
        (StartMcpServer, WHIMBREL_LAUNCH_START_MCP_SERVER, "WHIMBREL_LAUNCH_START_MCP_SERVER"),
        (ShowEncryptionAvailable, WHIMBREL_LAUNCH_SHOW_ENCRYPTION_AVAILABLE, "WHIMBREL_LAUNCH_SHOW_ENCRYPTION_AVAILABLE"),
        (
            ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false },
            WHIMBREL_LAUNCH_SHOW_ENCRYPTED_FILE_MISSING,
            "WHIMBREL_LAUNCH_SHOW_ENCRYPTED_FILE_MISSING",
        ),
        (ShowPlainBackupKept, WHIMBREL_LAUNCH_SHOW_PLAIN_BACKUP_KEPT, "WHIMBREL_LAUNCH_SHOW_PLAIN_BACKUP_KEPT"),
        (NoticeLibraryMissing, WHIMBREL_LAUNCH_NOTICE_LIBRARY_MISSING, "WHIMBREL_LAUNCH_NOTICE_LIBRARY_MISSING"),
    ];
    let header = include_str!("../include/whimbrel_core.h");
    for (n, (action, constant, name)) in all.iter().enumerate() {
        assert_eq!(action.code(), n as i32, "{action:?}");
        assert_eq!(*constant, n as i32, "{name}");
        let line = format!("#define {name} ({n})");
        assert!(header.contains(&line), "header lacks `{line}`");
    }
}

#[test]
fn decision_round_trips_through_json() {
    let dir = tempfile::tempdir().unwrap();
    let db = CString::new(dir.path().join("core-memory.sqlite").to_str().unwrap()).unwrap();
    let files = take(whimbrel_launch_observe_files(db.as_ptr())).unwrap();
    assert_eq!(files["db_file"], "missing");
    assert_eq!(files["migration"], json!({"kind": "none"}));

    let input = json!({
        "files": files,
        "db_key": "absent",
        "pending_key": "absent",
        "migrate_at_next_launch": false,
        "recording": false,
        "settings": {"fast_search": true},
        "physical_ram_bytes": 34_359_738_368u64,
        "good_launches": 0,
        "this_launch": {"resume": "not_run", "migration": "not_tried", "db_key_rejected": false,
                        "pending_key_rejected": false, "key_store_failed": false}
    });
    let c = CString::new(input.to_string()).unwrap();
    let out = take(whimbrel_launch_decision(c.as_ptr())).unwrap();
    assert_eq!(
        out,
        json!({"steps": [{"code": WHIMBREL_LAUNCH_NEW_INSTALL_ONBOARDING, "action": "new_install_onboarding", "reuse_pending_key": false}]})
    );

    let mut with_key = input.clone();
    with_key["db_key"] = json!("found");
    let c = CString::new(with_key.to_string()).unwrap();
    let out = take(whimbrel_launch_decision(c.as_ptr())).unwrap();
    assert_eq!(out["steps"][0], json!({"code": 14, "action": "open_keyed", "key_source": "db_key", "cache_kib": 1_048_576}));
    assert_eq!(out["steps"][1], json!({"code": 22, "action": "start_mcp_server"}));
    // Nothing was created.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    // Pinned JSON of codes 6 and 24 as the shells read them.
    let mut no_access = input.clone();
    no_access["files"]["db_file"] = json!("no_access");
    let c = CString::new(no_access.to_string()).unwrap();
    let out = take(whimbrel_launch_decision(c.as_ptr())).unwrap();
    assert_eq!(out["steps"], json!([{"code": 6, "action": "show_damaged", "reason": "no_access"}]));
    let mut held = input.clone();
    held["files"]["db_file"] = json!("open_elsewhere");
    let c = CString::new(held.to_string()).unwrap();
    let out = take(whimbrel_launch_decision(c.as_ptr())).unwrap();
    assert_eq!(out["steps"], json!([{"code": 6, "action": "show_damaged", "reason": "open_elsewhere"}]));
    let mut bak = input.clone();
    bak["files"]["plain_backup_exists"] = json!(true);
    let c = CString::new(bak.to_string()).unwrap();
    let out = take(whimbrel_launch_decision(c.as_ptr())).unwrap();
    assert_eq!(
        out["steps"],
        json!([{"code": 24, "action": "show_encrypted_file_missing", "backup_present": true, "encrypted_copy_present": false}])
    );

    // A misspelt (camelCase) field or a missing this_launch is an error (NULL), never a
    // default that could make the shell's launch loop.
    let mut camel = input.clone();
    camel["thisLaunch"] = camel["this_launch"].take();
    camel.as_object_mut().unwrap().remove("this_launch");
    let c = CString::new(camel.to_string()).unwrap();
    assert!(whimbrel_launch_decision(c.as_ptr()).is_null());
    let mut typo = input.clone();
    typo["this_launch"]["dbKeyRejected"] = json!(true);
    let c = CString::new(typo.to_string()).unwrap();
    assert!(whimbrel_launch_decision(c.as_ptr()).is_null());
    let mut missing = input;
    missing.as_object_mut().unwrap().remove("this_launch");
    let c = CString::new(missing.to_string()).unwrap();
    assert!(whimbrel_launch_decision(c.as_ptr()).is_null());
}

#[test]
fn bad_input_is_null() {
    assert!(whimbrel_launch_decision(ptr::null()).is_null());
    assert!(whimbrel_launch_observe_files(ptr::null()).is_null());
    let garbage = CString::new("{\"files\": 3}").unwrap();
    assert!(whimbrel_launch_decision(garbage.as_ptr()).is_null());
}

#[test]
fn fast_search_offered_for_a_16_gb_computer() {
    const GIB: u64 = 1024 * 1024 * 1024;
    assert!(!whimbrel_fast_search_available(14 * GIB - 1));
    assert!(whimbrel_fast_search_available(14 * GIB));
    assert!(whimbrel_fast_search_available(16 * GIB));
}

/// Task 13c item 4: the "a store was opened here" marker over the C ABI. Mark after an open,
/// clear at the notice step and in Delete everything's flag cleanup; observe_files reports it.
#[test]
fn store_existed_marker_round_trips_through_the_c_abi() {
    let dir = tempfile::tempdir().unwrap();
    let db = CString::new(dir.path().join("core-memory.sqlite").to_str().unwrap()).unwrap();
    assert_eq!(take(whimbrel_launch_observe_files(db.as_ptr())).unwrap()["store_existed"], false);
    assert!(whimbrel_launch_mark_store_existed(db.as_ptr()));
    assert!(dir.path().join("store_existed").is_file());
    let files = take(whimbrel_launch_observe_files(db.as_ptr())).unwrap();
    assert_eq!(files["store_existed"], true);
    let input = json!({
        "files": files,
        "db_key": "found",
        "pending_key": "absent",
        "migrate_at_next_launch": false,
        "recording": false,
        "settings": {"fast_search": false},
        "physical_ram_bytes": 8_589_934_592u64,
        "good_launches": 0,
        "this_launch": {"resume": "not_run", "migration": "not_tried", "db_key_rejected": false,
                        "pending_key_rejected": false, "key_store_failed": false},
    });
    let input = CString::new(input.to_string()).unwrap();
    let plan = take(whimbrel_launch_decision(input.as_ptr())).unwrap();
    let codes: Vec<i64> = plan["steps"].as_array().unwrap().iter().map(|s| s["code"].as_i64().unwrap()).collect();
    assert_eq!(
        codes,
        [WHIMBREL_LAUNCH_OPEN_KEYED, WHIMBREL_LAUNCH_NOTICE_LIBRARY_MISSING, WHIMBREL_LAUNCH_START_MCP_SERVER].map(i64::from)
    );
    assert!(whimbrel_launch_clear_store_existed(db.as_ptr()));
    assert!(whimbrel_launch_clear_store_existed(db.as_ptr()), "already gone is fine");
    assert!(!dir.path().join("store_existed").exists());
    assert!(!whimbrel_launch_mark_store_existed(ptr::null()));
    assert!(!whimbrel_launch_clear_store_existed(ptr::null()));
}
