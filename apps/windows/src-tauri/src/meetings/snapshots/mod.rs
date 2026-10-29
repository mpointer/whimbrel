//! Meeting snapshots (FR-MTG-11): one image per slide shown in a recorded meeting.
//! docs/superpowers/plans/2026-10-28-windows-meeting-snapshots.md

pub mod faces;
pub mod filter;
pub mod service;
pub mod storage;
pub mod target;

#[cfg(test)]
pub(crate) mod fixtures;
