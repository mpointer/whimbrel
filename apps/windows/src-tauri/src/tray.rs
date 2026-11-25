//! The tray (plan Task 10). Its icon follows capture and sync state, and turns red while a
//! meeting records (M3.3 Task 3). Left-click toggles a quick-glance popup, and right-click
//! shows the menu.
//!
//! The popup is a small undecorated window. Like the main window, it is destroyed rather
//! than hidden when it loses focus, so no second WebView2 renderer idles all day
//! (docs/12 W2).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDateTime, TimeZone};
use schemars::JsonSchema;
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Rect, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::capture::service::ReadProblem;
use crate::l10n::tr;
use crate::state::AppState;
use crate::syncing::controller::SyncState;

pub const TRAY_ID: &str = "whimbrel";
/// The popup window's label (also listed in `capabilities/default.json`).
pub const POPUP: &str = "popup";
/// The popup's size, in logical pixels (taller by a line for the next meeting, topics piece 2).
const POPUP_SIZE: (f64, f64) = (300.0, 272.0);
/// The redesigned popup's size: Recording, Memory with its three choices, Needs attention and Next meeting, then Open
/// Whimbrel and Quit. The recording row with Stop, the memory state line and the footer are pinned and the rest scrolls between
/// them, so at 200% text the pinned parts need the height, and the worst case at 100% (a paused recording with a problem, memory on, a next meeting) fits without scrolling: 560 (it was 480).
const POPUP_SIZE_REDESIGN: (f64, f64) = (320.0, 560.0);

/// The popup's size for the look in use.
pub fn popup_size(redesign: bool) -> (f64, f64) {
    if redesign {
        POPUP_SIZE_REDESIGN
    } else {
        POPUP_SIZE
    }
}

/// A topic the popup asked the main window to open, until the main window takes it.
static PENDING_TOPIC: Mutex<Option<i64>> = Mutex::new(None);
/// Tells an open main window that a topic is waiting (`take_pending_open_topic`).
pub const OPEN_TOPIC_PENDING_EVENT: &str = "open-topic-pending";
/// The gap between the popup and the taskbar, in logical pixels.
const POPUP_MARGIN: f64 = 8.0;
/// How often the icon and tooltip are recomputed: two status reads and one registry
/// read. The icon is replaced only when it changes.
const REFRESH_EVERY: Duration = Duration::from_secs(3);
/// A click this soon after the popup closed on focus loss is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(400);
/// Focus lost this soon after opening is the tray click finishing, not the user leaving.
const OPEN_GRACE: Duration = Duration::from_millis(600);

static POPUP_CLOSED_AT: Mutex<Option<Instant>> = Mutex::new(None);
/// What the tray shows now: (icon, light taskbar, status line).
static SHOWN: Mutex<Option<(IconState, bool, String)>> = Mutex::new(None);
/// What the right-click menu shows now (redesign only): which extra items it has, the memory line's text, and that line's item,
/// so a change of words can be made in place (`menu_plan`). It is set only after `set_menu` has succeeded, so a menu that
/// failed to build or install is tried again on the next refresh, and cleared when the menu is rebuilt for a new language.
///
/// Refreshes run on several threads (the 3 s tick, `tray_status` commands, `relabel`), and each applies its result AFTER releasing
/// this lock (the tray setters wait on the main thread, so the lock is never held across them). That leaves a window in which
/// another thread installs a newer menu. `generation` closes it: it is bumped under the lock on every install and every invalidation
/// (a new language, a stale write). A refresh notes the generation when it decides, and after applying its change it records it
/// only if the generation is still the same, checked under the lock. If it moved, a newer menu is the truth: the write is dropped
/// (`SetText`), or, for a `Rebuild` that already called `set_menu`, the state is cleared so the next refresh installs a menu it
/// knows rather than trusting a record of one the tray may no longer hold.
static MENU_SHOWN: Mutex<MenuState> = Mutex::new(MenuState { generation: 0, shown: None });

/// Two `SetText`s can also race (each plans from the same record, then calls `set_text` on the same item). `SET_TEXT_LOCK` is held
/// across the `set_text` call and the record write, so they run one after the other: the live text and the record are written in
/// the same order, and the later one wins in both. Nothing on the main thread takes this lock, so waiting on the main thread
/// inside it cannot deadlock.
static SET_TEXT_LOCK: Mutex<()> = Mutex::new(());

struct MenuState {
    /// Bumped, under the lock, whenever `shown` is replaced or cleared.
    generation: u64,
    shown: Option<MenuShown>,
}

struct MenuShown {
    /// `memory_menu_ids` of the menu installed.
    ids: Vec<&'static str>,
    /// The text of the memory line now.
    text: String,
    /// The memory line's item (`MenuItem` is `Send + Sync`: tauri marks its inner type so), for `set_text`.
    memory_item: Option<MenuItem<tauri::Wry>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureLine {
    On,
    /// Enabled, but paused for now (e.g. the screen is locked).
    Paused,
    Off,
    /// Enabled, but the service couldn't start.
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncLine {
    Off,
    SettingUp,
    /// When content last synced, if it has this session.
    Synced(Option<i64>),
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// A meeting is recording.
    Recording,
    Idle,
    Paused,
    SyncError,
    /// A meeting is recording and the recording is paused (redesign only; with the switch off a paused recording shows `Paused`).
    RecordingPaused,
}

/// The popup's view of the tray state.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrayStatus {
    /// "Capturing · Synced 2:05 PM", also the tray tooltip.
    pub line: String,
    pub capture_enabled: bool,
    /// When the recording meeting started, if one is recording.
    pub recording_started_unix_ms: Option<i64>,
    /// When a timed capture pause ends, if one is running.
    pub resume_at_unix_ms: Option<i64>,
    /// The recording meeting is paused (recording pause, not capture pause).
    pub recording_paused: bool,
    /// When that recording pause began.
    pub paused_since_unix_ms: Option<i64>,
    /// Sync is in error: the same fact as the tooltip's "Sync error" (`SyncLine::Error`).
    pub sync_error: bool,
    /// The sync half of `line` in the words people see ("Synced 2:05 PM", "Sync setting up"), or empty while sync is off or in error.
    pub sync_text: String,
    /// A Calendar or Gmail sign-in expired and needs signing in again (redesign only; false with the switch off).
    pub google_sign_in_expired: bool,
}

/// Whether the popup warns about an expired Google sign-in: only with the redesign on, and only when an account is marked.
/// The marker is asked for only when the switch is on.
pub fn google_sign_in_warning(redesign: bool, any_expired: impl FnOnce() -> bool) -> bool {
    redesign && any_expired()
}

pub fn sync_line(state: SyncState, last_synced_ms: Option<i64>, content_error: bool) -> SyncLine {
    match state {
        SyncState::NotConfigured | SyncState::NotSignedIn => SyncLine::Off,
        SyncState::SigningIn | SyncState::WaitingForApproval | SyncState::WaitingForGroupKey => SyncLine::SettingUp,
        SyncState::Ready if content_error => SyncLine::Error,
        SyncState::Ready => SyncLine::Synced(last_synced_ms),
        SyncState::Error => SyncLine::Error,
    }
}

/// Recording wins: it's the one state the user must never miss. A sync error needs
/// attention, so it wins over paused capture.
pub fn icon_state(capture: CaptureLine, sync: SyncLine, recording: bool) -> IconState {
    if recording {
        return IconState::Recording;
    }
    match (capture, sync) {
        (_, SyncLine::Error) => IconState::SyncError,
        (CaptureLine::On, _) => IconState::Idle,
        _ => IconState::Paused,
    }
}

pub fn status_line(capture: CaptureLine, sync: SyncLine, time: impl Fn(i64) -> String) -> String {
    let capture = match capture {
        CaptureLine::On => tr("Capturing", &[]),
        CaptureLine::Paused => tr("Capture paused", &[]),
        CaptureLine::Off => tr("Capture off", &[]),
        CaptureLine::Stopped => tr("Capture stopped", &[]),
    };
    let sync = match sync {
        SyncLine::Off => tr("Sync off", &[]),
        SyncLine::SettingUp => tr("Sync setting up", &[]),
        SyncLine::Synced(Some(ms)) => tr("Synced {time}", &[("time", &time(ms))]),
        SyncLine::Synced(None) => tr("Sync on", &[]),
        SyncLine::Error => tr("Sync error", &[]),
    };
    format!("{capture} · {sync}")
}

/// The sync half of `status_line` for the popup's quiet sync line: what the old status line said, or empty while sync is off or
/// in error (an error is `TrayStatus::sync_error`, and the popup puts it under "Needs attention").
pub fn sync_part(sync: SyncLine, time: impl Fn(i64) -> String) -> String {
    match sync {
        SyncLine::Off | SyncLine::Error => String::new(),
        SyncLine::SettingUp => tr("Sync setting up", &[]),
        SyncLine::Synced(Some(ms)) => tr("Synced {time}", &[("time", &time(ms))]),
        SyncLine::Synced(None) => tr("Sync on", &[]),
    }
}

/// The icon for a state. A paused recording is still a recording, so with the redesign on it has an icon of its own;
/// with it off it shows the capture-paused icon, which no other state uses while a meeting records.
pub fn icon_for(capture: CaptureLine, sync: SyncLine, recording: bool, recording_paused: bool, redesign: bool) -> IconState {
    if recording_paused {
        return if redesign { IconState::RecordingPaused } else { IconState::Paused };
    }
    icon_state(capture, sync, recording)
}

/// Whether a recording's problem is shown now. While the recording is paused the microphone and system audio deliver nothing by
/// design, so "the microphone isn't delivering audio" (`audio::gave_up_reason`, which also words a resume that stalled) is not shown;
/// any other problem (a headset at call quality, a recording that could not start, a part that did not save) is still true while
/// paused and is. The popup and the live screen follow the same rule (ui/src/meetings/redesign/liveModel.ts `degradedShown`): keep
/// the two in step.
pub fn problem_shown(degraded: Option<&str>, paused: bool) -> bool {
    use crate::capture::audio::{gave_up_reason, Source};
    match degraded {
        None => false,
        Some(reason) => !paused || (reason != gave_up_reason(Source::Microphone) && reason != gave_up_reason(Source::EndpointLoopback)),
    }
}

/// The recording's words in the tooltip: "Recording since 10:02", "Recording paused" or "Recording problem". None when
/// nothing records. `problem` is false with the redesign off.
pub fn recording_part(started_unix_ms: Option<i64>, paused: bool, problem: bool, time: impl Fn(i64) -> String) -> Option<String> {
    match started_unix_ms {
        // A problem that is still true while paused (`problem_shown`) is said as well as the pause, never instead of it.
        Some(_) if paused && problem => Some(format!("{} · {}", tr("Recording paused", &[]), tr("Recording problem", &[]))),
        Some(_) if paused => Some(tr("Recording paused", &[])),
        Some(_) if problem => Some(tr("Recording problem", &[])),
        Some(ms) => Some(tr("Recording since {time}", &[("time", &time(ms))])),
        None => None,
    }
}

/// The tooltip as it has always been: the product name, the recording if any, then the capture and sync line.
pub fn tooltip_classic(recording: Option<&str>, line: &str) -> String {
    match recording {
        Some(r) => format!("Whimbrel · {r} · {line}"),
        None => format!("Whimbrel · {line}"),
    }
}

/// The memory state in the words people see everywhere: finer than `CaptureLine`, which has no deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryState {
    On,
    /// A pause the person chose; memory comes back at this time (unix ms).
    PausedUntil(i64),
    /// Paused by the app with no time to name (the screen is locked, say).
    Paused,
    Off,
    /// Enabled, but the service could not start.
    Stopped,
}

/// A timed pause ahead of the clock wins over the service's own state, which can lag a moment behind a pause just set. A
/// deadline on a capture that is off or stopped is stale and never shown.
pub fn memory_state(capture: CaptureLine, resume_at_unix_ms: Option<i64>, now_ms: i64) -> MemoryState {
    match capture {
        CaptureLine::Off => MemoryState::Off,
        CaptureLine::Stopped => MemoryState::Stopped,
        CaptureLine::On | CaptureLine::Paused => match resume_at_unix_ms {
            Some(at) if at > now_ms => MemoryState::PausedUntil(at),
            _ if capture == CaptureLine::Paused => MemoryState::Paused,
            _ => MemoryState::On,
        },
    }
}

/// "Memory on", "Memory paused until 3:10 PM", "Memory off". A pause that ends on another day says "tomorrow".
///
/// The popup says the same words, from the same fields (ui/src/popup/popupModel.ts `popupMemory` and `memoryWords`): keep the two
/// in step. `the_memory_words_per_capture_status` here and "memory words, the same in the tray and the popup" in
/// popupModel.test.ts list the same rows, one per status.
pub fn memory_text(memory: MemoryState, now_ms: i64, time: impl Fn(i64) -> String, same_day: impl Fn(i64, i64) -> bool) -> String {
    match memory {
        MemoryState::On => tr("Memory on", &[]),
        MemoryState::Off => tr("Memory off", &[]),
        MemoryState::Stopped => tr("Memory stopped", &[]),
        MemoryState::Paused => tr("Memory paused", &[]),
        MemoryState::PausedUntil(at) if same_day(at, now_ms) => tr("Memory paused until {time}", &[("time", &time(at))]),
        MemoryState::PausedUntil(at) => tr("Memory paused until tomorrow at {time}", &[("time", &time(at))]),
    }
}

/// The memory words for the tooltip: `memory_text`, except that memory that is on but cannot read some windows says so, in a short
/// form (a tooltip is cut at 127 characters). The popup says it in full ("Memory on, some windows not read",
/// ui/src/popup/popupModel.ts `memoryWords` with a read problem): keep the two in step.
pub fn memory_text_with_problem(memory: MemoryState, problem: Option<ReadProblem>, now_ms: i64, time: impl Fn(i64) -> String, same_day: impl Fn(i64, i64) -> bool) -> String {
    match (memory, problem) {
        (MemoryState::On, Some(_)) => tr("Memory on, some windows not read", &[]),
        _ => memory_text(memory, now_ms, time, same_day),
    }
}

/// The tooltip with the redesign on: the product name, the recording state if any, a sync problem if any, then the memory state,
/// joined as `tooltip_classic` joins them. The name stays: the hover tooltip shows only this text, so without it nothing on screen
/// says which app the icon belongs to. Windows also puts the app name in front of a tray icon's accessible name, so a screen reader
/// hears "Whimbrel Whimbrel · Memory off". That doubling is the same with the switch off (main has it) and is accepted.
/// Windows cuts a tooltip at 127 characters, from the end: the sync problem sits before the memory part so a long language cuts
/// the part that is least likely to be the reason a person looked. Every English tooltip is at most 111 characters
/// (`every_tooltip_fits`, the name and its separator included); translators must keep these strings short, or a tooltip in their
/// language is cut.
pub fn tooltip_redesigned(recording: Option<String>, memory: String, sync_problem: bool) -> String {
    let mut parts: Vec<String> = vec!["Whimbrel".to_string()];
    parts.extend(recording);
    if sync_problem {
        parts.push(tr("Sync error", &[]));
    }
    parts.push(memory);
    parts.join(" · ")
}

/// What the recording means for the right-click menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingMenu {
    None,
    Recording,
    Paused,
}

/// A timed pause's three choices, in the order the popup lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseChoice {
    FifteenMinutes,
    OneHour,
    UntilTomorrow,
}

/// The ids of the items the redesign adds above the existing ones, in order: the memory line (disabled), then Pause memory
/// with its three choices or Resume memory, then (while a meeting records) Pause or Resume recording and Stop recording.
/// A "-" is a separator. Pause memory is offered only while memory is on and Resume only during a timed pause: with memory
/// off, stopped, or paused for another reason the line alone is shown.
pub fn memory_menu_ids(memory: MemoryState, recording: RecordingMenu) -> Vec<&'static str> {
    let mut ids = vec!["memory-state"];
    match memory {
        MemoryState::On => ids.push("pause-memory"),
        MemoryState::PausedUntil(_) => ids.push("resume-memory"),
        MemoryState::Paused | MemoryState::Off | MemoryState::Stopped => {}
    }
    match recording {
        RecordingMenu::None => {}
        RecordingMenu::Recording => ids.extend(["recording-pause", "recording-stop"]),
        RecordingMenu::Paused => ids.extend(["recording-resume", "recording-stop"]),
    }
    ids.push("-");
    ids
}

/// The tray menu item that copies a short message with a link, so a person can tell a friend (rollout row 1 of
/// docs/superpowers/plans/2026-11-24-referral-and-share.md). It needs no account and sends nothing.
pub const TELL_FRIEND_ID: &str = "tell-friend";

/// The item's words, in the shell's language.
pub fn tell_friend_text() -> String {
    tr("Tell a friend", &[])
}

/// The fixed items at the end of the redesign menu, top to bottom: Tell a friend sits after the Discord invite and above Quit.
pub fn menu_tail_ids() -> [&'static str; 7] {
    ["open", "ask-window", "meeting-bar", "feedback", "discord", TELL_FRIEND_ID, "quit"]
}

/// When a timed pause chosen at `now` ends, in unix ms: 15 minutes or an hour from now, or 4:00 AM local time on the next
/// calendar day for "until tomorrow" (4:00 AM the same day between midnight and 4:00 AM). The popup's `pauseDeadline`
/// follows the same rule, case for case.
pub fn pause_deadline_ms(choice: PauseChoice, now_ms: i64) -> i64 {
    match choice {
        PauseChoice::FifteenMinutes => now_ms + 15 * 60_000,
        PauseChoice::OneHour => now_ms + 60 * 60_000,
        PauseChoice::UntilTomorrow => {
            let now = Local.timestamp_millis_opt(now_ms).single().map(|t| t.naive_local());
            match now {
                Some(now) => local_to_ms(tomorrow_target(now)).unwrap_or(now_ms + 12 * 60 * 60_000),
                None => now_ms + 12 * 60 * 60_000,
            }
        }
    }
}

/// 4:00 AM on the next calendar day, or on the same day while it is not yet 4:00 AM. Pure, on local clock time.
pub fn tomorrow_target(now: NaiveDateTime) -> NaiveDateTime {
    let date = now.date();
    let four = date.and_hms_opt(4, 0, 0).unwrap_or(now);
    if now < four {
        four
    } else {
        date.succ_opt().unwrap_or(date).and_hms_opt(4, 0, 0).unwrap_or(four)
    }
}

/// A local clock time as unix ms. A time skipped by a clock change moves an hour on; a repeated one takes its first.
fn local_to_ms(at: NaiveDateTime) -> Option<i64> {
    Local
        .from_local_datetime(&at)
        .earliest()
        .or_else(|| Local.from_local_datetime(&(at + chrono::Duration::hours(1))).earliest())
        .map(|t| t.timestamp_millis())
}

/// Whether two instants fall on the same local calendar day.
fn same_local_day(a_ms: i64, b_ms: i64) -> bool {
    let day = |ms: i64| Local.timestamp_millis_opt(ms).single().map(|t| t.date_naive());
    matches!((day(a_ms), day(b_ms)), (Some(a), Some(b)) if a == b)
}

/// Where the popup's top-left corner goes, in physical pixels. It is centered over the
/// tray icon, against the taskbar's edge of the work area (bottom or top), and kept inside
/// the work area. `icon` and `work` are (x, y, width, height).
pub fn popup_origin(icon: (i32, i32, u32, u32), popup: (u32, u32), work: (i32, i32, u32, u32), margin: i32) -> (i32, i32) {
    let (ix, iy, iw, ih) = (icon.0, icon.1, icon.2 as i32, icon.3 as i32);
    let (pw, ph) = (popup.0 as i32, popup.1 as i32);
    let (wx, wy, ww, wh) = (work.0, work.1, work.2 as i32, work.3 as i32);
    let min_x = wx + margin;
    let x = (ix + iw / 2 - pw / 2).clamp(min_x, (wx + ww - pw - margin).max(min_x));
    let y = if iy + ih / 2 >= wy + wh / 2 { wy + wh - ph - margin } else { wy + margin };
    (x, y)
}

/// `popup_origin` for the redesigned popup: Y is kept inside the work area too (a popup taller than it pins to the top
/// margin; `popup_fit` shrinks it first). For any popup that already fits, this is `popup_origin` exactly. The classic popup
/// keeps `popup_origin` unchanged.
pub fn popup_origin_clamped(icon: (i32, i32, u32, u32), popup: (u32, u32), work: (i32, i32, u32, u32), margin: i32) -> (i32, i32) {
    let (x, y) = popup_origin(icon, popup, work, margin);
    let min_y = work.1 + margin;
    (x, y.clamp(min_y, (work.1 + work.3 as i32 - popup.1 as i32 - margin).max(min_y)))
}

/// The popup's size in physical pixels after it is capped to the work area less a margin on each side, so it never needs
/// more room than the screen has (the body scrolls instead; the recording row with Stop and the footer are pinned). `work`
/// is (x, y, width, height). A popup that already fits is returned as it was.
pub fn popup_fit(popup: (u32, u32), work: (i32, i32, u32, u32), margin: i32) -> (u32, u32) {
    crate::text_scale::fit_to_work_area(popup, work, margin)
}

fn current(state: &AppState) -> (CaptureLine, SyncLine) {
    let capture = match &state.capture {
        Some(c) => {
            let s = c.status();
            if !s.enabled {
                CaptureLine::Off
            } else if !s.running {
                CaptureLine::Stopped
            } else if s.paused.is_some() {
                CaptureLine::Paused
            } else {
                CaptureLine::On
            }
        }
        None => CaptureLine::Stopped,
    };
    let sync = match &state.sync {
        Some(c) => {
            let v = c.view();
            sync_line(v.state, v.content_last_synced_ms, v.content_error.is_some())
        }
        None => SyncLine::Off,
    };
    (capture, sync)
}

fn clock_time(ms: i64) -> String {
    crate::l10n::short_time(ms, &crate::l10n::ui_language())
}

/// Whether the taskbar uses the light theme (`SystemUsesLightTheme`). Unset means dark,
/// the Windows 11 default.
fn taskbar_is_light() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    result.is_ok() && value == 1
}

/// The rendered icons (`apps/windows/scripts/render-icons.mjs`, from `design/tray-glyph.svg`). The Mac's glyph is a
/// template image the OS tints; Windows has none, so the glyph follows the taskbar theme.
fn icon_bytes(icon: IconState, light_taskbar: bool) -> &'static [u8] {
    match (icon, light_taskbar) {
        (IconState::Idle, true) => &include_bytes!("../icons/tray/idle-light.ico")[..],
        (IconState::Idle, false) => &include_bytes!("../icons/tray/idle-dark.ico")[..],
        (IconState::Paused, true) => &include_bytes!("../icons/tray/paused-light.ico")[..],
        (IconState::Paused, false) => &include_bytes!("../icons/tray/paused-dark.ico")[..],
        (IconState::SyncError, _) => &include_bytes!("../icons/tray/sync-error.ico")[..],
        (IconState::Recording, _) => &include_bytes!("../icons/tray/recording.ico")[..],
        (IconState::RecordingPaused, true) => &include_bytes!("../icons/tray/recording-paused-light.ico")[..],
        (IconState::RecordingPaused, false) => &include_bytes!("../icons/tray/recording-paused-dark.ico")[..],
    }
}

/// Recomputes the tray state and updates the icon and tooltip if they changed.
pub fn refresh(app: &AppHandle) -> TrayStatus {
    let state = app.state::<AppState>();
    let (capture, sync) = current(&state);
    let recording = state.meetings.as_ref().map(|m| m.status());
    let recording_started_unix_ms = recording.as_ref().and_then(|r| r.started_unix_ms);
    let recording_paused = recording.as_ref().is_some_and(|r| r.paused);
    // The tooltip names a problem only with the redesign on; with it off the tooltip is what it was.
    let redesign = state.settings.lock().unwrap().redesign_enabled();
    let recording_problem = redesign && recording.as_ref().is_some_and(|r| problem_shown(r.degraded.as_deref(), r.paused));
    let paused_since_unix_ms = recording.as_ref().and_then(|r| r.paused_since_unix_ms);
    let capture_status = state.capture.as_ref().map(|c| c.status());
    let resume_at_unix_ms = capture_status.as_ref().and_then(|c| c.resume_at_unix_ms);
    let now_ms = chrono::Utc::now().timestamp_millis();
    // Screen-text reading that is failing, said only with the redesign on.
    let read_problem = if redesign { capture_status.as_ref().and_then(|c| c.read_problem(now_ms)) } else { None };
    let memory = memory_state(capture, resume_at_unix_ms, now_ms);
    let line = status_line(capture, sync, clock_time);
    let recording_text = recording_part(recording_started_unix_ms, recording_paused, recording_problem, clock_time);
    let tooltip = if redesign {
        tooltip_redesigned(recording_text, memory_text_with_problem(memory, read_problem, now_ms, clock_time, same_local_day), sync == SyncLine::Error)
    } else {
        tooltip_classic(recording_text.as_deref(), &line)
    };
    let icon = icon_for(capture, sync, recording_started_unix_ms.is_some(), recording_paused, redesign);
    let shown = (icon, taskbar_is_light(), tooltip.clone());
    // Decide under the lock, but never call the tray setters holding it. Off the main
    // thread they dispatch to the main thread and wait; a command running `refresh` on the
    // main thread would then wait for this lock, and neither returns (M3.3 Task 3's hang).
    let changed = {
        let mut last = SHOWN.lock().unwrap();
        let changed = last.as_ref() != Some(&shown);
        if changed {
            *last = Some(shown.clone());
        }
        changed
    };
    if changed {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            match Image::from_bytes(icon_bytes(shown.0, shown.1)) {
                Ok(image) => {
                    let _ = tray.set_icon(Some(image));
                }
                Err(e) => tracing::error!("could not load a tray icon: {e}"),
            }
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }
    // The menu's memory and recording items follow the same state (redesign only). Decided under the lock and applied after it is
    // released, like the icon. A change of words alone (the memory line, or "tomorrow at" after midnight) is made in place on
    // the line's item, so an open menu is left alone; only a change of which items there are replaces the whole menu.
    if redesign {
        let ids = memory_menu_ids(memory, recording_menu(recording_started_unix_ms.is_some(), recording_paused));
        let text = memory_text(memory, now_ms, clock_time, same_local_day);
        let plan = {
            let last = MENU_SHOWN.lock().unwrap();
            let plan = menu_plan(last.shown.as_ref().map(|s| (s.ids.as_slice(), s.text.as_str(), s.memory_item.is_some())), &ids, &text);
            (plan, last.generation)
        };
        let (plan, generation) = plan;
        match plan {
            MenuPlan::Keep => {}
            MenuPlan::SetText => {
                // The item is cloned out and the lock released before `set_text`, which waits on the main thread.
                let item = {
                    let last = MENU_SHOWN.lock().unwrap();
                    let item = if last.generation == generation { last.shown.as_ref().and_then(|s| s.memory_item.clone()) } else { None };
                    item
                };
                if let Some(item) = item {
                    let _serial = SET_TEXT_LOCK.lock().unwrap();
                    if item.set_text(&text).is_ok() {
                        // Written back only into the menu this plan was made from: if a newer one was installed meanwhile,
                        // `item` belonged to the old menu and its text says nothing about the new one.
                        let mut last = MENU_SHOWN.lock().unwrap();
                        if last.generation == generation {
                            if let Some(shown) = last.shown.as_mut() {
                                shown.text = text;
                            }
                        }
                    }
                }
            }
            MenuPlan::Rebuild => {
                if let Some(tray) = app.tray_by_id(TRAY_ID) {
                    let rebuilt = menu_for(app, true, memory, recording_menu(recording_started_unix_ms.is_some(), recording_paused), now_ms);
                    match rebuilt {
                        Ok(menu) => {
                            let memory_item = memory_line_item(&menu);
                            // Remembered only once the menu is installed: a failure is retried on the next refresh. Replacing
                            // the menu can still close one the person has open (tray menus cannot be edited in place when items
                            // appear or go); that is why only a change of items takes this path.
                            if tray.set_menu(Some(menu)).is_ok() {
                                let mut last = MENU_SHOWN.lock().unwrap();
                                // Recorded only if nothing else installed or cleared a menu since this plan: otherwise the tray
                                // may hold either menu, so nothing is recorded and the next refresh builds a known one.
                                let current = last.generation == generation;
                                last.generation += 1;
                                last.shown = if current { Some(MenuShown { ids, text, memory_item }) } else { None };
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "could not rebuild the tray menu"),
                    }
                }
            }
        }
    }
    // The in-meeting window follows the recording (redesign only).
    crate::meetings::meeting_bar::sync(app, recording_started_unix_ms);
    TrayStatus {
        line,
        capture_enabled: capture != CaptureLine::Off,
        recording_started_unix_ms,
        resume_at_unix_ms,
        recording_paused,
        paused_since_unix_ms,
        sync_error: sync == SyncLine::Error,
        sync_text: sync_part(sync, clock_time),
        google_sign_in_expired: google_sign_in_warning(redesign, crate::connectors::accounts::any_sign_in_expired),
    }
}

/// What the menu needs when its state may have changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuPlan {
    /// Nothing changed.
    Keep,
    /// The same items, other words on the memory line: set its text in place.
    SetText,
    /// Other items, or no menu of ours installed yet (or nothing to set the text on): build and install a new one.
    Rebuild,
}

/// `shown` is what the installed menu has (its ids, the memory line's text, whether the line's item is held).
pub fn menu_plan(shown: Option<(&[&str], &str, bool)>, ids: &[&str], text: &str) -> MenuPlan {
    match shown {
        Some((shown_ids, _, _)) if shown_ids != ids => MenuPlan::Rebuild,
        Some((_, shown_text, _)) if shown_text == text => MenuPlan::Keep,
        Some((_, _, true)) => MenuPlan::SetText,
        _ => MenuPlan::Rebuild,
    }
}

/// The memory line's item of a menu built by `menu_for`.
fn memory_line_item(menu: &Menu<tauri::Wry>) -> Option<MenuItem<tauri::Wry>> {
    menu.items().ok()?.into_iter().find_map(|kind| {
        let id: &str = kind.id().as_ref();
        if id == "memory-state" {
            kind.as_menuitem().cloned()
        } else {
            None
        }
    })
}

fn recording_menu(recording: bool, paused: bool) -> RecordingMenu {
    match (recording, paused) {
        (false, _) => RecordingMenu::None,
        (true, false) => RecordingMenu::Recording,
        (true, true) => RecordingMenu::Paused,
    }
}

/// The memory and recording state the menu follows, read now.
fn menu_state(app: &AppHandle) -> (MemoryState, RecordingMenu, i64) {
    let state = app.state::<AppState>();
    let (capture, _) = current(&state);
    let resume_at = state.capture.as_ref().and_then(|c| c.status().resume_at_unix_ms);
    let recording = state.meetings.as_ref().map(|m| m.status());
    let started = recording.as_ref().and_then(|r| r.started_unix_ms).is_some();
    let paused = recording.as_ref().is_some_and(|r| r.paused);
    let now_ms = chrono::Utc::now().timestamp_millis();
    (memory_state(capture, resume_at, now_ms), recording_menu(started, paused), now_ms)
}

/// The right-click menu, in the current language.
fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let redesign = app.state::<AppState>().settings.lock().unwrap().redesign_enabled();
    if !redesign {
        // The menu it always was: no status reads. The state arguments are not used with the switch off.
        return menu_for(app, false, MemoryState::On, RecordingMenu::None, 0);
    }
    let (memory, recording, now_ms) = menu_state(app);
    menu_for(app, true, memory, recording, now_ms)
}

/// The menu for a state. With the redesign off it is the menu it always was; the memory and recording items are the
/// redesign's, above the existing ones.
fn menu_for(app: &AppHandle, redesign: bool, memory: MemoryState, recording: RecordingMenu, now_ms: i64) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, "open", tr("Open Whimbrel", &[]), true, None::<&str>)?;
    let ask_window = MenuItem::with_id(app, "ask-window", tr("Ask about this window…", &[]), true, None::<&str>)?;
    let feedback = MenuItem::with_id(app, "feedback", tr("Send Feedback…", &[]), true, None::<&str>)?;
    let discord = MenuItem::with_id(app, "discord", tr("Join the Whimbrel Discord…", &[]), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", tr("Quit Whimbrel", &[]), true, None::<&str>)?;
    // "Show in-meeting window" brings the floating window back after it was closed (redesign only).
    if redesign {
        let bar = MenuItem::with_id(app, "meeting-bar", tr("Show in-meeting window", &[]), true, None::<&str>)?;
        let menu = Menu::new(app)?;
        for id in memory_menu_ids(memory, recording) {
            match id {
                "memory-state" => {
                    let text = memory_text(memory, now_ms, clock_time, same_local_day);
                    menu.append(&MenuItem::with_id(app, id, text, false, None::<&str>)?)?;
                }
                "pause-memory" => {
                    let choices = Submenu::new(app, tr("Pause memory", &[]), true)?;
                    choices.append(&MenuItem::with_id(app, "pause-15m", tr("Pause for 15 minutes", &[]), true, None::<&str>)?)?;
                    choices.append(&MenuItem::with_id(app, "pause-1h", tr("Pause for 1 hour", &[]), true, None::<&str>)?)?;
                    choices.append(&MenuItem::with_id(app, "pause-tomorrow", tr("Pause until tomorrow", &[]), true, None::<&str>)?)?;
                    menu.append(&choices)?;
                }
                "resume-memory" => menu.append(&MenuItem::with_id(app, id, tr("Resume memory", &[]), true, None::<&str>)?)?,
                "recording-pause" => menu.append(&MenuItem::with_id(app, id, tr("Pause recording", &[]), true, None::<&str>)?)?,
                "recording-resume" => menu.append(&MenuItem::with_id(app, id, tr("Resume recording", &[]), true, None::<&str>)?)?,
                "recording-stop" => menu.append(&MenuItem::with_id(app, id, tr("Stop recording", &[]), true, None::<&str>)?)?,
                _ => menu.append(&PredefinedMenuItem::separator(app)?)?,
            }
        }
        let tell_friend = MenuItem::with_id(app, TELL_FRIEND_ID, tell_friend_text(), true, None::<&str>)?;
        for id in menu_tail_ids() {
            let item = match id {
                "open" => &open,
                "ask-window" => &ask_window,
                "meeting-bar" => &bar,
                "feedback" => &feedback,
                "discord" => &discord,
                TELL_FRIEND_ID => &tell_friend,
                _ => &quit,
            };
            menu.append(item)?;
        }
        return Ok(menu);
    }
    Menu::with_items(app, &[&open, &ask_window, &feedback, &discord, &quit])
}

/// After a language change: a new menu, and the tooltip re-rendered (its cached text differs).
/// Call off the main thread, like `refresh`.
pub fn relabel(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        match menu(app) {
            Ok(menu) => {
                // The new menu's items are not the ones `MENU_SHOWN` holds: forget them (bumping the generation) BEFORE the
                // menu is installed, so a concurrent refresh that planned from the old record cannot write into, or record
                // itself as, this menu; the refresh below installs a menu it knows.
                {
                    let mut last = MENU_SHOWN.lock().unwrap();
                    last.generation += 1;
                    last.shown = None;
                }
                let _ = tray.set_menu(Some(menu));
                // And again AFTER it is installed: a refresh that ran between the first clear and `set_menu` may have installed
                // and recorded its own menu, which this (older) one has just replaced. Bumping and clearing here makes that
                // record void, so the refresh below rebuilds a menu it knows.
                let mut last = MENU_SHOWN.lock().unwrap();
                last.generation += 1;
                last.shown = None;
            }
            Err(e) => tracing::warn!(error = %e, "could not rebuild the tray menu"),
        }
    }
    refresh(app);
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let menu = menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(icon_bytes(IconState::Idle, taskbar_is_light()))?)
        .tooltip("Whimbrel")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => crate::app::show_main_window(app),
            // The window the user was in before the tray click (hotkey_panel::foreground_watch).
            "ask-window" => crate::hotkey_panel::open_from_tray(app),
            "meeting-bar" => crate::meetings::meeting_bar::show_from_tray(app),
            // The redesign's memory and recording items (`menu_for`): each calls what the popup's control calls.
            "pause-15m" => pause_memory(app, PauseChoice::FifteenMinutes),
            "pause-1h" => pause_memory(app, PauseChoice::OneHour),
            "pause-tomorrow" => pause_memory(app, PauseChoice::UntilTomorrow),
            "resume-memory" => resume_memory(app),
            "recording-pause" => recording_action(app, RecordingAction::Pause),
            "recording-resume" => recording_action(app, RecordingAction::Resume),
            "recording-stop" => recording_action(app, RecordingAction::Stop),
            "feedback" => {
                // The bundle reads logs and the store: off the event loop.
                let app = app.clone();
                std::thread::spawn(move || {
                    if let Err(e) = crate::feedback::send_with_bundle(&app) {
                        tracing::warn!(error = %e, "send feedback from the tray");
                    }
                });
            }
            "discord" => {
                if let Err(e) = crate::syncing::controller::open_in_browser(crate::commands::ai::DISCORD_URL) {
                    tracing::warn!(error = %e, "open the Discord invite from the tray");
                }
            }
            // Copies a short message with a plain link, then says in a toast whether it worked. Off the event loop: a busy
            // clipboard is retried with short sleeps.
            TELL_FRIEND_ID => {
                let aumid = app.config().identifier.clone();
                std::thread::spawn(move || crate::tell_friend::copy_and_notify(&aumid));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } => {
                toggle_popup(tray.app_handle(), rect);
            }
            // The menu opens on the button's release. Close the popup on the press, before it. Left open, the popup
            // blocked the menu until it was closed by hand (#999), most likely because the popup's focus-loss destroy
            // lands while the menu takes the foreground, and the menu closes with it.
            TrayIconEvent::Click { button: MouseButton::Right, button_state: MouseButtonState::Down, .. } => {
                close_popup_for_menu(tray.app_handle());
            }
            _ => {}
        })
        .build(app)?;
    refresh(app);
    let handle = app.clone();
    std::thread::Builder::new().name("whimbrel-tray".into()).spawn(move || loop {
        std::thread::sleep(REFRESH_EVERY);
        refresh(&handle);
    })?;
    Ok(())
}

/// "Pause for 15 minutes" and the other choices of the menu's Pause memory: `capture_pause_until`, the command the popup's
/// picker calls, then an immediate refresh. Off the event loop, like Send Feedback: the command saves settings and the
/// refresh sets the tray's icon and menu.
fn pause_memory(app: &AppHandle, choice: PauseChoice) {
    let app = app.clone();
    std::thread::spawn(move || {
        let deadline = pause_deadline_ms(choice, chrono::Utc::now().timestamp_millis());
        if let Err(e) = crate::commands::capture_pause_until(app.state::<AppState>(), deadline) {
            tracing::warn!(error = %e, "pause memory from the tray");
        }
        refresh(&app);
    });
}

/// "Resume memory": `capture_resume_now`, the popup's Resume now.
fn resume_memory(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::commands::capture_resume_now(app.state::<AppState>()) {
            tracing::warn!(error = %e, "resume memory from the tray");
        }
        refresh(&app);
    });
}

/// What a recording item of the menu does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordingAction {
    Pause,
    Resume,
    Stop,
}

/// "Pause recording", "Resume recording" and "Stop recording": the commands the popup's buttons call
/// (`meeting_record_pause`, `meeting_record_resume`, `meeting_record_stop`). They run off the main thread, as they do
/// from the page, and refresh the tray themselves.
fn recording_action(app: &AppHandle, action: RecordingAction) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = match action {
            RecordingAction::Pause => crate::commands::meetings::meeting_record_pause(app).await,
            RecordingAction::Resume => crate::commands::meetings::meeting_record_resume(app).await,
            RecordingAction::Stop => crate::commands::meetings::meeting_record_stop(app).await,
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, ?action, "recording control from the tray");
        }
    });
}

/// A right-click on the tray icon closes the popup if it is open, as a click outside it would (so `POPUP_CLOSED_AT` is
/// set, unlike `popup_close`).
fn close_popup_for_menu(app: &AppHandle) {
    if let Some(popup) = app.get_webview_window(POPUP) {
        *POPUP_CLOSED_AT.lock().unwrap() = Some(Instant::now());
        let _ = popup.destroy();
    }
}

fn toggle_popup(app: &AppHandle, anchor: Rect) {
    if let Some(popup) = app.get_webview_window(POPUP) {
        let _ = popup.destroy();
        return;
    }
    if POPUP_CLOSED_AT.lock().unwrap().is_some_and(|t| t.elapsed() < REOPEN_GUARD) {
        return;
    }
    if let Err(e) = open_popup(app, anchor) {
        tracing::error!("could not open the tray popup: {e}");
    }
}

fn open_popup(app: &AppHandle, anchor: Rect) -> tauri::Result<()> {
    let redesign = app.state::<AppState>().settings.lock().unwrap().redesign_enabled();
    // With the redesign on the popup grows with the Windows text scale (`text_scale.rs`); with it off this is `popup_size`, no read.
    let (w, h) = crate::text_scale::size_for_look(popup_size(redesign), redesign);
    let popup = WebviewWindowBuilder::new(app, POPUP, WebviewUrl::App("index.html".into()))
        .title("Whimbrel")
        .inner_size(w, h)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    // The tray reports physical pixels; the monitor's scale turns the logical popup size
    // and margin into them.
    let icon_pos: PhysicalPosition<i32> = anchor.position.to_physical(1.0);
    let icon_size = anchor.size.to_physical::<u32>(1.0);
    if let Some(monitor) = app.monitor_from_point(icon_pos.x as f64, icon_pos.y as f64)? {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let work = (area.position.x, area.position.y, area.size.width, area.size.height);
        let margin = (POPUP_MARGIN * scale).round() as i32;
        let wanted = ((w * scale).round() as u32, (h * scale).round() as u32);
        // The redesigned popup is tall (560): on a short screen at a high scale it is shrunk to the work area (its body
        // scrolls), so the recording row and Stop are on screen. The classic popup's size and position are as they were.
        let origin = |size: (u32, u32)| {
            popup_origin((icon_pos.x, icon_pos.y, icon_size.width, icon_size.height), size, work, margin)
        };
        if redesign {
            let size = popup_fit(wanted, work, margin);
            let clamped = |size: (u32, u32)| popup_origin_clamped((icon_pos.x, icon_pos.y, icon_size.width, icon_size.height), size, work, margin);
            // Move to the tray's monitor FIRST: a size set while the window is still on the monitor it was created on is
            // rescaled by Windows when the window then crosses to a monitor with another scale (480x672 can become 720x1008).
            let (x, y) = clamped(size);
            popup.set_position(PhysicalPosition::new(x, y))?;
            if size != wanted {
                popup.set_size(PhysicalSize::new(size.0, size.1))?;
            }
            // The origin is then computed from the size actually applied, and the window placed again.
            let applied = popup.outer_size().map(|s| (s.width, s.height)).unwrap_or(size);
            let (x, y) = clamped(applied);
            popup.set_position(PhysicalPosition::new(x, y))?;
            // Everything the placement used, in physical pixels, so a popup that lands away from the icon can be explained from the log.
            tracing::info!(icon_x = icon_pos.x, icon_y = icon_pos.y, icon_w = icon_size.width, icon_h = icon_size.height, ?work, scale, ?wanted, ?applied, x, y, "tray popup placed");
        } else {
            let (x, y) = origin(wanted);
            popup.set_position(PhysicalPosition::new(x, y))?;
        }
    }
    let closer = popup.clone();
    let opened = Instant::now();
    popup.on_window_event(move |event| {
        if let WindowEvent::Focused(focused) = *event {
            let elapsed_ms = opened.elapsed().as_millis() as u64;
            tracing::info!(focused, elapsed_ms, "tray popup focus");
            if focused {
                return;
            }
            if opened.elapsed() < OPEN_GRACE {
                // The shell takes the foreground back as the tray click finishes, just
                // after the popup appears; take it again rather than close at once.
                let _ = closer.set_focus();
                return;
            }
            *POPUP_CLOSED_AT.lock().unwrap() = Some(Instant::now());
            let _ = closer.destroy();
        }
    });
    popup.show()?;
    popup.set_focus()?;
    Ok(())
}

/// The popup's status. It also updates the tray icon at once (after the popup toggles
/// capture, say) rather than on the next tick. Async, so it runs off the main thread: the
/// tray setters wait on the main thread.
#[tauri::command]
pub async fn tray_status(app: AppHandle) -> Result<TrayStatus, crate::error::CommandError> {
    tauri::async_runtime::spawn_blocking(move || refresh(&app))
        .await
        .map_err(|e| crate::error::CommandError::store(tr("Tray status failed: {error}", &[("error", &e.to_string())])))
}

/// Opens the main window; the popup closes as it loses focus.
#[tauri::command]
pub fn popup_open_main(app: AppHandle) {
    crate::app::show_main_window(&app);
}

/// The popup's next meeting chip: brings the main window up on the topic. The main window also
/// asks on mount (`take_pending_open_topic`), since showing it may have just created it.
#[tauri::command]
pub fn popup_open_topic(app: AppHandle, topic_id: i64) {
    *PENDING_TOPIC.lock().unwrap() = Some(topic_id);
    crate::app::show_main_window(&app);
    let _ = app.emit_to(crate::app::MAIN_WINDOW, OPEN_TOPIC_PENDING_EVENT, ());
}

/// The topic the popup asked for, once.
#[tauri::command]
pub fn take_pending_open_topic() -> Option<i64> {
    PENDING_TOPIC.lock().unwrap().take()
}

#[tauri::command]
pub fn popup_quit(app: AppHandle) {
    app.exit(0);
}

/// Escape in the redesigned popup closes it. Unlike a close on focus loss it leaves `POPUP_CLOSED_AT` alone: the next
/// tray click opens the popup again at once.
#[tauri::command]
pub fn popup_close(app: AppHandle) {
    if let Some(popup) = app.get_webview_window(POPUP) {
        let _ = popup.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: (i32, i32, u32, u32) = (0, 0, 1920, 1032);
    const POPUP_PX: (u32, u32) = (300, 184);

    #[test]
    fn the_line_reads_capture_then_sync() {
        let t = |_| "2:05 PM".to_string();
        assert_eq!(status_line(CaptureLine::On, SyncLine::Synced(Some(1)), t), "Capturing · Synced 2:05 PM");
        assert_eq!(status_line(CaptureLine::Off, SyncLine::Off, t), "Capture off · Sync off");
        assert_eq!(status_line(CaptureLine::Paused, SyncLine::Synced(None), t), "Capture paused · Sync on");
        assert_eq!(status_line(CaptureLine::Stopped, SyncLine::Error, t), "Capture stopped · Sync error");
    }

    #[test]
    fn a_sync_error_wins_the_icon_and_anything_but_capturing_dims_it() {
        assert_eq!(icon_state(CaptureLine::On, SyncLine::Synced(None), false), IconState::Idle);
        assert_eq!(icon_state(CaptureLine::On, SyncLine::Off, false), IconState::Idle);
        assert_eq!(icon_state(CaptureLine::Off, SyncLine::Synced(None), false), IconState::Paused);
        assert_eq!(icon_state(CaptureLine::Stopped, SyncLine::SettingUp, false), IconState::Paused);
        assert_eq!(icon_state(CaptureLine::Off, SyncLine::Error, false), IconState::SyncError);
    }

    #[test]
    fn recording_wins_the_icon_over_everything() {
        assert_eq!(icon_state(CaptureLine::Off, SyncLine::Error, true), IconState::Recording);
        assert_eq!(icon_state(CaptureLine::On, SyncLine::Synced(None), true), IconState::Recording);
    }

    #[test]
    fn sync_states_map_to_lines() {
        assert_eq!(sync_line(SyncState::Ready, Some(5), false), SyncLine::Synced(Some(5)));
        assert_eq!(sync_line(SyncState::Ready, Some(5), true), SyncLine::Error);
        assert_eq!(sync_line(SyncState::WaitingForApproval, None, false), SyncLine::SettingUp);
        assert_eq!(sync_line(SyncState::NotSignedIn, None, false), SyncLine::Off);
        assert_eq!(sync_line(SyncState::Error, None, false), SyncLine::Error);
    }

    #[test]
    fn the_popup_sits_above_a_bottom_taskbar_centered_on_its_icon() {
        assert_eq!(popup_origin((1000, 1040, 24, 32), POPUP_PX, WORK, 8), (862, 840));
    }

    #[test]
    fn the_popup_stays_inside_the_work_area() {
        // An icon near the right edge: the popup is pushed left to fit.
        assert_eq!(popup_origin((1880, 1040, 24, 32), POPUP_PX, WORK, 8), (1612, 840));
        // A work area narrower than the popup pins it to the left margin.
        assert_eq!(popup_origin((10, 1040, 24, 32), POPUP_PX, (0, 0, 200, 1032), 8), (8, 840));
    }

    #[test]
    fn the_popup_is_shrunk_to_the_work_area_and_stays_on_screen() {
        // A 1366x768 laptop at 150%: the taskbar takes 72 px, leaving 696 high (1366x696 physical); the 320x560 popup is
        // 480x840 physical and would need 840 + 12 against 696.
        let work = (0, 0, 1366, 696);
        let margin = 12;
        let size = popup_fit((480, 840), work, margin);
        assert_eq!(size, (480, 672), "capped to the work area less a margin above and below");
        // Icon in the taskbar below the work area: the popup sits against the bottom edge of the work area, its top at the margin.
        let (x, y) = popup_origin_clamped((1300, 700, 36, 36), size, work, margin);
        assert_eq!((x, y), (874, 12));
        assert!(y >= work.1 + margin && y + size.1 as i32 + margin <= work.1 + work.3 as i32);
        // Top taskbar (work area starts at y 72): same, from the top.
        let top = (0, 72, 1366, 696);
        let (_, y) = popup_origin_clamped((1300, 10, 36, 36), popup_fit((480, 840), top, margin), top, margin);
        assert_eq!(y, 84);
        // A popup that fits is left alone, and the classic one is never resized.
        assert_eq!(popup_fit((480, 560), WORK, 12), (480, 560));
        assert_eq!(popup_fit(POPUP_PX, WORK, 8), POPUP_PX);
        // Too tall and not shrunk: y still stays inside (pinned to the top margin) rather than running off the top.
        assert_eq!(popup_origin_clamped((1000, 1040, 24, 32), (300, 2000), WORK, 8).1, 8);
        // The classic origin is the old formula, unclamped: bottom edge less the height, even above the work area.
        assert_eq!(popup_origin((1000, 1040, 24, 32), (300, 2000), WORK, 8), (862, 1032 - 2000 - 8));
        // A work area narrower than the popup shrinks its width too.
        assert_eq!(popup_fit((480, 560), (0, 0, 400, 1000), 12), (376, 560));
    }

    #[test]
    fn the_popup_hangs_below_a_top_taskbar() {
        assert_eq!(popup_origin((1000, 8, 24, 32), POPUP_PX, (0, 48, 1920, 1032), 8), (862, 56));
    }

    fn ten_past_three(_: i64) -> String {
        "3:10 PM".to_string()
    }

    #[test]
    fn a_paused_recording_has_its_own_icon_only_with_the_redesign() {
        assert_eq!(icon_for(CaptureLine::On, SyncLine::Synced(None), true, true, true), IconState::RecordingPaused);
        // Recording wins over every other state, paused or not.
        assert_eq!(icon_for(CaptureLine::Off, SyncLine::Error, true, true, true), IconState::RecordingPaused);
        assert_eq!(icon_for(CaptureLine::Off, SyncLine::Error, true, false, true), IconState::Recording);
        // With the switch off it borrows the capture-paused icon, as it always did.
        assert_eq!(icon_for(CaptureLine::On, SyncLine::Synced(None), true, true, false), IconState::Paused);
        // No recording: the icon follows capture and sync as before.
        assert_eq!(icon_for(CaptureLine::On, SyncLine::Synced(None), false, false, true), IconState::Idle);
        assert_eq!(icon_for(CaptureLine::Off, SyncLine::Error, false, false, true), IconState::SyncError);
    }

    #[test]
    fn memory_follows_a_pause_that_is_still_ahead() {
        let now = 1_000_000;
        assert_eq!(memory_state(CaptureLine::On, None, now), MemoryState::On);
        assert_eq!(memory_state(CaptureLine::Paused, Some(now + 1), now), MemoryState::PausedUntil(now + 1));
        // The service can lag a moment behind a pause just set: the deadline decides.
        assert_eq!(memory_state(CaptureLine::On, Some(now + 60_000), now), MemoryState::PausedUntil(now + 60_000));
        // A deadline that has passed is no pause. Paused for another reason has no time to name.
        assert_eq!(memory_state(CaptureLine::On, Some(now), now), MemoryState::On);
        assert_eq!(memory_state(CaptureLine::Paused, None, now), MemoryState::Paused);
        assert_eq!(memory_state(CaptureLine::Paused, Some(now - 5), now), MemoryState::Paused);
        // A stale deadline on capture that is off or stopped is never shown.
        assert_eq!(memory_state(CaptureLine::Off, Some(now + 60_000), now), MemoryState::Off);
        assert_eq!(memory_state(CaptureLine::Stopped, Some(now + 60_000), now), MemoryState::Stopped);
    }

    #[test]
    fn the_memory_words() {
        let today = |_: i64, _: i64| true;
        let other_day = |_: i64, _: i64| false;
        assert_eq!(memory_text(MemoryState::On, 0, ten_past_three, today), "Memory on");
        assert_eq!(memory_text(MemoryState::Off, 0, ten_past_three, today), "Memory off");
        assert_eq!(memory_text(MemoryState::Stopped, 0, ten_past_three, today), "Memory stopped");
        assert_eq!(memory_text(MemoryState::Paused, 0, ten_past_three, today), "Memory paused");
        assert_eq!(memory_text(MemoryState::PausedUntil(5), 0, ten_past_three, today), "Memory paused until 3:10 PM");
        assert_eq!(memory_text(MemoryState::PausedUntil(5), 0, ten_past_three, other_day), "Memory paused until tomorrow at 3:10 PM");
    }

    // The popup's popupModel.test.ts says the same thing in full for memory that is on and cannot read some windows.
    #[test]
    fn memory_that_cannot_read_some_windows_says_so_only_while_it_is_on() {
        let today = |_: i64, _: i64| true;
        for problem in [ReadProblem::Unavailable, ReadProblem::Failed] {
            assert_eq!(memory_text_with_problem(MemoryState::On, Some(problem), 0, ten_past_three, today), "Memory on, some windows not read");
            assert_eq!(memory_text_with_problem(MemoryState::Off, Some(problem), 0, ten_past_three, today), "Memory off");
            assert_eq!(memory_text_with_problem(MemoryState::Stopped, Some(problem), 0, ten_past_three, today), "Memory stopped");
            assert_eq!(memory_text_with_problem(MemoryState::Paused, Some(problem), 0, ten_past_three, today), "Memory paused");
            assert_eq!(memory_text_with_problem(MemoryState::PausedUntil(5), Some(problem), 0, ten_past_three, today), "Memory paused until 3:10 PM");
        }
        assert_eq!(memory_text_with_problem(MemoryState::On, None, 0, ten_past_three, today), "Memory on");
    }

    #[test]
    fn the_google_sign_in_warning_is_asked_for_only_with_the_redesign_on() {
        let mut asked = 0;
        assert!(!google_sign_in_warning(false, || {
            asked += 1;
            true
        }));
        assert_eq!(asked, 0, "with the switch off nothing new is read");
        assert!(google_sign_in_warning(true, || true));
        assert!(!google_sign_in_warning(true, || false));
    }

    #[test]
    fn the_redesigned_tooltip_reads_recording_then_a_sync_problem_then_memory() {
        assert_eq!(tooltip_redesigned(None, "Memory on".into(), false), "Whimbrel · Memory on");
        assert_eq!(
            tooltip_redesigned(Some("Recording since 10:02 AM".into()), "Memory paused until 3:10 PM".into(), false),
            "Whimbrel · Recording since 10:02 AM · Memory paused until 3:10 PM"
        );
        // The sync problem comes before the memory part: Windows cuts a long tooltip from the end.
        assert_eq!(tooltip_redesigned(Some("Recording paused".into()), "Memory off".into(), true), "Whimbrel · Recording paused · Sync error · Memory off");
        assert_eq!(tooltip_redesigned(None, "Memory on".into(), true), "Whimbrel · Sync error · Memory on");
    }

    /// Windows shows at most 127 characters of a tooltip. Every tooltip this function can produce in English (the product name
    /// included) is at most 111; the strings a translator controls add up to at most 100, so a language that needs a third more of
    /// those still fits. Translators must keep the tooltip strings short ("Recording since {time}",
    /// "Memory paused until tomorrow at {time}", "Sync error"): a longer one is cut at the end.
    #[test]
    fn every_tooltip_fits() {
        let time = |_: i64| "12:59 PM".to_string();
        let recordings = [
            recording_part(None, false, false, time),
            recording_part(Some(1), false, false, time),
            recording_part(Some(1), true, false, time),
            recording_part(Some(1), false, true, time),
            recording_part(Some(1), true, true, time),
        ];
        let today = |_: i64, _: i64| true;
        let other_day = |_: i64, _: i64| false;
        let mut memories = vec![];
        for memory in [MemoryState::On, MemoryState::Paused, MemoryState::Off, MemoryState::Stopped, MemoryState::PausedUntil(5)] {
            memories.push(memory_text(memory, 0, time, today));
            memories.push(memory_text(memory, 0, time, other_day));
        }
        memories.push(memory_text_with_problem(MemoryState::On, Some(ReadProblem::Failed), 0, time, today));
        let mut longest = 0;
        for recording in &recordings {
            for memory in &memories {
                for sync_problem in [false, true] {
                    let tip = tooltip_redesigned(recording.clone(), memory.clone(), sync_problem);
                    let chars = tip.chars().count();
                    longest = longest.max(chars);
                    assert!(chars <= 111, "{chars} characters (the middle dot is one, not two bytes): {tip}");
                }
            }
        }
        assert!(longest > 50, "the test reached the long cases");
        // The hover tooltip shows only our text, so it starts with the product name, as the classic one does.
        assert!(tooltip_redesigned(None, "Memory off".into(), false).starts_with("Whimbrel · "));
        assert_eq!(tooltip_classic(None, "x").split(" · ").next(), tooltip_redesigned(None, "x".into(), false).split(" · ").next());
    }

    // One row per capture status the tray knows: (what `current` reads, the timed pause's end) to the words. The popup's
    // popupModel.test.ts, "memory words, the same in the tray and the popup", lists the same rows: change both together.
    #[test]
    fn the_memory_words_per_capture_status() {
        let now = 1_000_000;
        let later = now + 60_000;
        let today = |_: i64, _: i64| true;
        let other_day = |_: i64, _: i64| false;
        let words = |capture: CaptureLine, resume: Option<i64>, same_day: &dyn Fn(i64, i64) -> bool| memory_text(memory_state(capture, resume, now), now, ten_past_three, same_day);
        assert_eq!(words(CaptureLine::On, None, &today), "Memory on");
        assert_eq!(words(CaptureLine::Paused, Some(later), &today), "Memory paused until 3:10 PM");
        assert_eq!(words(CaptureLine::Paused, Some(later), &other_day), "Memory paused until tomorrow at 3:10 PM");
        assert_eq!(words(CaptureLine::Paused, None, &today), "Memory paused");
        assert_eq!(words(CaptureLine::Off, None, &today), "Memory off");
        assert_eq!(words(CaptureLine::Stopped, None, &today), "Memory stopped");
        // On with a failing OCR: the tooltip's short form (the popup's row says it in full).
        let problem = |capture: CaptureLine| memory_text_with_problem(memory_state(capture, None, now), Some(ReadProblem::Failed), now, ten_past_three, today);
        assert_eq!(problem(CaptureLine::On), "Memory on, some windows not read");
        assert_eq!(problem(CaptureLine::Off), "Memory off");
        assert_eq!(problem(CaptureLine::Stopped), "Memory stopped");
        assert_eq!(problem(CaptureLine::Paused), "Memory paused");
    }

    #[test]
    fn the_popup_gets_the_sync_facts_the_old_status_line_had() {
        assert_eq!(sync_part(SyncLine::Synced(Some(1)), ten_past_three), "Synced 3:10 PM");
        assert_eq!(sync_part(SyncLine::Synced(None), ten_past_three), "Sync on");
        assert_eq!(sync_part(SyncLine::SettingUp, ten_past_three), "Sync setting up");
        // Off says nothing, and an error is `sync_error`, not a quiet line.
        assert_eq!(sync_part(SyncLine::Off, ten_past_three), "");
        assert_eq!(sync_part(SyncLine::Error, ten_past_three), "");
    }

    #[test]
    fn the_menu_is_replaced_only_when_its_items_change() {
        let on = ["memory-state", "pause-memory", "-"];
        let paused = ["memory-state", "resume-memory", "-"];
        // Nothing installed yet: build.
        assert_eq!(menu_plan(None, &on, "Memory on"), MenuPlan::Rebuild);
        // The same items and words: leave an open menu alone.
        assert_eq!(menu_plan(Some((&on[..], "Memory on", true)), &on, "Memory on"), MenuPlan::Keep);
        // The same items, other words (the day rolled over): in place.
        assert_eq!(menu_plan(Some((&paused[..], "Memory paused until 4:00 AM", true)), &paused, "Memory paused until tomorrow at 4:00 AM"), MenuPlan::SetText);
        // Other items: replace. And with no item to set the text on, replace rather than leave the words stale.
        assert_eq!(menu_plan(Some((&on[..], "Memory on", true)), &paused, "Memory paused until 3:10 PM"), MenuPlan::Rebuild);
        assert_eq!(menu_plan(Some((&paused[..], "a", false)), &paused, "b"), MenuPlan::Rebuild);
    }

    #[test]
    fn the_recording_part_of_the_tooltip() {
        assert_eq!(recording_part(None, false, false, ten_past_three), None);
        assert_eq!(recording_part(Some(1), false, false, ten_past_three).as_deref(), Some("Recording since 3:10 PM"));
        assert_eq!(recording_part(Some(1), true, false, ten_past_three).as_deref(), Some("Recording paused"));
        assert_eq!(recording_part(Some(1), false, true, ten_past_three).as_deref(), Some("Recording problem"));
        // A problem that is still true while paused is said as well as the pause, never instead of it.
        assert_eq!(recording_part(Some(1), true, true, ten_past_three).as_deref(), Some("Recording paused · Recording problem"));
    }

    #[test]
    fn a_pause_hides_only_the_problems_it_causes() {
        use crate::capture::audio::{gave_up_reason, Source};
        let mic = gave_up_reason(Source::Microphone);
        let system = gave_up_reason(Source::EndpointLoopback);
        // Not paused: every problem is shown.
        assert!(problem_shown(Some(&mic), false));
        assert!(problem_shown(Some(&system), false));
        assert!(problem_shown(Some("Your headset switched to call quality, so the meeting's audio may sound muffled."), false));
        assert!(!problem_shown(None, false));
        // Paused: audio that is not delivered is the pause itself, so it is not a problem now.
        assert!(!problem_shown(Some(&mic), true));
        assert!(!problem_shown(Some(&system), true));
        assert!(!problem_shown(None, true));
        // A problem the pause did not cause is still shown.
        assert!(problem_shown(Some("Your headset switched to call quality, so the meeting's audio may sound muffled."), true));
        assert!(problem_shown(Some("Whimbrel couldn't start recording."), true));
        assert!(problem_shown(Some("Part of this recording didn't finish saving, so some audio may be missing."), true));
    }

    #[test]
    fn the_classic_tooltip_is_what_it_always_was() {
        assert_eq!(tooltip_classic(None, "Capturing · Sync on"), "Whimbrel · Capturing · Sync on");
        assert_eq!(tooltip_classic(Some("Recording paused"), "Capturing · Sync on"), "Whimbrel · Recording paused · Capturing · Sync on");
    }

    #[test]
    fn tell_a_friend_has_its_own_id_and_words_and_sits_after_discord_above_quit() {
        assert_eq!(TELL_FRIEND_ID, "tell-friend");
        assert_eq!(tell_friend_text(), "Tell a friend");
        assert_eq!(menu_tail_ids(), ["open", "ask-window", "meeting-bar", "feedback", "discord", "tell-friend", "quit"]);
        // The id is unique among the menu's own ids, so the click handler cannot mistake it.
        let mut ids = menu_tail_ids().to_vec();
        ids.extend(memory_menu_ids(MemoryState::On, RecordingMenu::Recording));
        ids.extend(["pause-15m", "pause-1h", "pause-tomorrow"]);
        ids.retain(|id| *id != "-");
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }

    #[test]
    fn the_menu_items_per_state() {
        use MemoryState::*;
        use RecordingMenu as R;
        assert_eq!(memory_menu_ids(On, R::None), ["memory-state", "pause-memory", "-"]);
        assert_eq!(memory_menu_ids(PausedUntil(9), R::None), ["memory-state", "resume-memory", "-"]);
        // Off, stopped and paused for another reason offer no pause or resume: the line says why.
        for memory in [Off, Stopped, Paused] {
            assert_eq!(memory_menu_ids(memory, R::None), ["memory-state", "-"]);
        }
        assert_eq!(memory_menu_ids(On, R::Recording), ["memory-state", "pause-memory", "recording-pause", "recording-stop", "-"]);
        assert_eq!(memory_menu_ids(Off, R::Paused), ["memory-state", "recording-resume", "recording-stop", "-"]);
    }

    #[test]
    fn the_popup_is_taller_only_with_the_redesign() {
        assert_eq!(popup_size(false), (300.0, 272.0));
        let (w, h) = popup_size(true);
        assert!(w >= 300.0 && h > 272.0);
        // Room for the pinned parts at 200% text: the recording row with Stop, the memory line and the footer.
        assert!(h >= 560.0);
    }

    #[test]
    fn a_recording_menu_follows_recording_and_pause() {
        assert_eq!(recording_menu(false, false), RecordingMenu::None);
        assert_eq!(recording_menu(false, true), RecordingMenu::None);
        assert_eq!(recording_menu(true, false), RecordingMenu::Recording);
        assert_eq!(recording_menu(true, true), RecordingMenu::Paused);
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    #[test]
    fn until_tomorrow_is_four_in_the_morning_on_the_next_day() {
        assert_eq!(tomorrow_target(at(2026, 11, 18, 10, 22)), at(2026, 11, 19, 4, 0));
        assert_eq!(tomorrow_target(at(2026, 11, 18, 23, 59)), at(2026, 11, 19, 4, 0));
        assert_eq!(tomorrow_target(at(2026, 11, 18, 4, 0)), at(2026, 11, 19, 4, 0));
        // Between midnight and 4:00 AM a late night is still today.
        assert_eq!(tomorrow_target(at(2026, 11, 18, 0, 30)), at(2026, 11, 18, 4, 0));
        assert_eq!(tomorrow_target(at(2026, 11, 18, 3, 59)), at(2026, 11, 18, 4, 0));
        // A month end.
        assert_eq!(tomorrow_target(at(2026, 11, 30, 12, 0)), at(2026, 12, 1, 4, 0));
    }

    #[test]
    fn the_timed_choices_end_in_the_future() {
        let now = 1_700_000_000_000;
        assert_eq!(pause_deadline_ms(PauseChoice::FifteenMinutes, now), now + 900_000);
        assert_eq!(pause_deadline_ms(PauseChoice::OneHour, now), now + 3_600_000);
        let tomorrow = pause_deadline_ms(PauseChoice::UntilTomorrow, now);
        assert!(tomorrow > now && tomorrow <= now + 29 * 3_600_000, "next 4:00 AM is at most a day and a few hours away");
    }
}
