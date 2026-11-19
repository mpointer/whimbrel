//! Per-device settings that core needs before the store is open (X4 encryption at rest).
//!
//! Core has no settings table for the main store, and these settings must be read before
//! the store opens (the database may be encrypted and its key not yet looked up), so the
//! shells persist them in their own per-device preferences, beside the other device-local
//! keys: Mac `UserDefaults`, Windows `%LOCALAPPDATA%\Whimbrel\data\settings.json`
//! (`apps/windows/src-tauri/src/settings.rs`). They are **never synced**: whether a
//! computer has the memory for a 1 GiB cache is that computer's business. The shell passes
//! the values into [`crate::launch_decision`].
//!
//! "Faster search (uses more memory)": `PRAGMA cache_size = -1048576` (1 GiB) on the MAIN
//! store only, off by default, offered only when the computer has at least 16 GiB of
//! physical memory, applied at the next launch. The MCP server and the per-call opens keep
//! SQLite's default cache. The cache only applies to the encrypted (keyed) store; a plain
//! store opens as it always has.

use memory::KeyedOpenOptions;
use serde::{Deserialize, Serialize};

/// The settings key both shells use for [`LaunchSettings::fast_search`] (device-local).
pub const FAST_SEARCH_KEY: &str = "storage.fastSearch";

/// The main store's cache with Faster search on: 1 GiB, in KiB.
pub const FAST_SEARCH_CACHE_KIB: i64 = 1_048_576;

/// Physical memory needed before Faster search is offered: a computer with 16 GB of RAM
/// installed. The constant is 14 GiB because the usable total the OS reports is lower than
/// what is installed (Windows `GlobalMemoryStatusEx.ullTotalPhys` leaves out memory reserved
/// by hardware and firmware, often 0.5 to 2 GiB). Shells should prefer the installed amount
/// where they can (Windows `GetPhysicallyInstalledSystemMemory`, Mac
/// `ProcessInfo.physicalMemory`).
pub const FAST_SEARCH_MIN_RAM_BYTES: u64 = 14 * 1024 * 1024 * 1024;

/// The device-local settings the launch reads before any store opens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSettings {
    /// "Faster search (uses more memory)". Off by default (the shell sends `false` when its
    /// preference is unset); takes effect at the next launch. Required in JSON.
    pub fast_search: bool,
}

/// Whether Faster search may be offered (and applied) on a computer with this much
/// physical memory: a 16 GB computer ([`FAST_SEARCH_MIN_RAM_BYTES`], 14 GiB reported).
/// Settings copy note: the bigger cache only applies to an encrypted database; on a plain
/// one the toggle has no effect.
pub fn fast_search_available(physical_ram_bytes: u64) -> bool {
    physical_ram_bytes >= FAST_SEARCH_MIN_RAM_BYTES
}

/// Which open a cache size is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRole {
    /// The app's main store (the one the launch opens).
    Main,
    /// The MCP server subprocess.
    McpServer,
    /// Short-lived per-call opens (skin image search, `open_as_device`, slow lanes).
    PerCall,
}

/// The keyed-open cache for `role`: 1 GiB only for the main store with Faster search on
/// and enough memory; `None` (SQLite's default) for everything else.
pub fn cache_kib_for(role: OpenRole, settings: &LaunchSettings, physical_ram_bytes: u64) -> Option<i64> {
    match role {
        OpenRole::Main if settings.fast_search && fast_search_available(physical_ram_bytes) => {
            Some(FAST_SEARCH_CACHE_KIB)
        }
        _ => None,
    }
}

/// [`cache_kib_for`] as the keyed-open options for `role`.
pub fn keyed_open_options(role: OpenRole, settings: &LaunchSettings, physical_ram_bytes: u64) -> KeyedOpenOptions {
    KeyedOpenOptions { cache_kib: cache_kib_for(role, settings, physical_ram_bytes), ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn fast_search_off_by_default() {
        assert!(!LaunchSettings::default().fast_search);
        // Off by default means the shell sends `false` when its own preference is unset; the
        // field itself is required in the decision's JSON, like every other field.
        assert!(serde_json::from_str::<LaunchSettings>("{}").is_err());
        let s: LaunchSettings = serde_json::from_str(r#"{"fast_search": false}"#).unwrap();
        assert!(!s.fast_search);
        assert_eq!(cache_kib_for(OpenRole::Main, &LaunchSettings::default(), 64 * GIB), None);
    }

    /// "16 GB installed": the threshold is 14 GiB of reported memory, so a 16 GB Windows PC
    /// whose reported total is about 15.4 GiB still qualifies; an 8 or 12 GB one does not.
    #[test]
    fn fast_search_unavailable_below_16_gib() {
        assert!(!fast_search_available(0));
        assert!(!fast_search_available(8 * GIB));
        assert!(!fast_search_available(12 * GIB));
        assert!(!fast_search_available(14 * GIB - 1));
        assert!(fast_search_available(14 * GIB));
        assert!(fast_search_available(15 * GIB + GIB / 2), "16 GB PC reporting 15.5 GiB");
        assert!(fast_search_available(16 * GIB));
        assert!(fast_search_available(64 * GIB));
        assert_eq!(FAST_SEARCH_MIN_RAM_BYTES, 14 * GIB);
        // On but not enough memory: no big cache.
        let on = LaunchSettings { fast_search: true };
        assert_eq!(cache_kib_for(OpenRole::Main, &on, 14 * GIB - 1), None);
    }

    #[test]
    fn launch_settings_reject_unknown_fields() {
        assert!(serde_json::from_str::<LaunchSettings>(r#"{"fastSearch": true}"#).is_err());
    }

    #[test]
    fn main_store_gets_1_gib_cache_only_when_enabled() {
        let on = LaunchSettings { fast_search: true };
        let off = LaunchSettings { fast_search: false };
        assert_eq!(cache_kib_for(OpenRole::Main, &on, 16 * GIB), Some(1_048_576));
        assert_eq!(keyed_open_options(OpenRole::Main, &on, 32 * GIB).cache_kib, Some(1_048_576));
        assert_eq!(cache_kib_for(OpenRole::Main, &off, 32 * GIB), None);
        assert_eq!(keyed_open_options(OpenRole::Main, &off, 32 * GIB).cache_kib, None);
        assert_eq!(cache_kib_for(OpenRole::Main, &on, 8 * GIB), None);
        assert_eq!(cache_kib_for(OpenRole::Main, &on, 14 * GIB), Some(1_048_576));
    }

    #[test]
    fn mcp_and_per_call_opens_never_get_the_big_cache() {
        for fast_search in [false, true] {
            let s = LaunchSettings { fast_search };
            for ram in [0, 8 * GIB, 16 * GIB, 128 * GIB] {
                for role in [OpenRole::McpServer, OpenRole::PerCall] {
                    assert_eq!(cache_kib_for(role, &s, ram), None, "{role:?} {fast_search} {ram}");
                    assert_eq!(keyed_open_options(role, &s, ram).cache_kib, None);
                }
            }
        }
    }

    #[test]
    fn settings_key_is_pinned() {
        assert_eq!(FAST_SEARCH_KEY, "storage.fastSearch");
    }
}
