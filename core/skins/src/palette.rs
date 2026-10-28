//! Colour palettes (`themes/*.json`, `skins/palettes/*.json`). Ports `ThemeToken`, `Theme` and
//! `ThemeLoader` (Theme.swift) and `ThemeDefaults`.

use crate::contrast::{self, Rgb};
use crate::format::is_valid_id;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// The colour tokens a palette sets (the JSON key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Token {
    BackgroundPage,
    BackgroundCard,
    Border,
    TextPrimary,
    TextSecondary,
    AccentPrimary,
    AccentSecondary,
    AccentHighlight,
    Error,
    Warning,
    Success,
    Link,
    Selection,
    Highlight,
    TextOnAccent,
}

impl Token {
    pub const ALL: [Token; 15] = [
        Token::BackgroundPage,
        Token::BackgroundCard,
        Token::Border,
        Token::TextPrimary,
        Token::TextSecondary,
        Token::AccentPrimary,
        Token::AccentSecondary,
        Token::AccentHighlight,
        Token::Error,
        Token::Warning,
        Token::Success,
        Token::Link,
        Token::Selection,
        Token::Highlight,
        Token::TextOnAccent,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Token::BackgroundPage => "backgroundPage",
            Token::BackgroundCard => "backgroundCard",
            Token::Border => "border",
            Token::TextPrimary => "textPrimary",
            Token::TextSecondary => "textSecondary",
            Token::AccentPrimary => "accentPrimary",
            Token::AccentSecondary => "accentSecondary",
            Token::AccentHighlight => "accentHighlight",
            Token::Error => "error",
            Token::Warning => "warning",
            Token::Success => "success",
            Token::Link => "link",
            Token::Selection => "selection",
            Token::Highlight => "highlight",
            Token::TextOnAccent => "textOnAccent",
        }
    }
}

/// `#RRGGBB` (upper case, as the palette files spell it).
pub fn hex_string(c: Rgb) -> String {
    format!("#{c:06X}")
}

/// One appearance's colours. A parsed palette holds all 15 tokens (missing ones filled from Default).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Variant(pub BTreeMap<Token, Rgb>);

impl Variant {
    pub fn get(&self, token: Token) -> Option<Rgb> {
        self.0.get(&token).copied()
    }

    pub fn set(&mut self, token: Token, colour: Rgb) {
        self.0.insert(token, colour);
    }

    fn from_list(list: [(Token, Rgb); 15]) -> Variant {
        Variant(list.into_iter().collect())
    }
}

impl Serialize for Variant {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (token, colour) in &self.0 {
            map.serialize_entry(token.name(), &hex_string(*colour))?;
        }
        map.end()
    }
}

/// Which variants a palette file ships. A one-variant palette serves both appearances with it, so
/// the app forces the matching appearance while it is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variants {
    Both,
    LightOnly,
    DarkOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub id: String,
    pub name: String,
    pub author: String,
    /// Both variants always hold all 15 tokens.
    pub light: Variant,
    pub dark: Variant,
    pub variants: Variants,
}

impl Palette {
    /// The palette in its file shape: only the variants it ships.
    pub fn to_json(&self) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("id".into(), self.id.clone().into());
        map.insert("name".into(), self.name.clone().into());
        map.insert("author".into(), self.author.clone().into());
        if self.variants != Variants::DarkOnly {
            map.insert(
                "light".into(),
                serde_json::to_value(&self.light).expect("variant serialises"),
            );
        }
        if self.variants != Variants::LightOnly {
            map.insert(
                "dark".into(),
                serde_json::to_value(&self.dark).expect("variant serialises"),
            );
        }
        Value::Object(map)
    }
}

/// The Default palette in code, so there is always a complete one even if `themes/default.json` is
/// missing. A test pins this and the file to the same values.
pub fn default_palette() -> Palette {
    use Token::*;
    Palette {
        id: "default".into(),
        name: "Default".into(),
        author: "Whimbrel".into(),
        light: Variant::from_list([
            (BackgroundPage, 0xF7F2E7),
            (BackgroundCard, 0xFFFFFF),
            (Border, 0xE4D9C2),
            (TextPrimary, 0x241E19),
            (TextSecondary, 0x6B5F4F),
            (AccentPrimary, 0xA8502E),
            (AccentSecondary, 0x55606E),
            (AccentHighlight, 0xD9A441),
            (Error, 0xB3261E),
            (Warning, 0x9A5B00),
            (Success, 0x3B7A3B),
            (Link, 0x2E5A87),
            (Selection, 0xEAE1CD),
            (Highlight, 0xF6E8D0),
            (TextOnAccent, 0xFFFFFF),
        ]),
        dark: Variant::from_list([
            (BackgroundPage, 0x1C1712),
            (BackgroundCard, 0x262019),
            (Border, 0x3A322B),
            (TextPrimary, 0xEDE6D9),
            (TextSecondary, 0xB7AB9A),
            (AccentPrimary, 0xC97B54),
            (AccentSecondary, 0x7C8794),
            (AccentHighlight, 0xE8B968),
            (Error, 0xF2877E),
            (Warning, 0xE0A04A),
            (Success, 0x7CC47C),
            (Link, 0x8FB4DC),
            (Selection, 0x312A24),
            (Highlight, 0x57462D),
            // White is 3.26:1 on the accent, below the 4.5:1 guard; near-black is 5.46:1.
            (TextOnAccent, 0x1C1712),
        ]),
        variants: Variants::Both,
    }
}

/// `#rrggbb` only.
fn hex(value: &Value) -> Option<Rgb> {
    let s = value.as_str()?;
    if s.chars().count() != 7 {
        return None;
    }
    u32::from_str_radix(s.strip_prefix('#')?, 16).ok()
}

/// `None` when any known token's value is not `#rrggbb`; unknown keys are ignored. Not contrast-checked.
fn variant(raw: &serde_json::Map<String, Value>) -> Option<Variant> {
    let mut v = Variant::default();
    for token in Token::ALL {
        if let Some(value) = raw.get(token.name()) {
            v.set(token, hex(value)?);
        }
    }
    Some(v)
}

fn filled(over: &Variant, base: &Variant) -> Variant {
    let mut v = base.clone();
    for (&token, &colour) in &over.0 {
        v.set(token, colour);
    }
    v
}

/// A palette file to a complete `Palette`, or `None` when the file is unusable. A variant that fails
/// the contrast guard is replaced by `fallback`'s variant of the same kind.
pub fn parse_palette(bytes: &[u8], fallback: &Palette) -> Option<Palette> {
    let Ok(Value::Object(root)) = serde_json::from_slice::<Value>(bytes) else {
        return None;
    };
    let id = root
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| is_valid_id(s))?;
    let name = root
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let light_raw = root.get("light").and_then(Value::as_object);
    let dark_raw = root.get("dark").and_then(Value::as_object);
    if light_raw.is_none() && dark_raw.is_none() {
        return None;
    }
    #[allow(clippy::manual_map)] // `?` inside the match arm needs the match
    let light = match light_raw {
        Some(r) => Some(filled(&variant(r)?, &fallback.light)),
        None => None,
    };
    #[allow(clippy::manual_map)]
    let dark = match dark_raw {
        Some(r) => Some(filled(&variant(r)?, &fallback.dark)),
        None => None,
    };
    // A palette that ships one variant serves both appearances.
    let mut light_palette = light.clone().or_else(|| dark.clone())?;
    let mut dark_palette = dark.or(light)?;
    if !contrast::passes(&light_palette) {
        light_palette = fallback.light.clone();
    }
    if !contrast::passes(&dark_palette) {
        dark_palette = fallback.dark.clone();
    }
    let variants = match (light_raw.is_some(), dark_raw.is_some()) {
        (false, _) => Variants::DarkOnly,
        (_, false) => Variants::LightOnly,
        _ => Variants::Both,
    };
    Some(Palette {
        id: id.to_string(),
        name: name.to_string(),
        author: root
            .get("author")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        light: light_palette,
        dark: dark_palette,
        variants,
    })
}

/// Every `*.json` directly inside `dir` (not recursive): the fallback first, then by name. Broken
/// files and duplicate ids are skipped; the fallback's id is never replaced.
pub fn load_palettes(dir: &Path, fallback: &Palette) -> Vec<Palette> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"));
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let mut seen = std::collections::BTreeSet::from([fallback.id.clone()]);
    let mut loaded = Vec::new();
    for file in files {
        let Some(p) = std::fs::read(&file)
            .ok()
            .and_then(|b| parse_palette(&b, fallback))
        else {
            continue;
        };
        if seen.insert(p.id.clone()) {
            loaded.push(p);
        }
    }
    loaded.sort_by(|a, b| a.name.cmp(&b.name));
    let mut all = vec![fallback.clone()];
    all.extend(loaded);
    all
}
