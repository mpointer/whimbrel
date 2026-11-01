# 17 — Competitive gaps, adoption roadmap, and monetization

**Status:** proposal for decision, 2026-10-31. Written against the tree at 0.9.43 and the
decisions in [14](14-pricing-proposal.md), [15](15-mobile-app-design.md) and
[16](16-open-core-commercial-model.md). It does not reopen the five decisions of 2026-10-29
(open source, no token resale, Cloud and Team as the two lines, independent business). It
asks what to build, in what order, so that those lines have customers.

**Sources.** The code (`core/`, `apps/`, `server/`, `website/`), docs 01–16, and a
competitor pass on 2026-10-31 (public pricing pages, help centers, app-store listings,
changelogs, and user threads on Reddit, Hacker News and X). Competitor facts carry a date;
anything unverified says so.

---

## 0. The honest summary

Whimbrel has more machinery than any competitor in its price range and fewer users than
all of them. The tree holds a Rust core of about 87,000 lines with 1,800 tests, three
shells, an E2E-encrypted sync relay, seven routines, three connectors, a local MCP server,
multilingual transcription and search, skins, and seven languages. Against that, the
hosted relay has one account and the website asks visitors to join a waitlist for the
features people say they want.

The gaps that matter are not features. They are:

1. **Time to first value.** Granola delivers a meeting summary before the user has read
   its onboarding. Whimbrel asks for five permissions, a 470 MB search model, an Ollama
   install or an API key, and a Google sign-in for sync before the first "aha". Every
   competitor that grew fast had one tap between install and the first result.
2. **Default quality on the critical path.** Whimbrel's default transcription model is
   whisper `tiny.en` (`base` for multilingual). Granola, Fathom and Otter use cloud
   speech models that are plainly better; users judge the whole product by the transcript.
   Local-first does not have to mean small-model: Apple's on-device `SpeechAnalyzer`
   (macOS 26) and Parakeet/whisper-large-v3-turbo on Apple silicon and modern x86 are both
   within budget and neither is wired in.
3. **The phone is read-only and silent.** Granola's 14,500 iOS ratings say where demand
   is: one-tap in-person recording. Whimbrel's iPhone app reads and asks but cannot record
   and cannot be pushed to. M4.3 and M4.4 are the two mobile milestones that create
   reasons to pay, and they are also the ones not started.
4. **Nothing leaves Whimbrel as a file.** Obsidian's whole position is "your notes are
   Markdown files you own." When Granola encrypted its local cache in March 2026 and broke
   the Obsidian export tools, the backlash was loud enough that its CEO called it a
   mistake. Whimbrel's Connected Topics already export Obsidian-style Markdown with
   stable `whimbrel_id` frontmatter, but meetings, the thing people most want in a vault,
   do not, and no import from Granola, Otter or a transcript file exists.
5. **The agent half of the pitch is a README sentence.** The product vision sells "notices
   and acts". Routines notice; the only acts are a draft email and a calendar block behind
   an approval. M2.3 (computer use, skills) has not started and should not start until
   the four items above are closed, but the pitch should stop promising it.

The monetization plan in doc 16 is right about *what* to charge for: servers the user does
not want to run and the organization around a team. It is silent on *why someone would
pay this year*, which is the question this document answers. The answer is **cloud
routines plus push**, built as a hosted twin of the primary-device routine engine that
already exists, sold as "Whimbrel keeps working when your laptop is closed," priced at or
above the current $12/$15 anchor, with the free product kept whole and the hosted relay cap
left exactly where doc 16 put it.

The roadmap below is three releases of adoption work before any billing code is written,
because a paid plan launched to one account is a line of code, not a business.

---

## 1. What Whimbrel is today (from the tree, 2026-10-31)

Written so the gap tables in §3 can refer to it. Where a line contradicts an older doc,
the tree wins.

| Area | Shipped at 0.9.43 | Not shipped (claimed or planned somewhere in docs) |
|---|---|---|
| **Capture** | Active-window text via Accessibility (Mac) and UIA (Windows), OCR fallback, simhash dedupe, per-app exclusions, password-manager auto-exclusion, credential redaction before storage (`core/memory/redact.rs`), lock/sleep gating | Timed pause ("pause for 15 min"), category exclusions (banking, health), per-URL exclusion patterns, delete-last-hour quick action |
| **Meetings** | Calendar, heuristic and Safari-Meet detection; system audio and mic; chunked live transcript during the meeting; diarization (sherpa-onnx) with speaker-count hints from Zoom/Meet/Teams UI; multilingual detection; slide and face snapshots with OCR; back-to-back split; dropped-call rejoin; recording safeguards (6 h cap, disk limits, crash recovery); structured editable summary (TL;DR, decisions, action items, open questions); regenerate with any model; custom instructions; meeting prep brief; per-meeting Ask; Join button; Away Mode (joins Zoom for you, beta pending) | Chrome/Edge/Arc in-browser detection; "For you" personalised section; cross-meeting speaker identity; import of external transcripts; share link; Markdown export of a meeting; calendar write-back of action items |
| **Memory** | SQLite + FTS5 + sqlite-vec; multilingual embeddings (paraphrase-multilingual-MiniLM-L12-v2, downloaded at first run); L1 items table; L2 episodes and Gmail threads with rule-based status; L3 profile facts with daily suggestions; Connected Topics (people, orgs, topics) named by the model; retention per class with media compression | Entity filtered views in the UI; weekly or per-project rollups surfaced; SQLCipher at rest (NFR-PRV-2 open on both platforms); full export (JSON + Markdown + audio) |
| **Chat** | Grounded Ask with citations, streaming on all providers; conversations with star/rename; prompt library and chips; Ollama, Anthropic, OpenAI, Gemini, OpenRouter; cheap-first routing; spend caps; usage screen; local-only mode; "what left my machine" ledger | Conversations do not sync; no attachments; no overlay or global hotkey; no command bar; no projects; no per-answer feedback; no "max effort" toggle |
| **Routines** | Daily brief, EOD recap, weekly digest, focus time, reply priorities, meeting prep, Away follow-up; cron times; primary-device model with failover; approvals inbox; per-routine auto-approve; per-routine model; run history synced | Natural-language routine authoring; event triggers ("when a meeting with X ends"); engagement-aware pause; any routine running while every device is asleep |
| **Connectors** | Gmail (search, read, draft, send, incremental `historyId` sync), Google Calendar (read, create, `syncToken` sync), Outlook 365 (read mail and calendar), multi-account | Outlook write and delta sync; Slack, Teams chat, Linear, Todoist, Notion, Drive, GitHub, Zoom recordings (all named in FR-MCP-2); user-registered third-party MCP servers; remote HTTP MCP client transport |
| **MCP server** | Seven read tools over localhost streamable HTTP with per-client scoped bearer tokens (`memory`, `meetings`, `transcripts`); copy-paste config for Claude Desktop and others | Write tools; cross-device merged answers; remote bridge for phone or other machines |
| **Sync** | E2E (X25519 device keys, wrapped group key, ChaCha20-Poly1305); per-account Turso database; Cloudflare Worker control plane; enroll, approve, name, revoke, swap, reset, delete; settings sync; content sync of meetings, items, routine runs, meeting prep, skins; row compaction; free limit of one computer plus one phone enforced server-side; waitlist with four "wants"; 180-day inactivity deletion with warning email | Conversations, note drafts and action-item edits do not sync; no backup or restore; no push; CRDT merge for mutable objects (last-writer-wins today); relay open sign-in is deployed but gated by `OPEN_SIGN_IN` |
| **Shells** | Mac (SwiftUI, 38k lines), Windows 11 (Tauri, 16k lines TS plus Rust), iPhone (TestFlight-ready: Today, Ask, Meetings, Routines, device approval, dictation, read-aloud); signed updaters on both desktops; skins by chat with sync; 6 Today layouts; es, fr-CA, fr-FR, de, pt-BR, en variants | Phone recording (M4.4), push (M4.3), widgets and App Intents (M4.5); Android; Intel Macs; Windows 10; ARM64 Windows |
| **Agent** | `core/agent` defines two approval-gated write actions (send email, create event) | Computer-use driver, guardrail engine, kill switch, audit log, skill format, skill library (all of FR-AGT) |
| **Commerce** | Website with Free, Cloud (planned), Team (planned), Support; compare pages; lead capture; "no telemetry" | Billing, entitlements, a plan on the account, any paid feature |

Three things in that table are better than anything a competitor ships and should be said
out loud: credential redaction before storage, the scoped local MCP server, and the
primary-device routine model with cross-device approvals. Two things are worse than every
competitor and gate adoption: the default transcription model and the setup path.

---

## 2. The field (checked 2026-10-31)

Prices are USD per user per month, annual billing unless noted, read from each vendor's
own pricing page on the date above unless marked *[unverified]*, which means the figure
came from a third party or from conflicting sources. Complaint themes are from public
threads and reviews, weighted by recurrence; none of it is a survey.

### 2.1 Granola

The category leader for bot-free meeting notes, and the product most Whimbrel visitors
will have tried first.

- **Plans.** Basic $0: unlimited meetings, AI chat, shared folders, templates, Slack, a
  **30-day history wall** (older notes are locked, not deleted, and unlock on upgrade),
  MCP limited to the last 30 days of personal notes. Business **$14** (monthly billing
  only): unlimited history, "advanced thinking models", Notion, HubSpot, Attio, Affinity
  and Zapier, MCP over workspace notes, API, admin. Enterprise from **$35**: SSO at 50+
  seats, SCIM, auto-deletion, training opt-out by default, audit API, HIPAA, Salesforce.
  There is no individual paid tier; the free-to-paid lever is the history wall.
- **Platforms.** Mac, Windows (June 2025), iPhone (May 2025), Android (July 2026), Apple
  Watch (July 2026). No Linux. Share links and Spaces are the web presence; no full web
  client was found.
- **Shipped 2025–2026.** Granola 2.0 with shared team folders and a $43M Series B (May
  2025); Recipes, saved prompts run from Chat (Sep 2025); Sign in with Microsoft and
  partial transcript deletion (Jan 2026); official MCP server (Feb 2026); Spaces, personal
  and enterprise APIs and a **$125M raise** (Mar 2026); agentic Chat with inline citations
  (Apr 2026); Briefs, a pre-meeting summary on Mac and Windows (May 2026); 32 languages
  (Sep 2026). Nothing resembling scheduled routines; Zapier is the automation path.
- **How it works.** Audio streams from the microphone and system audio to Granola's
  transcription provider; cached audio is deleted after transcription. Cloud summaries.
  Trains on anonymised data by default with a one-click opt-out. SOC 2. No local or
  offline mode, no end-to-end encryption, no bring-your-own-key.
- **What people complain about.** The 30-day wall, introduced with the February 2026
  rebrand. **In March 2026 Granola encrypted its local cache**, which broke the community
  export tools people used to get notes into Obsidian and elsewhere; an a16z partner
  criticised it publicly and the CEO acknowledged the mistake, but a July 2026 build
  closed the remaining workarounds. MCP exposes 30 days and no transcripts; CSV export is
  the only full-history path. Share links default to public. **Chamberlain v. Granola**, a
  proposed class action filed 2026-07-30 in the Northern District of California, alleges
  that bot-free recording captures non-consenting participants under California's
  all-party consent law. Notes are "siloed"; non-English quality is uneven.
- **Does not do.** Screen or work memory outside meetings, local models, your own key,
  routines, self-hosting.

### 2.2 Littlebird

Closest in shape to Whimbrel, and the comparison the website already makes.

- **Plans.** Basic $0, no end date: 25 meeting notes total, 5 routines, daily credits,
  Gmail, Calendar, Drive and Outlook, mobile chat. Plus **$20 monthly / $17 annual**:
  unlimited notes, 100 routines, monthly credits, Max mode, image generation, MCP. Power
  $50 / $42.50 with 2.5× credits. Pro from $100. Team from $17 per seat; Enterprise custom;
  Student $15. One credit pool with a reset date.
- **Platforms.** Mac and Windows desktop (screen and meeting capture), iPhone, Android, a
  **web app** at app.littlebird.ai, iMessage, and WhatsApp in beta. Mobile is chat,
  routines and notifications only. (Doc 08's July inventory said "no web app"; that is no
  longer true.)
- **Features.** Active-window text memory, bot-free meetings, Routines (saved prompts on a
  daily, weekly or monthly schedule; the closest analogue to Whimbrel's), Chat, the Hover
  overlay, follow-up drafts that never auto-send, integrations with Gmail, Google
  Calendar, Drive, Docs, Outlook, Slack, Notion and Linear, a hosted MCP server over OAuth,
  Projects, Assistant Notes, a command bar, chat sharing.
- **How it works.** Cloud, by design: "for local-only systems, Littlebird might not be the
  right fit." Encrypted in transit and at rest on AWS US East, not end to end; context is
  sent to AI providers; retained until deleted. SOC 2 Type II report dated September 2026.
  Pause, per-app, per-site and category exclusions; password managers always excluded;
  banking and adult sites excluded by default. Training policy not stated.
- **Company.** $11M seed led by Lotus Studio, announced 2026-03-24; founders ex-Sentieo;
  about weekly mobile releases.
- **What people complain about.** The always-watching feeling; answers with wrong dates
  and timelines; context quality that depends on what each app exposes; polish
  ("promising, not polished", 2.6 of 5 in one poll against 4.8 on G2 from three reviews);
  Screen Recording and Accessibility permissions.

Whimbrel does not win a feature race against a funded team shipping weekly. It wins where
Littlebird has said in writing it will not go: local, end to end encrypted, your own
models.

### 2.3 Obsidian

Not a competitor. The standard for what a local-first product owes its users, and the
clearest precedent for doc 16's business model.

- **Plans.** The app is free for personal and commercial use; a $50 per user per year
  commercial licence is optional and adds nothing. **Sync Standard $4 annual / $5
  monthly**: 1 GB, one vault, one month of version history. **Sync Plus $8 / $10**: 10 GB
  (expandable to 100), ten vaults, twelve months of history. AES-256 end-to-end encrypted,
  shared vaults, Mac, Windows, Linux, iOS, Android and a headless server, no stated device
  limit. Publish $8 / $10 per site.
- **Ecosystem.** More than 8,000 community plugins and themes; roughly 150M plugin installs.
  Plugins may not include telemetry. 1.13 added a Settings search API and a SecretStorage
  API for plugin credentials.
- **Recent.** Bases (database views over notes; kanban in 1.14, October 2026), the
  Obsidian CLI (2026, "control Obsidian from your terminal"), the Web Clipper, Canvas on
  the open JSON Canvas format. **No first-party AI**, and the roadmap says nothing about
  it; the CEO published official Claude Skills for working with vaults instead.
- **Why people love it.** Plain Markdown files, no lock-in, no telemetry, no account
  required, sub-second launch, keyboard-first (Cmd+P commands, Cmd+O quick switcher), a
  scriptable URI and CLI, and a community that fills every gap.

Why it matters to Whimbrel, in order:

1. **Business-model proof.** Everything local is free; the server you run is the product.
   Obsidian's price also sets the ceiling for "encrypted sync with history and nothing
   else": $4 to $8. A Cloud plan at $12 to $15 has to include something Obsidian Sync
   cannot, which is a server-side agent.
2. **The file is the API, and the demand is proven.** When Granola encrypted its cache, the
   Granola-to-Obsidian plugins broke and people were angry enough that an investor said
   so in public. Nobody in the meeting-notes category writes notes into the user's vault
   as a first-class feature. `core/api/topics_export.rs` already writes Obsidian-style
   Markdown for topics, people and organisations with `whimbrel_id` frontmatter and a
   user-notes block that survives rewrites. Extending it to meetings and briefs is the
   single highest-leverage integration Whimbrel can ship, and most of it exists.
3. **UX standard.** Skins are Whimbrel's themes; MCP and the planned skill format are its
   plugins; the command bar, quick switcher and CLI are what is missing.

### 2.4 The rest of the field

| Product | What it is (2026-10-31) | Price | Local processing | Windows | Routines | Multi-device |
|---|---|---|---|---|---|---|
| **Limitless / Rewind** | Acquired by Meta. Pendant sales stopped 2025-12-05; desktop and web recording disabled; Rewind capture disabled 2025-12-19; accounts outside the US deleted. Effectively gone | n/a | No | Was | No | Was cloud |
| **Microsoft Recall** | Copilot+ PC screenshot timeline, opt-in, Windows Hello gated, snapshots stay on device; disabled by default on managed devices. No meetings, no routines | Hardware | Yes | Only | No | No |
| **Screenpipe** | Ambient screen and audio capture with "pipes" (scheduled workflows), local MCP. Moved from MIT to a source-available licence. Free for one device; Basic $21 monthly or $250 a year; Business $42 per seat; Enterprise for commercial source builds | $0 / $21 / $42 | Capture yes; AI by tier | Yes | Yes (Basic+) | Business+ |
| **Anarlog** (ex-Hyprnote) | MIT, local-first bot-free notes with Ollama or your key; the closest open-source competitor named in doc 16. Lite $8, Pro $25 for hosted sync and sharing *[vendor blog]* | $0 / $8 / $25 | Yes | Yes | No | Paid |
| **Meetily** | MIT community edition, local Whisper plus Ollama or a key; Pro about $10 for diarisation, templates, team deploy *[unverified]* | $0 / ~$10 | Yes | Yes | No | No |
| **MacWhisper** | Whisper on-device, one-time €59–65, BYOK in Pro | One-time | Yes | No | No | No |
| **Otter.ai** | Bot-based. Free 300 min a month, 30 min per conversation, 25 conversations of history; Pro $8.33 annual / $16.99 monthly; Business $19.99 / $30; Enterprise with 100-seat SSO minimum | $8.33 / $19.99 | No | Web | Enterprise | Cloud |
| **Fireflies** | Bot-based plus Chrome extension. Free with limited AI summaries; Pro $10 / $18; Business $19 / $29; Enterprise $39 | $10 / $19 / $39 | No | Web | Zapier, API | Cloud |
| **Fathom** | Free unlimited recordings and summaries; bot-free capture in beta on Mac only. Premium $16 / $20; Team $15 / $19; Business $25 / $34; API and MCP | $16 / $25 | No | Not stated | Zapier, Make | Cloud |
| **tl;dv** | Free unlimited recordings, AI notes on the first 10 meetings then the first 10 minutes, 3-month retention. Pro €18 / €29; Business €29 / €39 | €18 / €29 | No | Yes | Workflows | Cloud |
| **Notion AI Meeting Notes** | Bot-free system-audio capture in the Mac and Windows desktop app, cloud transcription. Needs Notion Business ($20 / $24) or Enterprise | $20 | No | Yes | Notion automations | Cloud |
| **Apple** | Notes recording with live transcription; iPhone call recording with summary on Apple Intelligence devices, not in the EU. The on-device `SpeechAnalyzer` API in iOS 26 and macOS 26 benchmarks faster than Whisper. No system-audio meeting capture on the Mac | Included | Yes | No | No | iCloud |
| **Google Meet Gemini notes** | "Take notes for me" on Workspace Business Standard ($14) and up, and Google AI Pro ($19.99). Meet only | Included | No | Web | No | Drive |
| **Zoom AI Companion** | Included in paid Workplace plans; a renamed, extended add-on priced $10–$20 *[conflicting, unverified]*. Zoom only | Included | No | Yes | Limited | Cloud |
| **Krisp** | Noise cancellation plus bot-free notes. No free plan (7-day trial); Core $8 / $16; Advanced $15 / $30; on-device transcription only at Enterprise | $8 / $15 | Enterprise only | Yes | No | Cloud |
| **Cluely** | Real-time on-screen meeting assistant, overlay first. Tiers conflict across sources ($0 / $19.99 / $149.99 "undetectability" in one) *[unverified]* | ~$20 | No | Yes | No | Cloud |
| **Notta** | Free 120 min a month; Pro $8.17 annual / $13.99 monthly; Business ~$16.67 | $8 / $17 | No | Web | No | Cloud |
| **ChatGPT Record mode** | Mac desktop only, Plus and above; system-audio recording into ChatGPT memory; scheduled Tasks exist separately | $20 | No | No | Tasks | Cloud |
| **Mem** | AI notes; Pro $12; Proactive $99 for an agent in Slack, iMessage and WhatsApp | $12 / $99 | No | Web, Mac | Proactive tier | Cloud |
| **Reflect** | $10 flat, no free tier, end-to-end encrypted notes with built-in AI | $10 | No | Web, Mac, iOS | No | E2EE cloud |
| **Capacities, Tana, Saner.ai** | PKM and planning apps adding meeting agents. Tana Pro about $20–$30 with botless meeting agents; Saner.ai bills external models to your own key *[unverified]* | $10–$30 | No | Mostly web | Some | Cloud |
| **Khoj** | AGPL personal AI over your files, self-hosted. Khoj Cloud sunset 2026-04-15 | $0 | Yes | Docker | Automations | Self-host |
| **Dayflow** | MIT Mac timeline from screen frames, local models or your own key; standup drafts. Windows on a waitlist | $0 / Pro | Yes | No | Drafts | No |
| **Hermes Agent, OpenClaw** | Open-source agent frameworks with skills, cron and messaging gateways, 100k+ stars each; neither is a memory or meeting product | $0 | Model-dependent | Yes | Yes | n/a |
| **Raycast AI** | Launcher with AI, notes and extensions; Pro $8 / $10, Pro+ $20, a shift toward AI credits *[unverified]*; Windows in public beta | $8 / $20 | No | Beta | No | Pro sync |

Three observations:

1. **Every tool that reached a mass audience did one thing in one tap first**: Granola a
   meeting, Raycast a command, Obsidian a note. Whimbrel's first tap is a permissions
   checklist.
2. **The only screen-memory survivors are local.** Rewind and Limitless are gone; Recall
   survived by going on-device and opt-in; Screenpipe survived by charging; Dayflow is
   local and free. Littlebird is the one cloud screen-memory product still growing, on
   venture money. Whimbrel's text-only, on-device stance is the market's conclusion, not a
   bet.
3. **Nobody combines ambient memory, bot-free meetings, routines and end-to-end sync on
   both desktops.** Littlebird is close and chose the cloud. Anarlog is local and chose
   meetings only. That gap is real and it is Whimbrel's.

### 2.5 What gates paid plans, and what converts

From the pricing pages (Circleback's comparison of 2026-10-16 agrees):

- **Price bands.** Entry individual $8 to $18; team $19 to $30; enterprise $35 to $39.
  Screen-memory products price higher: Screenpipe $21 / $42, Littlebird Power $42.50 and
  Pro $100, Mem Proactive $99.
- **History walls** (Granola 30 days, Circleback 30 days, tl;dv 3 months, Otter 25
  conversations) are the strongest lever and the one doc 16 rules out, correctly: a
  local-first product cannot expire a file on the user's own disk and stay honest.
- **Minute and credit caps** (Otter, Notta, Littlebird) convert and are resented; the
  complaint threads are mostly about running out mid-day.
- **AI feature counts and "better model" tiers** (Fathom, tl;dv, Granola thinking models,
  Littlebird Max mode) are paid everywhere. Whimbrel gives the model choice away with the
  user's own key; that is a reason to adopt, not to pay.
- **Integrations, API and MCP** are paid almost universally. Whimbrel's are free and must
  stay so (doc 16).
- **Team and admin** is where the seat money is, and it needs a product for the second
  person first.
- **Device count and encrypted sync** (Obsidian) converts a committed base at a low price
  and very low churn. It is the only lever Whimbrel's relay already enforces.
- **On-device processing is itself sold as an enterprise feature** (Krisp). Whimbrel
  ships it free to everyone, which is the enterprise conversation doc 16 wants.
- **Working while you are away** (bots join without you; Littlebird routines run in the
  cloud) is the lever nobody prices separately and the one Whimbrel's architecture can
  sell honestly.

No mainstream cloud notetaker offers bring-your-own-key or a local-model tier; it exists
only in indie and open-source tools (Anarlog, Meetily, MacWhisper, Dayflow, Khoj,
Saner.ai). Open-source monetisation in this category has gone three ways: Screenpipe
relicensed and charges $21 to $42; Anarlog sells hosted sync and sharing on top of a free
local app at $8 and $25; Khoj shut its hosted tier. No vendor publishes conversion rates.

### 2.6 Verification notes

Granola, Littlebird and Obsidian prices were read from their pricing pages on 2026-10-31
and agree with doc 14's 2026-10-16 check except that Littlebird's monthly Plus price is
$20 (doc 14 quoted only the $17 annual figure). Not verified this session: the Zoom
add-on price, Cluely's tiers, Littlebird's Windows ship date and training policy, Dayflow
Pro's price, Meetily's and Anarlog's paid tiers (vendor sources only), and any primary
Reddit or Hacker News thread text (sentiment above is secondhand). Re-check before any
public comparison page quotes a number.

---

## 3. The gaps

Two lists: what blocks someone from adopting Whimbrel in the first week, and what blocks
someone from staying and paying. Each line says what exists in the tree to build on.

### 3.1 Adoption gaps (first hour, first week)

| # | Gap | Who does it better | What exists to build on | Size |
|---|---|---|---|---|
| A1 | **First run asks for everything before giving anything.** Five TCC prompts, a 470 MB model, a provider key or Ollama, sign-in | Granola: one permission, first summary in the first meeting | `OnboardingView` is a checklist in Settings, not a first-run flow, and nothing outside Settings requests a permission; `PermissionsManager` knows which capability each grant unlocks; `api::models` downloads in the background, but the Mac prompts first | M |
| A2 | **Default transcript quality.** `tiny.en` by default; `base` multilingual. Every competitor's cloud model is better, and the transcript is the product | Granola, Fathom, Otter (cloud); Apple (on-device `SpeechAnalyzer`) | `core/transcribe` is a concrete `Transcriber` over whisper.cpp, so a backend trait has to be introduced first; `MeetingModelsCard` already lets users pick; `api::models` handles downloads. Add `SpeechAnalyzer` as a backend on macOS 26 and offer `large-v3-turbo` / Parakeet downloads on capable hardware, with the live chunker already proven | M |
| A3 | **No model, no chat.** A user with no key and no Ollama gets an empty Ask | Granola: works out of the box (they pay); Obsidian: no AI, no expectation | `api::routing` cheap-first chain; the guided key on-ramp shipped in #428. Add a one-click local model download (a 3–4B instruct GGUF via llama.cpp or a bundled Ollama launcher) so Ask works with zero accounts, slowly | M |
| A4 | **Nothing to search on day one.** Screen memory needs days to be useful; meetings need a meeting | Granola imports nothing either, but shows value in the first call | Import: `.vtt`/`.srt`/`.txt` transcripts, Granola and Otter exports, Apple Notes recordings, Zoom cloud recordings via connector, as first-class `meeting` rows (FR-MEM-8). Import of a Markdown folder (an Obsidian vault) as `items` is the same ingest path | M |
| A5 | **No in-browser meeting detection beyond Safari.** Most Meet users are on Chrome | Granola detects every platform | The `AXEnhancedUserInterface` decision in roadmap §Out-of-scope; on Windows the UIA address-bar read already exists for Edge/Chrome | S |
| A6 | **The app is not where the user is.** No overlay, no command palette, no quick switcher, no global hotkey on Mac | Littlebird Hummingbird; Raycast; Obsidian Cmd+P / Cmd+O; Cluely | Windows has `tauri-plugin-global-shortcut` available; the Mac has none. `assemble_context` is the one call an overlay needs; the live transcript service exposes the running meeting | M |
| A7 | **Chat does not follow you.** Conversations are device-local; the phone cannot resume a Mac thread | Littlebird, ChatGPT | `core/sync-engine` content rows; `conversation` is the missing row type (doc 15 §3 "Later") | S |
| A8 | **Meetings stay inside Whimbrel.** No Markdown export, no share link, no "open in Obsidian" | Granola (share links, Notion, Slack), every bot tool | `topics_export.rs` writes Obsidian-style Markdown with stable ids and preserved user notes; `MeetingSummary` is structured JSON. Export meetings and briefs into the same `Whimbrel/` folder, continuously, with a "vault folder" setting | S |
| A9 | **Intel Macs and Windows 10 are unsupported.** A real share of the installed base in 2026 | Granola supports Intel | Hardware decision, not code. Document it, do not fight it | – |
| A10 | **The website promises an agent that does not exist.** README item 3 and the vision doc's "notices and acts" | n/a | Rewrite the pitch around what ships: remembers, briefs, drafts, approves. Agency returns when M2.3 exists | S |
| A11 | **Consent is a settings footnote.** FR-MTG-6 asked for an announcement template; the tree has a one-line warning and Away Mode's chat note. Granola is being sued in California over exactly this | Granola's "Heads Up" (Labs) notice; nobody does it well | Away Mode's `ZoomChatPosting` already posts a chat line; generalise to an optional recording notice (chat line or spoken clip) per meeting, with a jurisdiction note in onboarding (NFR-PRV-7) | S |

### 3.2 Retention and revenue gaps (first month onward)

| # | Gap | Who does it better | What exists to build on | Size |
|---|---|---|---|---|
| R1 | **Routines stop when the lid closes.** Doc 15's own first consequence | Littlebird (cloud routines); every bot tool (the bot shows up without you) | `core/routines` and `api::routine_runs::tick` are pure Rust over `Store`; `sync-engine` already pulls an account's rows into any peer. A cloud runner is one more enrolled device that happens to be a Worker or container | L |
| R2 | **The phone cannot record or be pushed.** The two things people rate Granola's phone app for | Granola (one-tap record, Watch); Littlebird (push) | M4.3 and M4.4 are designed in doc 15 with the Worker routes and APNs extension specified; `core/transcribe` builds for iOS already | L |
| R3 | **No backup, no restore, no "fresh Mac" story.** Sync is replication, not backup; delete the only computer and the Turso copy is the only copy, undecryptable without a device | Obsidian Sync (version history); every cloud tool trivially | Turso has point-in-time restore on paid plans; a recovery key (a wrapped group key under a user passphrase, stored in the control plane) is the missing primitive and the thing that makes "sign in on a new Mac" possible | M |
| R4 | **Mutable things do not merge.** Settings use field-level last-writer-wins; note drafts and action-item edits do not sync at all | Obsidian Sync (merge), Notion | FR-SYNC-4 named CRDTs; per-field LWW already exists. Automerge is heavy; a per-field vector clock on `meeting_note_draft` and `meeting_action_item` is enough | M |
| R5 | **No team.** Not a shared meeting, not a shared routine result, not a second seat | Granola Business, Littlebird Team | A "space" is a second group key issued by the relay, with per-item consent to re-encrypt a row under it; `sync_row` already carries an `entity_tag` and row type | L |
| R6 | **Connectors stop at mail and calendar.** Slack, Linear, Todoist, Notion, Drive are named in FR-MCP-2 and absent | Littlebird ("hundreds"), Granola (Slack, Notion, HubSpot, Zapier) | `mcp-host` is generic; FR-MCP-4 (register any MCP server) is the multiplier: every connector in the MCP ecosystem for the cost of a settings pane and the remote HTTP transport | M |
| R7 | **Action items die as text.** No calendar, task or CRM write-back | Granola recipes; Fireflies CRM | `agent::WriteActionKind` has two kinds; add `create_task` and `create_calendar_event` from an action item and route through the existing approval inbox | S |
| R8 | **No encryption at rest.** NFR-PRV-2 is open; a stolen laptop reads the SQLite file | Obsidian (file-level FileVault only), Littlebird (cloud) | SQLCipher via `rusqlite`'s `bundled-sqlcipher` feature in `core/memory`, key in Keychain / Credential Manager, one migration | M |
| R9 | **Speaker identity resets every meeting.** "Speaker 2" is the same colleague every Tuesday | Nobody does this on-device; Fathom/Otter do it in the cloud | `core/diarize` already computes speaker embeddings; persist them with a calendar-attendee seed (BL-6) | M |
| R10 | **No proactive surface.** Briefs sit in a tab | Littlebird Glance and push; Granola notifications | Today already exists on three platforms; what is missing is a notification policy and the push plumbing from R2 | S |
| R11 | **Usage is visible but spend is not controllable per routine on the phone**, and the phone has no model without a hosted key | Littlebird (credits, hosted model) | `usage.rs` and spend caps exist; a per-routine budget in the routine row is a small change. The phone problem is structural (doc 15 §0) and is answered by R1 | S |

### 3.3 Things that are not gaps (keep the divergence)

- No plaintext cloud memory. Littlebird's hosted MCP and server-readable store are the
  reason to choose Whimbrel, not a feature to copy.
- No credit economy and no hosted model. Doc 16, decided.
- No screenshots in ambient capture. Rewind, Recall and Screenpipe all paid for that.
- No bot. Every bot tool's reviews carry the "everyone sees the bot" complaint.
- Heuristic meeting detection stays. Granola and Littlebird are calendar-and-manual only.

---

## 4. Roadmap: three releases of adoption, then the business

Each release has a gate taken from doc 16's instruments (the update feed's weekly unique
clients and the relay's device list) plus one qualitative check. Durations assume one
developer with AI help, full time; double them beside a day job, as doc 16 says.

### Release 1 — "First meeting, first minute" (4–6 weeks)

Goal: a new user gets a good transcript and summary from their first meeting with one
permission granted and no account, key or model decision made.

| Item | Gap | What ships |
|---|---|---|
| 1.1 Setup in one screen | A1 | Onboarding asks for Microphone and Screen/System Audio only; Accessibility, Calendar and Notifications are requested the first time the feature is touched, from inside that feature. The search model downloads in the background with keyword search working meanwhile (it already does; say so in the UI). Sign-in is never shown until the user opens Sync or installs a second device |
| 1.2 Better transcripts by default | A2 | On macOS 26, `SpeechAnalyzer` as the default `transcribe` backend behind the existing trait; on other Macs and on Windows, offer `whisper-large-v3-turbo` (quantised) as a one-click download on hardware that meets a measured speed floor, with `base` as the floor model. Live chunking stays. Measure word error rate on three real meetings before and after, record it in `docs/release-evidence/` |
| 1.3 Ask works with nothing configured | A3 | A "Download a small local model (2 GB)" button on the empty Ask state that fetches a 3–4B instruct GGUF and runs it through a bundled llama.cpp (or launches a bundled Ollama). The guided key on-ramp stays one click away for users who want a better model |
| 1.4 Chrome and Edge meeting detection | A5 | Decide the `AXEnhancedUserInterface` lifecycle (set on detection of a meeting-capable tab, unset on quit) and ship it; Windows already reads the address bar |
| 1.5 Pitch matches product | A10 | README item 3 and the website hero drop "agentic desktop automation" until M2.3 exists. "Remembers, briefs, drafts, asks before it acts" is true today and is the privacy pitch |
| 1.6 Import | A4 | `.vtt`, `.srt`, `.txt`, Granola CSV and Otter exports as `meeting` rows with transcript segments and a model-written summary; a folder of Markdown (an Obsidian vault) as `items`. The empty Meetings state offers it. Granola's CSV is its only full-history export, so "bring your Granola history with you" is a real switching offer |
| 1.7 Recording notice | A11 | Optional per-meeting notice (chat line where the platform allows it, otherwise a reminder to announce), off by default, with the jurisdiction note moved from Settings into onboarding |

Gate: a fresh install on a Mac the author has never touched produces a summary of a real
meeting with no settings opened; weekly unique clients grow for three consecutive weeks
after the release.

### Release 2 — "Where you already are" (4–6 weeks)

Goal: Whimbrel is reachable from any app and its output is usable in the user's own tools.

| Item | Gap | What ships |
|---|---|---|
| 2.1 Overlay and command bar | A6 | One global hotkey (default double-tap Option on Mac, Alt+Space on Windows) opens a floating panel with: Ask over `assemble_context` plus the current window's text plus the live transcript if a meeting is running; a command list (start meeting, pause capture 15/60 min, open today's brief, jump to a meeting); and a quick switcher over meetings, conversations and topics. Windows first (plugin exists), Mac in the same release |
| 2.2 Obsidian and Markdown export, continuous | A8 | Extend `topics_export.rs` to meetings (summary, decisions, action items, transcript link, snapshots as attachments) and routine outputs, written into a user-chosen folder (an Obsidian vault) on completion and on edit, with `whimbrel_id` frontmatter, wikilinks to the already-exported people and topics, and the preserved `## My notes` block. Document it as the Obsidian integration. Also: a single-meeting "Copy as Markdown" and "Share…" sheet |
| 2.3 Conversations sync | A7 | A `conversation` content row type; the phone resumes a Mac thread |
| 2.4 Timed pause and category exclusions | A1 | "Pause for 15 min / 1 h / until tomorrow" in the menu bar and tray; shipped lists for banking, health and adult domains |
| 2.5 Action items become actions | R7 | "Add to calendar", "Create task" (Todoist or Google Tasks via a small MCP connector) and "Draft follow-up" on every action item, through the existing approval inbox |
| 2.6 Register any MCP server | R6 | Settings pane for command- or URL-based servers with the existing scope model; remote streamable-HTTP transport in `mcp-host`. This replaces writing Slack, Linear and Notion connectors by hand; ship a curated list of known-good community servers |

Gate: a working day is drivable from the hotkey without opening the main window; at least
twenty users have an export folder configured (the setting syncs, so the relay can count
it without reading content).

### Release 3 — "It keeps working" (6–8 weeks)

Goal: the two capabilities doc 16 identified as the reason to pay exist and have been used
free by the author's own account for two weeks.

| Item | Gap | What ships |
|---|---|---|
| 3.1 Phone push and approvals (M4.3) | R2, R10 | Exactly as doc 15 §5.4: `POST /v1/push/register` and `/poke`, APNs, a content-free poke, a decrypting notification extension, Lock Screen approve with Face ID. Ships on the free plan during early access so it can be measured; moves behind Cloud when billing exists, as doc 16 decided |
| 3.2 Phone recording (M4.4, M4.5) | R2 | On-device whisper on the phone, Live Activity, Lock Screen widget, Action Button and Siri intents. Granola's most-praised feature; this is the phone app's launch moment |
| 3.3 Recovery key and backup | R3 | A user passphrase wraps the group key; the wrapped blob lives in the control plane; "Sign in on a new computer" restores from Turso. Nightly Turso snapshots for hosted accounts with a restore button |
| 3.4 Cloud routine runner, version 1 | R1 | A hosted twin of the primary device: a Worker (or a container, see §5.2) enrolled as a device named "Whimbrel Cloud", holding a per-routine key the user grants, running `api::routine_runs::tick` over the account's synced rows for the routines marked "run in the cloud", writing results as synced rows and poking the phone. Daily brief, EOD recap, weekly digest and meeting prep over synced data only. No connector tokens server-side until Google verification is done; the toggle says what the server can read. Author-only until Release 4 |
| 3.5 Encryption at rest | R8 | SQLCipher in `core/memory` with the key in the OS keychain; one-time migration; measured search latency before and after |

Gate: the author's account runs the daily brief from the cloud for fourteen consecutive
days with the laptop closed overnight; the phone receives it by push; a meeting recorded
on the phone appears on the Mac with a summary. Doc 16's Phase C numbers (40% week-4
retention of installs, 200 retained second devices, 20 named requests) are re-read here;
if they are met, Release 4 starts.

### Release 4 — Cloud goes on sale (doc 16 Phase D, 2–4 months)

Billing and entitlements (Stripe, a plan on the account, an entitlement check in the
apps), the hosted-relay device cap lifted for Cloud, push and cloud routines gated to
Cloud, backup and restore included, routing intelligence when usage signal exists. The
price is set in §5.1. No Team code until Cloud revenue covers hosting.

### Release 5 — Team (doc 16 Phase E) and the agent (M2.3)

Spaces (a second group key with per-item consent), delegated routines, the organization
key, admin basics. In parallel and only once Cloud is paying for itself, M2.3's computer-use
driver and skill format, because by then there is a user base to sell "acts" to and a
cloud runner for skills to run on.

### What is deliberately not on the roadmap

- Android before iPhone recording and push are measured.
- Slack, Linear or Notion connectors written by hand; FR-MCP-4 is the route.
- A web viewer (FR-WEB). The phone covers "read it away from the desk".
- Image generation, Projects as a container, "Max mode". None of these is a reason anyone
  cited for choosing or leaving a tool.
- SOC 2 before a buyer has asked for it in writing.

---

## 5. Monetization

Doc 16's two lines stand. This section says what each line contains, what it costs to run,
how it is priced, and which parts of the architecture make it defensible.

### 5.1 Cloud, for one person

**The promise, in one sentence:** Whimbrel keeps working when your laptop is closed, and
every device you own is on it.

| In Cloud | Why it needs a server | Cost driver |
|---|---|---|
| Hosted relay, unlimited devices (Free: one computer plus one phone) | The relay is the company's | One Turso database per account; Cloudflare requests |
| Daily backups, restore, recovery key | Snapshots and the wrapped-key blob live server-side | Turso storage; snapshot retention |
| Push to the phone | APNs/FCM credentials and the poke endpoint are the company's | Negligible per poke |
| Cloud routines: brief, recap, digest, prep, and later connector-backed routines | Something has to be awake | Compute per run (seconds), plus the user's own model bill, paid by the user's key |
| Routing intelligence (per-task model and prompt selection learned from outcomes) | Needs aggregate signal | Negligible |
| Email support | A person | Time |

**Price:** $12 per month annual, $15 monthly, as the website's anchor already says. The
comparison the buyer makes is Littlebird Plus at $17 annual ($20 monthly) with a credit
meter against Whimbrel Cloud at $12 plus their own model bill, which doc 14 puts at under $1 a month on cheap
models for a typical user and about $5 on a mid-tier model. The pricing page should show
that sum. Below $12 the plan reads as "sync with backups" and competes with Obsidian Sync
at $4 to $8; the cloud runner is what justifies the difference.

**Cost per Cloud account (estimate, to be measured in Release 3):** a Turso database on a
paid plan, snapshot storage for a few hundred megabytes, a few thousand Worker requests, and
a few minutes of runner compute a month. Under $1. The free base costs the same minus the
runner; doc 16's instruction to reclaim inactive databases after 180 days is already
implemented in `server/src/inactivity.ts`.

### 5.2 How cloud routines stay honest (the design that decides whether Cloud exists)

Doc 16 decision 3 said yes, scoped. The architecture that satisfies the scope:

1. **The runner is a device.** It enrolls like a Mac, gets its own X25519 keypair, and is
   approved by the user from a real device, which wraps the group key for it. The user
   sees "Whimbrel Cloud" in Settings → Devices, can rename it and can revoke it, and
   revocation rotates the Turso token exactly as it does for a lost laptop today. No new
   trust model; one more row in `device`.
2. **Per-routine grant, not account-wide.** The toggle "Run in the cloud" on a routine
   writes a `cloud_grant` row listing the row types that routine may read (`meeting`,
   `routine_run`, `meeting_prep`, `items` of kind `email` for reply priorities, and so
   on). The runner enforces it in code and the toggle shows the list in plain words: "The
   cloud runner can read your meeting summaries and routine history while this runs."
3. **The runner holds plaintext only while a run executes.** It pulls the rows the grant
   allows, decrypts in memory, runs `api::routine_runs::tick` with the user's own provider
   key (synced through settings sync, which already carries opt-in API keys), encrypts
   the `routine_run` result under the group key, pushes it, pokes the phone, and drops
   everything. Nothing is persisted in the clear. The design doc for Release 3.4 says
   exactly this and the Settings toggle links to it.
4. **Where it runs.** Cloudflare Workers cannot hold the Rust core's native dependencies
   (whisper, sqlite-vec) and have short CPU limits; the runner does not need either. A
   slimmed `api` build without `transcribe`/`diarize` (the same feature split M4.0 needs
   for the phone) compiled to WASM runs on Workers for the four synced-data routines. If
   it does not fit, a small container (Fly.io or Cloudflare Containers) running the Rust
   binary per account on a cron is the fallback and keeps the same device model.
5. **Connectors come last and only after Google's restricted-scope verification**, which
   doc 16 says starts now. Until then cloud routines read synced rows only, and Gmail
   ingestion keeps happening on the primary desktop.

If item 3 cannot be stated truthfully in a sentence in the UI, Cloud ships as sync plus
backup plus push at about $6 and billing waits; doc 16 already accepted that fallback.

### 5.3 Team, for the second person

Only after Cloud pays its own bill. The product:

- **Spaces.** The relay issues a space key. Sharing a meeting into a space re-encrypts
  that row under the space key with the capturer's consent, recorded as a row. The relay
  still holds ciphertext.
- **Delegated routines.** A routine whose result lands in a teammate's inbox, and whose
  "Create task" or "Draft reply" step a teammate's approval inbox can take. This is the
  "agentic collaboration" doc 16 scoped and it reuses `approval_request` /
  `approval_decision` rows from the phone design.
- **Organization key.** An admin pastes one provider key; seats use it through settings
  sync at the space level; the admin sees spend from the existing usage ledger pushed as
  rows. This is the one-click provisioning doc 16 said the comparison depends on.
- **Admin basics and one bill.** Seat billing on the same Stripe customer.

Price per seat is set only once Cloud has paying teams, and shown next to the
organization's model bill, as doc 16 requires. The likely number is $16 per seat annual,
undercutting Granola Business at $14 only once the model bill is added back, so the
pricing page must make that sum visible rather than hide it.

### 5.4 Support and self-hosting

Unchanged from doc 16: the relay and update feed are documented for self-hosting
(`docs/self-hosting.md`), and a support contract sells to the company that wants a person
responsible. Release 3's recovery key and backup make self-hosting materially better too,
which helps that sale.

### 5.5 Other ideas, graded

Ideas that came up while reading the tree, each with a verdict so they are not re-raised.

| Idea | Verdict | Why |
|---|---|---|
| **Skin marketplace** | Not now | Skins sync and are made by chat; a gallery of community skins on the website costs little and helps adoption. Charging for skins contradicts "nothing local is held back" |
| **Paid connector packs** | No | Connectors run on the user's device; doc 16's rule. FR-MCP-4 gives them away anyway |
| **Hosted MCP endpoint for the account** ("let Claude Desktop on my phone see my memory") | Later, as a Cloud feature | It is the cloud runner's read path exposed over MCP with the same per-client scopes the local server already enforces. Honest version of Littlebird's hosted MCP: scoped, revocable, decrypt-on-request. A genuine reason to pay for people who live in other AI tools |
| **Transcription compute for slow machines** (upload audio, get a transcript) | No | Resells compute, the audio leaves the machine, and it is exactly the thing the privacy pitch says Whimbrel never does |
| **Away Mode as a Cloud feature** | Partly | Joining Zoom needs a desktop; the follow-up email draft and "ask for the recording" are routines and can run in the cloud. Keep Away Mode free; its follow-up works better with Cloud |
| **Prepaid credit pack for people who do not want a key** | No, per doc 16 decision 4 | Revisit only with paying Cloud accounts to carry the fraud exposure |
| **Lifetime or founder licence** | No | Nothing local to license; a lifetime Cloud plan is a liability on a per-account cost |
| **Referral: a month of Cloud for each friend who installs** | Yes, cheap | Needs billing first; costs under $1 per referred account |
| **"Export everything" as a reason to trust, not a reason to pay** | Yes, free | BL-7. Full JSON + Markdown + audio export is the answer to "what if this app dies" and sits next to open source on the website |
| **Enterprise edition** (SSO, SCIM, device policy) | When a buyer has paid for a pilot, per doc 16 | Not before |

---

## 6. What this changes in the other documents

- **README and website:** item 3 ("agentic desktop automation") moves to "planned" and
  out of the lead; the hero says what ships. The comparison pages add rows for on-device
  transcription quality (after Release 1.2 has numbers), Obsidian export, and overlay.
- **Doc 05:** Releases 1–3 slot in as M4.6 (first-run), M4.7 (overlay and export) and the
  existing M4.3/M4.4/M4.5, with M2.3 explicitly after Phase D.
- **Doc 08:** FR-OVR, FR-CMD, FR-MEM-8, FR-CHAT-6 (conversation sync), FR-MCP-4 move from
  LB-2/LB-3 into Releases 1–2; FR-PRJ, FR-CHAT-11, FR-CHAT-12 and FR-WEB are dropped from
  the backlog.
- **Doc 15:** M4.3 and M4.4 keep their designs; the Release 3 gate adopts them.
- **Doc 16:** no decision reopened. §5.2 here is the design decision 3 asked for.

## 7. Decisions needed

1. **Adoption before billing.** Three releases (14–20 weeks full time) before any paid
   plan. The alternative, billing first, sells to the one account that exists.
2. **Release 1.2's transcription default.** `SpeechAnalyzer` on macOS 26 and a larger
   whisper model elsewhere, measured. This is the single change most likely to move
   retention, and it changes what the transcript looks like for existing users.
3. **Release 2.2: the Obsidian vault folder as the export target**, continuous, on by
   request. This is the integration to lead with on the website.
4. **Release 3.4's runner model**: the cloud runner is an enrolled device with
   per-routine grants, as §5.2 describes. Approve the model before the design doc is
   written, or Cloud becomes a $6 sync plan.
5. **Cut the agent from the pitch** until M2.3 exists.
