//! `.whimbrelskin` export and import.

use serde_json::{json, Value};
use skins::format::Archetype;
use skins::package::{self, PackageError, IMAGE_BYTE_LIMIT, MAX_BYTES, MAX_FILES};
use skins::palette::{default_palette, Palette};
use skins::repair::{archetype_pairs, pairs_pass};
use skins::{whimbrel_default, UserSkinStore};
use std::io::{Cursor, Write};
use std::time::{Duration, Instant};
use zip::write::SimpleFileOptions;

fn store() -> (tempfile::TempDir, UserSkinStore) {
    let dir = tempfile::tempdir().unwrap();
    let s = UserSkinStore::new(dir.path().join("skins"));
    (dir, s)
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

/// A PNG's first bytes: enough for the magic-number check.
fn png() -> Vec<u8> {
    let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend_from_slice(b"fake image body");
    v
}

fn palette_json(id: &str) -> Value {
    let mut p: Palette = default_palette();
    p.id = id.into();
    p.to_json()
}

/// A zip of `entries` (a name ending in `/` is a folder), written as the Mac's `ditto` does: regular files 0644.
fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    // Unix host, as the Mac writes them; on Windows zip 9 would otherwise mark entries DOS.
    let file = SimpleFileOptions::default()
        .system(zip::System::Unix)
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let dir = SimpleFileOptions::default()
        .system(zip::System::Unix)
        .unix_permissions(0o755);
    for (name, bytes) in entries {
        if name.ends_with('/') {
            z.add_directory(*name, dir).unwrap();
        } else {
            z.start_file(*name, file).unwrap();
            z.write_all(bytes).unwrap();
        }
    }
    z.finish().unwrap().into_inner()
}

fn manifest(id: &str, name: &str, extra: Value) -> Value {
    let mut m = serde_json::to_value(whimbrel_default()).unwrap();
    m["id"] = id.into();
    m["name"] = name.into();
    m["palettes"] = json!([format!("{id}-p1")]);
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        m[k] = v;
    }
    m
}

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}

/// A well-formed package for a skin called `id`.
fn package_for(id: &str, name: &str, extra: Value) -> Vec<u8> {
    zip_of(&[
        (&format!("{id}/"), vec![]),
        (
            &format!("{id}/skin.json"),
            bytes(&manifest(id, name, extra)),
        ),
        (&format!("{id}/palettes/"), vec![]),
        (
            &format!("{id}/palettes/{id}-p1.json"),
            bytes(&palette_json(&format!("{id}-p1"))),
        ),
    ])
}

fn saved(s: &UserSkinStore, id: &str) -> Value {
    serde_json::from_slice(&s.manifest(id).unwrap()).unwrap()
}

// --- the happy paths

#[test]
fn import_adds_a_skin_with_renamed_palettes() {
    let (_d, s) = store();
    let id = package::import(
        &s,
        &package_for("user-game-day-1", "Game Day", json!({})),
        soon(),
    )
    .unwrap();
    assert_eq!(id, "user-game-day-1");
    let m = saved(&s, &id);
    assert_eq!(m["name"], "Game Day");
    assert_eq!(m["palettes"], json!(["user-game-day-1-p1"]));
    assert_eq!(s.palette_files(&id).len(), 1);
    assert_eq!(s.list().len(), 1);
    let history = s.folder(&id).unwrap().join("history");
    assert_eq!(
        skins::history::numbers(&history),
        [1],
        "history starts at version 1"
    );
}

#[test]
fn import_adds_user_prefix() {
    let (_d, s) = store();
    let id = package::import(&s, &package_for("game-day", "Game Day", json!({})), soon()).unwrap();
    assert_eq!(id, "user-game-day");
    assert_eq!(saved(&s, &id)["id"], "user-game-day");
    assert_eq!(
        saved(&s, &id)["palettes"],
        json!(["user-game-day-p1"]),
        "palettes follow the new id"
    );
}

#[test]
fn import_clash_renames_id_and_name() {
    let (_d, s) = store();
    let pkg = package_for("user-a-1", "My Skin", json!({}));
    assert_eq!(package::import(&s, &pkg, soon()).unwrap(), "user-a-1");
    let second = package::import(&s, &pkg, soon()).unwrap();
    assert_eq!(second, "user-a-1-2");
    assert_eq!(saved(&s, &second)["name"], "My Skin 2");
    let third = package::import(&s, &pkg, soon()).unwrap();
    assert_eq!(
        (third.as_str(), saved(&s, &third)["name"].as_str().unwrap()),
        ("user-a-1-3", "My Skin 3")
    );
    assert_eq!(s.skin_ids().len(), 3, "import never overwrites");
    assert_eq!(saved(&s, "user-a-1")["name"], "My Skin");
}

#[test]
fn a_name_clash_alone_renames_both() {
    let (_d, s) = store();
    package::import(
        &s,
        &package_for("user-one-1", "Same Name", json!({})),
        soon(),
    )
    .unwrap();
    let id = package::import(
        &s,
        &package_for("user-two-1", "Same Name", json!({})),
        soon(),
    )
    .unwrap();
    // The id was free, but the name was not: both move together to the next suffix.
    assert_eq!(id, "user-two-1-2");
    assert_eq!(saved(&s, &id)["name"], "Same Name 2");
}

#[test]
fn a_long_name_still_fits_with_the_suffix() {
    let (_d, s) = store();
    let name = "n".repeat(40);
    let pkg = package_for("user-a-1", &name, json!({}));
    package::import(&s, &pkg, soon()).unwrap();
    let id = package::import(&s, &pkg, soon()).unwrap();
    let got = saved(&s, &id)["name"].as_str().unwrap().to_string();
    assert_eq!(got.chars().count(), 40);
    assert!(got.ends_with(" 2"));
}

#[test]
fn a_blank_name_becomes_my_skin() {
    let (_d, s) = store();
    let id = package::import(&s, &package_for("user-a-1", "   ", json!({})), soon());
    // A name of only spaces is missing: parse_skin keeps it (non-empty), and import calls it "My skin".
    let id = id.unwrap();
    assert_eq!(saved(&s, &id)["name"], "My skin");
}

#[test]
fn export_then_import_roundtrip() {
    let (_d, a) = store();
    let (_e, b) = store();
    let k = {
        let mut k = whimbrel_default();
        k.id = "user-trip-1".into();
        k.name = "Trip".into();
        k.palettes = vec!["user-trip-1-p1".into()];
        k.today.archetype = Archetype::Standard;
        k.decoration.imagery = vec![skins::format::Image {
            asset: "crest.png".into(),
            placement: skins::format::Placement::Badge,
            text_free: true,
        }];
        k
    };
    let mut p = default_palette();
    p.id = "user-trip-1-p1".into();
    a.save(&k, &[p], &[("crest.png".into(), png())]).unwrap();
    a.add_image("user-trip-1", b"unplaced", "extra.png")
        .unwrap();

    let zip = package::export(&a, "user-trip-1").unwrap();
    let id = package::import(&b, &zip, soon()).unwrap();
    assert_eq!(id, "user-trip-1");
    let m = saved(&b, &id);
    assert_eq!(m["name"], "Trip");
    assert_eq!(m["decoration"]["imagery"][0]["asset"], "crest.png");
    assert_eq!(
        std::fs::read(b.image_path(&id, "crest.png").unwrap()).unwrap(),
        png()
    );
    assert!(
        b.image_path(&id, "extra.png").is_err(),
        "only the images the skin places travel"
    );
    // The same skin on both sides.
    assert_eq!(a.list()[0].0, b.list()[0].0);
    assert_eq!(a.list()[0].1, b.list()[0].1);
}

#[test]
fn export_excludes_chat_and_history() {
    let (_d, s) = store();
    let mut p = default_palette();
    p.id = "user-a-1-p1".into();
    let mut k = whimbrel_default();
    k.id = "user-a-1".into();
    k.name = "A".into();
    k.palettes = vec![p.id.clone()];
    s.save(&k, &[p.clone()], &[]).unwrap();
    s.save(&k, &[p], &[]).unwrap();
    std::fs::write(s.folder("user-a-1").unwrap().join("chat.json"), b"[]").unwrap();
    let zip = package::export(&s, "user-a-1").unwrap();
    let archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
    let mut names: Vec<String> = archive.file_names().map(|n| n.unwrap().into_owned()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "user-a-1/",
            "user-a-1/palettes/",
            "user-a-1/palettes/user-a-1-p1.json",
            "user-a-1/skin.json"
        ]
    );
}

#[test]
fn an_exported_package_is_written_like_the_macs() {
    // Regular files 0644 and folders 0755, with a Unix host, so the Mac's `zipinfo` shows `-rw-r--r--` and `drwxr-xr-x`.
    let (_d, s) = store();
    let mut k = whimbrel_default();
    k.id = "user-a-1".into();
    k.name = "A".into();
    k.palettes = vec![];
    s.save(&k, &[], &[]).unwrap();
    let zip = package::export(&s, "user-a-1").unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
    for i in 0..archive.len() {
        let e = archive.by_index(i).unwrap();
        let mode = e.unix_mode().expect("a Unix mode");
        // The whole mode, so a DOS-marked entry (synthesized 0o775/0o664) fails too.
        assert_eq!(
            mode & 0o170777,
            if e.is_dir() { 0o040755 } else { 0o100644 },
            "{}",
            e.name().unwrap()
        );
    }
}

#[test]
fn export_of_an_unknown_skin_is_an_error() {
    let (_d, s) = store();
    assert!(matches!(
        package::export(&s, "user-nothing-1"),
        Err(PackageError::InvalidSkin(_))
    ));
    assert!(package::export(&s, "../x").is_err());
}

#[test]
fn export_refuses_what_import_would() {
    let (_d, s) = store();
    let mut k = whimbrel_default();
    k.id = "user-big-1".into();
    k.name = "Big".into();
    k.palettes = vec![];
    k.decoration.imagery = (0..MAX_FILES)
        .map(|i| skins::format::Image {
            asset: format!("i{i}.png"),
            placement: skins::format::Placement::Header,
            text_free: false,
        })
        .collect();
    let images: Vec<(String, Vec<u8>)> = (0..MAX_FILES)
        .map(|i| (format!("i{i}.png"), png()))
        .collect();
    s.save(&k, &[], &images).unwrap();
    // 200 images plus skin.json is 201 files.
    assert!(matches!(
        package::export(&s, "user-big-1"),
        Err(PackageError::TooLargeToExport)
    ));
}

// --- palettes and images

#[test]
fn palettes_are_renamed_in_order_and_unlisted_ones_dropped() {
    let (_d, s) = store();
    let mut m = manifest("user-p-1", "P", json!({}));
    m["palettes"] = json!([
        "first",
        "second",
        "default",
        "user-someone-elses-p1",
        "first"
    ]);
    let zip = zip_of(&[
        ("top/skin.json", bytes(&m)),
        ("top/palettes/first.json", bytes(&palette_json("first"))),
        ("top/palettes/second.json", bytes(&palette_json("second"))),
        (
            "top/palettes/unlisted.json",
            bytes(&palette_json("unlisted")),
        ),
    ]);
    let id = package::import(&s, &zip, soon()).unwrap();
    let got = saved(&s, &id);
    // "default" is a built-in and stays; the borrowed user palette is dropped; a repeat maps to the same new id.
    assert_eq!(
        got["palettes"],
        json!([
            format!("{id}-p1"),
            format!("{id}-p2"),
            "default",
            format!("{id}-p1")
        ])
    );
    let ids: Vec<String> = s.palette_files(&id).into_iter().map(|(i, _)| i).collect();
    assert_eq!(ids, [format!("{id}-p1"), format!("{id}-p2")]);
    for (pid, data) in s.palette_files(&id) {
        let v: Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(v["id"], pid.as_str(), "the file carries its new id");
    }
}

#[test]
fn an_imported_palette_that_fails_the_archetype_is_repaired() {
    let (_d, s) = store();
    let zip = package_for(
        "user-g-1",
        "G",
        json!({"today": {"archetype": "glass", "options": {}}}),
    );
    let id = package::import(&s, &zip, soon()).unwrap();
    let (_, palettes) = s.list().remove(0);
    assert_eq!(palettes[0].id, format!("{id}-p1"));
    assert!(pairs_pass(
        &palettes[0].light,
        &archetype_pairs(Archetype::Glass)
    ));
    assert!(pairs_pass(
        &palettes[0].dark,
        &archetype_pairs(Archetype::Glass)
    ));
}

#[test]
fn imagery_follows_the_names_the_store_gives() {
    let (_d, s) = store();
    let m = manifest(
        "user-i-1",
        "I",
        json!({"decoration": {"headerBand": "none", "dividers": "none", "texture": "none", "imagery": [
            {"asset": "Pic One.PNG", "placement": "header", "textFree": false}]}}),
    );
    // The file inside the package is named as the manifest says; the store flattens it, and the placement follows.
    let zip = zip_of(&[
        ("top/skin.json", bytes(&m)),
        (
            "top/palettes/user-i-1-p1.json",
            bytes(&palette_json("user-i-1-p1")),
        ),
        ("top/images/Pic One.PNG", png()),
    ]);
    let id = package::import(&s, &zip, soon()).unwrap();
    assert_eq!(
        saved(&s, &id)["decoration"]["imagery"][0]["asset"],
        "pic-one.png"
    );
    assert!(s.image_path(&id, "pic-one.png").is_ok());
}

#[test]
fn an_image_that_is_not_one_is_refused() {
    let (_d, s) = store();
    let m = manifest("user-i-1", "I", json!({}));
    for body in [
        b"MZ not an image".to_vec(),
        vec![],
        png()
            .into_iter()
            .chain(vec![0u8; IMAGE_BYTE_LIMIT])
            .collect(),
    ] {
        let zip = zip_of(&[("top/skin.json", bytes(&m)), ("top/images/x.png", body)]);
        assert!(package::import(&s, &zip, soon()).is_err());
    }
    assert!(
        s.skin_ids().is_empty(),
        "a refused import leaves nothing behind"
    );
}

#[test]
fn a_package_with_a_bad_manifest_or_palette_is_refused() {
    let (_d, s) = store();
    let ok_palette = bytes(&palette_json("user-a-1-p1"));
    for (skin_json, palette) in [
        (b"not json".to_vec(), ok_palette.clone()),
        (br#"{"name":"No id"}"#.to_vec(), ok_palette.clone()),
        (
            br#"{"id":"Bad Id","name":"x"}"#.to_vec(),
            ok_palette.clone(),
        ),
        (
            bytes(&manifest("user-a-1", "A", json!({}))),
            b"not json".to_vec(),
        ),
        (
            bytes(&manifest("user-a-1", "A", json!({}))),
            br#"{"id":"p","name":"p","light":{"textPrimary":"red"}}"#.to_vec(),
        ),
    ] {
        let zip = zip_of(&[
            ("top/skin.json", skin_json),
            ("top/palettes/user-a-1-p1.json", palette),
        ]);
        assert!(matches!(
            package::import(&s, &zip, soon()),
            Err(PackageError::InvalidSkin(_))
        ));
    }
    let no_skin = zip_of(&[("top/palettes/a.json", ok_palette)]);
    assert!(matches!(
        package::import(&s, &no_skin, soon()),
        Err(PackageError::InvalidSkin(_))
    ));
    assert!(s.skin_ids().is_empty());
}

// --- hostile packages

fn hostile(name: &str) -> Vec<u8> {
    zip_of(&[(name, bytes(&manifest("user-a-1", "A", json!({}))))])
}

#[test]
fn import_refuses_zip_slip() {
    let (_d, s) = store();
    for name in [
        "../skin.json",
        "top/../../skin.json",
        "/etc/skin.json",
        "top/palettes/../../../evil.json",
        "top\\skin.json",
        "C:\\x\\skin.json",
        "top/skin.json/../../x",
        "./top/skin.json",
        "top//skin.json",
        ".hidden/skin.json",
        "-dash/skin.json",
        "top/*.json",
        "top/[a].json",
        "top/ctl\u{1}.json",
    ] {
        let r = package::import(&s, &hostile(name), soon());
        assert!(
            matches!(
                r,
                Err(PackageError::UnsafeEntry(_)) | Err(PackageError::Unreadable)
            ),
            "{name:?}: {r:?}"
        );
    }
    assert!(s.skin_ids().is_empty());
    assert!(!s.root().parent().unwrap().join("evil.json").exists());
}

#[test]
fn import_refuses_entries_outside_the_layout() {
    let (_d, s) = store();
    let m = bytes(&manifest("user-a-1", "A", json!({})));
    for extra in [
        "top/chat.json",
        "top/history/1.json",
        "top/palettes/x.txt",
        "top/images/sub/x.png",
        "top/other/x.json",
        "top/palettes/a/b.json",
        "top/skin.json/x",
    ] {
        let zip = zip_of(&[("top/skin.json", m.clone()), (extra, b"x".to_vec())]);
        assert!(
            matches!(
                package::import(&s, &zip, soon()),
                Err(PackageError::UnsafeEntry(_))
            ),
            "{extra}"
        );
    }
    // Two top folders.
    let zip = zip_of(&[("one/skin.json", m.clone()), ("two/skin.json", m.clone())]);
    assert!(matches!(
        package::import(&s, &zip, soon()),
        Err(PackageError::UnsafeEntry(_))
    ));
    // A file at the root.
    let zip = zip_of(&[("skin.json", m)]);
    assert!(matches!(
        package::import(&s, &zip, soon()),
        Err(PackageError::UnsafeEntry(_))
    ));
}

#[test]
fn import_refuses_links_and_odd_modes_and_sees_duplicates_once() {
    let (_d, s) = store();
    let m = bytes(&manifest("user-a-1", "A", json!({})));
    // The writer will not make two entries of one name, so make them equal after the fact (same length).
    let mut dup = zip_of(&[("top/skin.json", m.clone()), ("top/skin.jsoo", m.clone())]);
    let needle = b"top/skin.jsoo";
    let mut at = 0;
    while let Some(pos) = dup[at..].windows(needle.len()).position(|w| w == needle) {
        dup[at + pos + needle.len() - 1] = b'n';
        at += pos + needle.len();
    }
    // The reader keys entries by name, so one of two equal names is all it ever sees: a second file cannot be
    // smuggled past the checks, and what is imported is a single skin.
    assert_eq!(archive_len(&dup), 1);
    assert!(package::import(&s, &dup, soon()).is_ok());
    assert_eq!(s.skin_ids().len(), 1);

    // The writer masks the mode, so set it in the central directory after the fact.
    let plain = zip_of(&[("top/skin.json", m)]);
    for (mode, what) in [
        (0o120777u16, "a symlink entry"),
        (0o010644, "a special file"),
        (0o040755, "a folder that is a file"),
    ] {
        let odd = with_mode(plain.clone(), "top/skin.json", mode);
        assert!(
            matches!(
                package::import(&s, &odd, soon()),
                Err(PackageError::UnsafeEntry(_))
            ),
            "{what}"
        );
    }
    let folder = zip_of(&[
        ("top/", vec![]),
        (
            "top/skin.json",
            bytes(&manifest("user-a-1", "A", json!({}))),
        ),
    ]);
    let odd = with_mode(folder, "top/", 0o100644);
    assert!(
        matches!(
            package::import(&s, &odd, soon()),
            Err(PackageError::UnsafeEntry(_))
        ),
        "a folder with a file mode"
    );
    assert!(
        package::import(&s, &with_mode(plain, "top/skin.json", 0o100600), soon()).is_ok(),
        "a regular file of another mode is fine"
    );
}

/// Sets the Unix mode (the high half of the external attributes) of the central-directory entry called `name`.
fn with_mode(mut zip: Vec<u8>, name: &str, mode: u16) -> Vec<u8> {
    let mut i = 0;
    while i + 46 <= zip.len() {
        if zip[i..i + 4] == [0x50, 0x4B, 0x01, 0x02] {
            let n = u16::from_le_bytes([zip[i + 28], zip[i + 29]]) as usize;
            if &zip[i + 46..i + 46 + n] == name.as_bytes() {
                zip[i + 40..i + 42].copy_from_slice(&mode.to_le_bytes());
            }
        }
        i += 1;
    }
    zip
}

/// Sets the encryption bit on every central-directory entry (and the local headers).
fn mark_encrypted(mut zip: Vec<u8>) -> Vec<u8> {
    for (sig, flag_at) in [([0x50, 0x4B, 0x01, 0x02], 8), ([0x50, 0x4B, 0x03, 0x04], 6)] {
        let mut i = 0;
        while i + 4 <= zip.len() {
            if zip[i..i + 4] == sig {
                zip[i + flag_at] |= 1;
            }
            i += 1;
        }
    }
    zip
}

#[test]
fn import_refuses_encrypted_entry() {
    let (_d, s) = store();
    let zip = mark_encrypted(package_for("user-a-1", "A", json!({})));
    assert!(matches!(
        package::import(&s, &zip, soon()),
        Err(PackageError::Encrypted)
    ));
    assert!(s.skin_ids().is_empty());
}

#[test]
fn import_refuses_over_200_files() {
    let (_d, s) = store();
    let m = bytes(&manifest("user-a-1", "A", json!({})));
    let mut entries: Vec<(String, Vec<u8>)> = vec![("top/skin.json".into(), m)];
    for i in 0..MAX_FILES {
        entries.push((format!("top/images/{i}.png"), png()));
    }
    let refs: Vec<(&str, Vec<u8>)> = entries
        .iter()
        .map(|(n, b)| (n.as_str(), b.clone()))
        .collect();
    assert_eq!(refs.len(), MAX_FILES + 1);
    assert!(matches!(
        package::import(&s, &zip_of(&refs), soon()),
        Err(PackageError::TooLarge)
    ));
    // The limit itself is fine.
    let ok: Vec<(&str, Vec<u8>)> = refs.iter().take(MAX_FILES).cloned().collect();
    assert!(package::import(&s, &zip_of(&ok), soon()).is_ok());
}

#[test]
fn import_refuses_a_huge_directory_without_walking_it() {
    let (_d, s) = store();
    let m = bytes(&manifest("user-a-1", "A", json!({})));
    let mut entries: Vec<(String, Vec<u8>)> = vec![("top/skin.json".into(), m)];
    for i in 0..(MAX_FILES + 10) {
        entries.push((format!("top/images/{i}.png"), vec![]));
    }
    let refs: Vec<(&str, Vec<u8>)> = entries
        .iter()
        .map(|(n, b)| (n.as_str(), b.clone()))
        .collect();
    assert!(matches!(
        package::import(&s, &zip_of(&refs), soon()),
        Err(PackageError::TooLarge)
    ));
}

#[test]
fn import_refuses_over_64mb() {
    let (_d, s) = store();
    let big = vec![0u8; MAX_BYTES + 1];
    assert!(matches!(
        package::import(&s, &big, soon()),
        Err(PackageError::TooLarge)
    ));
}

#[test]
fn import_refuses_an_entry_that_inflates_past_its_limit() {
    let (_d, s) = store();
    let m = bytes(&manifest("user-a-1", "A", json!({})));
    // 30 MB of zeros deflates to a few KB: a zip bomb the file-size cap alone would let through.
    let bomb = zip_of(&[
        ("top/skin.json", m),
        ("top/images/bomb.png", vec![0u8; 30 * 1024 * 1024]),
    ]);
    assert!(bomb.len() < 1024 * 1024);
    assert!(matches!(
        package::import(&s, &bomb, soon()),
        Err(PackageError::TooLarge)
    ));
    assert!(s.skin_ids().is_empty());
}

#[test]
fn import_enforces_the_total_byte_budget() {
    let (_d, s) = store();
    let m = bytes(&manifest("user-a-1", "A", json!({})));
    // Seven near-limit images are 70 MB of content from a small package; the budget stops it at 64 MB.
    let mut png_body = png();
    png_body.resize(IMAGE_BYTE_LIMIT - 10, 0);
    let mut entries: Vec<(String, Vec<u8>)> = vec![("top/skin.json".into(), m)];
    for i in 0..7 {
        entries.push((format!("top/images/i{i}.png"), png_body.clone()));
    }
    let refs: Vec<(&str, Vec<u8>)> = entries
        .iter()
        .map(|(n, b)| (n.as_str(), b.clone()))
        .collect();
    let zip = zip_of(&refs);
    assert!(zip.len() < MAX_BYTES);
    assert!(matches!(
        package::import(&s, &zip, soon()),
        Err(PackageError::TooLarge)
    ));
}

#[test]
fn import_respects_deadline() {
    let (_d, s) = store();
    let past = Instant::now() - Duration::from_secs(1);
    assert!(matches!(
        package::import(&s, &package_for("user-a-1", "A", json!({})), past),
        Err(PackageError::TimedOut)
    ));
    assert!(s.skin_ids().is_empty());
}

#[test]
fn import_of_garbage_is_unreadable() {
    let (_d, s) = store();
    for bytes in [
        vec![],
        b"not a zip".to_vec(),
        vec![0x50, 0x4B, 0x03, 0x04],
        package_for("user-a-1", "A", json!({}))[..30].to_vec(),
    ] {
        assert!(
            matches!(
                package::import(&s, &bytes, soon()),
                Err(PackageError::Unreadable)
            ),
            "{} bytes",
            bytes.len()
        );
    }
}

// --- a package made by the Mac

const MAC_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/mac-export.whimbrelskin"
);

#[test]
#[ignore = "needs tests/fixtures/mac-export.whimbrelskin, exported by the Mac app"]
fn import_mac_export_fixture() {
    let (_d, s) = store();
    let zip = std::fs::read(MAC_FIXTURE).expect("the Mac-made fixture");
    let id = package::import(&s, &zip, soon()).unwrap();
    assert!(id.starts_with("user-"));
    let (skin, palettes) = s.list().remove(0);
    assert!(!skin.name.is_empty());
    assert!(!palettes.is_empty());
    // And what we export from it imports again.
    let again = package::import(&s, &package::export(&s, &id).unwrap(), soon()).unwrap();
    assert_ne!(again, id);
}

fn archive_len(zip: &[u8]) -> usize {
    zip::ZipArchive::new(Cursor::new(zip)).unwrap().len()
}
