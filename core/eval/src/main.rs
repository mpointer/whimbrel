//! `eval`: the harness CLI.
//!
//!   eval retrieval [--seed N] [--profile quick|full] [--real-model DIR] [--json OUT] [--baseline FILE] [--strict]
//!   eval baseline  [--seed N] [--profile quick|full] [--real-model DIR] --out FILE
//!   eval answers   [--seed N] [--profile quick|full] [--routing-json FILE] [--ollama-model NAME] [--json OUT]
//!   eval graded    (as answers) --grader-routing-json FILE
//!   eval routines  [--k N] [--seed N]
//!   eval preferences FILE
//!
//! Every command exits 0 unless the run itself failed; `--strict` makes a `worse`
//! retrieval verdict exit 1 (not used by CI this round).

use std::collections::HashMap;

use eval::corpus::{self, CorpusProfile, DEFAULT_SEED};
use eval::report::{self, RetrievalReport, RunInfo, Verdict};
use eval::retrieval::{self, Phrasing};
use eval::{answers, graded, open_store, preference, routines, sets, EmbedderChoice};

fn parse_args(args: &[String]) -> (String, HashMap<String, String>, Vec<String>) {
    let cmd = args.first().cloned().unwrap_or_else(|| "help".into());
    let mut flags = HashMap::new();
    let mut positional = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            let takes_value = !matches!(name, "strict" | "help");
            if takes_value {
                flags.insert(
                    name.to_string(),
                    args.get(i + 1).cloned().unwrap_or_default(),
                );
                i += 2;
            } else {
                flags.insert(name.to_string(), "true".into());
                i += 1;
            }
        } else {
            positional.push(a.clone());
            i += 1;
        }
    }
    (cmd, flags, positional)
}

fn embedder_choice(flags: &HashMap<String, String>) -> EmbedderChoice {
    match flags.get("real-model").map(String::as_str).or_else(|| {
        std::env::var("WHIMBREL_EVAL_REAL_MODEL_DIR")
            .ok()
            .as_deref()
            .map(|_| "env")
    }) {
        Some("env") => EmbedderChoice::Real(
            std::env::var("WHIMBREL_EVAL_REAL_MODEL_DIR")
                .unwrap_or_default()
                .into(),
        ),
        Some(dir) => EmbedderChoice::Real(dir.into()),
        None => EmbedderChoice::Fake,
    }
}

fn git_sha() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn seeded(
    flags: &HashMap<String, String>,
) -> Result<
    (
        memory::Store,
        Option<eval::tempdir::TempDir>,
        corpus::Corpus,
        Vec<sets::Question>,
        EmbedderChoice,
    ),
    String,
> {
    let seed = flags
        .get("seed")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SEED);
    let profile = CorpusProfile::named(flags.get("profile").map(String::as_str).unwrap_or("full"))
        .ok_or("profile must be quick or full")?;
    let choice = embedder_choice(flags);
    let (store, dir) = open_store(&choice).map_err(|e| e.to_string())?;
    let now = corpus::default_now_unix_ms();
    let corpus = corpus::generate(&store, seed, &profile, now).map_err(|e| e.to_string())?;
    let questions = sets::derive(&corpus);
    Ok((store, dir, corpus, questions, choice))
}

fn retrieval_report(flags: &HashMap<String, String>) -> Result<RetrievalReport, String> {
    let (store, _dir, corpus, questions, choice) = seeded(flags)?;
    let (natural, _) = retrieval::run(&store, &questions, corpus.now_unix_ms, Phrasing::Natural);
    let (terse, _) = retrieval::run(&store, &questions, corpus.now_unix_ms, Phrasing::Terse);
    Ok(RetrievalReport {
        run: RunInfo {
            seed: corpus.seed,
            profile: corpus.profile.name.clone(),
            embedder: choice.label(),
            now_unix_ms: corpus.now_unix_ms,
            git_sha: git_sha(),
            counts: corpus.counts.clone(),
        },
        natural,
        terse,
    })
}

fn write_json<T: serde::Serialize>(path: &str, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("{path}: {e}"))
}

fn run(cmd: &str, flags: &HashMap<String, String>, positional: &[String]) -> Result<i32, String> {
    match cmd {
        "retrieval" => {
            let report = retrieval_report(flags)?;
            let baseline: Option<RetrievalReport> = match flags.get("baseline") {
                Some(path) => std::fs::read_to_string(path)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok()),
                None => None,
            };
            if let Some(path) = flags.get("baseline") {
                if baseline.is_none() {
                    eprintln!("baseline {path} missing or unreadable");
                }
            }
            let (verdict, deltas) = report::compare(&report, baseline.as_ref());
            println!("{}", report::markdown(&report, verdict, &deltas));
            if let Some(path) = flags.get("json") {
                write_json(path, &report)?;
            }
            Ok(
                if flags.contains_key("strict") && verdict == Verdict::Worse {
                    1
                } else {
                    0
                },
            )
        }
        "baseline" => {
            let report = retrieval_report(flags)?;
            let out = flags.get("out").ok_or("--out FILE is required")?;
            write_json(out, &report)?;
            println!(
                "{}",
                report::markdown(&report, Verdict::BaselineMissing, &[])
            );
            println!("wrote {out}");
            Ok(0)
        }
        "answers" | "graded" => {
            let (store, _dir, _corpus, questions, _) = seeded(flags)?;
            let routing_json = flags
                .get("routing-json")
                .map(|p| std::fs::read_to_string(p).map_err(|e| e.to_string()))
                .transpose()?;
            let routing = api::RoutingConfig::from_json(routing_json.as_deref());
            let ollama_model = flags
                .get("ollama-model")
                .cloned()
                .unwrap_or_else(|| "gemma4:12b".into());
            let parent = format!("eval-{}", corpus::default_now_unix_ms());
            let mut answerer = |q: &sets::Question| {
                api::ask::answer(
                    &store,
                    &q.natural,
                    &ollama_model,
                    eval::APP_CONTEXT_LIMIT,
                    &[],
                    &routing,
                )
                .ok()
            };
            let (result, outcomes) = answers::run(&questions, &mut answerer);
            println!("{}", report::table("Answer eval", &result.sets));
            let usage = store.sum_usage_by_parent(&parent).unwrap_or_default();
            println!(
                "calls {} cents {} tokens {}+{}",
                usage.calls, usage.estimated_cost_cents, usage.input_tokens, usage.output_tokens
            );
            if cmd == "graded" {
                let grader_json = flags
                    .get("grader-routing-json")
                    .map(|p| std::fs::read_to_string(p).map_err(|e| e.to_string()))
                    .transpose()?
                    .ok_or("--grader-routing-json FILE is required")?;
                let grader_routing = api::RoutingConfig::from_json(Some(&grader_json));
                let chain = api::routing::build_chain_for(
                    llm::TaskKind::Chat,
                    &ollama_model,
                    &grader_routing,
                );
                let link = chain
                    .first()
                    .ok_or("the grader routing has no usable provider")?;
                let (g, verdicts) = graded::run(
                    &questions,
                    &outcomes,
                    link.client.as_ref(),
                    link.provider.as_str(),
                    &link.model,
                );
                println!("{}", serde_json::to_string_pretty(&g).unwrap_or_default());
                if let Some(path) = flags.get("json") {
                    write_json(
                        path,
                        &serde_json::json!({ "answers": result, "outcomes": outcomes, "graded": g, "verdicts": verdicts }),
                    )?;
                }
            } else if let Some(path) = flags.get("json") {
                write_json(
                    path,
                    &serde_json::json!({ "answers": result, "outcomes": outcomes }),
                )?;
            }
            Ok(0)
        }
        "routines" => {
            let k = flags.get("k").and_then(|s| s.parse().ok()).unwrap_or(4);
            let seed = flags
                .get("seed")
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_SEED);
            let r = routines::run(k, seed).map_err(|e| e.to_string())?;
            println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            Ok(0)
        }
        "preferences" => {
            let path = positional.first().ok_or("eval preferences FILE")?;
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let (records, skipped) = preference::parse_jsonl(&text);
            let s = preference::summarize(&records, skipped);
            println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default());
            Ok(0)
        }
        _ => {
            eprintln!("usage: eval retrieval|baseline|answers|graded|routines|preferences [flags]  (see src/main.rs)");
            Ok(2)
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, flags, positional) = parse_args(&args);
    match run(&cmd, &flags, &positional) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("eval: {e}");
            std::process::exit(1);
        }
    }
}
