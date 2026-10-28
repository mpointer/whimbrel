//! The MCP server sidecar's lifecycle (M3.4 Task 5; design: docs/superpowers/plans/
//! 2026-10-16-mcp-server-sidecar-windows.md).
//!
//! `whimbrel-mcp-server` is a separate process that opens the app's database and serves
//! it on `http://127.0.0.1:4319/mcp` to clients holding a token. This module starts and
//! stops it, and reports honestly what it is doing. Rules the design fixes:
//!
//! - Off by default; the caller starts it only when the user turned it on.
//! - Started **once**: a second `start` while one runs does nothing.
//! - Started only after the app's own database exists (two connections creating a
//!   brand-new database at once can fail; see `store_actor`'s tests).
//! - **Never restarted automatically.** A server that died shows as failed until the user
//!   switches it off and on.
//! - Killed with the app, even if the app is killed: the real spawner puts it in a Job
//!   Object (`job.rs`).
//!
//! Spawning goes through [`Spawner`], so the rules above are tested with a fake child.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The address clients use; the server binds this itself.
pub const ADDRESS: &str = "http://127.0.0.1:4319/mcp";
/// The sidecar's file name, next to the app when installed.
pub const EXE_NAME: &str = "whimbrel-mcp-server.exe";

/// What Settings shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Running,
    /// It could not start, or it stopped on its own. Why, in one line.
    Failed(String),
}

/// Everything the server is started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub exe: PathBuf,
    pub db: PathBuf,
    pub models: PathBuf,
    /// The local model for the one digest call it can make. Never a cloud model.
    pub ollama_model: String,
}

/// A started process.
pub trait Child: Send {
    /// `Some(exit code)` once it has exited.
    fn try_wait(&mut self) -> Result<Option<i32>, String>;
    fn kill(&mut self);
    /// The last line it wrote to stderr, which is where it explains a failure.
    fn last_stderr_line(&self) -> Option<String>;
}

pub trait Spawner: Send + Sync {
    fn spawn(&self, spec: &SpawnSpec) -> Result<Box<dyn Child>, String>;
}

enum State {
    Stopped,
    Running(Box<dyn Child>),
    Failed(String),
}

pub struct McpServer {
    spawner: Box<dyn Spawner>,
    state: Mutex<State>,
}

impl McpServer {
    pub fn new(spawner: Box<dyn Spawner>) -> Self {
        McpServer { spawner, state: Mutex::new(State::Stopped) }
    }

    /// Starts the server unless one is already running. Refuses when the database does not
    /// exist yet, because the sidecar must never be the first to create it.
    pub fn start(&self, spec: &SpawnSpec) -> Status {
        let mut state = self.state.lock().unwrap();
        Self::reap(&mut state);
        if matches!(*state, State::Running(_)) {
            return Status::Running;
        }
        if !spec.db.is_file() {
            *state = State::Failed("Whimbrel's database isn't open yet, so the server was not started.".into());
        } else {
            match self.spawner.spawn(spec) {
                Ok(child) => *state = State::Running(child),
                Err(e) => *state = State::Failed(format!("The MCP server did not start: {e}")),
            }
        }
        Self::describe(&state)
    }

    /// Stops it. A server that already failed goes back to stopped, so the toggle reads "off".
    pub fn stop(&self) -> Status {
        let mut state = self.state.lock().unwrap();
        if let State::Running(child) = &mut *state {
            child.kill();
        }
        *state = State::Stopped;
        Status::Stopped
    }

    /// What it is doing now. Notices an unexpected exit; never starts it again.
    pub fn status(&self) -> Status {
        let mut state = self.state.lock().unwrap();
        Self::reap(&mut state);
        Self::describe(&state)
    }

    fn reap(state: &mut State) {
        let State::Running(child) = state else { return };
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(code)) => {
                let why = match child.last_stderr_line() {
                    Some(line) => format!("The MCP server stopped (exit code {code}): {line}"),
                    None => format!("The MCP server stopped unexpectedly (exit code {code})."),
                };
                *state = State::Failed(why);
            }
            Err(e) => *state = State::Failed(format!("Could not check the MCP server: {e}")),
        }
    }

    fn describe(state: &State) -> Status {
        match state {
            State::Stopped => Status::Stopped,
            State::Running(_) => Status::Running,
            State::Failed(why) => Status::Failed(why.clone()),
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Where the sidecar program is: next to the app when installed, else `bin` in the data
/// folder, where `prepare-sidecar.ps1` stages it for a development build.
pub fn locate(app_dir: &Path, data_root: &Path) -> Result<PathBuf, String> {
    let installed = app_dir.join(EXE_NAME);
    if installed.is_file() {
        return Ok(installed);
    }
    let staged = data_root.join("bin").join(EXE_NAME);
    if staged.is_file() {
        return Ok(staged);
    }
    Err(format!("The MCP server program ({EXE_NAME}) isn't installed. Run apps\\windows\\scripts\\prepare-sidecar.ps1."))
}

// --- the real spawner ---------------------------------------------------------------------------

pub struct RealSpawner {
    job: crate::job::KillOnCloseJob,
}

impl RealSpawner {
    pub fn new() -> Result<Self, String> {
        Ok(RealSpawner { job: crate::job::KillOnCloseJob::new().map_err(|e| format!("could not create the job object: {e}"))? })
    }
}

struct RealChild {
    child: std::process::Child,
    last_line: std::sync::Arc<Mutex<Option<String>>>,
}

impl Child for RealChild {
    fn try_wait(&mut self) -> Result<Option<i32>, String> {
        self.child.try_wait().map(|s| s.map(|s| s.code().unwrap_or(-1))).map_err(|e| e.to_string())
    }
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    fn last_stderr_line(&self) -> Option<String> {
        self.last_line.lock().unwrap().clone()
    }
}

impl Spawner for RealSpawner {
    fn spawn(&self, spec: &SpawnSpec) -> Result<Box<dyn Child>, String> {
        use std::io::{BufRead, BufReader};
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = Command::new(&spec.exe)
            .arg("--db")
            .arg(&spec.db)
            .arg("--models")
            .arg(&spec.models)
            .env("WHIMBREL_OLLAMA_MODEL", &spec.ollama_model)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| e.to_string())?;
        // Assigned straight after spawning; if that fails the server must not run unguarded.
        if let Err(e) = self.job.assign(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("could not guard it with the job object: {e}"));
        }
        let last_line = std::sync::Arc::new(Mutex::new(None));
        if let Some(stderr) = child.stderr.take() {
            let sink = last_line.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        *sink.lock().unwrap() = Some(line);
                    }
                }
            });
        }
        Ok(Box::new(RealChild { child, last_line }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    /// A child the test controls: exits when told to.
    #[derive(Clone, Default)]
    struct Fake {
        exit: Arc<Mutex<Option<i32>>>,
        killed: Arc<AtomicUsize>,
        line: Arc<Mutex<Option<String>>>,
    }

    impl Child for Fake {
        fn try_wait(&mut self) -> Result<Option<i32>, String> {
            Ok(*self.exit.lock().unwrap())
        }
        fn kill(&mut self) {
            self.killed.fetch_add(1, Ordering::SeqCst);
        }
        fn last_stderr_line(&self) -> Option<String> {
            self.line.lock().unwrap().clone()
        }
    }

    struct FakeSpawner {
        spawned: Arc<AtomicUsize>,
        fail: Option<String>,
        child: Fake,
        specs: Arc<Mutex<Vec<SpawnSpec>>>,
    }

    impl Spawner for FakeSpawner {
        fn spawn(&self, spec: &SpawnSpec) -> Result<Box<dyn Child>, String> {
            self.spawned.fetch_add(1, Ordering::SeqCst);
            self.specs.lock().unwrap().push(spec.clone());
            match &self.fail {
                Some(e) => Err(e.clone()),
                None => Ok(Box::new(self.child.clone())),
            }
        }
    }

    struct Rig {
        server: McpServer,
        spawned: Arc<AtomicUsize>,
        child: Fake,
        specs: Arc<Mutex<Vec<SpawnSpec>>>,
        spec: SpawnSpec,
    }

    fn rig(fail: Option<&str>) -> Rig {
        let dir = std::env::temp_dir().join(format!("whimbrel-mcp-rig-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("core-memory.sqlite");
        std::fs::write(&db, b"").unwrap();
        let (spawned, child, specs) = (Arc::new(AtomicUsize::new(0)), Fake::default(), Arc::new(Mutex::new(Vec::new())));
        let spawner = FakeSpawner { spawned: spawned.clone(), fail: fail.map(str::to_string), child: child.clone(), specs: specs.clone() };
        Rig {
            server: McpServer::new(Box::new(spawner)),
            spawned,
            child,
            specs,
            spec: SpawnSpec { exe: dir.join(EXE_NAME), db, models: dir.join("models"), ollama_model: "gemma4:latest".into() },
        }
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    #[test]
    fn it_is_stopped_until_started() {
        let r = rig(None);
        assert_eq!(r.server.status(), Status::Stopped);
        assert_eq!(r.spawned.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn starting_runs_it_with_the_database_models_and_local_model() {
        let r = rig(None);
        assert_eq!(r.server.start(&r.spec), Status::Running);
        assert_eq!(r.server.status(), Status::Running);
        assert_eq!(r.specs.lock().unwrap()[0], r.spec);
    }

    #[test]
    fn a_second_start_does_not_start_a_second_server() {
        let r = rig(None);
        r.server.start(&r.spec);
        assert_eq!(r.server.start(&r.spec), Status::Running);
        assert_eq!(r.spawned.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stopping_kills_it_and_reads_stopped() {
        let r = rig(None);
        r.server.start(&r.spec);
        assert_eq!(r.server.stop(), Status::Stopped);
        assert_eq!(r.child.killed.load(Ordering::SeqCst), 1);
        assert_eq!(r.server.status(), Status::Stopped);
    }

    #[test]
    fn it_will_not_start_before_the_database_exists() {
        let r = rig(None);
        std::fs::remove_file(&r.spec.db).unwrap();
        let status = r.server.start(&r.spec);
        assert!(matches!(&status, Status::Failed(why) if why.contains("database isn't open")), "{status:?}");
        assert_eq!(r.spawned.load(Ordering::SeqCst), 0, "nothing was spawned");
    }

    #[test]
    fn a_program_that_cannot_start_says_why() {
        let r = rig(Some("program not found"));
        let status = r.server.start(&r.spec);
        assert_eq!(status, Status::Failed("The MCP server did not start: program not found".into()));
    }

    #[test]
    fn an_unexpected_exit_is_reported_with_its_last_line_and_is_not_restarted() {
        let r = rig(None);
        r.server.start(&r.spec);
        *r.child.line.lock().unwrap() = Some("port 4319 is already in use; another program is listening on 127.0.0.1:4319".into());
        *r.child.exit.lock().unwrap() = Some(1);
        let status = r.server.status();
        assert_eq!(
            status,
            Status::Failed("The MCP server stopped (exit code 1): port 4319 is already in use; another program is listening on 127.0.0.1:4319".into())
        );
        // Looking again, any number of times, never starts another.
        for _ in 0..5 {
            r.server.status();
        }
        assert_eq!(r.spawned.load(Ordering::SeqCst), 1, "no automatic restart");
    }

    #[test]
    fn an_exit_with_nothing_on_stderr_still_says_it_stopped() {
        let r = rig(None);
        r.server.start(&r.spec);
        *r.child.exit.lock().unwrap() = Some(-1);
        assert_eq!(r.server.status(), Status::Failed("The MCP server stopped unexpectedly (exit code -1).".into()));
    }

    #[test]
    fn after_a_failure_switching_off_then_on_starts_it_again() {
        let r = rig(None);
        r.server.start(&r.spec);
        *r.child.exit.lock().unwrap() = Some(1);
        assert!(matches!(r.server.status(), Status::Failed(_)));
        assert_eq!(r.server.stop(), Status::Stopped);
        *r.child.exit.lock().unwrap() = None;
        assert_eq!(r.server.start(&r.spec), Status::Running);
        assert_eq!(r.spawned.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn dropping_the_manager_stops_a_running_server() {
        let r = rig(None);
        r.server.start(&r.spec);
        let killed = r.child.killed.clone();
        drop(r.server);
        assert_eq!(killed.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_program_is_found_next_to_the_app_first_then_in_the_staging_folder() {
        let root = std::env::temp_dir().join(format!("whimbrel-mcp-locate-{}-{}", std::process::id(), rand_suffix()));
        let (app, data) = (root.join("app"), root.join("data"));
        let staged = data.join("bin");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&staged).unwrap();

        let err = locate(&app, &data).unwrap_err();
        assert!(err.contains("prepare-sidecar.ps1"), "{err}");

        std::fs::write(staged.join(EXE_NAME), b"x").unwrap();
        assert_eq!(locate(&app, &data).unwrap(), staged.join(EXE_NAME));

        std::fs::write(app.join(EXE_NAME), b"x").unwrap();
        assert_eq!(locate(&app, &data).unwrap(), app.join(EXE_NAME), "installed beside the app wins");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_real_spawner_starts_a_process_guards_it_and_reports_its_exit() {
        // cmd.exe stands in for the server: handed `--db ...` it exits straight away, which
        // exercises spawning, the job assignment and noticing an exit (any exit of a server is a failure).
        let cmd = PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| "C:/Windows/System32/cmd.exe".into()));
        let dir = std::env::temp_dir().join(format!("whimbrel-mcp-real-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("core-memory.sqlite");
        std::fs::write(&db, b"").unwrap();
        let server = McpServer::new(Box::new(RealSpawner::new().unwrap()));
        let spec = SpawnSpec { exe: cmd, db, models: dir.join("models"), ollama_model: "gemma4:latest".into() };
        server.start(&spec);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut status = server.status();
        while status == Status::Running && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
            status = server.status();
        }
        match status {
            Status::Failed(why) => assert!(why.contains("exit code"), "{why}"),
            other => panic!("expected the stand-in to exit with a failure, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_real_spawner_reports_a_missing_program() {
        let dir = std::env::temp_dir().join(format!("whimbrel-mcp-real2-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("core-memory.sqlite");
        std::fs::write(&db, b"").unwrap();
        let server = McpServer::new(Box::new(RealSpawner::new().unwrap()));
        let spec = SpawnSpec { exe: dir.join("nope.exe"), db, models: dir.join("models"), ollama_model: "m".into() };
        assert!(matches!(server.start(&spec), Status::Failed(why) if why.starts_with("The MCP server did not start:")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Real end to end against the built `whimbrel-mcp-server.exe` (run `prepare-sidecar.ps1`
    /// first). Ignored by default because it needs that program and port 4319 free:
    /// `cargo test --lib -- --ignored the_real_sidecar`.
    #[test]
    #[ignore = "needs the built sidecar and port 4319"]
    fn the_real_sidecar_enforces_tokens_revokes_at_once_and_reports_a_busy_port() {
        use std::io::{Read, Write};
        use std::net::TcpStream;

        let exe = std::env::var_os("WHIMBREL_SIDECAR_EXE")
            .map(PathBuf::from)
            .or_else(|| crate::paths::Paths::from_env().map(|p| p.root.join("bin").join(EXE_NAME)))
            .filter(|p| p.is_file())
            .expect("build the sidecar first: apps/windows/scripts/prepare-sidecar.ps1");
        let dir = std::env::temp_dir().join(format!("whimbrel-mcp-e2e-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("core-memory.sqlite");
        // The app's own connection stays open throughout, like the real app's.
        let store = memory::Store::open(&db, &dir.join("models")).unwrap();
        let good = crate::commands::mcp::create_client(&store, "e2e", &["memory".to_string()]).unwrap();

        let status_of = |token: Option<&str>| -> u16 {
            let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}"#;
            let auth = token.map(|t| format!("Authorization: Bearer {t}
")).unwrap_or_default();
            let req = format!(
                "POST /mcp HTTP/1.1
Host: 127.0.0.1:4319
Content-Type: application/json
Accept: application/json, text/event-stream
{auth}Content-Length: {}
Connection: close

{body}",
                body.len()
            );
            let mut stream = TcpStream::connect("127.0.0.1:4319").unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
            stream.write_all(req.as_bytes()).unwrap();
            let mut head = String::new();
            let mut buf = [0u8; 256];
            while !head.contains("
") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                head.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
        };

        let server = McpServer::new(Box::new(RealSpawner::new().unwrap()));
        let spec = SpawnSpec { exe, db, models: dir.join("models"), ollama_model: "gemma4:latest".into() };
        assert_eq!(server.start(&spec), Status::Running);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while TcpStream::connect("127.0.0.1:4319").is_err() {
            assert!(std::time::Instant::now() < deadline, "the server never started listening: {:?}", server.status());
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        assert_eq!(status_of(None), 401, "no token");
        assert_eq!(status_of(Some("kmcp_1.0000000000000000")), 401, "a wrong token");
        let ok = status_of(Some(&good.token));
        assert!((200..300).contains(&ok), "a valid token should be let in, got {ok}");

        store.revoke_mcp_client_token(good.id).unwrap();
        assert_eq!(status_of(Some(&good.token)), 401, "revoking takes effect on the very next request");

        // Port already taken: a second server cannot bind, and says so.
        let second = McpServer::new(Box::new(RealSpawner::new().unwrap()));
        second.start(&spec);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut busy = second.status();
        while busy == Status::Running && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(100));
            busy = second.status();
        }
        match busy {
            Status::Failed(why) => assert!(why.contains("already in use"), "{why}"),
            other => panic!("expected the second server to fail on the busy port, got {other:?}"),
        }

        // Stopping really frees the port.
        server.stop();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while TcpStream::connect("127.0.0.1:4319").is_ok() {
            assert!(std::time::Instant::now() < deadline, "the port is still taken after stop");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
