//! Monthly AI budgets on Windows (plan 2026-11-05-windows-provider-budgets.md): the synced
//! budget settings pushed to core, and the 80% / 100% notices in a small popup at the top
//! right, built like the question alert (`question_alert.rs`) and for the same reason, not a
//! toast (Mike's choice, 2026-11-05). "Raise budget" opens Settings → AI.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::l10n::tr;
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;

/// The popup's label (also listed in `capabilities/default.json`).
pub const BUDGET_ALERT: &str = "budget-alert";
/// The event the main window opens a Settings section on.
pub const OPEN_SETTINGS_EVENT: &str = "open-settings";
const SIZE: (f64, f64) = (380.0, 170.0);
const MARGIN: f64 = 12.0;
/// How often notices are looked for: after this PC's calls, a sync pull, or a budget change.
const POLL: Duration = Duration::from_secs(15);

/// One notice as the popup shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BudgetNoticeView {
    pub provider: String,
    /// 80 or 100.
    pub threshold: u8,
    pub spent_cents: i64,
    pub budget_cents: i64,
    /// UTC month, "YYYY-MM"; the popup names it in the UI language.
    pub month: String,
}

fn queue() -> &'static Mutex<Vec<BudgetNoticeView>> {
    static QUEUE: OnceLock<Mutex<Vec<BudgetNoticeView>>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Only the highest threshold per provider: 80% and 100% reached together show as 100%.
pub fn highest_per_provider(notices: Vec<memory::provider_budget::BudgetNotice>) -> Vec<BudgetNoticeView> {
    let mut out: Vec<BudgetNoticeView> = Vec::new();
    for n in notices {
        let view = BudgetNoticeView { provider: n.provider, threshold: n.threshold, spent_cents: n.spent_cents, budget_cents: n.budget_cents, month: n.month };
        match out.iter_mut().find(|o| o.provider == view.provider) {
            Some(o) if o.threshold < view.threshold => *o = view,
            Some(_) => {}
            None => out.push(view),
        }
    }
    out
}

/// Adds `fresh` (one per provider) to the queue, replacing a provider's older notice.
fn enqueue(fresh: Vec<BudgetNoticeView>) {
    let mut q = queue().lock().unwrap();
    for n in fresh {
        q.retain(|o| o.provider != n.provider);
        q.push(n);
    }
}

/// Pushes this PC's budget settings to core (all paid providers, 0 for none). At launch, after a
/// budget is saved, and after each settings sync round.
pub fn push(store: &StoreHandle, settings: &Mutex<SettingsFile>) {
    let budgets = settings.lock().unwrap().monthly_budgets();
    let pushed = store.call_blocking(move |s| api::budget::set_budgets(s, &budgets).map_err(|e| e.to_string()));
    if let Err(e) | Ok(Err(e)) = pushed.map_err(|e| e.to_string()) {
        tracing::warn!(error = %e, "could not push the monthly budgets");
    }
}

/// Takes the notices core has newly reached and queues them.
pub fn check(store: &StoreHandle) {
    let now = chrono::Utc::now().timestamp_millis();
    let taken = store.call_blocking(move |s| api::budget::take_notices(s, now).map_err(|e| e.to_string()));
    match taken.map_err(|e| e.to_string()) {
        Ok(Ok(notices)) if !notices.is_empty() => {
            tracing::info!(count = notices.len(), "budget notices reached");
            enqueue(highest_per_provider(notices));
        }
        Ok(Ok(_)) => {}
        Ok(Err(e)) | Err(e) => tracing::warn!(error = %e, "could not read the budget notices"),
    }
}

/// Pushes the budgets once, then looks for notices every [`POLL`] and opens the popup when any are queued.
pub fn watch(app: AppHandle, store: StoreHandle, settings: Arc<Mutex<SettingsFile>>) {
    push(&store, &settings);
    let spawned = std::thread::Builder::new().name("whimbrel-budget-alerts".into()).spawn(move || loop {
        check(&store);
        show_if_queued(&app);
        std::thread::sleep(POLL);
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the budget alert watcher");
    }
}

/// Opens the popup when notices are queued and it isn't up.
pub fn show_if_queued(app: &AppHandle) {
    if !queue().lock().unwrap().is_empty() && app.get_webview_window(BUDGET_ALERT).is_none() {
        if let Err(e) = open(app) {
            tracing::error!(error = %e, "could not open the budget alert");
        }
    }
}

fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(BUDGET_ALERT) {
        let _ = window.destroy();
    }
}

fn open(app: &AppHandle) -> tauri::Result<()> {
    let (w, h) = SIZE;
    let popup = WebviewWindowBuilder::new(app, BUDGET_ALERT, WebviewUrl::App("index.html".into()))
        .title(tr("Raise budget", &[]))
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
        // Below whichever alerts are up.
        for (label, height) in [(crate::alert::ALERT, 190.0), (crate::question_alert::QUESTION_ALERT, 210.0)] {
            if app.get_webview_window(label).is_some() {
                y += ((height + MARGIN) * scale).round() as i32;
            }
        }
        popup.set_position(PhysicalPosition::new(x, y))?;
    }
    popup.show()?;
    Ok(())
}

/// The queued notices, oldest first.
#[tauri::command]
pub fn budget_alert_status() -> Vec<BudgetNoticeView> {
    queue().lock().unwrap().clone()
}

/// Raise budget: the main window opens Settings at AI, and the notices are dropped.
#[tauri::command]
pub fn budget_alert_raise(app: AppHandle) {
    queue().lock().unwrap().clear();
    hide(&app);
    open_ai_settings(&app);
}

/// Brings the main window up on Settings → AI.
pub fn open_ai_settings(app: &AppHandle) {
    crate::app::show_main_window(app);
    let _ = app.emit_to(crate::app::MAIN_WINDOW, OPEN_SETTINGS_EVENT, "ai");
}

#[tauri::command]
pub fn budget_alert_dismiss(app: AppHandle) {
    queue().lock().unwrap().clear();
    hide(&app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::provider_budget::BudgetNotice;

    fn notice(provider: &str, threshold: u8) -> BudgetNotice {
        BudgetNotice { provider: provider.into(), threshold, spent_cents: 2000, budget_cents: 2500, month: "2026-11".into() }
    }

    #[test]
    fn only_the_highest_threshold_per_provider() {
        let shown = highest_per_provider(vec![notice("openai", 80), notice("anthropic", 80), notice("openai", 100)]);
        let pairs: Vec<(&str, u8)> = shown.iter().map(|n| (n.provider.as_str(), n.threshold)).collect();
        assert_eq!(pairs, [("openai", 100), ("anthropic", 80)]);
    }

    #[test]
    fn a_newer_notice_replaces_the_providers_older_one() {
        queue().lock().unwrap().clear();
        enqueue(highest_per_provider(vec![notice("google", 80)]));
        enqueue(highest_per_provider(vec![notice("google", 100), notice("openai", 80)]));
        let q = budget_alert_status();
        let pairs: Vec<(&str, u8)> = q.iter().map(|n| (n.provider.as_str(), n.threshold)).collect();
        assert_eq!(pairs, [("google", 100), ("openai", 80)]);
        queue().lock().unwrap().clear();
    }
}
