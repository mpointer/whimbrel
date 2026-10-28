# Whimbrel

An ambient desktop assistant that builds a persistent, private memory of your work, and acts on it.

It combines four capabilities that today live in separate products:

1. **Bot-free meeting capture** (like [Granola](https://granola.ai)) — records system audio + microphone on your machine, no bot joins the call. Works with Zoom, Teams, Google Meet, Slack huddles, and in-person conversations. Triggered by calendar entries *and* by detecting meeting audio.
2. **Always-on screen context** (like [Littlebird](https://littlebird.ai)) — reads the text of your active window (not screenshots) to build a searchable timeline of everything you worked on.
3. **Agentic desktop automation** (like [Hermes Agent](https://hermes-agent.org) and [OpenClaw](https://openclaw.ai)) — planned, not built. Today `core/agent` defines approval-gated write actions for routines (create a calendar event, send an email). The computer-use driver, guardrails, audit log and skill format are milestone M2.3 in [docs/05-roadmap.md](docs/05-roadmap.md) and have not started.
4. **Routines + MCP tool suite** — cron-scheduled, proactive briefings and automations (daily brief, end-of-day digest, inbox triage) over a generic MCP host. Connectors that ship today: Outlook 365, Gmail and Google Calendar (`mcp-servers/`). Slack, Linear, Todoist and Google Drive are on the list, not in the tree.

All of it aggregates **across devices and app instances** — Macs and Windows PCs today, phones next — into one consolidated, end-to-end-encrypted picture of your activity, meetings, and tasks.

## Platform strategy

| Phase | Target | Status |
|-------|--------|--------|
| 1 | macOS desktop (capture, meetings, memory, chat) | Shipped: signed, notarized builds with verified auto-update |
| 2 | Routines, MCP suite, multi-device sync | Shipped: routines, three connectors, E2E-encrypted sync via the relay in `server/`. Agentic automation (M2.3) not started |
| 3 | Windows desktop client | Shipped: Tauri shell, NSIS installer, signed updater ([12](docs/12-windows-app-design.md)) |
| 4 | iOS and Android companion apps | In progress: iOS enrollment and sync peer ([15](docs/15-mobile-app-design.md)); Android not started |

## Documentation

| Doc | Contents |
|-----|----------|
| [01 – Product Vision](docs/01-product-vision.md) | Problem, personas, competitive landscape, differentiation |
| [02 – Functional Requirements](docs/02-functional-requirements.md) | Numbered, testable requirements by subsystem |
| [03 – Non-Functional Requirements](docs/03-non-functional-requirements.md) | Privacy, performance, security, compliance |
| [04 – Architecture](docs/04-architecture.md) | Component design: Swift shell + Rust core, data model, LLM strategy |
| [05 – Roadmap](docs/05-roadmap.md) | Phased milestones with acceptance criteria |
| [06 – Risks & Open Questions](docs/06-risks-open-questions.md) | Legal, platform, and product risks |
| [07 – Sync & Web Viewer](docs/07-sync-and-web-viewer.md) | Superseded design — shipped sync uses Cloudflare Worker + per-account Turso (see 04/05) |
| [08 – Littlebird Parity & Beyond](docs/08-littlebird-parity-spec.md) | Phased requirements to match, then surpass, Littlebird's feature set |
| [12 – Windows Desktop Client](docs/12-windows-app-design.md) | Phase 3 design: Tauri/Rust shell, UIA + WASAPI capture layer, the core consolidation it depends on |
| [14 – Pricing Proposal](docs/14-pricing-proposal.md) | Proposed tiers vs. Granola and Littlebird, AI cost model, what billing needs built first |
| [15 – Mobile App Design](docs/15-mobile-app-design.md) | Phone app vs. Littlebird's and Granola's: brief, ask, approve, record; phone as a sync peer, push, approvals, milestones |
| [Self-hosting](docs/self-hosting.md) | Running the sync relay and the update feed under your own Cloudflare, Turso and Google accounts |

The design plans and specs these documents refer to (`docs/superpowers/`) stay in the
private development repository; a link into that folder points at an internal document.

## Development and this repository

Development happens in a private repository that runs the project's signing and
self-hosted CI. This repository is published from it at each release
(`scripts/publish-public.sh`): one commit per release with the whole tree except the
design history, tagged with the version. Pull requests here are welcome and are applied
in the private repository, then arrive with the next release.

## Repository layout

```
apps/macos/    SwiftUI shell + native capture services (ScreenCaptureKit, EventKit, Accessibility)
apps/windows/  Tauri v2 shell + Rust capture services (UIA, WASAPI)
apps/mobile/   iOS companion app (Android not started)
core/          Shared Rust core: ingestion, memory, agent runtime, MCP host, routines, sync
server/        Sync relay (Cloudflare Worker); server-updates/ serves the update feed
website/       Marketing site + lead capture (Cloudflare Worker, static pages, D1)
docs/          Requirements and architecture documents
```

## Status

Early access. Mac and Windows builds are signed and distributed from the website; every feature is free during early access. The roadmap's per-milestone notes in [docs/05-roadmap.md](docs/05-roadmap.md) were written as the work happened and some status lines there lag the tree; the platform table above is current. [docs/08-littlebird-parity-spec.md](docs/08-littlebird-parity-spec.md) is the parity build-out backlog.

Not built yet, whatever older documents say: the computer-use agent and skill format (M2.3), connectors beyond Outlook 365, Gmail and Google Calendar, and Android.

## Building from source

Every component builds without signing credentials or the hosted services; sync and auto-update are simply unavailable until configured.

- **Rust core:** `cd core && cargo build`. Needs CMake (whisper.cpp builds from source) and network access on the first build (sherpa-onnx downloads a prebuilt ONNX Runtime). Tests: [core/README.md](core/README.md).
- **macOS app:** Xcode 15+, open `apps/macos/Whimbrel.xcodeproj` and run. A free Apple ID is enough for a Debug build; the whisper model is fetched separately. Details, permission prompts and the TCC caveat for unsigned builds: [apps/macos/README.md](apps/macos/README.md).
- **Windows app:** Node, Rust (MSVC) and VS Build Tools, then `npm run tauri dev` in `apps/windows`. Details: [apps/windows/README.md](apps/windows/README.md).
- **Connectors:** `npm ci` in each `mcp-servers/*` directory.
- **Relay and update feed:** [docs/self-hosting.md](docs/self-hosting.md).

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE). Contributions: [CONTRIBUTING.md](CONTRIBUTING.md).
