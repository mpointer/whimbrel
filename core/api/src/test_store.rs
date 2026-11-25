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

/// Adds one item as a local MCP client would (`mcp_item`, token 1, label `outlook-work`), with `email`
/// as its one person when given. Returns its `items.id`.
pub(crate) fn add_mcp_item(store: &memory::Store, id: &str, title: &str, text: &str, email: Option<&str>) -> i64 {
    use memory::mcp_items::{McpPerson, McpWriter, NewMcpItem, WriteClock};
    let item = NewMcpItem {
        label: "outlook-work".into(),
        external_id: id.into(),
        kind: "email".into(),
        ts: "2026-11-21T14:03:00Z".into(),
        title: Some(title.into()),
        text: text.into(),
        url: Some("https://outlook.office.com/mail/1".into()),
        people: email
            .map(|e| vec![McpPerson { name: Some("Ana Ruiz".into()), email: Some(e.into()), role: Some("from".into()) }])
            .unwrap_or_default(),
    };
    { let live: &memory::Store = store; while live.list_mcp_client_tokens().unwrap().len() < 12 { live.insert_mcp_client_token(&memory::NewMcpClientToken { name: "t".into(), token_hash: "h".into(), token_salt: "s".into(), scopes: vec!["write".into()] }).unwrap(); } }
    let w = McpWriter { token_id: 1, client_name: "Claude Code".into() };
    let clock = WriteClock { now_unix_ms: 1_795_269_780_000, day_start_unix_ms: 1_795_219_200_000 };
    let r = store.upsert_mcp_items(&w, &[item], &Default::default(), clock).expect("add_items");
    assert_eq!(r.accepted, 1, "{r:?}");
    store
        .mcp_item_id(1, "outlook-work", id)
        .unwrap()
        .expect("added item row")
}
