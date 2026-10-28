//! Where Whimbrel keeps its files on Windows — docs/12 §4 "Data directory".
//!
//! Everything lives under `%LOCALAPPDATA%\Whimbrel\data\`. Local, not Roaming: a
//! gigabyte-sized SQLite file must never ride Windows profile roaming. It's a subfolder of
//! the install folder (`%LOCALAPPDATA%\Whimbrel`), so the installer's own `skins\` and
//! `node\` stay apart from the data's, and the uninstaller (which deletes its files by
//! name, then a non-recursive `RMDir`) leaves it alone.
//!
//! Through 0.9.34 the data was in `%LOCALAPPDATA%\Kestrel\`. `settle` moves it here at
//! launch, in one rename (docs/superpowers/plans/2026-10-27-windows-data-folder-whimbrel.md).

use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub root: PathBuf,
    /// The memory store.
    pub db: PathBuf,
    /// Whisper and diarization models (M3.3); the MiniLM directory sits inside it.
    pub models: PathBuf,
    /// The embedding model directory — the memory store's `models_dir` (plan Decision 12).
    pub minilm: PathBuf,
    pub logs: PathBuf,
    pub settings: PathBuf,
    /// Meeting audio and transcripts (M3.3): `system-audio-<stamp>.wav`,
    /// `mic-audio-<stamp>.wav`, `transcript-<stamp>.txt`. Never synced.
    pub recordings: PathBuf,
    /// Skins the user made or imported: one folder per skin, `user-<slug>/` (Task 6; never synced here).
    pub skins: PathBuf,
    /// The old folder, when `root` is the new one: stored paths under it are re-rooted
    /// onto `root` at launch (`state::reroot_stored_paths`).
    pub legacy_root: Option<PathBuf>,
}

/// What `settle` found, for the log.
#[derive(Debug)]
pub enum Settled {
    /// Already in `Whimbrel\data`, or a fresh install.
    InPlace,
    /// The old folder was just renamed to `Whimbrel\data`.
    Moved,
    /// Both folders hold a database. `Whimbrel\data` is used and the old one is left as it is.
    BothExist,
    /// The rename failed (a file in the old folder is open somewhere). The old folder is
    /// used for now, and the next launch tries again.
    MoveFailed(io::Error),
}

/// The data folder through 0.9.34, in `%LOCALAPPDATA%`.
const LEGACY_DIR: &str = "Kestrel";

fn data_dir(local_app_data: &Path) -> PathBuf {
    local_app_data.join("Whimbrel").join("data")
}

fn has_db(dir: &Path) -> bool {
    Paths::under(dir).db.is_file()
}

/// Renames `old` to `new`. Whatever is already in `new` (never a database: `settle` checked)
/// is set aside first, then moved back in wherever the data has no entry of that name.
/// If the rename fails, `new` is put back as it was.
fn move_into_place(old: &Path, new: &Path) -> io::Result<()> {
    std::fs::create_dir_all(new.parent().expect("data_dir has a parent"))?;
    let aside = new.with_file_name(format!("data-before-move-{}", std::process::id()));
    let set_aside = new.exists();
    if set_aside {
        std::fs::rename(new, &aside)?;
    }
    if let Err(e) = std::fs::rename(old, new) {
        if set_aside {
            let _ = std::fs::rename(&aside, new);
        }
        return Err(e);
    }
    if set_aside {
        for entry in std::fs::read_dir(&aside)?.flatten() {
            let to = new.join(entry.file_name());
            if !to.exists() {
                let _ = std::fs::rename(entry.path(), to);
            }
        }
        // Only if everything went back in; a name the data already has stays here.
        let _ = std::fs::remove_dir(&aside);
    }
    Ok(())
}

impl Paths {
    pub fn under(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            db: root.join("kestrel.sqlite"),
            models: root.join("models"),
            minilm: root.join("models").join("minilm"),
            logs: root.join("logs"),
            settings: root.join("settings.json"),
            recordings: root.join("recordings"),
            skins: root.join("skins"),
            legacy_root: None,
            root,
        }
    }

    /// Where the data is now, without moving anything: `%LOCALAPPDATA%\Whimbrel\data`, or
    /// the old folder while it hasn't been moved yet. `None` if the variable is unset
    /// (never on a real Windows session).
    pub fn from_env() -> Option<Self> {
        std::env::var_os("LOCALAPPDATA").map(|dir| Self::found(Path::new(&dir)))
    }

    /// At launch, before anything opens a file in the data folder: `settle` against
    /// `%LOCALAPPDATA%`.
    pub fn settle_from_env() -> Option<(Self, Settled)> {
        std::env::var_os("LOCALAPPDATA").map(|dir| Self::settle(Path::new(&dir)))
    }

    /// Moves the old folder to `Whimbrel\data` if it holds the database and the new one
    /// doesn't, then returns where the data is. The database decides, not the folder: a
    /// development build's staging scripts can create `Whimbrel\data\bin` or `connectors`
    /// before the move, and that mustn't pass for the data. A directory rename on one
    /// volume is atomic, so the data is never split between the two; Windows refuses it
    /// while a file inside is open.
    pub fn settle(local_app_data: &Path) -> (Self, Settled) {
        let (new, old) = (data_dir(local_app_data), local_app_data.join(LEGACY_DIR));
        let settled = match (has_db(&new), has_db(&old)) {
            (true, true) => Settled::BothExist,
            (true, false) | (false, false) => Settled::InPlace,
            (false, true) => match move_into_place(&old, &new) {
                Ok(()) => Settled::Moved,
                Err(e) => Settled::MoveFailed(e),
            },
        };
        (Self::found(local_app_data), settled)
    }

    fn found(local_app_data: &Path) -> Self {
        let (new, old) = (data_dir(local_app_data), local_app_data.join(LEGACY_DIR));
        if !has_db(&new) && has_db(&old) {
            return Self::under(old);
        }
        Self { legacy_root: Some(old), ..Self::under(new) }
    }

    /// Meeting transcription's whisper model (M3.3 Decision 11).
    pub fn whisper_model(&self) -> PathBuf {
        self.models.join("ggml-tiny.en.bin")
    }

    /// Diarization's pyannote segmentation model.
    pub fn segmentation_model(&self) -> PathBuf {
        self.models.join("pyannote-segmentation-3-0.onnx")
    }

    /// Diarization's speaker embedding model (not the MiniLM search model).
    pub fn speaker_embedding_model(&self) -> PathBuf {
        self.models.join("wespeaker_en_voxceleb_resnet34_LM.onnx")
    }

    /// Scratch WAVs for transcribing while a meeting is still recording. Deliberately not
    /// under `recordings`: `api::retention::scan_orphans` treats every unreferenced `.wav`
    /// there as an orphan and deletes it, which would pull slices out from under the
    /// worker. Created on demand and emptied by it.
    pub fn live_slices(&self) -> PathBuf {
        self.root.join("cache").join("live")
    }

    /// Creates the directories (not the files) if missing.
    pub fn ensure_dirs(&self) -> io::Result<()> {
        for dir in [&self.root, &self.models, &self.minilm, &self.logs, &self.recordings, &self.skins] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_rooted_under_one_directory() {
        let p = Paths::under(r"C:\Users\u\AppData\Local\Whimbrel\data");
        assert_eq!(p.db, Path::new(r"C:\Users\u\AppData\Local\Whimbrel\data\kestrel.sqlite"));
        assert_eq!(p.minilm, Path::new(r"C:\Users\u\AppData\Local\Whimbrel\data\models\minilm"));
        assert_eq!(p.settings, Path::new(r"C:\Users\u\AppData\Local\Whimbrel\data\settings.json"));
        for child in [&p.db, &p.models, &p.minilm, &p.logs, &p.settings, &p.recordings, &p.skins] {
            assert!(child.starts_with(&p.root));
        }
    }

    #[test]
    fn from_env_uses_local_not_roaming_appdata() {
        let p = Paths::from_env().expect("LOCALAPPDATA is set on Windows");
        // The old folder only on a PC whose data hasn't moved yet.
        assert!(p.root.ends_with(r"Whimbrel\data") || p.root.ends_with("Kestrel"), "{}", p.root.display());
        let local = std::env::var_os("LOCALAPPDATA").unwrap();
        assert!(p.root.starts_with(local));
    }

    /// A temporary stand-in for `%LOCALAPPDATA%`.
    fn local(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whimbrel-settle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_fresh_install_uses_the_new_folder() {
        let local = local("fresh");
        let (p, settled) = Paths::settle(&local);
        assert!(matches!(settled, Settled::InPlace));
        assert_eq!(p.root, local.join(r"Whimbrel\data"));
        assert_eq!(p.legacy_root, Some(local.join("Kestrel")));
        std::fs::remove_dir_all(&local).unwrap();
    }

    #[test]
    fn the_old_folder_moves_whole_into_the_new_one() {
        let local = local("move");
        let old = local.join("Kestrel");
        std::fs::create_dir_all(old.join("recordings")).unwrap();
        std::fs::write(old.join("kestrel.sqlite"), b"db").unwrap();
        std::fs::write(old.join(r"recordings\system-audio-S.wav"), b"wav").unwrap();
        // The installed app is already in Whimbrel.
        std::fs::create_dir_all(local.join(r"Whimbrel\skins")).unwrap();

        let (p, settled) = Paths::settle(&local);
        assert!(matches!(settled, Settled::Moved), "{settled:?}");
        assert!(!old.exists());
        assert_eq!(std::fs::read(&p.db).unwrap(), b"db");
        assert_eq!(std::fs::read(p.recordings.join("system-audio-S.wav")).unwrap(), b"wav");
        assert!(local.join(r"Whimbrel\skins").is_dir(), "the app's own folders are untouched");

        let (again, settled) = Paths::settle(&local);
        assert!(matches!(settled, Settled::InPlace), "the next launch has nothing to move");
        assert_eq!(again, p);
        std::fs::remove_dir_all(&local).unwrap();
    }

    #[test]
    fn a_new_folder_without_a_database_does_not_pass_for_the_data() {
        // A development build staged its sidecar and connectors before the move.
        let local = local("staged");
        std::fs::create_dir_all(local.join(r"Whimbrel\data\bin")).unwrap();
        std::fs::write(local.join(r"Whimbrel\data\bin\server.exe"), b"staged").unwrap();
        std::fs::create_dir_all(local.join(r"Whimbrel\data\connectors")).unwrap();
        std::fs::create_dir_all(local.join(r"Kestrel\bin")).unwrap();
        std::fs::write(local.join(r"Kestrel\kestrel.sqlite"), b"db").unwrap();
        std::fs::write(local.join(r"Kestrel\bin\server.exe"), b"old").unwrap();

        assert_eq!(Paths::found(&local).root, local.join("Kestrel"), "before the move, the data is where the database is");
        let (p, settled) = Paths::settle(&local);
        assert!(matches!(settled, Settled::Moved), "{settled:?}");
        assert_eq!(std::fs::read(&p.db).unwrap(), b"db");
        assert!(p.root.join("connectors").is_dir(), "staged folders the data lacks are put back");
        assert_eq!(std::fs::read(p.root.join(r"bin\server.exe")).unwrap(), b"old", "the data's own entries win");
        assert!(!local.join("Kestrel").exists());
        std::fs::remove_dir_all(&local).unwrap();
    }

    #[test]
    fn with_both_folders_the_new_one_wins_and_the_old_is_left_alone() {
        let local = local("both");
        std::fs::create_dir_all(local.join("Kestrel")).unwrap();
        std::fs::write(local.join(r"Kestrel\kestrel.sqlite"), b"old").unwrap();
        std::fs::create_dir_all(local.join(r"Whimbrel\data")).unwrap();
        std::fs::write(local.join(r"Whimbrel\data\kestrel.sqlite"), b"new").unwrap();

        let (p, settled) = Paths::settle(&local);
        assert!(matches!(settled, Settled::BothExist));
        assert_eq!(std::fs::read(&p.db).unwrap(), b"new");
        assert_eq!(p.root, local.join(r"Whimbrel\data"));
        assert_eq!(std::fs::read(local.join(r"Kestrel\kestrel.sqlite")).unwrap(), b"old");
        std::fs::remove_dir_all(&local).unwrap();
    }

    #[test]
    fn while_a_file_is_open_the_old_folder_stays_in_use() {
        let local = local("locked");
        let old = local.join("Kestrel");
        std::fs::create_dir_all(&old).unwrap();
        // Shared the way SQLite opens its database: read and write, not delete.
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new().write(true).create(true).truncate(true).share_mode(0x1 | 0x2).open(old.join("kestrel.sqlite")).unwrap();

        let (p, settled) = Paths::settle(&local);
        assert!(matches!(settled, Settled::MoveFailed(_)), "{settled:?}");
        assert_eq!(p.root, old);
        assert_eq!(p.legacy_root, None);
        assert!(!local.join(r"Whimbrel\data").exists(), "nothing was split off");

        drop(held);
        let (p, settled) = Paths::settle(&local);
        assert!(matches!(settled, Settled::Moved), "and the next launch moves it: {settled:?}");
        assert!(p.db.is_file());
        std::fs::remove_dir_all(&local).unwrap();
    }

    #[test]
    fn from_env_never_moves_anything() {
        // `from_env` is what tests and tools call; only `settle` renames.
        let local = local("nomove");
        std::fs::create_dir_all(local.join("Kestrel")).unwrap();
        std::fs::write(local.join(r"Kestrel\kestrel.sqlite"), b"db").unwrap();
        assert_eq!(Paths::found(&local).root, local.join("Kestrel"));
        assert!(local.join("Kestrel").exists());
        std::fs::remove_dir_all(&local).unwrap();
    }

    #[test]
    fn ensure_dirs_creates_every_directory_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!("whimbrel-paths-test-{}", std::process::id()));
        let p = Paths::under(&root);
        p.ensure_dirs().unwrap();
        p.ensure_dirs().unwrap();
        for dir in [&p.root, &p.models, &p.minilm, &p.logs, &p.recordings, &p.skins] {
            assert!(dir.is_dir(), "{} missing", dir.display());
        }
        assert!(!p.db.exists(), "ensure_dirs must not create the database");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
