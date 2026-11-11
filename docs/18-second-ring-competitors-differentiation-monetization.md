# 18 — The second ring: Bluedot, Grain, Voicenotes, Shadow, Fellow

**Status:** decided 2026-11-11 (§9 records each decision). Written against the tree at 0.9.68, the live
website copy, and a competitor pass made the same day. Docs [16](16-open-core-commercial-model.md),
[17](17-competitive-gaps-roadmap-monetization.md) and [roadmap-2026-11](roadmap-2026-11/00-executive-summary.md)
measured Whimbrel against Granola and Littlebird. This one measures it against the five
products one ring out, and it reopens two decisions those documents closed, because what
shipped since has already overturned them.

**Sources.** Each vendor's home, pricing, security and release-notes pages, read
2026-11-11; press for funding; G2 and Trustpilot through search snippets only (direct
fetches returned 403); Reddit returned nothing usable. Pricing was fetched twice
independently and agrees except where marked. **[unverified]** means the figure is from a
third party, a snippet, or conflicting sources. Whimbrel facts are from `CHANGELOG.md` and
`website/site/pages`, not from the older docs.

**Assumption that everything below depends on.** No adoption numbers are in the tree. Doc
17 said the hosted relay had one account on 2026-10-31. This document assumes active
installs are still in the tens, not the thousands. If that is wrong, §6's ordering changes:
with a real base, pricing moves ahead of positioning.

---

## 0. The verdict

Whimbrel is the most capable product in this set and the worst-positioned one. Five
companies, at least two of them with twenty people or fewer, have each taken one sentence
of Whimbrel's pitch and built a business on it:

| Their headline | The sentence of Whimbrel's they took | What they charge |
|---|---|---|
| Shadow: "The meeting assistant built for your AI agents." | Local capture, on-device transcription, memory other AI tools can read | $8 a month for the AI |
| Voicenotes: "The notetaker for your confidential meetings." | Private by design | $9 and $24 a seat |
| Bluedot: "Invisible, privacy-first AI note taker" | No bot, privacy first | $14 to $32 a seat |
| Fellow: "The secure AI meeting assistant" | Security and governance | $7 to $25 a seat |
| Grain: "The AI notetaker for agents and teams" | Context for Claude and ChatGPT | $15 to $29 a seat |

Three of those five (Voicenotes, Bluedot, Fellow) make a privacy claim over a cloud
architecture that stores customer recordings on the vendor's AWS account. Whimbrel and Shadow are the
only two here that keep recordings on the user's machine, and Whimbrel is the only one of
the six that does not lead with its claim to a named buyer.

Four harsh findings, each expanded below:

1. **Whimbrel's hero is category boilerplate.** "No meeting bot", "local first", "any
   model", "connects to your AI tools" are now on every competitor's page. The things only
   Whimbrel has (memory of non-meeting work, a page per client with what is owed, proof
   that nothing left the machine) are below the fold or absent.
2. **Whimbrel charges for the wrong thing.** Every competitor's paid plan is the AI, at
   one price. Whimbrel's paid plan is a third device, and its AI is a prepaid meter most
   users never have to touch. The problem is not the price; it is that very few users have
   any reason to pay anything.
3. **Nobody is named as the buyer.** Each of the five names one. Whimbrel names three
   personas and sells to none.
4. **The surface area is a venture company's and the staff is one.** Four platforms, seven
   languages, skins, six Today layouts, Away Mode, question alerts. None of these was a
   reason anyone in this research gave for choosing or leaving a product.

The path: pick the buyer for whom "it is stored on my computer, not yours" is a requirement rather
than a preference, sell that buyer a flat plan with AI included and a ceiling they set,
make the proof of privacy the marketing, and make Whimbrel the place their AI tools get
context about their whole workday. Stop building everything else until that sells.

---

## 1. What is true about Whimbrel today (0.9.68)

Docs 16 and 17 are thirteen and eleven days old and already wrong in places that matter.

| Earlier decision or gap | What the tree and the website say now |
|---|---|
| Doc 16 decision 4: no token resale, no managed key, ever | **Reversed.** Whimbrel AI shipped in 0.9.66: prepaid credit, cost plus 30%, stops at $0 |
| Doc 17 §5.1: Cloud at $12 to $15, justified by cloud routines; "at $5 it does not justify building billing" | **Cloud is on sale at $5** for unlimited devices. Cloud routines, backups, restore and push are listed as "coming" |
| Doc 17 A2, default transcript quality | Closed in part: Apple's on-device engine on macOS 26, a larger whisper model offered elsewhere |
| A3, no model means no chat | Closed: a 1.9 GB local model from the empty Ask screen (0.9.48) |
| A4, nothing to search on day one | Closed: import of `.vtt`, `.srt`, `.txt`, Otter, Granola CSV, Markdown folders |
| A5, Chrome-family meeting detection | Closed (0.9.62) |
| R9, speaker identity resets | Closed: names from invitees, voices remembered, synced (0.9.57) |
| Connected Topics "built and invisible" | Shipped on every platform, with owed and owing steps and an Obsidian vault export |
| A6, overlay and global hotkey | **Still open.** Nothing in the changelog through 0.9.68 |
| R2, phone recording and push | **Partly open.** Android and iPhone read and ask; phone reminders shipped on Cloud; no phone recording |
| R8, encryption at rest | **Still open.** No SQLCipher in the tree |
| R1, cloud routines | **Still open** |
| R5, Team | **Still open**, unpriced |

The engineering pace is not the problem. Twenty-five releases in eleven days closed most of
doc 17's adoption list. What did not move in that time is the pitch, the buyer and the
price.

---

## 2. The five products

Prices are USD per user per month. "Annual / monthly" means the annual-billing price and
the month-to-month price.

### 2.1 Shadow (shadow.do, Taper Labs)

The closest twin, and the one to study hardest.

- **What it is.** A native Mac app (Apple silicon only). No bot; starts and stops with
  calls on Zoom, Slack, Teams, Meet, Webex and Discord. On-device transcription; raw audio
  stays on the Mac; "smart screenshots" of shared windows. Output is Markdown files on
  disk, which is how it reaches Obsidian, Claude, Codex and OpenClaw. Webhooks to Zapier,
  n8n and Make.
- **The idea worth taking.** Skills. Meeting Skills are prompts that run when a call ends.
  Action Skills are hotkey tools that take voice, the screen or selected text as input:
  Quick Reply drafts an answer from the thread on screen; Voice Typing dictates. This is
  doc 17's item 2.1, the overlay, shipped by a ten-person company and sold.
- **Price.** Free: unlimited transcription, recording and screenshots, **no AI**. Plus
  $8 a month: all AI features, described as unlimited. Two-week trial, no card. Annual
  price not shown.
- **Weak.** Mac only, no phone, no sync, no team, no compliance claim, no own-key option,
  cloud model calls with the provider unnamed. Thin, partly incentivised reviews.
  Founders, about ten staff, seed of undisclosed size **[unverified]**.
- **What it teaches.** The same architecture as Whimbrel, a narrower product, one sentence
  of positioning, and a higher price than Whimbrel gets. Free capture with paid AI is a
  line users accept from a local-first product. Whimbrel does more than Shadow on every
  row except two: the hotkey, and the clarity of the offer.

### 2.2 Voicenotes (voicenotes.com)

- **What it is.** Began in 2024 as a voice-memo "second brain" from the founders of Buy Me
  a Coffee; repositioned in 2026 as a meeting notetaker. Botless capture on Mac and
  Windows, apps for iPhone, Android, Apple Watch and Wear OS, web, a Chrome extension.
  Dictation from the Fn key. A public MCP server (March 2026), an OpenClaw skill, an
  Obsidian plugin. A bot exists on Business and up.
- **Price.** Free: 30 minutes a meeting, 14-day history. Pro $9: three hours of
  transcription a day, live transcript, speaker names, the Claude and ChatGPT connectors.
  Business $24: unlimited, bot, pre-meeting briefs, API. Enterprise: SSO, audit logs,
  retention, consent management.
- **Privacy, read closely.** The headline is "confidential". The privacy policy says
  cloud only, AWS in the US, model calls to OpenAI, Anthropic and ElevenLabs, no on-device
  processing, and that end-to-end encryption is not feasible for its AI features. SOC 2
  Type II since March 2026.
- **Weak.** Reliability shows its voice-memo roots: failed uploads lose recordings because
  nothing is kept locally. Trustpilot complaints about support and refunds.
- **Scale.** "1 million downloads" on the homepage **[unverified]**; no funding found.
- **What it teaches.** Two things. The phone and the watch are where daily habit forms,
  and dictation is a use every day whether or not there is a meeting. And "confidential"
  is now a headline that a cloud-only product is willing to claim, which means the word
  alone no longer differentiates; only proof does.

### 2.3 Bluedot (bluedothq.com)

- **What it is.** Botless capture through a Chrome extension and desktop apps, with iPhone,
  Android and Watch. Summaries, chat across meetings, templates, CRM and ATS sync, a
  Claude connector, a ChatGPT plugin, an MCP server. Names legal, financial services and
  real estate as verticals.
- **Price.** Free: **five meetings, lifetime**. Basic $14 / $18 (audio only, one hour a
  recording). Pro $20 / $25. Business $32 / $39 (CRM, SSO). Cancelling removes access to
  recordings, with a month to restore.
- **Privacy.** AWS, SOC 2 Type II, "never used to train". No local processing. Consent is
  an optional message; "invisible" is the product, so the consent risk sits with the user.
- **Company.** About $0.6M raised and under twenty people **[unverified, PitchBook and
  Tracxn]**, against a claim of "100,000+ companies" **[unverified, self-reported]**.
  G2 4.8 from about 223 reviews **[snippet]**.
- **What it teaches.** Distribution. Its blog is a comparison-content operation (six
  posts appeared dated 10 and 11 November **[unverified; the dates may be a rendering
  artefact]**). A small team with a thinner product than Whimbrel and a five-meeting free
  tier has a G2 profile, search presence and paying customers, and Whimbrel has none of
  the three. It also shows that a stingy free tier does not stop a botless product from
  growing.

### 2.4 Grain (grain.com)

- **What it is.** A meeting recorder for sales, customer success and research teams,
  since 2020, now repositioned as the layer that "delivers [meetings] to Claude, ChatGPT,
  or any AI tool". Bot plus desktop capture (added October 2025, audio only
  **[unverified]**). Clips, playlists, workspace libraries, coaching scorecards, HubSpot
  and Salesforce sync, an MCP server, voice signatures, a recording-consent feature
  (October 2026).
- **Price.** Free: meetings to 45 minutes, 30-day history. Pro $15 / $19. Business
  $29 / $39, two-seat minimum, CRM sync and shared libraries. Unlimited free viewer seats.
  **[partly unverified: the pricing page rendered malformed on both fetches; these
  figures are from the homepage and a third party.]**
- **Company.** $16M Series A in 2022, about $20M total, about twenty staff, no round
  since. Ships monthly.
- **Weak.** Libraries get messy at scale, navigation churn, coaching trails Gong. One G2
  review: it "eventually become obsolete and frustrating".
- **What it teaches.** Where seat money comes from: CRM write-back, shared libraries and
  coaching, bought by a team lead. That is a different product from Whimbrel and Whimbrel
  should refuse it. It also shows that "meeting context for your AI agents" is now the
  fallback position of a six-year-old incumbent, so that sentence alone is no longer a
  wedge either.

### 2.5 Fellow (fellow.ai)

- **What it is.** A meeting-management product (agendas, 1:1s) rebuilt around AI notes.
  Bot, botless desktop capture (Mac, January 2026), or in-person from the phone. Ask
  Fellow across meetings, pre-meeting briefs, fifty-plus integrations, an MCP server, a
  Claude connector.
- **The part that matters.** The headline is "The secure AI meeting assistant" and a
  section reads "Built to serve regulated financial institutions". Its August 2026
  governance suite: a compliance portal that searches and flags recordings, consent
  capture before joining (Microsoft 365 only), password-protected recaps, retention
  schedules that delete raw data before summaries, transcript redaction, zero-day
  retention, and claims of SEC 17a-4 and FINRA-ready audit trails. All vendor claims.
- **Price.** Free: five AI notes, lifetime, up to five seats. Team $7 / $11, ten notes a
  user a month, twenty seats. Business $15 / $23, unlimited, fifty seats. Enterprise $25,
  annual only, ten-user minimum, HIPAA and BAA.
- **Company.** $24M Series A in 2021, no round since. "6,000+ teams" on the homepage;
  "eight figures in AI-based revenue" attributed to the CEO **[unverified]**. G2 4.7 from
  about 2,400 reviews **[snippet]**.
- **What it teaches.** The best-reviewed and probably largest company in this set sells
  privacy as **governance to an organisation**: consent records, retention, redaction,
  audit. Not architecture. Fellow's recordings are on Fellow's servers, and a compliance
  officer buys it anyway, because that buyer wants controls and paperwork, not a threat
  model. Two cautions: the governance suite is three months old and the revenue figure is
  unverified, so this is a bet Fellow has made, not a result. And it cuts against §4.3 as
  much as for it: where a compliance department exists, it will choose Fellow's portal
  over Whimbrel's architecture. Whimbrel's opening is the professional who has
  confidentiality duties and no compliance department.

### 2.6 The article

[BuildBetter's "7 Best Littlebird AI Alternatives"](https://blog.buildbetter.ai/7-best-littlebird-ai-alternatives-in-2026-compared/)
is not usable as a comparison and is useful as evidence.

- The URL says seven, the page title says two, the body details five (OpenAGI,
  BuildBetter, Aviso, Wilddog Notes, Fellow) and names five more in passing. It is tagged
  as an archived, repurposed post.
- Its description of Littlebird's weaknesses ("insights stay in dashboards", "generic
  theme extraction") describes a customer-feedback tool. It is not about Littlebird's
  product at all.
- It is a vendor page that ranks the vendor's own $40-a-seat product as the pick, with a
  partner in first place.

Two findings follow. **Whimbrel is not in it**, nor in any of the comparison content read
for this document. And a search for "Littlebird AI alternatives" on 2026-11-11 returned
about two dozen results: two BuildBetter posts, a Screenpipe compare page, a Carly post
and aggregator listicles (alternativeto, toolradar, topai and similar), several of which
do not agree on what Littlebird is. Whimbrel, the most literal answer to that query on the
market, was not among them. One search is not a ranking study, but the field is thin and
low in quality, so a sourced page of Whimbrel's own has a fair chance (inference).

---

## 3. The comparison

Whimbrel's column is from the changelog at 0.9.68.

| Capability | Whimbrel | Shadow | Voicenotes | Bluedot | Grain | Fellow |
|---|---|---|---|---|---|---|
| Capture without a bot | Yes | Yes | Yes (bot on Business) | Yes | Both | Both |
| On-device transcription | Yes | Yes | No | No | No | No |
| Audio and stored transcripts never on the vendor's servers | Yes | Yes | No | No | No | No |
| Text sent to a hosted model when AI runs | Only if the user picks one; none with a local model | Yes (provider unnamed) | Yes | Yes | Yes | Yes |
| Memory of work outside meetings | Yes (screen text, email) | Partial (on-demand screen) | No | No | No | No |
| A page per client or project, with what is owed | Yes (Topics) | No | No | No | Partial (libraries) | Partial (agendas) |
| Scheduled briefs and routines | Yes (seven) | Partial (run at meeting end) | Partial (briefs, $24) | No | No | Yes (briefs) |
| Hotkey actions on what is on screen | **No** | **Yes** | Dictation only | No | No | No |
| Dictation | iPhone only (into Ask) | Yes, system-wide | Yes, system-wide | No | No | No |
| Phone recording | **No** | No | **Yes, plus watch** | Yes, plus watch | Beta | Yes |
| Windows | Yes | No | Yes | Yes | Yes | Yes |
| MCP or agent access | Local server, seven read tools | Files and webhooks | Hosted MCP | Hosted MCP | Hosted MCP | Hosted MCP |
| Works from Claude or ChatGPT on the web or phone | **No** (localhost only) | No | Yes | Yes | Yes | Yes |
| Own model keys or local models | Yes | No | No | No | No | No |
| End-to-end encrypted sync | Yes | No sync | No | No | No | No |
| Open source | Yes | No | No | No | No | No |
| Consent workflow | Reminder text | Unverified | Enterprise | Optional message | Yes (Oct 2026) | Yes (M365) |
| Retention, redaction, audit | Retention only | No | Enterprise | Business | Enterprise | Yes |
| SOC 2 | No | No | Yes | Yes | Yes | Yes |
| CRM write-back, clips, coaching | No | No | Enterprise | Business | Yes | Business |
| Team sharing | No | No | Yes | Yes | Yes | Yes |
| Entry paid price | $5 (devices); AI metered separately | $8 (AI) | $9 | $14 | $15 | $7 |
| What the paid plan unlocks | A third device | All AI | Longer meetings, live transcript, connectors | Unlimited meetings | Longer meetings, history | More notes |

Read down Whimbrel's column: it wins every row about where data lives and what is
remembered, and loses every row about reach (phone capture, hotkey, access from the AI
tools people already use) and every row an organisation buys on.

---

## 4. The critique

### 4.1 Positioning: the hero says what everyone says

The live hero is "The AI assistant that remembers your work", with chips for "Free and
open source", "No meeting bot" and "Runs local models", then four cards: bot-free capture,
text not screenshots, local first, any model.

- Littlebird's headline is "The AI that understands your work." A visitor who has seen
  both cannot tell them apart from the first screen.
- "No meeting bot" is claimed by all five products in §2 and by Granola, Littlebird,
  Notion and Fathom. It has stopped being information.
- "Local first" and "any model" are architecture. They answer a question the visitor has
  not asked yet. Shadow has the same architecture and does not mention it in its headline;
  it says who the product is for.
- The claim only Whimbrel and Shadow can make in this set, that recordings and transcripts
  are stored on the user's machine and not on the vendor's servers, appears as the fourth
  section of the page.
- The feature only Whimbrel has, a page per client with the next meeting and what is owed
  in both directions, carries a "New" tag half way down.
- The pricing page has five columns (Free, Cloud, Whimbrel AI, Team, Support), two of which
  cannot be bought. A visitor has to read all five to learn that the product is free.

### 4.2 Monetisation: almost nobody has a reason to pay

| | What the paid plan is | Price a customer sees |
|---|---|---|
| Shadow | All AI features | $8 |
| Voicenotes | Meeting length, live transcript, connectors | $9 to $24 |
| Fellow | Notes past ten a month | $7 to $25 |
| Bluedot | Meetings past five, ever | $14 to $32 |
| Grain | History and meeting length | $15 to $29 |
| **Whimbrel** | **A third device ($5), and separately a prepaid meter for hosted AI** | **$0 for most; about $11.50 for a user who buys both** |

The $11.50: $5 for Cloud plus about $6.50 of Whimbrel AI credit, which is doc 14's
estimate of $5 a month of mid-tier model cost with the 30% added. Of that, Whimbrel keeps
the $5 and about $1.50 of margin. The competitors' prices are also before their model
costs, so revenue per fully paying customer is not where Whimbrel loses. It loses on
**how many users pay anything**:

1. **Device count is the weakest thing to gate.** It is unrelated to how much value the
   user gets, most people have one computer, and the people with three are the open-source
   enthusiasts most able to run their own relay.
2. **The AI, the thing every competitor charges for, has three free routes** (the bundled
   local model, Ollama, the user's own key) and one paid route that asks the user to do
   arithmetic. "Cost plus 30%" is honest, and it is a pricing page that requires knowing
   what a token is. Every competitor's answer to "what will this cost me" is one number.
3. **A prepaid meter discourages the usage the product needs.** The standing constraint,
   recorded in project memory, is to encourage usage, take a margin on it, and leave the
   user in control of what they spend. Credits satisfy the second and third and work
   against the first: every question visibly costs money.

At $5, doc 16's own ceiling of a thousand subscribers is $5,000 a month. §5.3 does not
raise the margin per subscriber much; it is an argument about attach rate.

### 4.3 The buyer: three personas is none

Doc 01 lists a manager, an individual contributor and a power user. Each competitor names
one buyer and builds the page around them. Whimbrel's three sentences could describe
anyone with a laptop, and its website does not show any of them.

The evidence in §2 says there are only two buyers in this category who pay for something
Whimbrel uniquely has:

- **People whose conversations are confidential and who are their own buyer.** Independent
  consultants, lawyers in small firms, financial advisers, executive recruiters, fractional
  executives, journalists, investors. Voicenotes ("confidential meetings"), Bluedot
  (legal, financial services) and Fellow (regulated finance) are all reaching for them with
  products that upload the recording. This buyer has no compliance department to sign off
  a SOC 2 report; the question they ask is "where does my client's call go?", and Whimbrel
  is the only product whose answer is "nowhere".
- **People who live in Claude, ChatGPT, Codex or Cursor and want those tools to know what
  they have been working on.** Shadow and Grain have both rebuilt their headline around
  them. Every competitor offers them meetings; only Whimbrel can offer the whole workday,
  and only without uploading it.

Team buyers (Grain's and Fellow's seats) are a third market, and Whimbrel has none of what
they buy: shared libraries, CRM write-back, admin, SSO, SOC 2.

### 4.4 Scope: built like a funded company

At 0.9.68 one engineer maintains a Mac shell, a Windows shell, an iPhone app, an Android
app, a relay, a website, seven languages hand-checked, skins made by chat, six Today
layouts, Away Mode, question alerts with draft answers, an optional TypeSafe integration,
and three connectors. In the complaint and praise evidence gathered for all five products,
none of skins, layouts, a translated interface, or a joining-on-your-behalf mode appears
as a reason anyone chose or left a tool. (Transcription accuracy in other languages does
appear, against Grain; that is a transcript-quality matter, not an interface one.) The reasons that do appear: the recording was
missed; the transcript was wrong; speaker names were wrong; it was not on my phone; it did
not get into my other tools; I did not trust where it went.

Away Mode deserves a specific note. It joins a Zoom call on the user's behalf and posts a
note in the chat saying so. The notice is the right instinct. But a participant still sees
an attendee who is not there, recording, which is what the "No meeting bot" claim tells
visitors Whimbrel does not do.

### 4.5 Proof: Whimbrel's privacy is as unverifiable as theirs

Voicenotes writes "nothing trains on your data, every word stays". Bluedot writes
"privacy-first". Fellow writes "secure". A buyer cannot test any of those, and today cannot
test Whimbrel's either without reading Rust. The assets exist: Apache source, the "what
left my machine" ledger, the Your data page, local-only mode. They are in Settings, not on
the home page, and no page shows the one demonstration nobody else can stage: turn off the
network and record a meeting.

Three things limit how far the claim can honestly go for the buyer in §4.3:

- **A hosted model sees the text it is given.** With Whimbrel AI or the user's own key, the
  transcript or context for that request leaves the machine, and Whimbrel AI requests pass
  through Whimbrel's server on the way to Anthropic (token counts are kept, content is
  not). Only the local-model path is fully offline. Any headline has to be true on the
  hosted path too, or the Voicenotes criticism in §2.2 applies to Whimbrel.
- There is no encryption at rest: a stolen laptop reads the SQLite file.
- The consent feature is a reminder with copyable wording.

---

## 5. The path

### 5.1 The position

> **Whimbrel is the private memory for people whose work is confidential.** It records and
> remembers on your own computer, keeps a page for every client, and gives your AI tools
> the context they are missing. Your recordings and your history are stored on your
> machine, never on ours, and you can see every request that leaves it.

Each clause is a fact in the tree today, and each excludes a named competitor: "on your
own computer" excludes Voicenotes, Bluedot, Grain, Fellow, Granola and Littlebird; "a page
for every client" excludes Shadow; "your AI tools" is the reason the second buyer comes
too.

The claim is deliberately the one that holds on every path, including hosted models
(§4.5). The stronger sentence, "nothing leaves your computer", is true only in local-only
mode and should be said only about that mode.

This is a hypothesis, not a finding. §7 says how to test it for two weeks of work before
committing the roadmap to it.

### 5.2 Messaging

**Hero options, in order of preference.**

1. "Your client calls stay on your computer." Sub: "Whimbrel records your meetings
   without a bot, remembers your work, and keeps a page for every client. Recordings,
   transcripts and screen history are stored on your Mac or PC, not on our servers. AI
   runs on your machine, or on a model you choose, and you can see every request sent."
2. "The meeting notes you can use on confidential work."
3. "Remembers everything. Stored with you." 

**Replace the three chips** with claims the competition cannot copy:

- "Records, transcribes and searches with Wi-Fi off"
- "See every request that ever left your computer"
- "Open source: check our work"

The first is true of capture, transcription, search and the local model; it must not be
shortened to imply hosted models work offline.

"No meeting bot" moves to the feature list, where it is still true and no longer the
headline.

**A second page for the second buyer,** at `/agents/` or `/mcp/`: "Give Claude your whole
workday, not only your meetings." Grain, Shadow, Voicenotes, Bluedot and Fellow all hand
agents a meeting transcript. Whimbrel hands them the meeting, the email thread before it,
the document that was open during it, and what was promised afterwards.

**One line against each.**

| Against | Line |
|---|---|
| Shadow | The same promise, on Windows and your phone too, with a memory of the work between meetings and a page for every client |
| Voicenotes | Their recordings and transcripts are stored in their cloud, by their own policy. Whimbrel's are stored on your computer, and you choose whether any text goes to a hosted model |
| Bluedot | Invisible to the other people on the call is not the same as private from the vendor |
| Fellow | Controls over recordings kept in their cloud, or recordings kept on your own machine |
| Grain | Built for a sales team's shared library. Whimbrel is built for one person's whole memory |
| Granola | Beautiful notes in their cloud; everything you worked on, on your machine |
| Littlebird | The same idea with the memory on AWS |

Keep these factual and sourced on the compare pages. The Voicenotes line in particular
must quote their privacy policy and carry the date it was read.

**Stop saying.** "Agent" anywhere on the site (doc 17 item 1.5, still true). "Free during
early access". Away Mode on the home page. The five-column pricing table.

### 5.3 Monetisation

Keep doc 16's principle that nothing that runs on the user's machine is withheld. Change
what is sold, to one plan a person can understand in a sentence.

| | **Free** | **Plus** (replaces Cloud and absorbs Whimbrel AI) | **Practice** (later, §6 Later) |
|---|---|---|---|
| Price | $0 | **$12 a month annual, $15 monthly** | $25 to $30 a seat, 2 to 15 seats |
| The app, unlimited | Yes | Yes | Yes |
| AI | Local model, or your own key at provider cost | **Included: Whimbrel AI with a monthly allowance, no key, no setup** | Included, pooled |
| When the allowance runs out | n/a | Falls back to the local model, or tops up to a monthly ceiling the user sets (default: no top-up) | Same, admin sets the ceiling |
| Private mode | Local model | **A setup choice: "Hosted" or "Private (on this computer only)", and a per-client switch (X6)** | Firm-wide policy |
| Sync | One computer and one phone hosted; unlimited on your own relay | Unlimited devices | Unlimited |
| Needs a server | No | Backups and restore, push, reach your memory from Claude or ChatGPT anywhere (§6 Next), cloud routines when they exist | Shared client pages, one bill, policy |
| Support | Community | Email | Priority |

Why this fits the standing constraint:

- **Encourages usage.** Inside the allowance, nothing the user does costs extra, so there
  is no reason to ration questions.
- **Carries a margin.** At $12 with an allowance worth $6 at Whimbrel AI's list price, the
  model cost is about $4.60 when the allowance is fully used. Against about $1 of relay
  cost and about $0.65 of payment fees, that leaves roughly $5.75 a subscriber at full
  use, and more from the subscribers who use less. Top-ups keep the existing 30%.
- **The user controls spend.** The ceiling is theirs, the default ceiling is the plan
  price, and the fallback at the ceiling is a working local model rather than a dead
  product. Littlebird's and Mem's credit complaints are about exactly this moment.

**What this does and does not fix, stated plainly.** Plus nets about what a customer who
buys Cloud and credit nets today. A thousand Plus subscribers is about $12,000 a month of
revenue and roughly half that after model and payment costs, which is still a one-person
business. The case for Plus is not margin. It is (a) attach rate: one price for "it just
works" gives the non-technical buyer a reason to pay that a device cap and a token meter do
not, and (b) unused allowance. Both are assumptions until measured.

Two further cautions. Doc 14's typical mid-tier user costs about $5 a month, or $6.50 at
list, which is already over a $6 allowance; the allowance only feels generous if the
cheap-first routing that Free already uses sends most calls to the small model. And doc
14's token estimates were never measured. The ledger now exists: measure a month of real
use before fixing the allowance or the price.

**Plus and the privacy claim.** Plus makes a hosted model the easy path for the buyer who
most cares where their work goes. So setup must ask, in plain words, "Hosted (better
answers; request text goes to Anthropic through our server, which keeps no content)" or
"Private (runs on this computer only)", and a client page must be able to force Private
for that client (X6). Without that choice, §5.1 cannot be said honestly to a Plus
customer.

What happens to today's offers:

- **Cloud at $5** stays for existing subscribers and stops being offered. Its features move
  into Plus. If a sync-only plan is wanted, keep it unadvertised.
- **Whimbrel AI credit packs** remain as the top-up inside Plus and as the way a Free user
  tries a hosted model once. They stop being a column on the pricing page.
- **Own keys and local models stay free.** That is the open-source promise and it is also
  the comparison that makes Plus honest: Plus is the price of not thinking about it.
- **The pricing page becomes two columns** (Free, Plus) with one line beneath for teams and
  self-hosting.

Expect conversion to stay low among the open-source audience, as doc 16 says. The buyer in
§5.1 is not that audience: a lawyer or a recruiter will not install Ollama or paste an API
key, and is used to paying $15 to $30 a month for a tool.

**Not recommended.** A capped free tier in the style of Bluedot's five meetings. It works
for them, and it would break the one promise ("every feature, unlimited history") that
makes Whimbrel's privacy claim free to test. Gate the hosted AI and the servers, never
local capture or history.

### 5.4 What to refuse

| Refuse | Because |
|---|---|
| CRM write-back, clips, coaching scorecards | Grain's ground; a team-lead purchase; needs a shared cloud library |
| A real meeting bot | Every bot product's reviews carry the complaint; Fellow's own include the bot not joining |
| SOC 2 before a paying buyer asks in writing | Whimbrel's argument is that there is nothing on its servers to audit. Spend the money on a published third-party review of the sync cryptography instead, which is evidence the cloud vendors cannot produce |
| A generic Team plan | Whimbrel has none of what Grain's and Fellow's seat buyers pay for. A small-practice plan (§6) is narrower and sellable |
| Computer use, multi-agent, a skills marketplace | Unchanged from roadmap-2026-11 §7.4 |
| More platforms, languages, skins or layouts | No evidence in this research that any of them wins a customer |

---

## 6. Roadmap

Three horizons. Durations assume one engineer, full time. Items already designed elsewhere
cite the document.

### Now: make the offer legible (three to four weeks, mostly not code)

| # | Item | Why now |
|---|---|---|
| N1 | New home page, pricing page and compare pages per §5.2; an "airplane mode" demo video; the "what left my machine" ledger shown on the site | The largest gap in this document, and the cheapest |
| N2 | Seven "alternative" pages: Littlebird, Granola, Shadow, Voicenotes, Bluedot, Grain, Fellow, plus one for people stranded by Rewind and Limitless. Each sourced and dated | §2.6: the Littlebird query returns aggregators and off-topic vendor posts, and not Whimbrel. Comparison content is Bluedot's main visible channel |
| N3 | The Plus plan per §5.3: one Stripe price, the allowance as a monthly grant into the existing credit ledger, the user-set ceiling, fallback to the local model at the ceiling | The server ledger, proxy and checkout already exist (0.9.64 to 0.9.67); this is packaging |
| N4 | Freeze list: skins, Today layouts, new languages, Away Mode (stays beta, off the site), TypeSafe, Android beyond parity fixes | Buys the time for everything below |
| N5 | Listings: G2, Product Hunt, the MCP server directories, the Obsidian community plugin list, a Show HN for the Apache release | Whimbrel appears in none of the comparison content read for this document |

Gate: a stranger can say what Whimbrel is and who it is for after ten seconds on the home
page (test with five people); the first Plus subscriber who is not the author.

### Next: reach, and proof (eight to ten weeks)

| # | Item | Answers | Builds on |
|---|---|---|---|
| X1 | **Hotkey panel with actions on what is on screen**: ask about this window, draft a reply to this thread, capture a note to this client's page, dictate. User-defined actions as saved prompts | Shadow's Action Skills, Littlebird's overlay, Voicenotes' dictation. The one feature row where the smallest competitor beats Whimbrel | Doc 17 item 2.1; `assemble_context`; the prompt library |
| X2 | **Phone recording** on iPhone, then Android, with the transcript made on the phone or on the paired computer | Voicenotes' and Bluedot's daily habit; the in-person client meeting, which is the confidential buyer's most common meeting | Doc 15 M4.4 |
| X3 | **Whimbrel as the context source for other AI tools**: MCP tools for topics, steps owed, email threads and screen memory, not only meetings; one-click setup for Claude, ChatGPT, Cursor and Codex; and **remote access through a tunnel to the user's own computer**, so Claude on the web or a phone can ask and the computer answers, with nothing stored server-side. Open question: a web MCP client's TLS normally ends at the relay's edge, so the relay may see a request in transit even if it stores nothing; the design has to say which, and the site must match | Every competitor's hosted MCP, without hosting the memory. The tunnel needs a server, so it is an honest Plus feature | The local MCP server and scoped tokens; the relay; doc 17 §5.5 "hosted MCP endpoint", redesigned so nothing is decrypted server-side |
| X4 | **Encryption at rest** | The gap that makes the confidentiality claim incomplete | Doc 17 item 3.5 |
| X5 | **Consent that leaves a record**: a notice posted to the meeting chat where the platform allows it, the wording and time stored with the meeting, a per-client default | Fellow's and Grain's consent features; the buyer in §5.1 is the one most exposed to consent law | Away Mode's chat posting; doc 17 item 1.7 |
| X6 | **Client controls on a topic page**: keep this client's material for N months, export everything about this client, delete everything about this client, exclude this client from hosted models | What a confidential practice is asked for by its own clients. Fellow sells the organisational version of this | Topics; retention classes; local-only routing; Export everything |
| X7 | Backups, restore and a recovery key | Already promised on the pricing page | Doc 17 item 3.3 |

Gate: twenty paying Plus subscribers, at least half of whom describe their work as
client-confidential when asked; phone recording used weekly by a third of phone installs.

### Later: only once Plus pays for itself

| # | Item | Condition |
|---|---|---|
| L1 | **Practice**, a plan for firms of two to fifteen: shared client pages with consent from whoever captured each item, one bill, one AI allowance, a firm-wide retention and exclusion policy pushed to every seat | Five firms ask for it by name. This is doc 16's Team narrowed to the buyer that values the architecture |
| L2 | Cloud routines on the runner model of doc 17 §5.2 | Plus subscribers ask for briefs with the laptop closed; phone reminders are the cheap test of that demand, already shipped |
| L3 | The commitments ledger and "since you were away" from roadmap-2026-11 F9 | After X1 to X3. It is the right retention feature and it is not what gets a first customer |
| L4 | A published third-party review of the sync cryptography and capture path | Before the first Practice sale |

Dropped or deferred from earlier roadmaps: the bi-temporal graph as the "spine of the next
year" (roadmap-2026-11) moves behind distribution and reach. Its user-facing pieces that
matter, the client page and what is owed, are already shipped as Topics. The eval harness
(F3) stays, because X1 and X3 change what retrieval is asked to do.

---

## 7. The test before committing

The position in §5.1 chooses a buyer on evidence about competitors' behaviour, not on
evidence about Whimbrel's own users, because there is too little of the latter. Two weeks
before N1 ships in full:

1. Two landing pages with the same download button: one for confidential client work
   (§5.2 hero 1), one for the AI-tools buyer ("Give Claude your whole workday"). A small
   equal spend on each, or equal posts in the communities where each buyer is. Compare
   download rate and first-meeting-recorded rate.
2. Ten conversations: five people who bill clients for confidential work, five who use
   Claude or Cursor daily. One question each: what do you use for meeting notes now, and
   what stops you using it on your most sensitive calls?
3. Ask every existing account which sentence on the current site made them install.

If the AI-tools page wins clearly, the roadmap keeps its items and swaps the order: X3
first, X1 second, and the home page leads with context for agents. If neither converts,
the problem is upstream of positioning and the next document is about first-run, not
strategy.

---

## 8. What this changes in the other documents

- **Doc 14:** its Pro plan at $12 / $15 with included AI returns, as Plus, with the
  allowance and ceiling of §5.3 in place of "fair use".
- **Doc 16:** constraint 1 (no revenue from model costs) and decision 4 are already
  reversed by Whimbrel AI; record that. Constraint 2 (nothing local withheld) stands and
  §5.3 keeps it. The Team line narrows to Practice.
- **Doc 17:** §5.1's Cloud price and package are superseded by what shipped at $5 and by
  §5.3. Item 2.1 (overlay) becomes X1 and moves ahead of everything in Release 3. §3.3's
  "no credit economy and no hosted model" no longer describes the product.
- **roadmap-2026-11:** the graph spine is deferred behind N1 to X7; §7.5's position ("the
  memory that is yours") is replaced by §5.1, which names who it is for.
- **Website:** home, pricing, compare, and a new agents page, per §5.2. "No telemetry"
  wording should match what the Your data page now says about the update check.

## 9. Decisions needed

1. **The buyer.** **Decided 2026-11-11: confidential client work is the primary buyer**
   for the next two quarters; AI-tools users are the secondary audience with their own
   page; team seats are refused for now. The §7 test runs alongside as a check, not a
   gate: with installs in the tens, two landing pages cannot give a meaningful result in
   two weeks, and the ten conversations are the useful part.
2. **Plus at $12 / $15 with included AI and a user-set ceiling**, replacing Cloud at $5 and
   the credit column. **Decided 2026-11-11: yes.** $12 a month annual, $15 monthly;
   existing Cloud subscribers keep their plan; the allowance is sized after a month of
   measured usage from the ledger. This formally reverses doc 16 decision 4.
3. **The freeze list in N4.** **Decided 2026-11-11: freeze all of it**, Android included:
   bug fixes only on skins, Today layouts, new languages, Away Mode and Android beyond
   parity, until Plus has paying users.
4. **Away Mode.** **Decided 2026-11-11: it stays in the app as a beta setting and comes off
   the website and all marketing.** No new work on it.
5. **X3's remote access.** **Decided 2026-11-11: a tunnel to the user's computer**, not a
   hosted memory. The design must state whether the relay can read a request in transit.
   If it can, the claim is "not stored on our servers", not "never seen by them".
6. **The wording of the privacy claim** (§5.1). **Decided 2026-11-11: the version that is
   true with hosted models**: recordings, transcripts and history are stored on your
   computer, not on our servers; AI runs on your machine or a model you choose. "Nothing
   leaves your computer" is used only about local-only mode.
