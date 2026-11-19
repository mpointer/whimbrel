//! Test-only helper for tests that scan a module's own source (`include_str!`) for what its
//! production part may or may not contain.

/// The part of a module's source before its `#[cfg(test)] mod tests`, with line endings
/// normalised first: a Windows checkout has CRLF, and an LF-only marker would never match,
/// so the scan would read the test module (and its own list of banned words) too. Panics
/// when the marker is missing, so a scan never silently covers the whole file.
pub(crate) fn production_part(src: &str) -> String {
    let src = src.replace("\r\n", "\n");
    let (production, _) = src
        .split_once("#[cfg(test)]\nmod tests")
        .expect("source has no `#[cfg(test)]\\nmod tests` marker");
    production.to_string()
}

#[cfg(test)]
mod tests {
    use super::production_part;

    #[test]
    fn crlf_source_gives_the_same_production_part_as_lf() {
        for src in [include_str!("speed_check.rs"), include_str!("encrypt_migration.rs")] {
            let lf = src.replace("\r\n", "\n");
            let crlf = lf.replace('\n', "\r\n");
            let want = production_part(&lf);
            assert_eq!(production_part(&crlf), want);
            assert!(want.len() < lf.len(), "the test module is left out");
            assert!(!want.contains("mod tests"), "the test module is left out");
        }
    }

    #[test]
    #[should_panic(expected = "marker")]
    fn missing_marker_panics() {
        production_part("pub fn f() {}\n");
    }
}
