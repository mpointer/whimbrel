//! The shared icon table, `skins/icons.json`. A skin names an icon by SF Symbol; the table says which
//! symbols are allowed (on both platforms) and which Segoe glyph Windows draws for each.

use crate::format::Role;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct IconEntry {
    pub symbol: String,
    /// The role the entry suits, for the catalogue offered to the model.
    pub role: Role,
    /// Segoe Fluent Icons code point.
    pub fluent: u32,
    /// Segoe MDL2 Assets code point, the fallback.
    pub mdl2: u32,
}

static TABLE: OnceLock<Vec<IconEntry>> = OnceLock::new();

pub fn icon_table() -> &'static [IconEntry] {
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("../../../skins/icons.json"))
            .expect("skins/icons.json is valid")
    })
}

pub fn is_allowed_icon(name: &str) -> bool {
    icon_table().iter().any(|e| e.symbol == name)
}
