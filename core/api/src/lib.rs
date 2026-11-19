//! `api` — the pure-Rust orchestration layer between Whimbrel's platform shells and the
//! core crates (docs/12-windows-app-design.md §3, milestone M3.0).
//!
//! Everything here used to live inside `core/ffi` as private helpers next to the
//! `extern "C"` functions. That made the C ABI the *only* way to reach orchestration
//! logic like "run the spend-cap check, then walk the provider chain, then log usage" —
//! fine for a Swift shell, absurd for a Rust one (the Windows shell would be marshalling
//! C strings inside a single binary). This crate exposes that logic as typed Rust:
//! `&memory::Store` in, `Result<T, ApiError>` out, every `T` a `serde::Serialize` type
//! whose field names are the exact JSON the Swift app already decodes.
//!
//! `core/ffi` is now a thin adapter over this crate: parse C strings → call `api` →
//! encode with [`envelope::envelope_json`] → hand back a C string. `whimbrel_core.h` and
//! every JSON shape the macOS app sees are unchanged by design; `core/ffi`'s own test
//! suite is the regression net for that claim.
//!
//! Module naming: what the design doc sketched as `api::llm::…` is split into
//! [`spend`], [`digest`], and (as they move) `ask`/`summary`, because a module named
//! `llm` would shadow the `llm` crate inside this crate root.
//!
//! Rule carried over from ADR 0001: nothing in this crate holds a lock or a queue while
//! it talks to a provider. Functions borrow `&Store` for the duration of a call exactly
//! as the FFI layer did; the *caller* decides how `Store` access is serialized (a
//! dispatch queue on macOS, an actor task on Windows).

pub mod ask;
pub mod window_ask;
pub mod window_read;
pub mod recap;
pub mod budget;
pub mod local_model;
pub mod import;
pub mod citations;
pub mod client_controls;
pub mod client_inventory;
pub mod client_delete;
pub mod client_export;
pub mod client_retention;
pub mod away_followup;
pub mod connectors;
pub mod digest;
pub mod envelope;
pub mod exclusion;
pub mod foreign_files;
#[cfg(test)]
mod exclusion_sweep;
pub mod hosted_models;
pub mod inference;
pub mod jobs;
pub mod live_question;
pub use memory::l10n;
pub use memory::background_gate;
pub mod mcp;
pub mod model_defaults;
pub mod models;
pub mod oauth;
pub mod retention;
pub mod prompts;
pub mod providers;
pub mod meeting_prep;
pub mod profile_suggest;
pub mod meeting_workspace;
pub mod media_retention;
pub mod meetings;
pub mod routine_runs;
pub mod runner_config;
pub mod routing;
pub mod speaker_names;
pub mod spend;
pub mod storage;
pub mod store;
pub mod store_key;
pub mod delete_everything;
pub mod encrypt_migration;
pub mod encrypt_setup;
pub mod launch_decision;
pub mod settings;
#[cfg(test)]
mod source_scan;
#[cfg(test)]
pub(crate) mod test_store;
pub mod speed_check;
pub mod summary;
pub mod title_check;
pub mod topic_judge;
pub mod topic_steps;
pub mod topics;
pub mod topics_export;
pub mod sync_ops;
pub mod update;
pub mod usage;
pub mod util;
pub mod write_action;
pub mod your_data;

pub use envelope::{last_provider_failure, ApiError, ApiResult};
pub(crate) use envelope::chain_failed;
pub use routing::RoutingConfig;
pub use routine_runs::{tick_cloud, CloudPolicy, RunnerRole};
