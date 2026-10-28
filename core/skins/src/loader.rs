//! Parses `skins/<id>.json` manifests. Ports `SkinLoader.swift`. A file is skipped when it is not JSON
//! or lacks a valid id and a name; any field-level problem is repaired from `base` and reported.

use crate::format::*;
use crate::icons::is_allowed_icon;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

/// An enum value by its manifest string; `fallback` (reported) when present but unknown.
fn pick<E: DeserializeOwned + Serialize>(
    raw: Option<&Map<String, Value>>,
    key: &str,
    fallback: E,
    repairs: &mut Vec<String>,
) -> E {
    let Some(value) = raw.and_then(|m| m.get(key)) else {
        return fallback;
    };
    if let Some(e) = value
        .as_str()
        .and_then(|s| serde_json::from_value::<E>(Value::String(s.into())).ok())
    {
        return e;
    }
    let name = serde_json::to_value(&fallback)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    repairs.push(format!("{key} is not a known value, using {name}"));
    fallback
}

/// A number clamped into `range`. Messages print floats as Swift does ("99.0", "0.0...24.0").
fn number(
    raw: Option<&Map<String, Value>>,
    key: &str,
    fallback: f64,
    range: (f64, f64),
    repairs: &mut Vec<String>,
) -> f64 {
    let Some(value) = raw.and_then(|m| m.get(key)) else {
        return fallback;
    };
    let Some(d) = value.as_f64() else {
        repairs.push(format!("{key} is not a number, using {fallback:?}"));
        return fallback;
    };
    let clamped = d.max(range.0).min(range.1);
    if clamped != d {
        repairs.push(format!(
            "{key} {d:?} is outside {:?}...{:?}, clamped to {clamped:?}",
            range.0, range.1
        ));
    }
    clamped
}

fn face(
    raw: Option<&Map<String, Value>>,
    key: &str,
    fallback: Face,
    repairs: &mut Vec<String>,
) -> Face {
    let Some(value) = raw.and_then(|m| m.get(key)) else {
        return fallback;
    };
    if let Some(f) = value.as_str().and_then(Face::parse) {
        return f;
    }
    repairs.push(format!("{key} font is not allowed, using the Whimbrel font"));
    fallback
}

/// `None` skips the file. Otherwise the repaired skin and the repair messages.
pub fn parse_skin(bytes: &[u8], base: &Skin) -> Option<(Skin, Vec<String>)> {
    let root = match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(m)) => m,
        _ => return None,
    };
    let id = root
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| is_valid_id(s))?;
    let name = root
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let mut repairs: Vec<String> = Vec::new();
    let obj = |key: &str| root.get(key).and_then(Value::as_object);

    let mut skin = base.clone();
    skin.id = id.to_string();
    skin.name = name.to_string();
    skin.author = root
        .get("author")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let palettes: Vec<String> = root
        .get("palettes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|s| is_valid_id(s))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    if palettes.is_empty() {
        repairs.push("no usable palettes, using the Whimbrel palette".into());
    } else {
        skin.palettes = palettes;
    }

    let typography = obj("typography");
    skin.typography = Typography {
        display: face(typography, "display", base.typography.display, &mut repairs),
        body: face(typography, "body", base.typography.body, &mut repairs),
        numerals: face(
            typography,
            "numerals",
            base.typography.numerals,
            &mut repairs,
        ),
        label: face(typography, "label", base.typography.label, &mut repairs),
        caps: pick(typography, "caps", base.typography.caps, &mut repairs),
    };

    let shape = obj("shape");
    skin.shape = Shape {
        radius: number(
            shape,
            "radius",
            base.shape.radius,
            RADIUS_RANGE,
            &mut repairs,
        ),
        border: pick(shape, "border", base.shape.border, &mut repairs),
        border_width: number(
            shape,
            "borderWidth",
            base.shape.border_width,
            WIDTH_RANGE,
            &mut repairs,
        ),
        density: pick(shape, "density", base.shape.density, &mut repairs),
    };

    let icons = obj("icons");
    for role in Role::ALL {
        let Some(value) = icons.and_then(|m| m.get(role.name())) else {
            continue;
        };
        match value.as_str() {
            Some(s) if is_allowed_icon(s) => {
                skin.icons.0.insert(role, s.to_string());
            }
            Some(s) => repairs.push(format!("icon \"{s}\" is not in the icon table")),
            None => repairs.push(format!(
                "icon {} is not a symbol name, using the Whimbrel icon",
                role.name()
            )),
        }
    }

    let decoration = obj("decoration");
    let mut imagery = Vec::new();
    for item in decoration
        .and_then(|m| m.get("imagery"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // The asset is a plain file name, never a path.
        let entry = item.as_object().and_then(|d| {
            let asset = d.get("asset")?.as_str()?;
            if asset.is_empty() || asset.contains('/') || asset.starts_with('.') {
                return None;
            }
            let placement =
                serde_json::from_value::<Placement>(d.get("placement")?.clone()).ok()?;
            let text_free = d.get("textFree").and_then(Value::as_bool).unwrap_or(false);
            Some(Image {
                asset: asset.to_string(),
                placement,
                text_free,
            })
        });
        match entry {
            Some(e) => imagery.push(e),
            None => repairs.push(
                "an imagery entry needs a plain file name asset and a placement, dropped".into(),
            ),
        }
    }
    skin.decoration = Decoration {
        header_band: pick(
            decoration,
            "headerBand",
            base.decoration.header_band,
            &mut repairs,
        ),
        dividers: pick(
            decoration,
            "dividers",
            base.decoration.dividers,
            &mut repairs,
        ),
        texture: pick(decoration, "texture", base.decoration.texture, &mut repairs),
        imagery,
    };

    let voice = obj("voice");
    for key in VoiceKey::ALL {
        let Some(value) = voice.and_then(|m| m.get(key.name())) else {
            continue;
        };
        match value.as_str() {
            Some(s) if key.fits(s) => {
                skin.voice.0.insert(key, s.to_string());
            }
            _ => repairs.push(format!(
                "voice {} is empty, not text or over {} characters, using the Whimbrel wording",
                key.name(),
                key.limit()
            )),
        }
    }

    let today = obj("today");
    skin.today.archetype = pick(today, "archetype", base.today.archetype, &mut repairs);
    skin.today.options = today
        .and_then(|m| m.get("options"))
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    skin.motion = pick(Some(&root), "motion", base.motion, &mut repairs);
    Some((skin, repairs))
}

/// Every `*.json` directly inside `dir` (not recursive, so `skins/invalid/` is never loaded):
/// the base first, then the rest by name. Broken files and duplicate ids are skipped; the first
/// file name wins a duplicate id.
pub fn load_bundled(dir: &Path) -> Vec<Skin> {
    let base = whimbrel_default();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"));
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let mut seen: BTreeSet<String> = BTreeSet::from([base.id.clone()]);
    let mut loaded = Vec::new();
    for file in files {
        let Some((skin, _)) = std::fs::read(&file)
            .ok()
            .and_then(|b| parse_skin(&b, &base))
        else {
            continue;
        };
        if skin.id == base.id {
            continue; // whimbrel.json mirrors the base
        }
        if seen.insert(skin.id.clone()) {
            loaded.push(skin);
        }
    }
    loaded.sort_by(|a, b| a.name.cmp(&b.name));
    let mut all = vec![base];
    all.extend(loaded);
    all
}
