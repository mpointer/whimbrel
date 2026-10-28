//! Opening the memory store the way a shell must: open, then run the one-time
//! startup recovery sweep. Moved from `core/ffi::whimbrel_memory_open` (M3.0).

use std::path::Path;

/// Opens (creating if needed) a memory store at `path`. `models_dir` may be empty —
/// "no embedding model available," which the store handles the same way as a directory
/// missing the model files: gracefully (see `core/memory`'s `EmbeddingModel::load`).
///
/// Also performs the one-time startup recovery sweep
/// (`Store::release_stale_executing_routine_runs`), returning any run stranded
/// `Executing` by a crash mid-write-action back to `PendingApproval`. This is the ONLY
/// place that sweep may run: a shell calls this once at its own launch, so nothing can
/// legitimately be in flight yet. Processes that merely open the same database file
/// (notably the `whimbrel-mcp-server` subprocess, which the user can start at any time)
/// must use `memory::Store::open` directly and never run it — see that method's doc
/// comment. A sweep failure is an open failure.
pub fn open(path: &Path, models_dir: &Path) -> rusqlite::Result<memory::Store> {
    let store = memory::Store::open(path, models_dir)?;
    store.release_stale_executing_routine_runs()?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_the_database_and_runs_the_startup_sweep() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir.path().join("t.sqlite"), Path::new("")).unwrap();
        assert!(store.list_routines().unwrap().is_empty());
    }
}
