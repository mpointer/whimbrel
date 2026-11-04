# 8. First 90 days, engagement design, measurement, and the product vision

## 8.1 The first 90 days (Nov 2026 to Jan 2027)

The constraint is one engineer plus coding agents, with doc 17's Release 2 (Windows
parity) and Release 1's remainder in the same window. The 90-day plan therefore has one
spine (make the dormant memory visible and give it structure) and three small satellites
that de-risk everything after.

| Weeks | Work | Done means |
|---|---|---|
| 1 to 2 | **F3 eval harness, phase 1**: seeded corpus, retrieval sets, local graders, CI job; `parent_trace_id` and `span_kind` on the ledger | A baseline recall at 10 and citation precision number exists in CI |
| 1 to 3 | **F4 trust surface**: capture indicator, exclusions on both desktops, export and delete everything, the "what the relay sees" page | Shipped in a point release; website page live |
| 3 to 6 | **F1 Connected Topics surfaced**: jobs enabled, FFI and Tauri, Topics view on both desktops, Obsidian export, topics and entities in sync | Users can open Topics, rename, merge, export |
| 4 to 5 | **F5 legible proactivity**: "why this" on Today cards, intensity setting, per-source opt-in | Dismissal rate measured before and after |
| 6 to 10 | **F2 phase 1**: `relation`, `commitment`, `decision` tables; heuristic extraction from attendees, headers, action items and decisions; `entity_page` and `facts_changed_since` read model; sync row types | 100 labelled relations in the eval set; precision per predicate reported |
| 9 to 12 | **F6 entity pages**: person and project pages on both desktops with facts, open commitments, last touch, next event, citations | Entity pages reachable from search and from meeting attendees |
| 11 to 12 | **F7 design and F8 spec** written as `docs/superpowers/specs`, trigger vocabulary and tiers decided; **O-21 Limitless import** if the export format is obtainable | Specs approved; H2 starts on day 91 with no design debt |

What is deliberately not in the 90 days: the tool loop (F8), condition triggers (F7),
local extraction models (F10), speaker identity (F12), anything Cloud. Each needs either
the eval harness or the graph to exist first.

**Release cadence.** Three point releases (0.9.52 trust surface and eval, 0.9.53 Topics,
0.9.54 entity pages), each with a release note that states the eval numbers.

**Decisions needed from the owner in the window**

1. Topics and entities join content sync (recommended: yes).
2. The predicate set for relations (document 6 §6.1 proposes eleven).
3. Grader model and team key for the graded eval sets.
4. Whether O-21 is worth two days (only if a Limitless export sample is at hand).

## 8.2 Engagement design

The product is ambient, so the engagement question is not "how often do they open it" but
"does the capture keep running and does the memory earn a return visit." Doc 17's R-gaps
cover the first; this section covers the second and sets rules.

**Loops, in order of introduction**

1. **The return loop (H1 to H2).** Open the app after time away; "since you were away" lists
   decisions, promises to you and changed dates from the graph delta (F9), each with a
   source. The loop closes when the user acts on one item (opens a source, marks a
   commitment done, replies). Metric: return-brief action rate.
2. **The commitment loop (H2).** A promise is captured; a due-soon nudge appears once, in
   Today, with the source; completion is detected from later evidence or marked by hand.
   Metric: commitment closure rate and nudge dismissal rate.
3. **The correction loop (H1 onward).** The user renames a topic, merges two people,
   rejects a fact. Each correction improves the graph and is the strongest retention
   signal available (a user who corrects is a user who depends). Metric: corrections per
   active user per week.
4. **The routine loop (H2).** A condition-triggered routine produces output the user reads
   within an hour of the event that caused it. Metric: read-within-an-hour rate.

**Rules that keep proactivity welcome** (from document 3 §3.8)

- Nothing surfaces without a visible reason and a source.
- One nudge per commitment; one brief per return; no streaks, no badges, no "you have not
  opened Whimbrel in 3 days" email. The product is not an app that wants attention; it is
  a memory that is there when needed.
- Intensity is a user setting with three levels, default middle; the lowest level is
  "only when I ask."
- Every proactive surface can be turned off per source in one click from the surface
  itself.
- Email to the user's own address (live alerts) stays the only unprompted outbound
  channel until mobile push exists, and stays opt-out.

**Onboarding touchpoints added by this roadmap**

- Day 1: the trust surface is the first screen after permissions, not a settings pane.
- Day 3: the first Topics view when at least three topics exist ("Whimbrel has started to
  see shape in your week").
- Week 2: the first entity page suggested from the most frequent attendee.
- Week 3: the first commitment nudge, with a one-line explanation of where it came from.

## 8.3 Measurement plan

Whimbrel sends almost nothing home (document 1 §1.7: a weekly unique desktop id, platform
and version) and the roadmap keeps it that way. Product metrics are therefore computed on
the device and shown to the user, and optionally contributed as aggregate counts through
an opt-in that defaults off. That opt-in is itself a trust feature: the user sees exactly
the counts that would be sent before they are sent.

**On-device metrics, visible to the user in settings** (a "your Whimbrel" page):

| Metric | Definition | Why |
|---|---|---|
| Capture uptime | hours captured / hours awake, 7-day | the only north star for an ambient product |
| Memory size | items, meetings, entities, relations, commitments | shows compounding |
| Return-brief action rate | actions / briefs shown | engagement loop 1 |
| Commitment closure rate | done / (done + dropped + overdue) | loop 2 |
| Corrections per week | renames, merges, rejects | loop 3 |
| Routine read-within-an-hour | reads / runs with output | loop 4 |
| Approval prompts per week | queued Irreversible steps shown | approval fatigue ceiling (target under 10) |
| Agent pass rate | verify-turn success / runs | reliability |
| Cost per week by provider | from `usage_log` | cheap-by-default kept honest |
| Flagged injection spans | count and rate | firewall health |

**Opt-in aggregate counts** (never content, never identifiers beyond the existing weekly
id): the same table as weekly buckets. Enough to learn whether Topics are opened and
whether commitments close; not enough to learn anything about a person.

**Eval metrics in CI** (from F3): recall at 10, citation precision, "I don't know" rate on
the not-in-memory set, multi-session accuracy, extraction precision and recall per
predicate per tier, pass^1 and pass^4 on the routine suite, cost per successful routine.
Each release note carries the table.

**Success thresholds for the year**

- Capture uptime median above 70% among week-4 retained users.
- Week-4 retention (capture still running) up 15 points over the pre-H1 baseline once a
  baseline exists through the opt-in counts.
- Extraction precision above 0.8 on commitments and decisions; corrections under 5% of
  surfaced facts.
- pass^4 above 0.7 on the routine suite before F8 ships; zero unapproved irreversible
  actions, measured as a hard invariant in tests, not as a rate.
- Approval prompts per active user under 10 a week.

## 8.4 Product vision, twelve to twenty-four months

**Whimbrel is the memory that is yours.** It sits on your machines, hears your meetings,
reads your mail and calendar, watches the screen you choose to show it, and turns all of
it into a structure you can read: the people you work with, what each of you promised,
what was decided and on what evidence, what changed and when. It acts on that structure
through routines you wrote in plain language and can read line by line, within limits
you set, with a record you keep. It syncs between your devices without anyone in the
middle able to read it. It runs on the models you choose, cheap by default and local when
you want. And it is open source with a relay you could run yourself, so it will still be
yours if the company is not.

By the end of 2027 a user who has run Whimbrel for a year opens it after a week away and
sees, in one screen: the four decisions made without them and the sources, the three
things people promised them and the two they promised others, the one fact that changed
(a date, a price, a role), and a drafted reply for each, waiting for a tap. Nothing in that
screen came from a cloud. Nothing in it is a guess without a citation. And the same screen
is on their phone, decrypted there, with the same approvals.

The business follows doc 16 and doc 17 without change: the local core free and open, Cloud
for routines that run when the laptop is closed, Team for a shared project graph, Support
for organisations. The roadmap in this folder is what makes each of those worth paying
for: a Cloud runner is only valuable when routines are event-driven and agentic; a Team
space is only valuable when the shared thing is a graph, not a folder of notes.

**What would make this wrong.** If the eval harness shows that structure does not improve
answers on personal-scale data, the graph becomes a view, not a retrieval layer, and the
year's spine shifts to F7 and F8 over rows. If local extraction never reaches usable
precision, the graph is a frontier-key feature, which still fits decision 4. If users do
not correct or revisit entity pages, memory stays ambient and the roadmap collapses to
doc 17's releases plus the trust surface. Each of those is a measured outcome in the first
two horizons, not a belief to defend.
