//! Helper process for the WAL crash-recovery test (NFR-REL-1). Opens a store at the
//! given path and inserts items in a tight loop until killed. Not meant to exit
//! cleanly — the test harness SIGKILLs this process mid-write on purpose.

use std::env;
use std::path::Path;

use memory::{NewCaptureItem, Store};

/// Rows committed before the "ready" sentinel is written — chosen so the test harness
/// can wait for real committed data (not a fixed sleep racing system load) before it
/// kills this process.
const ROWS_BEFORE_READY: i64 = 50;

fn main() {
    let path = env::args().nth(1).expect("usage: crash_writer <db-path>");
    let ready_path = Path::new(&path).with_extension("ready");
    // No models_dir needed for this crash-recovery smoke test — inserts run in a tight
    // loop and don't need real embeddings; an empty path degrades gracefully via
    // EmbeddingModel::load's ModelFilesMissing, same as Store::open_in_memory().
    let store = Store::open(Path::new(&path), Path::new("")).expect("failed to open store");

    let mut i: i64 = 0;
    loop {
        let item = NewCaptureItem {
            device_id: "crash-writer".to_string(),
            ts_unix_ms: i,
            app_bundle_id: "com.whimbrel.crashtest".to_string(),
            window_title: format!("Item {i}"),
            url: None,
            text: format!("crash test row {i}"),
            hash: i,
            source: "ax".to_string(),
        };
        store.insert_capture_item(&item).expect("insert failed");
        i += 1;
        if i == ROWS_BEFORE_READY {
            std::fs::write(&ready_path, b"ready").expect("failed to write ready sentinel");
        }
    }
}
