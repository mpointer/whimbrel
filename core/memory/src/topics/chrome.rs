//! Screen chrome: word runs (menus, tab bars, bookmark bars, sidebars) repeated across many
//! episodes. They are cut from episode topic text so they never become phrases, alias hits or
//! cluster signal. Ingest collapses all whitespace, so a capture body is one long line: chrome
//! is found as word 3-grams, not lines.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};

// v2: chrome is stored per app; the new key rebuilds the table at once on upgrade.
const BUILT_KEY: &str = "topic_chrome_built_at_v2";
/// The `topic_chrome.app` of 3-grams that are chrome in every app.
const GLOBAL: &str = "*";
const REBUILD_MS: i64 = 86_400_000;
const WINDOW_MS: i64 = 30 * REBUILD_MS;
const MAX_ITEMS: i64 = 200; // same window as the episode vector sample
const MAX_EPISODES: i64 = 2000; // ponytail: newest 2000 episodes bound the rebuild scan
const MIN_EPISODES: usize = 5; // with fewer episodes, "more than 20%" proves nothing
const TEXT_CAP: usize = 1_500;
const SHINGLE: usize = 3;

fn lower_words(body: &str) -> Vec<String> {
    body.split_whitespace().map(str::to_lowercase).collect()
}

/// FNV-1a over lowercase words, stored as i64.
fn hash(words: &[String]) -> i64 {
    super::fnv1a(words.iter().flat_map(|w| w.bytes().chain(*b" "))) as i64
}

/// Chrome 3-gram hashes by app (`GLOBAL` for every app).
pub(crate) type Chrome = HashMap<String, HashSet<i64>>;

pub(crate) fn load_chrome(conn: &Connection) -> rusqlite::Result<Chrome> {
    let mut out = Chrome::new();
    for row in conn
        .prepare("SELECT app, hash FROM topic_chrome")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?
    {
        let (app, h) = row?;
        out.entry(app).or_default().insert(h);
    }
    Ok(out)
}

/// Rebuilds `topic_chrome` when older than 24 h: word 3-grams present in more than 20% of
/// the last 30 days' episodes (chrome everywhere), or of those of one app with at least 5
/// episodes (chrome in that app only).
pub(crate) fn rebuild_chrome_if_stale(conn: &Connection, now_ms: i64) -> rusqlite::Result<()> {
    let built: i64 = conn
        .query_row(
            "SELECT value FROM kv_meta WHERE key = ?1",
            [BUILT_KEY],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if now_ms - built < REBUILD_MS {
        return Ok(());
    }
    let eps: Vec<(String, String, i64, i64)> = conn
        .prepare(
            "SELECT device_id, COALESCE(app_bundle_id, ''), start_item_id, end_item_id \
             FROM episodes WHERE start_ts_unix_ms >= ?1 ORDER BY start_ts_unix_ms DESC LIMIT ?2",
        )?
        .query_map(params![now_ms - WINDOW_MS, MAX_EPISODES], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut all: HashMap<i64, usize> = HashMap::new();
    let mut apps: HashMap<&str, (usize, HashMap<i64, usize>)> = HashMap::new();
    for (device, app, lo, hi) in &eps {
        let mut stmt = conn.prepare_cached(
            "SELECT body_text FROM items WHERE kind = 'capture' AND device_id = ?1 \
             AND id BETWEEN ?2 AND ?3 ORDER BY id LIMIT ?4",
        )?;
        let mut seen = HashSet::new();
        for body in stmt.query_map(params![device, lo, hi, MAX_ITEMS], |r| {
            r.get::<_, String>(0)
        })? {
            seen.extend(lower_words(&body?).windows(SHINGLE).map(hash));
        }
        let (n, counts) = apps.entry(app).or_default();
        *n += 1;
        for h in seen {
            *all.entry(h).or_default() += 1;
            *counts.entry(h).or_default() += 1;
        }
    }
    let over = |c: usize, n: usize| n >= MIN_EPISODES && c * 5 > n;
    conn.execute("DELETE FROM topic_chrome", [])?;
    let mut ins =
        conn.prepare_cached("INSERT OR IGNORE INTO topic_chrome (app, hash) VALUES (?1, ?2)")?;
    for (h, c) in &all {
        if over(*c, eps.len()) {
            ins.execute(params![GLOBAL, h])?;
        }
    }
    for (app, (n, counts)) in &apps {
        for (h, c) in counts {
            if over(*c, *n) {
                ins.execute(params![app, h])?;
            }
        }
    }
    conn.execute(
        "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![BUILT_KEY, now_ms.to_string()],
    )?;
    Ok(())
}

/// Distinct runs of non-chrome words from an episode's captures, read lazily until `TEXT_CAP`
/// chars. A word inside a 3-gram that is chrome everywhere or in `app` is cut.
pub(crate) fn capture_lines(
    conn: &Connection,
    device: &str,
    app: &str,
    lo: i64,
    hi: i64,
    chrome: &Chrome,
) -> rusqlite::Result<Vec<String>> {
    let sets: Vec<&HashSet<i64>> = [GLOBAL, app]
        .iter()
        .filter_map(|a| chrome.get(*a))
        .collect();
    let mut stmt = conn.prepare_cached(
        "SELECT body_text FROM items WHERE kind = 'capture' AND device_id = ?1 \
         AND id BETWEEN ?2 AND ?3 ORDER BY id LIMIT ?4",
    )?;
    let mut rows = stmt.query(params![device, lo, hi, MAX_ITEMS])?;
    let (mut out, mut seen, mut chars) = (Vec::new(), HashSet::new(), 0);
    while let Some(row) = rows.next()? {
        let body: String = row.get(0)?;
        let words: Vec<&str> = body.split_whitespace().collect();
        let mut keep = vec![true; words.len()];
        for (i, w) in lower_words(&body).windows(SHINGLE).enumerate() {
            let h = hash(w);
            if sets.iter().any(|s| s.contains(&h)) {
                keep[i..i + SHINGLE].fill(false);
            }
        }
        let runs = words
            .iter()
            .zip(&keep)
            .collect::<Vec<_>>()
            .split(|(_, k)| !**k)
            .map(|r| r.iter().map(|(w, _)| **w).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>();
        for run in runs {
            if !run.is_empty() && seen.insert(run.to_lowercase()) {
                chars += run.chars().count() + 1;
                out.push(run);
            }
        }
        if chars >= TEXT_CAP {
            break;
        }
    }
    Ok(out)
}

/// Summary (if any), window title, then the capture runs, de-duplicated and capped.
pub(crate) fn episode_topic_text(
    summary: &str,
    window_title: &str,
    capture_lines: &[String],
) -> (String, usize) {
    let mut seen = HashSet::new();
    let mut parts: Vec<&str> = Vec::new();
    let mut head_parts = 0;
    for (i, s) in [window_title, summary]
        .into_iter()
        .chain(capture_lines.iter().map(String::as_str))
        .enumerate()
    {
        let k = s
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if !k.is_empty() && seen.insert(k) {
            parts.push(s.trim());
            if i == 0 {
                head_parts += 1;
            }
        }
    }
    // Redacted before the cut so a secret straddling it cannot survive in part.
    let head = crate::redact::redact_secrets(&parts[..head_parts].join("\n"))
        .chars()
        .count();
    let text: String = crate::redact::redact_secrets(&parts.join("\n"))
        .chars()
        .take(TEXT_CAP)
        .collect();
    let head = head.min(text.chars().count());
    (text, head)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::FakeEmbedder;
    use crate::Store;
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s =
            Store::open_with_embedder(&dir.path().join("t.db"), Arc::new(FakeEmbedder)).unwrap();
        (dir, s)
    }

    const NOW: i64 = 100 * REBUILD_MS;

    /// One episode of app `app` whose single capture is `body`; returns the capture's id.
    fn episode(s: &Store, app: &str, body: &str) -> i64 {
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, body_text) VALUES ('d','capture','screen',?1,?2)",
                params![NOW - 1000, body],
            )
            .unwrap();
        let id = s.conn.last_insert_rowid();
        s.conn.execute("INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) VALUES ('d',?3,'Doc',?1,?1,?2,?2,1,'pending',1,1)", params![NOW - 1000, id, app]).unwrap();
        id
    }

    fn text_of(s: &Store, app: &str, item: i64) -> String {
        let chrome = load_chrome(&s.conn).unwrap();
        capture_lines(&s.conn, "d", app, item, item, &chrome)
            .unwrap()
            .join("\n")
    }

    // Ingest collapses all whitespace, so real capture bodies are one line each (the cause of
    // the empty chrome table): chrome must be found inside single-line bodies.
    #[test]
    fn chrome_inside_single_line_bodies_is_dropped() {
        let (_d, s) = store();
        let mut first = 0;
        for i in 0..10 {
            let body = match i {
                0..=2 => {
                    format!("Back Forward Reload View site information Falcon{i} launch plan{i}")
                }
                3 => "Q3 pricing sheet for Falcon".to_string(),
                _ => format!("something else entirely number{i}"),
            };
            let id = episode(&s, "a", &body);
            if i == 0 {
                first = id;
            }
        }
        rebuild_chrome_if_stale(&s.conn, NOW).unwrap();
        let t = text_of(&s, "a", first);
        assert!(!t.contains("Reload"), "{t}");
        assert!(t.contains("Falcon0 launch plan0"), "{t}");
        // Within 24 h nothing is rebuilt, even if the data changes.
        s.conn.execute("DELETE FROM episodes", []).unwrap();
        rebuild_chrome_if_stale(&s.conn, NOW + REBUILD_MS - 1).unwrap();
        assert!(!text_of(&s, "a", first).contains("Reload"));
        rebuild_chrome_if_stale(&s.conn, NOW + REBUILD_MS).unwrap();
        assert!(text_of(&s, "a", first).contains("Reload"));
    }

    #[test]
    fn chrome_is_also_counted_per_app() {
        let (_d, s) = store();
        // 3 of app a's 10 episodes (30%) but 3 of all 60 (5%).
        let (mut first, mut in_b) = (0, 0);
        for i in 0..61 {
            let (app, body) = match i {
                0..=2 => (
                    "a",
                    format!("NordPass Password Manager Digital Vault note{i} alpha{i} beta{i}"),
                ),
                3..=9 => ("a", format!("plain note{i} alpha{i} beta{i}")),
                60 => (
                    "b",
                    "Review the NordPass Password Manager contract".to_string(),
                ),
                _ => ("b", format!("other note{i} alpha{i} beta{i}")),
            };
            let id = episode(&s, app, &body);
            match i {
                0 => first = id,
                60 => in_b = id,
                _ => {}
            }
        }
        rebuild_chrome_if_stale(&s.conn, NOW).unwrap();
        let t = text_of(&s, "a", first);
        assert!(!t.contains("NordPass"), "{t}");
        assert!(t.contains("alpha0 beta0"), "{t}");
        // Chrome of app a only: kept in app b's text.
        let t = text_of(&s, "b", in_b);
        assert!(t.contains("NordPass Password Manager"), "{t}");
    }

    #[test]
    fn episode_text_puts_title_first_and_dedupes() {
        let lines = vec![
            "Falcon launch plan".to_string(),
            "falcon  launch plan".to_string(),
        ];
        let (t, head) = episode_topic_text("Planned the launch", "Doc", &lines);
        assert_eq!(head, "Doc".chars().count());
        assert_eq!(t, "Doc\nPlanned the launch\nFalcon launch plan");
    }
}
