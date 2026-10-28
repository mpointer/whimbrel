# 03 — Non-Functional Requirements

Numbered `NFR-<AREA>-<n>`. All apply from Phase 1 unless noted.

## Privacy & data handling

| ID | Requirement |
|----|-------------|
| NFR-PRV-1 | Local-first: all captured data is stored and indexed on-device by default. Cloud involvement is limited to (a) E2E-encrypted sync ciphertext, (b) LLM API calls the user has configured, (c) MCP connector traffic the user has authorized. |
| NFR-PRV-2 | Encryption at rest for the local store (SQLCipher or equivalent, key in Keychain) and TLS 1.2+ in transit. Sync payloads are end-to-end encrypted; the relay cannot decrypt. |
| NFR-PRV-3 | User data is never used to train models — ours or any vendor's. Hosted-LLM calls must use no-training/zero-retention API tiers where offered. |
| NFR-PRV-4 | Every prompt sent to a hosted model that includes memory-derived content is loggable and inspectable by the user ("what left my machine?" view). |
| NFR-PRV-5 | Full export (JSON + markdown + audio files) and full account/data deletion, including synced replicas, completable within 30 days. |
| NFR-PRV-6 | Compliance path: GDPR and CCPA from launch posture; SOC 2 Type II targeted within 12 months of commercial launch. |
| NFR-PRV-7 | Recording-consent support: jurisdiction-aware guidance surfaced in-product; announcement templates; per-meeting discard (see FR-MTG-6). |

## Performance (always-on daemon)

| ID | Requirement |
|----|-------------|
| NFR-PRF-1 | Idle/ambient capture: ≤5% average CPU on Apple silicon, ≤500 MB RSS excluding any resident local LLM. |
| NFR-PRF-2 | During meeting transcription: CPU burst permitted; the machine must remain responsive (no dropped meeting audio; UI thread never blocked). |
| NFR-PRF-3 | Battery: ambient capture adds ≤5% to daily battery drain on a MacBook under typical usage. |
| NFR-PRF-4 | Search returns first results in <500 ms over 1 year of single-user data; timeline scrolling at 60 fps. |
| NFR-PRF-5 | Capture-to-searchable latency ≤60 s for screen text; meeting summary delivered ≤2 min after meeting end. |

## Reliability

| ID | Requirement |
|----|-------------|
| NFR-REL-1 | Capture daemon supervises itself: crash → auto-restart ≤5 s; no more than the in-flight batch of captures lost (WAL journaling). |
| NFR-REL-2 | A meeting recording in progress survives app crash and restart with at most 10 s of audio loss. |
| NFR-REL-3 | Fully functional offline except hosted-LLM chat, remote MCP tools, and sync; all queue and recover when connectivity returns. |
| NFR-REL-4 | OS update resilience: on macOS majors, degrade gracefully and telemetry-flag broken capture paths rather than silently losing data. |

## Security

| ID | Requirement |
|----|-------------|
| NFR-SEC-1 | All secrets (LLM keys, MCP OAuth tokens, sync keys) in the OS keychain; nothing sensitive in plaintext config. |
| NFR-SEC-2 | Agent actions run under the layered guardrail policy (FR-AGT-2) with tamper-evident audit logs. |
| NFR-SEC-3 | Signed and notarized distribution with Sparkle-style signed auto-updates. Mac App Store is out of scope (capture entitlements). |
| NFR-SEC-4 | Third-party MCP servers and skills are treated as untrusted: permission manifests enforced, no transitive privilege escalation, prompts from tool outputs never auto-execute privileged actions. |
| NFR-SEC-5 | Annual third-party penetration test before commercial launch; dependency and supply-chain scanning in CI. |

## Portability & maintainability

| ID | Requirement |
|----|-------------|
| NFR-PRT-1 | All non-UI, non-capture logic (ingestion, memory, agent, MCP host, routines, sync, LLM routing) lives in the shared Rust core with a C-ABI/FFI boundary, buildable for macOS, Windows, and Android from day one of Phase 1. |
| NFR-PRT-2 | Platform capture layers are thin adapters implementing a common capture-source trait (macOS: ScreenCaptureKit/AX; Windows: Graphics Capture + UIA + WASAPI loopback; Android: mic + notifications only). |
| NFR-PRT-3 | CI runs core tests on all three targets from Phase 1 even though only macOS ships first. Landed: macOS runs the full `cargo test --workspace`; Windows runs `cargo test --workspace` with four crates excluded from test execution and build-verified only (`transcribe`, `diarize`, `ffi`, and `sync-engine`), where `sync-engine` was added beyond the original plan because its test binary hits a real MSVC LNK2005 duplicate-symbol linker conflict between rusqlite's bundled sqlite3 and libsql's `core`-feature bundled sqlite3 (both statically link the same C symbol names, which MSVC's `link.exe` rejects and macOS's linker tolerates); Android cross-compiles via `cargo-ndk` to `aarch64-linux-android` using `cargo ndk -t arm64-v8a build --workspace --exclude diarize --exclude ffi`, build/check-only for every included crate since an x86 CI host cannot execute `aarch64` binaries regardless of crate (`diarize` is excluded because its `sherpa-onnx-sys` v1.13.4 dependency has no prebuilt ONNX Runtime archive for `os=android, arch=aarch64`, with build.rs panicking "Unsupported target for sherpa-onnx prebuilt libs: os=android, arch=aarch64", and `ffi` is excluded only transitively via its path-dependency on `diarize`; `transcribe`, via whisper-rs-sys/CMake, is *not* excluded and cross-compiles successfully for Android). The Android job also requires `RUSTFLAGS: -C target-feature=+fp16` to build the `memory` crate's `candle`/`gemm-f16` dependency (needed for the local embedding model), a real runtime constraint and not just a build flag: Android binaries produced by this pipeline require an ARMv8.2-A+ (FEAT_FP16-capable) device, and older/budget cores predating ARMv8.2 could SIGILL on these instructions. |

## Accessibility & localization

| ID | Requirement |
|----|-------------|
| NFR-ACC-1 | The app UI itself is fully VoiceOver-navigable (ironic failure mode to avoid: an Accessibility-API consumer that isn't accessible). |
| NFR-ACC-2 | Transcription in ≥10 languages at launch; UI localization English-first with string externalization from day one. |
