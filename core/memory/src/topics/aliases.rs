//! Alias matching: which seed topics' aliases appear in a source's text, and the pass
//! that files unassigned sources under seed topics before clustering.

use rusqlite::{params, Connection, OptionalExtension};

/// Topic ids (deduped, in seed order) with at least one alias in `text`.
/// Case-insensitive, except an all-capitals alias under 4 characters (RAG, CMS) is
/// case-sensitive. A match must sit on word boundaries; whitespace runs are equivalent.
pub fn alias_hits(text: &str, seeds: &[(i64, Vec<String>)]) -> Vec<i64> {
    alias_counts(text, seeds)
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(id, _)| id)
        .collect()
}

/// Sources whose text starts with a head (an episode's window title, an email's subject and
/// sender, a meeting's title): only a name there files the source. A mention in the rest
/// (summary, screen text, body) only makes the topic a candidate for Jev.
pub fn head_alias_hits(text: &str, head_len: usize, seeds: &[(i64, Vec<String>)]) -> Vec<i64> {
    let split = text
        .char_indices()
        .nth(head_len)
        .map_or(text.len(), |(i, _)| i);
    alias_hits(&text[..split], seeds)
}

/// Up to `max` passages of `text` (about `radius` characters either side) around matches of
/// `aliases`, under the same matching rules as `alias_hits`. Long emails and meeting
/// transcripts lose these mentions to the stored-text cap; Jev reads them from here.
pub fn alias_snippets(text: &str, aliases: &[String], radius: usize, max: usize) -> Vec<String> {
    let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let plain = words(text);
    let lower = plain.to_lowercase();
    let ok = |h: &str, i: usize, n: usize| {
        let f = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
        f(h[..i].chars().next_back()) && f(h[i + n..].chars().next())
    };
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for a in aliases {
        let a = words(a);
        if a.is_empty() {
            continue;
        }
        let exact = a.chars().count() < 4 && a == a.to_uppercase() && a != a.to_lowercase();
        let (hay, needle) = if exact {
            (&plain, a.clone())
        } else {
            (&lower, a.to_lowercase())
        };
        for (i, m) in hay.match_indices(needle.as_str()) {
            if ok(hay, i, m.len()) {
                spans.push((i, i + m.len()));
            }
        }
    }
    spans.sort();
    let mut out: Vec<String> = Vec::new();
    let mut last_end = 0;
    for (start, end) in spans {
        if out.len() == max {
            break;
        }
        if !out.is_empty() && start < last_end {
            continue; // inside the previous passage
        }
        let mut lo = start.saturating_sub(radius);
        while !plain.is_char_boundary(lo) {
            lo -= 1;
        }
        let mut hi = (end + radius).min(plain.len());
        while !plain.is_char_boundary(hi) {
            hi += 1;
        }
        out.push(plain[lo..hi].to_string());
        last_end = hi;
    }
    out
}

/// Per seed, in seed order: how many alias matches `text` holds.
fn alias_counts(text: &str, seeds: &[(i64, Vec<String>)]) -> Vec<(i64, usize)> {
    let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let plain = words(text);
    let lower = plain.to_lowercase();
    seeds
        .iter()
        .map(|(id, aliases)| {
            let n = aliases
                .iter()
                .map(|a| {
                    let a = words(a);
                    if a.is_empty() {
                        return 0;
                    }
                    let exact =
                        a.chars().count() < 4 && a == a.to_uppercase() && a != a.to_lowercase();
                    if exact {
                        count_word(&plain, &a)
                    } else {
                        count_word(&lower, &a.to_lowercase())
                    }
                })
                .sum();
            (*id, n)
        })
        .collect()
}

fn count_word(hay: &str, needle: &str) -> usize {
    let ok = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    hay.match_indices(needle)
        .filter(|(i, m)| ok(hay[..*i].chars().next_back()) && ok(hay[i + m.len()..].chars().next()))
        .count()
}

const SEED_VER_KEY: &str = "topic_seed_ver";
const SWEPT_KEY: &str = "topic_seed_swept";
const CURSOR_KEY: &str = "topic_seed_sweep_cursor";
// ponytail: sweeps 2,000 sources per run; a table of N sources takes N/2000 runs after each
// seed change. Raise the chunk if that lag matters.
#[cfg(not(test))]
const SWEEP_CHUNK: i64 = 2_000;
#[cfg(test)]
const SWEEP_CHUNK: i64 = 2;

/// Marks the seed set changed: every source is swept again by `assign_seeds`.
pub(crate) fn bump_seed_version(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kv_meta (key, value) VALUES (?1, '1') \
         ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
        [SEED_VER_KEY],
    )?;
    Ok(())
}

fn kv(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM kv_meta WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

fn kv_put(conn: &Connection, key: &str, v: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, v],
    )?;
    Ok(())
}

/// Files one source: `'seed'` members for its hits (upgrading an `auto` row, leaving `user`
/// and `removed`); exactly one hit leaves the judge queue, 0 or several join it unless a
/// verdict is already stored for the source's key, which is applied instead.
fn file_source(
    tx: &Connection,
    own_device: Option<&str>,
    sid: i64,
    text: &str,
    head_len: Option<i64>,
    seeds: &[(i64, Vec<String>)],
) -> rusqlite::Result<()> {
    let hits = match head_len {
        Some(h) => head_alias_hits(text, h.max(0) as usize, seeds),
        None => alias_hits(text, seeds),
    };
    // One topic named: filed. None or several (a Terminal title carries both the folder and
    // the profile name): Jev picks, from those named first.
    if let [t] = hits[..] {
        tx.execute(
            "INSERT INTO topic_member (topic_id, source_id, origin, score) VALUES (?1, ?2, 'seed', 1.0) \
             ON CONFLICT(topic_id, source_id) DO UPDATE SET origin = 'seed' WHERE origin = 'auto'",
            params![t, sid],
        )?;
        tx.execute("DELETE FROM topic_judge_todo WHERE source_id = ?1", [sid])?;
    } else {
        tx.execute(
            "DELETE FROM topic_member WHERE source_id = ?1 AND origin = 'seed'",
            [sid],
        )?;
        if !super::judge::apply_stored_verdict(tx, own_device, sid)? {
            tx.execute(
                "INSERT OR IGNORE INTO topic_judge_todo (source_id) VALUES (?1)",
                [sid],
            )?;
        }
    }
    Ok(())
}

/// Live seed topics (not deleted, not merged) with their aliases, in topic id order.
pub(crate) fn live_seeds(conn: &Connection) -> rusqlite::Result<Vec<(i64, Vec<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT s.topic_id, s.aliases FROM topic_seed s JOIN topic t ON t.id = s.topic_id \
         WHERE s.deleted = 0 AND s.merged_into_key IS NULL AND t.merged_into IS NULL \
         ORDER BY s.topic_id",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get::<_, String>(1)?)))?
        .map(|r| r.map(|(id, a)| (id, serde_json::from_str(&a).unwrap_or_default())))
        .collect();
    rows
}

/// Files up to `batch` unassigned, non-noise sources (the same oldest-first set clustering
/// takes) under the seed topics whose aliases they contain, as `'seed'` members. Sources
/// with 0 or several hits are queued in `topic_judge_todo` for Jev (when a live seed
/// exists) unless a stored verdict decides them. When the seed set changed since the last
/// sweep, also sweeps already-assigned sources, `SWEEP_CHUNK` per run, resuming from a kv
/// cursor. Returns whether a sweep is still unfinished. A seed member is not an orphan, so
/// clustering will not start a new topic from it.
pub(crate) fn assign_seeds(
    tx: &Connection,
    own_device: Option<&str>,
    batch: usize,
) -> rusqlite::Result<bool> {
    let seeds = live_seeds(tx)?;
    if seeds.is_empty() {
        return Ok(false);
    }
    let todo: Vec<(i64, String, Option<i64>)> = {
        let mut stmt = tx.prepare(
            "SELECT s.id, COALESCE(x.text, s.title), x.head_len FROM topic_source s \
             LEFT JOIN topic_source_text x ON x.source_id = s.id \
             WHERE s.assigned_at_unix_ms IS NULL AND s.noise = 0 \
             ORDER BY s.ts_unix_ms, s.id LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([batch as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    for (sid, text, head) in &todo {
        file_source(tx, own_device, *sid, text, *head, &seeds)?;
    }

    let ver = kv(tx, SEED_VER_KEY)?.unwrap_or_default();
    if kv(tx, SWEPT_KEY)?.as_deref() == Some(ver.as_str()) {
        return Ok(false);
    }
    // Cursor is "ver:last_id"; a cursor from an older seed version restarts the sweep.
    let from: i64 = kv(tx, CURSOR_KEY)?
        .and_then(|c| {
            let (v, id) = c.split_once(':')?;
            (v == ver).then(|| id.parse().ok())?
        })
        .unwrap_or(0);
    let chunk: Vec<(i64, String, Option<i64>)> = {
        let mut stmt = tx.prepare(
            "SELECT source_id, text, head_len FROM topic_source_text x JOIN topic_source s ON s.id = x.source_id \
             WHERE x.source_id > ?1 AND s.noise = 0 ORDER BY x.source_id LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![from, SWEEP_CHUNK], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    for (sid, text, head) in &chunk {
        file_source(tx, own_device, *sid, text, *head, &seeds)?;
    }
    if (chunk.len() as i64) < SWEEP_CHUNK {
        kv_put(tx, SWEPT_KEY, &ver)?;
        Ok(false)
    } else {
        let last = chunk.last().map_or(from, |c| c.0);
        kv_put(tx, CURSOR_KEY, &format!("{ver}:{last}"))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeds(list: &[(i64, &str)]) -> Vec<(i64, Vec<String>)> {
        list.iter()
            .map(|(i, a)| (*i, vec![a.to_string()]))
            .collect()
    }

    #[test]
    fn short_capital_alias_matches_whole_word_case_sensitive() {
        let s = seeds(&[(7, "RAG")]);
        assert_eq!(alias_hits("Notes on RAG eval", &s), vec![7]);
        for t in ["Brian Piragis", "drag the file", "rag doll"] {
            assert!(alias_hits(t, &s).is_empty(), "{t}");
        }
    }

    #[test]
    fn long_alias_matches_case_insensitive_across_whitespace() {
        let s = seeds(&[(3, "Genesys AVA")]);
        assert_eq!(alias_hits("genesys  AVA workshop", &s), vec![3]);
        assert_eq!(alias_hits("x\tGENESYS\nava.", &s), vec![3]);
    }

    #[test]
    fn only_the_head_files_directly() {
        let s = seeds(&[(1, "NewsHound"), (2, "Maximus")]);
        // Window title "Maximus pricing review"; the rest is summary and screen text.
        let text = "Maximus pricing review\nThe user also had NewsHound NewsHound open";
        assert_eq!(head_alias_hits(text, 22, &s), vec![2]);
        assert!(head_alias_hits("Realti work\nNewsHound Maximus tabs", 11, &s).is_empty());
    }

    #[test]
    fn two_seeds_both_hit() {
        let s = seeds(&[(1, "Maximus"), (2, "CMS")]);
        assert_eq!(alias_hits("Maximus CMS handoff", &s), vec![1, 2]);
    }
}
