//! Whimbrel Windows shell — docs/12-windows-app-design.md.
//!
//! M3.1 (capture spike) landed the capture layer; M3.2 builds the Tauri app on top of
//! the same modules (`docs/superpowers/plans/2026-09-10-m3.2-shell-skeleton.md`).

#![cfg(windows)]

pub mod alert;
pub mod app;
pub mod capture;
pub mod chat;
pub mod commands;
pub mod connectors;
pub mod device_labels;
pub mod error;
pub mod identity;
pub mod job;
pub mod jobs;
pub mod logging;
pub mod mcp_server;
pub mod meetings;
pub mod paths;
pub mod profile;
pub mod restart;
pub mod retention;
pub mod routines;
pub mod secrets;
pub mod settings;
pub mod skin_maker;
pub mod skin_sync;
pub mod state;
pub mod store_actor;
pub mod syncing;
pub mod tray;
