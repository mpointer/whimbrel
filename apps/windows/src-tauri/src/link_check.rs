//! The installed load check: `whimbrel-windows.exe --link-check <report-file>`.
//!
//! It proves the native libraries linked and load on this machine (the Arm smoke test runs it
//! on a Windows on Arm runner): core's store in memory (bundled SQLite and sqlite-vec), the
//! model manifest, whisper.cpp, and on x64 sherpa-onnx/ONNX Runtime. It touches no profile
//! data, starts no window and no Tauri runtime, and writes its report to a file because the
//! release app is a GUI-subsystem program with no console.
//!
//! The capture-spike example (`cargo run --example capture-spike -- link-check`) prints the
//! same report.

use std::path::{Path, PathBuf};

/// The hidden argument the app's `main` looks for first.
pub const FLAG: &str = "--link-check";

/// The report file when the arguments ask for the link check (`--link-check <file>` as the
/// first argument after the program), else `None`. A bare `--link-check` with no file is
/// also a link check, reported to `Err` by [`run_from_args`].
pub fn requested(args: &[String]) -> Option<Option<PathBuf>> {
    match args.get(1) {
        Some(a) if a == FLAG => Some(args.get(2).map(PathBuf::from)),
        _ => None,
    }
}

/// Runs the checks; `Ok` holds the report, `Err` the report so far and what failed.
pub fn link_check() -> Result<String, String> {
    let mut out = String::new();
    match memory::Store::open_in_memory() {
        Ok(store) => {
            drop(store);
            out.push_str("memory:     rusqlite bundled SQLite + sqlite-vec OK\n");
        }
        Err(e) => return Err(format!("{out}memory:     in-memory store failed: {e}\n")),
    }

    let specs = api::models::manifest(&api::models::ModelDestinations {
        whisper_model_path: PathBuf::from("w"),
        segmentation_model_path: PathBuf::from("s"),
        embedding_model_path: PathBuf::from("e"),
        whisper_multilingual_path: None,
    });
    if specs.is_empty() {
        return Err(format!("{out}api:        the model manifest is empty\n"));
    }
    out.push_str(&format!("api:        {} model specs\n", specs.len()));

    // Nonexistent paths: these fail at runtime, but reaching the call proves whisper.cpp
    // and sherpa-onnx/ONNX Runtime linked.
    let missing = Path::new("does-not-exist.bin");
    match transcribe::Transcriber::load(missing) {
        Ok(_) => out.push_str("transcribe: unexpectedly loaded\n"),
        Err(e) => out.push_str(&format!("transcribe: linked (expected load error: {e})\n")),
    }
    #[cfg(target_arch = "x86_64")]
    {
        let paths = diarize::DiarizationModelPaths {
            segmentation_model_path: missing.into(),
            embedding_model_path: missing.into(),
        };
        match diarize::diarize(missing, &paths, None) {
            Ok(_) => out.push_str("diarize:    unexpectedly ran\n"),
            Err(e) => out.push_str(&format!("diarize:    linked (expected error: {e})\n")),
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    out.push_str("diarize:    not in this build\n");

    let hwnd = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    out.push_str(&format!("windows:    foreground HWND {:?}\n", hwnd.0));
    Ok(out)
}

/// The app's `--link-check` path: writes the report to the file and returns the exit code
/// (0 when every check passed and the report was written).
pub fn run_from_args(report: Option<PathBuf>) -> i32 {
    let Some(path) = report else { return 2 };
    let (text, code) = match link_check() {
        Ok(t) => (format!("{t}result:     ok\n"), 0),
        Err(t) => (format!("{t}result:     FAILED\n"), 1),
    };
    match std::fs::write(&path, text) {
        Ok(()) => code,
        Err(_) => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_a_leading_flag_asks_for_the_check() {
        assert_eq!(requested(&args(&["app.exe", "--link-check", "r.txt"])), Some(Some(PathBuf::from("r.txt"))));
        assert_eq!(requested(&args(&["app.exe", "--link-check"])), Some(None));
        assert_eq!(requested(&args(&["app.exe"])), None);
        assert_eq!(requested(&args(&["app.exe", "--autostart"])), None);
        // Not the first argument: a normal start (the single-instance plugin forwards argv).
        assert_eq!(requested(&args(&["app.exe", "--autostart", "--link-check", "r.txt"])), None);
        assert_eq!(requested(&args(&["app.exe", "--link-checker", "r.txt"])), None);
    }

    #[test]
    fn the_check_passes_and_writes_its_report() {
        let dir = std::env::temp_dir().join(format!("whimbrel-link-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let report = dir.join("report.txt");
        assert_eq!(run_from_args(Some(report.clone())), 0);
        let text = std::fs::read_to_string(&report).unwrap();
        assert!(text.contains("sqlite-vec OK"), "{text}");
        assert!(text.contains("model specs"), "{text}");
        assert!(text.ends_with("result:     ok\n"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_report_file_is_a_usage_error() {
        assert_eq!(run_from_args(None), 2);
    }
}
