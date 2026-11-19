//! Filtered capture search for Library (docs/ui-redesign/screens/home.md, "Search filters"): the keyword
//! and semantic capture searches with an optional app and an optional time range, plus the app choices
//! the filter offers. Read-only; no schema change, no new index.

use crate::{embedding_to_json, fts5_all_words, sanitize_fts5_query, CaptureItem, Store, ITEMS_VEC_MAX_K};
use rusqlite::types::Value;
use rusqlite::params_from_iter;

/// An app filter with no dates reaches back this far, so it never scans the whole table.
pub const APP_FILTER_DEFAULT_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// One app seen in a time range.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CaptureApp {
    pub app_bundle_id: String,
    pub captures: i64,
}

impl Store {
    /// `search_keyword` (`semantic` false) or `search_semantic` (true), narrowed to captures from `app`
    /// and with `start_unix_ms <= ts < end_unix_ms`. With an app and no dates the range is
    /// `[now_unix_ms - 30 days, ...)`. With no filter at all it is exactly the unfiltered search. Semantic
    /// search filters after the nearest-neighbour step (which already reads up to `ITEMS_VEC_MAX_K`
    /// neighbours), so it can return fewer than `limit`; it is never padded.
    pub fn search_captures_filtered(
        &self,
        query: &str,
        semantic: bool,
        app: Option<&str>,
        start_unix_ms: Option<i64>,
        end_unix_ms: Option<i64>,
        limit: usize,
        now_unix_ms: i64,
    ) -> rusqlite::Result<Vec<CaptureItem>> {
        let mut start = start_unix_ms;
        if app.is_some() && start.is_none() && end_unix_ms.is_none() {
            start = Some(now_unix_ms - APP_FILTER_DEFAULT_WINDOW_MS);
        }
        if app.is_none() && start.is_none() && end_unix_ms.is_none() {
            return if semantic { self.search_semantic(query, limit) } else { self.search_keyword(query, limit) };
        }
        // Filter conditions numbered from `first`, with their values.
        let conditions = |first: usize| -> (String, Vec<Value>) {
            let mut sql = String::new();
            let mut vals = Vec::new();
            let mut n = first;
            if let Some(a) = app {
                sql.push_str(&format!(" AND i.app_bundle_id = ?{n}"));
                vals.push(Value::Text(a.to_string()));
                n += 1;
            }
            if let Some(s) = start {
                sql.push_str(&format!(" AND i.ts_unix_ms >= ?{n}"));
                vals.push(Value::Integer(s));
                n += 1;
            }
            if let Some(e) = end_unix_ms {
                sql.push_str(&format!(" AND i.ts_unix_ms < ?{n}"));
                vals.push(Value::Integer(e));
            }
            (sql, vals)
        };
        if semantic {
            let Ok(embedding) = self.embed(query) else {
                return Ok(Vec::new());
            };
            let total_items: i64 = self.conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?;
            let k = total_items.clamp(1, ITEMS_VEC_MAX_K);
            let (extra, vals) = conditions(4);
            let mut stmt = self.conn.prepare(&format!(
                "SELECT i.id, i.device_id, i.ts_unix_ms, i.app_bundle_id, i.title, i.url, i.body_text, i.hash, i.source, NULL \
                 FROM items_vec v \
                 JOIN items i ON i.id = v.rowid \
                 WHERE v.embedding MATCH ?1 AND k = ?2 AND i.kind = 'capture'{extra} \
                 ORDER BY v.distance \
                 LIMIT ?3"
            ))?;
            let mut all = vec![Value::Text(embedding_to_json(&embedding)), Value::Integer(k), Value::Integer(limit as i64)];
            all.extend(vals);
            let rows = stmt.query_map(params_from_iter(all), Self::row_to_item)?;
            rows.collect()
        } else {
            let (extra, vals) = conditions(4);
            let mut stmt = self.conn.prepare(&format!(
                "SELECT i.id, i.device_id, i.ts_unix_ms, i.app_bundle_id, i.title, i.url, i.body_text, i.hash, i.source, NULL \
                 FROM items_fts f \
                 JOIN items i ON i.id = f.rowid \
                 WHERE items_fts MATCH ?1 AND i.kind = 'capture'{extra} \
                 ORDER BY f.rowid IN (SELECT rowid FROM items_fts WHERE items_fts MATCH ?3) DESC, rank \
                 LIMIT ?2"
            ))?;
            let mut all = vec![
                Value::Text(sanitize_fts5_query(query)),
                Value::Integer(limit as i64),
                Value::Text(fts5_all_words(query)),
            ];
            all.extend(vals);
            let rows = stmt.query_map(params_from_iter(all), Self::row_to_item)?;
            rows.collect()
        }
    }

    /// The apps with captures in `[start_unix_ms, end_unix_ms)`, most captures first (ties by id), at most
    /// `limit`. A time range is required, so the read is a range of `idx_items_kind_ts`.
    pub fn capture_apps(
        &self,
        start_unix_ms: i64,
        end_unix_ms: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<CaptureApp>> {
        let mut stmt = self.conn.prepare(
            "SELECT app_bundle_id, COUNT(*) FROM items \
             WHERE kind = 'capture' AND ts_unix_ms >= ?1 AND ts_unix_ms < ?2 \
               AND app_bundle_id IS NOT NULL AND app_bundle_id != '' \
             GROUP BY app_bundle_id ORDER BY COUNT(*) DESC, app_bundle_id ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![start_unix_ms, end_unix_ms, limit as i64], |r| {
            Ok(CaptureApp { app_bundle_id: r.get(0)?, captures: r.get(1)? })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewCaptureItem;

    const DAY: i64 = 24 * 60 * 60 * 1000;

    fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::test_store::open_with_embedder(&dir.path().join("w.db"), std::sync::Arc::new(crate::tests::FakeEmbedder));
        (s, dir)
    }

    fn put(s: &Store, n: i64, ts: i64, app: &str, text: &str) {
        s.insert_capture_item(&NewCaptureItem {
            device_id: "d".into(),
            ts_unix_ms: ts,
            app_bundle_id: app.into(),
            window_title: "Window".into(),
            url: None,
            text: text.into(),
            hash: n,
            source: "ax".into(),
        })
        .unwrap();
    }

    fn ids(v: Vec<CaptureItem>) -> Vec<i64> {
        v.into_iter().map(|c| c.id).collect()
    }

    fn seeded() -> (Store, tempfile::TempDir, i64) {
        let (s, d) = store();
        let now = 100 * DAY;
        put(&s, 1, now - 1000, "com.a", "budget review notes");
        put(&s, 2, now - 2 * DAY, "com.b", "budget spreadsheet");
        put(&s, 3, now - 40 * DAY, "com.a", "budget from long ago");
        put(&s, 4, now - 3 * DAY, "com.a", "unrelated gardening");
        (s, d, now)
    }

    #[test]
    fn no_filters_is_exactly_the_unfiltered_search() {
        let (s, _d, now) = seeded();
        for q in ["budget", "gardening", "nothing matches this"] {
            assert_eq!(
                s.search_captures_filtered(q, false, None, None, None, 10, now).unwrap(),
                s.search_keyword(q, 10).unwrap()
            );
            assert_eq!(
                s.search_captures_filtered(q, true, None, None, None, 10, now).unwrap(),
                s.search_semantic(q, 10).unwrap()
            );
        }
    }

    #[test]
    fn an_app_filter_keeps_only_that_app_and_defaults_to_thirty_days() {
        let (s, _d, now) = seeded();
        let hits = s.search_captures_filtered("budget", false, Some("com.a"), None, None, 10, now).unwrap();
        assert_eq!(hits.len(), 1, "the 40 day old row is out of the default window");
        assert_eq!(hits[0].app_bundle_id, "com.a");
        assert_eq!(hits[0].ts_unix_ms, now - 1000);
        // An explicit range replaces the default.
        let hits = s
            .search_captures_filtered("budget", false, Some("com.a"), Some(0), Some(now - 30 * DAY), 10, now)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ts_unix_ms, now - 40 * DAY);
    }

    #[test]
    fn a_time_range_alone_is_half_open() {
        let (s, _d, now) = seeded();
        let hits = s
            .search_captures_filtered("budget", false, None, Some(now - 3 * DAY), Some(now - 1000), 10, now)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ts_unix_ms, now - 2 * DAY, "the row at the end bound is out");
    }

    #[test]
    fn semantic_search_filters_after_the_neighbours_and_does_not_pad() {
        let (s, _d, now) = seeded();
        let all = s.search_captures_filtered("budget", true, None, Some(0), Some(now + 1), 10, now).unwrap();
        assert_eq!(all.len(), 4);
        let one = s.search_captures_filtered("budget", true, Some("com.b"), None, None, 10, now).unwrap();
        assert_eq!(ids(one.clone()).len(), 1);
        assert_eq!(one[0].app_bundle_id, "com.b");
        let none = s.search_captures_filtered("budget", true, Some("com.zzz"), None, None, 10, now).unwrap();
        assert!(none.is_empty());
        let limited = s.search_captures_filtered("budget", true, Some("com.a"), Some(0), None, 2, now).unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn capture_apps_counts_the_apps_in_a_range_most_frequent_first() {
        let (s, _d, now) = seeded();
        let apps = s.capture_apps(now - 10 * DAY, now + 1, 10).unwrap();
        assert_eq!(
            apps,
            vec![
                CaptureApp { app_bundle_id: "com.a".into(), captures: 2 },
                CaptureApp { app_bundle_id: "com.b".into(), captures: 1 },
            ]
        );
        assert_eq!(s.capture_apps(now - 10 * DAY, now + 1, 1).unwrap().len(), 1);
        assert_eq!(s.capture_apps(0, now + 1, 10).unwrap()[0].captures, 3);
        assert!(s.capture_apps(0, 1, 10).unwrap().is_empty());
    }
}
