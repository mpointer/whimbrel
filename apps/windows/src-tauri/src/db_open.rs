//! The secondary store lanes' opens (X4). How the MAIN store opens is core's launch decision,
//! run by `crate::launch` at launch; the lanes then open with the key the main store opened
//! with (`StoreHandle::open_lane`), or plain when it opened plain. The lanes use the default
//! page cache: Faster search's cache is the main store's only.

use memory::{KeyedOpenOptions, Store};

use crate::db_key::KeyBytes;

/// A secondary lane's connection, keyed when the main store was.
pub fn open_lane(db: &std::path::Path, embedder: std::sync::Arc<dyn memory::Embedder>, key: Option<&KeyBytes>) -> Result<Store, String> {
    match key {
        Some(key) => Store::open_with_embedder_keyed(db, embedder, &key.db_key(), &KeyedOpenOptions::default()).map_err(|e| e.to_string()),
        None => Store::open_with_embedder(db, embedder).map_err(|e| e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lane_opens_with_the_main_stores_key_and_refuses_another() {
        let d = std::env::temp_dir().join(format!("whimbrel-dbopen-lane-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let db = d.join("kestrel.sqlite");
        let key = KeyBytes::new([42; 32]);
        let opts = KeyedOpenOptions::default();
        let main = match api::store_key::open_store_with_options(&db, &d.join("models"), api::store_key::KeyLookup::Found(key.db_key()), &opts) {
            api::store_key::OpenOutcome::Opened(s) => s,
            other => panic!("{other:?}"),
        };
        let lane = open_lane(&db, main.embedder(), Some(&key)).unwrap();
        drop(lane);
        assert!(open_lane(&db, main.embedder(), Some(&KeyBytes::new([7; 32]))).is_err(), "a wrong key opens no lane");
        drop(main);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The client controls' own connection (delete, expiry) opens like a lane, so it carries the key on an
    /// encrypted database. A plain `Store::open_with_embedder` there fails with "file is not a database".
    #[test]
    fn the_client_controls_own_connection_opens_like_a_lane() {
        let src = include_str!("commands/client_controls.rs");
        let code = src.split_once("#[cfg(test)]").map(|(p, _)| p).unwrap_or(src);
        assert!(code.contains("crate::db_open::open_lane(&db, embedder, key.as_deref())"));
        assert!(!code.contains("Store::open_with_embedder("), "an unkeyed open of the database");
    }
}
