//! The user skin store and its undo history.

use skins::format::{Archetype, Skin};
use skins::history::{self, Version, VersionPalette};
use skins::palette::{default_palette, Palette, Token};
use skins::repair::{all_pairs, archetype_pairs, pairs_pass};
use skins::store::{skin_name, StoreError, DELETED_DIR, HISTORY_LIMIT};
use skins::{whimbrel_default, UserSkinStore};
use std::time::Duration;

fn store() -> (tempfile::TempDir, UserSkinStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = UserSkinStore::new(dir.path().join("skins"));
    (dir, store)
}

fn skin(id: &str, name: &str) -> Skin {
    let mut s = whimbrel_default();
    s.id = id.into();
    s.name = name.into();
    s.palettes = vec![format!("{id}-p1")];
    s
}

fn palette(id: &str) -> Palette {
    let mut p = default_palette();
    p.id = id.into();
    p
}

/// A skin whose palette already passes every archetype, so the gate leaves it alone.
fn safe_palette(id: &str) -> Palette {
    let all = all_pairs();
    let p = palette(id);
    let (fixed, _) = skins::repair(&p, &all);
    assert!(pairs_pass(&fixed.light, &all) && pairs_pass(&fixed.dark, &all));
    fixed
}

fn save(store: &UserSkinStore, id: &str, name: &str) -> u32 {
    store
        .save(&skin(id, name), &[safe_palette(&format!("{id}-p1"))], &[])
        .unwrap()
}

// --- ids and names

#[test]
fn folders_exist_only_for_user_ids() {
    let (_d, s) = store();
    assert!(s.folder("user-a-1").is_ok());
    for bad in [
        "whimbrel",
        "user-",
        "User-a",
        "user-a/b",
        "user-..",
        "user-a b",
        "../user-a",
        "",
        "user-a\\b",
        "user-é",
    ] {
        assert!(
            matches!(s.folder(bad), Err(StoreError::InvalidId(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn name_capped_before_empty_check() {
    assert_eq!(skin_name("   ", ""), "");
    assert_eq!(skin_name("  Game Day  ", ""), "Game Day");
    let long = "x".repeat(60);
    assert_eq!(skin_name(&long, "").chars().count(), 40);
    assert_eq!(skin_name(&long, " 2").chars().count(), 40);
    assert!(skin_name(&long, " 2").ends_with("x 2"));
    // A cut that lands after a space drops that space.
    let spaced = format!("{} {}", "a".repeat(39), "b".repeat(10));
    assert_eq!(skin_name(&spaced, ""), "a".repeat(39));
    // A name of only spaces past the cut counts as empty once capped.
    assert_eq!(skin_name(&format!("{}x", " ".repeat(5)), ""), "x");
    // Counted in characters as a person sees them.
    assert_eq!(skin_name(&"e\u{0301}".repeat(45), "").chars().count(), 80);
}

#[test]
fn new_ids_are_slugged_and_unique() {
    let (_d, s) = store();
    assert_eq!(s.new_skin_id("Game Day!"), "user-game-day-1");
    assert_eq!(s.new_skin_id(""), "user-skin-1");
    assert_eq!(s.new_skin_id("***"), "user-skin-1");
    save(&s, "user-game-day-1", "Game Day");
    assert_eq!(s.new_skin_id("Game Day"), "user-game-day-2");
    // Cut at 40 characters; a cut that ends on a hyphen loses it.
    let id = s.new_skin_id(&format!("{} {}", "a".repeat(39), "b"));
    assert_eq!(id, format!("user-{}-1", "a".repeat(39)));
    assert!(id.len() <= "user-".len() + 40 + 2);
}

// --- history

/// A `history/1.json` as the Mac's `JSONEncoder` writes a `Version` (`Data` as base64, here in its key order).
const MAC_HISTORY: &str = r#"{"manifest":"eyJpZCI6InVzZXItZ2FtZS1kYXktMSIsIm5hbWUiOiJHYW1lIERheSJ9","palettes":[{"data":"eyJpZCI6InVzZXItZ2FtZS1kYXktMS1wMSJ9","id":"user-game-day-1-p1"}]}"#;

#[test]
fn history_roundtrips_mac_format() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("1.json"), MAC_HISTORY).unwrap();
    let v = history::read(dir.path(), 1).unwrap();
    assert_eq!(v.manifest, br#"{"id":"user-game-day-1","name":"Game Day"}"#);
    assert_eq!(v.palettes.len(), 1);
    assert_eq!(v.palettes[0].id, "user-game-day-1-p1");
    assert_eq!(v.palettes[0].data, br#"{"id":"user-game-day-1-p1"}"#);
    // What Rust writes is what Swift reads: the same keys, base64 strings for the bytes.
    history::write(dir.path(), 2, &v).unwrap();
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("2.json")).unwrap()).unwrap();
    let mac: serde_json::Value = serde_json::from_str(MAC_HISTORY).unwrap();
    assert_eq!(written, mac);
    assert_eq!(history::read(dir.path(), 2).unwrap(), v);
    assert_eq!(history::numbers(dir.path()), [1, 2]);
}

#[test]
fn history_rejects_bad_base64_and_unknown_files_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("1.json"),
        r#"{"manifest":"not base64!!","palettes":[]}"#,
    )
    .unwrap();
    assert!(history::read(dir.path(), 1).is_err());
    std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
    std::fs::write(dir.path().join("x.json"), "x").unwrap();
    std::fs::write(dir.path().join("3.json"), "{}").unwrap();
    assert_eq!(history::numbers(dir.path()), [1, 3]);
}

#[test]
fn saving_appends_versions_and_returns_their_numbers() {
    let (_d, s) = store();
    assert_eq!(save(&s, "user-a-1", "A"), 1);
    assert_eq!(save(&s, "user-a-1", "A2"), 2);
    assert_eq!(save(&s, "user-b-1", "B"), 1, "numbers are per skin");
}

#[test]
fn undo_restores_exactly() {
    let (_d, s) = store();
    s.save_raw(
        "user-a-1",
        b"manifest one",
        &[("user-a-1-p1".into(), b"palette one".to_vec())],
    )
    .unwrap();
    s.save_raw(
        "user-a-1",
        b"manifest two",
        &[("user-a-1-p2".into(), b"palette two".to_vec())],
    )
    .unwrap();
    assert_eq!(s.manifest("user-a-1").unwrap(), b"manifest two");
    assert!(s.undo("user-a-1").unwrap());
    assert_eq!(s.manifest("user-a-1").unwrap(), b"manifest one");
    assert_eq!(
        s.palette_files("user-a-1"),
        [("user-a-1-p1".to_string(), b"palette one".to_vec())],
        "version two's palette is gone"
    );
    assert!(
        !s.undo("user-a-1").unwrap(),
        "nothing before the first version"
    );
    assert_eq!(s.manifest("user-a-1").unwrap(), b"manifest one");
    // A save after an undo continues from the restored version.
    assert_eq!(s.save_raw("user-a-1", b"manifest three", &[]).unwrap(), 2);
}

#[test]
fn undo_of_an_unknown_skin_is_false_and_of_a_bad_id_an_error() {
    let (_d, s) = store();
    assert!(!s.undo("user-nothing-1").unwrap());
    assert!(s.undo("../x").is_err());
}

#[test]
fn history_capped_at_20() {
    let (_d, s) = store();
    for i in 1..=25 {
        s.save_raw("user-a-1", format!("manifest {i}").as_bytes(), &[])
            .unwrap();
    }
    let dir = s.folder("user-a-1").unwrap().join("history");
    let numbers = history::numbers(&dir);
    assert_eq!(numbers.len(), HISTORY_LIMIT);
    assert_eq!((numbers[0], *numbers.last().unwrap()), (6, 25));
    for _ in 0..19 {
        assert!(s.undo("user-a-1").unwrap());
    }
    assert!(!s.undo("user-a-1").unwrap());
    assert_eq!(s.manifest("user-a-1").unwrap(), b"manifest 6");
}

#[test]
fn a_palette_id_that_is_not_an_id_is_refused() {
    let (_d, s) = store();
    let err = s.save_raw("user-a-1", b"{}", &[("../evil".into(), b"x".to_vec())]);
    assert!(matches!(err, Err(StoreError::InvalidId(_))));
    assert!(!s.root().join("evil.json").exists());
}

#[test]
fn a_version_serialises_like_the_macs_codable() {
    let v = Version {
        manifest: vec![0xFF, 0x00, 0x7F],
        palettes: vec![VersionPalette {
            id: "p".into(),
            data: vec![1, 2, 3, 4],
        }],
    };
    let json = serde_json::to_value(&v).unwrap();
    assert_eq!(json["manifest"], "/wB/");
    assert_eq!(json["palettes"][0]["data"], "AQIDBA==");
}

// --- saving typed skins, the gate, loading

#[test]
fn a_saved_skin_lists_with_its_palettes() {
    let (_d, s) = store();
    save(&s, "user-a-1", "Zed");
    save(&s, "user-b-1", "Alpha");
    let listed = s.list();
    assert_eq!(
        listed
            .iter()
            .map(|(k, _)| k.name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Zed"]
    );
    assert_eq!(listed[0].0.id, "user-b-1");
    assert_eq!(listed[0].1.len(), 1);
    assert_eq!(listed[0].1[0].id, "user-b-1-p1");
}

#[test]
fn list_skips_broken_skins_and_foreign_palettes() {
    let (_d, s) = store();
    save(&s, "user-a-1", "Good");
    let broken = s.folder("user-broken-1").unwrap();
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("skin.json"), "not json").unwrap();
    std::fs::create_dir_all(s.folder("user-empty-1").unwrap()).unwrap();
    // A palette whose id is not `<skin>-...` cannot shadow another skin's colours.
    let dir = s.folder("user-a-1").unwrap().join("palettes");
    let mut foreign = safe_palette("slate");
    foreign.name = "Foreign".into();
    std::fs::write(
        dir.join("slate.json"),
        serde_json::to_vec(&foreign.to_json()).unwrap(),
    )
    .unwrap();
    let listed = s.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0]
            .1
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["user-a-1-p1"]
    );
}

#[test]
fn the_folder_name_wins_over_the_manifest_id() {
    let (_d, s) = store();
    let mut k = skin("user-other-9", "Mismatch");
    k.palettes = vec![];
    s.save_raw("user-a-1", &serde_json::to_vec(&k).unwrap(), &[])
        .unwrap();
    assert_eq!(s.list()[0].0.id, "user-a-1");
}

#[test]
fn save_repairs_a_palette_that_fails_the_archetype() {
    let (_d, s) = store();
    let mut k = skin("user-glass-1", "Glassy");
    k.today.archetype = Archetype::Glass;
    let raw = palette("user-glass-1-p1"); // Default fails Glass (secondary text on the glow)
    assert!(!pairs_pass(&raw.light, &archetype_pairs(Archetype::Glass)));
    s.save(&k, std::slice::from_ref(&raw), &[]).unwrap();
    let (_, palettes) = s.list().remove(0);
    let saved = &palettes[0];
    assert_eq!(saved.id, "user-glass-1-p1");
    assert!(pairs_pass(&saved.light, &archetype_pairs(Archetype::Glass)));
    assert!(pairs_pass(&saved.dark, &archetype_pairs(Archetype::Glass)));
    assert_ne!(saved.light, raw.light);
    // Hue and the rest of the palette are kept.
    assert_eq!(
        saved.light.get(Token::BackgroundPage),
        raw.light.get(Token::BackgroundPage)
    );
}

#[test]
fn save_leaves_a_passing_palette_byte_identical() {
    let (_d, s) = store();
    let p = safe_palette("user-a-1-p1");
    s.save(&skin("user-a-1", "A"), std::slice::from_ref(&p), &[])
        .unwrap();
    assert_eq!(s.list()[0].1[0], p);
}

#[test]
fn the_gate_replaces_a_borrowed_palette_with_a_repaired_copy() {
    let mut k = skin("user-a-1", "A");
    k.today.archetype = Archetype::Glass;
    k.palettes = vec!["default".into(), "slate".into()];
    let lookup = |id: &str| match id {
        "default" => Some(default_palette()),
        _ => None,
    };
    let gated = skins::save_gate_repair(&k, &[], &lookup);
    assert_eq!(
        gated.skin.palettes,
        ["user-a-1-p1", "slate"],
        "default fails Glass and is copied; slate cannot be found"
    );
    assert_eq!(gated.palettes.len(), 1);
    assert_eq!(gated.palettes[0].id, "user-a-1-p1");
    assert_eq!(gated.notes.len(), 1);
    assert!(
        gated.notes[0].starts_with("I adjusted text secondary"),
        "{}",
        gated.notes[0]
    );
    assert!(gated.notes[0].ends_with("so text stays readable."));
    // A skin that suits its borrowed palette is left alone.
    k.today.archetype = Archetype::Standard;
    let same = skins::save_gate_repair(&k, &[], &lookup);
    assert_eq!(same.skin.palettes, k.palettes);
    assert!(same.palettes.is_empty() && same.notes.is_empty());
}

#[test]
fn saving_with_images_writes_them_by_name_and_refuses_bad_names() {
    let (_d, s) = store();
    let p = safe_palette("user-a-1-p1");
    s.save(
        &skin("user-a-1", "A"),
        std::slice::from_ref(&p),
        &[("pitch.png".into(), b"png".to_vec())],
    )
    .unwrap();
    assert_eq!(
        std::fs::read(s.image_path("user-a-1", "pitch.png").unwrap()).unwrap(),
        b"png"
    );
    for bad in ["../x.png", "A.png", ".hidden", "a/b.png", "", "sp ace.png"] {
        let r = s.save(
            &skin("user-a-1", "A"),
            std::slice::from_ref(&p),
            &[(bad.into(), b"x".to_vec())],
        );
        assert!(matches!(r, Err(StoreError::InvalidImageName(_))), "{bad:?}");
    }
}

// --- rename and delete

#[test]
fn rename_is_a_new_version_and_can_be_undone() {
    let (_d, s) = store();
    save(&s, "user-a-1", "Old");
    s.rename("user-a-1", &format!("  {}  ", "n".repeat(60)))
        .unwrap();
    assert_eq!(s.list()[0].0.name, "n".repeat(40));
    assert_eq!(s.list()[0].1.len(), 1, "palettes are kept");
    assert!(s.undo("user-a-1").unwrap());
    assert_eq!(s.list()[0].0.name, "Old");
    assert!(s.rename("user-nothing-1", "x").is_err());
}

#[test]
fn delete_moves_to_deleted_and_purge_after_30_days() {
    let (_d, s) = store();
    save(&s, "user-a-1", "A");
    save(&s, "user-b-1", "B");
    s.delete("user-a-1").unwrap();
    assert_eq!(s.skin_ids(), ["user-b-1"]);
    assert!(s.list().iter().all(|(k, _)| k.id != "user-a-1"));
    assert_eq!(s.deleted_ids(), ["user-a-1"]);
    assert!(
        s.root()
            .join(DELETED_DIR)
            .join("user-a-1/skin.json")
            .exists(),
        "nothing is lost"
    );
    // Fresh: kept. Stamped 31 days ago: purged.
    assert_eq!(s.purge_deleted(Duration::from_secs(30 * 86_400)), 0);
    let marker = s.root().join(DELETED_DIR).join("user-a-1/.deleted-at");
    let ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        - 31 * 86_400_000;
    std::fs::write(&marker, ago.to_string()).unwrap();
    assert_eq!(s.purge_deleted(Duration::from_secs(30 * 86_400)), 1);
    assert!(s.deleted_ids().is_empty());
    assert_eq!(s.skin_ids(), ["user-b-1"], "a live skin is never purged");
}

#[test]
fn deleting_again_replaces_the_older_copy_and_a_missing_skin_is_an_error() {
    let (_d, s) = store();
    save(&s, "user-a-1", "First");
    s.delete("user-a-1").unwrap();
    save(&s, "user-a-1", "Second");
    s.delete("user-a-1").unwrap();
    let kept =
        std::fs::read_to_string(s.root().join(DELETED_DIR).join("user-a-1/skin.json")).unwrap();
    assert!(kept.contains("Second"));
    assert!(matches!(
        s.delete("user-gone-1"),
        Err(StoreError::NotFound(_))
    ));
    assert!(s.delete("whimbrel").is_err());
}

#[test]
fn deleted_skins_never_appear_as_skins() {
    let (_d, s) = store();
    save(&s, "user-a-1", "A");
    s.delete("user-a-1").unwrap();
    assert!(s.skin_ids().is_empty());
    assert!(s.list().is_empty());
    assert!(s.folder(".deleted").is_err());
}

// --- images

#[test]
fn image_names_are_flat_unique_and_bounded() {
    let (_d, s) = store();
    save(&s, "user-a-1", "A");
    let add = |name: &str| s.add_image("user-a-1", b"x", name).unwrap();
    assert_eq!(
        add("https://example.com/Pics/My Photo.PNG?size=big#top"),
        "my-photo.png"
    );
    assert_eq!(add("my-photo.png"), "my-photo-2.png");
    assert_eq!(add("my-photo.png"), "my-photo-3.png");
    assert_eq!(add(".hidden"), "hidden");
    assert_eq!(add("noext"), "noext");
    assert_eq!(add("noext"), "noext-2");
    assert_eq!(add(""), "image");
    assert_eq!(
        add("???"),
        "image-2",
        "a name of only a query leaves nothing"
    );
    assert_eq!(add("é.jpg"), "-.jpg");
    assert_eq!(add("archive.tar.gz"), "archive.tar.gz");
    assert_eq!(
        add("file.abcdefgh"),
        "file.abcdefgh",
        "eight characters is an extension"
    );
    assert_eq!(
        add("file.abcdefghi"),
        "file.abcdefghi",
        "nine is part of the name"
    );
    let nine = add(&format!("{}.abcdefghi", "z".repeat(80)));
    let eight = add(&format!("{}.abcdefgh", "z".repeat(80)));
    assert_eq!((nine.len(), eight.len()), (40, 40));
    assert!(
        nine.starts_with(&"z".repeat(40)),
        "no extension, so the stem takes all 40"
    );
    assert!(eight.ends_with(".abcdefgh") && eight.starts_with(&"z".repeat(31)));
    assert_eq!(
        add("file.waytoolongext"),
        "file.waytoolongext",
        "a long extension is part of the name"
    );
    let long = add(&format!("{}.jpeg", "a".repeat(80)));
    assert_eq!(long.len(), 40, "stem and extension fit the limit together");
    assert!(long.ends_with(".jpeg"));
    assert_eq!(add("sub/dir/Pic.JPG"), "pic.jpg");
    for name in std::fs::read_dir(s.folder("user-a-1").unwrap().join("images")).unwrap() {
        let name = name.unwrap().file_name().to_string_lossy().into_owned();
        assert!(skins::store::is_image_name(&name), "{name}");
    }
}

#[test]
fn image_path_refuses_names_that_are_not_images() {
    let (_d, s) = store();
    save(&s, "user-a-1", "A");
    s.add_image("user-a-1", b"x", "ok.png").unwrap();
    assert!(s.image_path("user-a-1", "ok.png").is_ok());
    for bad in [
        "../skin.json",
        "..",
        ".",
        "a/b",
        "OK.png",
        "",
        ".hidden",
        "ok.png\\",
    ] {
        assert!(s.image_path("user-a-1", bad).is_err(), "{bad:?}");
    }
    assert!(matches!(
        s.image_path("user-a-1", "missing.png"),
        Err(StoreError::NotFound(_))
    ));
    assert!(s.image_path("whimbrel", "ok.png").is_err());
    // A folder where a file belongs is not an image.
    std::fs::create_dir(s.folder("user-a-1").unwrap().join("images/dir.png")).unwrap();
    assert!(s.image_path("user-a-1", "dir.png").is_err());
}

#[cfg(unix)]
fn link(target: &std::path::Path, at: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, at).is_ok()
}

#[cfg(windows)]
fn link(target: &std::path::Path, at: &std::path::Path) -> bool {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, at).is_ok()
    } else {
        std::os::windows::fs::symlink_file(target, at).is_ok()
    }
}

#[test]
fn image_path_refuses_symlink() {
    let (d, s) = store();
    save(&s, "user-a-1", "A");
    let outside = d.path().join("secret.png");
    std::fs::write(&outside, b"secret").unwrap();
    let images = s.folder("user-a-1").unwrap().join("images");
    std::fs::create_dir_all(&images).unwrap();
    if !link(&outside, &images.join("leak.png")) {
        eprintln!("skipped: this account cannot create symlinks");
        return;
    }
    assert!(matches!(
        s.image_path("user-a-1", "leak.png"),
        Err(StoreError::Unsafe(_))
    ));
    // A linked images folder is refused too, even for a real file inside it.
    let other = d.path().join("elsewhere");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("real.png"), b"x").unwrap();
    std::fs::remove_dir_all(&images).unwrap();
    assert!(link(&other, &images));
    assert!(matches!(
        s.image_path("user-a-1", "real.png"),
        Err(StoreError::Unsafe(_))
    ));
}
