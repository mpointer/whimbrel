//! Performance in the log (plan 2026-11-11, phase 1). When a PC is slow, the app log should say why.
//! Joe's laptop slowed down after installing Whimbrel (2026-11-10), and his log couldn't tell local AI
//! from transcription, capture or the database.
//!
//! - **At launch, one `this PC` line:** memory, cores, battery and power, and what Ollama has
//!   installed and loaded.
//! - **Every 5 minutes, one `resource use` line:** free memory, CPU and memory of Whimbrel's own
//!   process tree and of Ollama's, Ollama's loaded models, power, and time since the user's last
//!   input. CPU is the average over the 5 minutes, from two `CpuSnapshot`s, so nothing blocks.
//!
//! Only Whimbrel's processes and Ollama's are measured or named. No content and no names of the
//! user's other programs ever reach these lines (users email this log, through Send feedback's
//! diagnostics bundle too). Ollama's model names are public model names.

use std::collections::HashSet;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde::Deserialize;

use crate::capture::proc_stats::{self, CpuSnapshot, ProcInfo};

/// How often the `resource use` line is written.
pub const EVERY: Duration = Duration::from_secs(5 * 60);
const OLLAMA: &str = "127.0.0.1:11434";
const OLLAMA_CONNECT: Duration = Duration::from_millis(300);
const OLLAMA_READ: Duration = Duration::from_secs(2);
/// The executables of an Ollama install: the server (and, in newer versions, its runners), the tray
/// app that starts it, and the older runner.
const OLLAMA_EXES: [&str; 3] = ["ollama.exe", "ollama app.exe", "ollama_llama_server.exe"];

/// Physical memory, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryStatus {
    pub total: u64,
    pub available: u64,
}

/// `GlobalMemoryStatusEx`; `None` if Windows won't say.
pub fn memory_status() -> Option<MemoryStatus> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(MemoryStatus { total: status.ullTotalPhys, available: status.ullAvailPhys })
}

/// Whether the PC has a battery: `BatteryFlag` 128 is "no system battery", 255 "unknown".
pub fn has_battery() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return false;
    }
    battery_present(status.BatteryFlag)
}

pub fn battery_present(flag: u8) -> bool {
    flag != 128 && flag != 255
}

/// One Ollama model with its size: on disk for `/api/tags`, in memory for `/api/ps`, where
/// `size_vram` is the part held in GPU memory.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub size_vram: u64,
}

#[derive(Deserialize)]
struct ModelList {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

/// What Ollama has: `None` when it isn't listening.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OllamaState {
    pub installed: Vec<OllamaModel>,
    pub loaded: Vec<OllamaModel>,
}

/// Reads `/api/tags` and `/api/ps`. A 300 ms TCP probe first, because a Windows connect to a closed
/// port retries for seconds (as in `jobs::OllamaProbe`).
pub fn ollama_state() -> Option<OllamaState> {
    let addr: SocketAddr = OLLAMA.parse().ok()?;
    TcpStream::connect_timeout(&addr, OLLAMA_CONNECT).ok()?;
    let agent = ureq::AgentBuilder::new().timeout_connect(OLLAMA_CONNECT).timeout(OLLAMA_READ).build();
    let read = |path: &str| -> Vec<OllamaModel> {
        agent
            .get(&format!("http://{OLLAMA}{path}"))
            .call()
            .ok()
            .and_then(|r| r.into_json::<ModelList>().ok())
            .map(|l| l.models)
            .unwrap_or_default()
    };
    Some(OllamaState { installed: read("/api/tags"), loaded: read("/api/ps") })
}

/// Bytes as GB with one decimal: `8.0`.
pub fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1e9)
}

/// Bytes as whole MB.
pub fn mb(bytes: u64) -> u64 {
    bytes / 1_000_000
}

/// `gemma4:12b 8.0 GB, qwen3:8b 5.2 GB`, or `none`.
pub fn installed_summary(models: &[OllamaModel]) -> String {
    if models.is_empty() {
        return "none".into();
    }
    models.iter().map(|m| format!("{} {} GB", m.name, gb(m.size))).collect::<Vec<_>>().join(", ")
}

/// `gemma4:12b 9.6 GB (GPU 9.6 GB)`, or `none`.
pub fn loaded_summary(models: &[OllamaModel]) -> String {
    if models.is_empty() {
        return "none".into();
    }
    models.iter().map(|m| format!("{} {} GB (GPU {} GB)", m.name, gb(m.size), gb(m.size_vram))).collect::<Vec<_>>().join(", ")
}

/// Whether `model` is among `installed`. Ollama lists `name:tag`, and a bare name means `:latest`.
pub fn is_installed(model: &str, installed: &[OllamaModel]) -> bool {
    installed.iter().any(|m| crate::chat::same_ollama_model(&m.name, model))
}

/// Ollama's processes: every tree rooted at one of [`OLLAMA_EXES`]. Nothing else is picked out of
/// `all`, so no other program's name or numbers can reach the log.
pub fn ollama_pids(all: &[ProcInfo]) -> Vec<u32> {
    let mut pids = HashSet::new();
    for exe in OLLAMA_EXES {
        for root in proc_stats::tree_roots_named(exe, all) {
            pids.extend(proc_stats::process_tree(root, all));
        }
    }
    let mut pids: Vec<u32> = pids.into_iter().collect();
    pids.sort_unstable();
    pids
}

/// This process and everything it started: the local model's server, the MCP server, connectors.
pub fn whimbrel_pids(all: &[ProcInfo]) -> Vec<u32> {
    proc_stats::process_tree(std::process::id(), all)
}

/// Memory summed over `pids`: private bytes and working set, and how many of `pids` Windows let us
/// read. Both measures are logged, because a model's weights that Ollama memory-maps count in the
/// working set, not in private bytes. The count keeps "unreadable" from reading as 0 MB: an Ollama
/// started by another account or elevated (as on a PC that serves it to the LAN) can't be opened.
fn memory_of(pids: &[u32]) -> (proc_stats::Memory, usize) {
    let read: Vec<proc_stats::Memory> = pids.iter().filter_map(|&p| proc_stats::memory(p)).collect();
    let total = read.iter().fold(proc_stats::Memory::default(), |a, m| proc_stats::Memory {
        working_set_bytes: a.working_set_bytes + m.working_set_bytes,
        private_bytes: a.private_bytes + m.private_bytes,
    });
    (total, read.len())
}

/// The launch line.
pub fn log_this_pc(ollama_model: &str) {
    let memory = memory_status();
    let ollama = ollama_state();
    tracing::info!(
        ram_gb = %memory.map_or("?".into(), |m| gb(m.total)),
        free_ram_gb = %memory.map_or("?".into(), |m| gb(m.available)),
        logical_cores = proc_stats::logical_cpus(),
        performance_cores = crate::meetings::perf::performance_cores().unwrap_or(0),
        battery = has_battery(),
        on_ac = crate::jobs::on_ac_power(),
        ollama = ollama.is_some(),
        ollama_model,
        ollama_model_installed = ollama.as_ref().is_some_and(|o| is_installed(ollama_model, &o.installed)),
        installed = %ollama.as_ref().map_or("none".into(), |o| installed_summary(&o.installed)),
        loaded = %ollama.as_ref().map_or("none".into(), |o| loaded_summary(&o.loaded)),
        "this PC"
    );
}

/// The two trees' CPU snapshots, kept between ticks.
pub struct Sampler {
    whimbrel: CpuSnapshot,
    ollama: CpuSnapshot,
}

impl Sampler {
    pub fn start() -> Self {
        let all = proc_stats::list_processes();
        Self { whimbrel: CpuSnapshot::take(&whimbrel_pids(&all)), ollama: CpuSnapshot::take(&ollama_pids(&all)) }
    }

    /// Writes the `resource use` line for the time since the last tick, and starts the next window.
    pub fn tick(&mut self) {
        let all = proc_stats::list_processes();
        let (whimbrel_pids, ollama_pids) = (whimbrel_pids(&all), ollama_pids(&all));
        let (whimbrel, ollama) = (CpuSnapshot::take(&whimbrel_pids), CpuSnapshot::take(&ollama_pids));
        let (whimbrel_cpu, ollama_cpu) = (whimbrel.since(&self.whimbrel), ollama.since(&self.ollama));
        let memory = memory_status();
        let ((whimbrel_mem, _), (ollama_mem, ollama_readable)) = (memory_of(&whimbrel_pids), memory_of(&ollama_pids));
        let loaded = if ollama_pids.is_empty() { None } else { ollama_state() };
        tracing::info!(
            free_ram_gb = %memory.map_or("?".into(), |m| gb(m.available)),
            whimbrel_cpu_pct = %format!("{:.1}", whimbrel_cpu.pct_of_machine),
            whimbrel_private_mb = mb(whimbrel_mem.private_bytes),
            whimbrel_working_set_mb = mb(whimbrel_mem.working_set_bytes),
            whimbrel_processes = whimbrel_pids.len(),
            ollama_cpu_pct = %format!("{:.1}", ollama_cpu.pct_of_machine),
            ollama_private_mb = mb(ollama_mem.private_bytes),
            ollama_working_set_mb = mb(ollama_mem.working_set_bytes),
            ollama_processes = ollama_pids.len(),
            ollama_readable = ollama_readable,
            ollama_loaded = %loaded.as_ref().map_or("none".into(), |o| loaded_summary(&o.loaded)),
            on_ac = crate::jobs::on_ac_power(),
            idle_s = crate::capture::session_gate::idle_duration().as_secs(),
            "resource use"
        );
        self.whimbrel = whimbrel;
        self.ollama = ollama;
    }
}

/// The lines for the diagnostics report's "This PC" section.
pub fn report_lines() -> Vec<String> {
    let memory = memory_status();
    let ollama = ollama_state();
    vec![
        format!(
            "Memory: {} GB, {} GB free",
            memory.map_or("?".into(), |m| gb(m.total)),
            memory.map_or("?".into(), |m| gb(m.available))
        ),
        format!(
            "Cores: {} logical, {} performance",
            proc_stats::logical_cpus(),
            crate::meetings::perf::performance_cores().map_or("?".into(), |n| n.to_string())
        ),
        format!("Battery: {}; on AC power: {}", has_battery(), crate::jobs::on_ac_power()),
        match &ollama {
            Some(o) => format!("Ollama: running; installed {}; loaded {}", installed_summary(&o.installed), loaded_summary(&o.loaded)),
            None => "Ollama: not running".into(),
        },
    ]
}

/// Starts the launch line and the 5-minute loop on their own thread.
pub fn start(ollama_model: String) {
    let _ = std::thread::Builder::new().name("whimbrel-perf".into()).spawn(move || {
        log_this_pc(&ollama_model);
        let mut sampler = Sampler::start();
        loop {
            std::thread::sleep(EVERY);
            sampler.tick();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, exe: &str) -> ProcInfo {
        ProcInfo { pid, parent, exe: exe.into() }
    }

    #[test]
    fn only_ollamas_processes_are_picked_out() {
        let all = vec![
            p(1, 0, "explorer.exe"),
            p(10, 1, "ollama app.exe"),
            p(11, 10, "ollama.exe"),
            p(12, 11, "ollama.exe"),
            p(13, 11, "ollama_llama_server.exe"),
            p(20, 1, "chrome.exe"),
            p(21, 20, "chrome.exe"),
        ];
        assert_eq!(ollama_pids(&all), vec![10, 11, 12, 13]);
        assert!(ollama_pids(&[p(1, 0, "explorer.exe"), p(2, 1, "notepad.exe")]).is_empty());
    }

    #[test]
    fn models_read_as_names_and_sizes() {
        let installed = vec![
            OllamaModel { name: "gemma4:12b".into(), size: 8_021_618_941, size_vram: 0 },
            OllamaModel { name: "qwen3:latest".into(), size: 5_200_000_000, size_vram: 0 },
        ];
        assert_eq!(installed_summary(&installed), "gemma4:12b 8.0 GB, qwen3:latest 5.2 GB");
        assert_eq!(installed_summary(&[]), "none");
        let loaded = vec![OllamaModel { name: "gemma4:12b".into(), size: 9_600_000_000, size_vram: 4_800_000_000 }];
        assert_eq!(loaded_summary(&loaded), "gemma4:12b 9.6 GB (GPU 4.8 GB)");
        assert!(is_installed("gemma4:12b", &installed));
        assert!(is_installed("qwen3", &installed), "a bare name means :latest");
        assert!(!is_installed("gemma4:4b", &installed));
    }

    #[test]
    fn ollamas_answers_parse_with_fields_missing() {
        let ps: ModelList = serde_json::from_str(r#"{"models":[{"name":"gemma4:12b","model":"gemma4:12b","size":1471330384,"size_vram":1471330384,"expires_at":"x"}]}"#).unwrap();
        assert_eq!(ps.models, vec![OllamaModel { name: "gemma4:12b".into(), size: 1_471_330_384, size_vram: 1_471_330_384 }]);
        let tags: ModelList = serde_json::from_str(r#"{"models":[{"name":"a:b"}]}"#).unwrap();
        assert_eq!(tags.models[0].size, 0);
        let empty: ModelList = serde_json::from_str("{}").unwrap();
        assert!(empty.models.is_empty());
    }

    #[test]
    fn the_battery_flag_reads_as_windows_documents_it() {
        assert!(!battery_present(128), "no system battery");
        assert!(!battery_present(255), "unknown");
        assert!(battery_present(8), "charging");
        assert!(battery_present(1), "high");
    }

    #[test]
    fn this_pc_reports_its_memory_and_the_report_has_its_lines() {
        let m = memory_status().expect("Windows reports memory");
        assert!(m.total > 1_000_000_000 && m.available <= m.total);
        let lines = report_lines();
        assert!(lines[0].starts_with("Memory: ") && lines.iter().any(|l| l.starts_with("Ollama: ")));
        assert!(!whimbrel_pids(&proc_stats::list_processes()).is_empty(), "this process at least");
    }
}
