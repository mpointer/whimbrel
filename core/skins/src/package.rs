//! A `.whimbrelskin` file: a zip of one of your skins' folders (`<id>/skin.json`, `palettes/*.json`,
//! `images/*`), never its chat or history. Ports `SkinPackage.swift`, entirely in memory: nothing is
//! extracted to disk, every entry name and size is checked from the central directory before any entry
//! is read, and what is read is validated before it reaches the store. Import adds a copy and never
//! overwrites.

use crate::format::whimbrel_default;
use crate::loader::parse_skin;
use crate::palette::{default_palette, parse_palette};
use crate::store::{archetype_repair, skin_name, StoreError, UserSkinStore};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Cursor, Read, Write};
use std::time::Instant;
use zip::write::SimpleFileOptions;

/// Zip-bomb caps: the package file, the listed files, and every byte read out of it.
pub const MAX_FILES: usize = 200;
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
/// The largest single image (and so the largest single entry).
pub const IMAGE_BYTE_LIMIT: usize = 10 * 1024 * 1024;

#[derive(Debug)]
pub enum PackageError {
    Unreadable,
    UnsafeEntry(String),
    TooLarge,
    TooLargeToExport,
    InvalidSkin(String),
    Encrypted,
    TimedOut,
    Store(StoreError),
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageError::Unreadable => write!(f, "this is not a readable skin package"),
            PackageError::UnsafeEntry(n) => {
                write!(f, "the package has an entry that is not allowed: {n}")
            }
            PackageError::TooLarge => write!(f, "the package is too large"),
            PackageError::TooLargeToExport => write!(f, "this skin is too large to export"),
            PackageError::InvalidSkin(n) => {
                write!(f, "the package does not hold a valid skin ({n})")
            }
            PackageError::Encrypted => write!(f, "the package is encrypted"),
            PackageError::TimedOut => write!(f, "importing took too long"),
            PackageError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PackageError {}

impl From<StoreError> for PackageError {
    fn from(e: StoreError) -> Self {
        PackageError::Store(e)
    }
}

fn pretty(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).expect("a JSON value serialises")
}

/// PNG, JPEG, GIF or WebP by their magic bytes, within the size limit.
pub fn is_usable_image(data: &[u8]) -> bool {
    data.len() <= IMAGE_BYTE_LIMIT
        && (data.starts_with(&[0x89, 0x50, 0x4E, 0x47])
            || data.starts_with(&[0xFF, 0xD8, 0xFF])
            || data.starts_with(b"GIF8")
            || (data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP")))
}

// --- export

/// The skin as a package: its manifest, palettes and the images it places. Refuses (`TooLargeToExport`)
/// what import would refuse, so your own export always imports.
pub fn export(store: &UserSkinStore, skin_id: &str) -> Result<Vec<u8>, PackageError> {
    let manifest = store
        .manifest(skin_id)
        .ok_or_else(|| PackageError::InvalidSkin(skin_id.to_string()))?;
    let mut files: Vec<(String, Vec<u8>)> =
        vec![(format!("{skin_id}/skin.json"), manifest.clone())];
    for (id, data) in store.palette_files(skin_id) {
        files.push((format!("{skin_id}/palettes/{id}.json"), data));
    }
    // Only the images this version places (undo, refines and later approvals leave others behind).
    let placed: BTreeSet<String> = serde_json::from_slice::<Value>(&manifest)
        .ok()
        .and_then(|m| m["decoration"]["imagery"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|i| i["asset"].as_str().map(String::from))
        .filter(|n| !n.contains('/') && n != "." && n != "..")
        .collect();
    let mut image_count = 0;
    for name in &placed {
        // The same checks as showing it: not a link, inside the folder.
        if let Ok(path) = store.image_path(skin_id, name) {
            if let Ok(bytes) = std::fs::read(path) {
                files.push((format!("{skin_id}/images/{name}"), bytes));
                image_count += 1;
            }
        }
    }
    let total: usize = files.iter().map(|(_, b)| b.len()).sum();
    if files.len() > MAX_FILES || total > MAX_BYTES {
        return Err(PackageError::TooLargeToExport);
    }

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    // Unix host on every platform: zip 9 marks Windows-written entries DOS, which drops the modes.
    let dir_options = SimpleFileOptions::default()
        .system(zip::System::Unix)
        .unix_permissions(0o755);
    let file_options = SimpleFileOptions::default()
        .system(zip::System::Unix)
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let io = |_| PackageError::Unreadable;
    zip.add_directory(format!("{skin_id}/"), dir_options)
        .map_err(io)?;
    zip.add_directory(format!("{skin_id}/palettes/"), dir_options)
        .map_err(io)?;
    if image_count > 0 {
        zip.add_directory(format!("{skin_id}/images/"), dir_options)
            .map_err(io)?;
    }
    for (name, bytes) in files {
        zip.start_file(name, file_options).map_err(io)?;
        zip.write_all(&bytes)
            .map_err(|_| PackageError::Unreadable)?;
    }
    Ok(zip.finish().map_err(io)?.into_inner())
}

// --- import

struct Package {
    manifest: Vec<u8>,
    palettes: BTreeMap<String, Vec<u8>>,
    images: BTreeMap<String, Vec<u8>>,
}

fn check_deadline(deadline: Instant) -> Result<(), PackageError> {
    if Instant::now() >= deadline {
        Err(PackageError::TimedOut)
    } else {
        Ok(())
    }
}

/// A plain path component: no `..`, leading `.` or `-`, control characters, or any of `\[]*?^`.
fn plain(part: &str) -> bool {
    !part.is_empty()
        && !part.starts_with('.')
        && !part.starts_with('-')
        && !part
            .chars()
            .any(|c| (c as u32) < 0x20 || c as u32 == 0x7F || "\\[]*?^".contains(c))
}

#[derive(PartialEq)]
enum Kind {
    Skin,
    Palette,
    Image,
}

/// The package's skin.json, palettes and images by file name, read from the central directory only.
fn read_package(bytes: &[u8], deadline: Instant) -> Result<Package, PackageError> {
    if bytes.len() > MAX_BYTES {
        return Err(PackageError::TooLarge);
    }
    check_deadline(deadline)?;
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| PackageError::Unreadable)?;
    // The entry count first, so a huge listing is never walked (+3 for the folders).
    if archive.len() > MAX_FILES + 3 {
        return Err(PackageError::TooLarge);
    }

    // Every name is checked before any entry is read: one top folder and the allowed paths.
    let mut files: Vec<(usize, Kind, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut top: Option<String> = None;
    for i in 0..archive.len() {
        // Opening an encrypted entry without a password fails; that is how it shows (nothing is decompressed).
        match archive.by_index(i) {
            Ok(_) => {}
            Err(zip::result::ZipError::UnsupportedArchive(m))
                if m.to_lowercase().contains("password") =>
            {
                return Err(PackageError::Encrypted)
            }
            Err(_) => return Err(PackageError::Unreadable),
        }
        let entry = archive
            .by_index_raw(i)
            .map_err(|_| PackageError::Unreadable)?;
        let name = entry
            .name()
            .map_err(|_| PackageError::Unreadable)?
            .to_string();
        if !seen.insert(name.clone()) {
            return Err(PackageError::Unreadable);
        }
        let is_dir = name.ends_with('/');
        let trimmed = if is_dir {
            &name[..name.len() - 1]
        } else {
            name.as_str()
        };
        let parts: Vec<&str> = trimmed.split('/').collect();
        // A link or any other special file is refused by its mode.
        let mode_ok = match entry.unix_mode().map(|m| m & 0o170000) {
            None => true,
            Some(0o040000) => is_dir,
            Some(0o100000) => !is_dir,
            Some(_) => false,
        };
        let unsafe_entry = || PackageError::UnsafeEntry(name.clone());
        if !mode_ok
            || !parts.iter().all(|p| plain(p))
            || top.as_deref().is_some_and(|t| t != parts[0])
        {
            return Err(unsafe_entry());
        }
        top = Some(parts[0].to_string());
        match (parts.len(), is_dir) {
            (1, true) => {}
            (2, true) if parts[1] == "palettes" || parts[1] == "images" => {}
            (2, false) if parts[1] == "skin.json" => {
                files.push((i, Kind::Skin, "skin.json".into()))
            }
            (3, false) if parts[1] == "palettes" && parts[2].ends_with(".json") => {
                files.push((i, Kind::Palette, parts[2].into()))
            }
            (3, false) if parts[1] == "images" => files.push((i, Kind::Image, parts[2].into())),
            _ => return Err(unsafe_entry()),
        }
    }
    if files.len() > MAX_FILES {
        return Err(PackageError::TooLarge);
    }

    let mut budget = MAX_BYTES;
    let mut manifest = None;
    let mut palettes = BTreeMap::new();
    let mut images = BTreeMap::new();
    for (index, kind, file) in files {
        check_deadline(deadline)?;
        let limit = budget.min(IMAGE_BYTE_LIMIT);
        let mut entry = archive
            .by_index(index)
            .map_err(|_| PackageError::Unreadable)?;
        if entry.size() > limit as u64 {
            return Err(PackageError::TooLarge);
        }
        // Read at most one byte past the limit, whatever the directory claims.
        let mut data = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        loop {
            check_deadline(deadline)?;
            let n = entry
                .read(&mut chunk)
                .map_err(|_| PackageError::Unreadable)?;
            if n == 0 {
                break;
            }
            data.extend_from_slice(&chunk[..n]);
            if data.len() > limit {
                return Err(PackageError::TooLarge);
            }
        }
        budget -= data.len();
        match kind {
            Kind::Skin => manifest = Some(data),
            Kind::Palette => {
                palettes.insert(file, data);
            }
            Kind::Image => {
                images.insert(file, data);
            }
        }
    }
    let manifest = manifest.ok_or_else(|| PackageError::InvalidSkin("no skin.json".into()))?;
    Ok(Package {
        manifest,
        palettes,
        images,
    })
}

/// Adds the package as a new skin of yours and returns its id: `user-` added when missing, and on an id
/// or name clash with one of yours, id `-2`, name " 2" (and so on). Its palettes become `<id>-p<N>`
/// and its history starts at version 1. `deadline` bounds the whole import (use 30 s from now).
pub fn import(
    store: &UserSkinStore,
    zip_bytes: &[u8],
    deadline: Instant,
) -> Result<String, PackageError> {
    let package = read_package(zip_bytes, deadline)?;

    let invalid = |what: &str| PackageError::InvalidSkin(what.to_string());
    let (parsed, _) =
        parse_skin(&package.manifest, &whimbrel_default()).ok_or_else(|| invalid("skin.json"))?;
    let archetype = parsed.today.archetype;
    let Ok(Value::Object(mut manifest)) = serde_json::from_slice::<Value>(&package.manifest) else {
        return Err(invalid("skin.json"));
    };
    let old_id = manifest
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("skin.json"))?
        .to_string();
    let raw_name = manifest
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("skin.json"))?
        .to_string();
    let fallback = default_palette();
    for (name, data) in &package.palettes {
        if parse_palette(data, &fallback).is_none() {
            return Err(invalid(name));
        }
    }
    for (name, data) in &package.images {
        if !is_usable_image(data) {
            return Err(invalid(name));
        }
    }
    check_deadline(deadline)?;

    // New id and name, suffixed together until neither clashes with one of yours.
    let base_id = if old_id.starts_with("user-") {
        old_id.clone()
    } else {
        format!("user-{old_id}")
    };
    let ids: BTreeSet<String> = store.skin_ids().into_iter().collect();
    let names: BTreeSet<String> = ids
        .iter()
        .filter_map(|id| store.manifest(id))
        .filter_map(|m| serde_json::from_slice::<Value>(&m).ok())
        .filter_map(|m| m["name"].as_str().map(String::from))
        .collect();
    // Capped first, so a name of only spaces counts as missing.
    let name = if skin_name(&raw_name, "").is_empty() {
        "My skin".to_string()
    } else {
        raw_name
    };
    let (mut new_id, mut new_name) = (base_id.clone(), skin_name(&name, ""));
    let mut n = 1;
    while ids.contains(&new_id) || names.contains(&new_name) {
        n += 1;
        new_id = format!("{base_id}-{n}");
        new_name = skin_name(&name, &format!(" {n}"));
    }

    // Palettes the manifest lists and the package carries become `<newId>-p<N>`. A listed id the package
    // does not carry stays only when it is a built-in palette (user palettes are all `user-...`), so the
    // skin never borrows another of your skins' palettes. Unlisted files are dropped.
    let mut palette_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut renamed_palettes: BTreeMap<String, String> = BTreeMap::new();
    let mut listed: Vec<Value> = Vec::new();
    for reference in manifest
        .get("palettes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let Some(reference) = reference.as_str() else {
            continue;
        };
        if let Some(id) = renamed_palettes.get(reference) {
            listed.push(Value::String(id.clone()));
            continue;
        }
        let carried = package
            .palettes
            .get(&format!("{reference}.json"))
            .and_then(|data| serde_json::from_slice::<Value>(data).ok())
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            });
        let Some(mut palette) = carried else {
            if !reference.starts_with("user-") {
                listed.push(Value::String(reference.to_string()));
            }
            continue;
        };
        let id = format!("{new_id}-p{}", palette_files.len() + 1);
        palette.insert("id".into(), Value::String(id.clone()));
        let mut rewritten = pretty(&Value::Object(palette));
        // A palette that does not suit the skin's archetype is repaired, as one the chat made would be.
        if let Some((fixed, _)) =
            parse_palette(&rewritten, &fallback).and_then(|p| archetype_repair(&p, &id, archetype))
        {
            rewritten = pretty(&fixed.to_json());
        }
        palette_files.push((id.clone(), rewritten));
        renamed_palettes.insert(reference.to_string(), id.clone());
        listed.push(Value::String(id));
    }
    manifest.insert("palettes".into(), Value::Array(listed));
    manifest.insert("id".into(), Value::String(new_id.clone()));
    manifest.insert("name".into(), Value::String(new_name));

    let result = (|| -> Result<(), PackageError> {
        // Images first: the store gives each its flat name, and the placements follow it.
        let mut renamed: BTreeMap<String, String> = BTreeMap::new();
        for (file, data) in &package.images {
            renamed.insert(file.clone(), store.add_image(&new_id, data, file)?);
        }
        if let Some(Value::Object(decoration)) = manifest.get_mut("decoration") {
            if let Some(Value::Array(imagery)) = decoration.get_mut("imagery") {
                for entry in imagery.iter_mut() {
                    let new = entry
                        .get("asset")
                        .and_then(Value::as_str)
                        .and_then(|a| renamed.get(a))
                        .cloned();
                    if let (Some(new), Some(obj)) = (new, entry.as_object_mut()) {
                        obj.insert("asset".into(), Value::String(new));
                    }
                }
            }
        }
        store.save_raw(
            &new_id,
            &pretty(&Value::Object(manifest.clone())),
            &palette_files,
        )?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = store.remove_folder(&new_id);
        return Err(e);
    }
    Ok(new_id)
}
