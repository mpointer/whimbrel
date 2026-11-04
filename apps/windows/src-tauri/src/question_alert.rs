//! The question alert popup (parity item a): questions another device heard Mike being
//! asked, from `live_alerts::QuestionAlerts`. A small always-on-top window at the top right,
//! built like the "Meeting detected" alert (`alert.rs`) and for the same reason, not a toast.
//! It stays until answered: Open meeting or Dismiss.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::error::CommandError;
use crate::l10n::tr;
use crate::live_alerts::{local_meeting, QuestionAlerts, QuestionNotice};
use crate::state::AppState;

/// The popup's label (also listed in `capabilities/default.json`).
pub const QUESTION_ALERT: &str = "question-alert";
/// The event the main window opens a meeting on.
pub const OPEN_MEETING_EVENT: &str = "open-meeting";
const SIZE: (f64, f64) = (380.0, 210.0);
const MARGIN: f64 = 12.0;
/// How often the watcher looks for queued notices: sync rounds are a minute apart.
const POLL: Duration = Duration::from_secs(2);
/// The meeting alert's height, to sit below it when both are up.
const MEETING_ALERT_HEIGHT: f64 = 190.0;

/// Opens the popup whenever notices are queued and it isn't up.
pub fn watch(app: AppHandle, alerts: Arc<QuestionAlerts>) {
    let spawned = std::thread::Builder::new().name("whimbrel-question-alerts".into()).spawn(move || loop {
        std::thread::sleep(POLL);
        if !alerts.is_empty() && app.get_webview_window(QUESTION_ALERT).is_none() {
            if let Err(e) = open(&app) {
                tracing::error!(error = %e, "could not open the question alert");
            }
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the question alert watcher");
    }
}

fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(QUESTION_ALERT) {
        let _ = window.destroy();
    }
}

fn open(app: &AppHandle) -> tauri::Result<()> {
    let (w, h) = SIZE;
    let popup = WebviewWindowBuilder::new(app, QUESTION_ALERT, WebviewUrl::App("index.html".into()))
        .title(tr("Question for you", &[]))
        .inner_size(w, h)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    if let Some(monitor) = popup.primary_monitor()? {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let (x, mut y) = crate::alert::alert_origin(
            ((w * scale).round() as u32, (h * scale).round() as u32),
            (area.position.x, area.position.y, area.size.width, area.size.height),
            (MARGIN * scale).round() as i32,
        );
        if app.get_webview_window(crate::alert::ALERT).is_some() {
            y += ((MEETING_ALERT_HEIGHT + MARGIN) * scale).round() as i32;
        }
        popup.set_position(PhysicalPosition::new(x, y))?;
    }
    popup.show()?;
    Ok(())
}

/// The queued notices, newest first.
#[tauri::command]
pub fn question_alert_status(state: tauri::State<'_, AppState>) -> Vec<QuestionNotice> {
    state.question_alerts.list()
}

/// A meeting the main window should open: `meeting_id` null opens Meetings with nothing
/// selected (the other device's meeting hasn't synced here yet).
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpenMeetingRequest {
    pub meeting_id: Option<i64>,
}

/// Open meeting: resolves this PC's id for the notice's meeting, brings the main window up
/// on it, and drops the notice; the popup closes once none are left.
#[tauri::command]
pub async fn question_alert_open(app: AppHandle, state: tauri::State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let alerts = state.question_alerts.clone();
    let Some(notice) = alerts.remove(&id) else { return Ok(()) };
    let meeting_id = state.store()?.call(move |s| local_meeting(s, &notice)).await?;
    alerts.set_pending_open(meeting_id);
    crate::app::show_main_window(&app);
    // The main window also asks on mount (`take_pending_open_meeting`): it may have just been created.
    let _ = app.emit_to(crate::app::MAIN_WINDOW, OPEN_MEETING_EVENT, OpenMeetingRequest { meeting_id });
    if alerts.is_empty() {
        hide(&app);
    }
    Ok(())
}

/// Dismiss: drops every queued notice. The questions stay on their meeting's page.
#[tauri::command]
pub fn question_alert_dismiss(app: AppHandle, state: tauri::State<'_, AppState>) {
    state.question_alerts.clear();
    hide(&app);
}

/// The meeting a question alert asked the main window to open, once.
#[tauri::command]
pub fn take_pending_open_meeting(state: tauri::State<'_, AppState>) -> Option<OpenMeetingRequest> {
    state.question_alerts.take_pending_open().map(|meeting_id| OpenMeetingRequest { meeting_id })
}
