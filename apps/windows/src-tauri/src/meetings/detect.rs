//! Meeting detection (M3.3 Task 6, docs/12 §4.5): which app on this PC is in a call.
//!
//! Windows gives a better signal than the Mac has. `IAudioSessionManager2` reports audio
//! sessions per process on both the output and the input endpoint, so "Zoom is playing
//! audio **and** Zoom has the microphone open" is a direct query rather than a guess from
//! device-in-use flags. The same per-process capture session is what ends a detected
//! meeting when the app drops back to its lobby (issue #84).
//!
//! A browser is a call only when it holds the microphone *and* its address bar is on a
//! meeting host, so an ordinary tab with a video playing never counts.

use std::collections::HashSet;

use api::meetings::heuristic::{self, Observation};

use crate::capture::{audio::AudioSessionInfo, proc_stats, window};

/// The desktop apps a call can run in (docs/12 §4.5's list, lowercased exe names).
pub const MEETING_APPS: [&str; 7] =
    ["zoom.exe", "ms-teams.exe", "teams.exe", "slack.exe", "ciscocollabhost.exe", "webex.exe", "webexmta.exe"];
/// Browsers whose address bar is worth reading when they hold the microphone.
pub const BROWSERS: [&str; 5] = ["msedge.exe", "chrome.exe", "firefox.exe", "brave.exe", "opera.exe"];

/// One poll's audio sessions, reduced to what detection needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sessions {
    /// Processes with an active session on an output device.
    pub rendering: HashSet<u32>,
    /// Processes with an active session on an input device: the microphone is open.
    pub capturing: HashSet<u32>,
}

impl Sessions {
    /// Reduces `audio_sessions()` output. Only active sessions count: an app that merely
    /// has a session open (Teams idle in the tray) is not in a call.
    pub fn from(sessions: &[AudioSessionInfo]) -> Self {
        let mut reduced = Sessions::default();
        for session in sessions.iter().filter(|s| s.state == "active" && s.pid != 0) {
            let set = if session.flow == "capture" { &mut reduced.capturing } else { &mut reduced.rendering };
            set.insert(session.pid);
        }
        reduced
    }

    fn any(&self, set: impl Fn(&Self) -> &HashSet<u32>, pids: &[u32]) -> bool {
        pids.iter().any(|pid| set(self).contains(pid))
    }

    pub fn tree_renders(&self, tree: &[u32]) -> bool {
        self.any(|s| &s.rendering, tree)
    }

    pub fn tree_captures(&self, tree: &[u32]) -> bool {
        self.any(|s| &s.capturing, tree)
    }

    /// Whether any process outside `own_tree` (Whimbrel and its web-view children) holds an
    /// active microphone session. A live call always does; Whimbrel opens the device only
    /// once it records. This is how a calendar prompt knows this PC is in the call, not
    /// merely near one.
    pub fn other_captures(&self, own_tree: &[u32]) -> bool {
        self.capturing.iter().any(|pid| !own_tree.contains(pid))
    }
}

/// An app that looks like it is in a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The process tree's root: what process loopback records.
    pub root_pid: u32,
    pub exe: String,
    /// What the alert calls it ("Zoom", "Google Meet").
    pub title: String,
    pub via_browser: bool,
    /// Whether the call's audio is live right now (render and capture both active).
    pub audio_live: bool,
}

/// Turns a candidate into the observation the debounce takes.
pub fn observation(candidate: Option<Candidate>) -> Observation {
    match candidate {
        Some(c) if c.audio_live => {
            Observation::Match { app_id: c.exe, title: c.title, via_browser: c.via_browser }
        }
        Some(c) => Observation::MatchWithoutAudio { app_id: c.exe },
        None => Observation::NoMatch { tracked_browser_on_meeting: None },
    }
}

/// The display name for a meeting app, for the alert's title.
pub fn app_title(exe: &str) -> String {
    match exe {
        "zoom.exe" => "Zoom",
        "ms-teams.exe" | "teams.exe" => "Microsoft Teams",
        "slack.exe" => "Slack",
        "ciscocollabhost.exe" | "webex.exe" | "webexmta.exe" => "Webex",
        other => other.trim_end_matches(".exe"),
    }
    .to_string()
}

/// Picks the app in a call from one poll's processes and sessions. Pure, so the rule is
/// testable without audio devices: a desktop app needs both an active output and an
/// active input session; a browser needs the microphone and a meeting URL, which the
/// caller resolves through `address` (a UIA read, which this function never does itself).
pub fn candidate(
    processes: &[proc_stats::ProcInfo],
    sessions: &Sessions,
    address: impl Fn(u32) -> Option<String>,
) -> Option<Candidate> {
    let mut best: Option<Candidate> = None;
    for exe in MEETING_APPS {
        for root in proc_stats::tree_roots_named(exe, processes) {
            let tree = proc_stats::process_tree(root, processes);
            let (renders, captures) = (sessions.tree_renders(&tree), sessions.tree_captures(&tree));
            if !renders && !captures {
                continue;
            }
            let candidate = Candidate {
                root_pid: root,
                exe: exe.to_string(),
                title: app_title(exe),
                via_browser: false,
                audio_live: renders && captures,
            };
            if candidate.audio_live {
                return Some(candidate);
            }
            best.get_or_insert(candidate);
        }
    }

    for exe in BROWSERS {
        for root in proc_stats::tree_roots_named(exe, processes) {
            let tree = proc_stats::process_tree(root, processes);
            // The microphone first: reading an address bar costs a UIA round trip, and a
            // browser that isn't listening is not in a call whatever tab is open.
            if !sessions.tree_captures(&tree) {
                continue;
            }
            let Some(provider) = address(root).as_deref().and_then(heuristic::meeting_url_provider) else {
                continue;
            };
            let candidate = Candidate {
                root_pid: root,
                exe: exe.to_string(),
                title: provider_title(provider),
                via_browser: true,
                audio_live: sessions.tree_renders(&tree),
            };
            if candidate.audio_live {
                return Some(candidate);
            }
            best.get_or_insert(candidate);
        }
    }
    best
}

fn provider_title(provider: api::connectors::calendar::MeetingProvider) -> String {
    use api::connectors::calendar::MeetingProvider::*;
    match provider {
        GoogleMeet => "Google Meet",
        Teams => "Microsoft Teams",
        Zoom => "Zoom",
        Webex => "Webex",
    }
    .to_string()
}

/// The address bar of a browser process's frontmost window, through UIA: the first Edit
/// control with a value that parses as a host. `None` when the browser has no window on
/// this desktop, or UIA won't answer.
pub fn browser_address(uia: &crate::capture::uia::Uia, pid: u32, tree: &[u32]) -> Option<String> {
    use windows::Win32::UI::Accessibility::{IUIAutomationValuePattern, UIA_EditControlTypeId, UIA_ValuePatternId};
    use windows::Win32::UI::Accessibility::{TreeScope_Descendants, UIA_ControlTypePropertyId};
    use windows::Win32::System::Variant::VARIANT;

    let _ = pid;
    let windows: Vec<_> = window::top_level_windows().into_iter().filter(|w| tree.contains(&w.pid)).collect();
    unsafe {
        let automation = uia.automation();
        let is_edit = automation.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_EditControlTypeId.0)).ok()?;
        for info in windows {
            let Ok(root) = automation.ElementFromHandle(info.hwnd) else { continue };
            let Ok(found) = root.FindAll(TreeScope_Descendants, &is_edit) else { continue };
            let count = found.Length().unwrap_or(0);
            for i in 0..count.min(8) {
                let Ok(element) = found.GetElement(i) else { continue };
                let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) else {
                    continue;
                };
                let value = pattern.CurrentValue().map(|v| v.to_string()).unwrap_or_default();
                if heuristic::meeting_url_provider(&value).is_some() {
                    return Some(value);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: u32, exe: &str) -> proc_stats::ProcInfo {
        proc_stats::ProcInfo { pid, parent, exe: exe.to_string() }
    }

    fn sessions(rendering: &[u32], capturing: &[u32]) -> Sessions {
        Sessions { rendering: rendering.iter().copied().collect(), capturing: capturing.iter().copied().collect() }
    }

    fn no_address(_: u32) -> Option<String> {
        None
    }

    #[test]
    fn the_microphone_counts_as_in_use_only_when_a_process_outside_whimbrel_holds_it() {
        let own = [10, 11];
        assert!(!sessions(&[], &[]).other_captures(&own), "nothing has the microphone");
        assert!(!sessions(&[300], &[]).other_captures(&own), "playing audio is not the microphone");
        assert!(!sessions(&[], &[10, 11]).other_captures(&own), "Whimbrel's own tree does not count");
        assert!(sessions(&[], &[10, 400]).other_captures(&own), "Zoom holds it alongside Whimbrel");
        assert!(sessions(&[], &[400]).other_captures(&[]), "an empty own tree excludes nothing");
    }

    #[test]
    fn only_active_sessions_count_and_the_system_process_is_ignored() {
        let raw = |pid, flow, state| AudioSessionInfo {
            flow,
            device: "Speakers".into(),
            default_device: true,
            pid,
            exe: "zoom.exe".into(),
            state,
            peak: 0.5,
        };
        let reduced = Sessions::from(&[
            raw(10, "render", "active"),
            raw(10, "capture", "active"),
            raw(11, "render", "inactive"),
            raw(0, "render", "active"),
        ]);
        assert_eq!(reduced, sessions(&[10], &[10]));
    }

    #[test]
    fn a_meeting_app_with_both_sessions_is_the_candidate() {
        let processes = vec![proc(100, 1, "zoom.exe"), proc(101, 100, "zoom.exe"), proc(200, 1, "code.exe")];
        let found = candidate(&processes, &sessions(&[101], &[100]), no_address).expect("a candidate");
        assert_eq!((found.root_pid, found.exe.as_str(), found.audio_live), (100, "zoom.exe", true));
        assert_eq!(found.title, "Zoom", "the alert names the app");
        assert!(!found.via_browser);
    }

    #[test]
    fn a_meeting_app_playing_audio_without_the_microphone_is_not_live() {
        let processes = vec![proc(100, 1, "ms-teams.exe")];
        let found = candidate(&processes, &sessions(&[100], &[]), no_address).expect("a candidate");
        assert!(!found.audio_live, "audio out alone is a notification, not a call");
        assert!(matches!(observation(Some(found)), Observation::MatchWithoutAudio { .. }));
    }

    #[test]
    fn nothing_matches_when_no_meeting_app_has_audio() {
        let processes = vec![proc(100, 1, "zoom.exe"), proc(200, 1, "msedge.exe")];
        assert_eq!(candidate(&processes, &sessions(&[200], &[]), no_address), None);
        assert!(matches!(observation(None), Observation::NoMatch { .. }));
    }

    #[test]
    fn a_browser_counts_only_with_the_microphone_and_a_meeting_url() {
        let processes = vec![proc(300, 1, "msedge.exe"), proc(301, 300, "msedge.exe")];
        let on_meet = |_: u32| Some("https://meet.google.com/abc-defg-hij".to_string());
        let on_docs = |_: u32| Some("https://docs.google.com/document/d/abc".to_string());

        assert_eq!(candidate(&processes, &sessions(&[300], &[]), on_meet), None, "no microphone, no call");
        assert_eq!(candidate(&processes, &sessions(&[300], &[301]), on_docs), None, "microphone on a document is not a call");

        let found = candidate(&processes, &sessions(&[300], &[301]), on_meet).expect("a candidate");
        assert_eq!((found.root_pid, found.title.as_str(), found.via_browser, found.audio_live), (300, "Google Meet", true, true));
    }

    #[test]
    fn a_desktop_app_wins_over_a_browser_tab() {
        let processes = vec![proc(100, 1, "zoom.exe"), proc(300, 1, "chrome.exe")];
        let on_meet = |_: u32| Some("https://meet.google.com/abc".to_string());
        let found = candidate(&processes, &sessions(&[100, 300], &[100, 300]), on_meet).expect("a candidate");
        assert_eq!(found.exe, "zoom.exe");
    }

    #[test]
    fn unknown_apps_keep_their_exe_name_in_the_title() {
        assert_eq!(app_title("webex.exe"), "Webex");
        assert_eq!(app_title("bluejeans.exe"), "bluejeans");
    }
}
