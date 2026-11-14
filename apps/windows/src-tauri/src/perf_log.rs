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
/// How often it is written while a meeting records.
pub const MEETING_EVERY: Duration = Duration::from_secs(60);
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

/// Which part of Whimbrel a process in its tree is, for the per-part CPU and memory: the app itself,
/// its WebView2 windows, the local model's helper, or anything else it started (connectors, the MCP
/// server). Only Whimbrel's own tree is ever sorted, so no other program is named.
pub fn part_of(exe: &str) -> &'static str {
    let exe = exe.to_ascii_lowercase();
    if exe == "msedgewebview2.exe" {
        "webview"
    } else if exe.contains("llama-server") {
        "local_model"
    } else if exe.starts_with("whimbrel") && !exe.contains("mcp") {
        "app"
    } else {
        "helpers"
    }
}

pub const PARTS: [&str; 4] = ["app", "webview", "local_model", "helpers"];

fn pids_by_part(whimbrel: &[u32], all: &[ProcInfo]) -> Vec<(&'static str, Vec<u32>)> {
    PARTS
        .iter()
        .map(|part| {
            let pids = whimbrel.iter().copied().filter(|pid| all.iter().any(|p| p.pid == *pid && part_of(&p.exe) == *part)).collect();
            (*part, pids)
        })
        .collect()
}

/// The whole machine's busy and total CPU time (`GetSystemTimes`), in 100 ns units.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemTimes {
    busy: u64,
    total: u64,
}

pub fn system_times() -> Option<SystemTimes> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetSystemTimes;
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    let v = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    // Kernel time includes idle time.
    let total = v(kernel) + v(user);
    Some(SystemTimes { busy: total.saturating_sub(v(idle)), total })
}

/// The machine's CPU use between two readings, in percent.
pub fn system_cpu_pct(earlier: SystemTimes, now: SystemTimes) -> f64 {
    let total = now.total.saturating_sub(earlier.total);
    if total == 0 {
        return 0.0;
    }
    100.0 * now.busy.saturating_sub(earlier.busy) as f64 / total as f64
}

/// Windows' memory load: the percent of physical memory in use.
pub fn memory_load_pct() -> Option<u32> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(status.dwMemoryLoad)
}

/// One graphics adapter: its name, its own memory, and whether it shares system memory instead
/// (an integrated GPU, whose work competes with a call's video for the same chip and RAM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub dedicated_mb: u64,
    pub shared_mb: u64,
}

impl Adapter {
    /// Under 512 MB of its own memory: an integrated GPU.
    pub fn integrated(&self) -> bool {
        self.dedicated_mb < 512
    }
}

/// The hardware graphics adapters (DXGI), software ones left out.
pub fn adapters() -> Vec<Adapter> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return Vec::new() };
    let mut out = Vec::new();
    for i in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
        out.push(Adapter {
            name: String::from_utf16_lossy(&desc.Description[..end]).trim().to_string(),
            dedicated_mb: desc.DedicatedVideoMemory as u64 >> 20,
            shared_mb: desc.SharedSystemMemory as u64 >> 20,
        });
    }
    out
}

pub fn adapters_summary(adapters: &[Adapter]) -> String {
    if adapters.is_empty() {
        return "none found".into();
    }
    adapters
        .iter()
        .map(|a| format!("{} ({}, {} MB own)", a.name, if a.integrated() { "integrated" } else { "dedicated" }, a.dedicated_mb))
        .collect::<Vec<_>>()
        .join("; ")
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
        gpus = %adapters_summary(&adapters()),
        "this PC"
    );
}

/// The two trees' CPU snapshots, Whimbrel's parts' and the machine's, kept between ticks.
pub struct Sampler {
    whimbrel: CpuSnapshot,
    ollama: CpuSnapshot,
    parts: Vec<(&'static str, CpuSnapshot)>,
    system: Option<SystemTimes>,
}

impl Sampler {
    pub fn start() -> Self {
        let all = proc_stats::list_processes();
        let whimbrel = whimbrel_pids(&all);
        let parts = pids_by_part(&whimbrel, &all).into_iter().map(|(part, pids)| (part, CpuSnapshot::take(&pids))).collect();
        Self { whimbrel: CpuSnapshot::take(&whimbrel), ollama: CpuSnapshot::take(&ollama_pids(&all)), parts, system: system_times() }
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
        // The same window for the performance report, per part and with the whole machine.
        let system = system_times();
        let system_cpu = self.system.zip(system).map(|(a, b)| system_cpu_pct(a, b));
        let mut line = format!(
            "resource system_cpu_pct={} memory_load_pct={} free_ram_gb={} ollama_cpu_pct={:.1} ollama_working_set_mb={}",
            system_cpu.map_or("?".into(), |c| format!("{c:.0}")),
            memory_load_pct().map_or("?".into(), |m| m.to_string()),
            memory.map_or("?".into(), |m| gb(m.available)),
            ollama_cpu.pct_of_machine,
            mb(ollama_mem.working_set_bytes),
        );
        let mut parts = Vec::new();
        for (part, pids) in pids_by_part(&whimbrel_pids, &all) {
            let now = CpuSnapshot::take(&pids);
            let cpu = self.parts.iter().find(|(p, _)| *p == part).map_or(0.0, |(_, before)| now.since(before).pct_of_machine);
            let (mem, _) = memory_of(&pids);
            if !pids.is_empty() {
                line.push_str(&format!(
                    " {part}_cpu_pct={cpu:.1} {part}_private_mb={} {part}_working_set_mb={}",
                    mb(mem.private_bytes),
                    mb(mem.working_set_bytes)
                ));
            }
            parts.push((part, now));
        }
        line.push_str(&format!(
            " on_ac={} idle_s={} recording={}",
            crate::jobs::on_ac_power(),
            crate::capture::session_gate::idle_duration().as_secs(),
            crate::perf_report::recording()
        ));
        crate::perf_report::note(line);
        self.whimbrel = whimbrel;
        self.ollama = ollama;
        self.parts = parts;
        self.system = system;
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
        format!(
            "Processor: {}; this build: {}{}",
            crate::gpu::native_machine(),
            std::env::consts::ARCH,
            if crate::gpu::emulated_on_arm() { " (runs under emulation)" } else { "" }
        ),
        format!("Graphics: {}", adapters_summary(&adapters())),
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
        let mut last = std::time::Instant::now();
        loop {
            std::thread::sleep(MEETING_EVERY);
            // Every minute while a meeting records, so a choppy call shows in the numbers.
            if crate::perf_report::recording() || last.elapsed() >= EVERY {
                last = std::time::Instant::now();
                sampler.tick();
            }
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
    fn whimbrels_processes_sort_into_parts() {
    assert_eq!(part_of("whimbrel-windows.exe"), "app");
    assert_eq!(part_of("Whimbrel.exe"), "app");
    assert_eq!(part_of("msedgewebview2.exe"), "webview");
    assert_eq!(part_of("whimbrel-llama-server.exe"), "local_model");
    assert_eq!(part_of("whimbrel-mcp-server.exe"), "helpers");
    assert_eq!(part_of("node.exe"), "helpers");
}

#[test]
fn the_machines_cpu_is_busy_over_total() {
    let a = SystemTimes { busy: 100, total: 1_000 };
    let b = SystemTimes { busy: 400, total: 2_000 };
    assert_eq!(system_cpu_pct(a, b), 30.0);
    assert_eq!(system_cpu_pct(b, b), 0.0, "no time passed");
}

#[test]
fn an_adapter_with_little_memory_of_its_own_is_integrated() {
    let a = |mb| Adapter { name: "x".into(), dedicated_mb: mb, shared_mb: 8_000 };
    assert!(a(128).integrated());
    assert!(!a(4_096).integrated());
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
