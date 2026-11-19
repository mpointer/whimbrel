//! Memory blocks for Home (docs/ui-redesign/screens/home.md, "Memory blocks"): one indexed read of
//! `items` (`kind = 'capture'`, served by `idx_items_kind_ts`) that returns time, app and window title
//! only, never body text, grouped in Rust. Read-only; no schema change, no new index.

use crate::{Store, EPISODE_IDLE_GAP_MS};
use serde::Serialize;

/// A new block starts after this much quiet between two captures (the episode gap, 10 minutes).
pub const BLOCK_GAP_MS: i64 = EPISODE_IDLE_GAP_MS;
/// Apps shown per block; the rest are counted in `more_apps`.
pub const BLOCK_MAX_APPS: usize = 4;
/// Titles shown per app; the rest are counted in `more_items`.
pub const BLOCK_MAX_TITLES: usize = 3;

/// One stored capture, reduced to the three columns a block needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRow {
    pub ts_unix_ms: i64,
    pub app_bundle_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayBlockApp {
    pub app_bundle_id: String,
    /// Distinct non-empty titles in first-seen order, at most `BLOCK_MAX_TITLES`.
    pub items: Vec<String>,
    /// Distinct titles left out.
    pub more_items: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayBlock {
    /// Time of the block's first capture.
    pub start_unix_ms: i64,
    /// Time of the block's last capture.
    pub end_unix_ms: i64,
    /// Apps in order of first appearance, at most `BLOCK_MAX_APPS`.
    pub apps: Vec<DayBlockApp>,
    /// Distinct apps left out.
    pub more_apps: u32,
}

/// Groups `rows` (ordered by time) into blocks. A block ends at a gap of more than `BLOCK_GAP_MS` or at
/// a meeting: rows inside a `(start, end)` meeting interval (half open, `[start, end)`) are dropped, and
/// a block never spans a meeting, so a block that overlaps one is cut at its start and end and a piece
/// with no rows left does not exist. A row with an empty app id still counts toward the block's time but
/// is not listed as an app; an empty title is skipped, but its app is still listed.
pub fn group_blocks(rows: &[BlockRow], meetings: &[(i64, i64)]) -> Vec<DayBlock> {
    let meetings: Vec<(i64, i64)> = meetings.iter().copied().filter(|(s, e)| s < e).collect();
    let mut out: Vec<DayBlock> = Vec::new();
    // Per open block: apps in first-seen order with every distinct title.
    let mut open: Option<(i64, i64, usize, Vec<(String, Vec<String>)>)> = None;
    let mut finish = |open: &mut Option<(i64, i64, usize, Vec<(String, Vec<String>)>)>, out: &mut Vec<DayBlock>| {
        if let Some((start, end, _, apps)) = open.take() {
            let more_apps = apps.len().saturating_sub(BLOCK_MAX_APPS) as u32;
            let apps = apps
                .into_iter()
                .take(BLOCK_MAX_APPS)
                .map(|(app, titles)| DayBlockApp {
                    more_items: titles.len().saturating_sub(BLOCK_MAX_TITLES) as u32,
                    items: titles.into_iter().take(BLOCK_MAX_TITLES).collect(),
                    app_bundle_id: app,
                })
                .collect();
            out.push(DayBlock { start_unix_ms: start, end_unix_ms: end, apps, more_apps });
        }
    };
    for row in rows {
        let ts = row.ts_unix_ms;
        if meetings.iter().any(|&(s, e)| ts >= s && ts < e) {
            continue;
        }
        // The number of meetings already over: it only grows with time, so a change means a meeting sits between.
        let segment = meetings.iter().filter(|&&(_, e)| e <= ts).count();
        let starts_new = match &open {
            Some((_, last, seg, _)) => *seg != segment || ts - *last > BLOCK_GAP_MS,
            None => true,
        };
        if starts_new {
            finish(&mut open, &mut out);
            open = Some((ts, ts, segment, Vec::new()));
        }
        let (_, end, _, apps) = open.as_mut().expect("a block is open");
        *end = ts;
        if row.app_bundle_id.is_empty() {
            continue;
        }
        let slot = match apps.iter().position(|(a, _)| *a == row.app_bundle_id) {
            Some(i) => i,
            None => {
                apps.push((row.app_bundle_id.clone(), Vec::new()));
                apps.len() - 1
            }
        };
        let titles = &mut apps[slot].1;
        if !row.title.is_empty() && !titles.contains(&row.title) {
            titles.push(row.title.clone());
        }
    }
    finish(&mut open, &mut out);
    out
}

impl Store {
    /// Memory blocks for captures in `[start_unix_ms, end_unix_ms)`: one indexed read of time, app and
    /// window title (never body text), grouped by `group_blocks`. `meetings` are `(start, end)` intervals
    /// the blocks are cut around.
    pub fn day_blocks(
        &self,
        start_unix_ms: i64,
        end_unix_ms: i64,
        meetings: &[(i64, i64)],
    ) -> rusqlite::Result<Vec<DayBlock>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts_unix_ms, COALESCE(app_bundle_id, ''), COALESCE(title, '') FROM items \
             WHERE kind = 'capture' AND ts_unix_ms >= ?1 AND ts_unix_ms < ?2 ORDER BY ts_unix_ms ASC",
        )?;
        let rows: Vec<BlockRow> = stmt
            .query_map(rusqlite::params![start_unix_ms, end_unix_ms], |r| {
                Ok(BlockRow { ts_unix_ms: r.get(0)?, app_bundle_id: r.get(1)?, title: r.get(2)? })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(group_blocks(&rows, meetings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn row(min: i64, app: &str, title: &str) -> BlockRow {
        BlockRow { ts_unix_ms: min * MIN, app_bundle_id: app.into(), title: title.into() }
    }

    #[test]
    fn a_gap_over_ten_minutes_starts_a_new_block_and_exactly_ten_does_not() {
        let rows = [row(0, "a", "x"), row(10, "a", "y"), row(21, "a", "z")];
        let blocks = group_blocks(&rows, &[]);
        assert_eq!(blocks.len(), 2);
        assert_eq!((blocks[0].start_unix_ms, blocks[0].end_unix_ms), (0, 10 * MIN));
        assert_eq!((blocks[1].start_unix_ms, blocks[1].end_unix_ms), (21 * MIN, 21 * MIN));
        assert_eq!(BLOCK_GAP_MS, 10 * MIN);
    }

    #[test]
    fn apps_keep_first_seen_order_and_titles_dedupe_and_skip_empties() {
        let rows = [
            row(0, "b", "one"),
            row(1, "a", ""),
            row(2, "b", "two"),
            row(3, "b", "one"),
            row(4, "a", "alpha"),
        ];
        let blocks = group_blocks(&rows, &[]);
        assert_eq!(blocks.len(), 1);
        let apps = &blocks[0].apps;
        assert_eq!(apps.iter().map(|a| a.app_bundle_id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(apps[0].items, ["one", "two"]);
        assert_eq!(apps[1].items, ["alpha"]);
        assert_eq!((apps[0].more_items, blocks[0].more_apps), (0, 0));
    }

    #[test]
    fn four_apps_and_three_titles_are_shown_and_the_rest_counted() {
        let mut rows = Vec::new();
        for (i, app) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
            rows.push(row(i as i64, app, "t"));
        }
        for n in 0..5 {
            rows.push(row(7, "a", &format!("title {n}")));
        }
        let blocks = group_blocks(&rows, &[]);
        let b = &blocks[0];
        assert_eq!(b.apps.len(), BLOCK_MAX_APPS);
        assert_eq!(b.more_apps, 2);
        // "a" has t + 5 titles = 6 distinct; 3 shown, 3 left.
        assert_eq!(b.apps[0].items, ["t", "title 0", "title 1"]);
        assert_eq!(b.apps[0].more_items, 3);
        assert_eq!(b.apps[1].more_items, 0);
    }

    #[test]
    fn a_meeting_drops_the_rows_inside_it_and_splits_the_block_around_it() {
        let rows = [row(0, "a", "before"), row(5, "a", "during"), row(9, "a", "after"), row(10, "a", "after too")];
        // Half open: the row at the meeting's end belongs to the work after it.
        let blocks = group_blocks(&rows, &[(4 * MIN, 9 * MIN)]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].apps[0].items, ["before"]);
        assert_eq!((blocks[1].start_unix_ms, blocks[1].end_unix_ms), (9 * MIN, 10 * MIN));
        assert_eq!(blocks[1].apps[0].items, ["after", "after too"]);
    }

    #[test]
    fn a_block_wholly_inside_a_meeting_vanishes_and_empty_meetings_cut_nothing() {
        let rows = [row(5, "a", "x"), row(6, "a", "y")];
        assert!(group_blocks(&rows, &[(0, 10 * MIN)]).is_empty());
        assert_eq!(group_blocks(&rows, &[(3 * MIN, 3 * MIN), (9 * MIN, 2 * MIN)]).len(), 1);
        assert!(group_blocks(&[], &[(0, MIN)]).is_empty());
    }

    #[test]
    fn two_meetings_with_no_rows_between_them_make_no_empty_block() {
        let rows = [row(0, "a", "x"), row(20, "a", "y")];
        let blocks = group_blocks(&rows, &[(5 * MIN, 8 * MIN), (8 * MIN, 12 * MIN)]);
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn the_store_read_selects_only_a_range_of_captures_and_never_returns_body_text() {
        let store = Store::open_in_memory().unwrap();
        let mut n = 0;
        let mut put = |ts: i64, app: &str, title: &str, text: &str| {
            n += 1;
            store
                .insert_capture_item(&crate::NewCaptureItem {
                    device_id: "d".into(),
                    ts_unix_ms: ts,
                    app_bundle_id: app.into(),
                    window_title: title.into(),
                    url: None,
                    text: text.into(),
                    hash: n,
                    source: "ax".into(),
                })
                .unwrap();
        };
        put(0, "com.a", "Doc", "SECRET BODY");
        put(MIN, "com.b", "Mail", "other body");
        put(2 * MIN, "com.a", "Doc", "again");
        put(30 * MIN, "com.a", "Late", "later");
        put(99 * MIN, "com.a", "Outside", "x");
        let blocks = store.day_blocks(0, 90 * MIN, &[]).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].apps.len(), 2);
        assert_eq!(blocks[0].apps[0].app_bundle_id, "com.a");
        assert_eq!(blocks[0].apps[0].items, ["Doc"]);
        assert_eq!(blocks[1].apps[0].items, ["Late"]);
        let json = serde_json::to_string(&blocks).unwrap();
        assert!(!json.contains("BODY") && !json.contains("body"), "{json}");
        // The meeting cut goes through the same read.
        let cut = store.day_blocks(0, 90 * MIN, &[(MIN, 3 * MIN)]).unwrap();
        assert_eq!(cut[0].apps.len(), 1);
        assert_eq!((cut[0].start_unix_ms, cut[0].end_unix_ms), (0, 0));
        // The read is served by the (kind, time) index.
        let plan: String = store
            .conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT ts_unix_ms, app_bundle_id, title FROM items \
                 WHERE kind = 'capture' AND ts_unix_ms >= 0 AND ts_unix_ms < 1 ORDER BY ts_unix_ms ASC",
            )
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(plan.contains("idx_items_kind_ts"), "{plan}");
    }
}
