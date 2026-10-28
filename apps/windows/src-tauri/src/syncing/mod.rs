//! Sync on Windows (plan Tasks 5–6): configuration, enrollment against the Worker, and
//! content sync. Named `syncing`, not `sync`, so it doesn't collide with the `sync`
//! crate (device keypairs, group keys) at the crate root.

pub mod config;
pub mod content;
pub mod controller;
pub mod enrollment;
pub mod keys;
