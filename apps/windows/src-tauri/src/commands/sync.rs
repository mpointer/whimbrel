//! Settings → Sync (plan Task 5). Every network action runs in `spawn_blocking`: off
//! the async runtime (the Worker client is blocking `ureq`) and off the store actor
//! (ADR 0001).

use std::sync::Arc;

use tauri::State;

use crate::error::CommandError;
use crate::state::AppState;
use crate::syncing::controller::{DeviceView, SyncController, SyncState, SyncView};
use crate::l10n::tr;

fn controller(state: &AppState) -> Result<Arc<SyncController>, CommandError> {
    state
        .sync
        .clone()
        .ok_or_else(|| CommandError::not_configured(tr("Sync isn't available because this PC's device identity didn't load (see About).", &[])))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, CommandError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CommandError::network(e.to_string()))?
        .map_err(CommandError::network)
}

#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> SyncView {
    match &state.sync {
        Some(c) => c.view(),
        None => SyncView {
            state: SyncState::NotConfigured,
            message: Some(tr("Sync isn't available because this PC's device identity didn't load (see About).", &[])),
            device_id: String::new(),
            content_last_synced_ms: None,
            content_error: None,
            settings_last_synced_ms: None,
            settings_error: None,
            device_limit: false,
            plan: None,
            plan_unknown: false,
        },
    }
}

/// Reads the plan again (Settings → Sync & Devices opening, or the window regaining focus).
#[tauri::command]
pub async fn sync_refresh_plan(state: State<'_, AppState>) -> Result<SyncView, CommandError> {
    let c = controller(&state)?;
    let reading = c.clone();
    let _ = tokio::task::spawn_blocking(move || reading.refresh_plan()).await;
    Ok(c.view())
}

/// Upgrade to Cloud: opens Stripe Checkout (`month` or `year`) in the browser.
#[tauri::command]
pub async fn sync_upgrade(state: State<'_, AppState>, interval: String) -> Result<SyncView, CommandError> {
    let c = controller(&state)?;
    let opening = c.clone();
    blocking(move || opening.upgrade(&interval)).await?;
    Ok(c.view())
}

/// Manage subscription: opens Stripe's customer portal in the browser.
#[tauri::command]
pub async fn sync_manage_subscription(state: State<'_, AppState>) -> Result<SyncView, CommandError> {
    let c = controller(&state)?;
    let opening = c.clone();
    blocking(move || opening.manage_subscription()).await?;
    Ok(c.view())
}

#[tauri::command]
pub async fn sync_sign_in(state: State<'_, AppState>) -> Result<SyncView, CommandError> {
    let c = controller(&state)?;
    let signing_in = c.clone();
    blocking(move || signing_in.sign_in()).await?;
    Ok(c.view())
}

#[tauri::command]
pub fn sync_cancel_sign_in(state: State<'_, AppState>) -> Result<SyncView, CommandError> {
    let c = controller(&state)?;
    c.cancel_sign_in();
    Ok(c.view())
}

#[tauri::command]
pub fn sync_check_again(state: State<'_, AppState>) -> Result<SyncView, CommandError> {
    let c = controller(&state)?;
    c.check_again();
    Ok(c.view())
}

#[tauri::command]
pub async fn sync_devices(state: State<'_, AppState>) -> Result<Vec<DeviceView>, CommandError> {
    let c = controller(&state)?;
    blocking(move || c.refresh_devices()).await
}

#[tauri::command]
pub async fn sync_set_device_name(state: State<'_, AppState>, name: String) -> Result<Vec<DeviceView>, CommandError> {
    let c = controller(&state)?;
    blocking(move || c.set_device_name(&name)).await
}

#[tauri::command]
pub async fn sync_approve_device(state: State<'_, AppState>, id: String) -> Result<Vec<DeviceView>, CommandError> {
    let c = controller(&state)?;
    blocking(move || c.approve(&id)).await
}

#[tauri::command]
pub async fn sync_revoke_device(state: State<'_, AppState>, id: String) -> Result<Vec<DeviceView>, CommandError> {
    let c = controller(&state)?;
    blocking(move || c.revoke(&id)).await
}
