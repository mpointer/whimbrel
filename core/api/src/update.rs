//! Update manifest decoding, version comparison and download verification (M3.0
//! part 2, Task 4), moved from the pure half of apps/macos `UpdateChecker.swift`.
//! The 24 h timer, the download itself, the unzip, the code-signature check and the
//! relaunch helper stay in the shell: they are platform-bound.

use std::io::Read;
use std::path::Path;

use sha2::Digest;

/// The version manifest the updates Worker serves. `sha256`, `commit`, `tag`,
/// `published_at` and `release_notes_url` are optional so a legacy two-field manifest
/// still decodes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub release_notes_url: Option<String>,
}

/// What the FFI returns: the decoded manifest plus whether it is newer than the
/// running version.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ManifestCheck {
    pub manifest: Manifest,
    pub newer: bool,
}

pub fn parse_manifest(json: &str) -> Result<Manifest, serde_json::Error> {
    serde_json::from_str(json)
}

/// Decodes `json` and compares it against `current_version`.
pub fn check_manifest(json: &str, current_version: &str) -> Result<ManifestCheck, serde_json::Error> {
    let manifest = parse_manifest(json)?;
    let newer = is_newer_version(&manifest.version, current_version);
    Ok(ManifestCheck { manifest, newer })
}

/// Simple three-component semver compare. Malformed input on either side is `false`:
/// a manifest fetch gone wrong must never trigger a phantom update.
pub fn is_newer_version(remote: &str, local: &str) -> bool {
    match (parse_semver(remote), parse_semver(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

fn parse_semver(version: &str) -> Option<[u64; 3]> {
    let parts: Vec<u64> = version.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    parts.try_into().ok()
}

/// `expected == None` means a legacy manifest with no `sha256`: skip the check (an
/// explicitly-defined legacy behavior), not an error. Case-insensitive, since hex
/// digests are conventionally lowercase but a manifest author could write uppercase.
pub fn hash_matches(expected: Option<&str>, actual: &str) -> bool {
    match expected {
        None => true,
        Some(expected) => expected.eq_ignore_ascii_case(actual),
    }
}

/// Streamed in 1 MB chunks, not loaded whole: the update zip is ~260 MB.
pub fn sha256_hex(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// `true` if the file at `path` hashes to `expected` (or `expected` is `None`, the
/// legacy no-hash manifest). An unreadable file is a mismatch.
pub fn verify_sha256(path: &Path, expected: Option<&str>) -> bool {
    match expected {
        None => true,
        Some(expected) => sha256_hex(path).map(|actual| hash_matches(Some(expected), &actual)).unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- is_newer_version (ported from UpdateCheckerTests.swift) ----

    #[test]
    fn newer_patch_minor_and_major_versions_are_newer() {
        assert!(is_newer_version("0.2.1", "0.2.0"));
        assert!(is_newer_version("0.3.0", "0.2.9"));
        assert!(is_newer_version("1.0.0", "0.9.9"));
    }

    #[test]
    fn equal_and_older_versions_are_not_newer() {
        assert!(!is_newer_version("0.2.0", "0.2.0"));
        assert!(!is_newer_version("0.1.0", "0.2.0"));
    }

    #[test]
    fn malformed_remote_or_local_version_is_never_newer() {
        // A malformed manifest must never trigger a phantom update.
        assert!(!is_newer_version("not-a-version", "0.2.0"));
        assert!(!is_newer_version("0.3.0", "also-not-a-version"));
        assert!(!is_newer_version("1.0", "0.9.9"));
        assert!(!is_newer_version("1.0.0.0", "0.9.9"));
    }

    #[test]
    fn version_compare_is_numeric_not_lexical() {
        assert!(is_newer_version("0.10.0", "0.9.0"));
    }

    // ---- manifest decoding ----

    #[test]
    fn decodes_full_manifest_with_all_seven_fields() {
        let m = parse_manifest(r#"{"version": "0.3.12", "url": "https://example.com/Whimbrel-0.3.12.zip", "sha256": "abc123", "commit": "a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3", "tag": "v0.3.12", "published_at": "2026-08-18T14:32:00Z", "release_notes_url": "https://github.com/mpointer/kestrel/releases/tag/v0.3.12"}"#).unwrap();
        assert_eq!(m.version, "0.3.12");
        assert_eq!(m.sha256.as_deref(), Some("abc123"));
        assert_eq!(m.commit.as_deref(), Some("a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3"));
        assert_eq!(m.tag.as_deref(), Some("v0.3.12"));
        assert_eq!(m.published_at.as_deref(), Some("2026-08-18T14:32:00Z"));
        assert_eq!(m.release_notes_url.as_deref(), Some("https://github.com/mpointer/kestrel/releases/tag/v0.3.12"));
    }

    #[test]
    fn decodes_legacy_two_field_manifest_with_none_new_fields() {
        let m = parse_manifest(r#"{"version": "0.3.11", "url": "https://example.com/Whimbrel-0.3.11.zip"}"#).unwrap();
        assert_eq!(m.version, "0.3.11");
        assert_eq!(m.url, "https://example.com/Whimbrel-0.3.11.zip");
        assert!(m.sha256.is_none() && m.commit.is_none() && m.tag.is_none() && m.published_at.is_none() && m.release_notes_url.is_none());
    }

    #[test]
    fn missing_required_version_or_url_fails_to_decode() {
        assert!(parse_manifest(r#"{"url": "https://example.com/Whimbrel.zip", "sha256": "abc123"}"#).is_err());
        assert!(parse_manifest(r#"{"version": "0.3.12", "sha256": "abc123"}"#).is_err());
        assert!(parse_manifest("not json").is_err());
    }

    #[test]
    fn malformed_optional_fields_still_decode_as_present_strings() {
        // Shape validity (hex-ness of a hash) isn't schema-enforced; hash_matches is
        // what rejects a malformed sha256 at the point it matters.
        let m = parse_manifest(r#"{"version": "0.3.12", "url": "https://example.com/Whimbrel-0.3.12.zip", "sha256": "not-valid-hex!!", "commit": "too-short"}"#).unwrap();
        assert_eq!(m.sha256.as_deref(), Some("not-valid-hex!!"));
        assert_eq!(m.commit.as_deref(), Some("too-short"));
    }

    #[test]
    fn check_manifest_reports_newer_against_the_running_version() {
        let json = r#"{"version": "0.3.12", "url": "https://example.com/x.zip"}"#;
        assert!(check_manifest(json, "0.3.11").unwrap().newer);
        assert!(!check_manifest(json, "0.3.12").unwrap().newer);
        assert!(!check_manifest(json, "garbage").unwrap().newer);
        let encoded = serde_json::to_value(check_manifest(json, "0.3.11").unwrap()).unwrap();
        assert_eq!(encoded["manifest"]["version"], "0.3.12");
        assert_eq!(encoded["manifest"]["sha256"], serde_json::Value::Null);
        assert_eq!(encoded["newer"], true);
    }

    // ---- hash_matches ----

    #[test]
    fn hash_matches_when_expected_is_none_legacy_manifest() {
        assert!(hash_matches(None, "anyvalue"));
    }

    #[test]
    fn hash_matches_equal_hashes_case_insensitively_and_rejects_mismatch() {
        let hash = "a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4f5061728394a5b6c7d8e9f";
        assert!(hash_matches(Some(hash), hash));
        assert!(hash_matches(Some("ABCDEF"), "abcdef"));
        assert!(!hash_matches(Some("abc123"), "def456"));
    }

    // ---- sha256_hex / verify_sha256 ----

    #[test]
    fn sha256_hex_of_known_fixture_matches_known_digest() {
        // "hello world\n": `echo "hello world" | shasum -a 256`.
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture.txt");
        std::fs::write(&fixture, "hello world\n").unwrap();
        assert_eq!(sha256_hex(&fixture).unwrap(), "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447");
        assert!(verify_sha256(&fixture, Some("A948904F2F0F479B8F8197694B30184B0D2ED1C1CD2A1EC0FB85D299A192A447")));
        assert!(verify_sha256(&fixture, None), "legacy manifest without a hash passes");
    }

    #[test]
    fn sha256_hex_of_one_byte_mutated_fixture_differs_and_verify_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original.txt");
        let mutated = dir.path().join("mutated.txt");
        std::fs::write(&original, "hello world\n").unwrap();
        std::fs::write(&mutated, "hello worle\n").unwrap();
        let original_hash = sha256_hex(&original).unwrap();
        assert_ne!(original_hash, sha256_hex(&mutated).unwrap());
        assert!(!verify_sha256(&mutated, Some(&original_hash)));
    }

    #[test]
    fn verify_sha256_of_a_missing_file_is_a_mismatch() {
        assert!(!verify_sha256(Path::new("/nonexistent/whimbrel.zip"), Some("abc")));
        assert!(sha256_hex(Path::new("/nonexistent/whimbrel.zip")).is_err());
    }
}
