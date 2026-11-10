pub mod aliases;
pub mod budget;
pub(crate) mod chrome;
pub mod cluster;
pub mod edit;
pub mod entities;
#[cfg(test)]
mod fixture;
pub mod index;
pub mod judge;
pub mod page;
pub mod read;
pub mod schema;
pub mod seeds;
pub mod snapshot;
pub mod step_text;
pub mod steps;

/// FNV-1a: stable across runs and builds, unlike `DefaultHasher`.
pub(crate) fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// Last write wins for synced topic rows: the later timestamp wins, and a tie goes to the
/// copy whose JSON hashes higher, so every device keeps the same one.
pub(crate) fn incoming_wins<T: serde::Serialize>(
    incoming: &T,
    incoming_ts: i64,
    local: &T,
    local_ts: i64,
) -> bool {
    let h = |p: &T| fnv1a(serde_json::to_vec(p).unwrap_or_default());
    incoming_ts > local_ts || (incoming_ts == local_ts && h(incoming) > h(local))
}

/// kv: "1" while the last `run_topic_batch` had work left (read by the judge estimate).
pub(crate) const BATCH_MORE_KEY: &str = "topic_batch_more";

/// One background step for the Mac: index new sources, then cluster (which matches seeds
/// first). True while either has work left.
pub fn run_topic_batch(store: &crate::Store, batch: usize) -> rusqlite::Result<bool> {
    let indexing = index::run_topic_index_job(store, batch)?;
    let clustering = cluster::run_topic_cluster_job(store, batch)?;
    let more = indexing || clustering;
    store.conn.execute(
        "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value WHERE value != excluded.value",
        rusqlite::params![BATCH_MORE_KEY, if more { "1" } else { "0" }],
    )?;
    Ok(more)
}
