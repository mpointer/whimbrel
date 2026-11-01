# Handoff: Whimbrel Release 1, "First meeting, first minute"

A prompt for the coding agent. Paste it whole. It scopes the first increment of
[17-competitive-gaps-roadmap-monetization.md](../17-competitive-gaps-roadmap-monetization.md)
§4 "Release 1" and nothing else. Written 2026-10-31 against 0.9.43.

---

## Read first, in this order

1. `docs/17-competitive-gaps-roadmap-monetization.md` — the strategy. Sections 0, 1, 3.1 and 4
   (Release 1) are the brief; everything else is context.
2. `docs/16-open-core-commercial-model.md` — the five decisions of 2026-10-29. They are settled.
   Do not design around them.
3. `docs/05-roadmap.md` and `docs/12-windows-app-design.md` — how milestones are recorded and
   how the Windows shell relates to core.
4. `CONTRIBUTING.md`, `core/README.md`, `apps/macos/README.md`, `apps/windows/README.md`.

Then survey the code paths each item names before writing anything. Doc 17 §3.1 lists, per gap,
what already exists to build on; verify each claim against the tree and correct the doc in your
first PR if any is stale.

## Scope: Release 1 only

Deliver the seven items in doc 17 §4 "Release 1", in this order, one PR each:

1. **1.1 Setup in one screen.** Onboarding asks for Microphone and Screen/System Audio only;
   Accessibility, Calendar and Notifications are requested the first time the feature that needs
   them is touched, from inside that feature. The search model downloads in the background with
   keyword search working meanwhile; say so in the UI. Sign-in is never shown until the user
   opens Sync or adds a second device.
2. **1.2 Better transcripts by default.** On macOS 26, Apple `SpeechAnalyzer` as a
   `core/transcribe` backend behind the existing trait and the default where available; on older
   Macs and on Windows, offer `whisper-large-v3-turbo` (quantised) as a one-click download gated
   on a measured speed floor, with `base` as the floor model. Live chunking
   (`core/transcribe/src/live.rs`) must keep working. Measure word error rate on three real
   meetings before and after and record it in `docs/release-evidence/`.
3. **1.3 Ask works with nothing configured.** A "Download a small local model" button on the
   empty Ask state that fetches a 3–4B instruct model and runs it locally with no account or key.
   Decide llama.cpp-in-process versus a bundled Ollama launcher in the design doc, with the
   licensing and installer-size numbers for each.
4. **1.4 Chrome and Edge meeting detection.** Decide the `AXEnhancedUserInterface` lifecycle on
   the Mac (set when a meeting-capable tab is detected, unset on quit) and ship it; confirm the
   Windows UIA address-bar path already covers Edge and Chrome.
5. **1.5 Pitch matches product.** README item 3 and the website hero drop "agentic desktop
   automation" from the lead and mark it planned. Website copy is in `website/site/pages/`; run
   the site tests.
6. **1.6 Import.** `.vtt`, `.srt`, `.txt`, Granola CSV and Otter exports as `meeting` rows with
   transcript segments and a model-written summary; a folder of Markdown as `items`. The empty
   Meetings state offers it.
7. **1.7 Recording notice.** An optional, off-by-default per-meeting notice (a chat line where
   the platform allows it, otherwise a reminder to announce), generalising Away Mode's
   `ZoomChatPosting`; move the jurisdiction note from Settings into onboarding.

Do not start Release 2 or 3 items, M2.3 (agent), billing, or anything in doc 17 §5. If an item
turns out to need one of those, stop and report rather than widen the PR.

## How to work

- Follow the repo's spec-then-plan convention: a `docs/superpowers/specs/<date>-<name>-design.md`
  before code, a `docs/superpowers/plans/<date>-<name>.md` with tasks, then the PR. Keep the
  design doc honest about what the server or a third party can see.
- Portable logic goes in `core/api` or the relevant core crate, never in a shell. Both shells get
  the feature in the same PR unless the design doc states why Windows follows in a named
  follow-up. Every user-facing string goes through `t()` / `tr()` and the l10n catalogs.
- Tests: Rust tests for every core change, Swift and Vitest tests for shell logic. Run
  `cargo test --workspace` in `core/`, the Swift tests, `npm test` in `apps/windows/ui`,
  `server/` and `website/` as touched, and `python3 scripts/rename-whimbrel.py --check` before
  every push.
- Each PR gets a `CHANGELOG.md` entry under the unreleased version, in the existing voice (plain
  sentences, platform subsections).
- Nothing paid is ever written into the open tree. No hosted model, no telemetry, no token
  resale. If a change would weaken "the relay sees only ciphertext," stop and ask.

## Ask before doing

- Changing the default transcription model for existing users (item 1.2): propose the migration
  and the opt-out in the design doc first.
- Anything that deletes or rewrites user data, changes the sync schema, or touches `server/`.
- Any new runtime dependency over 50 MB in the installer.

## Release 1 gate (from doc 17)

A fresh install on a Mac you have never configured produces a summary of a real meeting with no
Settings pane opened. Record that run, with the WER numbers from 1.2, in
`docs/release-evidence/`.

## Report back per item

One short message: what shipped, the PR, test results verbatim, what was measured, anything you
deferred and why. If you got blocked, say exactly what you need.
