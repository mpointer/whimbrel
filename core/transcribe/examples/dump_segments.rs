//! Dumps one channel's whisper segments as `start_cs<TAB>end_cs<TAB>text` for
//! `bleed_report`. Input must be 16kHz mono WAV (`ffmpeg -i in.caf -ar 16000 -ac 1 out.wav`).
//! `cargo run --release -p transcribe --example dump_segments -- model.bin in.wav > out.tsv`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let t = transcribe::Transcriber::load(std::path::Path::new(&a[1])).unwrap();
    let segs = t
        .transcribe_wav_with_threads(std::path::Path::new(&a[2]), Some(8))
        .unwrap();
    for s in segs {
        println!("{}\t{}\t{}", s.start_cs, s.end_cs, s.text);
    }
}
