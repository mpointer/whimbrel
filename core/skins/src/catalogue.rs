//! The allowed values for every skin axis, sent to the skin maker with each turn so the model picks
//! from the same lists the loaders accept. Built from the same constants as the loaders, so it cannot
//! drift from them. Ports `SkinCatalogue.swift`, except that icons are the shared table's symbols per
//! role.

use crate::contrast::{MIN_RATIO, REQUIRED_PAIRS};
use crate::format::*;
use crate::icons::icon_table;
use crate::palette::Token;
use serde::Serialize;
use serde_json::{json, Map, Value};

/// The manifest names of a list of enum values.
fn names<E: Serialize>(values: &[E]) -> Vec<Value> {
    values
        .iter()
        .map(|v| serde_json::to_value(v).expect("an enum serialises"))
        .collect()
}

pub fn catalogue() -> Value {
    let mut weights: Vec<&str> = Weight::ALL.iter().map(|w| w.name()).collect();
    weights.sort_unstable();
    let mut icon_values = Map::new();
    for role in Role::ALL {
        let symbols: Vec<&str> = icon_table()
            .iter()
            .filter(|e| e.role == role)
            .map(|e| e.symbol.as_str())
            .collect();
        icon_values.insert(role.name().into(), json!(symbols));
    }
    let mut voice_keys = Map::new();
    for key in VoiceKey::ALL {
        voice_keys.insert(key.name().into(), json!({ "limit": key.limit() }));
    }
    let pairs: Vec<String> = REQUIRED_PAIRS
        .iter()
        .map(|(fg, bg)| format!("{} on {}", fg.name(), bg.name()))
        .collect();
    json!({
        "manifest": {
            "shape": r#"{"id","name","palettes":[palette ids],"typography":{"display","body","numerals","label","caps"},"shape":{"radius","border","borderWidth","density"},"icons":{role: SF Symbol name},"decoration":{"headerBand","dividers","texture","imagery":[{"asset","placement","textFree"}]},"voice":{key: text},"today":{"archetype","options":{}},"motion"}"#,
            "imagery": "only images already in the current manifest; new images are added by the user approving them",
        },
        "fonts": {
            "families": Family::ALL.iter().map(|f| f.name()).collect::<Vec<_>>(),
            "weights": weights,
            "format": r#""Family", "Family/weight" or "Family/weight/italic""#,
        },
        "typography": { "caps": names(&Caps::ALL) },
        "shape": {
            "radius": { "min": RADIUS_RANGE.0, "max": RADIUS_RANGE.1 },
            "borderWidth": { "min": WIDTH_RANGE.0, "max": WIDTH_RANGE.1 },
            "border": names(&Border::ALL),
            "density": names(&Density::ALL),
        },
        "icons": {
            "roles": Role::ALL.iter().map(|r| r.name()).collect::<Vec<_>>(),
            "values": icon_values,
            "rule": "a name must be one of these; any role may use any listed name",
        },
        "decoration": {
            "headerBand": names(&HeaderBand::ALL),
            "dividers": names(&Divider::ALL),
            "texture": names(&Texture::ALL),
            "imageryPlacement": names(&Placement::ALL),
        },
        "voice": {
            "keys": voice_keys,
            "name": format!("greetings may use the literal {{name}}; the limit counts it as {MEASURED_NAME_LENGTH} characters"),
        },
        "archetypes": names(&Archetype::ALL),
        "motion": names(&Motion::ALL),
        "palette": {
            "shape": r##"{"id","name","light":{token:"#rrggbb"},"dark":{token:"#rrggbb"}}; light, dark or both"##,
            "tokens": Token::ALL.iter().map(|t| t.name()).collect::<Vec<_>>(),
            "colour": "#rrggbb only",
            "contrast": { "minimum": MIN_RATIO, "pairs": pairs },
        },
    })
}
