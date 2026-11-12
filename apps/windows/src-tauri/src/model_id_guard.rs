//! CI guard: no hosted model id literal in the Windows app's own code. Defaults come from
//! core (`api::model_defaults`, the updates server's file over core's table), so a pinned id
//! here would go stale the way `gemini-2.5-flash` did (2026-11-11). Like `colors.test` does
//! for hex colors. Tests are exempt (Rust `#[cfg(test)]` modules, `*.test.ts(x)`), as are
//! comments, generated files and the allowlist below.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// The hosted model id families the guard looks for.
    const PREFIXES: [&str; 3] = ["gpt-", "claude-", "gemini-"];

    /// Files allowed to name model ids, each with why.
    const ALLOWED_FILES: [(&str, &str); 1] = [
        // Whimbrel AI's built-in model list and default: our own server's catalogue, refreshed
        // from it (`whimbrel_ai::offered`), not a hosted provider's default.
        ("src-tauri/src/whimbrel_ai.rs", "Whimbrel AI's own catalogue"),
    ];

    /// Single ids allowed in one file.
    const ALLOWED_IDS: [(&str, &str); 1] = [
        // The UI's copy of `whimbrel_ai::DEFAULT_MODEL`, for before the server's list arrives.
        ("ui/src/settings/aiModel.ts", "claude-sonnet-5-5"),
    ];

    fn windows_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
    }

    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "generated" && name != "node_modules" {
                    files(&path, out);
                }
            } else if (name.ends_with(".rs") || name.ends_with(".ts") || name.ends_with(".tsx"))
                && !name.contains(".test.")
                && name != "model_id_guard.rs"
            {
                out.push(path);
            }
        }
    }

    /// The code a guard should read: a Rust file without its `#[cfg(test)]` items (a test
    /// module or a test-only fn, wherever it sits), with `//` comments (and doc comments)
    /// and `/* */` lines dropped. An item ends where its braces balance, or at a `;` line
    /// before any brace opens.
    fn code_lines(text: &str, rust: bool) -> Vec<(usize, String)> {
        let mut lines = Vec::new();
        let mut in_block = false;
        // Inside a test item: the brace depth so far, and whether one has opened.
        let mut test_item: Option<(i64, bool)> = None;
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if rust && trimmed.starts_with("#[cfg(test)]") {
                test_item = Some((0, false));
                continue;
            }
            if let Some((depth, opened)) = test_item.as_mut() {
                let code = line.split("//").next().unwrap_or("");
                *depth += code.matches('{').count() as i64 - code.matches('}').count() as i64;
                *opened |= code.contains('{');
                if (*opened && *depth <= 0) || (!*opened && code.trim_end().ends_with(';')) {
                    test_item = None;
                }
                continue;
            }
            if in_block {
                in_block = !line.contains("*/");
                continue;
            }
            if trimmed.starts_with("/*") {
                in_block = !line.contains("*/");
                continue;
            }
            if trimmed.starts_with('*') {
                continue;
            }
            let code = match line.find("//") {
                // Keep a URL's `https://` inside a string; a model id never follows one.
                Some(at) if !line[..at].ends_with(':') => &line[..at],
                _ => line,
            };
            lines.push((i + 1, code.to_string()));
        }
        lines
    }

    /// Every model id in `line`: a family prefix followed by a digit or letter.
    fn ids(line: &str) -> Vec<String> {
        let mut found = Vec::new();
        for prefix in PREFIXES {
            let mut from = 0;
            while let Some(at) = line[from..].find(prefix) {
                let start = from + at;
                let before_ok = start == 0 || !matches!(line.as_bytes()[start - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'/');
                let rest = &line[start + prefix.len()..];
                let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_'))).unwrap_or(rest.len());
                if before_ok && rest.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
                    found.push(format!("{prefix}{}", &rest[..end]));
                }
                from = start + prefix.len();
            }
        }
        found
    }

    #[test]
    fn the_windows_app_names_no_hosted_model_id_outside_tests() {
        let root = windows_root();
        let mut paths = Vec::new();
        files(&root.join("src-tauri").join("src"), &mut paths);
        files(&root.join("ui").join("src"), &mut paths);
        assert!(paths.len() > 50, "the guard found too few files to be reading the right tree");
        let mut hits = Vec::new();
        for path in paths {
            let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            if ALLOWED_FILES.iter().any(|(f, _)| *f == rel) {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in code_lines(&text, rel.ends_with(".rs")) {
                for id in ids(&line) {
                    if !ALLOWED_IDS.iter().any(|(f, allowed)| *f == rel && *allowed == id) {
                        hits.push(format!("{rel}:{n}: {id}"));
                    }
                }
            }
        }
        assert!(
            hits.is_empty(),
            "hosted model ids belong in core (api::model_defaults), not the Windows app:\n{}",
            hits.join("\n")
        );
    }

    #[test]
    fn the_guard_finds_ids_and_skips_tests_and_comments() {
        assert_eq!(ids(r#"let m = "gpt-5.4-mini"; // or gemini-x"#), ["gpt-5.4-mini", "gemini-x"]);
        assert_eq!(ids("claude-haiku-5-5"), ["claude-haiku-5-5"]);
        assert!(ids("my-gpt-thing, claude-, gemini-").is_empty(), "not an id");
        let rust = "fn a() {}\n// gpt-4o-mini\nlet x = \"claude-x\";\n#[cfg(test)]\nfn helper() {\n  \"gpt-test\"\n}\nlet y = \"gemini-after\";\n#[cfg(test)]\nuse x::y;\nlet z = \"gpt-z\";\n#[cfg(test)]\nmod tests { \"gemini-2\" }";
        let lines = code_lines(rust, true);
        let found: Vec<String> = lines.iter().flat_map(|(_, l)| ids(l)).collect();
        assert_eq!(found, ["claude-x", "gemini-after", "gpt-z"], "code between and after test items is read");
        let ts = "/**\n * gpt-4o\n */\nconst u = \"https://x\"; const m = \"gemini-y\";";
        let found: Vec<String> = code_lines(ts, false).iter().flat_map(|(_, l)| ids(l)).collect();
        assert_eq!(found, ["gemini-y"]);
    }
}
