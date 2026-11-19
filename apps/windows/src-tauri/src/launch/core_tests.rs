//! The coordinator end to end over real core (Mac `LaunchCoordinatorFFITests`): the real
//! decision, resume/precheck/run, real SQLCipher opens with the plan's cache, the launch
//! screens' file actions, `settings.json`, and the live key store over `MemorySecrets`.
//! Temp folders only; no Credential Manager, no MCP server, no real profile.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use api::launch_decision::RecoveryReason;
use memory::{MeetingStatus, MeetingTrigger, NewMeeting};

use super::live::{CoreMigration, CoreOpener, LiveKeys, SettingsFileLaunchSettings};
use super::*;
use crate::db_key::{DbKeyStore, KeyBytes, Lookup};
use crate::secrets::{MemorySecrets, SecretStore, SYNC_SERVICE};

const GIB: u64 = 1024 * 1024 * 1024;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("whimbrel-launch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn db(&self) -> PathBuf {
        self.0.join("kestrel.sqlite")
    }
    fn models(&self) -> PathBuf {
        self.0.join("models")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Answers screens from `responses`, then quits, so a failure cannot hang.
#[derive(Default)]
struct Headless {
    screens: Mutex<Vec<LaunchScreen>>,
    responses: Mutex<VecDeque<LaunchResponse>>,
    progress: Mutex<Vec<f64>>,
    /// The new-install setup: runs `begin_setup` with this passphrase.
    passphrase: Option<String>,
    /// What the folder and the key store held when the setup started.
    seen_at_setup: Mutex<Option<(usize, bool)>>,
    watch: Option<(PathBuf, Arc<MemorySecrets>)>,
}

impl LaunchPresenter for Headless {
    fn present(&self, screen: &LaunchScreen) -> LaunchResponse {
        self.screens.lock().unwrap().push(screen.clone());
        self.responses.lock().unwrap().pop_front().unwrap_or(LaunchResponse::Quit)
    }
    fn run_onboarding(&self, existing: Option<&KeyBytes>) -> Option<KeyBytes> {
        if let Some((dir, secrets)) = &self.watch {
            let files = std::fs::read_dir(dir).unwrap().count();
            let any_key = ["db-key", "db-key-pending"].iter().any(|a| secrets.get(SYNC_SERVICE, a).unwrap().is_some());
            *self.seen_at_setup.lock().unwrap() = Some((files, any_key));
        }
        let pass = self.passphrase.as_deref()?;
        let material = match existing {
            Some(k) => api::encrypt_setup::setup_for_key(&k.db_key(), pass),
            None => api::encrypt_setup::begin_setup(pass),
        }
        .ok()?;
        assert!(api::encrypt_setup::verify_passphrase(&material.recovery_file, pass));
        Some(KeyBytes::new(*material.key.expose_secret_bytes()))
    }
    fn migration_progress(&self, p: &api::encrypt_migration::Progress) {
        self.progress.lock().unwrap().push(p.overall);
    }
}

type McpLog = Arc<Mutex<Vec<Option<KeyBytes>>>>;

fn launch(
    dir: &Dir,
    keys: Arc<LiveKeys>,
    settings: Arc<SettingsFileLaunchSettings>,
    presenter: Arc<Headless>,
    mcp: McpLog,
) -> LaunchCoordinator<memory::Store> {
    let db = dir.db();
    let observe_db = db.clone();
    let started = std::time::Instant::now();
    LaunchCoordinator::new(Collaborators {
        observe_files: Box::new(move || api::launch_decision::observe_files(&observe_db)),
        decide: Box::new(core_decide),
        keys,
        settings,
        // A 32 GiB computer, whatever runs the test: Faster search applies.
        physical_memory: Box::new(|| 32 * GIB),
        recording: Box::new(|| false),
        migration: Arc::new(CoreMigration::new(&db)),
        opener: Box::new(CoreOpener { db, models: dir.models() }),
        start_mcp_server: Box::new(move |k| mcp.lock().unwrap().push(k.cloned())),
        presenter,
        now_ms: Box::new(move || started.elapsed().as_millis() as u64),
    })
}

fn settings_at(dir: &Dir) -> Arc<SettingsFileLaunchSettings> {
    Arc::new(SettingsFileLaunchSettings(Arc::new(Mutex::new(crate::settings::SettingsFile::load(&dir.0.join("settings.json"))))))
}

fn meeting(title: &str) -> NewMeeting {
    NewMeeting {
        device_id: "dev".into(),
        start_unix_ms: 1000,
        end_unix_ms: None,
        title: title.into(),
        attendees: vec![],
        trigger: MeetingTrigger::Manual,
        audio_path: None,
        status: MeetingStatus::Completed,
    }
}

fn opened(o: LaunchOutcome<memory::Store>) -> LaunchOpened<memory::Store> {
    match o {
        LaunchOutcome::Opened(o) => o,
        other => panic!("expected an open, got {other:?}"),
    }
}

fn found(l: Lookup) -> Option<KeyBytes> {
    match l {
        Lookup::Found(k) => Some(k),
        _ => None,
    }
}

fn header(db: &Path) -> Vec<u8> {
    std::fs::read(db).unwrap()[..16].to_vec()
}

#[test]
fn a_plain_database_migrates_at_the_first_launch_and_opens_keyed_at_the_second() {
    let dir = Dir::new("migrate");
    let id = {
        let store = api::store::open(&dir.db(), &dir.models()).unwrap();
        store.insert_meeting(&meeting("Before encryption")).unwrap()
    };
    let secrets = Arc::new(MemorySecrets::default());
    let keys = Arc::new(LiveKeys(DbKeyStore::new(secrets.clone())));
    // The Settings opt-in: the setup key stored as db-key-pending and the flag set.
    let key = keys.0.create_pending().unwrap();
    let settings = settings_at(&dir);
    settings.set_migrate_at_next_launch(true);
    settings.0.lock().unwrap().set(api::settings::FAST_SEARCH_KEY, Some(true));
    let presenter = Arc::new(Headless::default());
    let mcp: McpLog = Arc::default();

    {
        let first = launch(&dir, keys.clone(), settings.clone(), presenter.clone(), mcp.clone());
        let o = opened(first.run());
        assert_eq!(o.key.as_ref(), Some(&key), "encrypted with the key the recovery setup made");
        assert_eq!(o.store.get_meeting(id).unwrap().unwrap().title, "Before encryption");
        assert_eq!(found(keys.lookup()), Some(key.clone()));
        assert!(matches!(keys.lookup_pending(), Lookup::Absent), "deleted after the keyed open");
        assert!(!settings.migrate_at_next_launch());
        assert_eq!(settings.good_launches(), 0, "not counted before the first window");
        first.first_window_rendered().unwrap().join().unwrap();
        assert_eq!(settings.good_launches(), 1);
        assert_eq!(*mcp.lock().unwrap(), vec![Some(key.clone())]);
        let progress = presenter.progress.lock().unwrap().clone();
        assert!(!progress.is_empty() && progress.windows(2).all(|p| p[0] <= p[1]), "{progress:?}");
        assert_eq!(progress.last(), Some(&1.0));
        assert!(api::encrypt_migration::plain_backup_exists(&api::encrypt_migration::MigrationPaths::beside(&dir.db())));

        // The speed check runs on the store the launch opened (Faster search on: 1 GiB).
        let report = api::speed_check::run_with(
            &o.store,
            &api::speed_check::SpeedContext { open_ms: Some(o.open_ms), physical_ram_bytes: Some(32 * GIB) },
            &|_| {},
        )
        .unwrap();
        assert!(report.encrypted);
        assert_eq!(report.cache_kib, api::settings::FAST_SEARCH_CACHE_KIB, "the plan's cache reached the main store");
        let text = api::speed_check::to_text(&report);
        assert!(text.starts_with("Whimbrel encryption speed check"), "{text}");
        assert!(!text.contains(&*dir.0.to_string_lossy()), "no paths");
    } // the first launch's store closes here
    assert_ne!(header(&dir.db()), b"SQLite format 3\0", "encrypted on disk");

    let second = launch(&dir, keys.clone(), settings.clone(), presenter.clone(), mcp.clone());
    let o = opened(second.run());
    assert_eq!(o.key.as_ref(), Some(&key));
    assert_eq!(o.store.get_meeting(id).unwrap().unwrap().title, "Before encryption");
    second.first_window_rendered().unwrap().join().unwrap();
    assert_eq!(settings.good_launches(), 2);
    assert_eq!(second.notes(), vec![]);
    assert_eq!(*mcp.lock().unwrap(), vec![Some(key.clone()), Some(key)]);
    assert_eq!(*presenter.screens.lock().unwrap(), vec![]);
}

#[test]
fn recovery_over_a_wrong_db_key_opens_and_keeps_the_previous_key() {
    let dir = Dir::new("recover");
    let k = KeyBytes::new(std::array::from_fn(|i| (i as u8).wrapping_mul(13).wrapping_add(5)));
    let wrong = KeyBytes::new([0x5A; 32]);
    let id = {
        let api::store_key::OpenOutcome::Opened(store) = api::store_key::open_store(&dir.db(), &dir.models(), api::store_key::KeyLookup::Found(k.db_key()))
        else {
            panic!("the keyed create failed")
        };
        store.insert_meeting(&meeting("Encrypted with K")).unwrap()
    };
    let secrets = Arc::new(MemorySecrets::default());
    let keys = Arc::new(LiveKeys(DbKeyStore::new(secrets.clone())));
    keys.0.store(&wrong, "db-key").unwrap();
    let settings = settings_at(&dir);
    let presenter = Arc::new(Headless::default());
    presenter.responses.lock().unwrap().push_back(LaunchResponse::RestoredKey(k.clone()));
    let mcp: McpLog = Arc::default();
    let previous = || secrets.get(SYNC_SERVICE, "db-key-previous").unwrap();

    {
        let first = launch(&dir, keys.clone(), settings.clone(), presenter.clone(), mcp.clone());
        let o = opened(first.run());
        assert_eq!(*presenter.screens.lock().unwrap(), vec![LaunchScreen::Recovery(RecoveryReason::KeyRejected)]);
        assert_eq!(o.key.as_ref(), Some(&k));
        assert_eq!(o.store.get_meeting(id).unwrap().unwrap().title, "Encrypted with K");
        assert_eq!(found(keys.lookup()), Some(k.clone()));
        assert_eq!(previous().as_deref(), Some(&wrong.as_bytes()[..]), "the rejected key is kept, never destroyed");
        assert_eq!(first.notes(), vec![]);
        assert_eq!(*mcp.lock().unwrap(), vec![Some(k.clone())]);
    }

    presenter.screens.lock().unwrap().clear();
    let second = launch(&dir, keys.clone(), settings, presenter.clone(), mcp);
    let o = opened(second.run());
    assert_eq!(*presenter.screens.lock().unwrap(), vec![], "no screen at the next launch");
    assert_eq!(o.key.as_ref(), Some(&k));
    assert_eq!(previous().as_deref(), Some(&wrong.as_bytes()[..]));
    drop(o);
    // Delete everything leaves no key behind, previous ones included.
    keys.0.delete().unwrap();
    assert!(matches!(keys.lookup(), Lookup::Absent));
    assert_eq!(previous(), None);
}

#[test]
fn the_never_create_open_leaves_a_missing_database_missing() {
    let dir = Dir::new("nevercreate");
    let opener = CoreOpener { db: dir.db(), models: dir.models() };
    let key = KeyBytes::new([0x33; 32]);
    let (kind, store) = opener.open_keyed(&key, None, true);
    assert!(matches!(kind, OpenKind::Damaged(_)), "{kind:?}");
    assert!(store.is_none());
    assert!(!dir.db().exists());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0, "nothing created");
    let (kind, store) = opener.open_keyed(&key, None, false);
    assert_eq!(kind, OpenKind::Opened);
    drop(store);
    assert_eq!(opener.open_keyed(&key, None, true).0, OpenKind::Opened);
}

#[test]
fn the_file_actions_work_on_real_files() {
    let dir = Dir::new("fileactions");
    let db = dir.db();
    let paths = api::encrypt_migration::MigrationPaths::beside(&db);
    let backup = paths.plain_backup();
    drop(api::store::open(&backup, &dir.models()).unwrap());
    for s in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", backup.display()));
    }
    std::fs::write(format!("{}-wal", db.display()), b"stray").unwrap();
    let m = CoreMigration::new(&db);
    let restored = m.restore_rollback_copy().unwrap();
    assert_eq!(restored.moved_aside.len(), 1, "the stray sidecar is set aside, never deleted");
    assert_eq!(restored.warning, None);
    assert_eq!(header(&db), b"SQLite format 3\0");
    assert!(matches!(m.restore_rollback_copy(), Err(MigrationError::Refused(_))), "the database exists now");
    assert!(matches!(m.reset_encryption_status(), Err(MigrationError::Refused(_))), "no state file");
    assert!(matches!(m.delete_plain_backup(true), Err(MigrationError::Refused(_))), "a plain database");

    // After a real migration the plaintext copy goes only after a keyed open this launch.
    let secrets = Arc::new(MemorySecrets::default());
    let keys = LiveKeys(DbKeyStore::new(secrets));
    let key = keys.0.create_pending().unwrap();
    let mut out = None;
    keys.with_migration_writer(&key, &mut |write| out = Some(m.run(&key, false, write, &|_| {})));
    assert!(matches!(out, Some(Ok(api::encrypt_migration::Phase::Done))), "{out:?}");
    assert!(backup.exists());
    assert!(matches!(m.delete_plain_backup(false), Err(MigrationError::Refused(_))));
    assert!(backup.exists());
    assert!(!m.delete_plain_backup(true).unwrap().is_empty());
    assert!(!backup.exists());
}

#[test]
fn a_new_install_runs_the_recovery_setup_then_opens_an_encrypted_store() {
    let dir = Dir::new("newinstall");
    let secrets = Arc::new(MemorySecrets::default());
    let keys = Arc::new(LiveKeys(DbKeyStore::new(secrets.clone())));
    // settings.json lives elsewhere so the data folder starts empty.
    let settings_dir = Dir::new("newinstall-settings");
    let settings = settings_at(&settings_dir);
    let presenter = Arc::new(Headless {
        passphrase: Some("correct horse battery staple".into()),
        watch: Some((dir.0.clone(), secrets.clone())),
        ..Default::default()
    });
    let mcp: McpLog = Arc::default();
    let c = launch(&dir, keys.clone(), settings, presenter.clone(), mcp.clone());
    let o = opened(c.run());
    assert_eq!(*presenter.seen_at_setup.lock().unwrap(), Some((0, false)), "nothing in the folder and no key before the setup");
    let key = o.key.clone().expect("keyed");
    assert_eq!(found(keys.lookup()), Some(key.clone()));
    assert_eq!(found(keys.lookup_pending()), Some(key.clone()), "stored first as pending; the same key");
    drop(o);
    assert_eq!(memory::db_file_state(&dir.db()).unwrap(), memory::DbFileState::NotPlain, "encrypted header");
    assert_eq!(*mcp.lock().unwrap(), vec![Some(key)]);
    assert_eq!(*presenter.screens.lock().unwrap(), vec![]);
}
