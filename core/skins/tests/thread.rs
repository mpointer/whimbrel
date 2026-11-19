//! The built-in Thread skin (`skins/thread.json`, `skins/palettes/thread.json`, both written by
//! `design/tokens/build.mjs`): it loads with no repair, its palette passes the Standard layout's contrast
//! pairs without being repaired, and it is listed only when the hidden redesign flag is on.

use skins::format::Archetype;
use skins::palette::{default_palette, load_palettes, parse_palette};
use skins::repair::{archetype_pairs, pairs_pass};
use skins::store::archetype_repair;
use skins::{
    is_redesign_skin, load_bundled, parse_skin, visible_palettes, visible_skins, whimbrel_default,
};
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_thread_skin_file_loads_with_zero_repairs() {
    let bytes = std::fs::read(repo().join("skins/thread.json")).unwrap();
    let (skin, repairs) = parse_skin(&bytes, &whimbrel_default()).expect("thread.json parses");
    assert_eq!(repairs, Vec::<String>::new());
    assert_eq!((skin.id.as_str(), skin.name.as_str(), skin.author.as_str()), ("thread", "Thread", "Whimbrel"));
    assert_eq!(skin.palettes, ["thread"]);
    assert_eq!(skin.today.archetype, Archetype::Standard);
    let options: Vec<(&str, &str)> = skin.today.options.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(
        options,
        [("actions", "buttons"), ("ask", "field"), ("labels", "plain"), ("nextMeeting", "card"), ("timeline", "line")]
    );
    // Every language keeps its strings: voice and translations are Whimbrel's.
    let base = whimbrel_default();
    assert_eq!(skin.voice, base.voice);
    let whimbrel_file = std::fs::read(repo().join("skins/whimbrel.json")).unwrap();
    let (whimbrel, _) = parse_skin(&whimbrel_file, &base).unwrap();
    assert_eq!(skin.voice_localized, whimbrel.voice_localized);
    assert!(skin.voice_localized.contains_key("es"));
    assert_eq!(skin.icons, whimbrel.icons);
}

#[test]
fn the_thread_palette_file_loads_with_both_variants() {
    let bytes = std::fs::read(repo().join("skins/palettes/thread.json")).unwrap();
    let palette = parse_palette(&bytes, &default_palette()).expect("thread palette parses");
    assert_eq!((palette.id.as_str(), palette.name.as_str()), ("thread", "Thread"));
    // Written in the file's own words: every color is stated, so no fallback color was used.
    let file: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for variant in ["light", "dark"] {
        assert_eq!(file[variant].as_object().unwrap().len(), 15, "{variant}");
    }
    assert_eq!(palette.to_json(), file, "serialises back to the file");
}

#[test]
fn the_thread_palette_passes_the_standard_layout_unrepaired() {
    let fallback = default_palette();
    let palette = load_palettes(&repo().join("skins/palettes"), &fallback)
        .into_iter()
        .find(|p| p.id == "thread")
        .expect("thread is in skins/palettes");
    let standard = archetype_pairs(Archetype::Standard);
    assert!(pairs_pass(&palette.light, &standard), "light");
    assert!(pairs_pass(&palette.dark, &standard), "dark");
    assert!(archetype_repair(&palette, "thread", Archetype::Standard).is_none(), "the app's own repair finds nothing to fix");
}

#[test]
fn thread_is_listed_only_when_the_redesign_flag_is_on() {
    let all = load_bundled(&repo().join("skins"));
    assert!(all.iter().any(|s| s.id == "thread"), "the loader does not know about the flag");
    let off: Vec<String> = visible_skins(all.clone(), false).into_iter().map(|s| s.id).collect();
    assert!(!off.contains(&"thread".to_string()));
    assert_eq!(off.len(), all.len() - 1);
    assert_eq!(off[0], "whimbrel");
    let on: Vec<String> = visible_skins(all.clone(), true).into_iter().map(|s| s.id).collect();
    assert_eq!(on, all.iter().map(|s| s.id.clone()).collect::<Vec<_>>());
    assert!(on.contains(&"thread".to_string()));
    assert!(is_redesign_skin("thread") && !is_redesign_skin("whimbrel") && !is_redesign_skin("user-thread-1"));
}

#[test]
fn the_thread_palette_is_listed_only_when_the_redesign_flag_is_on() {
    let pool = load_palettes(&repo().join("skins/palettes"), &default_palette());
    assert!(pool.iter().any(|p| p.id == "thread"));
    assert!(!visible_palettes(pool.clone(), false).iter().any(|p| p.id == "thread"));
    assert_eq!(visible_palettes(pool.clone(), true).len(), pool.len());
    assert_eq!(visible_palettes(pool.clone(), false).len(), pool.len() - 1);
}
