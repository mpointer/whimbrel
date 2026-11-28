//! The one-time `slice_tag` backfill (docs/superpowers/specs/2026-11-24-runner-slice-tag-design.md, "Backfill"):
//! the `kind_tag` backfill's walk ([`crate::kind_backfill`]) over the same rows, filling `slice_tag` instead. It
//! tags the `items` rows on the relay pushed before clients tagged them by slice, so the runner's slice-filtered
//! pull can skip them at the query. The cloud runner's `backfill` command runs it after the kind pass; the apps
//! never run it.
//!
//! Everything but the column is the kind pass's: the per-peer walk by primary-key range, one UPDATE per distinct
//! tag in a chunk, progress saved per chunk, the advisory claim (its own row type,
//! [`ROW_TYPE_SLICE_BACKFILL_CLAIM`], so the two passes never hold each other), the retries, the stall rule and
//! the 10 s floor. Its kv keys are its own (`slice_backfill:1:<peer>`, `slice_backfill_done`,
//! `slice_backfill_undecryptable:1`). A row that does not open or decode stays NULL (the pull fetches it) and is
//! counted.

use libsql::Connection;
use memory::Store;
use std::time::Duration;
use sync::GroupKey;

use crate::content::{
    item_payload_slice_tag, ROW_TYPE_SLICE_BACKFILL_CLAIM, SLICE_TAG_COLUMN_SEEN,
};
use crate::kind_backfill::{BackfillStop, KindBackfillProgress, Pass};
use crate::SyncEngineError;

/// One relay row of a peer's window: an `items` row still without a slice.
const CHUNK_SQL: &str = "SELECT local_seq, ciphertext FROM sync_row \
     WHERE device_id = ?1 AND local_seq > ?2 AND local_seq <= ?3 AND row_type = 'items' AND slice_tag IS NULL \
     ORDER BY local_seq LIMIT ?4";

/// The slice pass: the kind pass's walk over `slice_tag`, with its own keys and claim.
pub(crate) static SLICE_PASS: Pass = Pass {
    column: "slice_tag",
    version: "1",
    kv_prefix: "slice_backfill",
    done_key: "slice_backfill_done",
    claim_row_type: ROW_TYPE_SLICE_BACKFILL_CLAIM,
    chunk_sql: CHUNK_SQL,
    column_seen: &SLICE_TAG_COLUMN_SEEN,
    tag: item_payload_slice_tag,
};

/// One bounded step of the slice pass, as [`crate::kind_backfill_step`] is of the kind pass. Tests only: the runner
/// steps through [`run_runner_slice_backfill`], and the apps never step it.
#[cfg(test)]
pub(crate) async fn slice_backfill_step(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    budget: Duration,
) -> Result<KindBackfillProgress, SyncEngineError> {
    crate::kind_backfill::pass_step(&SLICE_PASS, conn, group_key, store, own_device_id, budget)
        .await
}

/// The cloud runner's long run of the slice pass, with the kind pass's stop rules
/// (`kind_backfill::run_runner_backfill`): steps of `step_budget` until finished, `total_budget` is spent (no step
/// starts with under 10 s left), or two steps in a row move nothing (another device's fresh slice claim). A failed
/// step is retried three times in a row before its error is returned.
pub(crate) async fn run_runner_slice_backfill(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    total_budget: Duration,
    step_budget: Duration,
    on_step: &mut dyn FnMut(&KindBackfillProgress, usize),
) -> Result<(KindBackfillProgress, BackfillStop), SyncEngineError> {
    crate::kind_backfill::run_runner_pass(
        &SLICE_PASS,
        conn,
        group_key,
        store,
        own_device_id,
        total_budget,
        step_budget,
        on_step,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{
        content_row_aad, ensure_content_schema, push_marker, refresh_marker, slice_tag_for,
        ItemPayload, KIND_TAG_COLUMN_SEEN, ROW_TYPE_ITEMS, ROW_TYPE_ITEMS_CAPABILITY,
        ROW_TYPE_KIND_BACKFILL_CLAIM, ROW_TYPE_MEETING,
    };
    use crate::kind_backfill::{CAPTURED_SQL, TEST_FAIL_STEPS, TEST_PACE};
    use crate::now_unix_ms;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use libsql::params;

    const DEVICES: [&str; 3] = ["dev-a", "dev-b", "dev-c"];
    const ROWS_PER_DEVICE: i64 = 30;
    const LONG: Duration = Duration::from_secs(60);
    /// A row a push (or another device) already gave a slice: the pass leaves it as it is.
    const FOREIGN: &str = "foreign-slice";
    const RUNNER: &str = "dev-runner";

    /// `(kind, source, ts_unix_ms)`: every source class, a blank-source calendar event (Google's class), and dates
    /// before 2000, in 2022/2023 and after 2029 (clamped buckets).
    const SAMPLES: [(&str, Option<&str>, i64); 7] = [
        ("capture", None, 1_700_000_000_000),
        ("transcript", None, 1_650_000_000_000),
        ("calendar_event", Some("outlook"), 1_700_000_000_000),
        ("calendar_event", Some(" "), 1_700_000_000_000),
        ("calendar_event", Some("google_calendar"), 1_926_201_600_000),
        ("capture", None, 900_000_000_000),
        ("calendar_event", None, 0),
    ];

    fn small_pace() {
        TEST_PACE.with(|c| c.set(Some((4, 7, Duration::ZERO))));
    }

    async fn relay() -> (tempfile::TempDir, libsql::Database, Connection, GroupKey) {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        ensure_content_schema(&conn).await.unwrap();
        (dir, db, conn, GroupKey::generate())
    }

    fn item(kind: &str, source: Option<&str>, ts: i64, body: &str) -> memory::AppliedItem {
        let calendar = kind == "calendar_event";
        memory::AppliedItem {
            kind: kind.to_string(),
            source: source.map(str::to_string),
            ts_unix_ms: ts,
            actors: None,
            title: calendar.then(|| body.to_string()),
            body_text: body.to_string(),
            refs: None,
            flags: None,
            hash: None,
            external_id: calendar.then(|| format!("{body}#1")),
            app_bundle_id: None,
            url: None,
        }
    }

    fn item_json(kind: &str, source: Option<&str>, ts: i64, body: &str) -> String {
        serde_json::to_string(&ItemPayload {
            source_meeting_id: None,
            meeting_local_seq: None,
            item: item(kind, source, ts, body),
        })
        .unwrap()
    }

    /// The slice a push gives this item (`slice_tag_for` over the row the push encodes).
    fn pushed_slice(key: &GroupKey, (kind, source, ts): (&str, Option<&str>, i64)) -> String {
        let row = memory::SyncableRow::Item {
            local_seq: 1,
            source_meeting_id: None,
            meeting_local_seq: None,
            item: item(kind, source, ts, "x"),
        };
        slice_tag_for(key, &row).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed(
        conn: &Connection,
        key: &GroupKey,
        device: &str,
        seq: i64,
        row_type: &str,
        json: &str,
        kind_tag: Option<String>,
        slice_tag: Option<String>,
    ) {
        let aad = content_row_aad(device, seq, row_type);
        let ciphertext = STANDARD.encode(sync::encrypt_payload(key, json.as_bytes(), &aad));
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms, kind_tag, slice_tag) \
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
            params![device, seq, row_type, ciphertext.as_str(), kind_tag, slice_tag],
        )
        .await
        .unwrap();
    }

    fn sample_of(device_index: usize, seq: i64) -> (&'static str, Option<&'static str>, i64) {
        SAMPLES[(seq as usize + device_index) % SAMPLES.len()]
    }

    /// Three devices with capability markers, `items` rows of every sample (slice NULL; kind tagged on even seqs,
    /// which the pass must not touch), a `meeting` row every fifth seq, one row a push already sliced and one
    /// another device sliced with a value no item produces.
    async fn fixture(conn: &Connection, key: &GroupKey) {
        for (d, device) in DEVICES.iter().enumerate() {
            push_marker(conn, key, device, ROW_TYPE_ITEMS_CAPABILITY)
                .await
                .unwrap();
            for seq in 1..=ROWS_PER_DEVICE {
                if seq % 5 == 0 {
                    seed(conn, key, device, seq, ROW_TYPE_MEETING, "{}", None, None).await;
                    continue;
                }
                let s = sample_of(d, seq);
                let kind = (seq % 2 == 0).then(|| sync::kind_tag(key, s.0));
                let slice = match seq {
                    s29 if s29 == ROWS_PER_DEVICE - 1 => Some(pushed_slice(key, s)),
                    28 => Some(FOREIGN.to_string()),
                    _ => None,
                };
                seed(
                    conn,
                    key,
                    device,
                    seq,
                    ROW_TYPE_ITEMS,
                    &item_json(s.0, s.1, s.2, &format!("{device} {seq}")),
                    kind,
                    slice,
                )
                .await;
            }
        }
    }

    type Row = (String, i64, String, Option<String>, Option<String>);

    /// Every row as `(device, local_seq, row_type, kind_tag, slice_tag)`, in key order.
    async fn snapshot(conn: &Connection) -> Vec<Row> {
        let mut rows = conn
            .query("SELECT device_id, local_seq, row_type, kind_tag, slice_tag FROM sync_row ORDER BY device_id, local_seq, row_type", ())
            .await
            .unwrap();
        let mut out = Vec::new();
        while let Some(r) = rows.next().await.unwrap() {
            out.push((
                r.get(0).unwrap(),
                r.get(1).unwrap(),
                r.get(2).unwrap(),
                r.get(3).unwrap(),
                r.get(4).unwrap(),
            ));
        }
        out
    }

    fn without_claims(rows: Vec<Row>) -> Vec<Row> {
        rows.into_iter()
            .filter(|r| !r.2.ends_with("_backfill_claim"))
            .collect()
    }

    async fn count(conn: &Connection, sql: &str) -> i64 {
        let mut rows = conn.query(sql, ()).await.unwrap();
        rows.next().await.unwrap().unwrap().get(0).unwrap()
    }

    async fn count_unsliced(conn: &Connection) -> i64 {
        count(
            conn,
            "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND slice_tag IS NULL",
        )
        .await
    }

    /// The fixture's devices fully and correctly sliced: every `items` row its push slice (the foreign one kept),
    /// every other row none, and `kind_tag` exactly as seeded.
    async fn assert_sliced(conn: &Connection, key: &GroupKey) {
        for (device, seq, row_type, kind, slice) in snapshot(conn).await {
            let Some(d) = DEVICES.iter().position(|x| *x == device) else {
                continue;
            };
            if row_type == ROW_TYPE_ITEMS {
                let s = sample_of(d, seq);
                let want = if seq == 28 {
                    FOREIGN.to_string()
                } else {
                    pushed_slice(key, s)
                };
                assert_eq!(slice.as_deref(), Some(want.as_str()), "{device}/{seq}");
                assert_eq!(
                    kind,
                    (seq % 2 == 0).then(|| sync::kind_tag(key, s.0)),
                    "{device}/{seq}: kind_tag untouched"
                );
            } else {
                assert_eq!(slice, None, "{device}/{seq}/{row_type} must stay unsliced");
                assert_eq!(kind, None, "{device}/{seq}/{row_type}");
            }
        }
    }

    async fn run_to_end(
        conn: &Connection,
        key: &GroupKey,
        store: &Store,
        own: &str,
        budget: Duration,
    ) -> (KindBackfillProgress, usize) {
        for steps in 1..=10_000 {
            let p = slice_backfill_step(conn, key, store, own, budget)
                .await
                .unwrap();
            if p.finished {
                return (p, steps);
            }
        }
        panic!("the slice backfill never finished");
    }

    fn keys(store: &Store, prefix: &str) -> Vec<String> {
        store
            .list_meta_with_prefix(prefix)
            .unwrap()
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    }

    async fn claim_at(conn: &Connection, key: &GroupKey, device: &str, row_type: &str, at_ms: i64) {
        push_marker(conn, key, device, row_type).await.unwrap();
        conn.execute(
            "UPDATE sync_row SET created_at_unix_ms = ?2 WHERE device_id = ?1 AND local_seq = 0 AND row_type = ?3",
            params![device, at_ms, row_type],
        )
        .await
        .unwrap();
    }

    async fn claim_rows(conn: &Connection, device: &str, row_type: &str) -> i64 {
        count(conn, &format!("SELECT COUNT(*) FROM sync_row WHERE device_id = '{device}' AND local_seq = 0 AND row_type = '{row_type}'")).await
    }

    #[test]
    fn the_slice_claim_is_its_own_row_type() {
        assert_eq!(ROW_TYPE_SLICE_BACKFILL_CLAIM, "slice_backfill_claim");
        assert_ne!(ROW_TYPE_SLICE_BACKFILL_CLAIM, ROW_TYPE_KIND_BACKFILL_CLAIM);
    }

    /// The retry line names the failing pass.
    #[test]
    fn each_pass_has_its_log_name() {
        assert_eq!(crate::kind_backfill::KIND_PASS.name(), "kind");
        assert_eq!(SLICE_PASS.name(), "slice");
    }

    /// It tags a mixed relay's `items` rows with the value `item_payload_slice_tag` derives; non-`items` rows,
    /// already-sliced rows and every `kind_tag` are left alone; only its own kv keys are written.
    #[tokio::test]
    async fn tags_exactly_the_unsliced_items_rows_of_every_device() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();

        let (p, _) = run_to_end(&conn, &key, &store, "dev-a", LONG).await;

        assert!(p.finished);
        assert_eq!(p.undecryptable, 0);
        assert_eq!(p.total, 3 * ROWS_PER_DEVICE as u64);
        assert_eq!(p.done, p.total);
        assert_sliced(&conn, &key).await;
        assert_eq!(count_unsliced(&conn).await, 0);
        assert!(store.get_meta("slice_backfill_done").unwrap().is_some());
        for d in DEVICES {
            assert_eq!(
                store
                    .get_meta(&format!("slice_backfill:1:{d}"))
                    .unwrap()
                    .as_deref(),
                Some("done")
            );
        }
        assert!(
            keys(&store, "kind_backfill").is_empty(),
            "{:?}",
            keys(&store, "kind_backfill")
        );
        assert_eq!(
            claim_rows(&conn, "dev-a", ROW_TYPE_SLICE_BACKFILL_CLAIM).await,
            1,
            "its own claim"
        );
        assert_eq!(
            claim_rows(&conn, "dev-a", ROW_TYPE_KIND_BACKFILL_CLAIM).await,
            0,
            "never the kind claim"
        );
    }

    /// The backfilled slice is the push path's for the same item (same `ts_unix_ms` and source), and what
    /// `item_payload_slice_tag` derives from the payload.
    #[tokio::test]
    async fn the_backfilled_slice_equals_the_push_slice() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        push_marker(&conn, &key, "dev-a", ROW_TYPE_ITEMS_CAPABILITY)
            .await
            .unwrap();
        for (i, s) in SAMPLES.iter().enumerate() {
            seed(
                &conn,
                &key,
                "dev-b",
                i as i64 + 1,
                ROW_TYPE_ITEMS,
                &item_json(s.0, s.1, s.2, "x"),
                None,
                None,
            )
            .await;
        }
        run_to_end(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            "dev-a",
            LONG,
        )
        .await;

        let mut distinct = std::collections::BTreeSet::new();
        for (i, s) in SAMPLES.iter().enumerate() {
            let mut rows = conn
                .query(
                    "SELECT slice_tag FROM sync_row WHERE device_id = 'dev-b' AND local_seq = ?1",
                    params![i as i64 + 1],
                )
                .await
                .unwrap();
            let got: Option<String> = rows.next().await.unwrap().unwrap().get(0).unwrap();
            assert_eq!(
                got.as_deref(),
                Some(pushed_slice(&key, *s).as_str()),
                "{s:?}"
            );
            assert_eq!(
                got,
                item_payload_slice_tag(&key, item_json(s.0, s.1, s.2, "x").as_bytes()),
                "{s:?}"
            );
            distinct.insert(got.unwrap());
        }
        // Blank and missing sources are Google's class, like `google_calendar`; the samples still differ by month.
        assert!(distinct.len() >= 6, "{distinct:?}");
    }

    #[tokio::test]
    async fn running_to_completion_twice_changes_nothing() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        run_to_end(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            "dev-a",
            LONG,
        )
        .await;
        let first = without_claims(snapshot(&conn).await);

        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));
        let (p, _) = run_to_end(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            "dev-a",
            LONG,
        )
        .await;
        let sql = CAPTURED_SQL.with(|c| c.borrow_mut().take().unwrap());

        assert!(p.finished);
        assert!(
            !sql.iter().any(|s| s.starts_with("UPDATE")),
            "nothing left to update: {sql:?}"
        );
        assert_eq!(without_claims(snapshot(&conn).await), first);
    }

    /// A step cut short saves its per-peer progress; the next step resumes from it and ends where an
    /// uninterrupted run does.
    #[tokio::test]
    async fn a_run_cut_into_tiny_steps_resumes_from_saved_progress() {
        small_pace();
        let key = GroupKey::generate();
        let (_d1, _db1, interrupted, _) = relay().await;
        let (_d2, _db2, whole, _) = relay().await;
        fixture(&interrupted, &key).await;
        fixture(&whole, &key).await;

        let store = Store::open_in_memory().unwrap();
        let first = slice_backfill_step(&interrupted, &key, &store, "dev-a", Duration::ZERO)
            .await
            .unwrap();
        assert!(!first.finished);
        assert!(first.done > 0 && first.done < first.total, "{first:?}");
        let saved = store
            .get_meta("slice_backfill:1:dev-a")
            .unwrap()
            .expect("progress saved for the first peer");
        assert_eq!(saved.parse::<u64>().unwrap(), first.done);
        let sliced = count(
            &interrupted,
            "SELECT COUNT(*) FROM sync_row WHERE slice_tag IS NOT NULL",
        )
        .await;
        assert!(sliced > 6 && sliced < 3 * 23, "stopped mid-way: {sliced}");

        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));
        let second = slice_backfill_step(&interrupted, &key, &store, "dev-a", Duration::ZERO)
            .await
            .unwrap();
        let sql = CAPTURED_SQL.with(|c| c.borrow_mut().take().unwrap());
        assert!(second.done > first.done);
        let read = sql
            .iter()
            .find(|s| s.starts_with("SELECT local_seq, ciphertext"))
            .expect("a chunk read");
        assert!(read.contains("slice_tag IS NULL"), "{read}");

        let (_, steps) = run_to_end(&interrupted, &key, &store, "dev-a", Duration::ZERO).await;
        assert!(steps > 3, "{steps}");
        run_to_end(
            &whole,
            &key,
            &Store::open_in_memory().unwrap(),
            "dev-a",
            LONG,
        )
        .await;
        assert_eq!(snapshot(&interrupted).await, snapshot(&whole).await);
        assert_sliced(&interrupted, &key).await;
    }

    #[tokio::test]
    async fn an_undecryptable_row_stays_null_and_is_counted() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES ('dev-b', 101, 'items', '!!not base64', 0)",
            (),
        )
        .await
        .unwrap();
        seed(
            &conn,
            &GroupKey::generate(),
            "dev-b",
            102,
            ROW_TYPE_ITEMS,
            &item_json("capture", None, 1, "other group"),
            None,
            None,
        )
        .await;
        seed(
            &conn,
            &key,
            "dev-c",
            103,
            ROW_TYPE_ITEMS,
            "{\"kind\": 7}",
            None,
            None,
        )
        .await;
        seed(
            &conn,
            &key,
            "dev-c",
            104,
            ROW_TYPE_ITEMS,
            &item_json("capture", None, 1_700_000_000_000, "after"),
            None,
            None,
        )
        .await;
        let store = Store::open_in_memory().unwrap();

        let (p, _) = run_to_end(&conn, &key, &store, "dev-a", LONG).await;

        assert!(p.finished);
        assert_eq!(p.undecryptable, 3);
        assert_eq!(count_unsliced(&conn).await, 3);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_row WHERE local_seq IN (101, 102, 103) AND slice_tag IS NULL").await, 3);
        assert_eq!(
            store
                .get_meta("slice_backfill_undecryptable:1")
                .unwrap()
                .as_deref(),
            Some("3")
        );
        let mut rows = conn
            .query("SELECT slice_tag FROM sync_row WHERE local_seq = 104", ())
            .await
            .unwrap();
        let tag: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(
            tag,
            pushed_slice(&key, ("capture", None, 1_700_000_000_000))
        );
        assert_eq!(
            slice_backfill_step(&conn, &key, &store, "dev-a", LONG)
                .await
                .unwrap(),
            p,
            "finished reports the same"
        );
    }

    #[tokio::test]
    async fn a_relay_whose_slice_tag_column_cannot_be_confirmed_returns_default_progress() {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE VIEW sync_row AS SELECT 1 AS device_id", ())
            .await
            .unwrap();
        SLICE_TAG_COLUMN_SEEN.reset();
        // The kind column being known says nothing about the slice column.
        KIND_TAG_COLUMN_SEEN.set();
        let store = Store::open_in_memory().unwrap();

        let p = slice_backfill_step(&conn, &GroupKey::generate(), &store, "dev-a", LONG)
            .await
            .unwrap();

        KIND_TAG_COLUMN_SEEN.reset();
        assert_eq!(p, KindBackfillProgress::default());
        assert_eq!(store.get_meta("slice_backfill_done").unwrap(), None);
        assert!(keys(&store, "slice_backfill").is_empty());
    }

    /// The claim-row site that matters: a device not yet on the relay as itself (no capability marker) neither
    /// claims nor tags, so a slice claim row alone never makes it a peer without markers.
    #[tokio::test]
    async fn a_device_without_its_capability_marker_neither_claims_nor_tags() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        let before = snapshot(&conn).await;

        let p = slice_backfill_step(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            "dev-new",
            LONG,
        )
        .await
        .unwrap();

        assert!(!p.finished);
        assert_eq!(snapshot(&conn).await, before);
        assert_eq!(
            claim_rows(&conn, "dev-new", ROW_TYPE_SLICE_BACKFILL_CLAIM).await,
            0
        );
        // The marker gates still see every peer as capable.
        assert!(
            crate::content::all_peers_have_marker(&conn, "dev-a", ROW_TYPE_ITEMS_CAPABILITY)
                .await
                .unwrap()
        );
    }

    /// A slice claim row (with the claimer's marker) is a marker like the kind claim: it does not stop the marker
    /// gates, and it counts as a marker in the pull's per-type counts.
    #[tokio::test]
    async fn a_slice_claim_counts_as_a_marker_and_never_holds_the_gates() {
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        refresh_marker(&conn, &key, "dev-b", ROW_TYPE_SLICE_BACKFILL_CLAIM)
            .await
            .unwrap();
        assert!(
            crate::content::all_peers_have_marker(&conn, "dev-a", ROW_TYPE_ITEMS_CAPABILITY)
                .await
                .unwrap()
        );
        assert_eq!(
            crate::pull_type_index(ROW_TYPE_SLICE_BACKFILL_CLAIM),
            crate::pull_type_index(ROW_TYPE_KIND_BACKFILL_CLAIM)
        );
        assert_eq!(
            crate::PULL_TYPE_NAMES[crate::pull_type_index(ROW_TYPE_SLICE_BACKFILL_CLAIM)],
            "marker"
        );
    }

    /// A held fresh slice claim holds the slice step without a write; a fresh kind claim does not (and a fresh
    /// slice claim does not hold the kind pass).
    #[tokio::test]
    async fn only_a_fresh_slice_claim_holds_the_slice_pass() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        fixture(&conn, &key).await;
        claim_at(
            &conn,
            &key,
            "dev-b",
            ROW_TYPE_SLICE_BACKFILL_CLAIM,
            now_unix_ms() - 60_000,
        )
        .await;
        let before = snapshot(&conn).await;

        let held = slice_backfill_step(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            "dev-a",
            LONG,
        )
        .await
        .unwrap();
        assert!(!held.finished);
        assert_eq!(held.done, 0);
        assert_eq!(
            snapshot(&conn).await,
            before,
            "nothing written, not even our own claim"
        );

        // The kind pass is not held by it.
        let kind = crate::kind_backfill_step(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            "dev-a",
            LONG,
        )
        .await
        .unwrap();
        assert!(kind.finished, "a slice claim never holds the kind pass");

        // A fresh kind claim does not hold the slice pass.
        conn.execute(
            "DELETE FROM sync_row WHERE row_type = ?1",
            params![ROW_TYPE_SLICE_BACKFILL_CLAIM],
        )
        .await
        .unwrap();
        claim_at(
            &conn,
            &key,
            "dev-b",
            ROW_TYPE_KIND_BACKFILL_CLAIM,
            now_unix_ms() - 60_000,
        )
        .await;
        let (p, _) = run_to_end(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            "dev-a",
            LONG,
        )
        .await;
        assert!(p.finished);
        assert_eq!(count_unsliced(&conn).await, 0);
    }

    /// Every statement reading or writing NULL slices names a device and a `local_seq` range or keys, and a chunk
    /// is written with one UPDATE per distinct slice in it, never one per row.
    #[tokio::test]
    async fn a_chunk_is_written_with_one_update_per_slice_and_bounded_by_the_primary_key() {
        TEST_PACE.with(|c| c.set(Some((500, 20_000, Duration::ZERO))));
        let (_dir, _db, conn, key) = relay().await;
        push_marker(&conn, &key, "dev-a", ROW_TYPE_ITEMS_CAPABILITY)
            .await
            .unwrap();
        for seq in 1..=1_200i64 {
            if seq % 13 == 0 {
                seed(
                    &conn,
                    &key,
                    "dev-big",
                    seq,
                    ROW_TYPE_MEETING,
                    "{}",
                    None,
                    None,
                )
                .await;
                continue;
            }
            let s = SAMPLES[seq as usize % SAMPLES.len()];
            seed(
                &conn,
                &key,
                "dev-big",
                seq,
                ROW_TYPE_ITEMS,
                &item_json(s.0, s.1, s.2, &format!("row {seq}")),
                None,
                None,
            )
            .await;
        }
        let null_items = count_unsliced(&conn).await;
        CAPTURED_SQL.with(|c| *c.borrow_mut() = Some(Vec::new()));

        run_to_end(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            "dev-a",
            LONG,
        )
        .await;

        let sql = CAPTURED_SQL.with(|c| c.borrow_mut().take().unwrap());
        let mut chunks: Vec<Vec<&String>> = Vec::new();
        for s in &sql {
            if s.starts_with("SELECT local_seq, ciphertext") {
                chunks.push(Vec::new());
            } else if s.starts_with("UPDATE") {
                assert!(s.starts_with("UPDATE sync_row SET slice_tag = ?1"), "{s}");
                assert!(s.contains("slice_tag IS NULL"), "{s}");
                assert!(!s.contains("kind_tag"), "{s}");
                chunks
                    .last_mut()
                    .expect("an UPDATE follows a chunk read")
                    .push(s);
            }
        }
        let written: Vec<&Vec<&String>> = chunks.iter().filter(|c| !c.is_empty()).collect();
        assert!(written.len() >= 3, "{}", written.len());
        for c in &written {
            assert!(
                c.len() <= SAMPLES.len(),
                "one UPDATE per slice in a chunk: {}",
                c.len()
            );
        }
        let seqs_written: usize = written
            .iter()
            .flat_map(|c| c.iter())
            .map(|s| s.matches('?').count() - 2)
            .sum();
        assert_eq!(
            seqs_written as i64, null_items,
            "every NULL row written once"
        );
        for s in sql.iter().filter(|s| s.contains("slice_tag IS NULL")) {
            assert!(s.contains("device_id = ?"), "{s}");
            assert!(
                (s.contains("local_seq > ?") && s.contains("local_seq <= ?"))
                    || s.contains("local_seq IN (?"),
                "{s}"
            );
        }
        assert_eq!(count_unsliced(&conn).await, 0);
    }

    // ---- The cloud runner's long run ----

    /// The fixture plus the runner's own `items` rows (NULL slices).
    async fn runner_fixture(conn: &Connection, key: &GroupKey) {
        fixture(conn, key).await;
        for seq in 1..=10 {
            seed(
                conn,
                key,
                RUNNER,
                seq,
                ROW_TYPE_ITEMS,
                &item_json("capture", None, 1_700_000_000_000, &format!("runner {seq}")),
                None,
                None,
            )
            .await;
        }
    }

    async fn runner_run(
        conn: &Connection,
        key: &GroupKey,
        store: &Store,
        total: Duration,
        step: Duration,
    ) -> (KindBackfillProgress, BackfillStop, usize) {
        let mut steps = 0;
        let (p, stop) = crate::runner_slice_backfill_observed_with_connection(
            conn,
            key,
            store,
            RUNNER,
            total,
            step,
            &mut |_, n| steps = n,
        )
        .await
        .unwrap();
        (p, stop, steps)
    }

    #[tokio::test]
    async fn the_runner_slice_backfill_tags_every_device_and_its_own_rows() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();

        let (p, stop, steps) = runner_run(&conn, &key, &store, LONG, LONG).await;

        assert_eq!(stop, BackfillStop::Finished);
        assert!(p.finished && steps >= 1);
        assert_eq!(p.total, (3 * ROWS_PER_DEVICE + 10) as u64);
        assert_eq!(count_unsliced(&conn).await, 0);
        assert_sliced(&conn, &key).await;
        let capture = pushed_slice(&key, ("capture", None, 1_700_000_000_000));
        assert_eq!(
            count(&conn, &format!("SELECT COUNT(*) FROM sync_row WHERE device_id = '{RUNNER}' AND slice_tag = '{capture}'")).await,
            10
        );
        assert!(keys(&store, "kind_backfill").is_empty());
    }

    /// The runner's store is new each execution: the walk's position comes back from the relay, so a rerun resumes
    /// where the last one stopped instead of walking every position again.
    #[tokio::test]
    async fn a_new_store_resumes_the_walk_from_the_position_saved_on_the_relay() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;

        let first = Store::open_in_memory().unwrap();
        let (p1, stop1, _) = runner_run(&conn, &key, &first, Duration::ZERO, Duration::ZERO).await;
        assert_eq!(stop1, BackfillStop::Budget);
        assert!(p1.done > 0 && !p1.finished);

        // A brand new store, one more tiny step: it starts at the saved position, not at 0.
        let second = Store::open_in_memory().unwrap();
        let mut first_done = None;
        let (_, _) = crate::runner_slice_backfill_observed_with_connection(
            &conn,
            &key,
            &second,
            RUNNER,
            Duration::ZERO,
            Duration::ZERO,
            &mut |p, n| {
                if n == 1 {
                    first_done = Some(p.done)
                }
            },
        )
        .await
        .unwrap();
        assert!(
            first_done.unwrap() > p1.done,
            "resumed at {} after {}",
            first_done.unwrap(),
            p1.done
        );

        // To the end on yet another new store; a finished walk is remembered too.
        let third = Store::open_in_memory().unwrap();
        let (p3, stop3, _) = runner_run(&conn, &key, &third, LONG, LONG).await;
        assert_eq!(stop3, BackfillStop::Finished);
        assert!(p3.finished);
        assert_eq!(count_unsliced(&conn).await, 0);
        let fourth = Store::open_in_memory().unwrap();
        let (p4, stop4, steps4) = runner_run(&conn, &key, &fourth, LONG, LONG).await;
        assert_eq!((stop4, steps4), (BackfillStop::Finished, 1));
        assert_eq!(p4.total, p3.total);
    }

    /// A payload that does not open (the plain "1" of a claim no runner saved into) starts the walk at 0.
    #[tokio::test]
    async fn an_unreadable_saved_position_starts_the_walk_at_zero() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        crate::content::ensure_content_schema(&conn).await.unwrap();
        push_marker(&conn, &key, RUNNER, ROW_TYPE_SLICE_BACKFILL_CLAIM)
            .await
            .unwrap();
        conn.execute(
            "UPDATE sync_row SET ciphertext = 'not base64 !' WHERE device_id = ?1 AND local_seq = 0 AND row_type = ?2",
            params![RUNNER, ROW_TYPE_SLICE_BACKFILL_CLAIM],
        )
        .await
        .unwrap();
        let (p, stop, _) =
            runner_run(&conn, &key, &Store::open_in_memory().unwrap(), LONG, LONG).await;
        assert_eq!(stop, BackfillStop::Finished);
        assert!(p.finished);
        assert_eq!(count_unsliced(&conn).await, 0);
    }

    #[tokio::test]
    async fn a_held_slice_claim_stops_the_runner_with_claim_held_and_no_write() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        claim_at(
            &conn,
            &key,
            "dev-b",
            ROW_TYPE_SLICE_BACKFILL_CLAIM,
            now_unix_ms() - 60_000,
        )
        .await;
        let unsliced = count_unsliced(&conn).await;

        let (p, stop, steps) =
            runner_run(&conn, &key, &Store::open_in_memory().unwrap(), LONG, LONG).await;

        assert_eq!(stop, BackfillStop::ClaimHeld);
        assert_eq!(steps, 2, "two steps without progress, then it stops");
        assert_eq!(p.done, 0);
        assert_eq!(count_unsliced(&conn).await, unsliced);
        assert_eq!(
            claim_rows(&conn, RUNNER, ROW_TYPE_SLICE_BACKFILL_CLAIM).await,
            0
        );
    }

    #[tokio::test]
    async fn a_two_step_budget_resumes_and_a_second_run_completes() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        let store = Store::open_in_memory().unwrap();

        let (p, stop, steps) =
            runner_run(&conn, &key, &store, Duration::ZERO, Duration::ZERO).await;
        assert_eq!(stop, BackfillStop::Budget);
        assert_eq!(steps, 1);
        assert!(p.done > 0 && p.done < p.total, "{p:?}");

        let (second, stop, _) = runner_run(&conn, &key, &store, LONG, LONG).await;
        assert_eq!(stop, BackfillStop::Finished);
        assert!(second.finished);
        assert_eq!(count_unsliced(&conn).await, 0);
    }

    #[tokio::test]
    async fn a_step_that_fails_twice_then_succeeds_still_finishes_and_a_fourth_failure_errors() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        TEST_FAIL_STEPS.with(|c| c.set(4));
        let r = crate::runner_slice_backfill_observed_with_connection(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            RUNNER,
            LONG,
            LONG,
            &mut |_, _| {},
        )
        .await;
        assert!(r.is_err());
        TEST_FAIL_STEPS.with(|c| c.set(2));
        let (p, stop, _) =
            runner_run(&conn, &key, &Store::open_in_memory().unwrap(), LONG, LONG).await;
        TEST_FAIL_STEPS.with(|c| c.set(0));
        assert_eq!(stop, BackfillStop::Finished);
        assert!(p.finished);
    }

    #[tokio::test]
    async fn no_step_starts_with_under_10_s_of_the_total_budget_left() {
        small_pace();
        let (_dir, _db, conn, key) = relay().await;
        runner_fixture(&conn, &key).await;
        let (p, stop, steps) = runner_run(
            &conn,
            &key,
            &Store::open_in_memory().unwrap(),
            Duration::from_secs(5),
            Duration::ZERO,
        )
        .await;
        assert!(!p.finished);
        assert_eq!(steps, 1);
        assert_eq!(stop, BackfillStop::Budget);
    }

    /// The apps never run it: a desktop round (which runs the kind pass) leaves every slice NULL, writes no slice
    /// claim and keeps no slice key.
    #[tokio::test]
    async fn a_desktop_round_never_runs_the_slice_pass() {
        small_pace();
        // Within the round's timeout, so the kind step runs (`kind_backfill_fits`).
        crate::TEST_BACKFILL_BUDGET.with(|c| c.set(Some(Duration::from_secs(5))));
        crate::TEST_MORE_TO_PULL.with(|c| c.set(Some(false)));
        let (_dir, _db, conn, key) = relay().await;
        for device in DEVICES {
            push_marker(&conn, &key, device, ROW_TYPE_ITEMS_CAPABILITY)
                .await
                .unwrap();
            for seq in 1..=10 {
                seed(
                    &conn,
                    &key,
                    device,
                    seq,
                    ROW_TYPE_ITEMS,
                    &item_json(
                        "capture",
                        None,
                        1_700_000_000_000,
                        &format!("{device} {seq}"),
                    ),
                    None,
                    None,
                )
                .await;
            }
        }
        let store = Store::open_in_memory().unwrap();
        let unsliced = count_unsliced(&conn).await;

        crate::content_round(&conn, &key, &store, "dev-own", &|_| {})
            .await
            .unwrap();

        crate::TEST_BACKFILL_BUDGET.with(|c| c.set(None));
        crate::TEST_MORE_TO_PULL.with(|c| c.set(None));
        assert!(
            count(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL"
            )
            .await
                < unsliced,
            "the kind pass ran"
        );
        assert_eq!(
            count_unsliced(&conn).await,
            unsliced,
            "the slice pass did not"
        );
        assert_eq!(count(&conn, &format!("SELECT COUNT(*) FROM sync_row WHERE row_type = '{ROW_TYPE_SLICE_BACKFILL_CLAIM}'")).await, 0);
        assert!(
            keys(&store, "slice_backfill").is_empty(),
            "{:?}",
            keys(&store, "slice_backfill")
        );
    }
}
