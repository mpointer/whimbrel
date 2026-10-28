//! Re-validates the bleed filter on real recordings (#188). Input: two TSVs of
//! `start_cs<TAB>end_cs<TAB>text`, one per channel, e.g. from `dump_segments`.
//! `cargo run --release -p transcribe --example bleed_report -- mic.tsv system.tsv`
use std::collections::HashSet;
use transcribe::postprocess::{filter_bleed_through, Line, BLEED_TIME_TOLERANCE_CS};

fn load(path: &str, speaker: &str) -> Vec<Line> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            Some(Line {
                speaker_label: speaker.into(),
                start_cs: it.next()?.parse().ok()?,
                end_cs: it.next()?.parse().ok()?,
                text: it.next()?.into(),
            })
        })
        .collect()
}

fn tokens(t: &str) -> HashSet<String> {
    t.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mic = load(&args[1], "Me");
    let system = load(&args[2], "Others");
    let kept: HashSet<i64> = filter_bleed_through(&mic, &system)
        .iter()
        .map(|l| l.start_cs)
        .collect();
    for m in &mic {
        let mt = tokens(&m.text);
        let best = system
            .iter()
            .filter(|s| {
                s.start_cs - BLEED_TIME_TOLERANCE_CS <= m.end_cs
                    && m.start_cs <= s.end_cs + BLEED_TIME_TOLERANCE_CS
            })
            .map(|s| mt.intersection(&tokens(&s.text)).count() as f64 / mt.len().max(1) as f64)
            .fold(0.0, f64::max);
        let dropped = !kept.contains(&m.start_cs);
        if dropped || best >= 0.3 {
            println!(
                "{} {:>7.1}s c={best:.2} | {}",
                if dropped { "DROP" } else { "keep" },
                m.start_cs as f64 / 100.0,
                m.text
            );
        }
    }
    println!("mic lines: {}, kept: {}", mic.len(), kept.len());
}
