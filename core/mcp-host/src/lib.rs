//! Real logic for the `mcp-host` subsystem (docs/04-architecture.md): spawns
//! a stdio MCP server, lists its tools, and calls one. Wraps the official
//! Rust MCP SDK, `rmcp`. `rmcp` is async (built on `tokio`); every function
//! here is a synchronous, blocking call over one lazily-initialized, global
//! `tokio::runtime::Runtime` (one per process, not one per call — a spawned
//! child process's stdio I/O runs as background async tasks that must keep
//! running between calls, so a fresh runtime per call would break that) —
//! the same "own the async, expose sync at the boundary" pattern `core/ffi`
//! already uses elsewhere.
//!
//! This crate never touches the Keychain or OAuth internals — every
//! function receives an already-valid credential (e.g. an access token) as
//! a plain string argument where a connector needs one, exactly like every
//! other FFI-adjacent function in this workspace receives already-resolved
//! inputs (a device public key, a whisper model path) rather than reaching
//! into platform-specific storage itself.
//!
//! Token refresh note: a connector's credentials (e.g. an OAuth access
//! token) are injected via `env` at `connect()` time and cannot be
//! updated on an already-running child process. If a call fails because
//! a token expired mid-session, the caller must `disconnect()` and
//! `connect()` again with a fresh token — retrying `call_tool` alone on
//! the same connector id will keep using the stale token.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use tokio::process::Command;
use tokio::runtime::Runtime;

pub type ConnectorId = u64;

#[derive(Debug)]
pub enum McpHostError {
    Spawn(String),
    ProtocolError(String),
    ToolError(String),
    UnknownConnector,
    MalformedArguments(String),
    MalformedToolResult(String),
}

impl fmt::Display for McpHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McpHostError::Spawn(msg) => write!(f, "failed to spawn MCP server: {msg}"),
            McpHostError::ProtocolError(msg) => write!(f, "MCP protocol error: {msg}"),
            McpHostError::ToolError(msg) => write!(f, "tool call failed: {msg}"),
            McpHostError::UnknownConnector => write!(f, "unknown connector id"),
            McpHostError::MalformedArguments(msg) => {
                write!(f, "malformed tool arguments JSON: {msg}")
            }
            McpHostError::MalformedToolResult(msg) => write!(f, "malformed tool result: {msg}"),
        }
    }
}

impl std::error::Error for McpHostError {}

pub struct ToolInfo {
    pub name: String,
    pub description: Option<String>,
}

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to create tokio runtime"))
}

/// A registry of live MCP connectors. `core/ffi` holds one instance of this
/// behind an opaque handle, the same shape as `memory::StoreHandle`.
pub struct McpHost {
    connectors: Mutex<HashMap<ConnectorId, RunningService<RoleClient, ()>>>,
    next_id: AtomicU64,
}

impl Default for McpHost {
    fn default() -> Self {
        Self::new()
    }
}

/// Windows only: keep a connector's console window from flashing. Every connector is a
/// Node script, and a GUI shell spawning one without this shows a console for each call —
/// several times a minute once calendar and mail polling run (M3.3 Task 7).
#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    /// `CREATE_NO_WINDOW`, from `windows-sys`' `Win32::System::Threading`. Spelled out
    /// rather than pulled in as a dependency for one constant.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}

impl McpHost {
    pub fn new() -> Self {
        McpHost {
            connectors: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Spawns `command` with `args` and `env`, performs the MCP `initialize`
    /// handshake over stdio, and returns an id for later `list_tools`/
    /// `call_tool`/`disconnect` calls.
    pub fn connect(
        &self,
        command: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<ConnectorId, McpHostError> {
        runtime().block_on(async {
            let mut cmd = Command::new(command);
            cmd.args(args);
            for (key, value) in env {
                cmd.env(key, value);
            }
            hide_console(&mut cmd);
            let transport =
                TokioChildProcess::new(cmd).map_err(|e| McpHostError::Spawn(e.to_string()))?;
            let service =
                ().serve(transport)
                    .await
                    .map_err(|e| McpHostError::ProtocolError(e.to_string()))?;

            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            self.connectors.lock().unwrap().insert(id, service);
            Ok(id)
        })
    }

    /// Lists the tools the connector at `id` exposes.
    pub fn list_tools(&self, id: ConnectorId) -> Result<Vec<ToolInfo>, McpHostError> {
        let peer = {
            let connectors = self.connectors.lock().unwrap();
            let service = connectors.get(&id).ok_or(McpHostError::UnknownConnector)?;
            service.peer().clone()
        };
        runtime().block_on(async {
            let result = peer
                .list_tools(None)
                .await
                .map_err(|e| McpHostError::ProtocolError(e.to_string()))?;
            Ok(result
                .tools
                .into_iter()
                .map(|t| ToolInfo {
                    name: t.name.to_string(),
                    description: t.description.map(|d| d.to_string()),
                })
                .collect())
        })
    }

    /// Calls `tool_name` on the connector at `id` with `arguments_json` (a
    /// JSON object string), returning the tool's text content as a JSON
    /// string. Returns `Err` both for transport/protocol failures and for a
    /// tool-level failure (`CallToolResult.is_error == Some(true)`) — the
    /// underlying MCP protocol distinguishes these, but this crate's public
    /// API deliberately does not, matching `core/ffi`'s existing
    /// null-on-any-failure convention one layer up.
    pub fn call_tool(
        &self,
        id: ConnectorId,
        tool_name: &str,
        arguments_json: &str,
    ) -> Result<String, McpHostError> {
        let arguments: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(arguments_json)
                .map_err(|e| McpHostError::MalformedArguments(e.to_string()))?;

        let peer = {
            let connectors = self.connectors.lock().unwrap();
            let service = connectors.get(&id).ok_or(McpHostError::UnknownConnector)?;
            service.peer().clone()
        };

        runtime().block_on(async {
            let params =
                CallToolRequestParams::new(tool_name.to_string()).with_arguments(arguments);
            let result = peer
                .call_tool(params)
                .await
                .map_err(|e| McpHostError::ProtocolError(e.to_string()))?;

            if result.is_error == Some(true) {
                let message = result
                    .content
                    .iter()
                    .find_map(|c| c.as_text().map(|t| t.text.clone()))
                    .unwrap_or_else(|| "tool returned an error with no text content".to_string());
                return Err(McpHostError::ToolError(message));
            }

            result
                .content
                .iter()
                .find_map(|c| c.as_text().map(|t| t.text.clone()))
                .ok_or_else(|| {
                    McpHostError::MalformedToolResult("tool result had no text content".to_string())
                })
        })
    }

    /// Disconnects the connector at `id`, killing its child process.
    pub fn disconnect(&self, id: ConnectorId) -> Result<(), McpHostError> {
        let service = self
            .connectors
            .lock()
            .unwrap()
            .remove(&id)
            .ok_or(McpHostError::UnknownConnector)?;
        runtime().block_on(async {
            service
                .cancel()
                .await
                .map_err(|e| McpHostError::ProtocolError(e.to_string()))?;
            Ok(())
        })
    }
}
