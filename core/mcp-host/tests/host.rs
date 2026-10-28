use mcp_host::McpHost;
use std::path::PathBuf;

fn fixture_path() -> String {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.push("tests/fixtures");
    assert!(
        dir.join("node_modules").is_dir(),
        "the echo-server fixture's dependencies are not installed; run `npm ci` in {}",
        dir.display()
    );
    dir.join("echo-server.mjs").to_str().unwrap().to_string()
}

#[test]
fn connects_lists_tools_and_calls_a_tool() {
    let host = McpHost::new();
    let id = host
        .connect("node", &[fixture_path()], &[])
        .expect("connect should succeed");

    let tools = host.list_tools(id).expect("list_tools should succeed");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");

    let result = host
        .call_tool(id, "echo", r#"{"message": "hello"}"#)
        .expect("call_tool should succeed");
    assert_eq!(result, "hello");

    host.disconnect(id).expect("disconnect should succeed");
}

#[test]
fn call_tool_passes_env_vars_to_the_spawned_process() {
    let host = McpHost::new();
    let id = host
        .connect(
            "node",
            &[fixture_path()],
            &[("MCP_HOST_TEST_ENV_VAR".to_string(), "injected".to_string())],
        )
        .expect("connect should succeed");

    let result = host
        .call_tool(id, "echo", r#"{"message": "hello"}"#)
        .expect("call_tool should succeed");
    assert_eq!(result, "injected:hello");
}

#[test]
fn call_tool_returns_an_error_when_the_tool_itself_fails() {
    let host = McpHost::new();
    let id = host
        .connect("node", &[fixture_path()], &[])
        .expect("connect should succeed");

    let result = host.call_tool(id, "echo", r#"{"message": "fail"}"#);
    assert!(result.is_err());
}

#[test]
fn operations_on_an_unknown_connector_id_return_an_error() {
    let host = McpHost::new();
    let result = host.list_tools(99999);
    assert!(result.is_err());
}

#[test]
fn connect_fails_for_a_nonexistent_command() {
    let host = McpHost::new();
    let result = host.connect("this-binary-does-not-exist-anywhere", &[], &[]);
    assert!(result.is_err());
}
