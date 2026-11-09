//! Review program for the topics success test (run on a COPY of a real database):
//!   cargo run --release -p api --example topics_review -- <db> <models dir> <seeds.txt> [--reset-topics] [--judge]
//! seeds.txt: one topic per line, `Name | alias, alias | kind`. Writes `topics-review.md`
//! next to the db. `--judge` is a paid TypeSafe call: needs TYPESAFE_API_KEY, prints the
//! estimate and waits for `y` on stdin.
use memory::topics::{run_topic_batch, seeds::SeedKind};
use rusqlite::Connection;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

const USD_PER_MILLION: f64 = 0.042;
const TOPIC_TABLES: [&str; 15] = [
    "topic_member",
    "topic_source_vec",
    "topic_source_text",
    "topic_source",
    "topic_stats",
    "topic_idea",
    "topic_dirty",
    "entity_mention",
    "entity",
    "topic_seed",
    "topic_series",
    "topic_judge_todo",
    "topic_verdict",
    "topic_chrome",
    "topic",
];
const SAMPLE: &str = "ORDER BY (m.source_id * 2654435761 + m.topic_id * 40503) % 1000003 LIMIT 20";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let (db, models, seeds) = (Path::new(pos[0]), Path::new(pos[1]), pos[2]);
    let reset = args.iter().any(|a| a == "--reset-topics");
    let judge = args.iter().any(|a| a == "--judge");
    if reset
        && !std::fs::canonicalize(db)
            .unwrap()
            .to_string_lossy()
            .contains("scratchpad")
    {
        panic!(
            "--reset-topics is destructive: refusing, db path is not under a scratchpad directory"
        );
    }
    let store = memory::Store::open(db, models).expect("open");
    let conn = Connection::open(db).expect("open reader"); // sqlite-vec is registered by Store::open
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    if reset {
        for t in TOPIC_TABLES {
            conn.execute(&format!("DELETE FROM {t}"), []).unwrap();
        }
        conn.execute(
            "DELETE FROM kv_meta WHERE key LIKE 'topic\\_%' ESCAPE '\\' OR key LIKE 'topic\\_index\\_hw:%' ESCAPE '\\'",
            [],
        )
        .unwrap();
        eprintln!("topic tables reset");
    }

    for line in std::fs::read_to_string(seeds)
        .expect("seeds")
        .lines()
        .filter(|l| !l.trim().is_empty())
    {
        let f: Vec<&str> = line.split('|').map(str::trim).collect();
        let aliases: Vec<String> = f
            .get(1)
            .map(|a| {
                a.split(',')
                    .map(str::trim)
                    .filter(|a| !a.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let kind = f.get(2).and_then(|k| SeedKind::parse(k));
        store.create_seed_topic(f[0], &aliases, kind).expect("seed");
    }

    let t = Instant::now();
    let mut n = 0;
    while run_topic_batch(&store, 200).expect("batch") {
        n += 1;
        if n % 25 == 0 {
            let (s, a): (i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*), COUNT(assigned_at_unix_ms) FROM topic_source",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            eprintln!(
                "batch {n}: {s} sources indexed, {a} assigned, {:?}",
                t.elapsed()
            );
        }
    }
    eprintln!("topics done in {:?}", t.elapsed());

    let est = store.topic_judge_estimate(USD_PER_MILLION).unwrap();
    println!(
        "judge queue: {} sources, ~{} tokens, ${:.2} at {USD_PER_MILLION}/M (indexing: {})",
        est.sources, est.tokens, est.usd, est.indexing
    );
    if judge {
        let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
        println!(
            "Judge {} sources for about ${:.2}? [y/N]",
            est.sources, est.usd
        );
        let mut ans = String::new();
        std::io::stdin().read_line(&mut ans).unwrap();
        if ans.trim() == "y" {
            let client = llm::typesafe::TypeSafeClient::new(key);
            store.start_topic_judging(est.tokens * 3 / 2).unwrap();
            loop {
                let o =
                    api::topic_judge::run_topic_judge_batch(&store, &client, 50, USD_PER_MILLION)
                        .unwrap();
                eprintln!(
                    "judged {} (rejected {}, over budget {})",
                    o.processed, o.auth_rejected, o.over_budget
                );
                if o.processed == 0 || o.auth_rejected || o.over_budget || o.paused_for_today {
                    break;
                }
            }
            while run_topic_batch(&store, 200).unwrap() {}
        }
    }

    for _ in 0..1000 {
        if store.topics_needing_names(1).unwrap().is_empty() {
            break;
        }
        api::topics::run_naming_batch_with_complete(&store, 200, "en", |_| Err("no model".into()))
            .unwrap();
    }
    write_review(&conn, &db.with_file_name("topics-review.md"));
}

fn day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn rows<T>(
    conn: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
    f: impl Fn(&rusqlite::Row) -> T,
) -> Vec<T> {
    conn.prepare(sql)
        .unwrap()
        .query_map(p, |r| Ok(f(r)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn write_review(conn: &Connection, path: &Path) {
    let mut md = String::from("# Topics review\n");
    let seeds = rows(conn, "SELECT t.id, t.name FROM topic t JOIN topic_seed s ON s.topic_id = t.id WHERE s.deleted = 0 AND t.merged_into IS NULL ORDER BY t.id", [], |r| (r.get::<_, i64>(0).unwrap(), r.get::<_, String>(1).unwrap()));
    println!("\nper seed (items | by origin | by source kind):");
    for (id, name) in &seeds {
        let by = |col: &str| {
            rows(conn, &format!("SELECT {col}, COUNT(*) FROM topic_member m JOIN topic_source s ON s.id = m.source_id WHERE m.topic_id = ?1 AND m.origin != 'removed' GROUP BY 1 ORDER BY 2 DESC"), [id], |r| format!("{}={}", r.get::<_, String>(0).unwrap(), r.get::<_, i64>(1).unwrap())).join(" ")
        };
        let (origins, kinds) = (by("m.origin"), by("s.kind"));
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM topic_member WHERE topic_id = ?1 AND origin != 'removed'",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        println!("  {name}: {n} | {origins} | {kinds}");
        writeln!(
            md,
            "\n## {name}: {n} items\nOrigins: {origins}\nKinds: {kinds}\n"
        )
        .unwrap();
        let sample = rows(conn, &format!("SELECT s.kind, s.ts_unix_ms, s.title, COALESCE(substr(x.text, 1, 120), ''), m.origin FROM topic_member m JOIN topic_source s ON s.id = m.source_id LEFT JOIN topic_source_text x ON x.source_id = s.id WHERE m.topic_id = ?1 AND m.origin != 'removed' {SAMPLE}"), [id], |r| (r.get::<_, String>(0).unwrap(), r.get::<_, i64>(1).unwrap(), r.get::<_, String>(2).unwrap(), r.get::<_, String>(3).unwrap(), r.get::<_, String>(4).unwrap()));
        for (k, ts, title, text, origin) in sample {
            writeln!(
                md,
                "- [{k}] {} | {title} | {} | {origin}",
                day(ts),
                text.replace('\n', " ")
            )
            .unwrap();
        }
    }
    let q: i64 = conn
        .query_row("SELECT COUNT(*) FROM topic_judge_todo", [], |r| r.get(0))
        .unwrap();
    println!("topic_judge_todo: {q} sources");

    md.push_str("\n# Top 50 topics by size\nFlag: most members are chrome-only episodes (text under 40 chars), one calendar title, or one repeated email subject.\n");
    let top = rows(conn, "SELECT t.id, t.name, COUNT(*) c FROM topic t JOIN topic_member m ON m.topic_id = t.id AND m.origin != 'removed' WHERE t.merged_into IS NULL GROUP BY t.id ORDER BY c DESC LIMIT 50", [], |r| (r.get::<_, i64>(0).unwrap(), r.get::<_, String>(1).unwrap(), r.get::<_, i64>(2).unwrap()));
    for (id, name, c) in top {
        let kinds = rows(conn, "SELECT s.kind, COUNT(*) FROM topic_member m JOIN topic_source s ON s.id = m.source_id WHERE m.topic_id = ?1 AND m.origin != 'removed' GROUP BY 1 ORDER BY 2 DESC", [id], |r| format!("{}={}", r.get::<_, String>(0).unwrap(), r.get::<_, i64>(1).unwrap())).join(" ");
        let ents = rows(conn, "SELECT e.display, COUNT(*) FROM topic_member m JOIN entity_mention em ON em.source_id = m.source_id JOIN entity e ON e.id = em.entity_id WHERE m.topic_id = ?1 AND m.origin != 'removed' GROUP BY e.id ORDER BY 2 DESC LIMIT 5", [id], |r| format!("{} ({})", r.get::<_, String>(0).unwrap(), r.get::<_, i64>(1).unwrap())).join(", ");
        let one = |kind: &str| -> i64 {
            conn.query_row("SELECT COALESCE(MAX(n), 0) FROM (SELECT COUNT(*) n FROM topic_member m JOIN topic_source s ON s.id = m.source_id WHERE m.topic_id = ?1 AND m.origin != 'removed' AND s.kind = ?2 GROUP BY s.title)", rusqlite::params![id, kind], |r| r.get(0)).unwrap()
        };
        let chrome: i64 = conn.query_row("SELECT COUNT(*) FROM topic_member m JOIN topic_source s ON s.id = m.source_id LEFT JOIN topic_source_text x ON x.source_id = s.id WHERE m.topic_id = ?1 AND m.origin != 'removed' AND s.kind = 'episode' AND LENGTH(COALESCE(x.text, '')) < 40", [id], |r| r.get(0)).unwrap();
        let mut flags = vec![];
        for (what, n) in [
            ("chrome-only episodes", chrome),
            ("one calendar title", one("calendar_event")),
            ("one email subject", one("email")),
        ] {
            if n * 2 > c {
                flags.push(format!("{what} {n}/{c}"));
            }
        }
        let flag = if flags.is_empty() {
            String::new()
        } else {
            format!("  FLAG: {}", flags.join("; "))
        };
        writeln!(md, "- {name} ({c}) | {kinds} | {ents}{flag}").unwrap();
    }
    std::fs::write(path, md).unwrap();
    eprintln!("wrote {}", path.display());
}
