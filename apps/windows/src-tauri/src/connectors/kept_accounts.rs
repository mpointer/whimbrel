//! The connected Google accounts (Calendar, Gmail) stay connected through Delete everything
//! (Mike, 2026-11-17). Their sign-ins live in Credential Manager, which the delete leaves alone,
//! but the list of connected accounts is the `connector_account` table in the memory database,
//! which goes. So the confirm writes the list to [`FILE_NAME`] in the data folder (kept by
//! `delete_everything::KEEP`), and the launch that opens the new database adds it back
//! ([`restore_at_launch`]), then removes the file.
//!
//! The file exists only between a confirmed delete's restart and the next launch that opens a
//! database: the confirm removes it again when it stops before the restart, and the restore runs
//! before anything else reads or changes the list. So the defaults it records are always the
//! user's latest choice, and a restore that a crash cut short simply runs again.
//!
//! The file holds the accounts' email addresses in plain text, beside the database. It lives in
//! the data folder under %LOCALAPPDATA%, whose inherited permissions already limit it to the user
//! (as for `settings.json`), and it exists only for that one restart. No tokens are in it.
//!
//! An account whose sign-in is no longer in Credential Manager is not added back: it would show
//! as connected with nothing to connect with.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use memory::Store;
use serde::{Deserialize, Serialize};

use super::accounts::{account_name_for, Kind};
use crate::secrets::{SecretStore, GOOGLE_OAUTH_SERVICE};
use crate::store_actor::StoreHandle;

/// In the data folder, kept by the delete; read and removed when the new database opens.
pub const FILE_NAME: &str = ".kept-google-accounts.json";
/// Written first, then renamed over [`FILE_NAME`], so a crash never leaves half a list.
const TEMP_NAME: &str = ".kept-google-accounts.json.tmp";

/// The Google connectors whose accounts are carried across.
const KINDS: [Kind; 2] = [Kind::Calendar, Kind::Gmail];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptAccount {
    /// The registry's connector id (`Kind::id`).
    pub connector: String,
    pub email: String,
    pub default: bool,
}

/// Every connected Google account, in connection order per connector.
pub fn list(store: &Store) -> Result<Vec<KeptAccount>, String> {
    let mut all = Vec::new();
    for kind in KINDS {
        let accounts = api::connectors::accounts::list(store, kind.id()).map_err(|e| e.to_string())?;
        for email in accounts.emails {
            let default = accounts.default_email.as_deref() == Some(email.as_str());
            all.push(KeptAccount { connector: kind.id().to_string(), email, default });
        }
    }
    Ok(all)
}

/// Removes the file (and a temp file a crash left), if there is one.
pub fn discard(root: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(root.join(TEMP_NAME));
    match std::fs::remove_file(root.join(FILE_NAME)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Writes the list for the launch after the delete, durably: a temp file, flushed to disk, then
/// renamed over the old one. Nothing connected: no file (and none left from an earlier delete).
pub fn write(root: &Path, accounts: &[KeptAccount]) -> std::io::Result<()> {
    if accounts.is_empty() {
        return discard(root);
    }
    let temp = root.join(TEMP_NAME);
    let mut f = std::fs::File::create(&temp)?;
    f.write_all(&serde_json::to_vec(accounts).map_err(std::io::Error::other)?)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&temp, root.join(FILE_NAME))
}

/// The confirm's step: `Some` list (read from the open database) is written for the new one.
/// `None` (no database, so no list) removes any older file, so the accounts are not listed again
/// after this delete, as the dialog says.
pub fn keep(root: &Path, accounts: Option<&[KeptAccount]>) -> std::io::Result<()> {
    match accounts {
        Some(list) => write(root, list),
        None => discard(root),
    }
}

/// The account registry `restore` writes to: the database, or a fake in tests.
pub trait Registry {
    fn add(&self, connector: &str, email: &str, now_unix_ms: i64) -> Result<bool, String>;
    fn set_default(&self, connector: &str, email: &str) -> Result<(), String>;
}

/// The error's SQLite code only, never its text, so no address can reach the log through it.
macro_rules! code_only {
    ($e:expr) => {
        $e.sqlite_error_code().map_or_else(|| "database error".to_string(), |c| format!("{c:?}"))
    };
}

impl Registry for Store {
    fn add(&self, connector: &str, email: &str, now_unix_ms: i64) -> Result<bool, String> {
        api::connectors::accounts::add(self, connector, email, now_unix_ms).map_err(|e| code_only!(e))
    }

    fn set_default(&self, connector: &str, email: &str) -> Result<(), String> {
        api::connectors::accounts::set_default(self, connector, email).map_err(|e| code_only!(e))
    }
}

fn kind_of(connector: &str) -> Option<Kind> {
    KINDS.into_iter().find(|k| k.id() == connector)
}

/// Adds the kept accounts back to the database that just opened, with their defaults, and
/// removes the file. An account already listed is not added twice (a restore a crash cut short,
/// or a delete that did not run and opened the old database), but its kept default is applied
/// again. Any failure keeps the file, so the next launch tries again. Returns the accounts it
/// added.
pub fn restore(root: &Path, registry: &dyn Registry, secrets: &dyn SecretStore, now_unix_ms: i64) -> Vec<KeptAccount> {
    let path = root.join(FILE_NAME);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %e.kind(), "could not read the kept Google accounts file; trying again at the next launch");
            }
            return Vec::new();
        }
    };
    let kept: Vec<KeptAccount> = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        // The category only: serde's message can quote a value from the file, an address.
        tracing::warn!(error = ?e.classify(), "the kept Google accounts file is unreadable; they need connecting again");
        Vec::new()
    });
    let mut added = Vec::new();
    let mut listed = Vec::new();
    for account in &kept {
        let Some(kind) = kind_of(&account.connector) else { continue };
        let signed_in = matches!(secrets.get_string(GOOGLE_OAUTH_SERVICE, &account_name_for(kind, &account.email)), Ok(Some(_)));
        if !signed_in {
            tracing::warn!(connector = kind.id(), "a kept Google account's sign-in is gone; it needs connecting again");
            continue;
        }
        match registry.add(kind.id(), &account.email, now_unix_ms) {
            Ok(true) => added.push(account.clone()),
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(error = %e, "could not add a kept Google account back; trying again at the next launch");
                return added;
            }
        }
        listed.push(account);
    }
    for account in listed.into_iter().filter(|a| a.default) {
        if let Err(e) = registry.set_default(&account.connector, &account.email) {
            tracing::warn!(error = %e, "could not make a kept Google account the default again; trying again at the next launch");
            return added;
        }
    }
    if let Err(e) = std::fs::remove_file(&path) {
        tracing::warn!(error = %e.kind(), "could not remove the kept Google accounts file");
    }
    added
}

/// The launch's call, once the launch decided (`app::finish_launch`): with a database (plain or
/// keyed) the kept accounts are listed again; without one the file waits for a launch that opens
/// it. Returns how many were added back.
pub fn restore_at_launch(root: &Path, store: Option<&StoreHandle>, secrets: Arc<dyn SecretStore>) -> usize {
    let Some(store) = store else { return 0 };
    let root = root.to_path_buf();
    let now = chrono::Utc::now().timestamp_millis();
    store.call_blocking(move |s| restore(&root, s, &*secrets, now).len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    struct Tmp(std::path::PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn root(name: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("whimbrel-kept-accounts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }

    fn sign_in(secrets: &MemorySecrets, kind: Kind, email: &str) {
        secrets.set_string(GOOGLE_OAUTH_SERVICE, &account_name_for(kind, email), "{}").unwrap();
    }

    fn connected(store: &Store, kind: Kind) -> api::connectors::accounts::Accounts {
        api::connectors::accounts::list(store, kind.id()).unwrap()
    }

    fn kept(connector: Kind, email: &str, default: bool) -> KeptAccount {
        KeptAccount { connector: connector.id().into(), email: email.into(), default }
    }

    #[test]
    fn the_accounts_and_their_defaults_come_back_in_the_new_database() {
        let t = root("roundtrip");
        let old = Store::open_in_memory().unwrap();
        for (kind, email) in [(Kind::Calendar, "a@example.com"), (Kind::Calendar, "b@example.com"), (Kind::Gmail, "b@example.com")] {
            api::connectors::accounts::add(&old, kind.id(), email, 1).unwrap();
        }
        api::connectors::accounts::set_default(&old, Kind::Calendar.id(), "b@example.com").unwrap();
        write(&t.0, &list(&old).unwrap()).unwrap();
        assert!(!t.0.join(TEMP_NAME).exists(), "the temp file was renamed into place");

        let secrets = MemorySecrets::default();
        for (kind, email) in [(Kind::Calendar, "a@example.com"), (Kind::Calendar, "b@example.com"), (Kind::Gmail, "b@example.com")] {
            sign_in(&secrets, kind, email);
        }
        let new = Store::open_in_memory().unwrap();
        assert_eq!(restore(&t.0, &new, &secrets, 2).len(), 3);
        assert_eq!(connected(&new, Kind::Calendar), connected(&old, Kind::Calendar), "same accounts, same order, same default");
        assert_eq!(connected(&new, Kind::Gmail), connected(&old, Kind::Gmail));
        assert!(!t.0.join(FILE_NAME).exists(), "the file goes once they are back");
        assert!(restore(&t.0, &new, &secrets, 3).is_empty(), "nothing to do at the next launch");
    }

    #[test]
    fn an_account_whose_sign_in_is_gone_is_not_added_back() {
        let t = root("no-sign-in");
        write(&t.0, &[kept(Kind::Gmail, "gone@example.com", true), kept(Kind::Gmail, "here@example.com", false)]).unwrap();
        let secrets = MemorySecrets::default();
        sign_in(&secrets, Kind::Gmail, "here@example.com");
        let new = Store::open_in_memory().unwrap();
        restore(&t.0, &new, &secrets, 1);
        assert_eq!(connected(&new, Kind::Gmail).emails, ["here@example.com"]);
        assert_eq!(connected(&new, Kind::Gmail).default_email.as_deref(), Some("here@example.com"), "the first one back is the default");
    }

    /// A crash after the accounts were added but before the default was set: the next launch
    /// finds them listed (the first one made default by `add`) and still applies the kept default.
    #[test]
    fn a_restore_cut_short_gets_its_default_at_the_next_launch() {
        let t = root("partial");
        write(&t.0, &[kept(Kind::Calendar, "a@example.com", false), kept(Kind::Calendar, "b@example.com", true)]).unwrap();
        let secrets = MemorySecrets::default();
        sign_in(&secrets, Kind::Calendar, "a@example.com");
        sign_in(&secrets, Kind::Calendar, "b@example.com");
        let new = Store::open_in_memory().unwrap();
        api::connectors::accounts::add(&new, Kind::Calendar.id(), "a@example.com", 1).unwrap();
        api::connectors::accounts::add(&new, Kind::Calendar.id(), "b@example.com", 1).unwrap();
        assert_eq!(connected(&new, Kind::Calendar).default_email.as_deref(), Some("a@example.com"));
        assert!(restore(&t.0, &new, &secrets, 2).is_empty(), "both were already listed");
        assert_eq!(connected(&new, Kind::Calendar).default_email.as_deref(), Some("b@example.com"));
        assert!(!t.0.join(FILE_NAME).exists());
    }

    /// A delete that did not run opens the old database, which still lists them, unchanged.
    #[test]
    fn the_old_database_that_still_lists_them_stays_as_it_was() {
        let t = root("did-not-run");
        let store = Store::open_in_memory().unwrap();
        api::connectors::accounts::add(&store, Kind::Calendar.id(), "a@example.com", 1).unwrap();
        api::connectors::accounts::add(&store, Kind::Calendar.id(), "b@example.com", 1).unwrap();
        api::connectors::accounts::set_default(&store, Kind::Calendar.id(), "b@example.com").unwrap();
        let before = connected(&store, Kind::Calendar);
        write(&t.0, &list(&store).unwrap()).unwrap();
        let secrets = MemorySecrets::default();
        sign_in(&secrets, Kind::Calendar, "a@example.com");
        sign_in(&secrets, Kind::Calendar, "b@example.com");
        assert!(restore(&t.0, &store, &secrets, 2).is_empty());
        assert_eq!(connected(&store, Kind::Calendar), before);
        assert!(!t.0.join(FILE_NAME).exists());
    }

    /// A registry that fails the n-th add (1-based), and records what it was asked.
    struct Failing {
        fail_at: usize,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl Registry for Failing {
        fn add(&self, _: &str, email: &str, _: i64) -> Result<bool, String> {
            let mut calls = self.calls.borrow_mut();
            calls.push(format!("add {email}"));
            if calls.len() == self.fail_at {
                Err("database is locked".into())
            } else {
                Ok(true)
            }
        }

        fn set_default(&self, _: &str, email: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("default {email}"));
            Ok(())
        }
    }

    #[test]
    fn an_add_that_fails_stops_and_keeps_the_file_for_the_next_launch() {
        let t = root("add-fails");
        write(&t.0, &[kept(Kind::Calendar, "a@example.com", false), kept(Kind::Calendar, "b@example.com", true), kept(Kind::Gmail, "c@example.com", true)]).unwrap();
        let secrets = MemorySecrets::default();
        for (kind, email) in [(Kind::Calendar, "a@example.com"), (Kind::Calendar, "b@example.com"), (Kind::Gmail, "c@example.com")] {
            sign_in(&secrets, kind, email);
        }
        let registry = Failing { fail_at: 2, calls: Default::default() };
        let added = restore(&t.0, &registry, &secrets, 1);
        assert_eq!(added, [kept(Kind::Calendar, "a@example.com", false)]);
        assert_eq!(*registry.calls.borrow(), ["add a@example.com", "add b@example.com"], "nothing after the failure, no default");
        assert!(t.0.join(FILE_NAME).is_file(), "the next launch tries again");
    }

    #[test]
    fn nothing_connected_writes_no_file_and_clears_an_old_one() {
        let t = root("empty");
        std::fs::write(t.0.join(FILE_NAME), b"[]").unwrap();
        write(&t.0, &[]).unwrap();
        assert!(!t.0.join(FILE_NAME).exists());
        write(&t.0, &[]).unwrap();
    }

    /// Without the database there is no list, and the dialog says the accounts need connecting
    /// again: an older file must not list them again after this delete.
    #[test]
    fn no_list_removes_an_older_file() {
        let t = root("no-list");
        write(&t.0, &[kept(Kind::Gmail, "old@example.com", true)]).unwrap();
        std::fs::write(t.0.join(TEMP_NAME), b"half").unwrap();
        keep(&t.0, None).unwrap();
        assert!(!t.0.join(FILE_NAME).exists() && !t.0.join(TEMP_NAME).exists());
        keep(&t.0, None).unwrap();
        keep(&t.0, Some(&[kept(Kind::Gmail, "new@example.com", false)])).unwrap();
        assert!(t.0.join(FILE_NAME).is_file());
    }

    #[test]
    fn an_unreadable_file_adds_nothing_and_goes() {
        let t = root("garbled");
        std::fs::write(t.0.join(FILE_NAME), b"not json").unwrap();
        let store = Store::open_in_memory().unwrap();
        assert!(restore(&t.0, &store, &MemorySecrets::default(), 1).is_empty());
        assert!(!t.0.join(FILE_NAME).exists());
    }

    /// The launch's call: run on an opened database (plain or keyed alike: both arrive as a
    /// store handle), skipped without one, which leaves the file for a later launch.
    #[test]
    fn the_launch_restores_with_a_database_and_waits_without_one() {
        let t = root("launch");
        write(&t.0, &[kept(Kind::Gmail, "a@example.com", true)]).unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        sign_in(&secrets, Kind::Gmail, "a@example.com");
        assert_eq!(restore_at_launch(&t.0, None, secrets.clone()), 0);
        assert!(t.0.join(FILE_NAME).is_file(), "no database: the file waits");
        let store = StoreHandle::spawn(|| Store::open_in_memory().map_err(|e| e.to_string())).unwrap();
        assert_eq!(restore_at_launch(&t.0, Some(&store), secrets), 1);
        assert!(!t.0.join(FILE_NAME).exists());
        let listed = store.call_blocking(|s| connected(s, Kind::Gmail)).unwrap();
        assert_eq!(listed.emails, ["a@example.com"]);
    }

    /// `finish_launch` hands every opened database to the restore, before `AppState` starts
    /// anything that reads the list (a source check: it needs a running Tauri app).
    #[test]
    fn finish_launch_restores_before_the_app_starts() {
        let src = include_str!("../app/mod.rs").replace("\r\n", "\n");
        let body = src.split("fn finish_launch(").nth(1).unwrap();
        let restore = body.find("kept_accounts::restore_at_launch(&paths.root, store.as_ref(),").expect("finish_launch restores");
        let start = body.find("AppState::start(").expect("finish_launch starts the app");
        assert!(restore < start);
        let opened = body.find("LaunchOutcome::Opened(o) =>").unwrap();
        let without = body.find("LaunchOutcome::OpenedWithoutStore => (None, None)").expect("no store without the database");
        assert!(opened < restore && without < restore, "both outcomes reach it");
    }
}
