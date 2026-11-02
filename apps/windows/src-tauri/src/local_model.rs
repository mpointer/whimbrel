//! The small local model behind Ask with nothing set up (Release 1 item 1.3, W-1.3). Core's
//! `api::local_model` runs the helper; this finds the files, tells core when they're there, and
//! says whether this PC can run it. The Mac's `LocalModel`.

use std::path::{Path, PathBuf};

use crate::paths::Paths;

const HELPER: &str = "whimbrel-llama-server.exe";
/// The Mac's limits: below 8 GB the model and the app's own memory don't fit comfortably.
pub const MINIMUM_MEMORY: u64 = 8 << 30;
/// The model's 1.9 GB and room to spare.
pub const MINIMUM_FREE_DISK: u64 = 4_000_000_000;

/// The helper beside the app (`llama\` in an install), else `WHIMBREL_LLAMA_SERVER` in
/// development, as core's real-helper test uses.
pub fn helper_path() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.parent()?.join("llama").join(HELPER);
    if beside.is_file() {
        return Some(beside);
    }
    std::env::var_os("WHIMBREL_LLAMA_SERVER").map(PathBuf::from).filter(|p| p.is_file())
}

pub fn is_installed(paths: &Paths) -> bool {
    helper_path().is_some() && paths.local_model().is_file()
}

pub fn can_offer(helper_bundled: bool, memory: u64, free_disk: u64) -> bool {
    helper_bundled && memory >= MINIMUM_MEMORY && free_disk >= MINIMUM_FREE_DISK
}

/// Whether to show the download button on this PC.
pub fn can_offer_here(paths: &Paths) -> bool {
    can_offer(helper_path().is_some(), physical_memory(), free_disk(&paths.models))
}

/// Run at launch and after a download or a removal. Stops a helper a crash left behind first.
pub fn configure(paths: &Paths) {
    sweep();
    let found = helper_path().filter(|_| paths.local_model().is_file()).map(|helper| (helper, paths.local_model()));
    tracing::info!(installed = found.is_some(), "local model");
    api::local_model::configure(found);
}

/// Removes the model; its chain link goes and a running helper stops.
pub fn remove(paths: &Paths) -> std::io::Result<()> {
    api::local_model::configure(None);
    match std::fs::remove_file(paths.local_model()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// By image name, which only this app's helper has. `/F`: a console program ignores a plain close.
fn sweep() {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", HELPER])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status();
}

fn physical_memory() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    match unsafe { GlobalMemoryStatusEx(&mut status) } {
        Ok(()) => status.ullTotalPhys,
        Err(_) => 0,
    }
}

/// Free space for this user on the drive holding `dir`; 0 when it can't be read.
fn free_disk(dir: &Path) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let probe = dir.ancestors().find(|p| p.exists()).unwrap_or(dir);
    let wide: Vec<u16> = probe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    match unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut available), None, None) } {
        Ok(()) => available,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_needs_the_helper_eight_gigabytes_and_room_on_disk() {
        let gb = 1u64 << 30;
        assert!(can_offer(true, 8 * gb, 4_000_000_000));
        assert!(!can_offer(false, 64 * gb, 100_000_000_000), "no helper in this build");
        assert!(!can_offer(true, 8 * gb - 1, 100_000_000_000));
        assert!(!can_offer(true, 16 * gb, 3_999_999_999));
    }

    /// Core's `local` streaming branch, end to end the way Ask streams: WHIMBREL_LLAMA_SERVER and
    /// WHIMBREL_LOCAL_MODEL point at a real helper and model. Run it alone, as core's own one.
    #[test]
    #[ignore]
    fn a_real_local_model_streams_an_answer() {
        let path = |k| PathBuf::from(std::env::var(k).unwrap());
        api::local_model::configure(Some((path("WHIMBREL_LLAMA_SERVER"), path("WHIMBREL_LOCAL_MODEL"))));
        let deltas = std::cell::Cell::new(0usize);
        let end = api::inference::start("Reply with the single word: ready", api::local_model::MODEL_ID, "local", None, None).map(|run| {
            let end = crate::chat::forward(
                &run.run_id,
                api::inference::poll,
                &|event| {
                    if matches!(event, crate::chat::ChatEvent::Delta { .. }) {
                        deltas.set(deltas.get() + 1);
                    }
                },
                || std::thread::sleep(crate::chat::POLL_INTERVAL),
            );
            api::inference::release(&run.run_id);
            end
        });
        api::local_model::configure(None);
        let end = end.expect("the local run started");
        assert!(end.produced_output && deltas.get() > 0, "{:?}", end.outcome);
        assert!(end.text.to_lowercase().contains("ready"), "{}", end.text);
    }

    #[test]
    fn this_pc_reports_its_memory_and_free_space() {
        assert!(physical_memory() > 1 << 30);
        assert!(free_disk(&std::env::temp_dir()) > 0);
        assert!(free_disk(&std::env::temp_dir().join("no-such-folder").join("deeper")) > 0, "a missing folder reads its drive");
    }
}
