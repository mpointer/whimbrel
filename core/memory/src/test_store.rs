//! Test stores for X6's suites (X4 Task 12): with the `sqlcipher` feature the store these helpers
//! open is the encrypted store (SQLCipher, a fixed test key, the product's keyed open); without
//! it, the plain store. Run them on the encrypted store with
//! `cargo test -p memory --features sqlcipher --lib`.

use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;

use crate::{Embedder, Store};

#[cfg(feature = "sqlcipher")]
const TEST_KEY: [u8; 32] = [0x5c; 32];

/// [`Store::open_with_embedder`], encrypted when built with `sqlcipher`.
pub(crate) fn open_with_embedder(path: &Path, embedder: Arc<dyn Embedder>) -> Store {
    #[cfg(feature = "sqlcipher")]
    {
        Store::open_with_embedder_keyed(path, embedder, &crate::DbKey::new(TEST_KEY), &Default::default())
            .expect("keyed test store")
    }
    #[cfg(not(feature = "sqlcipher"))]
    {
        Store::open_with_embedder(path, embedder).expect("test store")
    }
}

/// A second, raw connection on the store at `path` (with the key when encrypted).
pub(crate) fn raw(path: &Path) -> Connection {
    let c = Connection::open(path).expect("raw test connection");
    #[cfg(feature = "sqlcipher")]
    {
        let hex: String = TEST_KEY.iter().map(|b| format!("{b:02x}")).collect();
        c.execute_batch(&format!("PRAGMA key = \"x'{hex}'\"; PRAGMA cipher_page_size = {}; PRAGMA cipher_hmac_algorithm = {};", crate::KEYED_PAGE_SIZE, crate::KEYED_HMAC_ALGORITHM))
            .expect("key the raw test connection");
    }
    c
}
