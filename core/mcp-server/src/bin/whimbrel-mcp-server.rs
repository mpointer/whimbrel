//! Real binary entry point: runs `WhimbrelMcpServer` over streamable-HTTP on
//! `http://127.0.0.1:4319/mcp`, with Task 3's bearer-token auth middleware enforced on
//! every request. The router itself (the `StreamableHttpService` nested under `/mcp`,
//! wrapped by `AuthLayer`) is built by `mcp_server::build_router`, shared with
//! `tests/real_client_integration.rs` so the real test can't silently drift from the
//! real router this binary serves — see that function's doc comment in
//! `core/mcp-server/src/lib.rs` for why the auth layer wraps the *outer* `axum::Router`
//! rather than the raw `StreamableHttpService` (the latter's `tower_service::Service`
//! impl responds with a different body type than `AuthLayer` expects, so it can only
//! compose one layer up, after `.nest_service` has already folded it into a `Router`).
//! See `lib.rs`'s module doc for the tool/scope model and `auth.rs`'s module doc for
//! more on the auth middleware itself.
//!
//! ## The DB path
//!
//! The app passes the database it opened as `--db <path>` (`McpServerProcessManager`), so
//! the app's data folder is defined only in the app. Without the flag, `WHIMBREL_DB` is
//! used, then the Mac's default location, for running this binary by hand from a checkout.
//!
//! ## The real models directory
//!
//! `EmbeddingModelLocator.swift` resolves the embedding model directory as
//! `Bundle.main.resourceURL!.appendingPathComponent("models")` — i.e. `<Whimbrel.app>/
//! Contents/Resources/models` once the app is installed (confirmed live on this machine:
//! that path holds the bundled `all-MiniLM-L6-v2` embedding model files). This binary
//! isn't an app bundle itself, but it *ships inside* one: `apps/macos/project.yml`
//! copies the compiled `whimbrel-mcp-server` binary into `Contents/Resources` as a plain
//! file resource, as a sibling of the `models/` folder resource copied from
//! `core/memory/models` (confirmed live on this machine: `ls -la /Applications/
//! Whimbrel.app/Contents/Resources/` shows both `whimbrel-mcp-server` and `models/`
//! directly alongside each other). So rather than hardcoding the app's install path
//! (which breaks for a `~/Applications` install, a DMG-mounted run, or any layout other
//! than exactly `/Applications/Whimbrel.app`), this resolves the models dir relative to
//! `std::env::current_exe()` — the binary's own real on-disk location at runtime is
//! always `<wherever Whimbrel.app ended up>/Contents/Resources/whimbrel-mcp-server`, so
//! its parent directory *is* `Contents/Resources` regardless of where the app bundle
//! itself lives, and `models` is always a direct sibling of it. Falls back to this
//! workspace's own `core/memory/models` (populated by `apps/macos/scripts/release.sh`
//! in a dev checkout, for running the binary straight out of `cargo build` rather than
//! from inside a bundle), and otherwise passes an empty path — `Store::open`'s embedding
//! lookup already degrades gracefully on a missing/empty models dir (see `core/memory/
//! src/lib.rs`'s `open_in_memory` doc comment), so this is a real, already-established
//! fallback, not a new special case.

use mcp_server::build_router;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Matches the UI-layer convention for the local Ollama model name (`RoutingSettings.swift`,
/// `MeetingDetailView.swift`, `TimelineView.swift` all default to this same string) — not a
/// core-crate constant (`core/llm` intentionally takes the model name from its caller), so
/// this binary picks the same default the Swift UI already uses, overridable via
/// `WHIMBREL_OLLAMA_MODEL` for anyone running against a differently-named local model.
const DEFAULT_OLLAMA_MODEL: &str = "gemma4:12b";

const LISTEN_ADDR: &str = "127.0.0.1:4319";

fn resolve_db_path(
    args: impl IntoIterator<Item = String>,
    env_db: Option<String>,
    home: Option<String>,
) -> PathBuf {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--db" {
            if let Some(path) = args.next() {
                return PathBuf::from(path);
            }
        }
    }
    if let Some(path) = env_db.filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    let home = home.expect("HOME must be set to locate ~/Library/Application Support");
    PathBuf::from(home).join("Library/Application Support/Whimbrel/core-memory.sqlite")
}

/// The value after `flag` in `args`, if both are there.
fn flag_value(args: impl IntoIterator<Item = String>, flag: &str) -> Option<String> {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == flag {
            return args.next();
        }
    }
    None
}

/// What to tell the user when the port cannot be bound, as one line the app can show.
fn bind_failure_message(addr: &str, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AddrInUse {
        format!("port {} is already in use; another program is listening on {addr}", addr.rsplit(':').next().unwrap_or(addr))
    } else {
        format!("could not listen on {addr}: {error}")
    }
}

fn resolve_models_dir(flag: Option<String>) -> PathBuf {
    // The Windows app passes its own models directory; the Mac finds it next to the executable.
    if let Some(dir) = flag.filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let bundled = exe_dir.join("models");
            if bundled.is_dir() {
                return bundled;
            }
        }
    }
    let repo_models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../memory/models");
    if repo_models.is_dir() {
        return repo_models;
    }
    PathBuf::new()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = resolve_db_path(
        std::env::args().skip(1),
        std::env::var("WHIMBREL_DB").ok(),
        std::env::var("HOME").ok(),
    );
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let models_dir = resolve_models_dir(flag_value(std::env::args().skip(1), "--models"));
    let store = memory::Store::open(&db_path, &models_dir)?;
    let store = Arc::new(Mutex::new(store));

    let ollama_model =
        std::env::var("WHIMBREL_OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_OLLAMA_MODEL.to_string());
    let llm_client: Arc<dyn llm::LlmClient + Send + Sync> =
        Arc::new(llm::OllamaClient::new(ollama_model));

    // Router construction (the `StreamableHttpService` nested under `/mcp`, `AuthLayer`
    // applied to the outer `Router`) is shared with `tests/real_client_integration.rs`
    // via `mcp_server::build_router` so the real test can never silently drift from the
    // real router this binary actually serves — see `build_router`'s doc comment.
    let app = build_router(Arc::clone(&store), llm_client);

    let listener = match tokio::net::TcpListener::bind(LISTEN_ADDR).await {
        Ok(listener) => listener,
        Err(e) => {
            // One plain line on stderr; the app shows it next to the toggle.
            eprintln!("{}", bind_failure_message(LISTEN_ADDR, &e));
            std::process::exit(1);
        }
    };
    println!("Whimbrel MCP server listening on http://{LISTEN_ADDR}/mcp");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn models_dir_prefers_the_flag() {
        assert_eq!(resolve_models_dir(Some("C:/app/models".into())), PathBuf::from("C:/app/models"));
        assert_eq!(flag_value(args(&["--db", "x", "--models", "m"]), "--models").as_deref(), Some("m"));
        assert_eq!(flag_value(args(&["--db", "x"]), "--models"), None);
        assert_eq!(flag_value(args(&["--models"]), "--models"), None, "a flag with no value is not a value");
    }

    #[test]
    fn a_busy_port_is_reported_in_one_plain_line() {
        let busy = std::io::Error::from(std::io::ErrorKind::AddrInUse);
        let msg = bind_failure_message("127.0.0.1:4319", &busy);
        assert!(msg.contains("port 4319 is already in use"), "{msg}");
        assert!(!msg.contains(char::from(10)));
        let other = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(bind_failure_message("127.0.0.1:4319", &other).starts_with("could not listen on 127.0.0.1:4319"));
    }

    #[test]
    fn db_path_prefers_flag_then_env_then_default_location() {
        let home = Some("/Users/me".to_string());
        let env = Some("/env/db.sqlite".to_string());
        assert_eq!(
            resolve_db_path(args(&["--db", "/flag/db.sqlite"]), env.clone(), home.clone()),
            PathBuf::from("/flag/db.sqlite")
        );
        assert_eq!(resolve_db_path(args(&[]), env, home.clone()), PathBuf::from("/env/db.sqlite"));
        assert_eq!(
            resolve_db_path(args(&["--db"]), Some(String::new()), home),
            PathBuf::from("/Users/me/Library/Application Support/Whimbrel/core-memory.sqlite")
        );
    }
}
