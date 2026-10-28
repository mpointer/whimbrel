//! File logging: `logs\whimbrel.<date>.log`, rotated daily, seven files kept.
//!
//! `WHIMBREL_LOG` overrides the filter (`EnvFilter` syntax, e.g. `whimbrel_windows=debug`).
//! Debug builds also write to stderr, which `npm run tauri dev` shows.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::EnvFilter;

/// Installs the global subscriber. Keep the returned guard alive for the life of the
/// process: dropping it flushes and stops the background writer. `None` means logging
/// could not be set up (the app runs anyway).
pub fn init(dir: &Path) -> Option<WorkerGuard> {
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("whimbrel")
        .filename_suffix("log")
        .max_log_files(7)
        .build(dir)
        .ok()?;
    let (file, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("WHIMBREL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false);
    let installed = if cfg!(debug_assertions) {
        builder.with_writer(file.and(std::io::stderr)).try_init()
    } else {
        builder.with_writer(file).try_init()
    };
    installed.ok()?;
    Some(guard)
}
