//! Capture layer — docs/12-windows-app-design.md §4.
//!
//! M3.1 measured the extraction modules (`uia`, `ocr`, `window`, `audio`); M3.2 Task 3
//! adds the ambient service that runs them (`service`), its pure rules (`schedule`,
//! `merge`, `exclusions`, `app_id`, `session_gate`) and its system notifications
//! (`events`).

pub mod app_id;
pub mod audio;
pub mod events;
pub mod exclusions;
pub mod frame;
pub mod merge;
pub mod ocr;
pub mod policy;
pub mod proc_stats;
pub mod schedule;
pub mod service;
pub mod session_gate;
pub mod uia;
pub mod window;
