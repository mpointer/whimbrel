//! Test stores for X6's suites (X4 Task 12): with the `sqlcipher` feature every store these
//! helpers open is the encrypted store (SQLCipher, a fixed test key, page size 16384, the
//! product's keyed open); without it, the plain store. Run X6's suites on the encrypted store
//! with `cargo test -p api --features sqlcipher --lib`.

use std::path::Path;

use rusqlite::Connection;

/// The fixed key of the test stores. Tests only.
#[cfg(feature = "sqlcipher")]
const TEST_KEY: [u8; 32] = [0x5c; 32];

/// Whether these helpers open the encrypted store.
pub(crate) const ENCRYPTED: bool = cfg!(feature = "sqlcipher");

/// Opens (creating) the store at `path`, encrypted when built with `sqlcipher`.
pub(crate) fn open(path: &Path, models_dir: &Path) -> memory::Store {
    #[cfg(feature = "sqlcipher")]
    {
        memory::Store::open_keyed(
            path,
            models_dir,
            &memory::DbKey::new(TEST_KEY),
            &memory::KeyedOpenOptions::default(),
        )
        .expect("keyed test store")
    }
    #[cfg(not(feature = "sqlcipher"))]
    {
        memory::Store::open(path, models_dir).expect("test store")
    }
}

/// A second, raw connection on the store at `path` (with the key when encrypted), for
/// fixtures that write or read tables directly.
pub(crate) fn raw(path: &Path) -> Connection {
    let c = Connection::open(path).expect("raw test connection");
    #[cfg(feature = "sqlcipher")]
    {
        let hex: String = TEST_KEY.iter().map(|b| format!("{b:02x}")).collect();
        c.execute_batch(&format!(
            "PRAGMA key = \"x'{hex}'\"; PRAGMA cipher_page_size = {}; PRAGMA cipher_hmac_algorithm = {};",
            memory::KEYED_PAGE_SIZE,
            memory::KEYED_HMAC_ALGORITHM
        ))
        .expect("key the raw test connection");
    }
    c
}
