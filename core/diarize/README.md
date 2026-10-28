# core/diarize

Real per-speaker diarization for the system-audio channel — see
`docs/superpowers/specs/2026-07-28-speaker-diarization-design.md`.

Needs two ONNX models, not checked into git (same reasoning as `core/transcribe`'s
whisper GGML model — models are large binary files that don't belong in version
control):

## Segmentation model

```
mkdir -p models
cd models
curl -L -o seg.tar.bz2 https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2
tar xjf seg.tar.bz2
rm seg.tar.bz2
```

This produces `models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx`.

## Speaker-embedding model

```
cd models
curl -L -o wespeaker_en_voxceleb_resnet34_LM.onnx https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34_LM.onnx
```

(Note: `recongition` — not a typo on our part, that's the actual upstream release tag name.)

## Test fixture (for the `#[ignore]`'d integration test)

A real multi-speaker WAV, 16kHz mono. The upstream project publishes one:

```
cd models
mkdir -p test-fixtures
curl -L -o test-fixtures/two-speakers.wav https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/1-two-speakers-en.wav
```

## Running the ignored test

```
cargo test -p diarize -- --ignored
```
