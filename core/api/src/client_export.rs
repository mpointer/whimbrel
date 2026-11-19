//! "Export everything about this client" (X6 client controls, spec section 6).
//!
//! [`export_client`] writes one folder, `Whimbrel client export <date>/` (a same-day second export gets
//! ` 2`, ` 3`, ...; the folder name never carries the client's name), inside the folder the user picked. It
//! holds exactly what the client inventory lists as the client's ([`crate::client_inventory`], so export,
//! delete and keep-for-N-months cover the same set, merged seeds included) and the possible matches the user
//! ticked that the inventory still offers, in the formats of "Export everything" (`your_data`): meetings as
//! Markdown (summary, decisions, action items, open questions, the meeting's notes and the transcript),
//! `emails.jsonl`, `calendar.jsonl` (removed events too), `captures.jsonl`, `episodes.jsonl`,
//! `conversations.jsonl`, `steps.jsonl` (with each step's note) and `topic.json` (the client's topic page).
//! A meeting shared with another client is exported whole; the other client's own material (its emails,
//! steps, meetings) is never included, since only this client's inventory is read.
//!
//! The folder's path is recorded in `client_export` (with an opaque hash of the client's key, never its
//! name) before anything is written into it, so a client delete can list it and, when asked, remove it, even
//! after an export that stopped part-way. Nothing here logs a path or any client text.
//!
//! The work is split so a caller that guards the store with a lock can release it for the slow part:
//! [`write_client_export`] reads the store and writes the text files; [`copy_client_media`] copies the
//! optional audio and slide images without the store. [`export_client`] runs both.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::client_inventory::{inventory, ClientItem, ItemKind};
use crate::envelope::{ApiError, ApiResult};
use crate::your_data::{file_stem, inside, meeting_markdown, rfc3339, ymd};

/// What to export beyond the client's linked material.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// The possible matches the user ticked. Only those the inventory still offers are exported.
    pub include_possible: Vec<ClientItem>,
    /// Copy the meetings' audio and slide images (off by default: they are large).
    pub include_media: bool,
    /// The only folders audio and slide images are copied from (the media folder setting). A file elsewhere,
    /// or reached through a symlink out of these, is not copied. Empty: no media is copied.
    pub media_roots: Vec<PathBuf>,
}

/// The media one meeting's rows name, to be copied into `media/<dest>/`. Paths are as stored, unchecked.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaToCopy {
    /// A file-name-safe folder name (the meeting file's stem).
    pub dest: String,
    pub audio: Vec<PathBuf>,
    pub slides_dir: Option<PathBuf>,
}

/// What [`write_client_export`] wrote, and the media still to copy.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientExport {
    /// The export folder (canonical, recorded in `client_export`).
    pub folder: PathBuf,
    /// Empty unless media was asked for.
    pub media: Vec<MediaToCopy>,
}

/// What [`copy_client_media`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaCopied {
    pub files: usize,
    /// Named media not copied on purpose: missing, not a plain file, outside the media folders, or a file of
    /// that name already in the export.
    pub skipped: usize,
    /// Media that should have been copied but could not be (unreadable, disk full). The text export is still
    /// complete; the caller says the media copy is partial.
    pub failed: usize,
}

/// A finished export: the folder and how the media copy went.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportDone {
    pub folder: PathBuf,
    pub media: MediaCopied,
}

impl ExportDone {
    /// The text is complete but some media could not be copied.
    pub fn partial(&self) -> bool {
        self.media.failed > 0
    }
}

/// The export folder's name: never the client's name.
const FOLDER_PREFIX: &str = "Whimbrel client export ";
/// Same-day exports tried before giving up.
const MAX_SUFFIX: u32 = 1_000;

fn db(_: rusqlite::Error) -> ApiError {
    ApiError::ProviderFailure
}

fn ioe(_: io::Error) -> ApiError {
    ApiError::ProviderFailure
}

/// A new folder, readable by this user only on unix (0700).
pub(crate) fn make_private_dir(p: &Path) -> io::Result<()> {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut b = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(p)
}

/// A new file (never an existing one or a symlink's target), readable by this user only on unix (0600).
pub(crate) fn new_private_file(p: &Path) -> io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(p)
}

fn write_new(p: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    let mut f = new_private_file(p)?;
    f.write_all(text.as_bytes())?;
    f.flush()
}

/// Exports everything about client `key` into a new `Whimbrel client export <date>/` folder inside
/// `dest_parent` and returns the folder. `Err(ApiError::NotFound)` when `key` is not a live client. A failure
/// while writing the text removes the half-written folder and its record. Media that could not be copied does
/// not fail the export (the text is complete); [`export_client_with_media`] says so.
pub fn export_client(
    store: &memory::Store,
    key: &str,
    dest_parent: &Path,
    opts: &ExportOptions,
) -> Result<PathBuf, ApiError> {
    export_client_with_media(store, key, dest_parent, opts).map(|d| d.folder)
}

/// [`export_client`], also returning how the media copy went ([`ExportDone::partial`]).
pub fn export_client_with_media(
    store: &memory::Store,
    key: &str,
    dest_parent: &Path,
    opts: &ExportOptions,
) -> ApiResult<ExportDone> {
    let export = write_client_export(store, key, dest_parent, opts, crate::util::now_unix_ms())?;
    let media = copy_client_media(&export, &opts.media_roots)?;
    Ok(ExportDone { folder: export.folder, media })
}

/// The part of [`export_client`] that needs the store: creates and records the folder, writes the text
/// files, and lists the media to copy (when `opts.include_media`). On error nothing is left: the folder is
/// removed and its record dropped (if the folder cannot be removed, the record stays so a client delete can
/// still find it).
pub fn write_client_export(
    store: &memory::Store,
    key: &str,
    dest_parent: &Path,
    opts: &ExportOptions,
    now_unix_ms: i64,
) -> ApiResult<ClientExport> {
    let inv = inventory(store, key)?;
    let topic_id = store
        .list_seed_topics()
        .map_err(db)?
        .into_iter()
        .find(|s| s.key == key)
        .map(|s| s.topic_id)
        .ok_or(ApiError::NotFound)?;
    // Linked items, then the ticked possible matches the inventory still offers (never anything else).
    let mut items: Vec<&ClientItem> = inv.linked.iter().collect();
    for t in &opts.include_possible {
        if inv.possible.iter().any(|p| p.kind == t.kind && p.id == t.id)
            && !items.iter().any(|x| x.kind == t.kind && x.id == t.id)
        {
            items.push(t);
        }
    }

    let folder = new_folder(dest_parent, now_unix_ms)?;
    let path = folder.to_string_lossy().into_owned();
    // Recorded before anything is written, so even a part-written folder can be found and removed.
    if let Err(e) = store.record_client_export(key, &path, now_unix_ms) {
        let _ = fs::remove_dir(&folder);
        return Err(db(e));
    }
    match write_text(store, &folder, items, topic_id, opts, now_unix_ms) {
        Ok(media) => Ok(ClientExport { folder, media }),
        Err(e) => {
            // Ours, just made: a real folder at its canonical path. Removed only then, record dropped only then.
            let ours = fs::symlink_metadata(&folder).map(|m| m.file_type().is_dir()).unwrap_or(false)
                && folder.canonicalize().is_ok_and(|c| c == folder);
            if ours && fs::remove_dir_all(&folder).is_ok() {
                let _ = store.forget_client_export(&path);
            }
            Err(e)
        }
    }
}

fn write_text(
    store: &memory::Store,
    folder: &Path,
    mut items: Vec<&ClientItem>,
    topic_id: i64,
    opts: &ExportOptions,
    now_unix_ms: i64,
) -> ApiResult<Vec<MediaToCopy>> {
    let meetings_dir = inside(folder, "meetings")?;
    make_private_dir(&meetings_dir).map_err(ioe)?;
    // Lower-cased: most disks (APFS, NTFS) treat "Kickoff" and "kickoff" as one name.
    let mut used: HashSet<String> = HashSet::new();
    let mut media = Vec::new();
    let mut emails = String::new();
    let mut calendar = String::new();
    let mut captures = String::new();
    let mut episodes = String::new();
    let mut conversations = String::new();
    let mut steps = String::new();
    items.sort_by(|a, b| a.date_unix_ms.cmp(&b.date_unix_ms).then(a.id.cmp(&b.id)));
    for item in items {
        let row_id = || item.id.parse::<i64>().map_err(|_| ApiError::ProviderFailure);
        match item.kind {
            ItemKind::Meeting => {
                let Some(m) = store.get_meeting(row_id()?).map_err(db)? else { continue };
                let (md, _) = meeting_markdown(store, &m)?;
                let base = format!("{} {}", ymd(m.start_unix_ms), file_stem(&m.title));
                let mut n = 1;
                let stem = loop {
                    let stem = if n == 1 { base.clone() } else { format!("{base} ({n})") };
                    n += 1;
                    if !used.insert(stem.to_lowercase()) {
                        continue;
                    }
                    match write_new(&inside(&meetings_dir, &format!("{stem}.md"))?, &md) {
                        Ok(()) => break stem,
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n < 10_000 => continue,
                        Err(e) => return Err(ioe(e)),
                    }
                };
                if opts.include_media {
                    let paths = store.meeting_media_paths(m.id).map_err(db)?;
                    if !paths.audio.is_empty() || paths.slides_dir.is_some() {
                        media.push(MediaToCopy {
                            dest: stem,
                            audio: paths.audio.into_iter().map(PathBuf::from).collect(),
                            slides_dir: paths.slides_dir.map(PathBuf::from),
                        });
                    }
                }
            }
            ItemKind::Email | ItemKind::CalendarEvent => {
                let (want, out) = match item.kind {
                    ItemKind::Email => ("email", &mut emails),
                    _ => ("calendar_event", &mut calendar),
                };
                if let Some(row) = store.export_item_row(row_id()?).map_err(db)?.filter(|r| r.kind == want) {
                    line(out, &row)?;
                }
            }
            ItemKind::Capture => {
                // `i:<items.id>`: held only in `items`. Otherwise a `capture_item.id`: its `items` twin when
                // there is one, else the `capture_item` row itself.
                let row = match item.id.strip_prefix("i:") {
                    Some(n) => {
                        let n = n.parse::<i64>().map_err(|_| ApiError::ProviderFailure)?;
                        store.export_item_row(n).map_err(db)?
                    }
                    None => {
                        let cap = row_id()?;
                        match store.capture_items_id(cap).map_err(db)? {
                            Some(n) => store.export_item_row(n).map_err(db)?,
                            None => store.capture_item_export_row(cap).map_err(db)?,
                        }
                    }
                };
                if let Some(row) = row.filter(|r| r.kind == "capture") {
                    line(&mut captures, &row)?;
                }
            }
            ItemKind::Episode => {
                if let Some(row) = store.episode_export_row(row_id()?).map_err(db)? {
                    line(&mut episodes, &row)?;
                }
            }
            ItemKind::Conversation => {
                if let Some(c) = store.get_conversation(row_id()?).map_err(db)? {
                    let value = serde_json::json!({
                        "id": c.id, "title": c.title, "starred": c.starred,
                        "created_at_unix_ms": c.created_at_unix_ms, "updated_at_unix_ms": c.updated_at_unix_ms,
                        "exchanges": c.exchanges,
                    });
                    line(&mut conversations, &value)?;
                }
            }
            ItemKind::Step => {
                if let Some(row) = store.step_export_row(&item.id).map_err(db)? {
                    line(&mut steps, &row)?;
                }
            }
            ItemKind::Thread | ItemKind::VaultNote | ItemKind::ExportFolder => {}
        }
    }
    for (name, text) in [
        ("emails.jsonl", &emails),
        ("calendar.jsonl", &calendar),
        ("captures.jsonl", &captures),
        ("episodes.jsonl", &episodes),
        ("conversations.jsonl", &conversations),
        ("steps.jsonl", &steps),
    ] {
        write_new(&inside(folder, name)?, text).map_err(ioe)?;
    }
    let page = store.topic_page(topic_id, now_unix_ms).map_err(db)?;
    let page = serde_json::to_string_pretty(&page).map_err(|_| ApiError::ProviderFailure)?;
    write_new(&inside(folder, "topic.json")?, &page).map_err(ioe)?;
    write_new(&inside(folder, "README.md")?, &readme(now_unix_ms, opts.include_media)).map_err(ioe)?;
    Ok(media)
}

/// Appends `v` as one JSON line.
fn line<T: serde::Serialize>(out: &mut String, v: &T) -> ApiResult<()> {
    out.push_str(&serde_json::to_string(v).map_err(|_| ApiError::ProviderFailure)?);
    out.push('\n');
    Ok(())
}

/// The backups this computer's own tools make, for the README's last line (it is read on the machine that wrote it).
const PLATFORM_BACKUPS: &str = if cfg!(windows) {
    "File History, Windows Backup"
} else {
    "Time Machine"
};

fn readme(now_unix_ms: i64, media: bool) -> String {
    format!(
        "# Whimbrel client export\n\nWritten {}.\n\n\
         - `meetings/`: one Markdown file per meeting with its summary, decisions, action items, open questions, \
         notes and transcript. A meeting shared with another client is here whole.\n\
         - `emails.jsonl`, `calendar.jsonl`, `captures.jsonl`: one JSON object per line.\n\
         - `episodes.jsonl`: screen activity summaries.\n\
         - `conversations.jsonl`: Ask conversations with their answers.\n\
         - `steps.jsonl`: steps and their notes.\n\
         - `topic.json`: the client's page.\n\
         {}\n\
         This folder is a plain copy of private data, and it is yours to keep or delete. Whimbrel remembers \
         where it is so that deleting the client can offer to remove it. Copies of this folder in {} \
         or cloud backups are yours to handle.\n",
        rfc3339(now_unix_ms),
        if media {
            "- `media/`: audio recordings and slide images, by meeting.\n"
        } else {
            "Audio recordings and slide images are not copied.\n"
        },
        PLATFORM_BACKUPS,
    )
}

/// Creates `parent/Whimbrel client export <date>[ n]/` (a new private folder, never an existing one) and
/// returns its canonical path. The user's `parent` keeps its own permissions.
fn new_folder(parent: &Path, now_unix_ms: i64) -> ApiResult<PathBuf> {
    fs::create_dir_all(parent).map_err(ioe)?;
    let parent = parent.canonicalize().map_err(ioe)?;
    let base = format!("{FOLDER_PREFIX}{}", ymd(now_unix_ms));
    for n in 1..=MAX_SUFFIX {
        let name = if n == 1 { base.clone() } else { format!("{base} {n}") };
        let path = inside(&parent, &name)?;
        match make_private_dir(&path) {
            Ok(()) => return path.canonicalize().map_err(ioe),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(ioe(e)),
        }
    }
    Err(ApiError::ProviderFailure)
}

/// Whether `name` is an export folder's name: the prefix, a `yyyy-mm-dd` date, and an optional number.
fn is_export_folder_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(FOLDER_PREFIX) else { return false };
    let (date, suffix) = match rest.split_once(' ') {
        Some((d, s)) => (d, Some(s)),
        None => (rest, None),
    };
    let b = date.as_bytes();
    let date_ok = b.len() == 10
        && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() });
    date_ok && suffix.is_none_or(|s| !s.is_empty() && s.len() <= 4 && s.bytes().all(|c| c.is_ascii_digit()))
}

/// Opens `p` for reading without following a symlink at its last component (O_NOFOLLOW on unix), and only
/// when the opened handle is a plain file.
fn open_plain(p: &Path) -> io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NOFOLLOW);
    }
    // Windows: open the link itself (symlink or junction) rather than its target, and refuse it
    // below. FILE_FLAG_OPEN_REPARSE_POINT.
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.custom_flags(0x0020_0000);
    }
    let f = o.open(p)?;
    let meta = f.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT: a symlink, junction or other reparse point.
        if meta.file_attributes() & 0x400 != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a plain file"));
        }
    }
    if !meta.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a plain file"));
    }
    Ok(f)
}

/// Copies the media [`write_client_export`] listed into `export.folder/media/<meeting>/`, only from plain
/// files (not symlinks, opened without following one) whose real path is inside one of `media_roots`. A slide
/// folder's plain files are copied; its subfolders and symlinks are not. Never writes over an existing file
/// (that counts as skipped). A copy that fails is counted in `failed` and its partial file removed; the export
/// goes on.
pub fn copy_client_media(export: &ClientExport, media_roots: &[PathBuf]) -> ApiResult<MediaCopied> {
    let mut out = MediaCopied::default();
    if export.media.is_empty() {
        return Ok(out);
    }
    let roots: Vec<PathBuf> = media_roots.iter().filter_map(|r| r.canonicalize().ok()).collect();
    let in_roots = |p: &Path| p.canonicalize().map(|real| roots.iter().any(|r| real.starts_with(r))).unwrap_or(false);
    let plain_file = |p: &Path| fs::symlink_metadata(p).map(|m| m.file_type().is_file()).unwrap_or(false);
    // A real folder of ours (made now or before), never a symlink placed since.
    let real_dir = |p: &Path| fs::symlink_metadata(p).map(|m| m.file_type().is_dir()).unwrap_or(false);
    let ensure_dir = |p: &Path| match make_private_dir(p) {
        Ok(()) => real_dir(p),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => real_dir(p),
        Err(_) => false,
    };
    let media_dir = inside(&export.folder, "media")?;
    let media_ok = ensure_dir(&media_dir);
    let mut folders: HashSet<String> = HashSet::new();
    for m in &export.media {
        let mut files: Vec<PathBuf> = Vec::new();
        for a in &m.audio {
            if plain_file(a) && in_roots(a) {
                files.push(a.clone());
            } else {
                out.skipped += 1;
            }
        }
        if let Some(dir) = &m.slides_dir {
            if real_dir(dir) && in_roots(dir) {
                match fs::read_dir(dir) {
                    Ok(rd) => {
                        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
                        entries.sort();
                        files.extend(entries.into_iter().filter(|p| plain_file(p) && in_roots(p)));
                    }
                    Err(_) => out.failed += 1,
                }
            } else {
                out.skipped += 1;
            }
        }
        if files.is_empty() {
            continue;
        }
        // `m.dest` is the meeting file's stem, unique (case-insensitively) among the meetings.
        let dest = match inside(&media_dir, &m.dest) {
            Ok(d) if media_ok && folders.insert(m.dest.to_lowercase()) && ensure_dir(&d) => d,
            _ => {
                out.failed += files.len();
                continue;
            }
        };
        let mut names: HashSet<String> = HashSet::new();
        for src in files {
            let name = safe_name(&src);
            let mut unique = name.clone();
            let mut n = 2;
            while !names.insert(unique.to_lowercase()) {
                unique = format!("{n} {name}");
                n += 1;
            }
            let Ok(target) = inside(&dest, &unique) else {
                out.failed += 1;
                continue;
            };
            let mut from = match open_plain(&src) {
                Ok(f) => f,
                Err(_) => {
                    out.failed += 1;
                    continue;
                }
            };
            let mut to = match new_private_file(&target) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    out.skipped += 1;
                    continue;
                }
                Err(_) => {
                    out.failed += 1;
                    continue;
                }
            };
            match io::copy(&mut from, &mut to) {
                Ok(_) => out.files += 1,
                Err(_) => {
                    drop(to);
                    let _ = fs::remove_file(&target);
                    out.failed += 1;
                }
            }
        }
    }
    Ok(out)
}

/// A media file's name, made safe: its stem as [`file_stem`] makes it and a short alphanumeric extension.
fn safe_name(p: &Path) -> String {
    let stem = file_stem(&p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    match p.extension().map(|e| e.to_string_lossy().into_owned()) {
        Some(ext) if !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) => {
            format!("{stem}.{ext}")
        }
        _ => stem,
    }
}

/// What [`remove_export_folder`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderRemoval {
    Removed,
    /// Verifiably not there: its parent folder is there and it is not.
    Gone,
    /// Its parent folder cannot be reached (a drive unplugged, a share offline, a parent renamed): not known
    /// to be gone. The record stays; the delete fails the kind so a re-run can finish it.
    Unreachable,
    /// Not removed: not a recorded export folder, a symlink, not a folder, moved behind a symlink, or not
    /// named as an export folder. Left for the user.
    Refused,
}

/// Removes recorded export folder `path` (fail closed): it must be recorded in `client_export`, absolute, a
/// real folder (not a symlink), its own canonical path (no symlinked parent) and named as an export folder.
/// The removal does not follow symlinks inside it. An I/O failure is an error (retry by re-running).
pub(crate) fn remove_export_folder(store: &memory::Store, path: &str) -> rusqlite::Result<io::Result<FolderRemoval>> {
    if !store.is_client_export(path)? {
        return Ok(Ok(FolderRemoval::Refused));
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Ok(Ok(FolderRemoval::Refused));
    }
    let meta = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let parent_here = p
                .parent()
                .and_then(|parent| fs::symlink_metadata(parent).ok())
                .is_some_and(|m| m.file_type().is_dir());
            return Ok(Ok(if parent_here { FolderRemoval::Gone } else { FolderRemoval::Unreachable }));
        }
        Err(e) => return Ok(Err(e)),
    };
    let named = p.file_name().and_then(|n| n.to_str()).is_some_and(is_export_folder_name);
    let canonical = p.canonicalize().is_ok_and(|c| c == p);
    if !meta.file_type().is_dir() || !named || !canonical {
        return Ok(Ok(FolderRemoval::Refused));
    }
    Ok(fs::remove_dir_all(p).map(|_| FolderRemoval::Removed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    struct Env {
        dir: tempfile::TempDir,
        store: memory::Store,
        c: Connection,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let store = crate::test_store::open(&path, dir.path());
        let c = crate::test_store::raw(&path);
        Env { dir, store, c }
    }

    fn file(e: &Env, topic: i64, kind: &str, r: i64, ts: i64) {
        let sid: i64 = match e.c.query_row(
            "SELECT id FROM topic_source WHERE kind = ?1 AND ref_id = ?2",
            params![kind, r],
            |x| x.get(0),
        ) {
            Ok(id) => id,
            Err(_) => {
                e.c.execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, ?3, 1)",
                    params![kind, r, ts],
                )
                .unwrap();
                e.c.last_insert_rowid()
            }
        };
        e.c.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![topic, sid])
            .unwrap();
    }

    fn meeting(e: &Env, title: &str, start: i64, said: &str, audio: Option<String>) -> i64 {
        let id = e
            .store
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev-a".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 1000),
                title: title.into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: audio,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        e.store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id: id,
                start_cs: 0,
                speaker_label: "A".into(),
                text: said.into(),
            }])
            .unwrap();
        id
    }

    fn email(e: &Env, ext: &str, subject: &str, body: &str) -> i64 {
        e.store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "dev-a".into(),
                external_id: ext.into(),
                ts_unix_ms: 1_000,
                from: "a@x.com".into(),
                to: "me@x.com".into(),
                subject: subject.into(),
                body_text: body.into(),
                gmail_thread_id: format!("t-{ext}"),
                account_email: "me@x.com".into(),
                gmail_labels: vec![],
                list_unsubscribe: false,
            }])
            .unwrap();
        e.c.query_row("SELECT id FROM items WHERE kind = 'email' AND external_id = ?1", [ext], |r| r.get(0)).unwrap()
    }

    fn calendar(e: &Env, ext: &str, title: &str) -> i64 {
        e.store
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: "dev-a".into(),
                source: "google_calendar".into(),
                external_id: ext.into(),
                ts_unix_ms: 2_000,
                title: title.into(),
                actors: vec![],
            }])
            .unwrap();
        e.c.query_row("SELECT id FROM items WHERE kind = 'calendar_event' AND external_id = ?1", [ext], |r| r.get(0))
            .unwrap()
    }

    fn step(e: &Env, key: &str, topic_key: &str, text: &str, note: Option<&str>) {
        e.c.execute(
            "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, \
             asked_at_unix_ms, updated_at_unix_ms, note) VALUES (?1, ?2, 'k', 'email', 't', ?3, 'mine', 1, 1, ?4)",
            params![key, topic_key, text, note],
        )
        .unwrap();
    }

    fn read(folder: &Path, name: &str) -> String {
        fs::read_to_string(folder.join(name)).unwrap()
    }

    /// Every file's text under `dir`, concatenated.
    fn all_text(dir: &Path) -> String {
        let mut out = String::new();
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                out.push_str(&all_text(&p));
            } else {
                out.push_str(&String::from_utf8_lossy(&fs::read(&p).unwrap()));
            }
        }
        out
    }

    const DAY: i64 = 1_763_000_000_000; // 2025-11-13 UTC

    #[test]
    fn export_contains_only_this_clients_material() {
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let g = e.store.create_seed_topic("Globex", &[], None).unwrap();
        // A seed merged into A: its material is A's.
        let m2 = e.store.create_seed_topic("Mergedco", &[], None).unwrap();
        e.c.execute("UPDATE topic_seed SET merged_into_key = ?1 WHERE key = ?2", [&a.key, &m2.key]).unwrap();

        let m_a = meeting(&e, "Kickoff", 10_000, "own meeting words", None);
        let m_shared = meeting(&e, "Joint sync", 20_000, "shared meeting words", None);
        let m_g = meeting(&e, "Globex review", 30_000, "globex secret words", None);
        file(&e, a.topic_id, "meeting", m_a, 10_000);
        file(&e, a.topic_id, "meeting", m_shared, 20_000);
        file(&e, g.topic_id, "meeting", m_shared, 20_000);
        file(&e, g.topic_id, "meeting", m_g, 30_000);
        let e_a = email(&e, "g-a", "Contract", "own email body");
        let e_g = email(&e, "g-g", "Invoice", "globex email body");
        let e_m = email(&e, "g-m", "Merged", "merged seed email body");
        file(&e, a.topic_id, "email", e_a, 1_000);
        file(&e, g.topic_id, "email", e_g, 1_000);
        file(&e, m2.topic_id, "email", e_m, 1_000);
        // A removed calendar event of A's: still exported.
        let cal = calendar(&e, "kick@x#1", "removed event title");
        file(&e, a.topic_id, "calendar_event", cal, 2_000);
        e.c.execute("UPDATE items SET flags = '{\"removed\":true,\"changed_at\":1}' WHERE id = ?1", [cal]).unwrap();
        step(&e, "s-a", &a.key, "own step text", Some("own step note"));
        step(&e, "s-g", &g.key, "globex step text", None);
        // Possible matches: one ticked, one not.
        let ticked = email(&e, "g-p1", "Hi", "met Zorblax ticked mail");
        let unticked = email(&e, "g-p2", "Hi", "met Zorblax unticked mail");
        let inv = inventory(&e.store, &a.key).unwrap();
        assert!(inv.linked.iter().any(|i| i.kind == ItemKind::CalendarEvent && i.removed));
        let pick = inv.possible.iter().find(|p| p.id == ticked.to_string()).unwrap().clone();
        assert!(inv.possible.iter().any(|p| p.id == unticked.to_string()));
        // A ticked item the inventory does not offer (Globex's own email) is never exported.
        let forged = ClientItem { kind: ItemKind::Email, id: e_g.to_string(), also_client: None, date_unix_ms: 0, removed: false, label: None, excerpt: None, app: None };

        let out = e.dir.path().join("out");
        let opts = ExportOptions { include_possible: vec![pick, forged], ..Default::default() };
        let folder = export_client(&e.store, &a.key, &out, &opts).unwrap();
        let text = all_text(&folder);
        for want in [
            "own meeting words",
            "shared meeting words",
            "own email body",
            "merged seed email body",
            "removed event title",
            "own step text",
            "own step note",
            "ticked mail",
        ] {
            assert!(text.contains(want), "missing {want}");
        }
        for never in ["globex secret words", "globex email body", "globex step text", "unticked mail"] {
            assert!(!text.contains(never), "exported {never}");
        }
        let meetings: Vec<String> =
            fs::read_dir(folder.join("meetings")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(meetings.len(), 2);
        // The shared meeting is whole: its transcript is there.
        let joint = meetings.iter().find(|n| n.contains("Joint sync")).unwrap();
        assert!(read(&folder.join("meetings"), joint).contains("## Transcript"));
        assert_eq!(read(&folder, "emails.jsonl").lines().count(), 3);
        assert_eq!(read(&folder, "calendar.jsonl").lines().count(), 1);
        assert_eq!(read(&folder, "steps.jsonl").lines().count(), 1);
        let page = read(&folder, "topic.json");
        assert!(page.contains("Zorblax") && !page.contains("Globex"));
        assert!(meetings.iter().all(|n| !n.contains("Globex")));
        let readme = read(&folder, "README.md");
        assert!(readme.contains(PLATFORM_BACKUPS) && readme.contains("yours to handle"));
        // Never the other platform's backup tool.
        assert_eq!(readme.contains("Time Machine"), !cfg!(windows), "{readme}");
        assert!(!readme.contains("Zorblax"));
    }

    #[test]
    fn export_includes_captures_by_either_id_and_episodes() {
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let cap = e
            .store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "dev-a".into(),
                ts_unix_ms: 5_000,
                app_bundle_id: "com.apple.TextEdit".into(),
                window_title: "Doc".into(),
                url: None,
                text: "episode capture words".into(),
                hash: 5,
                source: "ax".into(),
            })
            .unwrap()
            .unwrap();
        let item: i64 = e
            .c
            .query_row("SELECT id FROM items WHERE source_table = 'capture_item' AND source_id = ?1", [cap], |r| r.get(0))
            .unwrap();
        // A capture held only in `items` (pulled), in the same episode range.
        e.c.execute(
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, app_bundle_id, title, actors, body_text, refs) \
             VALUES ('dev-a', 'capture', 'ax', 5001, 'com.apple.TextEdit', 'Doc', '[]', 'pulled capture words', '{}')",
            [],
        )
        .unwrap();
        let pulled = e.c.last_insert_rowid();
        e.c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, \
             end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('dev-a', 'com.apple.TextEdit', 'Doc', 5000, 5001, ?1, ?2, 2, 'episode summary words', 'summarized', 1, 1)",
            params![item, pulled],
        )
        .unwrap();
        let ep = e.c.last_insert_rowid();
        file(&e, a.topic_id, "episode", ep, 5_000);
        // A different capture whose capture_item id equals the pulled items id must not be picked up instead.
        let inv = inventory(&e.store, &a.key).unwrap();
        let ids: Vec<&str> = inv.linked.iter().filter(|i| i.kind == ItemKind::Capture).map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&cap.to_string().as_str()) && ids.contains(&format!("i:{pulled}").as_str()), "{ids:?}");
        let folder = export_client(&e.store, &a.key, &e.dir.path().join("out"), &ExportOptions::default()).unwrap();
        let caps = read(&folder, "captures.jsonl");
        assert_eq!(caps.lines().count(), 2);
        assert!(caps.contains("episode capture words") && caps.contains("pulled capture words"));
        assert!(read(&folder, "episodes.jsonl").contains("episode summary words"));
    }

    #[test]
    fn export_records_folder() {
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let g = e.store.create_seed_topic("Globex", &[], None).unwrap();
        let out = e.dir.path().join("out");
        let folder = export_client(&e.store, &a.key, &out, &ExportOptions::default()).unwrap();
        let path = folder.to_string_lossy().into_owned();
        assert_eq!(e.store.client_exports(&a.key).unwrap(), vec![path.clone()]);
        assert!(e.store.client_exports(&g.key).unwrap().is_empty());
        let rows: Vec<(String, i64)> = e
            .c
            .prepare("SELECT path, created_at_unix_ms FROM client_export")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, path);
        let raw: String = e.c.query_row("SELECT path || created_at_unix_ms || client FROM client_export", [], |r| r.get(0)).unwrap();
        assert!(!raw.contains("Zorblax") && !raw.contains(&a.key), "no client name or key in the record");
        // The inventory lists it as left for the user.
        let inv = inventory(&e.store, &a.key).unwrap();
        assert!(inv.left_for_you.contains(&crate::client_inventory::LeftForYou::ExportFolders(vec![folder.clone()])));
        // Re-runnable: a second export the same day is a new, recorded folder.
        let again = export_client(&e.store, &a.key, &out, &ExportOptions::default()).unwrap();
        assert_ne!(again, folder);
        assert_eq!(e.store.client_exports(&a.key).unwrap().len(), 2);
    }

    #[test]
    fn export_folder_name_has_no_client_name() {
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let out = e.dir.path().join("out");
        let opts = ExportOptions::default();
        let first = write_client_export(&e.store, &a.key, &out, &opts, DAY).unwrap().folder;
        let second = write_client_export(&e.store, &a.key, &out, &opts, DAY).unwrap().folder;
        let name = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name(&first), "Whimbrel client export 2025-11-13");
        assert_eq!(name(&second), "Whimbrel client export 2025-11-13 2");
        assert!(is_export_folder_name(&name(&first)) && is_export_folder_name(&name(&second)));
        assert!(!is_export_folder_name("Whimbrel client export 2025-11-13 Zorblax"));
        assert!(!is_export_folder_name("Zorblax"));
    }

    #[test]
    fn export_excludes_media_by_default() {
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let media = e.dir.path().join("media");
        let outside = e.dir.path().join("elsewhere");
        fs::create_dir_all(media.join("slides-1")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let audio = media.join("system-audio-1.m4a");
        fs::write(&audio, b"audio").unwrap();
        fs::write(media.join("slides-1").join("10.00.00.png"), b"png").unwrap();
        fs::write(outside.join("secret.txt"), b"outside secret").unwrap();
        // A symlink in the slide folder pointing out of the media folder is never followed.
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("secret.txt"), media.join("slides-1").join("link.png")).unwrap();
        let m = meeting(&e, "Kickoff", 10_000, "words", Some(audio.to_string_lossy().into()));
        e.c.execute("UPDATE meeting SET snapshots_dir = ?1 WHERE id = ?2", params![media.join("slides-1").to_string_lossy(), m])
            .unwrap();
        // A second meeting whose audio is outside the media folder.
        let m2 = meeting(&e, "Other", 20_000, "words", Some(outside.join("secret.txt").to_string_lossy().into()));
        file(&e, a.topic_id, "meeting", m, 10_000);
        file(&e, a.topic_id, "meeting", m2, 20_000);
        let out = e.dir.path().join("out");

        let plain = ExportOptions { media_roots: vec![media.clone()], ..Default::default() };
        let folder = export_client(&e.store, &a.key, &out, &plain).unwrap();
        assert!(!folder.join("media").exists(), "no media unless asked");
        assert!(read(&folder, "README.md").contains("not copied"));

        let with = ExportOptions { include_media: true, media_roots: vec![media.clone()], ..Default::default() };
        let export = write_client_export(&e.store, &a.key, &out, &with, DAY).unwrap();
        let copied = copy_client_media(&export, &with.media_roots).unwrap();
        assert_eq!(copied.files, 2, "the audio and the one real slide");
        let text = all_text(&export.folder.join("media"));
        assert!(text.contains("audio") && text.contains("png"));
        assert!(!text.contains("outside secret"), "nothing from outside the media folder");
        // No media roots: nothing is copied.
        let none = write_client_export(&e.store, &a.key, &out, &with, DAY).unwrap();
        assert_eq!(copy_client_media(&none, &[]).unwrap().files, 0);
    }

    #[test]
    fn delete_with_exports_removes_recorded_folders_only() {
        use crate::client_delete::{delete_client, DeleteSelection};
        use crate::client_inventory::LeftForYou;
        let e = env();
        e.store.set_own_device_id("dev-a");
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let g = e.store.create_seed_topic("Globex", &[], None).unwrap();
        let out = e.dir.path().join("out");
        let mine = export_client(&e.store, &a.key, &out, &ExportOptions::default()).unwrap();
        let theirs = export_client(&e.store, &g.key, &out, &ExportOptions::default()).unwrap();
        // A folder named like an export but never recorded.
        let stray = out.join("Whimbrel client export 2020-01-01");
        fs::create_dir_all(&stray).unwrap();
        // A recorded folder that has since become a symlink to elsewhere: refused, the target untouched; and a
        // symlink inside a recorded folder is removed as a link, its target stays.
        let target = e.dir.path().join("precious");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep.txt"), b"keep").unwrap();
        #[cfg(unix)]
        let swapped = {
            let swapped = export_client(&e.store, &a.key, &out, &ExportOptions::default()).unwrap();
            fs::remove_dir_all(&swapped).unwrap();
            std::os::unix::fs::symlink(&target, &swapped).unwrap();
            std::os::unix::fs::symlink(&target, mine.join("link")).unwrap();
            swapped
        };

        let sel = DeleteSelection { key: a.key.clone(), delete_exports: true, ..Default::default() };
        let receipt = delete_client(&e.store, &sel, &[], None).unwrap();
        assert!(!mine.exists(), "the client's recorded export is removed");
        assert!(theirs.exists(), "another client's export stays");
        assert!(stray.exists(), "an unrecorded folder stays");
        assert!(target.join("keep.txt").exists(), "never follows a symlink out");
        assert!(receipt.counts.contains(&(ItemKind::ExportFolder, 1)));
        #[cfg(unix)]
        {
            assert!(
                receipt.left_for_you.contains(&LeftForYou::ExportFolders(vec![swapped.clone()])),
                "{:?}",
                receipt.left_for_you
            );
            assert!(receipt.left_for_you.iter().any(|l| matches!(l, LeftForYou::FilesNotRemoved(v) if v.contains(&swapped))));
        }
        assert!(e.store.client_exports(&g.key).unwrap().len() == 1);
        assert!(!e.store.is_client_export(&mine.to_string_lossy()).unwrap());

        // Without `delete_exports` the folders stay and are listed, and the purge drops their records.
        let k = e.store.create_seed_topic("Initech", &[], None).unwrap();
        let kept = export_client(&e.store, &k.key, &out, &ExportOptions::default()).unwrap();
        let sel = DeleteSelection { key: k.key.clone(), ..Default::default() };
        let receipt = delete_client(&e.store, &sel, &[], None).unwrap();
        assert!(kept.exists());
        assert!(receipt.left_for_you.contains(&LeftForYou::ExportFolders(vec![kept.clone()])));
        assert!(receipt.complete());
        assert!(!e.store.is_client_export(&kept.to_string_lossy()).unwrap(), "record dropped with the client");
    }

    #[test]
    fn delete_removes_a_merged_seeds_export() {
        use crate::client_delete::{delete_client, DeleteSelection};
        let e = env();
        e.store.set_own_device_id("dev-a");
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let b = e.store.create_seed_topic("Mergedco", &[], None).unwrap();
        let out = e.dir.path().join("out");
        let before_merge = export_client(&e.store, &b.key, &out, &ExportOptions::default()).unwrap();
        e.c.execute("UPDATE topic_seed SET merged_into_key = ?1 WHERE key = ?2", [&a.key, &b.key]).unwrap();
        let sel = DeleteSelection { key: a.key.clone(), delete_exports: true, ..Default::default() };
        let receipt = delete_client(&e.store, &sel, &[], None).unwrap();
        assert!(!before_merge.exists());
        assert!(receipt.counts.contains(&(ItemKind::ExportFolder, 1)));
        assert_eq!(e.c.query_row("SELECT COUNT(*) FROM client_export", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn an_unreachable_export_is_kept_listed_and_fails_the_kind() {
        use crate::client_delete::{delete_client, ClientRecord, DeleteSelection};
        use crate::client_inventory::LeftForYou;
        let e = env();
        e.store.set_own_device_id("dev-a");
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let usb = e.dir.path().join("usb");
        let folder = export_client(&e.store, &a.key, &usb.join("Clients"), &ExportOptions::default()).unwrap();
        // The drive is unplugged: the parent is gone, so the folder is not known to be gone.
        let away = e.dir.path().join("usb-away");
        fs::rename(&usb, &away).unwrap();
        let sel = DeleteSelection { key: a.key.clone(), delete_exports: true, ..Default::default() };
        let receipt = delete_client(&e.store, &sel, &[], None).unwrap();
        assert!(receipt.failed.contains(&ItemKind::ExportFolder));
        assert_eq!(receipt.client, ClientRecord::StillPresent);
        assert!(e.store.is_client_export(&folder.to_string_lossy()).unwrap(), "record kept");
        assert!(receipt.left_for_you.contains(&LeftForYou::ExportFolders(vec![folder.clone()])));
        assert!(receipt.left_for_you.iter().any(|l| matches!(l, LeftForYou::FilesNotRemoved(v) if v.contains(&folder))));
        // Plugged back in: a re-run removes it and finishes.
        fs::rename(&away, &usb).unwrap();
        let receipt = delete_client(&e.store, &sel, &[], None).unwrap();
        assert!(!folder.exists());
        assert!(receipt.complete(), "{receipt:?}");
    }

    #[test]
    fn a_refused_export_is_not_blamed_on_a_missing_device_id() {
        use crate::client_delete::{delete_client, DeleteSelection, DeleteStage};
        use crate::client_inventory::LeftForYou;
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        // Recorded, but not named as an export folder: refused.
        let odd = e.dir.path().join("Renamed by hand");
        fs::create_dir_all(&odd).unwrap();
        let odd = odd.canonicalize().unwrap();
        e.store.record_client_export(&a.key, &odd.to_string_lossy(), 1).unwrap();
        let sel = DeleteSelection { key: a.key.clone(), delete_exports: true, ..Default::default() };
        let receipt = delete_client(&e.store, &sel, &[], None).unwrap();
        assert!(odd.exists());
        assert!(!receipt.failed_stages.contains(&DeleteStage::NoDeviceId), "{receipt:?}");
        assert!(receipt.left_for_you.iter().any(|l| matches!(l, LeftForYou::FilesNotRemoved(v) if v.contains(&odd))));
    }

    #[test]
    fn remove_refuses_paths_that_are_not_recorded() {
        let e = env();
        let dir = e.dir.path().join("Whimbrel client export 2025-11-13");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.canonicalize().unwrap().to_string_lossy().into_owned();
        assert_eq!(remove_export_folder(&e.store, &p).unwrap().unwrap(), FolderRemoval::Refused);
        assert!(dir.exists());
        // Recorded but not named as an export folder.
        let odd = e.dir.path().join("Zorblax");
        fs::create_dir_all(&odd).unwrap();
        let odd = odd.canonicalize().unwrap().to_string_lossy().into_owned();
        e.store.record_client_export("k", &odd, 1).unwrap();
        assert_eq!(remove_export_folder(&e.store, &odd).unwrap().unwrap(), FolderRemoval::Refused);
        let root = e.dir.path().canonicalize().unwrap();
        // The parent is there and the folder is not: gone.
        let gone = root.join("Whimbrel client export 2025-11-14").to_string_lossy().into_owned();
        e.store.record_client_export("k", &gone, 1).unwrap();
        assert_eq!(remove_export_folder(&e.store, &gone).unwrap().unwrap(), FolderRemoval::Gone);
        // The parent is missing (a drive unplugged): not known to be gone.
        let away = root.join("nowhere").join("Whimbrel client export 2025-11-13").to_string_lossy().into_owned();
        e.store.record_client_export("k", &away, 1).unwrap();
        assert_eq!(remove_export_folder(&e.store, &away).unwrap().unwrap(), FolderRemoval::Unreachable);
    }

    /// Windows: a symlink is opened as the link and refused, never followed to its target.
    /// Making a file symlink needs Developer Mode or admin; without it the test has nothing
    /// to check and says so.
    #[cfg(windows)]
    #[test]
    fn open_plain_refuses_a_symlink_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let (target, link) = (dir.path().join("secret.txt"), dir.path().join("link.png"));
        fs::write(&target, b"outside secret").unwrap();
        assert!(open_plain(&target).is_ok(), "a plain file opens");
        if let Err(e) = std::os::windows::fs::symlink_file(&target, &link) {
            eprintln!("skipped: cannot make a symlink here ({e})");
            return;
        }
        assert!(open_plain(&link).is_err(), "the link is refused, not followed");
    }

    #[test]
    fn a_failed_export_leaves_no_folder_and_no_record() {
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let m = meeting(&e, "Kickoff", 10_000, "words", None);
        file(&e, a.topic_id, "meeting", m, 10_000);
        // The topic page read (the last one) fails after the meetings were written.
        e.c.execute_batch("DROP TABLE topic_role").unwrap();
        let out = e.dir.path().join("out");
        assert!(export_client(&e.store, &a.key, &out, &ExportOptions::default()).is_err());
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0, "the half-written folder is removed");
        assert_eq!(e.c.query_row("SELECT COUNT(*) FROM client_export", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn media_that_fails_to_copy_leaves_a_complete_text_export_flagged_partial() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return; // root reads anything
        }
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let media = e.dir.path().join("media");
        let slides = media.join("slides-1");
        fs::create_dir_all(&slides).unwrap();
        fs::write(slides.join("1.png"), b"png").unwrap();
        let m = meeting(&e, "Kickoff", 10_000, "words", None);
        e.c.execute("UPDATE meeting SET snapshots_dir = ?1 WHERE id = ?2", params![slides.to_string_lossy(), m]).unwrap();
        file(&e, a.topic_id, "meeting", m, 10_000);
        fs::set_permissions(&slides, fs::Permissions::from_mode(0o000)).unwrap();
        let opts = ExportOptions { include_media: true, media_roots: vec![media.clone()], ..Default::default() };
        let done = export_client_with_media(&e.store, &a.key, &e.dir.path().join("out"), &opts);
        fs::set_permissions(&slides, fs::Permissions::from_mode(0o755)).unwrap();
        let done = done.unwrap();
        assert!(done.media.failed >= 1 && done.media.files == 0, "{:?}", done.media);
        assert!(done.partial());
        assert!(done.folder.join("README.md").exists() && done.folder.join("meetings").exists());
        assert!(e.store.is_client_export(&done.folder.to_string_lossy()).unwrap());
    }

    #[test]
    fn same_day_meetings_differing_in_case_get_their_own_files_and_media() {
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let media = e.dir.path().join("media");
        for d in ["one", "two"] {
            fs::create_dir_all(media.join(d)).unwrap();
            fs::write(media.join(d).join("system-audio.m4a"), d.as_bytes()).unwrap();
        }
        let m1 = meeting(&e, "Kickoff", 10_000, "first words", Some(media.join("one/system-audio.m4a").to_string_lossy().into()));
        let m2 = meeting(&e, "kickoff", 20_000, "second words", Some(media.join("two/system-audio.m4a").to_string_lossy().into()));
        file(&e, a.topic_id, "meeting", m1, 10_000);
        file(&e, a.topic_id, "meeting", m2, 20_000);
        let opts = ExportOptions { include_media: true, media_roots: vec![media.clone()], ..Default::default() };
        let done = export_client_with_media(&e.store, &a.key, &e.dir.path().join("out"), &opts).unwrap();
        assert_eq!(fs::read_dir(done.folder.join("meetings")).unwrap().count(), 2);
        let text = all_text(&done.folder.join("meetings"));
        assert!(text.contains("first words") && text.contains("second words"));
        assert_eq!(done.media.files, 2);
        assert_eq!(fs::read_dir(done.folder.join("media")).unwrap().count(), 2);
        let media_text = all_text(&done.folder.join("media"));
        assert!(media_text.contains("one") && media_text.contains("two"));

        // A file already at the destination is not counted as copied.
        let export = write_client_export(&e.store, &a.key, &e.dir.path().join("out"), &opts, DAY).unwrap();
        let first = &export.media[0];
        let dest = export.folder.join("media").join(&first.dest);
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("system-audio.m4a"), b"already").unwrap();
        let copied = copy_client_media(&export, &opts.media_roots).unwrap();
        assert_eq!((copied.files, copied.skipped), (1, 1));
        assert_eq!(fs::read(dest.join("system-audio.m4a")).unwrap(), b"already");
    }

    #[cfg(unix)]
    #[test]
    fn the_export_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let e = env();
        let a = e.store.create_seed_topic("Zorblax", &[], None).unwrap();
        let media = e.dir.path().join("media");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("a.m4a"), b"x").unwrap();
        let m = meeting(&e, "Kickoff", 10_000, "words", Some(media.join("a.m4a").to_string_lossy().into()));
        file(&e, a.topic_id, "meeting", m, 10_000);
        let opts = ExportOptions { include_media: true, media_roots: vec![media.clone()], ..Default::default() };
        let folder = export_client(&e.store, &a.key, &e.dir.path().join("out"), &opts).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&folder), 0o700);
        assert_eq!(mode(&folder.join("meetings")), 0o700);
        assert_eq!(mode(&folder.join("README.md")), 0o600);
        assert_eq!(mode(&folder.join("emails.jsonl")), 0o600);
        let md = fs::read_dir(folder.join("meetings")).unwrap().next().unwrap().unwrap().path();
        assert_eq!(mode(&md), 0o600);
        let sub = fs::read_dir(folder.join("media")).unwrap().next().unwrap().unwrap().path();
        assert_eq!(mode(&folder.join("media")), 0o700);
        assert_eq!(mode(&sub), 0o700);
        assert_eq!(mode(&sub.join("a.m4a")), 0o600);
    }
}
