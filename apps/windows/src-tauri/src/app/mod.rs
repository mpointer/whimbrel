//! The Tauri shell (M3.2): process lifecycle, the main window, the tray.
//!
//! Whimbrel is tray-resident. Closing the main window destroys the window *and its
//! WebView2 renderer* (docs/12 W2: a renderer is 80–150 MB, and the window is closed
//! most of the day) while the process keeps running in the tray. Reopening builds the
//! window again from its `tauri.conf.json` entry. Only an explicit Quit exits.

pub(crate) mod update;

use tauri::{AppHandle, Manager, RunEvent, WebviewWindowBuilder};

use std::sync::Arc;

use crate::commands;
use crate::logging;
use crate::paths::Paths;
use crate::secrets::CredentialManager;
use crate::state::AppState;

/// The main window's label in `tauri.conf.json`.
pub(crate) const MAIN_WINDOW: &str = "main";
/// How often the store's queue-wait percentiles go to the log (Task 10's NFR-PRF-1 run
/// reads them there, next to the sync and answer lines they coincide with).
const QUEUE_LOG_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

pub fn run() {
    // First, before anything opens a file in it: move the data folder from its old name.
    let (paths, settled) = Paths::settle_from_env().expect("LOCALAPPDATA is not set");
    if let Err(e) = paths.ensure_dirs() {
        // No log file yet; stderr is all there is.
        eprintln!("could not create {}: {e}", paths.root.display());
    }
    // Before logging starts: a release launch hands over to a supervisor here (Task 10).
    crate::restart::supervise_or_continue(&paths.logs);
    let _log_guard = logging::init(&paths.logs);
    crate::restart::install_panic_log(&paths.logs);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), root = %paths.root.display(), "Whimbrel starting");
    // A release launch moves the folder before handing over, so the supervised child
    // usually finds it in place; the re-root count in `state` shows a move happened.
    match &settled {
        crate::paths::Settled::InPlace => {}
        crate::paths::Settled::Moved => tracing::info!("moved the data folder here from its old name"),
        crate::paths::Settled::BothExist => {
            tracing::warn!(old = ?paths.legacy_root, "the old data folder is still there beside this one; it's left as it is")
        }
        crate::paths::Settled::MoveFailed(e) => {
            tracing::warn!(error = %e, "could not move the data folder to its new name; using the old one until the next launch")
        }
    }

    let app = tauri::Builder::default()
        // Must be the first plugin: a second launch hands over and exits before any
        // other plugin or window initializes.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tracing::info!("second launch; focusing the running instance");
            show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            // Identity, then the store (which learns its own device id before anything
            // else can touch it), then settings. Failures land in `startup_errors`.
            let mut state = AppState::start(paths, Arc::new(CredentialManager));
            state.start_capture();
            state.start_jobs();
            state.start_sync();
            state.start_detection(app.handle().clone());
            state.start_retention();
            state.start_routines();
            state.start_mcp_server();
            if let Some(store) = &state.store {
                store.log_queue_wait_every("main", QUEUE_LOG_EVERY);
            }
            if let Some(slow) = &state.slow_store {
                slow.log_queue_wait_every("slow", QUEUE_LOG_EVERY);
            }
            app.manage(state);
            crate::tray::build(app.handle())?;
            commands::general::apply_autostart_at_launch(app.handle(), &app.state::<AppState>());
            crate::restart::schedule_crash_test(app.handle());
            update::start(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::capture_status,
            commands::set_capture_enabled,
            commands::library::library_recent,
            commands::library::library_episodes,
            commands::library::library_search,
            commands::library::library_capture,
            commands::library::library_meeting,
            commands::library::library_episode,
            commands::library::library_settings,
            commands::library::set_show_screen_captures,
            commands::library::set_episode_app_hidden,
            commands::sync::sync_status,
            commands::sync::sync_sign_in,
            commands::sync::sync_cancel_sign_in,
            commands::sync::sync_check_again,
            commands::sync::sync_devices,
            commands::sync::sync_set_device_name,
            commands::sync::sync_approve_device,
            commands::sync::sync_revoke_device,
            commands::ai::ai_settings,
            commands::ai::set_api_key,
            commands::ai::set_image_search_key,
            commands::ai::set_prompt,
            commands::ai::set_summary_instructions,
            commands::ai::provider_models,
            commands::ai::key_check,
            commands::ai::key_format_hint,
            commands::ai::usage_rows,
            commands::ai::open_external,
            commands::capture::capture_settings,
            commands::capture::set_excluded_apps,
            commands::models::model_status,
            commands::models::model_download_start,
            commands::models::model_download_poll,
            commands::models::model_download_cancel,
            commands::models::meeting_models_status,
            commands::models::meeting_models_download_start,
            commands::meetings::meeting_record_start,
            commands::meetings::meeting_record_stop,
            commands::meetings::meeting_recording_status,
            commands::workspace::meeting_list,
            commands::workspace::meeting_workspace,
            commands::workspace::meeting_transcript,
            commands::workspace::meeting_save_notes,
            commands::workspace::meeting_save_overrides,
            commands::workspace::meeting_reset_overrides,
            commands::workspace::meeting_seed_action_items,
            commands::workspace::meeting_add_action_item,
            commands::workspace::meeting_update_action_item,
            commands::workspace::meeting_delete_action_item,
            commands::workspace::meeting_reorder_action_items,
            commands::workspace::meeting_regenerate,
            commands::workspace::meeting_regeneration_conflict,
            commands::workspace::meeting_forget,
            commands::workspace::meeting_most_relevant,
            commands::snapshots::meeting_snapshots,
            commands::snapshots::meeting_snapshot_thumbnail,
            commands::snapshots::meeting_snapshot_delete,
            commands::snapshots::meeting_set_keep_snapshots,
            commands::snapshots::meeting_snapshot_open,
            commands::snapshots::meeting_snapshot_copy,
            commands::snapshots::meeting_snapshots_show,
            commands::snapshots::meeting_snapshot_settings,
            commands::snapshots::set_meeting_snapshots_enabled,
            commands::snapshots::choose_meeting_snapshots_folder,
            commands::snapshots::reset_meeting_snapshots_folder,
            crate::alert::alert_status,
            crate::alert::alert_accept,
            crate::alert::alert_dismiss,
            commands::meetings::meeting_settings,
            commands::meetings::set_meeting_settings,
            commands::connectors::calendar_accounts,
            commands::connectors::calendar_connect,
            commands::connectors::calendar_disconnect,
            commands::connectors::calendar_sync_now,
            commands::mcp::mcp_server_state,
            commands::mcp::mcp_set_enabled,
            commands::mcp::mcp_clients,
            commands::mcp::mcp_create_client,
            commands::mcp::mcp_revoke_client,
            commands::connectors::gmail_accounts,
            commands::connectors::gmail_connect,
            commands::connectors::gmail_disconnect,
            commands::connectors::gmail_delete_stored_mail,
            commands::retention::retention_settings,
            commands::retention::set_retention_settings,
            commands::retention::retention_run_now,
            commands::retention::retention_preview,
            commands::prep::next_meeting_prep,
            commands::profile::profile_get,
            commands::profile::profile_set_fields,
            commands::profile::profile_set_switches,
            commands::profile::profile_add_fact,
            commands::profile::profile_update_fact,
            commands::profile::profile_remove_fact,
            commands::profile::profile_suggestions,
            commands::profile::profile_accept_suggestion,
            commands::profile::profile_dismiss_suggestion,
            commands::routines::routines_list,
            commands::routines::routine_set_enabled,
            commands::routines::routine_set_schedule,
            commands::routines::routine_runs,
            commands::routines::routine_set_auto_approve,
            commands::routines::routine_set_model_pref,
            commands::routines::routine_model_options,
            commands::routines::routines_host,
            commands::routines::routines_set_host,
            commands::approvals::approvals_list,
            commands::approvals::approvals_count,
            commands::approvals::approval_approve,
            commands::approvals::approval_reject,
            commands::models::restart_app,
            commands::chat::chat_ask,
            commands::chat::chat_stop,
            commands::chat::chat_conversations,
            commands::chat::chat_conversation,
            commands::chat::chat_rename_conversation,
            commands::chat::chat_star_conversation,
            commands::chat::chat_delete_conversation,
            commands::chat::chat_starter_prompts,
            commands::chat::chat_spend,
            commands::today::today_overview,
            commands::today::today_recap,
            commands::schedule::today_schedule,
            commands::schedule::meeting_record_start_event,
            commands::skins::skins_list,
            commands::skins::skin_image,
            commands::skins::skin_sources,
            commands::skins::skin_turn,
            commands::skins::skin_turn_stop,
            commands::skins::skin_chat_load,
            commands::skins::skin_chat_post,
            commands::skins::skin_undo,
            commands::skins::skins_revision,
            commands::skins::skin_rename,
            commands::skins::skin_delete,
            commands::skins::skin_export,
            commands::skins::skin_import,
            commands::skins::skin_image_search,
            commands::skins::skin_image_generate,
            commands::skins::skin_image_thumbnail,
            commands::skins::skin_image_reject,
            commands::skins::skin_image_approve,
            commands::skins::skin_image_add_file,
            crate::tray::tray_status,
            crate::feedback::send_feedback,
            crate::tray::popup_open_main,
            crate::tray::popup_quit,
            commands::general::general_settings,
            commands::general::set_autostart,
            commands::get_routing_settings,
            commands::set_routing_settings,
            update::update_ready,
            update::install_update,
            update::check_for_updates,
            update::update_check_status,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the Whimbrel app");

    app.run(|app, event| match event {
        // Closing the last window asks to exit with no code; staying in the tray is the
        // point. Tray → Quit calls `exit(0)`, which carries a code and goes through.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        // Tauri ends the process with 0 whatever code was asked for, but the supervisor
        // needs `RESTART_REQUESTED` to relaunch. Tauri's own cleanup (the tray icon, …)
        // would run after this callback, so it runs here first.
        RunEvent::Exit => {
            // A meeting recording at Quit is stopped properly: its WAVs finalize and its
            // row completes, instead of waiting for the next launch's repair.
            if let Some(meetings) = &app.state::<AppState>().meetings {
                if meetings.is_recording() {
                    let _ = meetings.stop();
                }
            }
            let code = crate::restart::requested_exit_code();
            if code != 0 {
                tracing::info!(code, "exiting with a requested code");
                app.cleanup_before_exit();
                std::process::exit(code);
            }
        }
        _ => {}
    });
    tracing::info!("Whimbrel exiting");
}

/// Brings the main window to the front, recreating it if it was closed.
pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let Some(config) = app.config().app.windows.iter().find(|w| w.label == MAIN_WINDOW).cloned() else {
        tracing::error!("tauri.conf.json has no `{MAIN_WINDOW}` window");
        return;
    };
    match WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(e) => tracing::error!("could not reopen the main window: {e}"),
    }
}

#[cfg(test)]
mod tests {
    /// Plan Task 10's version hand-check, automated: the shell's three version fields
    /// equal the repo's VERSION.
    #[test]
    fn the_shell_versions_match_the_repo_version() {
        let repo = include_str!("../../../../../VERSION").trim();
        assert_eq!(env!("CARGO_PKG_VERSION"), repo, "src-tauri/Cargo.toml");
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(conf["version"], repo, "tauri.conf.json");
        let ui: serde_json::Value = serde_json::from_str(include_str!("../../../ui/package.json")).unwrap();
        assert_eq!(ui["version"], repo, "ui/package.json");
        let cli: serde_json::Value = serde_json::from_str(include_str!("../../../package.json")).unwrap();
        assert_eq!(cli["version"], repo, "apps/windows/package.json");
    }
}
