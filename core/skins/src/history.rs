//! The undo history of a user skin: `history/<n>.json`, each a saved version of the manifest and
//! palettes. The format is the Mac's `Codable` `Version`, so either app reads what the other wrote:
//! `{"manifest": <base64>, "palettes": [{"id": "...", "data": <base64>}]}`.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::io;
use std::path::Path;

fn to_b64<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&STANDARD.encode(bytes))
}

fn from_b64<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let text = String::deserialize(d)?;
    STANDARD
        .decode(text.as_bytes())
        .map_err(serde::de::Error::custom)
}

/// One palette file as it was saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionPalette {
    pub id: String,
    #[serde(serialize_with = "to_b64", deserialize_with = "from_b64")]
    pub data: Vec<u8>,
}

/// One saved version: the manifest and palette bytes exactly as written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    #[serde(serialize_with = "to_b64", deserialize_with = "from_b64")]
    pub manifest: Vec<u8>,
    pub palettes: Vec<VersionPalette>,
}

/// The version numbers present in `dir` (`<n>.json`), ascending.
pub fn numbers(dir: &Path) -> Vec<u32> {
    let mut found: Vec<u32> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .strip_suffix(".json")
                        .and_then(|n| n.parse().ok())
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_unstable();
    found
}

pub fn read(dir: &Path, n: u32) -> io::Result<Version> {
    let bytes = std::fs::read(dir.join(format!("{n}.json")))?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write(dir: &Path, n: u32, version: &Version) -> io::Result<()> {
    let bytes =
        serde_json::to_vec(version).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    crate::store::write_atomic(&dir.join(format!("{n}.json")), &bytes)
}
