//! Settings → AI assistants (MCP) (M3.4 Task 5): the server switch, and the client tokens
//! that decide who may read Whimbrel's memory through it.
//!
//! A token is created here, shown once, and stored only as a salted hash (core). Revoking
//! takes effect on the server's next request, with no restart: the server reads the token
//! table on every call.

use std::sync::Mutex;

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::error::CommandError;
use crate::mcp_server::{self, McpServer, SpawnSpec, Status};
use crate::settings::SettingsFile;
use crate::state::AppState;

/// The scopes a client can hold, in the order the card lists them. Core checks each tool
/// against these names; anything else would be a token that opens nothing.
pub const SCOPES: [&str; 3] = ["memory", "meetings", "transcripts"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    /// The user's switch. It can be on while the server is not running (it failed).
    pub enabled: bool,
    /// `stopped`, `running` or `failed`.
    pub status: String,
    pub address: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpClientView {
    pub id: i64,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_unix_ms: i64,
    pub revoked_unix_ms: Option<i64>,
}

/// Returned once, when a client is made. The token is not recoverable afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpNewClient {
    pub id: i64,
    pub token: String,
}

pub fn server_view(enabled: bool, status: Status) -> McpServerView {
    let (status, error) = match status {
        Status::Stopped => ("stopped", None),
        Status::Running => ("running", None),
        Status::Failed(why) => ("failed", Some(why)),
    };
    McpServerView { enabled, status: status.into(), address: mcp_server::ADDRESS.into(), error }
}

/// Saves the switch and starts or stops the server to match. `spec` is how to start it
/// (or why it cannot be), evaluated by the caller because it reads the install location.
pub fn apply_enabled(
    server: &McpServer,
    settings: &Mutex<SettingsFile>,
    enabled: bool,
    spec: impl FnOnce() -> Result<SpawnSpec, String>,
) -> Result<McpServerView, CommandError> {
    {
        let mut settings = settings.lock().unwrap();
        settings.set_mcp_server_enabled(enabled);
        settings.save().map_err(|e| CommandError::store(format!("could not save the setting: {e}")))?;
    }
    let status = if enabled {
        match spec() {
            Ok(spec) => server.start(&spec),
            Err(why) => Status::Failed(why),
        }
    } else {
        server.stop()
    };
    Ok(server_view(enabled, status))
}

/// Makes a client token. A blank name, no scopes, or a scope that does not exist is refused.
pub fn create_client(store: &memory::Store, name: &str, scopes: &[String]) -> Result<McpNewClient, CommandError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CommandError::invalid_input("Give the client a name."));
    }
    if name.chars().count() > 60 {
        return Err(CommandError::invalid_input("Keep the client name under 60 characters."));
    }
    if let Some(unknown) = scopes.iter().find(|s| !SCOPES.contains(&s.as_str())) {
        return Err(CommandError::invalid_input(format!("{unknown} is not something a client can be given access to.")));
    }
    // In the card's order, once each, whatever order the caller used.
    let granted: Vec<String> = SCOPES.iter().filter(|s| scopes.iter().any(|g| g == *s)).map(|s| s.to_string()).collect();
    if granted.is_empty() {
        return Err(CommandError::invalid_input("Choose what this client may read."));
    }
    let made = api::mcp::generate_client_token(store, name, granted).map_err(|e| CommandError::store(e.to_string()))?;
    Ok(McpNewClient { id: made.id, token: made.token })
}

pub fn client_views(store: &memory::Store) -> Result<Vec<McpClientView>, CommandError> {
    let mut clients: Vec<McpClientView> = api::mcp::list_client_tokens(store)
        .map_err(|e| CommandError::store(e.to_string()))?
        .into_iter()
        .map(|t| McpClientView {
            id: t.id,
            name: t.name,
            scopes: t.scopes,
            created_unix_ms: t.created_at_unix_ms,
            revoked_unix_ms: t.revoked_at_unix_ms,
        })
        .collect();
    clients.sort_by(|a, b| b.created_unix_ms.cmp(&a.created_unix_ms).then(b.id.cmp(&a.id)));
    Ok(clients)
}

fn spawn_spec(state: &AppState) -> Result<SpawnSpec, String> {
    let app_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    Ok(SpawnSpec {
        exe: mcp_server::locate(&app_dir, &state.paths.root)?,
        db: state.paths.db.clone(),
        models: state.paths.minilm.clone(),
        ollama_model: state.settings.lock().unwrap().routing().ollama_model,
    })
}

fn current_view(state: &AppState) -> McpServerView {
    let enabled = state.settings.lock().unwrap().mcp_server_enabled();
    match &state.mcp_server {
        Some(server) => server_view(enabled, server.status()),
        None => server_view(enabled, Status::Failed("The MCP server could not be set up on this PC.".into())),
    }
}

/// Starts the server at launch if the user left it on. After the store is open.
pub fn start_if_enabled(state: &AppState) {
    if state.store.is_none() || !state.settings.lock().unwrap().mcp_server_enabled() {
        return;
    }
    let Some(server) = &state.mcp_server else { return };
    match spawn_spec(state) {
        Ok(spec) => {
            let status = server.start(&spec);
            tracing::info!(?status, "MCP server started at launch");
        }
        Err(why) => tracing::warn!(why, "the MCP server is on but cannot start"),
    }
}

#[tauri::command]
pub fn mcp_server_state(state: State<'_, AppState>) -> McpServerView {
    current_view(&state)
}

#[tauri::command]
pub async fn mcp_set_enabled(app: AppHandle, enabled: bool) -> Result<McpServerView, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let Some(server) = &state.mcp_server else {
            return Err(CommandError::not_configured("The MCP server could not be set up on this PC."));
        };
        if enabled && state.store.is_none() {
            return Err(CommandError::store("the database is not open; see Settings for the startup error"));
        }
        apply_enabled(server, &state.settings, enabled, || spawn_spec(&state))
    })
    .await
    .map_err(|e| CommandError::store(format!("switching the MCP server failed: {e}")))?
}

#[tauri::command]
pub async fn mcp_clients(state: State<'_, AppState>) -> Result<Vec<McpClientView>, CommandError> {
    state.store()?.call(|s| client_views(s)).await?
}

#[tauri::command]
pub async fn mcp_create_client(state: State<'_, AppState>, name: String, scopes: Vec<String>) -> Result<McpNewClient, CommandError> {
    state.store()?.call(move |s| create_client(s, &name, &scopes)).await?
}

#[tauri::command]
pub async fn mcp_revoke_client(state: State<'_, AppState>, id: i64) -> Result<Vec<McpClientView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            s.revoke_mcp_client_token(id).map_err(|e| CommandError::store(e.to_string()))?;
            client_views(s)
        })
        .await?
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use memory::Store;

    use super::*;
    use crate::mcp_server::{Child, Spawner};

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    // --- tokens ---

    #[test]
    fn a_new_client_gets_its_token_once_and_the_list_never_shows_it() {
        let store = Store::open_in_memory().unwrap();
        let made = create_client(&store, "  Claude Desktop ", &strings(&["memory"])).unwrap();
        assert!(made.token.starts_with(&format!("kmcp_{}.", made.id)), "{}", made.token);

        let listed = client_views(&store).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Claude Desktop", "the name is trimmed");
        let json = serde_json::to_string(&listed).unwrap();
        let secret = made.token.split('.').nth(1).unwrap();
        assert!(!json.contains(secret) && !json.contains("hash") && !json.contains("salt"), "{json}");
    }

    #[test]
    fn scopes_are_kept_in_the_cards_order_once_each() {
        let store = Store::open_in_memory().unwrap();
        create_client(&store, "a", &strings(&["transcripts", "memory", "memory"])).unwrap();
        assert_eq!(client_views(&store).unwrap()[0].scopes, ["memory", "transcripts"]);
    }

    #[test]
    fn a_client_needs_a_name_and_a_real_scope() {
        let store = Store::open_in_memory().unwrap();
        let message = |r: Result<McpNewClient, CommandError>| r.unwrap_err().to_string();
        assert!(message(create_client(&store, "   ", &strings(&["memory"]))).contains("name"));
        assert!(message(create_client(&store, "x", &[])).contains("what this client may read"));
        assert!(message(create_client(&store, "x", &strings(&["admin"]))).contains("admin"));
        assert!(message(create_client(&store, "x", &strings(&["memory", "everything"]))).contains("everything"));
        assert!(message(create_client(&store, &"n".repeat(61), &strings(&["memory"]))).contains("60"));
        assert!(client_views(&store).unwrap().is_empty(), "nothing was made");
    }

    #[test]
    fn revoking_marks_the_client_and_leaves_the_others() {
        let store = Store::open_in_memory().unwrap();
        let a = create_client(&store, "a", &strings(&["memory"])).unwrap();
        let b = create_client(&store, "b", &strings(&["meetings"])).unwrap();
        store.revoke_mcp_client_token(a.id).unwrap();
        let listed = client_views(&store).unwrap();
        let by_id = |id: i64| listed.iter().find(|c| c.id == id).unwrap();
        assert!(by_id(a.id).revoked_unix_ms.is_some());
        assert!(by_id(b.id).revoked_unix_ms.is_none());
    }

    #[test]
    fn the_newest_client_is_listed_first() {
        let store = Store::open_in_memory().unwrap();
        create_client(&store, "first", &strings(&["memory"])).unwrap();
        create_client(&store, "second", &strings(&["memory"])).unwrap();
        assert_eq!(client_views(&store).unwrap()[0].name, "second");
    }

    // --- the switch ---

    struct CountingSpawner(Arc<AtomicUsize>);
    struct Alive;
    impl Child for Alive {
        fn try_wait(&mut self) -> Result<Option<i32>, String> {
            Ok(None)
        }
        fn kill(&mut self) {}
        fn last_stderr_line(&self) -> Option<String> {
            None
        }
    }
    impl Spawner for CountingSpawner {
        fn spawn(&self, _: &SpawnSpec) -> Result<Box<dyn Child>, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Alive))
        }
    }

    fn temp_settings(name: &str) -> (Mutex<SettingsFile>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("whimbrel-mcp-cmd-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        (Mutex::new(SettingsFile::load(&path)), path)
    }

    fn spec_with_db(name: &str) -> SpawnSpec {
        let dir = std::env::temp_dir().join(format!("whimbrel-mcp-cmd-db-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("core-memory.sqlite");
        std::fs::write(&db, b"").unwrap();
        SpawnSpec { exe: dir.join("whimbrel-mcp-server.exe"), db, models: dir.join("models"), ollama_model: "m".into() }
    }

    #[test]
    fn it_is_off_by_default() {
        let (settings, _) = temp_settings("default");
        assert!(!settings.lock().unwrap().mcp_server_enabled());
    }

    #[test]
    fn switching_on_saves_the_choice_and_starts_the_server() {
        let spawned = Arc::new(AtomicUsize::new(0));
        let server = McpServer::new(Box::new(CountingSpawner(spawned.clone())));
        let (settings, path) = temp_settings("on");
        let view = apply_enabled(&server, &settings, true, || Ok(spec_with_db("on"))).unwrap();
        assert_eq!(view, McpServerView { enabled: true, status: "running".into(), address: "http://127.0.0.1:4319/mcp".into(), error: None });
        assert_eq!(spawned.load(Ordering::SeqCst), 1);
        assert!(SettingsFile::load(&path).mcp_server_enabled(), "saved to disk, so a relaunch resumes it");
    }

    #[test]
    fn switching_off_stops_it_and_saves_the_choice() {
        let server = McpServer::new(Box::new(CountingSpawner(Arc::new(AtomicUsize::new(0)))));
        let (settings, path) = temp_settings("off");
        apply_enabled(&server, &settings, true, || Ok(spec_with_db("off"))).unwrap();
        let view = apply_enabled(&server, &settings, false, || unreachable!("no spec is needed to stop")).unwrap();
        assert_eq!((view.enabled, view.status.as_str()), (false, "stopped"));
        assert_eq!(server.status(), Status::Stopped, "the process itself was stopped, not just the label");
        assert!(!SettingsFile::load(&path).mcp_server_enabled());
    }

    #[test]
    fn switching_on_with_no_program_installed_keeps_the_switch_on_and_says_why() {
        let server = McpServer::new(Box::new(CountingSpawner(Arc::new(AtomicUsize::new(0)))));
        let (settings, _) = temp_settings("missing");
        let view = apply_enabled(&server, &settings, true, || Err("The MCP server program isn't installed.".into())).unwrap();
        assert_eq!((view.enabled, view.status.as_str()), (true, "failed"));
        assert_eq!(view.error.as_deref(), Some("The MCP server program isn't installed."));
    }
}
