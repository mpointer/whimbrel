//! A debug switch for release checks: when `WHIMBREL_HOSTED_REQUEST_LOG` names a file, the URL
//! and JSON body of every model, image and image-search request are appended to it, one JSON
//! line per request. It shows exactly what left the machine (the client-controls gate reads it to
//! prove an excluded client's name never reaches a hosted model). Headers, where the keys and
//! tokens live, are never written, and the URLs passed in carry no key. Off unless the variable is
//! set; the file holds prompt text, so it is created readable by its owner only.

use std::path::{Path, PathBuf};

pub const ENV_VAR: &str = "WHIMBREL_HOSTED_REQUEST_LOG";

fn target() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = tests::OVERRIDE.with(|p| p.borrow().clone()) {
        return Some(path);
    }
    static PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var_os(ENV_VAR).filter(|v| !v.is_empty()).map(PathBuf::from))
        .clone()
}

/// Appends one request to the log when the switch is on. Never fails the request itself.
pub(crate) fn record<T: serde::Serialize + ?Sized>(provider: &str, url: &str, body: &T) {
    if let Some(path) = target() {
        if let Err(e) = append(&path, provider, url, body) {
            eprintln!("{ENV_VAR}: could not write {}: {e}", path.display());
        }
    }
}

fn append<T: serde::Serialize + ?Sized>(path: &Path, provider: &str, url: &str, body: &T) -> std::io::Result<()> {
    use std::io::Write;
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let body = serde_json::to_value(body).unwrap_or(serde_json::Value::Null);
    let line = serde_json::json!({ "at_unix_ms": at_unix_ms, "provider": provider, "url": url, "body": body });
    let _guard = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    writeln!(file, "{line}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        pub(crate) static OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Turns the switch on for the current thread, writing to a fresh file; returns its path.
    pub(crate) fn log_to_temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whimbrel-request-log-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("requests.jsonl");
        let _ = std::fs::remove_file(&path);
        OVERRIDE.with(|p| *p.borrow_mut() = Some(path.clone()));
        path
    }

    pub(crate) fn read_lines(path: &Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn record_appends_one_json_line_per_request() {
        let path = log_to_temp("append");
        record("anthropic", "https://api.example/v1/messages", &serde_json::json!({ "prompt": "O’Brien Partners" }));
        record("openai", "https://api.example/v1/chat/completions", &serde_json::json!({ "n": 2 }));
        let lines = read_lines(&path);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["provider"], "anthropic");
        assert_eq!(lines[0]["url"], "https://api.example/v1/messages");
        assert_eq!(lines[0]["body"]["prompt"], "O’Brien Partners");
        assert!(lines[0]["at_unix_ms"].as_u64().unwrap() > 0);
        assert_eq!(lines[1]["body"]["n"], 2);
        // Non-ASCII stays readable, so a plain text search for the name finds it.
        assert!(std::fs::read_to_string(&path).unwrap().contains("O’Brien Partners"));
    }

    #[test]
    fn record_without_the_switch_writes_nothing() {
        OVERRIDE.with(|p| *p.borrow_mut() = None);
        // The real env var is never set in the test run, so this must be a no-op.
        assert!(std::env::var_os(ENV_VAR).is_none());
        assert!(target().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn the_log_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = log_to_temp("mode");
        record("google", "https://api.example/x", &serde_json::json!({}));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
