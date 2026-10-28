//! Crash restart (plan Task 10; docs/12 open question 4).
//!
//! Release builds use `panic = "abort"`, and `install_panic_log`'s hook ends the process
//! on every panic. A small supervisor brings it back: a release launch re-runs itself as
//! `whimbrel-windows.exe --supervise`, which starts the app as a child and relaunches it
//! when it exits with anything but 0 (Quit) or `RESTART_REQUESTED`.
//!
//! Why a supervisor rather than a Scheduled Task with restart-on-failure: Task Scheduler
//! restarts a task that fails to start, not a program that exits nonzero. A probe task
//! whose action exits 1 ran once in 160 s despite a 1-minute restart interval.

use std::collections::VecDeque;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::state::AppState;

pub const SUPERVISE_ARG: &str = "--supervise";
/// Set on the supervised child, so it runs the app instead of supervising again.
pub const SUPERVISED_ENV: &str = "WHIMBREL_SUPERVISED";
/// The app exits with this to be relaunched at once (e.g. to load a new search model).
pub const RESTART_REQUESTED: i32 = 75;
/// A panic's exit code (Rust's own convention).
pub const PANIC_EXIT: u32 = 101;
/// The relaunch delay after the 1st to 5th crash inside `CRASH_WINDOW`; a 6th gives up.
const BACKOFF: [Duration; 5] =
    [Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(10), Duration::from_secs(30)];
const CRASH_WINDOW: Duration = Duration::from_secs(600);

/// The environment variable that arms a crash test: `actor` panics inside a store job,
/// `main` panics on the main (UI) thread, and `restart` asks for a supervised restart
/// (`RESTART_REQUESTED`). Each fires 5 s after launch. Test-only; an ordinary launch
/// never sets it.
pub const CRASH_TEST_ENV: &str = "WHIMBREL_CRASH_TEST";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Exit,
    Relaunch(Duration),
}

/// What the supervisor does when the app exits with `code`. `crashes` holds the times of
/// recent crashes and is updated.
pub fn next_step(code: i32, crashes: &mut VecDeque<Instant>, now: Instant) -> Next {
    match code {
        0 => Next::Exit,
        RESTART_REQUESTED => Next::Relaunch(Duration::ZERO),
        _ => {
            while crashes.front().is_some_and(|t| now.duration_since(*t) > CRASH_WINDOW) {
                crashes.pop_front();
            }
            crashes.push_back(now);
            BACKOFF.get(crashes.len() - 1).map_or(Next::Exit, |d| Next::Relaunch(*d))
        }
    }
}

pub fn is_supervised() -> bool {
    std::env::var_os(SUPERVISED_ENV).is_some()
}

/// Drops `SUPERVISED_ENV` from this process, so a program it starts doesn't inherit it,
/// and returns whether it was set. The update's installer relaunches the app with the
/// environment it was given; with the variable set, that app would run unsupervised.
pub fn leave_supervision() -> bool {
    let was = is_supervised();
    std::env::remove_var(SUPERVISED_ENV);
    was
}

/// Undoes `leave_supervision` when the program it was for never started.
pub fn rejoin_supervision() {
    std::env::set_var(SUPERVISED_ENV, "1");
}

static REQUESTED_EXIT: AtomicI32 = AtomicI32::new(0);

/// Exits the app with `code`. Tauri's event loop always ends the process with 0 whatever
/// `AppHandle::exit` was given, so the code is kept here, and `app::run` exits with it
/// once Tauri reports the exit.
pub fn exit_with(app: &AppHandle, code: i32) {
    REQUESTED_EXIT.store(code, Ordering::SeqCst);
    app.exit(code);
}

pub fn requested_exit_code() -> i32 {
    REQUESTED_EXIT.load(Ordering::SeqCst)
}

/// Called first at launch, before logging starts. In a release build that is not already
/// the supervised child, it never returns: `--supervise` runs the supervisor, and a plain
/// launch (Start menu, the Run entry, a double-click) starts one and steps aside. Debug
/// builds run unsupervised.
pub fn supervise_or_continue(logs_dir: &Path) {
    if cfg!(debug_assertions) || is_supervised() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if std::env::args().any(|a| a == SUPERVISE_ARG) {
        std::process::exit(supervise(&exe, logs_dir));
    }
    match Command::new(&exe).arg(SUPERVISE_ARG).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => note(logs_dir, &format!("could not start the supervisor ({e}); running unsupervised")),
    }
}

fn supervise(exe: &Path, logs_dir: &Path) -> i32 {
    let mut crashes = VecDeque::new();
    let mut first = true;
    loop {
        let started = Instant::now();
        let mut child = Command::new(exe);
        child.env(SUPERVISED_ENV, "1").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if !first {
            // A crash test crashes the first child only, not every relaunch.
            child.env_remove(CRASH_TEST_ENV);
        }
        first = false;
        let code = match child.status() {
            Ok(status) => status.code().unwrap_or(-1),
            Err(e) => {
                note(logs_dir, &format!("could not start Whimbrel: {e}"));
                return 1;
            }
        };
        let ran = started.elapsed().as_secs();
        match next_step(code, &mut crashes, Instant::now()) {
            Next::Exit if code == 0 => return 0,
            Next::Exit => {
                note(logs_dir, &format!("Whimbrel exited with code {code} (0x{:08X}) after {ran} s: {} crashes in 10 minutes, so not relaunching", code as u32, crashes.len()));
                return code;
            }
            Next::Relaunch(delay) => {
                note(logs_dir, &format!("Whimbrel exited with code {code} (0x{:08X}) after {ran} s; relaunching in {} s", code as u32, delay.as_secs()));
                std::thread::sleep(delay);
            }
        }
    }
}

/// One line in `logs\supervisor.log`. The supervisor keeps its own file rather than
/// sharing the app's rolling log with a second process.
fn note(logs_dir: &Path, message: &str) {
    let line = format!("{} {message}\n", chrono::Local::now().to_rfc3339());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(logs_dir.join("supervisor.log")) {
        let _ = std::io::Write::write_all(&mut file, line.as_bytes());
    }
}

/// Appends each panic's message, location and thread to `logs\crash.log`, synchronously,
/// then ends the process with `PANIC_EXIT` (release builds). The regular log can't be
/// trusted with the reason: its writer flushes on a background thread. And `panic =
/// "abort"` alone doesn't reliably end the process: a panic on the main (UI) thread left
/// it running, hung, in Task 10's crash test.
pub fn install_panic_log(logs_dir: &Path) {
    let path = logs_dir.join("crash.log");
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let line = format!("{} panic on thread '{}': {info}\n", chrono::Local::now().to_rfc3339(), thread.name().unwrap_or("unnamed"));
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = std::io::Write::write_all(&mut file, line.as_bytes());
        }
        default(info);
        if cfg!(panic = "abort") {
            use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
            unsafe {
                let _ = TerminateProcess(GetCurrentProcess(), PANIC_EXIT);
            }
        }
    }));
}

pub fn schedule_crash_test(app: &AppHandle) {
    let Ok(mode) = std::env::var(CRASH_TEST_ENV) else { return };
    tracing::warn!(mode = %mode, "crash test armed; panicking in 5 s");
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        match mode.as_str() {
            "actor" => {
                if let Some(store) = &app.state::<AppState>().store {
                    let _ = store.call_blocking(|_| -> () { panic!("crash test: a store job panicked") });
                }
            }
            "main" => {
                let _ = app.run_on_main_thread(|| panic!("crash test: the main thread panicked"));
            }
            "restart" => exit_with(&app, RESTART_REQUESTED),
            other => tracing::warn!(mode = %other, "unknown crash test; expected `actor`, `main` or `restart`"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_started_after_leaving_supervision_does_not_inherit_it() {
        rejoin_supervision();
        assert!(leave_supervision(), "it was set");
        let child = Command::new("cmd").args(["/C", "if defined WHIMBREL_SUPERVISED (exit 1) else (exit 0)"]).status().unwrap();
        assert!(child.success(), "the child saw the variable");
        assert!(!leave_supervision(), "leaving twice finds it gone");
        rejoin_supervision();
        assert!(is_supervised());
        std::env::remove_var(SUPERVISED_ENV);
    }

    #[test]
    fn quit_ends_supervision_and_a_requested_restart_is_immediate() {
        let mut crashes = VecDeque::new();
        let now = Instant::now();
        assert_eq!(next_step(0, &mut crashes, now), Next::Exit);
        assert_eq!(next_step(RESTART_REQUESTED, &mut crashes, now), Next::Relaunch(Duration::ZERO));
        assert!(crashes.is_empty(), "neither counts as a crash");
    }

    #[test]
    fn crashes_back_off_and_a_sixth_within_ten_minutes_gives_up() {
        let mut crashes = VecDeque::new();
        let t0 = Instant::now();
        let delays: Vec<Next> = (0..6).map(|i| next_step(PANIC_EXIT as i32, &mut crashes, t0 + Duration::from_secs(i * 60))).collect();
        assert_eq!(
            delays,
            [1, 2, 5, 10, 30].map(|s| Next::Relaunch(Duration::from_secs(s))).into_iter().chain([Next::Exit]).collect::<Vec<_>>()
        );
    }

    #[test]
    fn old_crashes_age_out_of_the_window() {
        let mut crashes = VecDeque::new();
        let t0 = Instant::now();
        for i in 0..5 {
            next_step(-1073740791, &mut crashes, t0 + Duration::from_secs(i));
        }
        // Eleven minutes later the earlier five no longer count.
        assert_eq!(next_step(1, &mut crashes, t0 + Duration::from_secs(660)), Next::Relaunch(Duration::from_secs(1)));
    }
}
