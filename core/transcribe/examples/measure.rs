//! Speed and word error rate for docs/release-evidence/. Prints numbers only, never text.
//!
//! The hypothesis file holds meeting text, so keep it out of the repo.
//!
//!   cargo run --release -p transcribe --example measure -- model <ggml.bin> <16k-mono.wav> <hypothesis-out.txt> [reference.txt]
//!   cargo run --release -p transcribe --example measure -- text <reference.txt> <hypothesis.txt>

use std::path::Path;
use std::time::Instant;

use transcribe::engine::word_error_rate;
use transcribe::Transcriber;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let read = |p: &str| std::fs::read_to_string(p).expect("readable text file");
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["text", reference, hypothesis] => println!("wer={:.4}", word_error_rate(&read(reference), &read(hypothesis))),
        ["model", model, wav, out, ref rest @ ..] => {
            let transcriber = Transcriber::load(Path::new(model)).expect("model loads");
            let seconds = {
                let r = hound::WavReader::open(wav).expect("wav opens");
                r.duration() as f64 / r.spec().sample_rate as f64
            };
            let started = Instant::now();
            let segments = transcriber.transcribe_wav(Path::new(wav)).expect("transcribes");
            let speed = seconds / started.elapsed().as_secs_f64();
            let text = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
            std::fs::write(out, &text).expect("hypothesis file writes");
            let wer = rest.first().map(|r| format!(" wer={:.4}", word_error_rate(&read(r), &text))).unwrap_or_default();
            println!("seconds={seconds:.1} speed={speed:.1}x{wer}");
        }
        _ => eprintln!("usage: measure model <ggml.bin> <wav> <hypothesis-out.txt> [reference.txt] | measure text <reference.txt> <hypothesis.txt>"),
    }
}
