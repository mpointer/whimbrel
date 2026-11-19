//! Spike 2 performance harness: what does SQLCipher cost? (X4 encryption at rest, design section 4.)
//!
//! Needs the `bench-hooks` feature (the pragma hooks do not exist in a product build).
//! Build it twice and run both back to back:
//!   plain:     cargo bench -p memory --bench encryption_cost --features bench-hooks --no-run
//!   SQLCipher: cargo bench -p memory --bench encryption_cost --features sqlcipher,bench-hooks --no-run
//! (use a separate CARGO_TARGET_DIR per build so they do not rebuild each other), then run the
//! printed binaries directly. Nothing here runs under `cargo test`; it is a bench target with
//! `harness = false`, and every mode needs explicit arguments.
//!
//! Modes (the first argument):
//!   gen <mult> <plain_out>                       synthetic eval corpus, `full` profile x mult, fake embedder
//!   queries <plain_db> <out.tsv> [n]             query sidecar drawn from words/text that exist in the corpus
//!   export <plain_db> <enc_out> <keyfile>        (sqlcipher build) fresh random key + sqlcipher_export copy
//!   run <db> <label> <queries.tsv> [flags]       time one database with this build
//!        --keyfile F      database is encrypted, key hex in F (sqlcipher build only)
//!        --models DIR     also run the search timings with the real embedder from DIR
//!        --open-reps N    cold open reps (default 20)
//!        --burst on|off|default   transcript write burst on a work copy (secure_delete setting)
//!        --reembed        full re-embed (cursor reset to 0, reindex_batch loop) on a work copy
//!        --work-dir D     where work copies go (default: next to the db)
//!        --skip-search
//!        --pragma "PRAGMA ...;"   (repeatable) run after the key and before the first read (keyed),
//!                         or right after open (plain / unkeyed). Spike 2b tuning variants.
//!        --nq N           use only the first N queries of each kind (20 of them warm-up)
//!        --warm-pass      one untimed pass over every query before timing (spike 2b)
//!   export writes HMAC_SHA256 (memory::KEYED_HMAC_ALGORITHM) unless --hmac-algorithm ALG overrides it
//!   (spike variants only; the reopen applies the same override).
//!   export takes optional --page-size N (cipher_page_size of the new file) and --no-hmac
//!   (cipher_use_hmac OFF, NOT RECOMMENDED, information only). Opening such a file needs the matching --pragma.
//!
//! Output is `RESULT key=value ...` lines; the results doc is built from them.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use memory::{
    DbKey, Embedder, EmbeddingError, MeetingStatus, MeetingTrigger, NewMeeting,
    NewTranscriptSegment, Store, EMBEDDING_OUTPUT_DIM,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

struct HashEmbedder;
impl Embedder for HashEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(text, &mut h);
        let seed = std::hash::Hasher::finish(&h);
        Ok((0..EMBEDDING_OUTPUT_DIM)
            .map(|i| ((seed.wrapping_add(i as u64) % 1000) as f32) / 1000.0)
            .collect())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Write a file that is mode 0600 from the moment it is created (keys, real-text query files).
fn write_private(path: &Path, data: &[u8]) {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path).expect("create private file");
    f.write_all(data).expect("write private file");
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Nearest-rank percentile of an already sorted slice.
fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn report(label: &str, metric: &str, mut v: Vec<f64>, extra: &str) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "RESULT label={label} metric={metric} n={} p50_ms={:.3} p95_ms={:.3} max_ms={:.3} mean_ms={:.3} {extra}",
        v.len(),
        pct(&v, 50.0),
        pct(&v, 95.0),
        v.last().copied().unwrap_or(f64::NAN),
        v.iter().sum::<f64>() / v.len().max(1) as f64,
    );
}

fn load_key(path: &Path) -> DbKey {
    let hex_s = std::fs::read_to_string(path).expect("keyfile");
    let hex_s = hex_s.trim();
    let bytes = hex::decode(hex_s).expect("key hex");
    let arr: [u8; 32] = bytes.try_into().expect("32 byte key");
    DbKey::new(arr)
}

fn arg_val(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

static EXTRA: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
static WARM_PASS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn open_store(db: &Path, key: Option<&DbKey>, emb: Arc<dyn Embedder>) -> rusqlite::Result<Store> {
    let extra: &[String] = EXTRA.get().map(|v| v.as_slice()).unwrap_or(&[]);
    match key {
        Some(k) => Store::open_with_embedder_keyed_tuned(db, emb, k, extra),
        None => {
            let s = Store::open_with_embedder(db, emb)?;
            for p in extra {
                s.exec_batch(p)?;
            }
            Ok(s)
        }
    }
}

/// A raw connection that knows about vec0 and the key, for setup work outside `Store`.
fn raw_conn(db: &Path, key_hex: Option<&str>) -> rusqlite::Connection {
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }
    let c = rusqlite::Connection::open(db).expect("open raw");
    if let Some(k) = key_hex {
        c.execute_batch(&format!(
            "PRAGMA key = \"x'{k}'\"; PRAGMA cipher_page_size = {}; PRAGMA cipher_hmac_algorithm = {};",
            memory::KEYED_PAGE_SIZE,
            memory::KEYED_HMAC_ALGORITHM
        ))
        .expect("key raw");
    }
    c
}

fn version_line(db: &Path, key_hex: Option<&str>) {
    let c = raw_conn(db, key_hex);
    let cv: Option<String> = c
        .query_row("PRAGMA cipher_version", [], |r| r.get::<_, Option<String>>(0))
        .ok()
        .flatten();
    println!(
        "VERSION sqlite={} cipher_version={:?}",
        rusqlite::version(),
        cv.unwrap_or_default()
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--bench").collect();
    match args.first().map(String::as_str) {
        Some("gen") => gen(&args),
        Some("queries") => queries(&args),
        Some("export") => export(&args),
        Some("vacuum") => {
            // vacuum <in> <out>: compacted plain copy (VACUUM INTO), so plain and encrypted share one layout.
            let c = raw_conn(Path::new(&args[1]), None);
            let _ = std::fs::remove_file(&args[2]);
            let t = Instant::now();
            c.execute_batch(&format!("VACUUM INTO '{}';", args[2])).expect("vacuum into");
            println!("VACUUM {} -> {} in {:.1}s", args[1], args[2], t.elapsed().as_secs_f64());
        }
        Some("run") => run(&args),
        _ => eprintln!("modes: gen | queries | export | run (see the file header)"),
    }
}

fn gen(args: &[String]) {
    let mult: usize = args[1].parse().expect("mult");
    let out = PathBuf::from(&args[2]);
    let _ = std::fs::remove_file(&out);
    let mut p = eval::corpus::CorpusProfile::full();
    p.name = format!("full-x{mult}");
    p.meetings *= mult;
    p.emails *= mult;
    p.captures *= mult;
    p.events *= mult;
    // Planted facts stop at 250 per kind: the generator draws unique invented names from a pool
    // of 768 and loops forever once vendor/venue facts exceed it (x50 would hang). So x50
    // scales volume (rows), not the number of planted facts. Days stay at 30: denser, not longer.
    p.facts_per_kind = (p.facts_per_kind * mult).min(250);
    let store = Store::open_with_embedder(&out, Arc::new(HashEmbedder)).expect("open");
    let t = Instant::now();
    let c = eval::corpus::generate(&store, 7, &p, eval::corpus::default_now_unix_ms()).expect("generate");
    println!("GEN x{mult} in {:.1}s counts={:?}", t.elapsed().as_secs_f64(), c.counts);
    drop(store);
    let c = raw_conn(&out, None);
    let _ = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
}

fn queries(args: &[String]) {
    let db = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let n: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(300);
    let c = raw_conn(&db, None);
    let (min, max): (i64, i64) = c
        .query_row("SELECT MIN(id), MAX(id) FROM items WHERE kind='capture'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("items range");
    let mut rng = StdRng::seed_from_u64(42);
    let mut lines = Vec::new();
    let mut seen = std::collections::HashSet::new();
    while lines.len() < n * 2 {
        let id = rng.gen_range(min..=max);
        let text: Option<String> = c
            .query_row(
                "SELECT body_text FROM items WHERE kind='capture' AND id >= ?1 AND length(body_text) >= 40 ORDER BY id LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .ok();
        let Some(text) = text else { continue };
        let words: Vec<&str> = text
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|w| w.chars().count() >= 6)
            .collect();
        if words.is_empty() {
            continue;
        }
        let word = words[rng.gen_range(0..words.len())].to_string();
        let sem: String = text.chars().take(200).collect::<String>().replace(['\t', '\n', '\r'], " ");
        if !seen.insert(sem.clone()) {
            continue;
        }
        lines.push(format!("kw\t{word}"));
        lines.push(format!("sem\t{sem}"));
    }
    let _ = std::fs::remove_file(&out);
    write_private(&out, (lines.join("\n") + "\n").as_bytes());
    println!("wrote {} query lines to {}", lines.len(), out.display());
}

fn export(args: &[String]) {
    #[cfg(not(feature = "sqlcipher"))]
    {
        let _ = args;
        eprintln!("export needs --features sqlcipher");
        std::process::exit(2);
    }
    #[cfg(feature = "sqlcipher")]
    {
        let plain = PathBuf::from(&args[1]);
        let out = PathBuf::from(&args[2]);
        let keyfile = PathBuf::from(&args[3]);
        let _ = std::fs::remove_file(&out);
        let key: [u8; 32] = rand::random();
        let key_hex = hex::encode(key);
        let _ = std::fs::remove_file(&keyfile);
        write_private(&keyfile, format!("{key_hex}\n").as_bytes());
        let c = raw_conn(&plain, None);
        let uv: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        let t = Instant::now();
        c.execute_batch(&format!(
            "ATTACH DATABASE '{}' AS encrypted KEY \"x'{key_hex}'\";",
            out.display()
        ))
        .unwrap();
        if let Some(ps) = arg_val(args, "--page-size") {
            c.execute_batch(&format!("PRAGMA encrypted.cipher_page_size = {ps};")).unwrap();
        }
        // The shipped HMAC algorithm unless overridden (spike variants only).
        let hmac_alg = arg_val(args, "--hmac-algorithm").unwrap_or_else(|| memory::KEYED_HMAC_ALGORITHM.to_string());
        c.execute_batch(&format!("PRAGMA encrypted.cipher_hmac_algorithm = {hmac_alg};")).unwrap();
        if has(args, "--no-hmac") {
            c.execute_batch("PRAGMA encrypted.cipher_use_hmac = OFF;").unwrap();
        }
        c.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(())).unwrap();
        c.execute_batch(&format!("PRAGMA encrypted.user_version = {uv}; DETACH DATABASE encrypted;"))
            .unwrap();
        println!(
            "EXPORT {} -> {} in {:.1}s, user_version={uv}",
            plain.display(),
            out.display(),
            t.elapsed().as_secs_f64()
        );
        // Prove the copy opens with the key through the real keyed path (with any matching pragmas).
        let k = load_key(&keyfile);
        let mut extra = Vec::new();
        if let Some(ps) = arg_val(args, "--page-size") {
            extra.push(format!("PRAGMA cipher_page_size = {ps};"));
        }
        if let Some(alg) = arg_val(args, "--hmac-algorithm") {
            extra.push(format!("PRAGMA cipher_hmac_algorithm = {alg};"));
        }
        if has(args, "--no-hmac") {
            extra.push("PRAGMA cipher_use_hmac = OFF;".to_string());
        }
        let s = Store::open_with_embedder_keyed_tuned(&out, Arc::new(HashEmbedder), &k, &extra).unwrap();
        println!("EXPORT reopened keyed ok, secure_delete={}", s.secure_delete_enabled().unwrap());
    }
}

struct Q {
    kw: Vec<String>,
    sem: Vec<String>,
}

fn load_queries(p: &Path) -> Q {
    let text = std::fs::read_to_string(p).expect("queries");
    let mut q = Q { kw: vec![], sem: vec![] };
    for l in text.lines() {
        match l.split_once('\t') {
            Some(("kw", w)) => q.kw.push(w.to_string()),
            Some(("sem", w)) => q.sem.push(w.to_string()),
            _ => {}
        }
    }
    q
}

fn id_hash(ids: impl Iterator<Item = i64>, acc: &mut u64) {
    for id in ids {
        *acc = acc.wrapping_mul(1099511628211).wrapping_add(id as u64);
    }
}

fn time_searches(store: &Store, q: &Q, label: &str, tag: &str, embedder: &Arc<dyn Embedder>) {
    const WARM: usize = 20;
    // The real-embedder call re-times only semantic search (keyword does not use the embedder) and
    // reuses the cache the fake call already warmed.
    let real_call = tag == "_real";
    if WARM_PASS.load(std::sync::atomic::Ordering::Relaxed) && !real_call {
        for w in &q.kw {
            let _ = store.search_keyword(w, 10).unwrap();
        }
        // One semantic query scans the whole vector table, so a short pass warms it.
        for t in q.sem.iter().take(30) {
            let _ = store.search_semantic(t, 10).unwrap();
        }
    }
    // Keyword.
    let mut acc = 0u64;
    let mut hits = 0usize;
    for w in q.kw.iter().take(WARM) {
        if real_call {
            break;
        }
        let _ = store.search_keyword(w, 10).unwrap();
    }
    let mut v = Vec::new();
    for w in q.kw.iter().skip(WARM) {
        if real_call {
            break;
        }
        let t = Instant::now();
        let r = store.search_keyword(w, 10).unwrap();
        v.push(ms(t.elapsed()));
        hits += r.len();
        id_hash(r.iter().map(|i| i.id), &mut acc);
    }
    if !real_call {
        report(label, &format!("search_keyword{tag}"), v, &format!("hits={hits} idhash={acc:x}"));
    }

    // Semantic (includes embedding the query text) plus the embed alone.
    let mut acc = 0u64;
    let mut hits = 0usize;
    for s in q.sem.iter().take(WARM) {
        let _ = store.search_semantic(s, 10).unwrap();
    }
    let mut v = Vec::new();
    for s in q.sem.iter().skip(WARM) {
        let t = Instant::now();
        let r = store.search_semantic(s, 10).unwrap();
        v.push(ms(t.elapsed()));
        hits += r.len();
        id_hash(r.iter().map(|i| i.id), &mut acc);
    }
    report(label, &format!("search_semantic{tag}"), v, &format!("hits={hits} idhash={acc:x}"));
    let mut v = Vec::new();
    for s in q.sem.iter().skip(WARM) {
        let t = Instant::now();
        let _ = embedder.embed(s).unwrap();
        v.push(ms(t.elapsed()));
    }
    report(label, &format!("embed_only{tag}"), v, "");
}

fn copy_db(db: &Path, work_dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(work_dir).unwrap();
    let dst = work_dir.join(name);
    for sfx in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{sfx}", dst.display()));
    }
    std::fs::copy(db, &dst).expect("copy db");
    dst
}

fn remove_db(p: &Path) {
    for sfx in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{sfx}", p.display()));
    }
}

fn run(args: &[String]) {
    let db = PathBuf::from(&args[1]);
    let label = args[2].clone();
    let queries_path = PathBuf::from(&args[3]);
    let key = arg_val(args, "--keyfile").map(|f| load_key(Path::new(&f)));
    let key_hex = arg_val(args, "--keyfile").map(|f| std::fs::read_to_string(f).unwrap().trim().to_string());
    let work_dir = arg_val(args, "--work-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| db.parent().unwrap().to_path_buf());
    let reps: usize = arg_val(args, "--open-reps").map(|s| s.parse().unwrap()).unwrap_or(20);
    let pragmas: Vec<String> = args
        .windows(2)
        .filter(|w| w[0] == "--pragma")
        .map(|w| w[1].clone())
        .collect();
    println!("PRAGMAS {pragmas:?}");
    let _ = EXTRA.set(pragmas);
    WARM_PASS.store(has(args, "--warm-pass"), std::sync::atomic::Ordering::Relaxed);
    let md = std::fs::metadata(&db).unwrap();
    println!("RUN label={label} db={} size_mib={} keyed={}", db.display(), md.len() >> 20, key.is_some());
    version_line(&db, key_hex.as_deref());

    // 1. Open plus init, a fresh connection every rep. The OS page cache stays warm (purge needs
    //    root); this is a cold connection, not a cold disk.
    // One untimed open first, so a one-time migration or first-touch cost is not charged to
    // whichever build happens to open the file first.
    drop(open_store(&db, key.as_ref(), Arc::new(HashEmbedder)).unwrap());
    let mut v = Vec::new();
    for _ in 0..reps {
        let t = Instant::now();
        let s = open_store(&db, key.as_ref(), Arc::new(HashEmbedder)).unwrap();
        v.push(ms(t.elapsed()));
        drop(s);
    }
    report(&label, "open_init", v, "");

    let mut q = load_queries(&queries_path);
    if let Some(n) = arg_val(args, "--nq") {
        let n: usize = n.parse().unwrap();
        q.kw.truncate(n);
        q.sem.truncate(n);
    }
    if !has(args, "--skip-search") {
        if !has(args, "--only-real") {
            let fake: Arc<dyn Embedder> = Arc::new(HashEmbedder);
            let store = open_store(&db, key.as_ref(), fake.clone()).unwrap();
            time_searches(&store, &q, &label, "_fake", &fake);
            drop(store);
        }
        if let Some(models) = arg_val(args, "--models") {
            // LazyModelEmbedder loads the model from a copy of the models folder; open_store applies
            // the key and any --pragma, so tuned (page size) files work too and nothing is pruned.
            let models = PathBuf::from(models);
            let emb: Arc<dyn Embedder> =
                Arc::new(memory::LazyModelEmbedder::new(memory::embedding_model_dir(&models)));
            let store = open_store(&db, key.as_ref(), emb.clone()).unwrap();
            if has(args, "--only-real") {
                // Warm the cache with a few semantic queries (one scans the whole vector table).
                for t in q.sem.iter().take(30) {
                    let _ = store.search_semantic(t, 10).unwrap();
                }
            }
            time_searches(&store, &q, &label, "_real", &emb);
        }
    }

    if let Some(mode) = arg_val(args, "--burst") {
        burst(&db, &work_dir, &label, key.as_ref(), &mode);
    }
    if has(args, "--reembed") {
        reembed(&db, &work_dir, &label, key.as_ref(), key_hex.as_deref());
    }
}

fn burst(db: &Path, work_dir: &Path, label: &str, key: Option<&DbKey>, mode: &str) {
    let work = copy_db(db, work_dir, &format!("burst-{label}.sqlite"));
    let store = open_store(&work, key, Arc::new(HashEmbedder)).unwrap();
    match mode {
        "on" => drop(store.set_secure_delete(true).unwrap()),
        "off" => drop(store.set_secure_delete(false).unwrap()),
        _ => {}
    }
    let sd = store.secure_delete_enabled().unwrap();
    let meeting = store
        .insert_meeting(&NewMeeting {
            device_id: "bench".into(),
            start_unix_ms: 1_700_000_000_000,
            end_unix_ms: None,
            title: "burst".into(),
            attendees: vec![],
            trigger: MeetingTrigger::Manual,
            audio_path: None,
            status: MeetingStatus::Recording,
        })
        .unwrap();
    // 1000 segments in batches of 10, the way live transcription flushes.
    let mut v = Vec::new();
    let total = Instant::now();
    for b in 0..100 {
        let segs: Vec<NewTranscriptSegment> = (0..10)
            .map(|i| NewTranscriptSegment {
                meeting_id: meeting,
                start_cs: (b * 10 + i) as i64 * 300,
                speaker_label: format!("S{}", i % 3),
                text: format!(
                    "burst segment {b}-{i} discussing quarterly planning, hiring and the zephyr{} rollout schedule",
                    b * 10 + i
                ),
            })
            .collect();
        let t = Instant::now();
        store.insert_transcript_segments(&segs).unwrap();
        v.push(ms(t.elapsed()));
    }
    let total_ms = ms(total.elapsed());
    report(label, "write_burst_batch10", v, &format!("secure_delete={sd} total_ms={total_ms:.1} segments=1000"));
    let t = Instant::now();
    let _ = store.delete_meeting(meeting).unwrap();
    println!(
        "RESULT label={label} metric=delete_meeting_1000_segments secure_delete={sd} ms={:.1}",
        ms(t.elapsed())
    );
    drop(store);
    remove_db(&work);
}

fn reembed(db: &Path, work_dir: &Path, label: &str, key: Option<&DbKey>, key_hex: Option<&str>) {
    let work = copy_db(db, work_dir, &format!("reembed-{label}.sqlite"));
    {
        let c = raw_conn(&work, key_hex);
        c.execute_batch(
            "INSERT INTO kv_meta (key, value) VALUES ('embedding_reindex_cursor', '0') \
             ON CONFLICT(key) DO UPDATE SET value = '0';",
        )
        .unwrap();
    }
    let store = open_store(&work, key, Arc::new(HashEmbedder)).unwrap();
    let t = Instant::now();
    let mut rows = 0usize;
    loop {
        let n = store.reindex_batch(200).unwrap();
        if n == 0 {
            break;
        }
        rows += n;
    }
    println!(
        "RESULT label={label} metric=full_reembed_fake rows={rows} total_ms={:.1} ms_per_row={:.4}",
        ms(t.elapsed()),
        ms(t.elapsed()) / rows.max(1) as f64
    );
    drop(store);
    remove_db(&work);
}
