# 06 — Risks & Open Questions

## Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|------------|--------|------------|
| R1 | **Recording-consent law.** Two-party-consent jurisdictions (e.g., CA, WA, several EU states) make silent meeting capture legally risky for the user. | High | High | Consent UX defaults (visible indicator, announcement templates, per-meeting discard); jurisdiction guidance in onboarding; legal review before commercial launch. |
| R2 | **Single system-audio capturer on macOS.** Zoom local recording, Granola, or another tool holding the tap blocks our capture. | Medium | Medium | Detect conflict, degrade to mic-only with a clear warning (FR-MTG-10); document coexistence guidance. |
| R3 | **TCC permission friction.** Four separate scary permission prompts can kill onboarding conversion. | High | Medium | Staged, value-first permission requests (FR-ONB-1); app remains useful with partial grants (FR-ONB-2). |
| R4 | **Privacy backlash / trust.** Always-on capture is the product's core and its biggest liability (cf. Rewind criticism, Microsoft Recall). | Medium | High | Text-only capture, no default screenshots, local-first storage, E2E sync, exclusion defaults, transparent "what left my machine" ledger (NFR-PRV-4). |
| R5 | **LLM cost at always-on scale.** Naive summarization of an 8-hour workday is expensive. | High | Medium | Local embeddings/transcription; batched summarization; cheap-model routing for routine work; user-visible cost controls. |
| R6 | **macOS API churn.** ScreenCaptureKit and TCC behavior shift across majors. | Medium | Medium | Capture-source abstraction (NFR-PRT-2); beta-OS CI; degrade-don't-lose-data policy (NFR-REL-4). |
| R7 | **Agent misfires.** A computer-use agent typing into the wrong window is a trust-ending event. | Medium | High | Accessibility-first actions, layered approvals, kill switch, audit log (FR-AGT-1…4); agency ships in Phase 2, after trust is established by capture/memory. |
| R8 | **Untrusted skills / MCP servers.** Third-party skills or tool outputs could attempt prompt injection or privilege escalation. | Medium | High | Permission manifests, no transitive escalation, tool outputs never auto-trigger privileged actions (NFR-SEC-4). |
| R9 | **Rust-core FFI overhead** slows early iteration vs. pure Swift. | Medium | Low | Accept the tax; it buys Phases 3–4. Keep the FFI surface small and message-based. |
| R10 | ~~**Dev-signing blocks TCC grants for Accessibility/Screen Recording.**~~ Resolved: Apple Development-signed builds carry `get-task-allow`, which pins TCC grants to the exact build hash rather than the Team ID — the grant never survives a rebuild. `apps/macos/scripts/release-publish.sh` now produces a real Developer ID Application-signed, notarized, stapled build (team `GP8PFLM3Q3`) — confirmed via `codesign -dvvv` (no `get-task-allow`) and `spctl -a -vv --type execute` (`accepted`, `source=Notarized Developer ID`); installing that build to `/Applications` and launching from Finder is the path that holds grants across rebuilds. Debug builds (day-to-day Xcode iteration) are unaffected and still lose grants on every rebuild — expected, not worth chasing, since the release script exists for exactly this case. See `apps/macos/README.md` § "Cutting a release". | Resolved | N/A | None — re-verify grants survive a rebuild on the next hands-on dogfooding session as a final live confirmation, but the mechanism itself is done. |
| R11 | **sqlite-vec crate build fragility.** The `sqlite-vec` Rust crate's current 0.1.10-alpha.* releases fail to build on at least one dev machine (missing `#include`d C source in the packaged crate — `sqlite-vec-diskann.c` not found). Pinned to `0.1.9`, which builds and passes tests, but future upgrades need to be verified, not assumed. | Confirmed on ≥1 machine | Medium (blocks upgrading, not current functionality) | Stay pinned to `0.1.9` until a newer release is verified to build cleanly; `core/memory`'s embedding is a deterministic placeholder regardless (see its module docs) pending a real local embedding model. |

## Open questions

1. ~~**Diarization depth.**~~ Resolved — real per-speaker separation on the system channel was built via the `core/diarize` crate (`sherpa-onnx`), not just channel-based (me vs. them) diarization; see `docs/superpowers/specs/2026-07-28-speaker-diarization-design.md`.
2. **Transcript language mixing.** How to handle code-switched meetings (two languages in one call)?
3. ~~**Sync relay hosting.**~~ Decided — see `07-sync-and-web-viewer.md`: self-hostable by design (thin relay + S3-compatible ciphertext storage, one code path for both), first-party hosted instance is small/cheap enough (~$5–10/mo) to offer alongside self-host rather than instead of it. Driven by a scope decision to build for personal/small-group use first, not public distribution — revisit if that changes.
4. **Skill marketplace.** Curated catalog vs. open sharing (OpenClaw-style) — and what review bar applies given R8?
5. **Windows capture parity.** UIA text extraction quality varies widely by app; how much OCR fallback will Windows need relative to macOS? Narrowed, not answered, by `12-windows-app-design.md` §4.1: the question is almost entirely about Chromium-hosted apps (Edge, new Teams, Slack, VS Code), which enable renderer accessibility on first UIA query at a lifetime CPU cost to that process. Native UIA apps (Word, Outlook, Explorer) extract cleanly. M3.1's capture spike measures yield, latency, and CPU delta per app on the real laptop before any UI is built; see that doc's W1.
6. **Pricing.** Littlebird anchors at free + $20/mo Pro. Which capabilities gate the paid tier here (likely: routines count, connectors, sync devices, hosted-model quota)?
7. ~~**Name.**~~ Partially decided — the macOS app itself is now named **Whimbrel** (`apps/macos`'s product name, bundle ID `com.mpointer.kestrel`, distinct from the real Littlebird it's modeled on). This was chosen for personal/small-group use, not a public brand decision — "littlebird-clone" remains the repo/project working title, and a public identity (if this is ever distributed beyond the current small group) is still open. Internal code identifiers (FFI crate/function prefix, OSLog subsystem, UserDefaults keys) were also renamed `littlebird_`/`com.littlebird.app` → `whimbrel_`/`com.mpointer.kestrel` (2026-08-05, `docs/superpowers/specs/2026-08-05-littlebird-to-kestrel-mechanical-rename-design.md`). Two things remain deliberately unaddressed by that rename: the on-disk data path (`~/Library/Application Support/littlebird/spike/`, real user data on two Macs, needs its own migration design; done in #180 B1, now `Application Support/Whimbrel/`) and the public-identity/branding question above.
8. ~~**Speaker-count hint for heuristically-detected meetings.**~~ Resolved —
   heuristically-detected meetings now get an estimated speaker-count hint via
   Accessibility-based readers for Zoom/Google Meet/Teams; see
   `docs/superpowers/specs/2026-08-02-heuristic-speaker-count-hint-design.md`.
9. **Context-stripping before LLM calls.** Related to R5 (LLM cost at always-on
   scale): captured screen-text and raw transcripts likely carry a lot of low-value
   text (repeated window chrome, boilerplate UI strings, filler words) that costs
   real tokens/cost in every `askQuestion`/`dailyDigest`/`summarizeMeeting` call
   without adding retrieval or summarization value. Investigate a filtering/
   compaction pass — conceptually similar to RTK (a personal CLI proxy that strips
   unnecessary text from tool output before it reaches an LLM) — applied to context
   assembled for these three calls, before it's sent to the provider. Open: where in
   the pipeline this belongs (capture-time vs. retrieval-time), whether it risks
   losing content the LLM actually needed, and whether the savings are worth the
   added complexity given local Ollama calls are already free.
