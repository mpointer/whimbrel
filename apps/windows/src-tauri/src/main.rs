// Release builds are a GUI app with no console window; debug builds keep the console
// so `npm run tauri dev` shows the log.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The installed load check (src/link_check.rs) runs before anything else: no logging,
    // no profile, no window, and no single-instance hand-off to a running app.
    let args: Vec<String> = std::env::args().collect();
    if let Some(report) = whimbrel_windows::link_check::requested(&args) {
        std::process::exit(whimbrel_windows::link_check::run_from_args(report));
    }
    whimbrel_windows::app::run();
}
