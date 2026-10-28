//! The skins you made, one folder each under `root` (`<data>/skins/<id>/`): `skin.json`,
//! `palettes/<id>.json`, `images/<flat name>`, and `history/<n>.json` (the last 20 versions of
//! manifest and palettes, for undo). Ports `UserSkinStore.swift`. Ids are `user-` prefixed so they never
//! collide with built-ins. A deleted skin moves to `.deleted/<id>/` and is purged after a while.

use crate::format::{is_valid_id, whimbrel_default, Archetype, Skin};
use crate::history::{self, Version, VersionPalette};
use crate::loader::parse_skin;
use crate::palette::{default_palette, parse_palette, Palette, Token, Variants};
use crate::repair::{all_pairs, archetype_pairs, pairs_pass, repair};
use serde_json::Value;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use unicode_segmentation::UnicodeSegmentation;

/// Versions kept per skin.
pub const HISTORY_LIMIT: usize = 20;
/// Longest skin name, slug and image name.
pub const NAME_LIMIT: usize = 40;
/// The folder deleted skins move into.
pub const DELETED_DIR: &str = ".deleted";
const DELETED_MARKER: &str = ".deleted-at";

#[derive(Debug)]
pub enum StoreError {
    InvalidId(String),
    InvalidImageName(String),
    UnreadableManifest(String),
    NotFound(String),
    /// The path is a link, or resolves outside the skin's folder.
    Unsafe(String),
    Io(io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::InvalidId(s) => write!(f, "not a usable skin or palette id: {s}"),
            StoreError::InvalidImageName(s) => write!(f, "not a usable image name: {s}"),
            StoreError::UnreadableManifest(s) => write!(f, "the manifest of {s} cannot be read"),
            StoreError::NotFound(s) => write!(f, "no such skin or file: {s}"),
            StoreError::Unsafe(s) => write!(f, "refused: {s}"),
            StoreError::Io(e) => write!(f, "{e}"),
            StoreError::Json(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
}

/// `name` trimmed and cut so that, with `suffix` (a clash's " 2"), it is at most `NAME_LIMIT` characters.
pub fn skin_name(name: &str, suffix: &str) -> String {
    let room = NAME_LIMIT.saturating_sub(suffix.graphemes(true).count());
    let cut: String = name.trim().graphemes(true).take(room).collect();
    let cut = cut.trim_matches(|c: char| c.is_whitespace() && c != '\n' && c != '\r');
    format!("{cut}{suffix}")
}

/// `user-` and then `a-z0-9-`: the only ids that become folders.
pub fn is_user_id(id: &str) -> bool {
    id.len() > "user-".len() && id.starts_with("user-") && is_valid_id(id)
}

/// An image file name inside a skin: lower-case `a-z0-9.-`, not hidden.
pub fn is_image_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.len() <= NAME_LIMIT + 16
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// Writes `bytes` beside `path` and renames it into place, so a crash leaves the old file or the new one.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn pretty(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).expect("a JSON value serialises")
}

pub struct UserSkinStore {
    root: PathBuf,
}

impl UserSkinStore {
    /// `root` is `<data>/skins`.
    pub fn new(root: impl Into<PathBuf>) -> UserSkinStore {
        UserSkinStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The skin's folder; ids become path components, so only `user-` ids of `a-z0-9-` pass.
    pub fn folder(&self, skin_id: &str) -> Result<PathBuf, StoreError> {
        if is_user_id(skin_id) {
            Ok(self.root.join(skin_id))
        } else {
            Err(StoreError::InvalidId(skin_id.to_string()))
        }
    }

    pub fn skin_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = std::fs::read_dir(&self.root)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| is_user_id(n))
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        ids
    }

    /// `user-<slug>-<n>` with the lowest `n` whose folder does not exist; the slug is reduced to `a-z0-9`
    /// words joined by `-`, at most `NAME_LIMIT` characters.
    pub fn new_skin_id(&self, slug: &str) -> String {
        let lower = slug.to_lowercase();
        let words: Vec<&str> = lower
            .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
            .filter(|w| !w.is_empty())
            .collect();
        let joined: String = words.join("-").chars().take(NAME_LIMIT).collect();
        let clean = if let Some(stripped) = joined.strip_suffix('-') {
            stripped.to_string()
        } else if joined.is_empty() {
            "skin".to_string()
        } else {
            joined
        };
        let mut n = 1;
        while self.root.join(format!("user-{clean}-{n}")).exists() {
            n += 1;
        }
        format!("user-{clean}-{n}")
    }

    // --- manifest, palettes, versions

    /// The current `skin.json` bytes, or `None` when the skin was never saved.
    pub fn manifest(&self, skin_id: &str) -> Option<Vec<u8>> {
        self.folder(skin_id)
            .ok()
            .and_then(|d| std::fs::read(d.join("skin.json")).ok())
    }

    /// The current palette files, by id, sorted.
    pub fn palette_files(&self, skin_id: &str) -> Vec<(String, Vec<u8>)> {
        self.folder(skin_id)
            .map(|d| current_palettes(&d))
            .unwrap_or_default()
    }

    /// Writes the skin as given and appends it as a new version; returns the version number. The caller
    /// has already validated and repaired it (see `save`, which does).
    pub fn save_raw(
        &self,
        skin_id: &str,
        manifest: &[u8],
        palettes: &[(String, Vec<u8>)],
    ) -> Result<u32, StoreError> {
        let dir = self.folder(skin_id)?;
        let version = Version {
            manifest: manifest.to_vec(),
            palettes: palettes
                .iter()
                .map(|(id, data)| VersionPalette {
                    id: id.clone(),
                    data: data.clone(),
                })
                .collect(),
        };
        write_current(&version, &dir)?;
        let history_dir = dir.join("history");
        std::fs::create_dir_all(&history_dir)?;
        let mut numbers = history::numbers(&history_dir);
        let next = numbers.last().copied().unwrap_or(0) + 1;
        history::write(&history_dir, next, &version)?;
        numbers.push(next);
        let excess = numbers.len().saturating_sub(HISTORY_LIMIT);
        for old in &numbers[..excess] {
            let _ = std::fs::remove_file(history_dir.join(format!("{old}.json")));
        }
        Ok(next)
    }

    /// Saves a skin, its palettes and any new images. Palettes that do not suit the skin's archetype are
    /// repaired first (the contrast gate), so nothing below 4.5:1 is ever written. `images` are written
    /// under the exact names given (see `is_image_name`); use `add_image` to have a name made.
    pub fn save(
        &self,
        skin: &Skin,
        palettes: &[Palette],
        images: &[(String, Vec<u8>)],
    ) -> Result<u32, StoreError> {
        let dir = self.folder(&skin.id)?;
        let gated = save_gate_repair(skin, palettes, &|_| None);
        if !images.is_empty() {
            let images_dir = dir.join("images");
            std::fs::create_dir_all(&images_dir)?;
            for (name, bytes) in images {
                if !is_image_name(name) {
                    return Err(StoreError::InvalidImageName(name.clone()));
                }
                write_atomic(&images_dir.join(name), bytes)?;
            }
        }
        let manifest = pretty(&serde_json::to_value(&gated.skin)?);
        let files: Vec<(String, Vec<u8>)> = gated
            .palettes
            .iter()
            .map(|p| (p.id.clone(), pretty(&p.to_json())))
            .collect();
        self.save_raw(&skin.id, &manifest, &files)
    }

    /// Drops the newest version and restores the one before it (manifest and palettes); false when there is none.
    pub fn undo(&self, skin_id: &str) -> Result<bool, StoreError> {
        let dir = self.folder(skin_id)?;
        let history_dir = dir.join("history");
        let numbers = history::numbers(&history_dir);
        if numbers.len() < 2 {
            return Ok(false);
        }
        let previous = history::read(&history_dir, numbers[numbers.len() - 2])?;
        write_current(&previous, &dir)?;
        std::fs::remove_file(history_dir.join(format!("{}.json", numbers[numbers.len() - 1])))?;
        Ok(true)
    }

    /// Rewrites `name` (capped, `skin_name`) in `skin.json` as a new version, so it can be undone.
    pub fn rename(&self, skin_id: &str, name: &str) -> Result<(), StoreError> {
        let manifest = self
            .manifest(skin_id)
            .ok_or_else(|| StoreError::UnreadableManifest(skin_id.to_string()))?;
        let Ok(Value::Object(mut root)) = serde_json::from_slice::<Value>(&manifest) else {
            return Err(StoreError::UnreadableManifest(skin_id.to_string()));
        };
        root.insert("name".into(), Value::String(skin_name(name, "")));
        let palettes = self.palette_files(skin_id);
        self.save_raw(skin_id, &pretty(&Value::Object(root)), &palettes)?;
        Ok(())
    }

    // --- images

    /// Writes `images/<flat name>` (lower-case `a-z0-9.-`, at most `NAME_LIMIT` before a `-2`, `-3`...
    /// that makes it unique; a URL's query and fragment are dropped) and returns the asset name.
    pub fn add_image(
        &self,
        skin_id: &str,
        bytes: &[u8],
        preferred_name: &str,
    ) -> Result<String, StoreError> {
        let images = self.folder(skin_id)?.join("images");
        std::fs::create_dir_all(&images)?;
        let (base, ext) = flat_image_name(preferred_name);
        let name_for = |n: u32| match (n, ext.is_empty()) {
            (1, true) => base.clone(),
            (1, false) => format!("{base}.{ext}"),
            (n, true) => format!("{base}-{n}"),
            (n, false) => format!("{base}-{n}.{ext}"),
        };
        let mut n = 1;
        while images.join(name_for(n)).exists() {
            n += 1;
        }
        let name = name_for(n);
        write_atomic(&images.join(&name), bytes)?;
        Ok(name)
    }

    /// The path of an approved image, refusing a link and anything that resolves outside the skin's folder.
    pub fn image_path(&self, skin_id: &str, name: &str) -> Result<PathBuf, StoreError> {
        let dir = self.folder(skin_id)?;
        if !is_image_name(name) || name == "." || name == ".." {
            return Err(StoreError::InvalidImageName(name.to_string()));
        }
        let images = dir.join("images");
        let path = images.join(name);
        // Neither the file nor its folders may be a link, and the real path must be where it claims to be.
        for p in [&dir, &images, &path] {
            let meta =
                std::fs::symlink_metadata(p).map_err(|_| StoreError::NotFound(name.to_string()))?;
            if meta.file_type().is_symlink() {
                return Err(StoreError::Unsafe(format!("{} is a link", p.display())));
            }
        }
        if !std::fs::symlink_metadata(&path)?.is_file() {
            return Err(StoreError::NotFound(name.to_string()));
        }
        let real_dir = std::fs::canonicalize(&dir)?;
        let real = std::fs::canonicalize(&path)?;
        if real != real_dir.join("images").join(name) {
            return Err(StoreError::Unsafe(format!(
                "{name} resolves outside the skin's folder"
            )));
        }
        Ok(path)
    }

    // --- delete

    /// Moves the skin to `.deleted/<id>/` (any older copy there is replaced) and stamps when.
    pub fn delete(&self, skin_id: &str) -> Result<(), StoreError> {
        let from = self.folder(skin_id)?;
        if !from.is_dir() {
            return Err(StoreError::NotFound(skin_id.to_string()));
        }
        let deleted = self.root.join(DELETED_DIR);
        std::fs::create_dir_all(&deleted)?;
        let to = deleted.join(skin_id);
        if to.exists() {
            std::fs::remove_dir_all(&to)?;
        }
        std::fs::rename(&from, &to)?;
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        write_atomic(&to.join(DELETED_MARKER), ms.to_string().as_bytes())?;
        Ok(())
    }

    /// Removes a skin's folder for good (a failed import's leftovers); not for a user's delete.
    pub(crate) fn remove_folder(&self, skin_id: &str) -> Result<(), StoreError> {
        let dir = self.folder(skin_id)?;
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    /// Ids in `.deleted/`.
    pub fn deleted_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = std::fs::read_dir(self.root.join(DELETED_DIR))
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| is_user_id(n))
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        ids
    }

    /// Removes what has sat in `.deleted/` for longer than `older_than`; returns how many were removed.
    pub fn purge_deleted(&self, older_than: Duration) -> usize {
        let now = SystemTime::now();
        let mut purged = 0;
        for id in self.deleted_ids() {
            let dir = self.root.join(DELETED_DIR).join(&id);
            let stamped = std::fs::read_to_string(dir.join(DELETED_MARKER))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|ms| UNIX_EPOCH + Duration::from_millis(ms));
            let when = stamped.or_else(|| std::fs::metadata(&dir).and_then(|m| m.modified()).ok());
            let age = when
                .and_then(|w| now.duration_since(w).ok())
                .unwrap_or(Duration::ZERO);
            if age > older_than && std::fs::remove_dir_all(&dir).is_ok() {
                purged += 1;
            }
        }
        purged
    }

    // --- loading

    /// Every user skin (validated against Whimbrel, id taken from its folder) by name, with its palettes.
    pub fn list(&self) -> Vec<(Skin, Vec<Palette>)> {
        let base = whimbrel_default();
        let fallback = default_palette();
        let mut out = Vec::new();
        for id in self.skin_ids() {
            let Some(bytes) = self.manifest(&id) else {
                continue;
            };
            let Some((mut skin, _)) = parse_skin(&bytes, &base) else {
                continue;
            };
            skin.id = id.clone(); // images and history are found by folder, so the folder name wins
                                  // A user palette id is `<skinId>-p<n>`, so it can never shadow a bundled or another skin's palette.
            let palettes = self
                .palette_files(&id)
                .iter()
                .filter_map(|(_, data)| parse_palette(data, &fallback))
                .filter(|p| p.id.starts_with(&format!("{id}-")))
                .collect();
            out.push((skin, palettes));
        }
        out.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        out
    }
}

/// `skin.json` and exactly these palettes (any others are removed).
fn write_current(version: &Version, dir: &Path) -> Result<(), StoreError> {
    for p in &version.palettes {
        if !is_valid_id(&p.id) {
            return Err(StoreError::InvalidId(p.id.clone()));
        }
    }
    std::fs::create_dir_all(dir)?;
    write_atomic(&dir.join("skin.json"), &version.manifest)?;
    let palettes = dir.join("palettes");
    if palettes.exists() {
        std::fs::remove_dir_all(&palettes)?;
    }
    std::fs::create_dir_all(&palettes)?;
    for p in &version.palettes {
        write_atomic(&palettes.join(format!("{}.json", p.id)), &p.data)?;
    }
    Ok(())
}

fn current_palettes(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir.join("palettes"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let id = name.strip_suffix(".json")?.to_string();
                    Some((id, std::fs::read(e.path()).ok()?))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// The flat name an image gets: `(stem, extension)`. Lower-case `a-z0-9.-`, a URL's query and fragment
/// dropped, the stem cut so the whole is at most `NAME_LIMIT`, and a stem of nothing is "image".
fn flat_image_name(preferred: &str) -> (String, String) {
    let path: &str = preferred.split(['?', '#']).next().unwrap_or("");
    let last = path.rsplit('/').next().unwrap_or("");
    let flat: String = last
        .to_lowercase()
        .graphemes(true)
        .map(|g| {
            if g.len() == 1
                && (g.as_bytes()[0].is_ascii_lowercase()
                    || g.as_bytes()[0].is_ascii_digit()
                    || g == ".")
            {
                g
            } else {
                "-"
            }
        })
        .collect();
    let flat = flat.trim_start_matches('.');
    // The text after the last dot, unless it is the whole name; a long "extension" is just part of the name.
    let full_ext = match flat.rfind('.') {
        Some(i) => &flat[i + 1..],
        None => "",
    };
    let ext = if full_ext.len() <= 8 { full_ext } else { "" };
    let whole = if ext.is_empty() {
        flat
    } else {
        &flat[..flat.len() - ext.len() - 1]
    };
    let room = NAME_LIMIT - if ext.is_empty() { 0 } else { ext.len() + 1 };
    let stem: String = whole.chars().take(room).collect();
    (
        if stem.is_empty() {
            "image".to_string()
        } else {
            stem
        },
        ext.to_string(),
    )
}

// --- the contrast gate

/// The result of `save_gate_repair`.
#[derive(Clone, Debug)]
pub struct Gated {
    pub skin: Skin,
    pub palettes: Vec<Palette>,
    /// One sentence per palette adjusted, for the chat to show.
    pub notes: Vec<String>,
}

/// `textSecondary` as "text secondary".
fn words(token: Token) -> String {
    token.name().chars().fold(String::new(), |mut out, c| {
        if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
        out
    })
}

/// `None` when `palette` already passes `archetype`'s pairs in both appearances, or when it cannot be
/// repaired to pass every archetype's pairs (it is then kept as it is); otherwise a copy with id `id`,
/// repaired. Ports `SkinChatSession.archetypeRepair`.
pub fn archetype_repair(
    palette: &Palette,
    id: &str,
    archetype: Archetype,
) -> Option<(Palette, Vec<Token>)> {
    let pairs = archetype_pairs(archetype);
    if pairs_pass(&palette.light, &pairs) && pairs_pass(&palette.dark, &pairs) {
        return None;
    }
    let mut candidate = palette.clone();
    candidate.id = id.to_string();
    if candidate.name.is_empty() {
        candidate.name = "Colours".to_string();
    }
    candidate.author = String::new();
    candidate.variants = Variants::Both;
    let all = all_pairs();
    let (fixed, changed) = repair(&candidate, &all);
    if !(pairs_pass(&fixed.light, &all) && pairs_pass(&fixed.dark, &all)) {
        return None;
    }
    Some((fixed, changed))
}

/// The contrast gate every save and import goes through: each palette the skin names must pass its
/// archetype's pairs. One of the skin's own palettes that fails is repaired in place; a built-in or
/// borrowed one (found by `lookup`) that fails is replaced by a repaired copy `<skin id>-p<N>`.
pub fn save_gate_repair(
    skin: &Skin,
    palettes: &[Palette],
    lookup: &dyn Fn(&str) -> Option<Palette>,
) -> Gated {
    let mut skin = skin.clone();
    let mut palettes = palettes.to_vec();
    let mut notes = Vec::new();
    let archetype = skin.today.archetype;
    let mut next = 1;
    for index in 0..skin.palettes.len() {
        let reference = skin.palettes[index].clone();
        let own = palettes.iter().position(|p| p.id == reference);
        let Some(palette) = own
            .map(|i| palettes[i].clone())
            .or_else(|| lookup(&reference))
        else {
            continue;
        };
        let id = if own.is_some() {
            reference.clone()
        } else {
            while palettes
                .iter()
                .any(|p| p.id == format!("{}-p{next}", skin.id))
            {
                next += 1;
            }
            format!("{}-p{next}", skin.id)
        };
        let Some((fixed, changed)) = archetype_repair(&palette, &id, archetype) else {
            continue;
        };
        let name = fixed.name.clone();
        match own {
            Some(i) => palettes[i] = fixed,
            None => palettes.push(fixed),
        }
        skin.palettes[index] = id;
        let adjusted: Vec<String> = changed.into_iter().map(words).collect();
        notes.push(format!(
            "I adjusted {} in \u{201C}{name}\u{201D} so text stays readable.",
            adjusted.join(", ")
        ));
    }
    Gated {
        skin,
        palettes,
        notes,
    }
}
