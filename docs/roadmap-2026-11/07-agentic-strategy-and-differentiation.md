# 7. Agentic AI strategy and differentiation

## 7.1 Where Whimbrel is on the agent ladder

| Rung | Meaning | Whimbrel today | After H2 | After H3 |
|---|---|---|---|---|
| 0 Reactive | answers when asked, with citations | yes (Ask, Meeting Ask) | | |
| 1 Scheduled | runs fixed prompts on a clock | yes (seven templates) | | |
| 2 Event-driven | runs on conditions | no | F7 | |
| 3 Single gated action | proposes one write, a person approves | yes (two tools) | | |
| 4 Bounded tool loop | plans and executes several steps with tiered approvals, verifies, logs | no | F8 | |
| 5 Persistent state | remembers what it did and what it learned across runs, as structure | no (routine runs only) | F2 and F9 | |
| 6 Delegated and remote | runs when the laptop is closed, with grants | no | | F11 |
| 7 Autonomous computer use | operates the desktop | no, and not planned (document 3 §3.4) | | |

The roadmap moves from rungs 0, 1 and 3 to rungs 2, 4, 5 and 6 in a year, and stops at 6
on purpose.

## 7.2 Principles

1. **Memory before agency.** An agent over rows guesses; an agent over a graph of people,
   commitments and decisions acts on structure. F2 precedes F8 in value even where F8
   ships first in time.
2. **Offer, do not act, by default.** The research in document 3 §3.8 is one-directional:
   anticipatory action raises perceived threat; offering does not. Every proactive surface
   says why it appeared and comes from a routine the user can see.
3. **Gate by reversibility, not by prompt.** Approval prompts get rubber-stamped at 93%
   (document 3 §3.4). Read-only runs free; reversible runs under a standing grant;
   irreversible always waits for a person; forbidden is never a tool.
4. **Verify every run.** A plan, execute, verify loop with a mandatory verification turn,
   because the third failure class in MAST is missing verification, and because pass^k
   collapses without it.
5. **Local by default, upgrade by key.** Extraction, classification and short summaries run
   on a 4B model; the user's own frontier key is an upgrade, never a requirement. This is
   decision 4 applied to agents.
6. **Everything leaves a trail the user owns.** Audit log, spans, cost per run, in SQLite
   on the device, exportable. The trail is a feature, and no cloud competitor can offer a
   user-held one.
7. **The firewall ships with the loop.** A tool-using agent fed screen text and email is
   the textbook injection target. O-33 is part of F8, not a follow-up.

## 7.3 Agent types and where they live

| Agent | Trigger | Tools | Tier ceiling | Horizon |
|---|---|---|---|---|
| Meeting follow-up | `meeting_ended` with a person or title match | read memory, `create_draft` | Reversible | H2 |
| Reply brief | `email_received` from a domain or entity | read memory, thread | ReadOnly | H2 |
| Commitment keeper | `commitment_due` | read memory, `create_draft`, calendar hold | Reversible | H2 |
| Return brief | first activity after `idle` of 8 hours or more | `facts_changed_since` | ReadOnly | H2 |
| Pre-mortem | `meeting_starting` for a decision-tagged meeting | graph, meeting prep | ReadOnly | H3 |
| Relationship health | weekly | graph, thread latency | ReadOnly | H3 |
| Cloud runner | any of the above when the laptop is closed | per-routine grants | as granted, never Irreversible auto | H3 |

Each is a routine template with a trigger and a tool grant list, not a new subsystem; the
difference from today's templates is F7 and F8 under them.

## 7.4 What stays out

- Autonomous desktop control, vertical sales or recruiting agents, meeting bots, a hosted
  memory server, managed inference. Reasons are in document 4 §4.5 and doc 16.
- Multi-agent orchestration. One governed loop with a verify turn beats a committee of
  agents for a single user; the inter-agent misalignment class in MAST is avoided by not
  having inter-agent anything.

## 7.5 Differentiation strategy

**The position.** Whimbrel is the memory that is yours: captured on your machine,
structured into people, promises and decisions, acted on by routines you can read, synced
end to end, run on the models you choose, open source. Each clause is a verified fact in
document 1 or a roadmap item here, and each is something a named competitor cannot say.

**Why competitors cannot follow quickly**

| Clause | Who is closest | Why they cannot copy it in a quarter |
|---|---|---|
| Captured on your machine, including screen | Littlebird (cloud), Screenpipe (local, no longer open source) | Littlebird's product is its AWS memory; moving it local is a rewrite and a business-model change |
| Structured into people, promises, decisions | nobody in the end-user market; Zep and Mem0 as infrastructure | Needs the capture, the structure and the UI together; infrastructure vendors have no capture, notetakers have no structure |
| Routines you can read, gated by reversibility, with a user-held audit log | Littlebird (drafts only, cloud), Claude Cowork (local files, no ambient memory) | Cloud products keep the log; a local audit trail is a different architecture |
| End-to-end synced memory | Screenpipe (optional) | Granola, Littlebird, Otter and the suites sync through vendor-readable storage by design |
| Your own models, cheap by default, local when you want | anarlog, OpenClaw | The subscription-resale model is the revenue; credits are spreading, not receding |
| Open source with a self-hostable relay | anarlog (meetings only) | Screenpipe went the other way; Granola and Littlebird are venture-funded closed products |

**Moats, in order of durability**

1. **The graph compounds.** Every day of capture adds nodes and edges a new product
   cannot backfill; a user who has six months of commitments and decisions in Whimbrel is
   not moving to a notetaker.
2. **Architecture.** Local-first plus E2E plus own keys is a set of choices, each of which
   forecloses a cloud competitor's revenue model. They can add a local mode (Littlebird
   says it is not offered); they will not move the memory.
3. **Trust record.** Document 2 §2.6 item 7: Dot, Rewind, Limitless desktop, Khoj Cloud,
   Zoom's custom companion all vanished within 18 months. Apache source plus a relay anyone
   can run is an answer no venture-backed closed product gives.
4. **Community routines** (O-30, H4): signed routine files traded in a gallery are a
   network effect that attaches to the format, not to a server.
5. **Eval discipline.** Shipping a published answer-quality number per release is rare and
   cheap to keep once F3 exists.

**What is not a moat.** Transcription quality (whisper.cpp is everyone's), summaries (every
product has them), the number of integrations (Littlebird has 100-plus; Whimbrel has
three), and price (cheap-by-default is a position, not a moat, until the graph makes
leaving expensive in the honest sense: the user loses their own structure).

**Positioning against each competitor in one line**

- Granola: beautiful notes in their cloud; Whimbrel remembers everything, on your machine.
- Littlebird: the same idea with the memory on AWS and the inference resold.
- Screenpipe: deep local capture for developers, no longer open source, no gated actions.
- anarlog: open-source meetings; Whimbrel is open-source memory with routines.
- Otter, Fireflies, Fathom, tl;dv: bots in your meetings; Whimbrel sends no bot.
- Notion, Microsoft, Google, Zoom: notes inside their suite; Whimbrel works across all of
  them and belongs to you.
- Mem0, Zep, Letta: memory for developers' agents; Whimbrel is memory for a person.
- OpenClaw, Pipali: agents without ambient memory; Whimbrel is the memory they lack.

## 7.6 Risks to the strategy

- **Local extraction underperforms** (document 3 §3.3 has no measurement). Mitigation:
  the heuristic extractor ships first, the graph is useful with heuristics alone, and a
  user key upgrades it.
- **Graph retrieval hurts small corpora** (document 3 §3.1). Mitigation: F3 before F6; the
  graph leg is a feature flag until the eval says yes.
- **Approval fatigue** makes the gates theatre. Mitigation: tiers, not prompts; the count
  of prompts per week is a tracked metric with a ceiling.
- **Proactivity backlash.** Mitigation: F5 first, offer-not-act, intensity control.
- **A platform ships ambient memory** (Microsoft Recall plus Copilot, Apple). Mitigation:
  they will be cloud-bound or OS-bound; the cross-platform, E2E, open-source position holds.
  Apple is the one that could make it local and private; it has shown no sign of
  system-audio capture of third-party meetings.
- **One engineer.** Every horizon above has one spine; if capacity drops, the satellites
  go, not the spine.
