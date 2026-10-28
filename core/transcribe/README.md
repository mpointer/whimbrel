# core/transcribe

On-device transcription via [whisper.cpp](https://github.com/ggerganov/whisper.cpp) (through the `whisper-rs` bindings) — FR-MTG-4, docs/04-architecture.md.

## Model file

`models/ggml-tiny.en.bin` is **not checked into git** (it's ~75MB and gitignored — see `core/transcribe/models/` in the repo root `.gitignore`). Fetch it once:

```bash
mkdir -p core/transcribe/models
curl -L -o core/transcribe/models/ggml-tiny.en.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin
```

Swap in a larger model (`ggml-base.en.bin`, `ggml-small.en.bin`, ...) from the same repo for better accuracy at the cost of speed; `Transcriber::load` takes any GGML whisper model path.

## What's implemented

`Transcriber::load(model_path)` + `Transcriber::transcribe_wav(wav_path)` — takes a 16kHz i16 PCM WAV (mono or stereo; stereo is downmixed), runs whisper.cpp's full pipeline, and returns `TranscriptSegment`s (start/end in centiseconds + text). No resampling — the input must already be 16kHz.

Channel-based diarization (FR-MTG-4: me = mic channel, others = system channel) is the caller's job: transcribe the mic and system-audio files separately and label the results accordingly. This crate only turns one mono WAV into text.

## Verifying it works

The one real test (`transcribes_a_synthesized_sentence`) is `#[ignore]`'d since it needs the model file present, so it isn't part of the default `cargo test` run:

```bash
cargo test -p transcribe -- --ignored --nocapture
```

It synthesizes a real audio sample with macOS's built-in `say` command (no pre-recorded fixture needed) and asserts the transcript actually matches — this has been verified to correctly transcribe "The quick brown fox jumps over the lazy dog" end to end.
