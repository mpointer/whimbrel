//! Finding Node and the connector scripts (M3.3 Decision 8, M3.4 Task 6).
//!
//! An installed Whimbrel carries its own `node\node.exe` and `connectors\<name>\` beside
//! its executable, so connecting Calendar or Gmail needs nothing else on the PC. Those are
//! used first. A development build has neither, and falls back to the `node` on `PATH` and
//! the scripts `prepare-connectors.ps1` stages under `%LOCALAPPDATA%\Whimbrel\data\connectors\`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use mcp_host::McpHost;

use crate::error::CommandError;

/// The connector's directory name and the environment variable it reads its token from.
pub const GOOGLE_CALENDAR: (&str, &str) = ("google-calendar", "GOOGLE_CALENDAR_ACCESS_TOKEN");
/// Gmail's connector directory and the variable it reads its token from (the plan names the same pair).
pub const GMAIL: (&str, &str) = ("gmail", "GMAIL_ACCESS_TOKEN");

/// Why a connector can't run, in words the Settings card can show.
pub fn missing_node() -> CommandError {
    CommandError::not_configured("Node.js wasn't found, so the Google connectors can't run (calendar syncing, sending approved emails). Whimbrel ships with it, so reinstall Whimbrel; if you build it from source, install Node LTS and restart.")
}

fn missing_script(path: &Path) -> CommandError {
    CommandError::not_configured(format!(
        "The connector isn't installed at {}. Reinstall Whimbrel, or if you build it from source run apps\\windows\\scripts\\prepare-connectors.ps1.",
        path.display()
    ))
}

/// The folder Whimbrel's executable is in, where an install puts `node\` and `connectors\`.
fn app_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// The Node that ships with Whimbrel, if this is an install that has it.
pub fn bundled_node(app_dir: &Path) -> Option<PathBuf> {
    let path = app_dir.join("node").join("node.exe");
    path.is_file().then_some(path)
}

/// Which `node.exe` to run: the one beside the app first, then (development) the one on `PATH`.
/// `on_path` is only asked when the bundled copy is absent.
pub fn resolve_node(app_dir: Option<&Path>, on_path: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    app_dir.and_then(bundled_node).or_else(on_path)
}

fn node_on_path() -> Option<PathBuf> {
    let output = std::process::Command::new("where").arg("node.exe").output().ok()?;
    let first = String::from_utf8_lossy(&output.stdout).lines().next()?.trim().to_string();
    let path = PathBuf::from(first);
    path.is_file().then_some(path)
}

/// The Node to run, cached: the `PATH` lookup runs a process, and polling asks often.
pub fn node_path() -> Option<&'static Path> {
    static NODE: OnceLock<Option<PathBuf>> = OnceLock::new();
    NODE.get_or_init(|| resolve_node(app_dir().as_deref(), node_on_path)).as_deref()
}

/// Where the staged (development) connectors live.
pub fn connectors_dir(root: &Path) -> PathBuf {
    root.join("connectors")
}

fn entry(dir: &Path, connector: &str) -> PathBuf {
    dir.join(connector).join("dist").join("index.js")
}

/// The script to run for a connector: the copy beside the app first, then the one the
/// development script staged under `root`. Or why neither exists.
pub fn resolve_script(app_dir: Option<&Path>, root: &Path, connector: &str) -> Result<PathBuf, CommandError> {
    if let Some(app_dir) = app_dir {
        let bundled = entry(&app_dir.join("connectors"), connector);
        if bundled.is_file() {
            return Ok(bundled);
        }
    }
    let staged = entry(&connectors_dir(root), connector);
    if staged.is_file() {
        return Ok(staged);
    }
    Err(missing_script(&staged))
}

/// The script to run for a connector, or why it can't be run.
pub fn script(root: &Path, connector: &str) -> Result<PathBuf, CommandError> {
    resolve_script(app_dir().as_deref(), root, connector)
}

/// Whether a connector can run at all right now (Node present, script staged).
pub fn ready(root: &Path, connector: &str) -> Result<(), CommandError> {
    node_path().ok_or_else(missing_node)?;
    script(root, connector).map(|_| ())
}

/// One connector call: spawn, list tools, call, disconnect. The token goes in the
/// environment, never on the command line, where other processes could read it. `label`
/// names the connector in messages ("calendar", "Gmail").
pub fn call_connector(
    host: &McpHost,
    root: &Path,
    (connector, token_env): (&str, &str),
    label: &str,
    tool: &str,
    args_json: &str,
    token: &str,
) -> Result<String, CommandError> {
    let node = node_path().ok_or_else(missing_node)?;
    let script = script(root, connector)?;
    let id = host
        .connect(
            &node.to_string_lossy(),
            &[script.to_string_lossy().into_owned()],
            &[(token_env.to_string(), token.to_string())],
        )
        .map_err(|e| CommandError::network(format!("the {label} connector didn't start: {e}")))?;
    let call = (|| {
        let tools = host.list_tools(id).map_err(|e| format!("the connector didn't list its tools: {e}"))?;
        if !tools.iter().any(|t| t.name == tool) {
            return Err(format!("the connector has no {tool} tool"));
        }
        host.call_tool(id, tool, args_json).map_err(|e| format!("{tool} failed: {e}"))
    })();
    let _ = host.disconnect(id);
    call.map_err(CommandError::network)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staged_connector_resolves_to_its_entry_script() {
        let root = std::env::temp_dir().join(format!("whimbrel-connectors-{}", std::process::id()));
        let dist = connectors_dir(&root).join("google-calendar").join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        assert!(script(&root, "google-calendar").is_err(), "no index.js yet");

        std::fs::write(dist.join("index.js"), b"// connector").unwrap();
        assert_eq!(script(&root, "google-calendar").unwrap(), dist.join("index.js"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// An app folder and a data folder in the temp directory, with the given files created.
    fn layout(files: &[&str]) -> (PathBuf, PathBuf, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!("whimbrel-node-layout-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
        let (app, data) = (base.join("app"), base.join("data"));
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        for f in files {
            let path = base.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        (base, app, data)
    }

    #[test]
    fn the_node_beside_the_app_beats_the_one_on_path() {
        let (base, app, _) = layout(&["app/node/node.exe"]);
        let on_path = || Some(PathBuf::from("C:/Program Files/nodejs/node.exe"));
        assert_eq!(resolve_node(Some(&app), on_path), Some(app.join("node").join("node.exe")));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn without_a_bundled_node_the_one_on_path_is_used() {
        let (base, app, _) = layout(&[]);
        let on_path = || Some(PathBuf::from("C:/Program Files/nodejs/node.exe"));
        assert_eq!(resolve_node(Some(&app), on_path), Some(PathBuf::from("C:/Program Files/nodejs/node.exe")));
        assert_eq!(resolve_node(None, on_path), Some(PathBuf::from("C:/Program Files/nodejs/node.exe")));
        assert_eq!(resolve_node(Some(&app), || None), None, "neither: nothing, so the card says so");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn with_a_bundled_node_the_path_is_never_searched() {
        // The PATH lookup runs a process; an install must not pay for it, or be thrown by it.
        let (base, app, _) = layout(&["app/node/node.exe"]);
        let found = resolve_node(Some(&app), || panic!("PATH must not be consulted when Node ships with the app"));
        assert!(found.is_some());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_connector_beside_the_app_beats_the_staged_development_copy() {
        let (base, app, data) = layout(&["app/connectors/gmail/dist/index.js", "data/connectors/gmail/dist/index.js"]);
        assert_eq!(resolve_script(Some(&app), &data, "gmail").unwrap(), app.join("connectors").join("gmail").join("dist").join("index.js"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn without_a_bundled_connector_the_staged_one_is_used() {
        let (base, app, data) = layout(&["data/connectors/gmail/dist/index.js"]);
        assert_eq!(resolve_script(Some(&app), &data, "gmail").unwrap(), data.join("connectors").join("gmail").join("dist").join("index.js"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn another_connectors_bundled_copy_does_not_stand_in() {
        let (base, app, data) = layout(&["app/connectors/google-calendar/dist/index.js"]);
        assert!(resolve_script(Some(&app), &data, "gmail").is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn with_neither_the_message_says_to_reinstall_or_stage() {
        let (base, app, data) = layout(&[]);
        let error = format!("{}", resolve_script(Some(&app), &data, "gmail").unwrap_err());
        assert!(error.contains("Reinstall Whimbrel") && error.contains("prepare-connectors.ps1"), "{error}");
        let node = format!("{}", missing_node());
        assert!(node.contains("reinstall Whimbrel") && node.contains("Node LTS"), "{node}");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_missing_connector_says_how_to_install_it() {
        let root = std::env::temp_dir().join("whimbrel-connectors-absent");
        let error = script(&root, "google-calendar").unwrap_err();
        assert!(format!("{error}").contains("prepare-connectors.ps1"), "{error}");
    }

    #[test]
    fn node_is_found_on_this_machine_or_reported_missing() {
        // Both outcomes are legitimate; what matters is that the lookup doesn't panic and
        // that a missing Node turns into a message rather than a silent failure.
        match node_path() {
            Some(path) => assert!(path.is_file()),
            None => assert!(format!("{}", missing_node()).contains("Node LTS")),
        }
    }
}
