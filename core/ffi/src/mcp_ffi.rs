//! FFI bindings for `mcp_host::McpHost`. Follows the same conventions as
//! `sync_ffi.rs`: JSON-in/JSON-out, null-pointer-or-negative-one-means-
//! failure, no error-detail envelope. Unlike `sync_ffi.rs`'s functions,
//! these are stateful — `whimbrel_mcp_open`/`whimbrel_mcp_close` open
//! and close a handle over an `McpHost` registry, the same opaque-handle
//! shape as `StoreHandle`.

use std::ffi::c_char;
use std::ptr;

use mcp_host::{ConnectorId, McpHost, McpHostError};

use crate::{cstr_to_string, json_to_c_string};

pub struct McpHostHandle(McpHost);

/// Opens a new MCP host registry. Never fails.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_mcp_open() -> *mut McpHostHandle {
    Box::into_raw(Box::new(McpHostHandle(McpHost::new())))
}

/// Closes an MCP host registry opened by `whimbrel_mcp_open`. Any
/// connectors still open on it are dropped (their child processes are
/// killed) when the underlying `McpHost` is dropped.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_mcp_close(handle: *mut McpHostHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

/// Spawns `command` with the given args/env and performs the MCP
/// `initialize` handshake. `args_json` is a JSON array of strings;
/// `env_json` is a JSON array of `[key, value]` two-element string arrays.
/// Returns a non-negative connector id on success, or `-1` on any failure
/// (null/invalid handle, malformed JSON, or a spawn/protocol error).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_mcp_connect(
    handle: *mut McpHostHandle,
    command: *const c_char,
    args_json: *const c_char,
    env_json: *const c_char,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let Some(command) = cstr_to_string(command) else {
        return -1;
    };
    let Some(args_json) = cstr_to_string(args_json) else {
        return -1;
    };
    let Ok(args) = serde_json::from_str::<Vec<String>>(&args_json) else {
        return -1;
    };
    let Some(env_json) = cstr_to_string(env_json) else {
        return -1;
    };
    let Ok(env_pairs) = serde_json::from_str::<Vec<(String, String)>>(&env_json) else {
        return -1;
    };

    let host = unsafe { &(*handle).0 };
    match host.connect(&command, &args, &env_pairs) {
        Ok(id) => id as i64,
        Err(_) => -1,
    }
}

/// Lists the tools the connector at `connector_id` exposes. Returns a JSON
/// array of `{"name": ..., "description": ...}`, or null on any failure
/// (null/invalid handle, unknown connector id, or a protocol error).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_mcp_list_tools(
    handle: *mut McpHostHandle,
    connector_id: i64,
) -> *mut c_char {
    if handle.is_null() || connector_id < 0 {
        return ptr::null_mut();
    }
    let host = unsafe { &(*handle).0 };
    let Ok(tools) = host.list_tools(connector_id as ConnectorId) else {
        return ptr::null_mut();
    };
    let json = serde_json::json!(tools
        .into_iter()
        .map(|t| serde_json::json!({ "name": t.name, "description": t.description }))
        .collect::<Vec<_>>());
    match serde_json::to_string(&json) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Calls `tool_name` on the connector at `connector_id` with
/// `arguments_json` (a JSON object string). Returns the tool's raw text
/// result as a JSON string, `{"error": "<message>"}` when the tool call
/// itself or the protocol fails, or null on any other failure (null/invalid
/// handle, unknown connector id, malformed arguments).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_mcp_call_tool(
    handle: *mut McpHostHandle,
    connector_id: i64,
    tool_name: *const c_char,
    arguments_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() || connector_id < 0 {
        return ptr::null_mut();
    }
    let Some(tool_name) = cstr_to_string(tool_name) else {
        return ptr::null_mut();
    };
    let Some(arguments_json) = cstr_to_string(arguments_json) else {
        return ptr::null_mut();
    };

    let host = unsafe { &(*handle).0 };
    let json = match host.call_tool(connector_id as ConnectorId, &tool_name, &arguments_json) {
        Ok(result) => serde_json::Value::String(result),
        Err(McpHostError::ToolError(message) | McpHostError::ProtocolError(message)) => {
            serde_json::json!({ "error": message })
        }
        Err(_) => return ptr::null_mut(),
    };
    match serde_json::to_string(&json) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Disconnects the connector at `connector_id`, killing its child process.
/// Returns `true` on success, `false` on any failure (null/invalid handle
/// or unknown connector id).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_mcp_disconnect(handle: *mut McpHostHandle, connector_id: i64) -> bool {
    if handle.is_null() || connector_id < 0 {
        return false;
    }
    let host = unsafe { &(*handle).0 };
    host.disconnect(connector_id as ConnectorId).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::path::PathBuf;

    fn to_cstring(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn fixture_path() -> String {
        let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        dir.pop();
        dir.push("mcp-host/tests/fixtures");
        assert!(
            dir.join("node_modules").is_dir(),
            "the echo-server fixture's dependencies are not installed; run `npm ci` in {}",
            dir.display()
        );
        dir.join("echo-server.mjs").to_str().unwrap().to_string()
    }

    fn json_string_from_ptr(ptr: *mut std::ffi::c_char) -> String {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        crate::whimbrel_core_free_string(ptr);
        s
    }

    #[test]
    fn connects_lists_tools_calls_a_tool_and_disconnects() {
        let handle = whimbrel_mcp_open();
        assert!(!handle.is_null());

        let command = to_cstring("node");
        let args_json = to_cstring(&serde_json::json!([fixture_path()]).to_string());
        let env_json = to_cstring("[]");
        let connector_id = whimbrel_mcp_connect(
            handle,
            command.as_ptr(),
            args_json.as_ptr(),
            env_json.as_ptr(),
        );
        assert!(connector_id >= 0);

        let tools_json = json_string_from_ptr(whimbrel_mcp_list_tools(handle, connector_id));
        let tools: serde_json::Value = serde_json::from_str(&tools_json).unwrap();
        assert_eq!(tools[0]["name"].as_str().unwrap(), "echo");

        let arguments = to_cstring(r#"{"message": "hello"}"#);
        let tool_name = to_cstring("echo");
        let result = json_string_from_ptr(whimbrel_mcp_call_tool(
            handle,
            connector_id,
            tool_name.as_ptr(),
            arguments.as_ptr(),
        ));
        let parsed: String = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed, "hello");

        assert!(whimbrel_mcp_disconnect(handle, connector_id));
        whimbrel_mcp_close(handle);
    }

    #[test]
    fn call_tool_returns_an_error_object_when_the_tool_itself_fails() {
        let handle = whimbrel_mcp_open();
        let command = to_cstring("node");
        let args_json = to_cstring(&serde_json::json!([fixture_path()]).to_string());
        let env_json = to_cstring("[]");
        let connector_id = whimbrel_mcp_connect(
            handle,
            command.as_ptr(),
            args_json.as_ptr(),
            env_json.as_ptr(),
        );
        assert!(connector_id >= 0);

        let arguments = to_cstring(r#"{"message": "fail"}"#);
        let tool_name = to_cstring("echo");
        let result =
            whimbrel_mcp_call_tool(handle, connector_id, tool_name.as_ptr(), arguments.as_ptr());
        let value: serde_json::Value = serde_json::from_str(&json_string_from_ptr(result)).unwrap();
        assert!(value["error"].as_str().is_some_and(|m| m.contains("intentional failure")), "{value}");

        whimbrel_mcp_close(handle);
    }

    #[test]
    fn connect_returns_negative_one_for_a_nonexistent_command() {
        let handle = whimbrel_mcp_open();
        let command = to_cstring("this-binary-does-not-exist-anywhere");
        let args_json = to_cstring("[]");
        let env_json = to_cstring("[]");
        let connector_id = whimbrel_mcp_connect(
            handle,
            command.as_ptr(),
            args_json.as_ptr(),
            env_json.as_ptr(),
        );
        assert_eq!(connector_id, -1);
        whimbrel_mcp_close(handle);
    }

    #[test]
    fn list_tools_returns_null_for_an_unknown_connector_id() {
        let handle = whimbrel_mcp_open();
        let result = whimbrel_mcp_list_tools(handle, 99999);
        assert!(result.is_null());
        whimbrel_mcp_close(handle);
    }

    #[test]
    fn open_returns_null_pointer_never() {
        // whimbrel_mcp_open() never fails (McpHost::new() is infallible) --
        // this test documents that invariant rather than testing a real failure path.
        let handle = whimbrel_mcp_open();
        assert!(!handle.is_null());
        whimbrel_mcp_close(handle);
    }
}
