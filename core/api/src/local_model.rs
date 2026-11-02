//! The small local model behind Ask with nothing configured: a bundled `llama-server` helper,
//! started on demand and stopped when idle. See
//! docs/superpowers/specs/2026-11-01-ask-with-a-local-model-design.md.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use rand::RngCore;

const IDLE_LIMIT: Duration = Duration::from_secs(5 * 60);
const IDLE_CHECK: Duration = Duration::from_secs(30);
/// Loading takes seconds on an SSD, longer from a cold slow disk.
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// Prompts are chunked to fit 4,096 tokens (`llm::max_transcript_chars_per_chunk`).
const CONTEXT_TOKENS: &str = "8192";
pub const MODEL_ID: &str = "local";

/// Where a running helper answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
}

struct Running {
    child: Child,
    endpoint: Endpoint,
    last_used: Instant,
}

#[derive(Default)]
struct State {
    paths: Option<(PathBuf, PathBuf)>,
    running: Option<Running>,
    watching: bool,
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Called by the shell when the helper and the model are both on disk; `None` when the model goes.
pub fn configure(paths: Option<(PathBuf, PathBuf)>) {
    let mut s = state();
    if paths.is_none() {
        kill(&mut s);
    }
    s.paths = paths;
}

pub fn is_configured() -> bool {
    state().paths.is_some()
}

/// Stops the helper, freeing its memory. The next model call starts it again.
pub fn stop() {
    kill(&mut state());
}

fn kill(s: &mut State) {
    if let Some(mut running) = s.running.take() {
        let _ = running.child.kill();
        let _ = running.child.wait();
    }
}

/// The running helper's endpoint, starting it if needed. Blocks while the model loads.
pub fn ensure_running() -> Result<Endpoint, String> {
    let mut s = state();
    if let Some(running) = s.running.as_mut() {
        if matches!(running.child.try_wait(), Ok(None)) {
            running.last_used = Instant::now();
            return Ok(running.endpoint.clone());
        }
        s.running = None;
    }
    let (server, model) = s.paths.clone().ok_or("The local model isn't installed.")?;
    let (child, endpoint) = start(&server, &model)?;
    s.running = Some(Running { child, endpoint: endpoint.clone(), last_used: Instant::now() });
    if !s.watching {
        s.watching = true;
        std::thread::spawn(watch_idle);
    }
    Ok(endpoint)
}

fn watch_idle() {
    loop {
        std::thread::sleep(IDLE_CHECK);
        let mut s = state();
        if s.running.as_ref().is_some_and(|r| r.last_used.elapsed() >= IDLE_LIMIT) {
            kill(&mut s);
        }
    }
}

fn start(server: &Path, model: &Path) -> Result<(Child, Endpoint), String> {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_err(|e| format!("no free local port: {e}"))?
        .port();
    let api_key = new_key();
    // The key goes through the environment, since other processes can read arguments.
    let mut command = Command::new(server);
    // llama-server is a console program, so Windows would open a window on every cold start.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let mut child = command
        .args(["--host", "127.0.0.1", "--port", &port.to_string(), "-ngl", "99", "-c", CONTEXT_TOKENS, "--jinja", "--reasoning", "off", "-m"])
        .arg(model)
        .env("LLAMA_API_KEY", &api_key)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("the local model helper didn't start: {e}"))?;
    let base_url = format!("http://127.0.0.1:{port}");
    let started = Instant::now();
    while started.elapsed() < START_TIMEOUT {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Err("the local model helper stopped while loading".into());
        }
        if ureq::get(&format!("{base_url}/health")).call().is_ok() {
            return Ok((child, Endpoint { base_url, api_key }));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("the local model took too long to load".into())
}

fn new_key() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Starts the helper on the first call of each request.
pub struct LocalClient;

impl llm::LlmClient for LocalClient {
    fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
        let endpoint = ensure_running().map_err(llm::LlmError::Provider)?;
        llm::OpenAIClient::new(endpoint.api_key, MODEL_ID).with_base_url(endpoint.base_url).complete(prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WHIMBREL_LLAMA_SERVER and WHIMBREL_LOCAL_MODEL point at a real helper and model.
    /// Run it alone: while it runs, the local link is in every chain in this process.
    #[test]
    #[ignore]
    fn a_real_helper_answers_and_stops() {
        let path = |k| PathBuf::from(std::env::var(k).unwrap());
        configure(Some((path("WHIMBREL_LLAMA_SERVER"), path("WHIMBREL_LOCAL_MODEL"))));
        let answer = llm::LlmClient::complete(&LocalClient, "Reply with the single word: ready");
        let endpoint = ensure_running();
        // First, so a failure doesn't leave the local link in other tests' chains.
        configure(None);
        let text = answer.unwrap().text;
        assert!(text.to_lowercase().contains("ready"), "{text}");
        assert!(!text.contains("<think>"), "{text}");
        let endpoint = endpoint.unwrap();
        assert!(ureq::get(&format!("{}/health", endpoint.base_url)).call().is_err());
    }

    #[test]
    fn keys_are_long_and_never_repeat() {
        assert_eq!(new_key().len(), 64);
        assert_ne!(new_key(), new_key());
    }

    #[test]
    fn a_missing_helper_is_an_error_not_a_hang() {
        let e = start(Path::new("/missing/whimbrel-llama-server"), Path::new("/missing/m.gguf")).unwrap_err();
        assert!(e.contains("didn't start"), "{e}");
    }

    /// A shell script stands in for the helper: it exits at once, like a bad model file.
    #[cfg(unix)]
    #[test]
    fn a_helper_that_exits_while_loading_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("helper");
        std::fs::write(&script, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = start(&script, Path::new("/missing/m.gguf")).unwrap_err();
        assert!(e.contains("stopped while loading"), "{e}");
    }
}
