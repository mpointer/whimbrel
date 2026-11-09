//! Review program for topic steps, run on a COPY of a real database with the LOCAL model:
//!   cargo run --release -p api --example topic_steps_review -- <db copy> <ollama model>
//!     [--names "Mike,Michael,Mike Pointer"] [--seeds seeds.txt] [--models <dir>] [--max <sources>]
//! Sets the names, loads seeds when the copy has none, indexes, finds steps with the local
//! model until the queue is idle (or `--max` sources), and writes `topic-steps-review.md`
//! next to the db: per seed topic, its page and each step's source and line. It takes no key
//! and never calls a hosted model.
use memory::topics::seeds::SeedKind;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let flags = ["--names", "--seeds", "--models", "--max"];
    let pos: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            !a.starts_with("--") && (*i == 0 || !flags.contains(&args[i - 1].as_str()))
        })
        .map(|(_, a)| a)
        .collect();
    let [db, model, ..] = pos[..] else {
        eprintln!("usage: topic_steps_review <db copy> <ollama model> [--names ..] [--seeds ..] [--models ..] [--max ..]");
        std::process::exit(2);
    };
    let (db, model) = (Path::new(db), model.as_str());
    // Never the live database: refuse anything inside the app data folder.
    let live = Path::new(&std::env::var("HOME").unwrap_or_default())
        .join("Library/Application Support/Whimbrel");
    let live = live.canonicalize().unwrap_or(live);
    let abs = db
        .canonicalize()
        .or_else(|_| std::path::absolute(db))
        .unwrap_or_else(|_| db.into());
    let lower = |p: &Path| p.to_string_lossy().to_lowercase();
    if Path::new(&lower(&abs)).starts_with(lower(&live)) {
        eprintln!(
            "refusing {}: it is inside the app data folder; pass a copy",
            abs.display()
        );
        std::process::exit(2);
    }
    let max: usize = flag("--max").map_or(usize::MAX, |m| m.parse().expect("--max"));
    let models = flag("--models").map_or(db.with_file_name("models"), |m| m.into());
    let store = memory::Store::open(db, &models).expect("open");

    if let Some(names) = flag("--names") {
        let names: Vec<String> = names.split(',').map(|n| n.trim().to_string()).collect();
        store.set_self_names(&names).expect("names");
    }
    if store.topic_page_list(0).expect("list").is_empty() {
        let seeds = flag("--seeds").expect("copy has no seed topics: pass --seeds <file>");
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
            store
                .create_seed_topic(f[0], &aliases, f.get(2).and_then(|k| SeedKind::parse(k)))
                .expect("seed");
        }
    }

    let t = Instant::now();
    let mut n = 0;
    while memory::topics::run_topic_batch(&store, 200).expect("batch") {
        n += 1;
        if n % 25 == 0 {
            eprintln!("indexing batch {n}, {:?}", t.elapsed());
        }
    }
    eprintln!("topics indexed in {:?}", t.elapsed());

    let chain = api::routing::build_local_chain(model, &api::routing::RoutingConfig::default());
    let t = Instant::now();
    let (mut done, mut stopped) = (0usize, "idle");
    while done < max {
        let o = api::topic_steps::run_local_steps_batch(&store, &chain, 10.min(max - done))
            .expect("steps");
        done += o.processed;
        eprintln!("{done} sources read, {:?}", t.elapsed());
        if o.no_model {
            stopped = "no model reachable";
            break;
        }
        if o.processed == 0 {
            break;
        }
    }
    if done >= max {
        stopped = "capped";
    }
    eprintln!("steps done in {:?} ({stopped})", t.elapsed());
    write_review(&store, db, model, done, stopped, t.elapsed());
}

fn day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn write_review(
    store: &memory::Store,
    db: &Path,
    model: &str,
    done: usize,
    stopped: &str,
    took: std::time::Duration,
) {
    let now = chrono::Utc::now().timestamp_millis();
    let mut md = format!(
        "# Topic steps review\nLocal model {model}. {done} sources read in {took:?}; run ended: {stopped}. Names: {}\n",
        store.self_names().unwrap().join(", ")
    );
    let (mut with_owe, mut seeds) = (0, 0);
    for e in store.topic_page_list(now).unwrap() {
        let Some(p) = store.topic_page(e.topic.topic_id, now).unwrap() else {
            continue;
        };
        seeds += 1;
        with_owe += !p.you_owe.is_empty() as usize;
        writeln!(
            md,
            "\n## {}{}",
            p.topic.name,
            p.kind.map_or(String::new(), |k| format!(" ({k})"))
        )
        .unwrap();
        writeln!(md, "Your role: {}", p.role.text).unwrap();
        match &p.lead {
            Some(l) => writeln!(md, "Lead: {l:?}").unwrap(),
            None => writeln!(md, "Lead: none").unwrap(),
        }
        match &p.next_meeting {
            Some(m) => {
                writeln!(md, "Next meeting: {} on {}", m.title, day(m.start_unix_ms)).unwrap()
            }
            None => writeln!(md, "Next meeting: none").unwrap(),
        }
        for (head, steps) in [
            ("You owe", &p.you_owe),
            ("Owed to you", &p.owed_to_you),
            ("Done", &p.done),
            ("Older", &p.older),
        ] {
            writeln!(md, "{head}: {}", steps.len()).unwrap();
            for s in steps {
                let due = s
                    .due_date
                    .as_deref()
                    .map_or(String::new(), |d| format!(" (due {d})"));
                writeln!(
                    md,
                    "- [{}] {}{due}\n  from {}: {}",
                    s.status, s.text, s.source_kind, s.source_title
                )
                .unwrap();
            }
        }
        writeln!(md, "Recent: {} items", p.recent_total).unwrap();
    }
    writeln!(md, "\n{with_owe} of {seeds} seeds show a \"You owe\" step.").unwrap();
    let path = db.with_file_name("topic-steps-review.md");
    std::fs::write(&path, md).unwrap();
    eprintln!(
        "wrote {} ({with_owe}/{seeds} seeds with You owe)",
        path.display()
    );
}
