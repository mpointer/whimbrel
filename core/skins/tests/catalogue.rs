//! The catalogue sent to the model. The fixture was derived by hand from the Mac's `SkinCatalogue.swift`
//! (with the icon table's symbols per role) and not from this crate, so a drift in either shows here.

use serde_json::Value;
use skins::{catalogue, icon_table, is_allowed_icon, whimbrel_default, parse_skin};

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

#[test]
fn catalogue_matches_mac_shape_except_icons() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/catalogue.json");
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let got = catalogue();
    let (mut a, mut b) = (got.clone(), fixture.clone());
    // Icons are the one deliberate difference from the Mac, and have their own comparison below.
    a["icons"] = Value::Null;
    b["icons"] = Value::Null;
    assert!(
        same_json(&a, &b),
        "catalogue differs from the Mac-derived fixture\n got      {a}\n expected {b}"
    );
    assert!(
        same_json(&got["icons"], &fixture["icons"]),
        "icons differ\n got      {}\n expected {}",
        got["icons"],
        fixture["icons"]
    );
}

#[test]
fn the_catalogue_offers_only_table_icons_for_every_role() {
    let c = catalogue();
    let roles: Vec<&str> = c["icons"]["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(roles.len(), 9);
    let mut offered = 0;
    for role in roles {
        let symbols = c["icons"]["values"][role].as_array().unwrap();
        assert!(symbols.len() >= 7, "{role}");
        for s in symbols {
            assert!(is_allowed_icon(s.as_str().unwrap()), "{s}");
            offered += 1;
        }
    }
    assert_eq!(
        offered,
        icon_table().len(),
        "every table symbol is offered once"
    );
}

fn loads_clean(manifest: &Value) -> skins::Skin {
    let (skin, repairs) =
        parse_skin(&serde_json::to_vec(manifest).unwrap(), &whimbrel_default()).unwrap();
    assert!(repairs.is_empty(), "{repairs:?}");
    skin
}

#[test]
fn everything_the_catalogue_offers_is_accepted_by_the_loader() {
    let c = catalogue();
    // The last offered value of every axis (the extremes), in one manifest.
    let mut icons = serde_json::Map::new();
    for role in c["icons"]["roles"].as_array().unwrap() {
        let name = role.as_str().unwrap();
        let offered = c["icons"]["values"][name].as_array().unwrap();
        icons.insert(name.into(), offered.last().unwrap().clone());
    }
    let last = |v: &Value| v.as_array().unwrap().last().unwrap().clone();
    let manifest = serde_json::json!({
        "id": "probe", "name": "Probe", "palettes": ["default"],
        "typography": {
            "display": "Avenir Next/ultraLight/italic",
            "body": last(&c["fonts"]["families"]), "numerals": c["fonts"]["families"][1],
            "label": c["fonts"]["families"][2], "caps": last(&c["typography"]["caps"]),
        },
        "shape": {
            "radius": c["shape"]["radius"]["max"], "borderWidth": c["shape"]["borderWidth"]["max"],
            "border": last(&c["shape"]["border"]), "density": last(&c["shape"]["density"]),
        },
        "icons": icons,
        "decoration": {
            "headerBand": last(&c["decoration"]["headerBand"]), "dividers": last(&c["decoration"]["dividers"]),
            "texture": last(&c["decoration"]["texture"]), "imagery": [],
        },
        "today": { "archetype": last(&c["archetypes"]), "options": {} },
        "motion": last(&c["motion"]),
    });
    let skin = loads_clean(&manifest);
    assert_eq!(skin.shape.radius, 24.0);
    assert_eq!(skin.shape.border_width, 4.0);
    // Every family at every weight is a face the loader keeps.
    for f in c["fonts"]["families"].as_array().unwrap() {
        for w in c["fonts"]["weights"].as_array().unwrap() {
            let face = format!("{}/{}", f.as_str().unwrap(), w.as_str().unwrap());
            let m = serde_json::json!({"id": "f", "name": "F", "palettes": ["default"], "typography": {"body": face}});
            loads_clean(&m);
        }
    }
    // Every offered enum value is kept, one axis at a time.
    for (axis, key, values) in [
        ("typography", "caps", &c["typography"]["caps"]),
        ("shape", "border", &c["shape"]["border"]),
        ("shape", "density", &c["shape"]["density"]),
        ("decoration", "headerBand", &c["decoration"]["headerBand"]),
        ("decoration", "dividers", &c["decoration"]["dividers"]),
        ("decoration", "texture", &c["decoration"]["texture"]),
    ] {
        for v in values.as_array().unwrap() {
            loads_clean(
                &serde_json::json!({"id": "e", "name": "E", "palettes": ["default"], axis: {key: v}}),
            );
        }
    }
    for key in ["archetypes", "motion"] {
        for v in c[key].as_array().unwrap() {
            let m = if key == "motion" {
                serde_json::json!({"id": "e", "name": "E", "palettes": ["default"], "motion": v})
            } else {
                serde_json::json!({"id": "e", "name": "E", "palettes": ["default"], "today": {"archetype": v}})
            };
            loads_clean(&m);
        }
    }
}

#[test]
fn the_voice_limits_offered_are_the_ones_enforced() {
    let c = catalogue();
    for (key, entry) in c["voice"]["keys"].as_object().unwrap() {
        let limit = entry["limit"].as_u64().unwrap() as usize;
        // A greeting is measured with {name} as 30 characters; the rest are plain text.
        let (fits, over) = if key.starts_with("greeting") {
            (
                format!("{}{{name}}", "a".repeat(limit - 30)),
                format!("{}{{name}}", "a".repeat(limit - 29)),
            )
        } else {
            ("a".repeat(limit), "a".repeat(limit + 1))
        };
        for (text, ok) in [(fits, true), (over, false)] {
            let m = serde_json::json!({"id": "v", "name": "V", "palettes": ["default"], "voice": {key: text}});
            let (_, repairs) =
                parse_skin(&serde_json::to_vec(&m).unwrap(), &whimbrel_default()).unwrap();
            assert_eq!(repairs.is_empty(), ok, "{key}");
        }
    }
}
