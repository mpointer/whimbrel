# 2. Competitive intelligence (checked 2026-11-03)

Method: web search plus direct reads of vendor pricing and documentation pages. Each claim
carries a source and the date visible on it (a "last updated", a post date, or a copyright
year when nothing better exists). Tags: **V** verified on a product page or docs, **A**
announced in press or a blog, **E** experimental or beta, **S** speculative, **N** none
found, **U** unknown, **P** partial. Doc 17 §2 (2026-10-31) covers the same field from the
adoption and pricing angle; this report adds the agentic, memory and knowledge-graph
dimensions and refreshes the facts. Where the two disagree, this one is newer.

## 2.1 Direct competitors

**Granola.** Bot-free "AI notepad": device-audio capture, enhanced notes, chat, Recipes
(saved prompts), pre-meeting briefs. $1.5B Series C, March 2026
([SiliconAngle, 2026-03-25](https://siliconangle.com/?p=736110)). Pricing: Basic $0 with a
30-day history wall, Business $14 per user per month, Enterprise $35; desktop, iOS, Android
and Apple Watch on every tier; API on Business
([granola.ai/pricing](https://www.granola.ai/pricing)). Hosted MCP server on all plans
([blog, 2026-02-04](https://www.granola.ai/blog/granola-mcp)). Cloud transcription and
storage; no local model, no own keys, no screen memory, no self-hosting (**N**). Strengths:
polish, distribution, integrations. Gaps: cloud-only privacy posture, no context outside
meetings, briefs limited to meeting prep.

**anarlog** (formerly Hyprnote, then Char). MIT desktop notetaker from Fastrepl: device
audio plus mic, local SQLite, on-device transcription, own keys for OpenAI, Anthropic and
Deepgram, local models through Ollama or LM Studio, a CLI, an MCP server, webhooks. macOS
mature, Windows and Linux beta. Free with no limits locally; Pro $14 per month or $140 a
year for hosted transcription, cloud sync and sharing
([anarlog.so/about, updated 2026-10-15](https://anarlog.so/about/)). Char split off as a
separate macOS-only "agentic notepad"
([char.com blog](https://unsigned.char.com/blog/char-is-now-anarlog/)). No screen memory,
no routines, no end-to-end multi-device sync found (**N**). The closest open-source
neighbour on meetings.

**Littlebird.** "Ambient work memory": active-window text every few seconds through
accessibility APIs, bot-free meetings, chat over that memory, Routines (scheduled prompts,
5 on Basic and 100 on Plus, delivered by email), meeting prep briefs, Projects, a hosted
MCP server (Plus and up), 100-plus integrations with write actions (Gmail drafts but never
sends; Calendar create and edit; Notion, Todoist, GitHub and Linear writes). Mac and
Windows; phone apps query but do not capture. Memory encrypted at rest on AWS us-east;
"local-only processing isn't currently offered"; SOC 2 Type II. Basic $0, Plus $17 annual or
$20 monthly, Power $42.50 or $50, Pro from $100, Team from $17 per seat, Student $15
([littlebird.ai/facts, reviewed 2026-10-16](https://littlebird.ai/facts)). Speaker
diarization "in development" (same page, 2026-08-04). $11M seed, March 2026. The closest
feature-for-feature competitor; the difference is architecture (cloud memory, resold
inference, no self-host, no source).

**Limitless and Rewind.** Acquired by Meta, announced 2025-12-05
([TechCrunch](https://www.techcrunch.com/2025/12/05/meta-acquires-ai-device-startup-limitless/)).
Pendant sales ended that day; desktop and web recording discontinued; Rewind capture
disabled 2025-12-19; service withdrawn from the EU, UK and several other regions
([limitless.ai](https://www.limitless.ai/)). Out of the market; its users are a target
segment.

**Screenpipe.** 24-hour screen and audio capture (accessibility tree with OCR fallback),
local Whisper large-v3-turbo or Deepgram, speaker identification, Ollama or any
OpenAI-compatible model, an MCP server, a local REST API, "pipes" (scheduled agents written
as Markdown), optional end-to-end encrypted sync, timeline search. Mac, Windows, Linux; no
mobile. Licence moved from MIT to a source-available commercial licence
([README, screenshots dated 2026-10-02](https://raw.githubusercontent.com/screenpipe/screenpipe/main/README.md)).
As of 2026-08: Free; Basic $25 a month or $250 a year (unlimited history, 150 AI credits);
Business $50 per seat; Business Max $100; Ultra $200; lifetime licences withdrawn
([screenpipe.com/llms-full.txt](https://screenpipe.com/llms-full.txt)). Strengths: deepest
local screen memory, developer platform. Weaknesses: no longer open source, subscription
creep, no connectors with write actions found, no mobile.

**Otter.ai.** Bot-based, with vertical agents (sales, SDR, recruiting). Basic $0 (300
minutes a month, 20 chat queries), Pro $16.99 or $8.33 annual, Business $30 or $19.99,
Enterprise custom; an MCP server marked new on Basic; API on Enterprise
([otter.ai/pricing](https://otter.ai/pricing)). Cloud only; bot presence; metered chat.

**Fireflies.ai.** Bot notetaker with AskFred chat, AI apps and voice agents, a Chrome
extension that records without the bot
([fireflies.ai/pricing](https://fireflies.ai/pricing)); Free, Pro $10, Business $19,
Enterprise $39 annual per a third party dated 2026-01-09
([claap.io](https://www.claap.io/blog/fireflies-pricing)). No local processing or own keys.

**Fathom.** Free unlimited recording and summaries; bot-free capture in beta on Mac.
Premium $20 or $16 annual, Team $19 or $15, Business $34 or $25; an API and MCP
([fathom.ai/pricing](https://fathom.ai/pricing)). Cloud only; meetings only.

**tl;dv.** Bot notetaker with a "no bot required" option, multi-meeting reports, 5,000-plus
integrations ([tldv.io/features](https://tldv.io/features/)); Pro EUR 29 or 18 annual,
Business EUR 39 or 29 ([tldv.io/pricing](https://tldv.io/pricing/)); a third party logged a
top-tier rise to $98 in April 2026
([costbench](https://costbench.com/changelog/tldv-price-increase-2026-04/)). Agent and MCP
status unknown.

**Notion AI Meeting Notes.** Bot-free system-audio capture in the desktop app
([TechCrunch, 2025-05-13](https://techcrunch.com/2025/05/13/notion-takes-on-ai-notetakers-like-granola-with-its-own-transcription-feature));
included in Business at $20 per member and Enterprise, with Custom Agents at $10 per 1,000
credits and Workers in beta ([notion.com/pricing](https://www.notion.com/pricing)). Notes
land in the workspace; cloud only; meetings only.

**Microsoft Recall and Teams recap.** Recall: opt-in local snapshots on Copilot+ PCs, GA
2025-04-25, with an admin disable policy
([Microsoft Learn, updated 2026-02-03](https://learn.microsoft.com/en-us/windows/apps/develop/windows-integration/recall/)).
Microsoft promoted it as a 2026 AI PC reason
([Windows Latest, 2026-02-20](https://windowslatest.com/2026/02/20/microsoft-says-2026-is-the-moment-for-ai-pcs-touts-windows-11-recall-copilot-and-the-highest-standard-of-security))
while a report says it is re-evaluating the Windows AI strategy (**A/S**,
[TweakTown](https://www.tweaktown.com/news/109998/microsoft-is-reevaluating-its-windows-11-ai-strategy-might-even-remove-copilot-features/index.html)).
Recall has no chat, routines or sync. M365 Copilot: Business add-on $18 annual (list $21),
Business Standard with Copilot $23.50, Premium $32; agentic features metered by Copilot
Credits ([microsoft.com](https://www.microsoft.com/en-us/microsoft-365-copilot/pricing)).

**Google Gemini in Meet.** "Take notes for me" on Workspace Standard and up
([workspace.google.com/pricing](https://workspace.google.com/pricing)); in-person notes GA
2026-08-13
([Workspace Updates](https://workspaceupdates.googleblog.com/2026/08/take-notes-with-me-for-in-person-meetings-is-now-available.html?m=1)).
The Gemini app has Scheduled Actions (up to 10, 2025-06-06,
[blog.google](https://blog.google/products/gemini/scheduled-actions-gemini-app/)) and
Personal Intelligence memory across Gmail, Photos and Search (January 2026,
[Android Authority](https://www.androidauthority.com/google-gemini-personal-intelligence-rollout-3632287)).
Meet only; cloud only.

**Apple.** Call recording with transcript and summary into Notes since iOS 18.1, with a
spoken recording announcement ([support.apple.com/121583](https://support.apple.com/121583)).
No system-audio capture of third-party meetings, no routines.

**Zoom.** Core AI Companion free in paid plans; ZoomMate launched 2026-06-01 at $20 per user
with 2,200 pooled credits for agentic workflows; an AI Productivity Suite at $10
([No Jitter](https://nojitter.com/digital-workplace/zoom-launches-zoommate-and-ai-productivity-suite);
[zoom.com](https://www.zoom.com/en/products/ai-assistant/)). Cloud only.

## 2.2 Agent memory and personal AI

**Mem.** Note app with a Mem Agent that "checks in when something needs you", scheduled
routines, Gmail, Slack and Todoist connections, voice meeting capture. Free; Plus $9; Pro
$29 (1 routine, 1 connection) up to Pro 20x $199; everything metered
([get.mem.ai/pricing](https://get.mem.ai/pricing)); the Pro jump from $12 unlimited to $29
capped logged 2026-08-25 ([usagepricing.com](https://www.usagepricing.com/blueprint/activity/mem-2026-08-25-metered-tier-ladder)).
Cloud, no screen memory, no own keys.

**Dot.** Shut down 2025-10-05
([TechCrunch](https://techcrunch.com/2025/09/05/personalized-ai-companion-app-dot-is-shutting-down)).

**Personal AI.** Repositioned as an enterprise memory platform
([personal.ai/platform](https://www.personal.ai/platform)); no free plan as of 2026-08. Not
a desktop competitor.

**Mem0.** Apache-2.0 memory SDK with a self-hosted server; graph memory and "Dream"
consolidation on the Custom hosted tier only
([mem0.ai/pricing, 2026-11-02](https://mem0.ai/pricing);
[README](https://raw.githubusercontent.com/mem0ai/mem0/main/README.md)). A component, not a
rival.

**Letta.** Apache-2.0 stateful-agent framework; cloud Free, Pro $20
([docs.letta.com](https://docs.letta.com/letta-code/pricing)). Developer infrastructure.

**Zep and Graphiti.** Temporal knowledge graph with `valid_from` and `valid_to` edges;
Graphiti is Apache-2.0, the Zep self-hosted edition is retired; cloud Flex $125 a month
([getzep.com/pricing](https://www.getzep.com/pricing)). Infrastructure, and the reference
design for temporal personal graphs.

**Khoj and Pipali.** AGPL "second brain" over documents with custom agents, scheduled
research newsletters and local or hosted models; Khoj Cloud shut down 2026-04-15; Pipali is
a new desktop "AI coworker" with skills, routines, MCP and a sandboxed terminal
([README](https://raw.githubusercontent.com/khoj-ai/khoj/master/README.md);
[dev.to review](https://dev.to/jovan_chan_9500711396d4e6/khoj-review-2026-your-self-hosted-ai-second-brain-28mi);
[Product Hunt](https://www.producthunt.com/products/pipali)). No ambient capture.

**OpenClaw.** MIT self-hosted agent gateway reachable over 29 chat channels; memory of
people and projects; skills on ClawHub; browser, file and shell tools; local or hosted
models; a foundation, no paid tier; Windows support announced 2026-06-02, phone apps
2026-06-30 ([openclaw.ai](https://openclaw.ai/);
[DigitalOcean](https://digitalocean.com/resources/articles/what-is-openclaw)). No screen or
meeting capture. Strong overlap on the own-model, self-host ethos.

**Cluely.** Real-time on-screen answers during calls; Pro $19.99, Pro plus
"undetectability" $149.99 ([cluely.com/pricing](https://cluely.com/pricing)). Live
assistance, not memory.

## 2.3 Other open-source notetakers

**Meetily.** MIT community edition, Tauri, local Whisper or Parakeet, Ollama or cloud
summaries; Pro $10 per user for speaker id, MCP, CLI and webhooks
([README](https://raw.githubusercontent.com/Zackriya-Solutions/meeting-minutes/main/README.md)).
No memory beyond meetings. **Vexa.** Apache-2.0 meeting-bot API, self-hosted or $0.30 per
bot-hour ([GitHub](https://github.com/Vexa-ai/vexa)). **MacWhisper.** Local file
transcription, EUR 59 to 65 lifetime. **Krisp.** Bot-free notes with noise cancellation;
sources conflict on the free tier and price.

## 2.4 General-purpose substitutes

**ChatGPT.** Pulse, proactive overnight briefs, in preview on Pro from 2025-09-25
([TechCrunch](https://techcrunch.com/2025/09/25/openai-launches-chatgpt-pulse-to-proactively-write-you-morning-briefs));
no confirmed Plus rollout as of November 2026 (**E**). Memory and Gmail and Calendar
connectors opt-in. **Claude.** Memory on by default for individual plans, editable Topics
([release notes, 2026-08-25](https://support.claude.com/en/articles/12138966-release-notes?lang=en));
Cowork desktop agent GA 2026-04-09 with projects, scheduled tasks and per-folder memory on
macOS and Windows ([guide, 2026-07-02](https://www.betterclaw.io/blog/claude-cowork-complete-guide)).
No ambient capture. **Microsoft 365 Copilot** and **Workspace Gemini**: see 2.1.

## 2.5 Competitive matrix

Whimbrel's column is from the verified inventory in document 1.

| Capability | Whimbrel | Granola | anarlog | Littlebird | Screenpipe | Otter | Fathom | Notion | Recall/Teams | Gemini Meet | Zoom |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Bot-free capture | V | V | V | V | V | N | E (Mac) | V | V (native) | V (native) | V (native) |
| On-device transcription | V | N | V | N | V | N | N | N | N | N | N |
| Screen memory | V | N | N | V | V | N | N | N | V (Recall) | N | N |
| Chat with citations | V | V | U | V | V | V | V | V | V | V | V |
| Proactive briefs and routines | V | P (briefs) | N | V | V (pipes) | P | N | N | N | P (scheduled) | P |
| Autonomous agents | N (approval-gated drafts only) | P (Recipes) | N | P | P | V (vertical) | N | V (Agent) | V (metered) | V | V (ZoomMate) |
| Knowledge graph or entity memory | P (built, unwired) | N | N | U | N | N | N | N | N | A (Personal Intelligence) | N |
| Cross-device end-to-end sync | V | N | P (Pro) | N | V (optional) | N | N | N | N | N | N |
| Self-host or open source | V (Apache) | N | V (MIT) | N | P (source-available) | N | N | N | N | N | N |
| Own model keys | V | N | V | N | V | N | N | N | N | N | N |
| Connectors with write actions | P (2, gated) | P | P (webhooks) | V | N | V (CRM) | V (CRM) | V | V | V | V |
| Local MCP server | V (7 read tools) | N (hosted) | V | N (hosted) | V | N (hosted) | N (hosted) | N | N | N | N |
| Mobile | E (read, ask) | V | N | P (query) | N | V | V | P | V | V | V |
| Team sharing | A | V | P | V | V | V | V | V | V | V | V |
| Pricing model | Open source + Cloud, Team, Support | Freemium seat | Open source + $14 | Freemium credits | Source-available + credits | Freemium minutes | Freemium seat | Bundle + credits | Bundle + credits | Bundle | Bundle + credits |

| Capability | Mem | Mem0 | Letta | Zep | Khoj/Pipali | OpenClaw | Cluely | Meetily | ChatGPT | Claude |
|---|---|---|---|---|---|---|---|---|---|---|
| Bot-free capture | V (voice) | N | N | N | N | N | V (overlay) | V | N | N |
| Screen memory | N | N | N | N | N | N | P (live) | N | N | N |
| Proactive briefs and routines | V (Pro) | N | N | N | V | A | N | N | E (Pulse) | V (Cowork tasks) |
| Autonomous agents | V | N | V | N | V (Pipali) | V | N | N | V | V (Cowork) |
| Knowledge graph | N | V (Custom tier) | N | V | N | P (file memory) | N | N | P (memory) | P (Topics) |
| End-to-end multi-device sync | N | N | N | N | N | N | N | N | N | N |
| Self-host or open source | N | V | V | P (Graphiti) | V | V | N | V | N | N |
| Own model keys | N | V | V | V (Ent.) | V | V | N | V | N | N |
| Local MCP server | N | U | U | V (cloud) | V | V | N | V (Pro) | N (client) | N (client) |

## 2.6 What nobody does well yet

1. **Local-first ambient memory with proactive routines.** Littlebird has routines and
   connectors but stores memory on AWS with no local mode; Screenpipe is local but has no
   gated write actions and left open source; anarlog is local and MIT but meetings only.
   No one combines all three.
2. **End-to-end encrypted multi-device sync of a personal memory.** Only Screenpipe offers
   optional end-to-end sync; every cloud product syncs through vendor-readable storage;
   Limitless, the one consumer product with cross-device recall, is gone.
3. **Own keys at the consumer tier.** Every mainstream product resells inference,
   increasingly through credits (Littlebird packs, Mem's ladder, Zoom's 2,200 credits,
   Notion's $10 per 1,000, Copilot Credits). Only open-source projects let users bring
   keys.
4. **Entity or knowledge-graph memory in an end-user product.** Graph memory lives in
   developer infrastructure (Zep, Mem0's Custom tier). No notetaker or ambient assistant has
   shipped a verified people, project and decision graph; Gemini Personal Intelligence and
   Claude Topics are the closest, both cloud-side.
5. **Speaker attribution in bot-free capture.** Littlebird's diarization is "in
   development"; Meetily paywalls it; Notion reviewers report weak labels. Whimbrel has
   on-device diarization but resets speaker identity each meeting.
6. **Approval-gated write actions with a local audit trail.** Littlebird's "drafts, never
   sends" is the right instinct, enforced in its cloud. No local-first product offers gated
   writes with a user-held record.
7. **Durability.** Dot, Rewind and Limitless desktop, Khoj Cloud and Zoom's Custom AI
   Companion all disappeared within 18 months. Open source plus a self-hostable relay is a
   credible answer to lock-in fear, and nobody else can make it.

## 2.7 Pricing landscape

- Freemium per seat at $14 to $20 for individuals is the band: Granola $14, anarlog $14,
  Littlebird $17 to $20, Fathom $16 to $20, Fireflies $10 to $19, Otter $8 to $17, Notion
  $20, Cluely $20, ChatGPT and Claude $20.
- Credits are the new meter and the new complaint: Littlebird packs, Mem $29 to $199, Zoom
  2,200 credits at $20, Notion $10 per 1,000, Copilot Credits, Screenpipe 150 to 1,600 per
  tier. A flat own-keys product is a clean counter-position.
- 2026 price rises: Mem $12 unlimited to $29 capped (August), tl;dv top tier to $98
  (April), Screenpipe lifetime to $25 to $200 monthly (August), Zoom's add-on to $20 then
  ZoomMate at $20.
- Bundled suites (Workspace $14 to $22 with Gemini; M365 with Copilot $23.50 to $32) make
  good-enough meeting notes free at the margin for their customers.
- Open source monetizes through hosted sync or transcription at $10 to $14 (anarlog,
  Meetily) or enterprise; Screenpipe's relicensing shows the pressure on capture tools.
- Implication: doc 17's Cloud at $12 to $15 sits with anarlog Pro and Meetily Pro; Team at
  $14 to $20 per seat sits with Granola Business and Littlebird Team; Support is
  uncontested among desktop open-source peers.

Evidence gaps not closed: tl;dv agent and MCP status, Fireflies' official prices, Zoom's
current add-on page, Pulse tier expansion, Microsoft's definitive Recall roadmap, whether
Mem0's graph memory remains in the open-source package.
