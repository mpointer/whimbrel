//! A labelled evaluation set for Find, with the real model Find uses: a made-up library (four meetings and three notes about a kite
//! festival, a garden club, a bake sale and a design sync, plus sixty one-line "Kite log N" notes) and about forty queries, each with the
//! rows it must find and the rows it may find. Every other row it returns is a wrong hit. Run with
//! `cargo test -p memory --release find_eval -- --ignored --nocapture`; `WHIMBREL_EMBEDDING_MODELS_DIR` points it at the model folder.
//!
//! Precision is the share of returned rows that are wanted or allowed; recall the share of wanted rows returned. Both are counted over
//! the whole answer (up to 200 rows, best first) and over its first 10 rows, by query group.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::*;
use crate::cards::tests::{capture, meeting};

/// The library, by label: `k0`.. for the kite festival meeting's lines, `g0`.. the garden club's, `b0`.. the bake sale's, `d0`.. the
/// design sync's, `n1`..`n3` the three notes and `log1`..`log60` the kite logs.
pub(super) struct Library {
    pub _dir: tempfile::TempDir,
    pub store: Store,
    pub ids: HashMap<String, i64>,
}

impl Library {
    pub fn id(&self, label: &str) -> i64 {
        *self.ids.get(label).unwrap_or_else(|| panic!("no row labelled {label}"))
    }
    pub fn label(&self, id: i64) -> String {
        self.ids.iter().find(|(_, v)| **v == id).map(|(k, _)| k.clone()).unwrap_or_else(|| format!("#{id}"))
    }
    /// The labels a label pattern names: `logs` is every kite log, anything else itself.
    fn expand(&self, labels: &[&str]) -> HashSet<i64> {
        let mut out = HashSet::new();
        for l in labels {
            if *l == "logs" {
                out.extend((1..=60).map(|n| self.id(&format!("log{n}"))));
            } else {
                out.insert(self.id(l));
            }
        }
        out
    }
}

pub(super) const KITE: [&str; 7] = [
    "The shuttle from the train station runs every twenty minutes on Saturday",
    "I called the bus company and they can add a second coach at noon",
    "Visitors who drive can park in the school lot on Elm Street",
    "The wind is steadier in the morning, so the big kites go up before lunch",
    "If it rains we move the workshop into the gym",
    "We need forty folding tables and a hundred chairs for the workshop tent",
    "The path to the field has to be wide enough for wheelchairs",
];
pub(super) const GARDEN: [&str; 4] = [
    "The plant sale raised four hundred dollars for new seed trays",
    "Rain barrels are half price at the hardware store this week",
    "We should sow native wildflowers in the meadow by the creek",
    "Membership dues stay at ten dollars a year",
];
pub(super) const BAKE: [&str; 4] = [
    "We need six more volunteers to staff the bake sale on Friday",
    "Cookies are a dollar each and whole pies are twelve dollars",
    "All the proceeds go to the school library fund",
    "Please label anything that contains nuts",
];
pub(super) const DESIGN: [&str; 4] = [
    "The sidebar redesign moves the search box to the top",
    "Screen reader users could not reach the settings button",
    "VoiceOver should read each list row with its full name",
    "We raised the text contrast on the dark theme",
];

/// The model folder: `WHIMBREL_EMBEDDING_MODELS_DIR`, else `models/<EMBEDDING_MODEL_ID>/` under this crate.
fn models_dir() -> std::path::PathBuf {
    std::env::var_os("WHIMBREL_EMBEDDING_MODELS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| crate::embedding::embedding_model_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("models")))
}

/// Builds the library with the real embedding model.
pub(super) fn library() -> Library {
    let s_embedder = std::sync::Arc::new(crate::embedding::LazyModelEmbedder::new(models_dir()));
    assert!(s_embedder.is_loaded(), "the embedding model is not there");
    library_with(s_embedder, crate::embedding::EMBEDDING_OUTPUT_DIM)
}

/// Builds the library with `embedder`, whose vectors are `dim` wide (the vector tables are made that wide first).
pub(super) fn library_with(embedder: std::sync::Arc<dyn crate::Embedder>, dim: usize) -> Library {
    let dir = tempfile::tempdir().unwrap();
    let s = crate::test_store::open_with_embedder(&dir.path().join("eval.db"), embedder);
    if dim != crate::embedding::EMBEDDING_OUTPUT_DIM {
        s.conn
            .execute_batch(&format!(
                "DROP TABLE capture_item_vec; DROP TABLE transcript_segment_vec; DROP TABLE items_vec;
                 CREATE VIRTUAL TABLE capture_item_vec USING vec0(embedding float[{dim}]);
                 CREATE VIRTUAL TABLE transcript_segment_vec USING vec0(embedding float[{dim}]);
                 CREATE VIRTUAL TABLE items_vec USING vec0(embedding float[{dim}]);"
            ))
            .unwrap();
    }
    let mut ids: HashMap<String, i64> = HashMap::new();
    let lines = |m: i64| -> Vec<i64> {
        let mut st = s.conn.prepare("SELECT id FROM items WHERE kind = 'transcript' AND thread_id = ?1 ORDER BY id").unwrap();
        st.query_map([m], |r| r.get(0)).unwrap().collect::<rusqlite::Result<Vec<i64>>>().unwrap()
    };
    for (prefix, start, title, people, text) in [
        ("k", 10_000_000, "Kite festival planning", ["Ana", "Leo"], &KITE[..]),
        ("g", 11_000_000, "Garden club monthly meeting", ["Priya", "Sam"], &GARDEN[..]),
        ("b", 12_000_000, "Bake sale volunteers", ["Ana", "Priya"], &BAKE[..]),
        ("d", 13_000_000, "Design sync", ["Sam", "Leo"], &DESIGN[..]),
    ] {
        let m = meeting(&s, start, title, &people, text);
        for (i, id) in lines(m).into_iter().enumerate() {
            ids.insert(format!("{prefix}{i}"), id);
        }
    }
    // A note's item, by its time (the helper returns the capture's own id).
    let note = |n: i64, ts: i64, title: &str, text: &str| -> i64 {
        capture(&s, n, ts, "com.notes", title, text);
        s.conn.query_row("SELECT id FROM items WHERE kind = 'capture' AND ts_unix_ms = ?1", [ts], |r| r.get(0)).unwrap()
    };
    ids.insert(
        "n1".into(),
        note(1, 14_000_000, "Kite festival day plan", "Kite festival on Saturday at Riverside Park. Gates open at nine. Shuttle buses leave the train station every twenty minutes. Overflow parking at the church lot."),
    );
    ids.insert(
        "n2".into(),
        note(2, 15_000_000, "Plant sale prices", "Plant sale prices: tomato seedlings 3 dollars, herbs 2 dollars, hanging baskets 15 dollars. Every dollar goes to the community garden."),
    );
    ids.insert(
        "n3".into(),
        note(3, 16_000_000, "Bake sale checklist", "Bake sale checklist: borrow two long tables and a cash box, bring change in small bills, print the price cards."),
    );
    for n in 1..=60 {
        ids.insert(format!("log{n}"), note(100 + n, 20_000_000 + n * 1_000, &format!("Kite log {n}"), &format!("Kite log {n}. Flew for {n} minutes.")));
    }
    // Every row has its vector: a model whose width the tables do not take would leave Find to the keywords alone.
    let vectors: i64 = s.conn.query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get(0)).unwrap();
    assert_eq!(vectors as usize, ids.len(), "rows without a vector");
    Library { _dir: dir, store: s, ids }
}

/// One labelled query: the rows it must find and the rows it may find. Anything else it returns is wrong.
pub(super) struct Case {
    pub group: &'static str,
    pub query: &'static str,
    pub want: &'static [&'static str],
    pub ok: &'static [&'static str],
}

const fn case(group: &'static str, query: &'static str, want: &'static [&'static str], ok: &'static [&'static str]) -> Case {
    Case { group, query, want, ok }
}

/// Every line of the kite festival meeting and its note, as "may find".
const FESTIVAL: &[&str] = &["k0", "k1", "k2", "k3", "k4", "k5", "k6", "n1"];
const BAKE_ALL: &[&str] = &["b0", "b1", "b2", "b3", "n3"];
const DESIGN_ALL: &[&str] = &["d0", "d1", "d2", "d3"];

pub(super) const CASES: &[Case] = &[
    // One topic word, mostly with no word in common with the rows it should find.
    case("word", "fundraising", &["g0", "b2", "n2"], &["b0", "b1", "g3", "n3"]),
    case("word", "money", &["g0", "g3", "b1", "n2"], &["b2", "n3", "g1"]),
    case("word", "weather", &["k3", "k4"], &["n1", "g1"]),
    case("word", "transportation", &["k0", "k1", "n1"], &["k2"]),
    case("word", "accessibility", &["d1", "d2", "k6"], &["d3"]),
    case("word", "parking", &["k2", "n1"], &["k0", "k1"]),
    case("word", "furniture", &["k5"], &["n3"]),
    case("word", "shuttle", &["k0", "n1"], &["k1"]),
    case("word", "dollars", &["g0", "g3", "b1", "n2"], &["b2", "n3"]),
    case("word", "wheelchairs", &["k6"], &["d1", "d2"]),
    case("word", "VoiceOver", &["d2"], &["d1", "d3"]),
    case("word", "volunteers", &["b0"], &["b2", "n3"]),
    case("word", "nuts", &["b3"], &["b1"]),
    // Phrases.
    case("phrase", "screen reader", &["d1", "d2"], &["d3", "k6"]),
    case("phrase", "bus company", &["k1"], &["k0", "n1"]),
    case("phrase", "plant sale", &["g0", "n2"], &["g1", "b0", "b1", "b2", "n3"]),
    case("phrase", "bake sale", &["b0", "n3"], &["b1", "b2", "b3", "g0", "n2"]),
    case("phrase", "rain barrels", &["g1"], &["k4"]),
    case("phrase", "sidebar redesign", &["d0"], &["d1", "d2", "d3"]),
    case("phrase", "folding tables and chairs", &["k5"], &["n3"]),
    case("phrase", "train station", &["k0", "n1"], &["k1"]),
    case("phrase", "dark theme contrast", &["d3"], &["d1", "d2"]),
    case("phrase", "\"bake sale\"", &["b0", "n3"], &["b1", "b2", "b3", "g0", "n2"]),
    // Questions full of common words.
    case("question", "accessibility for blind users", &["d1", "d2"], &["d3", "k6"]),
    case("question", "Where is the kite festival?", &["n1"], &["k0", "k1", "k2", "k3", "k4", "k5", "k6", "logs"]),
    case("question", "What did we decide about parking?", &["k2", "n1"], &["k0", "k1"]),
    case("question", "How much are the cookies?", &["b1"], &["b0", "b2", "n3", "n2", "g3"]),
    case("question", "When does the shuttle leave?", &["k0", "n1"], &["k1"]),
    case("question", "Who is running the bake sale?", &["b0"], BAKE_ALL),
    case("question", "what is the price of a pie", &["b1"], &["b0", "b2", "n2", "n3", "g3"]),
    case("question", "is it going to rain", &["k4"], &["k3", "g1"]),
    case("question", "how do people get to the festival", &["k0", "k1", "n1"], &["k2", "k3", "k4", "k5", "k6"]),
    case("question", "the", &[], &[]),
    case("question", "for", &[], &[]),
    // Other languages the app ships, with their own common words.
    case("language", "la venta de pasteles", &["b0", "n3"], &["b1", "b2", "b3", "g0", "n2"]),
    case("language", "le transport pour le festival", &["k0", "k1", "n1"], FESTIVAL),
    case("language", "wo kann man parken", &["k2", "n1"], &["k0", "k1"]),
    case("language", "o dinheiro da venda", &["g0", "b1", "n2"], &["g3", "b0", "b2", "n3"]),
    // Nonsense finds nothing.
    case("nonsense", "zebra quantum harmonica", &[], &[]),
    case("nonsense", "xyzzy plover", &[], &[]),
    case("nonsense", "xyzzy", &[], &[]),
    case("nonsense", "quantum", &[], &[]),
    case("nonsense", "plover", &[], &[]),
    case("nonsense", "asdf qwerty", &[], &[]),
    // Sixty kite logs that all say "for".
    case("flood", "flew for minutes", &["logs"], FESTIVAL),
    case("flood", "kite log 42", &["log42"], &["logs", "n1", "k3"]),
    case("flood", "accessibility for the design sync", &["d1", "d2"], DESIGN_ALL),
];

/// One query's precision and recall over its whole answer and over its first 10 rows. A query that returns nothing has precision 1;
/// one that wants nothing has no recall (nonsense is judged by precision alone: any row it returns is wrong).
#[derive(Default, Clone, Copy)]
pub(super) struct Score {
    pub found: usize,
    pub precision: f64,
    pub recall: Option<f64>,
    pub precision10: f64,
    pub recall10: Option<f64>,
    /// Average precision of the ranked answer against the wanted rows: 1 when every wanted row comes before anything else.
    pub ap: Option<f64>,
}

/// Scores by group, averaged over its queries (each query weighs the same, so the sixty kite logs count as one query).
#[derive(Default, Clone)]
pub(super) struct Tally {
    pub queries: usize,
    p: f64,
    p10: f64,
    r: Vec<f64>,
    r10: Vec<f64>,
    ap: Vec<f64>,
}

impl Tally {
    fn add(&mut self, s: Score) {
        self.queries += 1;
        self.p += s.precision;
        self.p10 += s.precision10;
        self.r.extend(s.recall);
        self.r10.extend(s.recall10);
        self.ap.extend(s.ap);
    }
    fn mean(v: &[f64]) -> f64 {
        if v.is_empty() { 1.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
    }
    pub fn precision(&self) -> f64 {
        self.p / self.queries.max(1) as f64
    }
    pub fn recall(&self) -> f64 {
        Self::mean(&self.r)
    }
    pub fn precision10(&self) -> f64 {
        self.p10 / self.queries.max(1) as f64
    }
    pub fn recall10(&self) -> f64 {
        Self::mean(&self.r10)
    }
    pub fn map(&self) -> f64 {
        Self::mean(&self.ap)
    }
}

/// One query's answer, best first, with its tally and the wanted rows it missed and the wrong rows it returned.
pub(super) struct Outcome {
    pub score: Score,
    pub missed: Vec<String>,
    pub wrong: Vec<String>,
}

pub(super) fn judge(lib: &Library, c: &Case) -> Outcome {
    let page = lib.store.find(c.query, &FindFilter::default(), FindOrder::Rank, 0, FIND_MAX_LIMIT).unwrap();
    let found: Vec<i64> = page.results.iter().map(|r| r.id).collect();
    let want = lib.expand(c.want);
    let mut good = want.clone();
    good.extend(lib.expand(c.ok));
    let top: Vec<i64> = found.iter().take(10).copied().collect();
    let share = |n: usize, of: usize| if of == 0 { 1.0 } else { n as f64 / of as f64 };
    let got = found.iter().filter(|id| want.contains(id)).count();
    let got10 = top.iter().filter(|id| want.contains(id)).count();
    // Recall in the first 10 counts against at most 10 wanted rows (the flood case wants 60).
    let score = Score {
        found: found.len(),
        precision: share(found.iter().filter(|id| good.contains(id)).count(), found.len()),
        recall: (!want.is_empty()).then(|| share(got, want.len())),
        precision10: share(top.iter().filter(|id| good.contains(id)).count(), top.len()),
        recall10: (!want.is_empty()).then(|| share(got10, want.len().min(10))),
        ap: (!want.is_empty()).then(|| {
            let mut hits = 0;
            let sum: f64 = found.iter().enumerate().filter(|(_, id)| want.contains(id)).map(|(i, _)| { hits += 1; hits as f64 / (i + 1) as f64 }).sum();
            sum / want.len() as f64
        }),
    };
    let mut missed: Vec<String> = want.iter().filter(|id| !found.contains(id)).map(|id| lib.label(*id)).collect();
    missed.sort();
    let wrong: Vec<String> = found.iter().filter(|id| !good.contains(id)).map(|id| lib.label(*id)).collect();
    Outcome { score, missed, wrong }
}

/// Runs every case, prints each query's result and a table by group, and returns the tallies by group (with "all").
pub(super) fn run_all(lib: &Library) -> (BTreeMap<&'static str, Tally>, Vec<(&'static Case, Outcome)>) {
    let mut by: BTreeMap<&'static str, Tally> = BTreeMap::new();
    let mut outcomes = Vec::new();
    for c in CASES {
        let o = judge(lib, c);
        let shown = |v: &[String]| if v.len() > 8 { format!("{} (+{} more)", v[..8].join(" "), v.len() - 8) } else { v.join(" ") };
        eprintln!(
            "{:9} {:38} found {:3} | missed [{}] | wrong [{}]",
            c.group,
            c.query,
            o.score.found,
            shown(&o.missed),
            shown(&o.wrong)
        );
        by.entry(c.group).or_default().add(o.score);
        by.entry("all").or_default().add(o.score);
        outcomes.push((c, o));
    }
    eprintln!("\n| group | queries | precision | recall | precision@10 | recall@10 | MAP |\n|---|---|---|---|---|---|---|");
    for (g, t) in &by {
        let r = |v: f64| if t.r.is_empty() { "-".to_string() } else { format!("{v:.2}") };
        eprintln!("| {g} | {} | {:.2} | {} | {:.2} | {} | {} |", t.queries, t.precision(), r(t.recall()), t.precision10(), r(t.recall10()), r(t.map()));
    }
    (by, outcomes)
}

/// Every row's cosine to `query`, best first, with its label.
pub(super) fn scores(lib: &Library, query: &str) -> Vec<(f32, String)> {
    let q = lib.store.embed_query(query).unwrap();
    let mut st = lib.store.conn.prepare("SELECT rowid, embedding FROM items_vec").unwrap();
    let mut out: Vec<(f32, String)> = st
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
        .unwrap()
        .map(|r| {
            let (id, blob) = r.unwrap();
            let v: Vec<f32> = blob.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            let dot: f32 = v.iter().zip(&q).map(|(a, b)| a * b).sum();
            let n = |x: &[f32]| x.iter().map(|y| y * y).sum::<f32>().sqrt();
            (dot / (n(&v) * n(&q)), lib.label(id))
        })
        .collect();
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    out
}

/// The labelled set, with the real model: prints every query and the precision and recall by group, and holds Find to the bar the
/// set was tuned to.
#[test]
#[ignore] // needs real model files
fn find_eval_labelled_set() {
    let lib = library();
    let (by, outcomes) = run_all(&lib);
    let noisy: Vec<&str> = outcomes.iter().filter(|(c, o)| c.group == "nonsense" && o.score.found > 0).map(|(c, _)| c.query).collect();
    assert!(noisy.is_empty(), "nonsense found something: {noisy:?}");
    let empty: Vec<&str> = outcomes.iter().filter(|(c, o)| ["the", "for"].contains(&c.query) && o.score.found > 0).map(|(c, _)| c.query).collect();
    assert!(empty.is_empty(), "common words only found something: {empty:?}");
    // The wrong hits the real-PC check saw stay out.
    let wrong = |q: &str| outcomes.iter().find(|(c, _)| c.query == q).map(|(_, o)| o.wrong.clone()).unwrap();
    assert!(wrong("accessibility for blind users").is_empty(), "{:?}", wrong("accessibility for blind users"));
    for (q, not) in [("parking", "g1"), ("furniture", "d0")] {
        assert!(!wrong(q).iter().any(|l| l == not), "{q} found {not}");
    }
    // Tuned to 0.93 and 0.76 (0.73 and 0.79 before the change, when common words found rows by keyword); the bar leaves room for a
    // model build that rounds differently.
    let all = &by["all"];
    assert!(all.precision() >= 0.9, "precision {:.2}", all.precision());
    assert!(all.recall() >= 0.72, "recall {:.2}", all.recall());
}

/// Prints the cosines of each query's best rows (`*` a wanted row, `-` a wrong one), to judge a floor by.
#[test]
#[ignore] // needs real model files
fn find_eval_print_scores() {
    let lib = library();
    for c in CASES {
        let want = lib.expand(c.want);
        let mut good = want.clone();
        good.extend(lib.expand(c.ok));
        let top: Vec<String> = scores(&lib, c.query)
            .iter()
            .take(8)
            .map(|(s, l)| {
                let id = lib.id(l);
                format!("{s:.3}{}{l}", if want.contains(&id) { "*" } else if good.contains(&id) { " " } else { "-" })
            })
            .collect();
        eprintln!("{:38} {}", c.query, top.join(" "));
    }
}

/// Writes every query's rows (label, cosine, whether a word of it matched by keyword, wanted, allowed) as JSON lines to
/// `FIND_EVAL_DUMP`, to try floor rules offline.
#[test]
#[ignore] // needs real model files
fn find_eval_dump_rows() {
    let Some(path) = std::env::var_os("FIND_EVAL_DUMP") else { return };
    let lib = library();
    let mut out = String::new();
    for c in CASES {
        let want = lib.expand(c.want);
        let ok = lib.expand(c.ok);
        let words = keyword_words(c.query);
        let matched: HashSet<i64> = if words.is_empty() {
            HashSet::new()
        } else {
            let m = sanitize_fts5_query(&words.join(" "));
            let mut st = lib.store.conn.prepare("SELECT rowid FROM items_fts WHERE items_fts MATCH ?1").unwrap();
            st.query_map([m], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        let rows: Vec<serde_json::Value> = scores(&lib, c.query)
            .into_iter()
            .map(|(cos, l)| {
                let id = lib.id(&l);
                serde_json::json!({"label": l, "cos": cos, "kw": matched.contains(&id), "want": want.contains(&id), "ok": ok.contains(&id)})
            })
            .collect();
        out.push_str(&serde_json::json!({"group": c.group, "query": c.query, "words": words.len(), "spaces": c.query.split_whitespace().count(), "rows": rows}).to_string());
        out.push('\n');
    }
    std::fs::write(path, out).unwrap();
}

/// The floor at library size: the labelled library plus 3,000 chunks of this repository's docs (about 100 words each), with each
/// hard query's best wanted row and best other row, by cosine and by how far above the query's mean it is (in standard deviations).
/// Prints only: it is what says whether a floor tuned on the small library holds as a library grows. Runs when `FIND_EVAL_SCALE` is
/// set (it takes minutes and asserts nothing).
#[test]
#[ignore] // needs real model files; slow (embeds 3,000 chunks)
fn find_eval_floor_at_library_size() {
    if std::env::var_os("FIND_EVAL_SCALE").is_none() {
        return;
    }
    let lib = library();
    let picked: Vec<Vec<f32>> = doc_chunks(100, 3000).iter().map(|c| lib.store.embed_passage(c).unwrap()).collect();
    let cosine = |a: &[f32], b: &[f32]| {
        let n = |x: &[f32]| x.iter().map(|y| y * y).sum::<f32>().sqrt();
        a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / (n(a) * n(b))
    };
    let queries = ["fundraising", "money", "weather", "transportation", "accessibility", "parking", "furniture", "plover", "xyzzy", "quantum", "zebra", "harmonica", "xyzzy plover", "zebra quantum harmonica", "asdf qwerty", "the", "for"];
    eprintln!("| query | best wanted | its z | best other (docs) | its z | best other (library) |\n|---|---|---|---|---|---|");
    for q in queries {
        let v = lib.store.embed_query(q).unwrap();
        let lib_rows = scores(&lib, q);
        let doc: Vec<f32> = picked.iter().map(|d| cosine(&v, d)).collect();
        let all: Vec<f32> = lib_rows.iter().map(|(c, _)| *c).chain(doc.iter().copied()).collect();
        let mean = all.iter().sum::<f32>() / all.len() as f32;
        let sd = (all.iter().map(|c| (c - mean).powi(2)).sum::<f32>() / all.len() as f32).sqrt();
        let z = |c: f32| (c - mean) / sd;
        let want: HashSet<String> = CASES.iter().find(|c| c.query == q).map(|c| c.want.iter().map(|s| s.to_string()).collect()).unwrap_or_default();
        let best_want = lib_rows.iter().find(|(_, l)| want.contains(l)).map(|(c, _)| *c);
        let best_doc = doc.iter().copied().fold(f32::MIN, f32::max);
        let best_lib_other = lib_rows.iter().find(|(_, l)| !want.contains(l)).map(|(c, _)| *c).unwrap_or(0.0);
        let w = best_want.map_or("-".to_string(), |c| format!("{c:.3} | {:.1}", z(c)));
        let w = if best_want.is_none() { "- | -".to_string() } else { w };
        eprintln!("| {q} | {w} | {best_doc:.3} | {:.1} | {best_lib_other:.3} |", z(best_doc));
    }
}

/// The model evaluation's results, which quote every nonsense query: left out of the docs corpus so they do not score against
/// themselves.
pub(super) const EVAL_RESULTS_DOC: &str = "2026-11-22-find-embedding-model-eval.md";

/// `n` chunks of about `words` words each of this repository's docs (every Markdown file under `docs/`), spread across the whole
/// corpus: the stand-in for a large library that #977's floor check used (100 words, 3,000 chunks).
pub(super) fn doc_chunks(words: usize, n: usize) -> Vec<String> {
    let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let mut files = Vec::new();
    let mut stack = vec![docs];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") && !p.ends_with(EVAL_RESULTS_DOC) {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut chunks: Vec<String> = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let all: Vec<&str> = text.split_whitespace().collect();
        chunks.extend(all.chunks(words).map(|c| c.join(" ")));
    }
    let step = (chunks.len() / n).max(1);
    chunks.into_iter().step_by(step).take(n).collect()
}

#[path = "find_eval_models.rs"]
mod models;
