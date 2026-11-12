//! The hotkey panel (docs/superpowers/specs/2026-11-11-hotkey-panel-design.md §3, §4, §10):
//! a global shortcut opens a small window that asks about the window that was in front.
//!
//! - The shortcut (default Control+Alt+Space) is registered from Rust at startup through
//!   `tauri-plugin-global-shortcut`. A shortcut another app owns is a status Settings
//!   reads (`hotkey_status`), never a crash.
//! - The press records the foreground window *before* the panel takes focus.
//! - `hotkey_panel_read` reads it (`reader.rs`) on its own thread under a 3-second
//!   deadline and returns a request id; `window_ask` (`commands/window_ask.rs`) carries
//!   that id back, and a stale one is refused. Nothing is sent until the user asks.
//! - Window text stays in this process (`api::window_read::PanelRequests`), never goes back to the page and is
//!   never logged. The panel window going away, however it is closed, drops it and stops
//!   an answer still streaming.

pub mod foreground_watch;
pub mod reader;
pub mod window;

use std::sync::Mutex;

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use api::window_ask::Destination;
use api::window_read::{ExcludeKind, PanelRequests, ReadOutcome, READ_DEADLINE};

use crate::capture::exclusions::Exclusions;
use crate::error::{CommandError, ProviderFailureReason};
use crate::l10n::tr;
use crate::settings::{ApiKeys, PromptSurface};
use crate::state::AppState;
use reader::{Target, UiaSource};

/// The default shortcut (spec §3, decision 1), in the plugin's accelerator form.
/// AltGr is Ctrl+Alt to Windows on many European layouts, so there AltGr+Space may open the
/// panel too, and it takes AltGr+Space from any app that types with it. Settings can change it.
pub const DEFAULT_SHORTCUT: &str = "Control+Alt+Space";

/// The panel's process-side state, managed by Tauri beside `AppState`.
#[derive(Default)]
pub struct HotkeyPanel {
    /// The current request (recorded window, read, shown destination, streaming answer),
    /// under one lock (`api::window_read::PanelRequests`).
    pub requests: PanelRequests<Target>,
    status: Mutex<HotkeyStatus>,
}

impl HotkeyPanel {
    /// Ends request `id` if it is still the current one: the text is dropped and an answer
    /// still streaming is cancelled (its prepared ask is then discarded by the stream's
    /// `Restore` arm). Returns the recorded window. A later request is left alone.
    fn drop_request(&self, id: u64) -> Option<Target> {
        let dropped = self.requests.drop_request(id)?;
        if let Some(run_id) = dropped.run {
            api::inference::cancel(&run_id);
        }
        Some(dropped.target)
    }
}

/// Whether the shortcut is registered, for Settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    pub shortcut: String,
    pub registered: bool,
    /// Why registering failed (Windows' or the parser's text), for Settings to show beside
    /// its own sentence. `None` when registered.
    pub error: Option<String>,
}

/// Where the question and window text will go (core's `window_ask::Destination`).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DestinationView {
    LocalOnly,
    Local,
    Hosted { provider: String },
}

impl From<&Destination> for DestinationView {
    fn from(d: &Destination) -> Self {
        match d {
            Destination::LocalOnly => DestinationView::LocalOnly,
            Destination::Local => DestinationView::Local,
            Destination::Hosted { provider } => DestinationView::Hosted { provider: provider.clone() },
        }
    }
}

/// What was read, without the text.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowReadView {
    #[serde(rename_all = "camelCase")]
    Read { app_name: String, title: String, is_selection: bool, truncated: bool },
    #[serde(rename_all = "camelCase")]
    Excluded { app_name: String, reason: ExcludeReasonView },
    #[serde(rename_all = "camelCase")]
    Unreadable { app_name: String },
}

impl From<&ReadOutcome> for WindowReadView {
    fn from(o: &ReadOutcome) -> Self {
        match o {
            ReadOutcome::Read(w) => WindowReadView::Read {
                app_name: w.app_name.clone(),
                title: w.title.clone(),
                is_selection: w.is_selection,
                truncated: api::window_read::will_truncate(w),
            },
            ReadOutcome::Excluded(app, why) => WindowReadView::Excluded { app_name: app.clone(), reason: ExcludeReasonView::from(*why) },
            ReadOutcome::Unreadable(app) => WindowReadView::Unreadable { app_name: app.clone() },
        }
    }
}

/// Why an app is excluded, so the panel can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExcludeReasonView {
    /// Whimbrel's own window.
    OwnApp,
    /// The secure desktop, a credential prompt or UAC.
    System,
    PasswordManager,
    PrivateWindow,
    /// Excluded in Settings → Capture.
    User,
}

impl From<ExcludeKind> for ExcludeReasonView {
    fn from(k: ExcludeKind) -> Self {
        match k {
            ExcludeKind::OwnApp => ExcludeReasonView::OwnApp,
            ExcludeKind::System => ExcludeReasonView::System,
            ExcludeKind::PasswordManager => ExcludeReasonView::PasswordManager,
            ExcludeKind::PrivateWindow => ExcludeReasonView::PrivateWindow,
            ExcludeKind::User => ExcludeReasonView::User,
        }
    }
}

/// `hotkey_panel_read`'s answer: the context and destination lines' facts.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyPanelRead {
    /// Echoed back by `window_ask`; a newer press makes it stale.
    pub request_id: u64,
    pub window: WindowReadView,
    pub destination: DestinationView,
    /// False when the destination's provider can't answer from here (a hosted provider
    /// with no key, or local-only with neither Ollama nor the local model), or when that
    /// could not be worked out within the read deadline: the panel refuses rather than
    /// let another provider answer.
    pub can_answer: bool,
    /// Why it can't, when known: the same reason `window_ask` would fail with.
    pub cannot_answer: Option<ProviderFailureReason>,
}

/// Registers the saved shortcut at startup. Failure is recorded, not raised.
pub fn setup(app: &AppHandle) {
    app.manage(HotkeyPanel::default());
    foreground_watch::start();
    let shortcut = app.state::<AppState>().settings.lock().unwrap().hotkey_panel_shortcut();
    let status = register(app, &shortcut);
    *app.state::<HotkeyPanel>().status.lock().unwrap() = status;
}

/// Registers `shortcut` to toggle the panel, reporting how it went.
fn register(app: &AppHandle, shortcut: &str) -> HotkeyStatus {
    let parsed = match shortcut.parse::<Shortcut>() {
        Ok(s) if !s.mods.is_empty() => s,
        Ok(_) => return failed(shortcut, "a global shortcut needs a modifier key".into()),
        Err(e) => return failed(shortcut, e.to_string()),
    };
    let result = app.global_shortcut().on_shortcut(parsed, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            toggle(app);
        }
    });
    match result {
        Ok(()) => {
            tracing::info!(shortcut, "hotkey panel shortcut registered");
            HotkeyStatus { shortcut: shortcut.to_string(), registered: true, error: None }
        }
        Err(e) => failed(shortcut, e.to_string()),
    }
}

fn failed(shortcut: &str, error: String) -> HotkeyStatus {
    tracing::warn!(shortcut, error = %error, "hotkey panel shortcut not registered");
    HotkeyStatus { shortcut: shortcut.to_string(), registered: false, error: Some(error) }
}

/// The shortcut: closes an open panel, otherwise records the foreground window and opens
/// the panel over it.
fn toggle(app: &AppHandle) {
    if window::is_open(app) {
        close(app, false);
        return;
    }
    open_for(app, crate::capture::window::foreground());
}

/// The tray's "Ask about this window…": the tray click has already taken the foreground,
/// so the window is the last one in front before Whimbrel or the taskbar
/// (`foreground_watch`).
pub fn open_from_tray(app: &AppHandle) {
    // Already open: bring it forward. Never close and reopen in one go: the old window's
    // destroy is queued, and its `Destroyed` handler would then end the new request.
    if let Some(panel) = app.get_webview_window(window::HOTKEY_PANEL) {
        let _ = panel.show();
        let _ = panel.set_focus();
        return;
    }
    open_for(app, foreground_watch::last_window().map(crate::capture::window::info));
}

/// Opens the panel for `recorded`, the window it will read.
fn open_for(app: &AppHandle, recorded: Option<crate::capture::window::WindowInfo>) {
    let panel = app.state::<HotkeyPanel>();
    let near = recorded.as_ref().and_then(|w| window_centre(w.hwnd));
    let (id, old_run) = panel.requests.begin(recorded.as_ref().map(Target::from_window));
    if let Some(run_id) = old_run {
        api::inference::cancel(&run_id);
    }
    if let Err(e) = window::open(app, near) {
        tracing::error!(error = %e, "could not open the hotkey panel");
        return;
    }
    // However the window goes (Escape, the shortcut, Alt+F4), this request ends with it.
    if let Some(w) = app.get_webview_window(window::HOTKEY_PANEL) {
        let app = app.clone();
        w.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                app.state::<HotkeyPanel>().drop_request(id);
            }
        });
    }
}

fn window_centre(hwnd: windows::Win32::Foundation::HWND) -> Option<(f64, f64)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.ok()?;
    Some((((r.left + r.right) / 2) as f64, ((r.top + r.bottom) / 2) as f64))
}

/// Closes the panel and drops what it read. `refocus` hands the foreground back to the
/// recorded window (Escape, or the panel's own close).
fn close(app: &AppHandle, refocus: bool) {
    let panel = app.state::<HotkeyPanel>();
    let target = panel.requests.current_id().and_then(|id| panel.drop_request(id));
    window::close(app);
    if let (true, Some(target)) = (refocus, target) {
        use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
        unsafe {
            if IsWindow(Some(target.hwnd())).as_bool() {
                let _ = SetForegroundWindow(target.hwnd());
            }
        }
    }
}

/// Reads the window recorded at the last press, at most `READ_DEADLINE`. Local only:
/// nothing is sent.
#[tauri::command]
pub async fn hotkey_panel_read(app: AppHandle) -> Result<HotkeyPanelRead, CommandError> {
    tauri::async_runtime::spawn_blocking(move || read_current(&app))
        .await
        .map_err(|e| CommandError::store(tr("Could not start the question: {error}", &[("error", &e.to_string())])))?
}

fn read_current(app: &AppHandle) -> Result<HotkeyPanelRead, CommandError> {
    // One deadline for the whole read, keys and route check included.
    let deadline = std::time::Instant::now() + READ_DEADLINE;
    let state = app.state::<AppState>();
    let panel = app.state::<HotkeyPanel>();
    let (id, target) = match panel.requests.current_id() {
        Some(id) => (id, panel.requests.target(id)),
        None => (panel.requests.begin(None).0, None),
    };
    let (settings, instructions, user_exclusions) = {
        let s = state.settings.lock().unwrap();
        (s.routing(), s.instructions(PromptSurface::Ask), s.excluded_bundle_ids())
    };
    // The destination depends on the mode and the preferred provider, not on keys.
    let destination = api::window_ask::destination(&settings.routing_config(&ApiKeys::default(), &instructions));

    // Whether the destination can answer (Credential Manager, then Ollama), worked out
    // while the window is read, within the same deadline.
    let route_check = {
        let (settings, secrets) = (settings.clone(), state.secrets.clone());
        api::window_read::spawn_pending("whimbrel-panel-route-check", move || -> Result<(), Option<ProviderFailureReason>> {
            let keys = ApiKeys::load(&*secrets).map_err(|_| None)?;
            crate::commands::window_ask::route_or_reason(&settings, &keys).map(|_| ()).map_err(Some)
        })
    };
    let outcome = match target {
        None => ReadOutcome::Unreadable(String::new()),
        Some(target) => {
            let exclusions = Exclusions { user: user_exclusions };
            api::window_read::spawn_pending("whimbrel-window-read", move || {
                let source = UiaSource::new(&target);
                reader::read_window(&source, &exclusions)
            })
            .wait_until(deadline)
            .unwrap_or_else(|| {
                tracing::info!("hotkey panel: the window did not answer in time");
                ReadOutcome::Unreadable(String::new())
            })
        }
    };
    let (can_answer, cannot_answer) = match route_check.wait_until(deadline) {
        Some(Ok(())) => (true, None),
        Some(Err(reason)) => (false, reason),
        None => {
            tracing::info!("hotkey panel: the route check did not finish in time");
            (false, None)
        }
    };
    let view = WindowReadView::from(&outcome);
    // Never the text: what kind of read it was.
    tracing::info!(request = id, outcome = ?view_kind(&view), "hotkey panel read the window");
    if let ReadOutcome::Read(input) = outcome {
        if !panel.requests.store_read(id, input) {
            tracing::info!(request = id, "hotkey panel read arrived after the panel closed or reopened; dropped");
        }
    }
    // Only for the current request: a slow read from an earlier press must not replace it.
    panel.requests.set_shown(id, destination.clone());
    Ok(HotkeyPanelRead { request_id: id, window: view, destination: DestinationView::from(&destination), can_answer, cannot_answer })
}

fn view_kind(v: &WindowReadView) -> &'static str {
    match v {
        WindowReadView::Read { is_selection: true, .. } => "selection",
        WindowReadView::Read { .. } => "window",
        WindowReadView::Excluded { .. } => "excluded",
        WindowReadView::Unreadable { .. } => "unreadable",
    }
}

/// Escape or the panel's close: drops the text and returns focus to the window. Async, so
/// it runs off the main thread.
#[tauri::command]
pub async fn hotkey_panel_close(app: AppHandle) {
    close(&app, true);
}

/// For Settings: the shortcut and whether it is registered.
#[tauri::command]
pub fn hotkey_status(panel: State<'_, HotkeyPanel>) -> HotkeyStatus {
    panel.status.lock().unwrap().clone()
}

/// Settings' "use another shortcut". The new one replaces the old only once it registers;
/// otherwise the old one is put back and the status says why the new one failed.
#[tauri::command]
pub fn set_hotkey_shortcut(app: AppHandle, state: State<'_, AppState>, shortcut: String) -> Result<HotkeyStatus, CommandError> {
    let shortcut = shortcut.trim().to_string();
    let panel = app.state::<HotkeyPanel>();
    let old = panel.status.lock().unwrap().clone();
    if old.registered {
        if let Err(e) = app.global_shortcut().unregister(old.shortcut.as_str()) {
            tracing::warn!(error = %e, "could not unregister the old hotkey panel shortcut");
        }
    }
    let status = register(&app, &shortcut);
    if status.registered {
        let saved = {
            let mut settings = state.settings.lock().unwrap();
            let previous = settings.hotkey_panel_shortcut();
            settings.set_hotkey_panel_shortcut(&shortcut);
            let saved = settings.save();
            if saved.is_err() {
                settings.set_hotkey_panel_shortcut(&previous);
            }
            saved
        };
        if let Err(e) = saved {
            // Not saved: the next launch would register the old one, so go back to it now
            // and say what is registered.
            if let Err(e) = app.global_shortcut().unregister(shortcut.as_str()) {
                tracing::warn!(error = %e, "could not unregister the unsaved hotkey panel shortcut");
            }
            let restored = if old.registered { register(&app, &old.shortcut) } else { old.clone() };
            *panel.status.lock().unwrap() = restored;
            return Err(CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])));
        }
    } else if old.registered {
        let restored = register(&app, &old.shortcut);
        *panel.status.lock().unwrap() = restored;
        return Ok(HotkeyStatus { error: status.error, registered: false, shortcut: status.shortcut });
    }
    *panel.status.lock().unwrap() = status.clone();
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::window_ask::WindowInput;

    #[test]
    fn the_read_view_never_carries_the_text() {
        let outcome = ReadOutcome::Read(WindowInput { app_name: "outlook".into(), title: "Inbox".into(), text: "secret body".into(), is_selection: true });
        let json = serde_json::to_string(&WindowReadView::from(&outcome)).unwrap();
        assert!(!json.contains("secret body"), "{json}");
        assert_eq!(json, r#"{"kind":"read","appName":"outlook","title":"Inbox","isSelection":true,"truncated":false}"#);
    }

    #[test]
    fn an_exclusion_says_why() {
        let json = serde_json::to_value(WindowReadView::from(&ReadOutcome::Excluded("1password".into(), ExcludeKind::PasswordManager))).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "excluded", "appName": "1password", "reason": "password_manager"}));
    }

    #[test]
    fn destinations_serialize_tagged_by_kind() {
        let hosted = Destination::Hosted { provider: "anthropic".into() };
        assert_eq!(serde_json::to_value(DestinationView::from(&hosted)).unwrap(), serde_json::json!({"kind": "hosted", "provider": "anthropic"}));
        assert_eq!(serde_json::to_value(DestinationView::from(&Destination::LocalOnly)).unwrap(), serde_json::json!({"kind": "local_only"}));
    }

    #[test]
    fn the_default_shortcut_parses_with_modifiers() {
        let s: Shortcut = DEFAULT_SHORTCUT.parse().unwrap();
        assert!(!s.mods.is_empty());
    }
}
