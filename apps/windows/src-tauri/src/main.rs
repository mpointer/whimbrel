// Release builds are a GUI app with no console window; debug builds keep the console
// so `npm run tauri dev` shows the log.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    whimbrel_windows::app::run();
}
