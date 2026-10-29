//! Which window a meeting's snapshots come from: only ever the meeting app's own window,
//! never a whole display. When it can't tell, nothing is captured that tick.
//!
//! - A meeting with a process tree (detected, or a calendar meeting that started while
//!   detection saw the app): the largest visible window in that tree, or for a browser,
//!   the window whose active tab is the meeting.
//! - A meeting without one: detection's live call app, else the first meeting app with a
//!   window. Slack is skipped there, since it is usually open for chat rather than a call.
//! - Minimised and cloaked windows, and anything `capture::exclusions` excludes (private
//!   browser windows, password managers, the user's list), are never captured.

use windows::Win32::Foundation::HWND;

use crate::capture::exclusions::Exclusions;
use crate::capture::proc_stats::{self, ProcInfo};
use crate::capture::uia::Uia;
use crate::capture::{app_id, window};
use crate::meetings::detect::{self, MEETING_APPS};

/// The meeting window to capture now, or `None`. `root_pid` is the meeting's process tree,
/// if it has one.
pub fn meeting_window(root_pid: Option<u32>, uia: Option<&Uia>, exclusions: &Exclusions) -> Option<HWND> {
    let processes = proc_stats::list_processes();
    let windows = window::top_level_windows();
    let root = root_pid.or_else(|| {
        let with_windows: Vec<u32> = windows.iter().map(|w| w.pid).collect();
        first_meeting_app(&processes, &with_windows)
    })?;
    let exe = processes.iter().find(|p| p.pid == root)?.exe.clone();
    let tree = proc_stats::process_tree(root, &processes);
    let showing: Vec<window::WindowInfo> =
        windows.into_iter().filter(|w| tree.contains(&w.pid) && !w.minimized && !window::is_cloaked(w.hwnd)).collect();

    let chosen = if detect::BROWSERS.contains(&exe.as_str()) {
        let (hwnd, _) = detect::meeting_browser_window(uia?, &showing)?;
        showing.into_iter().find(|w| w.hwnd == hwnd)?
    } else {
        let areas: Vec<u64> = showing.iter().map(|w| window::area(w.hwnd)).collect();
        showing.into_iter().nth(largest(&areas)?)?
    };
    let app = app_id::normalize(&chosen.app_id, &chosen.exe);
    if exclusions.check(&chosen.exe, &app, &chosen.title, chosen.pid, std::process::id()).is_some() {
        return None;
    }
    Some(chosen.hwnd)
}

/// The first meeting app, in `MEETING_APPS` order and never Slack, whose process tree has
/// a window (`pids_with_windows`).
fn first_meeting_app(processes: &[ProcInfo], pids_with_windows: &[u32]) -> Option<u32> {
    MEETING_APPS.iter().filter(|exe| **exe != "slack.exe").find_map(|exe| {
        proc_stats::tree_roots_named(exe, processes)
            .into_iter()
            .find(|root| proc_stats::process_tree(*root, processes).iter().any(|pid| pids_with_windows.contains(pid)))
    })
}

/// The index of the largest non-empty area.
fn largest(areas: &[u64]) -> Option<usize> {
    areas.iter().enumerate().filter(|(_, a)| **a > 0).max_by_key(|(_, a)| **a).map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: u32, exe: &str) -> ProcInfo {
        ProcInfo { pid, parent, exe: exe.to_string() }
    }

    #[test]
    fn the_largest_window_wins_and_empty_ones_never_do() {
        assert_eq!(largest(&[100, 900, 300]), Some(1));
        assert_eq!(largest(&[0, 0]), None);
        assert_eq!(largest(&[]), None);
    }

    #[test]
    fn without_a_tree_the_first_meeting_app_with_a_window_is_used() {
        let processes = vec![proc(10, 1, "zoom.exe"), proc(11, 10, "zoom.exe"), proc(20, 1, "ms-teams.exe")];
        assert_eq!(first_meeting_app(&processes, &[11, 20]), Some(10), "a child's window counts for its tree");
        assert_eq!(first_meeting_app(&processes, &[20]), Some(20), "Zoom has no window");
        assert_eq!(first_meeting_app(&processes, &[]), None);
    }

    #[test]
    fn slack_open_for_chat_is_never_the_fallback() {
        let processes = vec![proc(30, 1, "slack.exe")];
        assert_eq!(first_meeting_app(&processes, &[30]), None);
    }
}
