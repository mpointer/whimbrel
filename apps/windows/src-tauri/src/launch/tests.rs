//! The coordinator through a strict fake world (port of the Mac `LaunchCoordinatorTests`).
//! The key fakes follow the live store's rules (never replace; proof by bytes). Plans are
//! scripted like the Mac's, or come from the REAL core decision (`use_core`).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use api::encrypt_migration::{MigrationError, MigrationStatus, Phase, Progress, RestoreOutcome, RestoreWarning, SkipReason};
use api::launch_decision::{
    DamageReason, FileObservation, FileState, KeySource, LaunchAction, LaunchAction::*, LaunchInput, LaunchPlan, MigrationResult,
    RecoveryReason, ResumeResult, SkipNote,
};

use super::*;
use crate::db_key::{DbKeyError, KeyBytes, Lookup};

fn key(b: u8) -> KeyBytes {
    KeyBytes::new([b; 32])
}
const K: u8 = 1;
const P: u8 = 2;
const R: u8 = 3;
const N: u8 = 4;
/// A recovery key that does not open the file.
const W: u8 = 5;

fn label(k: Option<&KeyBytes>) -> String {
    match k {
        None => "nil".into(),
        Some(k) => match k.as_bytes()[0] {
            K => "K",
            P => "P",
            R => "R",
            N => "N",
            W => "W",
            _ => "?",
        }
        .into(),
    }
}

fn files(db_file: FileState, backup: bool, tmp: bool, migration: MigrationStatus) -> FileObservation {
    FileObservation { db_file, plain_backup_exists: backup, encrypted_tmp_exists: tmp, migration, store_existed: false }
}
fn plain_none() -> FileObservation {
    files(FileState::Plain, false, false, MigrationStatus::None)
}
fn not_plain() -> FileObservation {
    files(FileState::NotPlain, false, false, MigrationStatus::None)
}
fn not_plain_done() -> FileObservation {
    files(FileState::NotPlain, true, false, MigrationStatus::Done)
}
fn missing_none() -> FileObservation {
    files(FileState::Missing, false, false, MigrationStatus::None)
}
fn missing_backup() -> FileObservation {
    files(FileState::Missing, true, false, MigrationStatus::None)
}
fn missing_backup_in_progress() -> FileObservation {
    files(FileState::Missing, true, false, MigrationStatus::InProgress { phase: Phase::Swapped })
}
fn missing_in_progress() -> FileObservation {
    files(FileState::Missing, false, false, MigrationStatus::InProgress { phase: Phase::Verified })
}

fn found(b: u8) -> Lookup {
    Lookup::Found(key(b))
}

#[derive(Clone)]
enum FakeRun {
    Done,
    Fail,
}

enum FakeRestore {
    Done { moved_aside: u32, warning: Option<RestoreWarning> },
    Partly(u32),
}

type Hook = Box<dyn FnMut(&LaunchScreen, &mut St) + Send>;

/// The fake world's state.
pub(super) struct St {
    log: Vec<String>,
    plans: VecDeque<Vec<LaunchAction>>,
    always: Option<Vec<LaunchAction>>,
    inputs: Vec<LaunchInput>,
    use_core: bool,
    files: FileObservation,

    db_key: Lookup,
    pending_key: Lookup,
    previous: Vec<KeyBytes>,
    key_write_fails: bool,
    unverified_write: bool,

    fast_search: bool,
    migrate_flag: bool,
    good_launches: u32,
    recording_now: bool,

    resume_ok: bool,
    precheck: Result<(), SkipReason>,
    run: FakeRun,
    cleanup_removes: bool,
    slow_cleanup: bool,
    cleanup_thread: Option<std::thread::ThreadId>,
    cleanup_in_progress: bool,
    ticked: bool,
    ticked_during_cleanup: bool,
    restore_results: VecDeque<FakeRestore>,
    reset_ok: bool,
    delete_ok: bool,

    keyed_outcomes: VecDeque<OpenKind>,
    plain_opens: bool,
    file_key: Option<KeyBytes>,
    startup_fails: bool,
    /// The keyed open with the file's key never runs (store thread did not start).
    open_never_runs: bool,
    before_keyed_open: Option<Box<dyn FnMut(&mut St) + Send>>,

    responses: VecDeque<LaunchResponse>,
    screens: Vec<LaunchScreen>,
    onboarding_key: Option<KeyBytes>,
    progress: Vec<f64>,
    on_present: Option<Hook>,
}

impl Default for St {
    fn default() -> Self {
        St {
            log: vec![],
            plans: VecDeque::new(),
            always: None,
            inputs: vec![],
            use_core: false,
            files: plain_none(),
            db_key: Lookup::Absent,
            pending_key: Lookup::Absent,
            previous: vec![],
            key_write_fails: false,
            unverified_write: false,
            fast_search: false,
            migrate_flag: false,
            good_launches: 0,
            recording_now: false,
            resume_ok: true,
            precheck: Ok(()),
            run: FakeRun::Done,
            cleanup_removes: true,
            slow_cleanup: false,
            cleanup_thread: None,
            cleanup_in_progress: false,
            ticked: false,
            ticked_during_cleanup: false,
            restore_results: VecDeque::new(),
            reset_ok: true,
            delete_ok: true,
            keyed_outcomes: VecDeque::new(),
            plain_opens: true,
            file_key: None,
            startup_fails: false,
            open_never_runs: false,
            before_keyed_open: None,
            responses: VecDeque::new(),
            screens: vec![],
            onboarding_key: Some(key(N)),
            progress: vec![],
            on_present: None,
        }
    }
}

#[derive(Default)]
pub(super) struct World(Mutex<St>);

impl World {
    fn w(&self) -> MutexGuard<'_, St> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn log(&self, s: impl Into<String>) {
        self.w().log.push(s.into());
    }

    /// The live store's rule: same bytes are fine, a different key is never replaced.
    fn never_replace(current: &mut Lookup, with: &KeyBytes, account: &'static str, fails: bool) -> Result<(), DbKeyError> {
        if fails {
            return Err(DbKeyError::Write("refused".into()));
        }
        match current {
            Lookup::Found(existing) if existing == with => Ok(()),
            Lookup::Found(_) => Err(DbKeyError::Mismatch(account)),
            Lookup::Unavailable(m) => Err(DbKeyError::Unavailable(m.clone())),
            Lookup::Absent => {
                *current = Lookup::Found(with.clone());
                Ok(())
            }
        }
    }

    fn replace(w: &mut St, proven: &KeyBytes, rejected: Option<&KeyBytes>) -> Result<(), DbKeyError> {
        if w.key_write_fails {
            return Err(DbKeyError::Write("refused".into()));
        }
        match &w.db_key {
            Lookup::Absent => {
                if w.unverified_write {
                    return Err(DbKeyError::StoreUnverified);
                }
                w.db_key = Lookup::Found(proven.clone());
                Ok(())
            }
            Lookup::Found(old) if old == proven => Ok(()),
            Lookup::Found(old) => {
                let Some(rejected) = rejected else { return Err(DbKeyError::NotRejectedThisLaunch) };
                if rejected != old {
                    return Err(DbKeyError::RejectedKeyChanged);
                }
                let old = old.clone();
                if !w.previous.contains(&old) {
                    w.previous.push(old);
                }
                if w.unverified_write {
                    return Err(DbKeyError::StoreUnverified);
                }
                w.db_key = Lookup::Found(proven.clone());
                Ok(())
            }
            Lookup::Unavailable(m) => Err(DbKeyError::Unavailable(m.clone())),
        }
    }
}

fn dup(l: &Lookup) -> Lookup {
    super::dup(l)
}

impl LaunchKeys for World {
    fn lookup(&self) -> Lookup {
        let mut w = self.w();
        w.log.push("lookup".into());
        dup(&w.db_key)
    }
    fn lookup_pending(&self) -> Lookup {
        let mut w = self.w();
        w.log.push("lookupPending".into());
        dup(&w.pending_key)
    }
    fn store_pending(&self, k: &KeyBytes) -> Result<(), DbKeyError> {
        let mut w = self.w();
        w.log.push(format!("storePending({})", label(Some(k))));
        let fails = w.key_write_fails;
        World::never_replace(&mut w.pending_key, k, "db-key-pending", fails)
    }
    fn store_key(&self, k: &KeyBytes) -> Result<(), DbKeyError> {
        let mut w = self.w();
        w.log.push(format!("storeKey({})", label(Some(k))));
        let fails = w.key_write_fails;
        World::never_replace(&mut w.db_key, k, "db-key", fails)
    }
    fn replace_after_recovery(&self, proven: &KeyBytes, replacing: Option<&KeyBytes>) -> Result<(), DbKeyError> {
        let mut w = self.w();
        w.log.push(format!("replace({},rejected:{})", label(Some(proven)), label(replacing)));
        World::replace(&mut w, proven, replacing)
    }
    fn promote_pending_key(&self, opened_with: &KeyBytes, replacing: Option<&KeyBytes>) -> Result<bool, DbKeyError> {
        let mut w = self.w();
        w.log.push(format!("promotePending(opened:{},rejected:{})", label(Some(opened_with)), label(replacing)));
        let Lookup::Found(p) = dup(&w.pending_key) else { return Err(DbKeyError::NoPendingKey) };
        if &p != opened_with {
            return Err(DbKeyError::PendingNotOpened);
        }
        World::replace(&mut w, &p, replacing)?;
        w.pending_key = Lookup::Absent;
        Ok(false)
    }
    fn delete_pending(&self) -> Result<(), DbKeyError> {
        let mut w = self.w();
        w.log.push("deletePending".into());
        w.pending_key = Lookup::Absent;
        Ok(())
    }
    fn with_migration_writer(&self, k: &KeyBytes, f: &mut dyn FnMut(&dyn Fn(&str, &memory::DbKey) -> Result<(), String>)) {
        let l = label(Some(k));
        f(&|account, _| {
            self.log(format!("migrationWrite({account},{l})"));
            Ok(())
        });
    }
}

impl LaunchSettingsStore for World {
    fn fast_search(&self) -> bool {
        self.w().fast_search
    }
    fn migrate_at_next_launch(&self) -> bool {
        self.w().migrate_flag
    }
    fn set_migrate_at_next_launch(&self, on: bool) {
        let mut w = self.w();
        w.migrate_flag = on;
        w.log.push(format!("setMigrateFlag({on})"));
    }
    fn good_launches(&self) -> u32 {
        self.w().good_launches
    }
    fn set_good_launches(&self, n: u32) {
        let mut w = self.w();
        w.good_launches = n;
        w.log.push(format!("setGoodLaunches({n})"));
    }
}

impl LaunchMigration for World {
    fn resume(&self) -> Result<Option<Phase>, MigrationError> {
        let mut w = self.w();
        w.log.push("resume".into());
        // Like core: the original comes back from <db>.plain.bak, no run left.
        if w.use_core && w.resume_ok && w.files.plain_backup_exists {
            w.files = plain_none();
        }
        if w.resume_ok {
            Ok(Some(Phase::CopyWritten))
        } else {
            Err(MigrationError::Recovery("busy".into()))
        }
    }
    fn precheck(&self, recording: bool) -> Result<(), SkipReason> {
        let mut w = self.w();
        w.log.push(format!("precheck(recording:{recording})"));
        w.precheck.clone()
    }
    fn run(
        &self,
        k: &KeyBytes,
        recording: bool,
        store_key: &dyn Fn(&str, &memory::DbKey) -> Result<(), String>,
        progress: &(dyn Fn(Progress) + Sync),
    ) -> Result<Phase, MigrationError> {
        self.log(format!("run({},recording:{recording})", label(Some(k))));
        let _ = store_key("db-key-pending", &k.db_key());
        progress(Progress { step: api::encrypt_migration::Step::Copy, step_fraction: 1.0, overall: 0.5, note: None });
        let outcome = self.w().run.clone();
        match outcome {
            FakeRun::Done => {
                let _ = store_key("db-key", &k.db_key());
                let mut w = self.w();
                w.db_key = Lookup::Found(k.clone());
                if w.use_core {
                    w.files = not_plain_done();
                    w.file_key = Some(k.clone());
                }
                Ok(Phase::Done)
            }
            FakeRun::Fail => Err(MigrationError::Verify("m".into())),
        }
    }
    fn cleanup_plain_backup(&self, good_launches: u32) -> bool {
        let slow = {
            let mut w = self.w();
            w.log.push(format!("cleanupPlainBackup({good_launches})"));
            w.cleanup_thread = Some(std::thread::current().id());
            w.slow_cleanup
        };
        if slow {
            self.w().cleanup_in_progress = true;
            std::thread::sleep(std::time::Duration::from_millis(200));
            let mut w = self.w();
            w.ticked_during_cleanup = w.ticked;
            w.cleanup_in_progress = false;
        }
        self.w().cleanup_removes
    }
    fn reset_failures(&self) -> Result<(), MigrationError> {
        self.log("resetFailures");
        Ok(())
    }
    fn restore_rollback_copy(&self) -> Result<RestoreOutcome, MigrationError> {
        let mut w = self.w();
        w.log.push("restoreRollbackCopy".into());
        let next = w.restore_results.pop_front().unwrap_or(FakeRestore::Done { moved_aside: 0, warning: None });
        match next {
            FakeRestore::Done { moved_aside, warning } => {
                if w.use_core {
                    w.files = plain_none();
                }
                Ok(RestoreOutcome { moved_aside: (0..moved_aside).map(|i| format!("f{i}")).collect(), warning })
            }
            FakeRestore::Partly(n) => {
                Err(MigrationError::PartlyDone { moved_aside: (0..n).map(|i| format!("f{i}")).collect(), message: "m".into() })
            }
        }
    }
    fn reset_encryption_status(&self) -> Result<String, MigrationError> {
        let mut w = self.w();
        w.log.push("resetEncryptionStatus".into());
        if !w.reset_ok {
            return Err(MigrationError::Refused("no".into()));
        }
        if w.use_core {
            w.files = missing_none();
        }
        Ok("state.aside".into())
    }
    fn clear_store_existed(&self) {
        self.w().log.push("clearStoreExisted".into());
    }
    fn delete_plain_backup(&self, opened: bool) -> Result<Vec<String>, MigrationError> {
        let mut w = self.w();
        w.log.push(format!("deletePlainBackup(opened:{opened})"));
        if w.delete_ok {
            Ok(vec!["x.plain.bak".into()])
        } else {
            Err(MigrationError::Refused("no".into()))
        }
    }
}

impl LaunchPresenter for World {
    fn present(&self, screen: &LaunchScreen) -> LaunchResponse {
        let mut w = self.w();
        w.log.push("present".into());
        w.screens.push(screen.clone());
        if let Some(mut hook) = w.on_present.take() {
            hook(screen, &mut *w);
            w.on_present = Some(hook);
        }
        w.responses.pop_front().unwrap_or(LaunchResponse::Quit)
    }
    fn run_onboarding(&self, existing: Option<&KeyBytes>) -> Option<KeyBytes> {
        let mut w = self.w();
        w.log.push(format!("onboarding(existing:{})", label(existing)));
        w.onboarding_key.clone()
    }
    fn migration_progress(&self, p: &Progress) {
        self.w().progress.push(p.overall);
    }
}

#[derive(Debug)]
pub(super) struct FakeStore {
    keyed: bool,
}

struct Opener(Arc<World>);

impl StoreOpener for Opener {
    type Store = FakeStore;
    fn open_plain(&self) -> Option<FakeStore> {
        let mut w = self.0.w();
        w.log.push("openPlain".into());
        w.plain_opens.then_some(FakeStore { keyed: false })
    }
    fn open_keyed(&self, k: &KeyBytes, cache: Option<i64>, must_exist: bool) -> (OpenKind, Option<FakeStore>) {
        let mut w = self.0.w();
        let cache = cache.map_or("nil".to_string(), |c| c.to_string());
        w.log.push(format!("openKeyed({},cache:{cache}{})", label(Some(k)), if must_exist { ",existing" } else { "" }));
        if let Some(mut hook) = w.before_keyed_open.take() {
            hook(&mut *w);
            w.before_keyed_open = Some(hook);
        }
        let outcome = if w.files.db_file == FileState::Missing {
            // Like core: a never-create open refuses; a normal open CREATES a database.
            if must_exist {
                OpenKind::Damaged("no database file".into())
            } else {
                w.log.push(format!("created({})", label(Some(k))));
                OpenKind::Opened
            }
        } else if let Some(fk) = &w.file_key {
            if fk == k {
                if w.open_never_runs {
                    OpenKind::NeverRan("no thread".into())
                } else if w.startup_fails {
                    OpenKind::StartupFailed("sweep".into())
                } else {
                    OpenKind::Opened
                }
            } else {
                OpenKind::KeyRejected("wrong key".into())
            }
        } else {
            w.keyed_outcomes.pop_front().unwrap_or(OpenKind::Opened)
        };
        let store = (outcome == OpenKind::Opened).then_some(FakeStore { keyed: true });
        (outcome, store)
    }
}

fn coordinator_with(world: &Arc<World>, recording: Box<dyn Fn() -> bool + Send + Sync>) -> LaunchCoordinator<FakeStore> {
    let (o, d, m) = (world.clone(), world.clone(), world.clone());
    LaunchCoordinator::new(Collaborators {
        observe_files: Box::new(move || {
            let mut w = o.w();
            w.log.push("observe".into());
            w.files
        }),
        decide: Box::new(move |input: &LaunchInput| {
            let mut w = d.w();
            w.log.push("decide".into());
            w.inputs.push(*input);
            if w.use_core {
                return core_decide(input);
            }
            if let Some(a) = &w.always {
                return Some(LaunchPlan { steps: a.clone() });
            }
            w.plans.pop_front().map(|steps| LaunchPlan { steps })
        }),
        keys: world.clone(),
        settings: world.clone(),
        physical_memory: Box::new(|| 32 * 1024 * 1024 * 1024),
        recording,
        migration: world.clone(),
        opener: Box::new(Opener(world.clone())),
        start_mcp_server: Box::new(move |k| m.log(format!("startMcp({})", label(k)))),
        presenter: world.clone(),
        now_ms: Box::new(|| 1000),
    })
}

fn coordinator(world: &Arc<World>) -> LaunchCoordinator<FakeStore> {
    let r = world.clone();
    coordinator_with(world, Box::new(move || r.w().recording_now))
}

fn world() -> Arc<World> {
    Arc::new(World::default())
}

const ASK: [&str; 4] = ["observe", "lookup", "lookupPending", "decide"];

/// `ASK` then `rest`, repeated per group.
fn seq(groups: &[&[&str]]) -> Vec<String> {
    groups.iter().flat_map(|g| ASK.iter().chain(g.iter()).map(|s| s.to_string())).collect()
}

fn log(w: &Arc<World>) -> Vec<String> {
    w.w().log.clone()
}

fn assert_never(w: &Arc<World>, prefixes: &[&str]) {
    let log = log(w);
    for p in prefixes {
        assert!(!log.iter().any(|l| l.starts_with(p)), "{p} must not be called: {log:?}");
    }
}

const FORBIDDEN_ON_A_SCREEN: [&str; 11] = [
    "openPlain",
    "openKeyed",
    "startMcp",
    "run(",
    "storeKey",
    "storePending",
    "promotePending",
    "deletePending",
    "setGoodLaunches",
    "cleanupPlainBackup",
    "setMigrateFlag",
];

fn forbidden_plus(extra: &[&'static str]) -> Vec<&'static str> {
    FORBIDDEN_ON_A_SCREEN.iter().copied().chain(extra.iter().copied()).collect()
}

fn opened(o: LaunchOutcome<FakeStore>) -> LaunchOpened<FakeStore> {
    match o {
        LaunchOutcome::Opened(o) => o,
        other => panic!("expected an open, got {other:?}"),
    }
}

fn is_quit(o: &LaunchOutcome<FakeStore>) -> bool {
    matches!(o, LaunchOutcome::Quit)
}

fn this_launch(w: &Arc<World>, n: usize) -> api::launch_decision::ThisLaunch {
    w.w().inputs[n].this_launch
}

fn screens(w: &Arc<World>) -> Vec<LaunchScreen> {
    w.w().screens.clone()
}

fn keyed(source: KeySource) -> LaunchAction {
    OpenKeyed { key_source: source, cache_kib: None }
}

fn set(w: &Arc<World>, f: impl FnOnce(&mut St)) {
    f(&mut w.w());
}

fn same_lookup(a: &Lookup, b: &Lookup) -> bool {
    match (a, b) {
        (Lookup::Found(x), Lookup::Found(y)) => x == y,
        (Lookup::Absent, Lookup::Absent) => true,
        (Lookup::Unavailable(x), Lookup::Unavailable(y)) => x == y,
        _ => false,
    }
}

fn db_key_is(w: &Arc<World>, b: u8) -> bool {
    same_lookup(&w.w().db_key, &found(b))
}

/// Review of 83e2b079: `run` holds its state across a screen's wait; `notes()` and
/// `opened_with_key()` (read by the sync `encryption_card` command) must not wait on it.
#[test]
fn notes_never_wait_on_a_launch_that_is_showing_a_screen() {
    let w = world();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    set(&w, move |s| {
        s.plans = [vec![ShowDamaged { reason: DamageReason::NoAccess }]].into();
        s.responses = [LaunchResponse::Quit].into();
        s.on_present = Some(Box::new(move |_, _| {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv_timeout(std::time::Duration::from_secs(10));
        }));
    });
    let c = Arc::new(coordinator(&w));
    let runner = {
        let c = c.clone();
        std::thread::spawn(move || is_quit(&c.run()))
    };
    entered_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the screen showed");
    // The screen is waiting (and `run` holds its lock). Read from another thread with a deadline.
    let (got_tx, got_rx) = std::sync::mpsc::channel();
    {
        let c = c.clone();
        std::thread::spawn(move || {
            let _ = got_tx.send((c.notes(), c.opened_with_key()));
        });
    }
    let got = got_rx.recv_timeout(std::time::Duration::from_secs(2));
    release_tx.send(()).unwrap();
    assert!(runner.join().unwrap(), "the launch quit");
    assert_eq!(got.expect("notes() blocked while a screen waited"), (vec![], false));
}

// ---- Plain file

#[test]
fn plain_with_no_key_opens_plain_then_the_mcp_server() {
    let w = world();
    set(&w, |s| s.plans = [vec![OpenPlain, StartMcpServer]].into());
    let c = coordinator(&w);
    let o = opened(c.run());
    assert_eq!(log(&w), seq(&[&["openPlain", "startMcp(nil)"]]));
    assert!(o.key.is_none() && !o.store.keyed);
    assert_eq!(c.notes(), vec![]);
    let input = w.w().inputs[0];
    assert_eq!(input.db_key, api::launch_decision::KeyPresence::Absent);
    assert_eq!(input.pending_key, api::launch_decision::KeyPresence::Absent);
    assert_eq!(input.files, plain_none(), "files passed through untouched");
    assert_eq!(input.physical_ram_bytes, 32 * 1024 * 1024 * 1024);
    assert_eq!(input.this_launch, Default::default());
    assert_never(&w, &["openKeyed", "resume", "precheck", "run(", "present", "onboarding"]);
}

#[test]
fn plain_with_the_key_store_unavailable_opens_plain_and_records_the_note() {
    let w = world();
    set(&w, |s| {
        s.db_key = Lookup::Unavailable("Access is denied. (os error 5)".into());
        s.plans = [vec![OpenPlainRecordKeyUnavailable, StartMcpServer]].into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(log(&w), seq(&[&["openPlain", "startMcp(nil)"]]));
    assert_eq!(w.w().inputs[0].db_key, api::launch_decision::KeyPresence::Unavailable);
    assert_eq!(c.notes(), vec![LaunchNote::KeyStoreUnavailable(LaunchDiagnostic("Access is denied. (os error 5)".into()))]);
    assert_never(&w, &["openKeyed", "run(", "storeKey", "storePending"]);
}

#[test]
fn plain_with_db_key_and_no_opt_in_offers_encryption() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.plans = [vec![OpenPlain, ShowEncryptionAvailable, StartMcpServer]].into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(log(&w), seq(&[&["openPlain", "startMcp(nil)"]]), "a plain store never gets the key");
    assert_eq!(c.notes(), vec![LaunchNote::EncryptionAvailable]);
}

// ---- Missing file

#[test]
fn missing_file_with_the_key_store_unavailable_is_a_hard_stop_with_nothing_created() {
    let w = world();
    set(&w, |s| {
        s.db_key = Lookup::Unavailable("denied".into());
        s.plans = [vec![HardStopNoKeyStorage], vec![HardStopNoKeyStorage]].into();
        s.responses = [LaunchResponse::Retry, LaunchResponse::Quit].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::KeyStorageUnavailable, LaunchScreen::KeyStorageUnavailable]);
    assert_eq!(log(&w), seq(&[&["present"], &["present"]]), "Retry asks again");
    assert_never(&w, &forbidden_plus(&["onboarding"]));
}

#[test]
fn new_install_runs_the_setup_stores_the_pending_key_then_db_key_then_creates_the_encrypted_store() {
    let w = world();
    set(&w, |s| s.plans = [vec![NewInstallOnboarding { reuse_pending_key: false }], vec![keyed(KeySource::DbKey), StartMcpServer]].into());
    let o = opened(coordinator(&w).run());
    assert_eq!(
        log(&w),
        seq(&[&["onboarding(existing:nil)", "storePending(N)", "storeKey(N)"], &["openKeyed(N,cache:nil)", "startMcp(N)"]])
    );
    assert_eq!(o.key, Some(key(N)));
    assert!(o.store.keyed);
}

#[test]
fn new_install_with_a_pending_key_reuses_it_and_never_makes_another() {
    let w = world();
    set(&w, |s| {
        s.pending_key = found(P);
        s.onboarding_key = Some(key(P));
        s.plans = [vec![NewInstallOnboarding { reuse_pending_key: true }], vec![keyed(KeySource::DbKey), StartMcpServer]].into();
    });
    opened(coordinator(&w).run());
    assert_eq!(
        log(&w),
        seq(&[&["onboarding(existing:P)", "storeKey(P)"], &["openKeyed(P,cache:nil)", "startMcp(P)"]]),
        "the pending key is already stored: only db-key is written"
    );
}

#[test]
fn a_fresh_key_is_never_made_while_a_key_exists_or_cannot_be_read() {
    for (db, pending) in [
        (Lookup::Absent, found(P)),
        (found(K), Lookup::Absent),
        (Lookup::Absent, Lookup::Unavailable("x".into())),
        (Lookup::Unavailable("x".into()), Lookup::Absent),
    ] {
        let w = world();
        set(&w, |s| {
            s.db_key = db;
            s.pending_key = pending;
            s.plans = [vec![NewInstallOnboarding { reuse_pending_key: false }]].into();
        });
        assert!(is_quit(&coordinator(&w).run()));
        assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::FreshKeyRefused)]);
        assert_never(&w, &forbidden_plus(&["onboarding"]));
    }
}

#[test]
fn a_failed_onboarding_write_is_retried_with_the_same_key() {
    let w = world();
    set(&w, |s| {
        s.key_write_fails = true;
        s.plans = [
            vec![NewInstallOnboarding { reuse_pending_key: false }],
            vec![HardStopNoKeyStorage],
            vec![NewInstallOnboarding { reuse_pending_key: false }],
            vec![keyed(KeySource::DbKey), StartMcpServer],
        ]
        .into();
        s.responses = [LaunchResponse::Retry].into();
        s.on_present = Some(Box::new(|_, s| s.key_write_fails = false));
    });
    let o = opened(coordinator(&w).run());
    let log = log(&w);
    assert_eq!(log.iter().filter(|l| l.starts_with("onboarding")).collect::<Vec<_>>(), ["onboarding(existing:nil)"], "one setup only");
    assert_eq!(log.iter().filter(|l| l.starts_with("store")).collect::<Vec<_>>(), ["storePending(N)", "storePending(N)", "storeKey(N)"]);
    assert_eq!(o.key, Some(key(N)));
}

#[test]
fn onboarding_that_hands_back_a_different_key_than_the_pending_one_is_refused() {
    let w = world();
    set(&w, |s| {
        s.pending_key = found(P);
        s.onboarding_key = Some(key(N));
        s.plans = [vec![NewInstallOnboarding { reuse_pending_key: true }]].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::OnboardingKeyMismatch)]);
    assert_never(&w, &FORBIDDEN_ON_A_SCREEN);
}

#[test]
fn new_install_key_store_failure_records_it_and_asks_again() {
    let w = world();
    set(&w, |s| {
        s.key_write_fails = true;
        s.plans = [vec![NewInstallOnboarding { reuse_pending_key: false }], vec![HardStopNoKeyStorage], vec![HardStopNoKeyStorage]].into();
        s.responses = [LaunchResponse::Retry, LaunchResponse::Quit].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert!(!this_launch(&w, 0).key_store_failed);
    assert!(this_launch(&w, 1).key_store_failed);
    assert!(!this_launch(&w, 2).key_store_failed, "Retry clears it");
    assert_eq!(screens(&w), vec![LaunchScreen::KeyStorageUnavailable, LaunchScreen::KeyStorageUnavailable]);
    assert_never(&w, &["openPlain", "openKeyed", "startMcp", "storeKey"]);
}

// ---- Encrypted file

#[test]
fn encrypted_with_the_key_store_unavailable_shows_the_retry_screen_and_retry_asks_again() {
    let w = world();
    set(&w, |s| {
        s.db_key = Lookup::Unavailable("The specified logon session does not exist. (os error 1312)".into());
        s.plans = [vec![ShowKeyUnavailableScreen], vec![keyed(KeySource::DbKey), StartMcpServer]].into();
        s.responses = [LaunchResponse::Retry].into();
        // Credential Manager answers by the time the user presses Retry.
        s.on_present = Some(Box::new(|_, s| s.db_key = found(K)));
    });
    let o = opened(coordinator(&w).run());
    assert_eq!(
        screens(&w),
        vec![LaunchScreen::KeyUnavailable(LaunchDiagnostic("The specified logon session does not exist. (os error 1312)".into()))]
    );
    assert_eq!(log(&w), seq(&[&["present"], &["openKeyed(K,cache:nil)", "startMcp(K)"]]));
    assert_eq!(w.w().inputs[1].db_key, api::launch_decision::KeyPresence::Found, "the keys are read again");
    assert_eq!(o.key, Some(key(K)));
}

#[test]
fn encrypted_with_no_key_shows_recovery_and_a_restored_key_is_stored_only_after_it_opened() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.file_key = Some(key(R));
        s.responses = [LaunchResponse::RestoredKey(key(R))].into();
    });
    let o = opened(coordinator(&w).run());
    assert_eq!(screens(&w), vec![LaunchScreen::Recovery(RecoveryReason::NeedsKey)]);
    assert_eq!(log(&w), seq(&[&["present"], &["openKeyed(R,cache:nil,existing)", "replace(R,rejected:nil)", "startMcp(R)"]]));
    assert_eq!(w.w().inputs[1].db_key, api::launch_decision::KeyPresence::Found, "the candidate stands in for db-key");
    assert!(db_key_is(&w, R));
    assert_eq!(o.key, Some(key(R)));
}

#[test]
fn recovery_over_a_rejected_key_proves_before_it_persists() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.responses = [LaunchResponse::RestoredKey(key(W)), LaunchResponse::RestoredKey(key(R))].into();
    });
    let c = coordinator(&w);
    let o = opened(c.run());
    assert_eq!(
        log(&w),
        seq(&[
            &["openKeyed(K,cache:nil)"],
            &["present"],
            &["openKeyed(W,cache:nil,existing)"],
            &["present"],
            &["openKeyed(R,cache:nil,existing)", "replace(R,rejected:K)", "startMcp(R)"],
        ]),
        "nothing is written before R opened; the rejected plans' MCP server never starts"
    );
    assert_eq!(screens(&w), vec![LaunchScreen::Recovery(RecoveryReason::KeyRejected); 2]);
    assert!(db_key_is(&w, R));
    assert_eq!(w.w().previous, vec![key(K)], "the old key is never destroyed");
    assert_eq!(o.key, Some(key(R)));
    assert_eq!(c.notes(), vec![]);

    // The next launch converges: no screen, no write.
    set(&w, |s| {
        s.log.clear();
        s.screens.clear();
    });
    opened(coordinator(&w).run());
    assert_eq!(log(&w), seq(&[&["openKeyed(R,cache:nil)", "startMcp(R)"]]));
}

#[test]
fn a_proven_key_that_cannot_be_stored_is_a_note_and_the_store_is_open() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.key_write_fails = true;
        s.responses = [LaunchResponse::RestoredKey(key(R))].into();
    });
    let c = coordinator(&w);
    let o = opened(c.run());
    assert_eq!(o.key, Some(key(R)));
    let log = log(&w);
    assert_eq!(log[log.len() - 3..], ["openKeyed(R,cache:nil,existing)", "replace(R,rejected:K)", "startMcp(R)"]);
    assert!(matches!(c.notes().first(), Some(LaunchNote::RecoveredKeyNotStored(LaunchKeyFailure::KeychainUnavailable, _))), "{:?}", c.notes());
    assert!(db_key_is(&w, K), "nothing replaced");
}

#[test]
fn the_pending_key_opens_and_is_promoted() {
    let w = world();
    set(&w, |s| {
        s.pending_key = found(P);
        s.plans = [vec![keyed(KeySource::PendingKey), PromotePendingKey, StartMcpServer]].into();
    });
    opened(coordinator(&w).run());
    assert_eq!(log(&w), seq(&[&["openKeyed(P,cache:nil)", "promotePending(opened:P,rejected:nil)", "startMcp(P)"]]));
    assert!(db_key_is(&w, P));
}

#[test]
fn a_rejected_db_key_with_a_pending_key_that_opens_converges() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.pending_key = found(P);
        s.file_key = Some(key(P));
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(
        log(&w),
        seq(&[&["openKeyed(K,cache:nil)"], &["openKeyed(P,cache:nil)", "promotePending(opened:P,rejected:K)", "startMcp(P)"]])
    );
    assert_eq!(c.notes(), vec![]);
    assert!(db_key_is(&w, P));
    assert!(matches!(w.w().pending_key, Lookup::Absent));
    assert_eq!(w.w().previous, vec![key(K)]);

    w.w().log.clear();
    let next = coordinator(&w);
    opened(next.run());
    assert_eq!(log(&w), seq(&[&["openKeyed(P,cache:nil)", "startMcp(P)"]]), "converged: no promotion again");
    assert_eq!(next.notes(), vec![]);
}

#[test]
fn a_different_pending_key_is_never_deleted_and_the_note_survives_a_refresh() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.pending_key = found(P);
        s.plans = [
            vec![keyed(KeySource::DbKey), DeletePendingKey, ShowPlainBackupKept, StartMcpServer],
            vec![keyed(KeySource::DbKey), DeletePendingKey, StartMcpServer],
        ]
        .into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert!(!log(&w).contains(&"deletePending".to_string()));
    assert!(same_lookup(&w.w().pending_key, &found(P)));
    let differs = LaunchNote::PendingKeyNotDeleted(LaunchKeyFailure::PendingDiffers, LaunchDiagnostic::default());
    assert_eq!(c.notes(), vec![differs.clone(), LaunchNote::PlainBackupKept]);
    c.delete_plain_backup();
    assert_eq!(c.notes(), vec![differs], "a note about what this launch did stays after a refresh");
}

#[test]
fn a_key_missing_for_a_planned_step_is_a_specific_error_not_a_loop() {
    let w = world();
    set(&w, |s| s.plans = [vec![keyed(KeySource::PendingKey), StartMcpServer]].into());
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::KeyMissing(KeySource::PendingKey))]);
    assert_eq!(log(&w).iter().filter(|l| *l == "decide").count(), 1);

    let w = world();
    set(&w, |s| {
        s.plans = [vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }]].into()
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::KeyMissing(KeySource::DbKey))]);
    assert_never(&w, &["run("]);
}

#[test]
fn damaged_offers_only_quit_and_no_access_offers_retry() {
    let w = world();
    set(&w, |s| {
        s.plans = [vec![ShowDamaged { reason: DamageReason::Damaged }]].into();
        s.responses = [LaunchResponse::Retry, LaunchResponse::RestoreRollbackCopy, LaunchResponse::Quit].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w).len(), 3, "an answer the screen does not offer shows it again");
    assert_eq!(screens(&w)[0], LaunchScreen::Damaged(DamageReason::Damaged));
    assert_eq!(log(&w).iter().filter(|l| *l == "decide").count(), 1);
    assert_never(&w, &forbidden_plus(&["restoreRollbackCopy"]));

    // Permissions: Retry looks at the files again.
    let w = world();
    set(&w, |s| {
        s.plans = [vec![ShowDamaged { reason: DamageReason::NoAccess }], vec![OpenPlain, StartMcpServer]].into();
        s.responses = [LaunchResponse::Retry].into();
    });
    opened(coordinator(&w).run());
    assert_eq!(screens(&w), vec![LaunchScreen::Damaged(DamageReason::NoAccess)]);
    assert_eq!(log(&w), seq(&[&["present"], &["openPlain", "startMcp(nil)"]]));

    // In use by another program (core 0b9f8deb): not damage. Retry looks at the files again.
    let held = LaunchScreen::Damaged(DamageReason::OpenElsewhere);
    assert!(held.offers(&LaunchResponse::Retry) && held.offers(&LaunchResponse::Quit));
    assert!(!held.offers(&LaunchResponse::ContinueWithoutStore) && !held.offers(&LaunchResponse::RestoreRollbackCopy));
    let w = world();
    set(&w, |s| {
        s.plans = [vec![ShowDamaged { reason: DamageReason::OpenElsewhere }], vec![OpenPlain, StartMcpServer]].into();
        s.responses = [LaunchResponse::Retry].into();
    });
    opened(coordinator(&w).run());
    assert_eq!(screens(&w), vec![held]);
    assert_eq!(log(&w), seq(&[&["present"], &["openPlain", "startMcp(nil)"]]));
}

// ---- Migration

#[test]
fn migration_resumes_then_prechecks_then_runs_then_opens_keyed_and_counts_only_after_the_first_window() {
    let w = world();
    set(&w, |s| {
        s.pending_key = found(P);
        s.migrate_flag = true;
        s.fast_search = true;
        s.plans = [
            vec![FinishUndoViaResume],
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::PendingKey, recording: false }],
            vec![
                OpenKeyed { key_source: KeySource::DbKey, cache_kib: Some(1_048_576) },
                DeletePendingKey,
                ClearMigrateFlag,
                StartCleanupCounter,
                StartMcpServer,
            ],
        ]
        .into();
    });
    let c = coordinator(&w);
    let o = opened(c.run());
    assert_eq!(
        log(&w),
        seq(&[
            &["resume"],
            &["precheck(recording:false)", "run(P,recording:false)", "migrationWrite(db-key-pending,P)", "migrationWrite(db-key,P)"],
            &["openKeyed(P,cache:1048576)", "deletePending", "setMigrateFlag(false)", "startMcp(P)"],
        ])
    );
    assert_eq!(this_launch(&w, 1).resume, ResumeResult::Ran);
    assert_eq!(this_launch(&w, 2).migration, MigrationResult::Done);
    assert!(w.w().inputs[0].settings.fast_search);
    assert!(w.w().inputs[0].migrate_at_next_launch);
    assert_eq!(w.w().progress, vec![0.5]);
    assert_eq!(w.w().good_launches, 0, "never counted at the open");
    assert_eq!(o.key, Some(key(P)));

    w.w().log.clear();
    c.first_window_rendered().unwrap().join().unwrap();
    assert!(c.first_window_rendered().is_none());
    assert_eq!(log(&w), ["setGoodLaunches(1)"], "once, after the first window");
}

#[test]
fn the_counter_steps_run_only_after_the_first_window() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.good_launches = 3;
        s.plans = [vec![keyed(KeySource::DbKey), CleanupPlainBackup { good_launches: 3 }, StartMcpServer]].into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(log(&w), seq(&[&["openKeyed(K,cache:nil)", "startMcp(K)"]]));
    c.first_window_rendered().unwrap().join().unwrap();
    let l = log(&w);
    assert_eq!(l[l.len() - 2..], ["cleanupPlainBackup(3)", "setGoodLaunches(0)"]);

    for (step, expected) in [(RecordGoodLaunch { count: 2 }, "setGoodLaunches(2)"), (ResetCleanupCounter, "setGoodLaunches(0)")] {
        let w = world();
        set(&w, |s| {
            s.db_key = found(K);
            s.plans = [vec![keyed(KeySource::DbKey), step, StartMcpServer]].into();
        });
        let c = coordinator(&w);
        opened(c.run());
        assert!(!log(&w).contains(&expected.to_string()));
        c.first_window_rendered().unwrap().join().unwrap();
        assert_eq!(log(&w).last().unwrap(), expected);
        assert!(c.first_window_rendered().is_none(), "only once");
    }
}

#[test]
fn a_failed_cleanup_keeps_the_counter() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.cleanup_removes = false;
        s.plans = [vec![keyed(KeySource::DbKey), CleanupPlainBackup { good_launches: 3 }, StartMcpServer]].into();
    });
    let c = coordinator(&w);
    opened(c.run());
    c.first_window_rendered().unwrap().join().unwrap();
    assert_eq!(log(&w).last().unwrap(), "cleanupPlainBackup(3)");
}

#[test]
fn a_precheck_skip_ends_the_plan_and_the_note_carries_its_reason() {
    let w = world();
    let skip = SkipReason::LowDisk { need_bytes: 10, free_bytes: 1 };
    set(&w, |s| {
        s.db_key = found(K);
        s.migrate_flag = true;
        s.precheck = Err(skip.clone());
        s.plans = [
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }],
            vec![OpenPlain, ShowMigrationSkipped { reason: SkipNote::Precheck }, StartMcpServer],
        ]
        .into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(log(&w), seq(&[&["precheck(recording:false)"], &["openPlain", "startMcp(nil)"]]));
    assert_eq!(this_launch(&w, 1).migration, MigrationResult::Skipped);
    let note = LaunchNote::MigrationSkipped { reason: SkipNote::Precheck, skip: Some(skip) };
    assert_eq!(c.notes(), vec![note.clone()]);
    // A note's button refreshes the notes: the skip detail is kept.
    set(&w, |s| s.plans = [vec![OpenPlain, ShowMigrationSkipped { reason: SkipNote::Precheck }, StartMcpServer]].into());
    c.delete_plain_backup();
    assert_eq!(c.notes(), vec![note]);
}

#[test]
fn note_actions_do_nothing_while_the_cleanup_runs() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.slow_cleanup = true;
        s.plans = [
            vec![keyed(KeySource::DbKey), CleanupPlainBackup { good_launches: 3 }, ShowPlainBackupKept, StartMcpServer],
            vec![keyed(KeySource::DbKey), StartMcpServer],
        ]
        .into();
    });
    let c = coordinator(&w);
    opened(c.run());
    let rendering = c.first_window_rendered().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !w.w().cleanup_in_progress && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(w.w().cleanup_in_progress, "the cleanup never started within 5 s");
    assert!(c.cleanup_running());
    assert_eq!(c.delete_plain_backup(), FileAction::Refused);
    assert_eq!(c.reset_encryption_status(), FileAction::Refused);
    assert!(!c.retry_after_failed_before());
    let l = log(&w);
    assert!(!l.iter().any(|x| x.starts_with("deletePlainBackup") || x == "resetEncryptionStatus" || x == "resetFailures"));
    assert_eq!(l.iter().filter(|x| *x == "decide").count(), 1, "no notes refresh meanwhile");
    rendering.join().unwrap();
    assert!(!c.cleanup_running());
    assert_eq!(c.delete_plain_backup(), FileAction::Done);
    assert!(log(&w).contains(&"deletePlainBackup(opened:true)".to_string()));
}

#[test]
fn the_plan_cannot_promote_a_pending_key_the_store_did_not_open_with() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.pending_key = found(P);
        s.plans = [vec![keyed(KeySource::DbKey), PromotePendingKey, StartMcpServer]].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::StepOutOfOrder(15))]);
    assert_never(&w, &["openKeyed", "promotePending", "startMcp"]);
}

#[test]
fn while_the_file_is_missing_retry_shows_the_same_screen_and_opens_nothing() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.responses = [LaunchResponse::RestoredKey(key(R)), LaunchResponse::Retry, LaunchResponse::Retry, LaunchResponse::Quit].into();
        s.on_present = Some(Box::new(|screen, s| {
            if matches!(screen, LaunchScreen::Recovery(_)) {
                s.files = missing_none();
            }
        }));
    });
    assert!(is_quit(&coordinator(&w).run()));
    let mut expected = vec![LaunchScreen::Recovery(RecoveryReason::KeyRejected)];
    expected.extend(std::iter::repeat_n(LaunchScreen::Error(LaunchError::DatabaseMissingForRecoveredKey), 3));
    assert_eq!(screens(&w), expected);
    let l = log(&w);
    assert_eq!(l.iter().filter(|x| x.starts_with("openKeyed")).collect::<Vec<_>>(), ["openKeyed(K,cache:nil)"], "nothing opens");
    assert_eq!(l.iter().filter(|x| *x == "decide").count(), 2, "nothing is decided while the restored key waits");
    assert_never(&w, &["created(", "replace(", "storeKey", "storePending", "startMcp", "onboarding"]);
    assert!(db_key_is(&w, K));
}

#[test]
fn when_the_file_is_back_the_kept_restored_key_is_tried_and_proven() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.responses = [LaunchResponse::RestoredKey(key(R)), LaunchResponse::Retry].into();
        s.on_present = Some(Box::new(|screen, s| match screen {
            LaunchScreen::Recovery(_) => s.files = missing_none(),
            LaunchScreen::Error(LaunchError::DatabaseMissingForRecoveredKey) => s.files = not_plain(),
            _ => {}
        }));
    });
    let o = opened(coordinator(&w).run());
    assert_eq!(
        screens(&w),
        vec![LaunchScreen::Recovery(RecoveryReason::KeyRejected), LaunchScreen::Error(LaunchError::DatabaseMissingForRecoveredKey)]
    );
    let l = log(&w);
    assert_eq!(l[l.len() - 3..], ["openKeyed(R,cache:nil,existing)", "replace(R,rejected:K)", "startMcp(R)"]);
    assert_eq!(o.key, Some(key(R)));
    assert_eq!(w.w().previous, vec![key(K)]);
    assert_never(&w, &["created("]);
}

#[test]
fn with_a_candidate_a_missing_file_with_its_backup_still_reaches_the_restore_screen() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.responses = [LaunchResponse::RestoredKey(key(R)), LaunchResponse::RestoreRollbackCopy].into();
        s.on_present = Some(Box::new(|screen, s| {
            if matches!(screen, LaunchScreen::Recovery(_)) {
                s.files = missing_backup();
            }
        }));
    });
    opened(coordinator(&w).run());
    assert_eq!(
        screens(&w),
        vec![
            LaunchScreen::Recovery(RecoveryReason::KeyRejected),
            LaunchScreen::EncryptedFileMissing { backup_present: true, encrypted_copy_present: false, last_action: None },
        ]
    );
    let l = log(&w);
    assert!(l.contains(&"restoreRollbackCopy".to_string()));
    assert_eq!(l[l.len() - 2..], ["openPlain", "startMcp(nil)"]);
    assert_never(&w, &["replace(", "storeKey", "storePending", "created("]);
    assert!(db_key_is(&w, K));
}

#[test]
fn with_a_candidate_a_missing_file_with_an_unfinished_run_still_resumes() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.responses = [LaunchResponse::RestoredKey(key(R))].into();
        s.on_present = Some(Box::new(|screen, s| {
            if matches!(screen, LaunchScreen::Recovery(_)) {
                s.files = missing_backup_in_progress();
            }
        }));
    });
    opened(coordinator(&w).run());
    let l = log(&w);
    assert!(l.contains(&"resume".to_string()), "{l:?}");
    assert_eq!(l[l.len() - 2..], ["openPlain", "startMcp(nil)"]);
    assert_never(&w, &["replace(", "storeKey", "storePending", "created("]);
}

#[test]
fn with_a_candidate_a_file_that_returns_plain_migrates_with_the_stored_key() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.migrate_flag = true;
        s.responses = [LaunchResponse::RestoredKey(key(R))].into();
        s.on_present = Some(Box::new(|screen, s| {
            if matches!(screen, LaunchScreen::Recovery(_)) {
                s.files = plain_none();
            }
        }));
    });
    let c = coordinator(&w);
    let o = c.run();
    assert!(matches!(o, LaunchOutcome::Opened(_)), "{:?} {:?}", screens(&w), log(&w));
    let l = log(&w);
    assert!(l.contains(&"run(K,recording:false)".to_string()), "{l:?}");
    assert!(
        !l.iter().any(|x| x.contains('R') && (x.starts_with("migrationWrite") || x.starts_with("run("))),
        "db-key-pending is never written with the restored key"
    );
    assert_never(&w, &["replace("]);
    // The drop restored the earlier rejection; the run that returned Done cleared it.
    let inputs = w.w().inputs.clone();
    let after_drop = inputs.iter().position(|i| i.files.db_file == FileState::Plain).unwrap();
    assert!(inputs[after_drop].this_launch.db_key_rejected, "K was rejected this launch");
    let last = inputs.last().unwrap().this_launch;
    assert_eq!(last.migration, MigrationResult::Done);
    assert!(!last.db_key_rejected && !last.pending_key_rejected, "a run that returns Done clears this launch's rejections");
}

#[test]
fn with_a_candidate_a_different_encrypted_file_rejects_it_and_stores_nothing() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.responses = [LaunchResponse::RestoredKey(key(R)), LaunchResponse::Quit].into();
        s.on_present = Some(Box::new(|screen, s| {
            if matches!(screen, LaunchScreen::Recovery(_)) {
                s.file_key = Some(key(N));
            }
        }));
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Recovery(RecoveryReason::KeyRejected); 2]);
    assert!(log(&w).contains(&"openKeyed(R,cache:nil,existing)".to_string()));
    assert_never(&w, &["replace(", "storeKey", "storePending", "startMcp"]);
}

/// Review of 83e2b079: an open that never ran (the store thread did not start, or the open
/// panicked) proves nothing, so the restored key is never stored and db-key is untouched.
#[test]
fn a_proof_open_that_never_ran_stores_nothing() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.open_never_runs = true;
        s.responses = [LaunchResponse::RestoredKey(key(R)), LaunchResponse::Quit].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w).last().unwrap(), &LaunchScreen::Error(LaunchError::KeyedOpenFailed(OpenKind::NeverRan("no thread".into()))));
    assert!(log(&w).contains(&"openKeyed(R,cache:nil,existing)".to_string()), "the proof open was tried");
    assert_never(&w, &["replace(", "storeKey", "storePending", "startMcp"]);
    assert!(db_key_is(&w, K), "db-key unchanged");
    assert!(w.w().previous.is_empty(), "nothing moved to db-key-previous");
}

#[test]
fn a_startup_failure_after_the_restored_key_opened_still_stores_it() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.startup_fails = true;
        s.responses = [LaunchResponse::RestoredKey(key(R)), LaunchResponse::Quit].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w).last().unwrap(), &LaunchScreen::Error(LaunchError::KeyedOpenFailed(OpenKind::StartupFailed("sweep".into()))));
    assert!(log(&w).contains(&"replace(R,rejected:K)".to_string()));
    assert!(db_key_is(&w, R));
    assert_eq!(w.w().previous, vec![key(K)]);
}

#[test]
fn a_file_that_vanishes_during_the_proof_open_is_never_created() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.responses = [LaunchResponse::RestoredKey(key(R)), LaunchResponse::Quit].into();
        s.before_keyed_open = Some(Box::new(|s| {
            if s.log.last().map(String::as_str) == Some("openKeyed(R,cache:nil,existing)") {
                s.files = missing_none();
            }
        }));
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(
        screens(&w),
        vec![LaunchScreen::Recovery(RecoveryReason::KeyRejected), LaunchScreen::Error(LaunchError::DatabaseMissingForRecoveredKey)]
    );
    assert_never(&w, &["created(", "replace(", "storeKey", "startMcp"]);
}

#[test]
fn a_db_key_that_changed_since_it_was_rejected_is_not_replaced() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.responses = [LaunchResponse::RestoredKey(key(R))].into();
        // Something else wrote db-key meanwhile.
        s.on_present = Some(Box::new(|_, s| s.db_key = found(N)));
    });
    let c = coordinator(&w);
    opened(c.run());
    assert!(log(&w).contains(&"replace(R,rejected:K)".to_string()));
    assert!(db_key_is(&w, N));
    assert_eq!(w.w().previous, vec![]);
    assert!(matches!(c.notes().first(), Some(LaunchNote::RecoveredKeyNotStored(LaunchKeyFailure::KeyChangedMeanwhile, _))), "{:?}", c.notes());
}

#[test]
fn an_unverified_store_is_its_own_note() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.file_key = Some(key(R));
        s.unverified_write = true;
        s.responses = [LaunchResponse::RestoredKey(key(R))].into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert!(matches!(c.notes().first(), Some(LaunchNote::RecoveredKeyStoreUnverified(_))), "{:?}", c.notes());
}

#[test]
fn the_backup_cleanup_runs_off_the_ui_thread() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.slow_cleanup = true;
        s.plans = [vec![keyed(KeySource::DbKey), CleanupPlainBackup { good_launches: 3 }, StartMcpServer]].into();
    });
    let c = coordinator(&w);
    opened(c.run());
    let handle = c.first_window_rendered().unwrap();
    // The caller's thread keeps working while the cleanup is busy.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !w.w().cleanup_in_progress && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    w.w().ticked = true;
    handle.join().unwrap();
    assert_ne!(w.w().cleanup_thread, Some(std::thread::current().id()));
    assert!(w.w().ticked_during_cleanup, "the caller ran other work while the cleanup was busy");
    assert_eq!(w.w().good_launches, 0);
}

#[test]
fn reset_after_a_failed_resume_does_not_show_the_same_screen_again() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = missing_in_progress();
        s.resume_ok = false;
        s.responses = [LaunchResponse::ResetEncryptionStatus].into();
        s.onboarding_key = None; // the new-install setup comes next; the test stops there
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(
        screens(&w),
        vec![LaunchScreen::EncryptedFileMissing { backup_present: false, encrypted_copy_present: false, last_action: None }]
    );
    assert_eq!(log(&w), seq(&[&["resume"], &["present", "resetEncryptionStatus", "clearStoreExisted"], &["onboarding(existing:nil)"]]), "Reset and start empty forgets the store marker");
    assert_eq!(this_launch(&w, 2).resume, ResumeResult::NotRun);
}

#[test]
fn a_migration_with_the_real_decision() {
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = plain_none();
        s.pending_key = found(P);
        s.migrate_flag = true;
    });
    let c = coordinator(&w);
    let o = opened(c.run());
    assert_eq!(
        log(&w),
        seq(&[
            &["precheck(recording:false)", "run(P,recording:false)", "migrationWrite(db-key-pending,P)", "migrationWrite(db-key,P)"],
            &["openKeyed(P,cache:nil)", "deletePending", "setMigrateFlag(false)", "startMcp(P)"],
        ])
    );
    assert_eq!(o.key, Some(key(P)));
    c.first_window_rendered().unwrap().join().unwrap();
    assert_eq!(w.w().good_launches, 1);
}

#[test]
fn an_unreadable_state_note_offers_reset_and_reset_never_opens_again() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.plans = [
            vec![OpenPlain, ShowMigrationSkipped { reason: SkipNote::StateUnreadable }, StartMcpServer],
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }],
        ]
        .into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(c.notes(), vec![LaunchNote::MigrationSkipped { reason: SkipNote::StateUnreadable, skip: None }]);
    w.w().log.clear();
    assert_eq!(c.reset_encryption_status(), FileAction::Done);
    let mut expected = vec!["resetEncryptionStatus".to_string()];
    expected.extend(ASK.iter().map(|s| s.to_string()));
    assert_eq!(log(&w), expected, "the notes are asked again, nothing is run");
    assert_eq!(c.notes(), vec![]);
    assert_never(&w, &["precheck", "run(", "openPlain", "openKeyed", "startMcp"]);
}

#[test]
fn a_failed_run_shows_the_failed_note() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.run = FakeRun::Fail;
        s.plans = [
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }],
            vec![OpenPlain, ShowMigrationFailed, StartMcpServer],
        ]
        .into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(this_launch(&w, 1).migration, MigrationResult::Failed);
    assert_eq!(c.notes(), vec![LaunchNote::MigrationFailed]);
}

#[test]
fn failed_before_shows_the_note_and_its_retry_resets_the_failures_without_opening_again() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.plans = [
            vec![OpenPlain, ShowFailedBefore { attempts: 2 }, StartMcpServer],
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }],
        ]
        .into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(c.notes(), vec![LaunchNote::FailedBefore { attempts: 2 }]);
    w.w().log.clear();
    assert!(c.retry_after_failed_before());
    let mut expected = vec!["resetFailures".to_string(), "setMigrateFlag(true)".to_string()];
    expected.extend(ASK.iter().map(|s| s.to_string()));
    assert_eq!(log(&w), expected);
    assert_eq!(c.notes(), vec![], "the migration runs at the next launch");
    assert_never(&w, &["precheck", "run(", "openPlain", "openKeyed", "startMcp"]);
}

#[test]
fn an_interrupted_migration_shows_retry_and_retry_resumes_again() {
    let w = world();
    set(&w, |s| {
        s.plans = [vec![FinishUndoViaResume], vec![ShowMigrationInterrupted], vec![FinishUndoViaResume], vec![OpenPlain, StartMcpServer]].into();
        s.resume_ok = false;
        s.responses = [LaunchResponse::Retry].into();
        s.on_present = Some(Box::new(|_, s| s.resume_ok = true));
    });
    opened(coordinator(&w).run());
    assert_eq!(screens(&w), vec![LaunchScreen::MigrationInterrupted]);
    assert_eq!(log(&w), seq(&[&["resume"], &["present"], &["resume"], &["openPlain", "startMcp(nil)"]]));
    assert_eq!(this_launch(&w, 1).resume, ResumeResult::Failed);
    assert_eq!(this_launch(&w, 2).resume, ResumeResult::NotRun, "Retry runs resume again");
    assert_eq!(this_launch(&w, 3).resume, ResumeResult::Ran);
}

#[test]
fn a_recording_in_progress_never_starts_a_migration() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.migrate_flag = true;
        s.recording_now = true;
        // A plan that (wrongly or from a stale read) says no recording.
        s.plans = [
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }],
            vec![OpenPlain, ShowMigrationSkipped { reason: SkipNote::Precheck }, StartMcpServer],
        ]
        .into();
    });
    opened(coordinator(&w).run());
    assert!(w.w().inputs[0].recording);
    assert_eq!(log(&w), seq(&[&["precheck(recording:true)"], &["openPlain", "startMcp(nil)"]]));
    assert_eq!(this_launch(&w, 1).migration, MigrationResult::Skipped);
    assert_never(&w, &["run("]);
}

#[test]
fn a_recording_that_starts_after_the_precheck_still_stops_the_run() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.plans = [
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }],
            vec![OpenPlain, ShowMigrationSkipped { reason: SkipNote::Precheck }, StartMcpServer],
        ]
        .into();
    });
    // Not recording when the inputs and the precheck are read; recording by the run.
    let r = w.clone();
    let c = coordinator_with(&w, Box::new(move || r.w().log.iter().any(|l| l.starts_with("precheck"))));
    opened(c.run());
    let l = log(&w);
    assert!(l.contains(&"precheck(recording:false)".to_string()));
    assert!(!l.iter().any(|x| x.starts_with("run(")), "{l:?}");
    assert_eq!(w.w().inputs.len(), 2);
    assert_eq!(this_launch(&w, 1).migration, MigrationResult::Skipped);
    assert_eq!(c.notes(), vec![LaunchNote::MigrationSkipped { reason: SkipNote::Precheck, skip: Some(SkipReason::Recording) }]);
}

// ---- Encrypted file missing and the plaintext copy

#[test]
fn encrypted_file_missing_with_the_backup_restores_on_confirm_then_opens_plain() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.plans = [
            vec![ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false }],
            vec![OpenPlain, ShowEncryptionAvailable, StartMcpServer],
        ]
        .into();
        s.restore_results = [FakeRestore::Done { moved_aside: 2, warning: Some(RestoreWarning::StateNotMovedAside("x".into())) }].into();
        s.responses = [LaunchResponse::RestoreRollbackCopy].into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(
        screens(&w),
        vec![LaunchScreen::EncryptedFileMissing { backup_present: true, encrypted_copy_present: false, last_action: None }]
    );
    assert_eq!(log(&w), seq(&[&["present", "restoreRollbackCopy"], &["openPlain", "startMcp(nil)"]]));
    assert_eq!(
        c.notes(),
        vec![
            LaunchNote::RestoredCopy { moved_aside: 2, warning: Some(RestoreWarningKind::StateNotMovedAside) },
            LaunchNote::EncryptionAvailable
        ]
    );
}

#[test]
fn a_partly_done_restore_is_shown_on_the_screen_and_can_be_pressed_again() {
    let w = world();
    set(&w, |s| {
        s.plans = [
            vec![ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false }],
            vec![ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false }],
            vec![OpenPlain, StartMcpServer],
        ]
        .into();
        s.restore_results = [FakeRestore::Partly(1), FakeRestore::Done { moved_aside: 1, warning: None }].into();
        s.responses = [LaunchResponse::RestoreRollbackCopy, LaunchResponse::RestoreRollbackCopy].into();
    });
    opened(coordinator(&w).run());
    assert_eq!(
        screens(&w),
        vec![
            LaunchScreen::EncryptedFileMissing { backup_present: true, encrypted_copy_present: false, last_action: None },
            LaunchScreen::EncryptedFileMissing {
                backup_present: true,
                encrypted_copy_present: false,
                last_action: Some(LaunchFileActionResult::Restore(FileAction::PartlyDone { moved_aside: 1 })),
            },
        ]
    );
    assert_eq!(log(&w).iter().filter(|l| *l == "restoreRollbackCopy").count(), 2);
}

#[test]
fn encrypted_file_missing_with_no_copy_offers_reset_and_only_copy_offers_nothing() {
    let w = world();
    set(&w, |s| {
        s.plans = [vec![ShowEncryptedFileMissing { backup_present: false, encrypted_copy_present: false }], vec![NewInstallOnboarding {
            reuse_pending_key: false,
        }]]
        .into();
        s.responses = [LaunchResponse::RestoreRollbackCopy, LaunchResponse::ResetEncryptionStatus].into();
        s.onboarding_key = None;
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(log(&w).iter().filter(|l| *l == "restoreRollbackCopy").count(), 0, "not offered without a backup");
    assert_eq!(log(&w).iter().filter(|l| *l == "resetEncryptionStatus").count(), 1);

    let w = world();
    set(&w, |s| {
        s.plans = [vec![ShowEncryptedFileMissing { backup_present: false, encrypted_copy_present: true }]].into();
        s.responses =
            [LaunchResponse::RestoreRollbackCopy, LaunchResponse::ResetEncryptionStatus, LaunchResponse::Retry, LaunchResponse::Quit].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w).len(), 4, "the support case offers no file action");
    assert_never(&w, &forbidden_plus(&["restoreRollbackCopy", "resetEncryptionStatus"]));
}

/// Core's NoticeLibraryMissing after the open: one launch note (the app's banner), never a
/// screen, and the launch goes on to the MCP server.
#[test]
fn a_missing_library_is_one_note_after_the_open_and_never_stops_the_launch() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.plans = [vec![keyed(KeySource::DbKey), NoticeLibraryMissing, StartMcpServer]].into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(c.notes(), vec![LaunchNote::LibraryMissing]);
    assert!(screens(&w).is_empty());
    assert!(log(&w).iter().any(|l| l.starts_with("startMcp")), "{:?}", log(&w));
    assert_eq!(validate(&[NoticeLibraryMissing, keyed(KeySource::DbKey)]), Some(LaunchError::StepOutOfOrder(26)), "only after an open");
    assert!(LaunchNote::LibraryMissing.is_launch_only(), "a refresh after a note action keeps it: the decision never repeats it");
    c.dismiss_library_missing();
    assert_eq!(c.notes(), vec![], "dismissed for the rest of the launch");
}

#[test]
fn plain_backup_kept_offers_delete_which_passes_the_keyed_open_and_never_opens_again() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.plans = [vec![keyed(KeySource::DbKey), ShowPlainBackupKept, StartMcpServer], vec![keyed(KeySource::DbKey), StartMcpServer]].into();
    });
    let c = coordinator(&w);
    opened(c.run());
    assert_eq!(c.notes(), vec![LaunchNote::PlainBackupKept]);
    w.w().log.clear();
    assert_eq!(c.delete_plain_backup(), FileAction::Done);
    let mut expected = vec!["deletePlainBackup(opened:true)".to_string()];
    expected.extend(ASK.iter().map(|s| s.to_string()));
    assert_eq!(log(&w), expected);
    assert_eq!(c.notes(), vec![]);
    assert_never(&w, &["openKeyed", "openPlain", "startMcp"]);
}

#[test]
fn delete_plain_backup_after_a_plain_open_says_the_store_was_not_opened_with_its_key() {
    let w = world();
    set(&w, |s| s.plans = [vec![OpenPlain, StartMcpServer], vec![OpenPlain, StartMcpServer]].into());
    let c = coordinator(&w);
    opened(c.run());
    c.delete_plain_backup();
    assert!(log(&w).contains(&"deletePlainBackup(opened:false)".to_string()));
}

// ---- Guards

#[test]
fn the_reask_cap_shows_an_error_instead_of_looping() {
    let w = world();
    set(&w, |s| {
        s.always = Some(vec![FinishUndoViaResume]);
        s.responses = [LaunchResponse::Quit].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(MAX_DECISIONS, 8);
    assert_eq!(log(&w).iter().filter(|l| *l == "decide").count(), MAX_DECISIONS as usize);
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::TooManyDecisions)]);
    assert_never(&w, &FORBIDDEN_ON_A_SCREEN);
}

#[test]
fn a_user_retry_starts_a_fresh_budget() {
    let w = world();
    set(&w, |s| {
        s.always = Some(vec![ShowKeyUnavailableScreen]);
        s.db_key = Lookup::Unavailable("x".into());
        s.responses = std::iter::repeat_n(LaunchResponse::Retry, 12).chain([LaunchResponse::Quit]).collect();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w).len(), 13);
    assert!(!screens(&w).contains(&LaunchScreen::Error(LaunchError::TooManyDecisions)), "presses of Retry are not a loop");
}

#[test]
fn the_mcp_server_starts_last_and_never_before_an_open() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.pending_key = found(K);
        s.migrate_flag = true;
        s.plans = [vec![keyed(KeySource::DbKey), DeletePendingKey, ClearMigrateFlag, StartMcpServer]].into();
    });
    opened(coordinator(&w).run());
    assert_eq!(log(&w).last().unwrap(), "startMcp(K)");

    let w = world();
    set(&w, |s| s.plans = [vec![StartMcpServer, OpenPlain]].into());
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::StepOutOfOrder(22))]);
    assert_never(&w, &["startMcp", "openPlain"]);

    // Nothing that changes files or keys, and no note, runs before an open either.
    for step in [PromotePendingKey, DeletePendingKey, ClearMigrateFlag, StartCleanupCounter, ShowEncryptionAvailable] {
        let w = world();
        set(&w, |s| s.plans = [vec![step, OpenPlain, StartMcpServer]].into());
        assert!(is_quit(&coordinator(&w).run()));
        assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::StepOutOfOrder(step.code()))]);
        assert_never(&w, &FORBIDDEN_ON_A_SCREEN);
    }
    // A migration step after an open is refused too, before anything of the plan runs.
    let w = world();
    set(&w, |s| {
        s.plans =
            [vec![OpenPlain, PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }]].into()
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_never(&w, &["openPlain", "precheck", "run(", "startMcp"]);
}

#[test]
fn fast_search_passes_the_cache_to_the_main_store_only() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.fast_search = true;
        s.plans = [vec![OpenKeyed { key_source: KeySource::DbKey, cache_kib: Some(1_048_576) }, StartMcpServer]].into();
    });
    opened(coordinator(&w).run());
    let l = log(&w);
    assert_eq!(l[l.len() - 2..], ["openKeyed(K,cache:1048576)", "startMcp(K)"], "the MCP server gets the key and nothing else");
    assert!(w.w().inputs[0].settings.fast_search);

    // With the real decision: a 32 GiB computer with Faster search on gets core's 1 GiB cache.
    let w = world();
    set(&w, |s| {
        s.use_core = true;
        s.files = not_plain();
        s.db_key = found(K);
        s.fast_search = true;
    });
    opened(coordinator(&w).run());
    assert!(log(&w).contains(&format!("openKeyed(K,cache:{})", api::settings::FAST_SEARCH_CACHE_KIB)));
}

#[test]
fn steps_out_of_order_and_a_failed_open_stop_with_an_error() {
    // validate on hand-built plans.
    assert_eq!(validate(&[OpenPlain, StartMcpServer]), None);
    assert_eq!(validate(&[StartMcpServer]), Some(LaunchError::StepOutOfOrder(22)));
    assert_eq!(validate(&[OpenPlain, StartMcpServer, ShowEncryptionAvailable]), Some(LaunchError::StepOutOfOrder(23)), "nothing after MCP");
    assert_eq!(validate(&[OpenPlain, OpenPlain]), Some(LaunchError::StepOutOfOrder(0)), "no second open");
    assert_eq!(validate(&[OpenPlain, ShowRecoveryScreen { reason: RecoveryReason::NeedsKey }]), Some(LaunchError::StepOutOfOrder(4)));
    assert_eq!(validate(&[OpenPlain, FinishUndoViaResume]), Some(LaunchError::StepOutOfOrder(11)));
    assert_eq!(validate(&[OpenPlain, PromotePendingKey]), Some(LaunchError::StepOutOfOrder(15)));
    assert_eq!(validate(&[keyed(KeySource::PendingKey), PromotePendingKey, StartMcpServer]), None);
    assert_eq!(validate(&[RecordGoodLaunch { count: 2 }]), Some(LaunchError::StepOutOfOrder(19)));

    let w = world();
    set(&w, |s| s.plans = [].into());
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::DecisionUnavailable)]);

    let w = world();
    set(&w, |s| {
        s.plain_opens = false;
        s.plans = [vec![OpenPlain, StartMcpServer]].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::PlainOpenFailed)]);
    assert_never(&w, &["startMcp"]);

    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.keyed_outcomes = [OpenKind::Damaged("x".into())].into();
        s.plans = [vec![keyed(KeySource::DbKey), StartMcpServer]].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::KeyedOpenFailed(OpenKind::Damaged("x".into())))]);
    assert_never(&w, &["startMcp"]);
}

#[test]
fn a_failed_plain_open_can_continue_without_the_database_and_never_starts_the_mcp_server() {
    let w = world();
    set(&w, |s| {
        s.plain_opens = false;
        s.plans = [vec![OpenPlain, StartMcpServer], vec![OpenPlain, StartMcpServer]].into();
        s.responses = [LaunchResponse::Retry, LaunchResponse::ContinueWithoutStore].into();
    });
    let c = coordinator(&w);
    assert!(matches!(c.run(), LaunchOutcome::OpenedWithoutStore), "{:?}", log(&w));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::PlainOpenFailed); 2]);
    assert_eq!(log(&w).iter().filter(|l| *l == "openPlain").count(), 2, "Try again opened again");
    assert_eq!(c.notes(), vec![LaunchNote::MemoryDatabaseUnavailable]);
    assert!(c.first_window_rendered().is_none());
    assert_never(&w, &["startMcp", "setGoodLaunches", "cleanupPlainBackup", "openKeyed"]);
    assert!(LaunchNote::MemoryDatabaseUnavailable.is_launch_only(), "kept when the notes are asked again");
}

#[test]
fn a_keyed_failure_never_continues_without_the_database() {
    let w = world();
    set(&w, |s| {
        s.db_key = found(K);
        s.keyed_outcomes = [OpenKind::Damaged("x".into())].into();
        s.plans = [vec![keyed(KeySource::DbKey), StartMcpServer]].into();
        s.responses = [LaunchResponse::ContinueWithoutStore].into();
    });
    assert!(is_quit(&coordinator(&w).run()));
    assert_eq!(screens(&w), vec![LaunchScreen::Error(LaunchError::KeyedOpenFailed(OpenKind::Damaged("x".into()))); 2]);
    assert_never(&w, &["startMcp"]);
}

#[test]
fn the_store_opener_is_never_called_on_a_screen_outcome() {
    let screens_steps = [
        HardStopNoKeyStorage,
        ShowKeyUnavailableScreen,
        ShowRecoveryScreen { reason: RecoveryReason::NeedsKey },
        ShowRecoveryScreen { reason: RecoveryReason::KeyRejected },
        ShowMigrationInterrupted,
        ShowDamaged { reason: DamageReason::Damaged },
        ShowDamaged { reason: DamageReason::NoAccess },
        ShowDamaged { reason: DamageReason::OpenElsewhere },
        ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false },
        ShowEncryptedFileMissing { backup_present: false, encrypted_copy_present: false },
        ShowEncryptedFileMissing { backup_present: false, encrypted_copy_present: true },
    ];
    for step in screens_steps {
        let w = world();
        set(&w, |s| s.plans = [vec![step]].into());
        assert!(is_quit(&coordinator(&w).run()), "{step:?}");
        assert_eq!(screens(&w).len(), 1, "{step:?}");
        assert_never(&w, &FORBIDDEN_ON_A_SCREEN);
    }
}

#[test]
fn every_screen_offers_quit_and_only_its_own_answers() {
    use LaunchResponse as Rs;
    let all = |s: &LaunchScreen| {
        [Rs::Retry, Rs::RestoreRollbackCopy, Rs::ResetEncryptionStatus, Rs::RestoredKey(key(R)), Rs::ContinueWithoutStore, Rs::Quit]
            .into_iter()
            .filter(|r| s.offers(r))
            .collect::<Vec<_>>()
    };
    let efm = |b, c| LaunchScreen::EncryptedFileMissing { backup_present: b, encrypted_copy_present: c, last_action: None };
    assert_eq!(all(&LaunchScreen::KeyStorageUnavailable), [Rs::Retry, Rs::Quit]);
    assert_eq!(all(&LaunchScreen::KeyUnavailable(LaunchDiagnostic::default())), [Rs::Retry, Rs::Quit]);
    assert_eq!(all(&LaunchScreen::Recovery(RecoveryReason::NeedsKey)), [Rs::RestoredKey(key(R)), Rs::Quit]);
    assert_eq!(all(&LaunchScreen::MigrationInterrupted), [Rs::Retry, Rs::Quit]);
    assert_eq!(all(&LaunchScreen::Damaged(DamageReason::Damaged)), [Rs::Quit]);
    assert_eq!(all(&LaunchScreen::Damaged(DamageReason::NoAccess)), [Rs::Retry, Rs::Quit]);
    assert_eq!(all(&LaunchScreen::Damaged(DamageReason::OpenElsewhere)), [Rs::Retry, Rs::Quit]);
    assert_eq!(all(&efm(true, false)), [Rs::RestoreRollbackCopy, Rs::Quit]);
    assert_eq!(all(&efm(true, true)), [Rs::RestoreRollbackCopy, Rs::Quit]);
    assert_eq!(all(&efm(false, false)), [Rs::ResetEncryptionStatus, Rs::Quit]);
    assert_eq!(all(&efm(false, true)), [Rs::Quit], "the .enc.tmp-only case is a support case");
    assert_eq!(all(&LaunchScreen::Error(LaunchError::PlainOpenFailed)), [Rs::Retry, Rs::ContinueWithoutStore, Rs::Quit]);
    assert_eq!(all(&LaunchScreen::Error(LaunchError::FreshKeyRefused)), [Rs::Retry, Rs::Quit]);
}

#[test]
fn the_launch_settings_live_in_settings_json_and_never_sync() {
    let dir = std::env::temp_dir().join(format!("whimbrel-launch-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");
    let file = Arc::new(Mutex::new(crate::settings::SettingsFile::load(&path)));
    let s = live::SettingsFileLaunchSettings(file.clone());
    assert!(!s.fast_search() && !s.migrate_at_next_launch());
    assert_eq!(s.good_launches(), 0);
    assert!(!path.exists(), "reading never writes a default");
    s.set_migrate_at_next_launch(true);
    s.set_good_launches(3);
    file.lock().unwrap().set(api::settings::FAST_SEARCH_KEY, Some(true));
    file.lock().unwrap().save().unwrap();
    let again = live::SettingsFileLaunchSettings(Arc::new(Mutex::new(crate::settings::SettingsFile::load(&path))));
    assert!(again.fast_search(), "core's pinned key");
    assert!(again.migrate_at_next_launch());
    assert_eq!(again.good_launches(), 3);
    // A negative or wrong-typed counter reads 0.
    file.lock().unwrap().set(live::keys::GOOD_LAUNCHES, Some(-4));
    assert_eq!(s.good_launches(), 0);
    file.lock().unwrap().set(live::keys::GOOD_LAUNCHES, Some("three"));
    assert_eq!(s.good_launches(), 0);
    assert_eq!(live::keys::FAST_SEARCH, "storage.fastSearch");
    let synced = crate::syncing::settings::synced_keys();
    for k in live::keys::ALL {
        assert!(!synced.iter().any(|s| s == k), "{k} must stay on this PC");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn launch_types_never_print_a_key() {
    let k = KeyBytes::new([0xAB; 32]);
    let shown = format!(
        "{:?} {:?} {:?}",
        LaunchResponse::RestoredKey(k.clone()),
        LaunchOutcome::<FakeStore>::Opened(LaunchOpened { store: FakeStore { keyed: true }, key: Some(k), open_ms: 1 }),
        LaunchScreen::Recovery(RecoveryReason::KeyRejected),
    );
    assert!(!shown.to_lowercase().contains("abab") && !shown.contains("171"), "{shown}");
}

