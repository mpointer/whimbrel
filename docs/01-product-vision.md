# 01 — Product Vision

## Problem

Knowledge workers lose context constantly:

- **Decisions evaporate.** They live in meeting audio nobody re-listens to, or in a chat scrollback nobody searches.
- **Work history is scattered.** A single project touches a browser, an IDE, email, Slack, documents, and three meetings a week. No tool sees the whole picture.
- **Repetitive digital chores persist.** Standup prep, inbox triage, status reports, and meeting follow-ups consume hours weekly despite being highly automatable.
- **Context is per-device.** Work on a laptop, a desktop, and a phone never merges into one record.

Existing tools each solve one slice:

- **Granola** — meetings only.
- **Rewind / Limitless** — screen memory only (and screenshot-based, which is heavy and privacy-fraught).
- **Hermes Agent / OpenClaw** — agentic action, but no ambient memory of *your* work to ground it.
- **Littlebird** — the closest integration of memory + meetings + routines, but closed-source, cloud-account-bound, and with multi-device treated as a companion-app afterthought.

## Product thesis

An assistant is only as useful as its context. Capture context ambiently (screen text + meeting audio), store it privately, and every downstream capability — search, chat, briefings, automation — becomes dramatically better because it no longer needs to be told what you're working on.

## Personas

| Persona | Situation | Primary jobs-to-be-done |
|---|---|---|
| **Manager / executive** | 15+ meetings/week across Zoom/Teams/Meet | Perfect recall of what was said and decided; automatic briefings before and after meetings; delegation of follow-ups |
| **Individual contributor** | Deep work across many apps and long-running projects | "What was I doing / where did I see that?" search; end-of-day recap; context restoration after interruptions |
| **Power user / automator** | Comfortable with agents and integrations | Safe desktop automation; scheduled routines wired to email, calendar, and task tools via MCP |

## Competitive landscape

| Capability | Littlebird | Granola | Rewind/Limitless | Hermes | OpenClaw | **This product** |
|---|---|---|---|---|---|---|
| Bot-free meeting transcription | ✅ | ✅ | partial | ❌ | ❌ | ✅ |
| Calendar-triggered recording | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ |
| Meeting-audio auto-detection | ✅ | partial | ❌ | ❌ | ❌ | ✅ |
| Ambient screen memory (text, not screenshots) | ✅ | ❌ | ❌ (screenshots) | ❌ | ❌ | ✅ |
| Context-grounded chat | ✅ | partial | ✅ | ✅ | ✅ | ✅ |
| Agentic desktop automation w/ guardrails | ❌ | ❌ | ❌ | ✅ | ✅ | planned (M2.3) |
| Scheduled routines / proactive briefings | ✅ | ❌ | ❌ | partial | ✅ (heartbeat) | ✅ |
| Rich external tool suite | partial | ❌ | ❌ | ✅ | ✅ | ✅ (MCP-native; Outlook 365, Gmail, Google Calendar ship) |
| Multi-device consolidated timeline | partial | ❌ | ❌ | ❌ | ❌ | ✅ (first-class) |
| Model-agnostic (local + hosted LLMs) | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ |
| Portable/shareable skill format | ❌ | ❌ | ❌ | ✅ | ✅ | planned (M2.3) |

The "This product" column states what the tree does today (2026-10). Rows marked planned are milestone M2.3 in [05-roadmap.md](05-roadmap.md), which has not started.

## Differentiation

1. **Multi-device aggregation as a first-class feature.** Every client instance (desktop or mobile) is a peer contributing to one end-to-end-encrypted timeline, with per-device attribution — not a thin remote-control companion.
2. **MCP-native integration layer.** Instead of bespoke integrations, an embedded MCP host gives the product every connector in the MCP ecosystem (Outlook 365, Gmail, Slack, Linear, …) plus user-added servers.
3. **Model-agnostic LLM layer.** Local models (Ollama) through hosted frontier models, routable per query by sensitivity and cost.
4. **Open, portable skill format** for routines and agent skills, in the OpenClaw tradition.
5. **Guardrailed agency.** Hermes-style layered approvals make desktop automation trustworthy enough for non-hobbyists.

## Success metrics (v1)

- Time-to-first-value: user gets a useful memory answer or meeting summary within the first day.
- ≥60% of meetings on the machine captured (opt-outs excluded) after week 1.
- Daily brief routine retention ≥50% at 30 days among users who enable it.
- Capture overhead within performance budget (see NFRs) with zero user-visible data loss.
