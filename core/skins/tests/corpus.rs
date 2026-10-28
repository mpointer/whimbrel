//! Runs the shared corpus in `skins/testdata/` (the Mac's `SkinCorpusTests` runs the same files), and
//! checks the shipped skins and the icon table.

use serde_json::Value;
use skins::format::{Face, Family, Role, VoiceKey, Weight};
use skins::{icon_table, is_valid_id, whimbrel_default, load_bundled, parse_skin};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo_skins() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skins")
}

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// A case's input: an object is the manifest; a string is the file's raw text (for "not JSON").
fn input_bytes(input: &Value) -> Vec<u8> {
    match input {
        Value::String(s) => s.as_bytes().to_vec(),
        other => serde_json::to_vec(other).unwrap(),
    }
}

/// JSON equality that treats 12 and 12.0 as the same number (a manifest may spell either).
fn same_json(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_json(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_json(v, w)))
        }
        _ => a == b,
    }
}

fn corpus_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(repo_skins().join("testdata"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
}

#[test]
fn corpus_cases_match() {
    let base = whimbrel_default();
    let files = corpus_files();
    assert!(
        files.len() >= 30,
        "the corpus has shrunk: {} cases",
        files.len()
    );
    let mut failures = Vec::new();
    for file in &files {
        let case = read_json(file);
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let want_repairs: Vec<String> = case["repairs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        match (
            parse_skin(&input_bytes(&case["input"]), &base),
            &case["expected"],
        ) {
            (None, Value::Null) => {}
            (None, _) => failures.push(format!("{name}: skipped, but a skin was expected")),
            (Some(_), Value::Null) => {
                failures.push(format!("{name}: loaded, but the file should be skipped"))
            }
            (Some((skin, repairs)), expected) => {
                let got = serde_json::to_value(&skin).unwrap();
                if !same_json(&got, expected) {
                    failures.push(format!(
                        "{name}: skin differs\n  got      {got}\n  expected {expected}"
                    ));
                }
                if repairs != want_repairs {
                    failures.push(format!("{name}: repairs differ\n  got      {repairs:?}\n  expected {want_repairs:?}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} corpus case(s) disagree:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn shipped_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(repo_skins())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
        .filter(|p| p.file_name().unwrap() != "icons.json")
        .collect();
    files.sort();
    files
}

#[test]
fn bundled_skins_load_without_repairs() {
    let base = whimbrel_default();
    let files = shipped_files();
    assert_eq!(files.len(), 7);
    for file in files {
        let (_, repairs) = parse_skin(&std::fs::read(&file).unwrap(), &base)
            .unwrap_or_else(|| panic!("{file:?} skipped"));
        assert_eq!(repairs, Vec::<String>::new(), "{file:?}");
    }
}

#[test]
fn whimbrel_json_mirrors_the_default() {
    let base = whimbrel_default();
    let (skin, repairs) = parse_skin(
        &std::fs::read(repo_skins().join("whimbrel.json")).unwrap(),
        &base,
    )
    .unwrap();
    assert!(repairs.is_empty());
    assert_eq!(skin, base);
}

#[test]
fn load_bundled_puts_the_base_first_then_by_name() {
    let skins = load_bundled(&repo_skins());
    let names: Vec<&str> = skins.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(skins.len(), 7);
    assert_eq!(skins[0].id, "whimbrel");
    let mut rest = names[1..].to_vec();
    rest.sort();
    assert_eq!(&names[1..], rest.as_slice());
}

#[test]
fn load_bundled_skips_broken_files_and_keeps_the_first_duplicate() {
    let skins = load_bundled(&repo_skins().join("invalid"));
    let ids: Vec<&str> = skins.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids[0], "whimbrel");
    assert_eq!(
        skins
            .iter()
            .filter(|s| s.id == "dup")
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Dup A"]
    );
    assert!(!ids.contains(&"game day") && !ids.contains(&"no-name"));
}

#[test]
fn load_bundled_of_a_missing_directory_is_just_the_base() {
    let skins = load_bundled(&repo_skins().join("no-such-dir"));
    assert_eq!(skins, vec![whimbrel_default()]);
}

#[test]
fn icon_table_covers_every_role_with_alternates() {
    let table = icon_table();
    let symbols: BTreeSet<&str> = table.iter().map(|e| e.symbol.as_str()).collect();
    assert_eq!(symbols.len(), table.len(), "a symbol is listed twice");
    let base = whimbrel_default();
    for role in Role::ALL {
        let for_role: Vec<_> = table.iter().filter(|e| e.role == role).collect();
        assert!(
            for_role.len() >= 7,
            "{role:?} has only {} entries",
            for_role.len()
        );
        assert!(
            for_role.iter().any(|e| e.symbol == base.icons.get(role)),
            "{role:?} lacks Whimbrel's own icon"
        );
    }
    for e in table {
        for (label, cp) in [("fluent", e.fluent), ("mdl2", e.mdl2)] {
            assert!(
                (0xE000..=0xF8FF).contains(&cp),
                "{} {label} {cp:#x} is outside the private use area",
                e.symbol
            );
        }
    }
}

#[test]
fn every_symbol_a_shipped_skin_uses_is_in_the_table() {
    for file in shipped_files() {
        let v = read_json(&file);
        for (role, symbol) in v["icons"].as_object().unwrap() {
            assert!(
                skins::is_allowed_icon(symbol.as_str().unwrap()),
                "{file:?} {role}: {symbol}"
            );
        }
    }
}

#[test]
fn faces_parse_and_spell_canonically() {
    assert_eq!(Face::parse("SF Pro").unwrap().spec(), "SF Pro/regular");
    let f = Face::parse("SF Pro/heavy/italic").unwrap();
    assert_eq!(
        (f.family, f.weight, f.italic),
        (Family::SfPro, Weight::Heavy, true)
    );
    assert_eq!(f.spec(), "SF Pro/heavy/italic");
    for bad in [
        "",
        "Comic Sans",
        "SF Pro/wide",
        "SF Pro/bold/oblique",
        "SF Pro/bold/italic/x",
        "sf pro",
        "SF Pro/",
    ] {
        assert!(Face::parse(bad).is_none(), "{bad:?} must not parse");
    }
    assert_eq!(Weight::Black.css(), 900);
    assert_eq!(Weight::UltraLight.css(), 100);
}

#[test]
fn ids_are_lowercase_letters_digits_and_hyphens() {
    for ok in ["a", "game-day", "user-2", "-", "0"] {
        assert!(is_valid_id(ok), "{ok}");
    }
    for bad in ["", "Game", "a b", "a_b", "é", "a/b", "a.b"] {
        assert!(!is_valid_id(bad), "{bad}");
    }
}

#[test]
fn voice_limits_count_name_as_thirty_characters() {
    assert!(VoiceKey::GreetingMorning.fits(&format!("{}{{name}}", "A".repeat(18))));
    assert!(!VoiceKey::GreetingMorning.fits(&format!("{}{{name}}", "A".repeat(19))));
    assert!(!VoiceKey::Tagline.fits(""));
    assert!(VoiceKey::Tagline.fits(&"t".repeat(60)) && !VoiceKey::Tagline.fits(&"t".repeat(61)));
    // Whimbrel's own wording must fit its limits.
    let base = whimbrel_default();
    for key in VoiceKey::ALL {
        assert!(key.fits(&base.voice.0[&key]), "{key:?}");
    }
}

#[test]
fn control_radius_is_two_thirds_of_the_card_radius() {
    let mut shape = whimbrel_default().shape;
    assert_eq!(shape.control_radius(), 8.0);
    shape.radius = 4.0;
    assert_eq!(shape.control_radius(), 3.0);
    shape.radius = 0.0;
    assert_eq!(shape.control_radius(), 0.0);
    shape.radius = 24.0;
    assert_eq!(shape.control_radius(), 16.0);
}

#[test]
fn a_skin_round_trips_through_serde() {
    let base = whimbrel_default();
    let text = serde_json::to_string(&base).unwrap();
    assert_eq!(serde_json::from_str::<skins::Skin>(&text).unwrap(), base);
    // The serialised form is itself a manifest the loader accepts untouched.
    let (again, repairs) = parse_skin(text.as_bytes(), &base).unwrap();
    assert!(repairs.is_empty());
    assert_eq!(again, base);
}
