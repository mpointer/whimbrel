//! The report the supervisor offers when it stops relaunching (2026-11-08, Mike's OK).
//!
//! An app that dies a few seconds after launch can't reach Settings → Send feedback. When the
//! supervisor gives up (six crashes in ten minutes), it saves the same kind of diagnostics zip
//! itself and asks, in a plain Windows message box, whether to email it. The zip holds what the
//! supervisor can gather without the app: a summary (version, Windows build, the PC's processor,
//! the exit code), the recent logs, and Windows' own crash records for Whimbrel (which name the
//! faulting module, as `vulkan_dzn.dll` did). Every line is redacted as in Send feedback.

use std::path::{Path, PathBuf};

use crate::diagnostics;
use crate::l10n::tr;

/// What the supervisor knows about the crashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashFacts {
    pub version: String,
    pub windows: String,
    pub machine: String,
    pub exit_code: i32,
    pub crashes: usize,
    /// How long the last run lasted, in seconds.
    pub last_ran_s: u64,
}

pub fn summary(f: &CrashFacts) -> String {
    format!(
        "Whimbrel {} stopped relaunching after {} crashes in 10 minutes.\n\
         Last exit code: {} (0x{:08X}) after {} s.\n\
         Windows: {}\nProcessor: {}\n",
        f.version, f.crashes, f.exit_code, f.exit_code as u32, f.last_ran_s, f.windows, f.machine
    )
}

/// The events in `wevtutil`'s text output that are about Whimbrel.
pub fn whimbrel_events(text: &str) -> String {
    text.split("Event[")
        .filter(|e| e.to_ascii_lowercase().contains("whimbrel"))
        .map(|e| format!("Event[{}", e.trim_end()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Windows' crash records from the last hour (Application Error and Windows Error Reporting),
/// newest first. Empty if `wevtutil` can't be run.
fn windows_events() -> String {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let query = "*[System[(Provider[@Name='Application Error'] or Provider[@Name='Windows Error Reporting']) and TimeCreated[timediff(@SystemTime) <= 3600000]]]";
    match std::process::Command::new("wevtutil")
        .args(["qe", "Application", &format!("/q:{query}"), "/c:20", "/rd:true", "/f:text"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(out) => whimbrel_events(&String::from_utf8_lossy(&out.stdout)),
        Err(e) => format!("could not read the Windows event log: {e}"),
    }
}

/// Saves the zip beside Send feedback's (same name pattern, so the same three are kept).
pub fn build(logs_dir: &Path, out_dir: &Path, facts: &CrashFacts, events: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut entries = vec![("crash-summary.txt".to_string(), summary(facts)), ("windows-events.txt".to_string(), events.to_string())];
    for log in diagnostics::log_files(logs_dir) {
        let name = log.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match diagnostics::tail(&log, diagnostics::MAX_LOG_BYTES) {
            Ok(text) => entries.push((format!("logs/{name}"), text)),
            Err(e) => entries.push((format!("logs/{name}.unreadable.txt"), e.to_string())),
        }
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = out_dir.join(format!("Whimbrel-diagnostics-{stamp}.zip"));
    diagnostics::write_zip(&path, &entries)?;
    diagnostics::prune(out_dir, diagnostics::KEEP_BUNDLES);
    Ok(path)
}

/// Saves the report, then asks whether to email it. Yes shows it in File Explorer and opens the
/// feedback email, as Send feedback does. Blocks the supervisor until answered; it is exiting.
pub fn offer(logs_dir: &Path, exit_code: i32, crashes: usize, last_ran_s: u64) {
    let root = logs_dir.parent().unwrap_or(logs_dir);
    // The supervisor never set a language: use the app's choice, as the app would.
    let settings = crate::settings::SettingsFile::load(&root.join("settings.json"));
    crate::l10n::apply_ui_language(&settings.app_language());
    let facts = CrashFacts {
        version: env!("CARGO_PKG_VERSION").to_string(),
        windows: crate::feedback::this_windows(),
        machine: crate::gpu::native_machine().to_string(),
        exit_code,
        crashes,
        last_ran_s,
    };
    let saved = build(logs_dir, &root.join("diagnostics"), &facts, &windows_events());
    let (text, path) = match saved {
        Ok(path) => (
            tr(
                "Whimbrel closed unexpectedly {n} times, so it won't restart on its own. A report with its recent logs was saved (email addresses and meeting titles removed).\n\nEmail the report to Whimbrel now?",
                &[("n", &crashes.to_string())],
            ),
            Some(path),
        ),
        Err(e) => (
            tr("Whimbrel closed unexpectedly {n} times, so it won't restart on its own. It couldn't save a report: {error}\n\nEmail Whimbrel about it now?", &[
                ("n", &crashes.to_string()),
                ("error", &e),
            ]),
            None,
        ),
    };
    if !ask(&text) {
        return;
    }
    let line = match &path {
        Some(path) => {
            if let Err(e) = diagnostics::reveal(path) {
                note(logs_dir, &format!("could not show the crash report in File Explorer: {e}"));
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            tr(
                "Please attach {file}, the diagnostics file Whimbrel just saved (File Explorer has it selected). It has recent logs and settings status, with email addresses and meeting titles removed.",
                &[("file", &name)],
            )
        }
        None => String::new(),
    };
    if let Err(e) = crate::feedback::open((!line.is_empty()).then_some(line.as_str())) {
        note(logs_dir, &format!("could not open the feedback email: {e}"));
    }
}

fn note(logs_dir: &Path, message: &str) {
    let line = format!("{} {message}\n", chrono::Local::now().to_rfc3339());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(logs_dir.join("supervisor.log")) {
        let _ = std::io::Write::write_all(&mut file, line.as_bytes());
    }
}

/// A Yes/No message box; true for Yes.
fn ask(text: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONWARNING, MB_SETFOREGROUND, MB_YESNO};
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Whimbrel"), MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND) == IDYES }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> CrashFacts {
        CrashFacts { version: "0.9.61".into(), windows: "Windows 11 24H2 (26100.1)".into(), machine: "arm64".into(), exit_code: -1073741819, crashes: 6, last_ran_s: 5 }
    }

    #[test]
    fn the_summary_names_the_code_in_hex_and_the_processor() {
        let s = summary(&facts());
        assert!(s.contains("0xC0000005") && s.contains("after 5 s") && s.contains("Processor: arm64") && s.contains("6 crashes"), "{s}");
    }

    #[test]
    fn only_whimbrels_events_are_kept() {
        let text = "Event[0]:\n  Source: Application Error\n  Description: Faulting application name: whimbrel-windows.exe\n  Faulting module name: vulkan_dzn.dll\n\
                    Event[1]:\n  Source: Application Error\n  Description: Faulting application name: other.exe\n";
        let kept = whimbrel_events(text);
        assert!(kept.contains("vulkan_dzn.dll") && !kept.contains("other.exe"), "{kept}");
        assert_eq!(whimbrel_events("Event[0]:\n other.exe\n"), "");
    }

    #[test]
    fn the_report_zips_the_summary_events_and_logs_redacted() {
        let root = std::env::temp_dir().join(format!("whimbrel-crash-report-{}", std::process::id()));
        let logs = root.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("supervisor.log"), "exited with code -1073741819\n").unwrap();
        std::fs::write(logs.join("whimbrel.2026-11-08.log"), "INFO sync account=joe@example.com\n").unwrap();
        let path = build(&logs, &root.join("diagnostics"), &facts(), "Event[0]: whimbrel-windows.exe vulkan_dzn.dll").unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let names: Vec<String> = (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().unwrap().to_string()).collect();
        assert_eq!(names, ["crash-summary.txt", "windows-events.txt", "logs/whimbrel.2026-11-08.log", "logs/supervisor.log"]);
        let mut log = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("logs/whimbrel.2026-11-08.log").unwrap(), &mut log).unwrap();
        assert!(!log.contains("joe@example.com"), "{log}");
        drop(zip);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
