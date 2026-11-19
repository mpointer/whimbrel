//! Settings > Capture > "Files kept aside" (X4 Task 13c item 3, Mike's decision 2026-11-16; the
//! Mac's Your data row). The encryption work sometimes sets `<db>.foreign-<digits>` files aside
//! beside the memory database (core's `api::foreign_files`); Whimbrel never uses them again.
//! The card shows only while some exist, with their count and total size, and deletes them
//! only when the user confirms. Never automatic.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::error::CommandError;
use crate::l10n::tr;
use crate::state::AppState;

/// The only window that may delete them (Settings lives in it).
const MAIN_WINDOW: &str = "main";

/// What the card shows: nothing at all while `count` is 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForeignFilesView {
    pub count: u32,
    pub bytes: i64,
}

/// What a delete did, from core's fresh listing at that moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForeignFilesDeleted {
    pub removed: u32,
    pub removed_bytes: i64,
    /// Names that matched but could not be removed.
    pub failed: Vec<String>,
}

pub fn view(files: &[api::foreign_files::ForeignFile]) -> ForeignFilesView {
    ForeignFilesView { count: files.len() as u32, bytes: files.iter().map(|f| f.size as i64).sum() }
}

pub fn deleted(report: std::io::Result<api::foreign_files::DeleteReport>) -> Result<ForeignFilesDeleted, CommandError> {
    match report {
        Ok(r) => Ok(ForeignFilesDeleted { removed: r.removed.len() as u32, removed_bytes: r.removed_bytes() as i64, failed: r.failed }),
        Err(e) => {
            tracing::warn!(error = %e, "could not list the files kept aside; nothing deleted");
            Err(CommandError::store(tr("Whimbrel could not read the data folder, so nothing was deleted. Try again.", &[])))
        }
    }
}

/// The delete goes ahead only when core finds as many files as the confirmation named, so a
/// changed set is usually caught. Not a guarantee: a file could appear between this count and
/// core's own listing, or one go while another appears. These are leftovers no process writes
/// in normal use, so the check is a guard, not a lock. `None`: go ahead.
pub fn changed_since_confirmed(found: usize, confirmed: u32) -> Option<CommandError> {
    (found != confirmed as usize).then(|| CommandError::invalid_input(tr("The files kept aside changed. Check the new count, then delete again.", &[])))
}

/// Core needs the database's ABSOLUTE path (a relative one lists nothing); the data folder's is.
#[tauri::command]
pub fn foreign_files_list(state: State<'_, AppState>) -> ForeignFilesView {
    view(&api::foreign_files::list(&state.paths.db))
}

/// The checks, then core's delete: a folder that can't be read says so (core's `list` would read
/// it as "no files", which the card would show as "changed"); a count other than the confirmed
/// one deletes nothing.
pub fn delete_confirmed(db: &std::path::Path, confirmed: u32) -> Result<ForeignFilesDeleted, CommandError> {
    let folder = db.parent().unwrap_or(db);
    if let Err(e) = std::fs::read_dir(folder) {
        return deleted(Err(e));
    }
    if let Some(e) = changed_since_confirmed(api::foreign_files::list(db).len(), confirmed) {
        return Err(e);
    }
    deleted(api::foreign_files::delete_all(db))
}

/// Confirmed in the card for `confirmed` files ([`delete_confirmed`]).
#[tauri::command]
pub fn foreign_files_delete(window: tauri::WebviewWindow, state: State<'_, AppState>, confirmed: u32) -> Result<ForeignFilesDeleted, CommandError> {
    if window.label() != MAIN_WINDOW {
        return Err(CommandError::invalid_input(tr("These files can only be deleted in Settings.", &[])));
    }
    delete_confirmed(&state.paths.db, confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removed even when an assertion fails first.
    struct Tmp(std::path::PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_card_counts_every_file_and_adds_their_sizes() {
        let guard = Tmp(std::env::temp_dir().join(format!("whimbrel-foreign-{}", std::process::id())));
        let dir = guard.0.clone();
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("kestrel.sqlite");
        assert_eq!(view(&api::foreign_files::list(&db)), ForeignFilesView { count: 0, bytes: 0 }, "none: the card is hidden");
        std::fs::write(dir.join("kestrel.sqlite.foreign-1"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("kestrel.sqlite.foreign-1-wal"), vec![0u8; 20]).unwrap();
        std::fs::write(dir.join("kestrel.sqlite.foreign-x"), b"look-alike").unwrap();
        assert_eq!(view(&api::foreign_files::list(&db)), ForeignFilesView { count: 2, bytes: 120 });
        let out = deleted(api::foreign_files::delete_all(&db)).unwrap();
        assert_eq!(out, ForeignFilesDeleted { removed: 2, removed_bytes: 120, failed: vec![] });
        assert!(dir.join("kestrel.sqlite.foreign-x").is_file(), "never a look-alike");
    }

    #[test]
    fn the_count_check_runs_before_any_delete_and_a_missing_folder_is_a_read_error() {
        let guard = Tmp(std::env::temp_dir().join(format!("whimbrel-foreign-check-{}", std::process::id())));
        let dir = guard.0.clone();
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("kestrel.sqlite");
        std::fs::write(dir.join("kestrel.sqlite.foreign-1"), b"a").unwrap();
        std::fs::write(dir.join("kestrel.sqlite.foreign-2"), b"b").unwrap();
        let e = delete_confirmed(&db, 1).unwrap_err();
        assert!(format!("{e:?}").contains("changed"), "{e:?}");
        assert!(dir.join("kestrel.sqlite.foreign-1").is_file() && dir.join("kestrel.sqlite.foreign-2").is_file(), "nothing deleted on a mismatch");
        assert_eq!(delete_confirmed(&db, 2).unwrap().removed, 2);
        let gone = delete_confirmed(&dir.join("no-such-folder").join("kestrel.sqlite"), 0).unwrap_err();
        assert!(format!("{gone:?}").contains("nothing was deleted"), "a read error, not \"changed\": {gone:?}");
    }

    #[test]
    fn the_delete_needs_the_count_the_confirmation_named() {
        assert!(changed_since_confirmed(2, 2).is_none());
        assert!(changed_since_confirmed(3, 2).is_some_and(|e| format!("{e:?}").contains("changed")), "a file that appeared since is never deleted unseen");
        assert!(changed_since_confirmed(1, 2).is_some());
    }

    #[test]
    fn the_missing_library_help_page_is_one_whimbrel_opens() {
        assert!(crate::commands::ai::external_url_allowed(crate::commands::launch::RESTORE_HELP_URL));
    }

    #[test]
    fn a_folder_that_cannot_be_read_says_nothing_was_deleted() {
        let e = deleted(Err(std::io::Error::other("denied"))).unwrap_err();
        assert!(format!("{e:?}").contains("nothing was deleted"), "{e:?}");
    }
}
