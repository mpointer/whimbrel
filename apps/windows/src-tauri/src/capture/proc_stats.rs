//! Per-process-tree CPU and memory sampling — the instrument for §4.1's "Chromium CPU
//! delta after Whimbrel attaches" column and NFR-PRF-1's ambient CPU/RSS budget.
//!
//! CPU is measured as kernel+user time consumed over a wall-clock window, summed over a
//! process tree (Chromium's cost lands in renderer children, not the browser process).
//! It is reported two ways because the two platforms' tools disagree: "% of one core"
//! (what macOS Activity Monitor shows, and so what NFR-PRF-1's 5% was written against)
//! and "% of the whole machine" (what Task Manager shows).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub parent: u32,
    /// Executable basename as Toolhelp reports it, lowercased (`msedge.exe`).
    pub exe: String,
}

/// Every process on the machine, one Toolhelp snapshot.
pub fn list_processes() -> Vec<ProcInfo> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                out.push(ProcInfo {
                    pid: entry.th32ProcessID,
                    parent: entry.th32ParentProcessID,
                    exe: String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase(),
                });
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// `root` plus every descendant. Pure over a snapshot, so it is unit-testable. Guards
/// against parent-PID reuse cycles (Windows recycles PIDs; a stale parent id can point
/// at an unrelated newer process).
pub fn process_tree(root: u32, all: &[ProcInfo]) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in all {
        if p.pid != p.parent {
            children.entry(p.parent).or_default().push(p.pid);
        }
    }
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    let mut out = Vec::new();
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue;
        }
        out.push(pid);
        if let Some(kids) = children.get(&pid) {
            stack.extend(kids.iter().copied());
        }
    }
    out
}

/// Roots of every tree of processes named `exe` — processes named `exe` whose parent is
/// not also named `exe`. For Edge that is the browser process(es); renderers hang off it.
pub fn tree_roots_named(exe: &str, all: &[ProcInfo]) -> Vec<u32> {
    let exe = exe.to_lowercase();
    let named: HashSet<u32> = all.iter().filter(|p| p.exe == exe).map(|p| p.pid).collect();
    all.iter()
        .filter(|p| p.exe == exe && !named.contains(&p.parent))
        .map(|p| p.pid)
        .collect()
}

fn filetime_100ns(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

struct ProcHandle(HANDLE);
impl Drop for ProcHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open(pid: u32) -> Option<ProcHandle> {
    unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid)
            .or_else(|_| OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid))
            .ok()
            .map(ProcHandle)
    }
}

/// Kernel + user CPU time consumed so far, in 100 ns units. `None` if the process is
/// gone or protected.
pub fn cpu_time_100ns(pid: u32) -> Option<u64> {
    let h = open(pid)?;
    let (mut c, mut e, mut k, mut u) = Default::default();
    unsafe { GetProcessTimes(h.0, &mut c, &mut e, &mut k, &mut u).ok()? };
    Some(filetime_100ns(k) + filetime_100ns(u))
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct Memory {
    pub working_set_bytes: u64,
    pub private_bytes: u64,
}

pub fn memory(pid: u32) -> Option<Memory> {
    let h = open(pid)?;
    let mut pmc = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    unsafe {
        GetProcessMemoryInfo(
            h.0,
            &mut pmc as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            pmc.cb,
        )
        .ok()?
    };
    Some(Memory {
        working_set_bytes: pmc.WorkingSetSize as u64,
        private_bytes: pmc.PrivateUsage as u64,
    })
}

pub fn logical_cpus() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// CPU consumed by a set of processes over a window.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct CpuWindow {
    pub wall_secs: f64,
    pub cpu_secs: f64,
    /// 100% == one logical core fully busy (Activity Monitor convention).
    pub pct_of_one_core: f64,
    /// 100% == every logical core fully busy (Task Manager convention).
    pub pct_of_machine: f64,
    pub processes: usize,
}

/// A CPU-time snapshot of a set of PIDs, to diff against a later one. Processes that
/// exit between snapshots drop out of both sides (their time is not counted); ones that
/// start are counted from zero, which is correct for "cost incurred in the window".
#[derive(Debug, Clone)]
pub struct CpuSnapshot {
    at: Instant,
    per_pid: HashMap<u32, u64>,
}

impl CpuSnapshot {
    pub fn take(pids: &[u32]) -> Self {
        let per_pid = pids.iter().filter_map(|&p| cpu_time_100ns(p).map(|t| (p, t))).collect();
        Self { at: Instant::now(), per_pid }
    }

    pub fn since(&self, earlier: &CpuSnapshot) -> CpuWindow {
        let wall = self.at.duration_since(earlier.at).as_secs_f64().max(1e-9);
        let cpu_100ns: u64 = self
            .per_pid
            .iter()
            .map(|(pid, now)| now.saturating_sub(*earlier.per_pid.get(pid).unwrap_or(&0)))
            .sum();
        let cpu_secs = cpu_100ns as f64 / 1e7;
        let one_core = 100.0 * cpu_secs / wall;
        CpuWindow {
            wall_secs: wall,
            cpu_secs,
            pct_of_one_core: one_core,
            pct_of_machine: one_core / logical_cpus() as f64,
            processes: self.per_pid.len(),
        }
    }
}

/// Re-resolves the tree under `root` at both ends so renderers spawned mid-window count.
pub fn measure_tree_cpu(root: u32, window: Duration) -> CpuWindow {
    let before = CpuSnapshot::take(&process_tree(root, &list_processes()));
    std::thread::sleep(window);
    let after = CpuSnapshot::take(&process_tree(root, &list_processes()));
    after.since(&before)
}

pub fn tree_memory(root: u32) -> Memory {
    process_tree(root, &list_processes())
        .into_iter()
        .filter_map(memory)
        .fold(Memory::default(), |acc, m| Memory {
            working_set_bytes: acc.working_set_bytes + m.working_set_bytes,
            private_bytes: acc.private_bytes + m.private_bytes,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, exe: &str) -> ProcInfo {
        ProcInfo { pid, parent, exe: exe.into() }
    }

    #[test]
    fn tree_includes_root_and_all_descendants_only() {
        let all = vec![
            p(1, 0, "explorer.exe"),
            p(10, 1, "msedge.exe"),
            p(11, 10, "msedge.exe"),
            p(12, 10, "msedge.exe"),
            p(13, 12, "msedge.exe"),
            p(20, 1, "code.exe"),
        ];
        let mut t = process_tree(10, &all);
        t.sort();
        assert_eq!(t, vec![10, 11, 12, 13]);
    }

    #[test]
    fn tree_survives_a_pid_reuse_cycle() {
        // 5's recorded parent is 6 and 6's is 5 — a stale-parent cycle.
        let all = vec![p(5, 6, "a.exe"), p(6, 5, "b.exe")];
        let mut t = process_tree(5, &all);
        t.sort();
        assert_eq!(t, vec![5, 6]);
    }

    #[test]
    fn roots_are_named_processes_with_differently_named_parents() {
        let all = vec![
            p(1, 0, "explorer.exe"),
            p(10, 1, "msedge.exe"),
            p(11, 10, "msedge.exe"),
            p(30, 1, "msedge.exe"), // a second, independent Edge instance
        ];
        let mut r = tree_roots_named("MSEDGE.EXE", &all);
        r.sort();
        assert_eq!(r, vec![10, 30]);
    }
}
