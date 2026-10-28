# 16. Open source, and what people pay for

Status: **proposal for decision**, 2026-10-21, revised the same day after the investor
review gate. It replaces the tier structure in [14-pricing-proposal.md](14-pricing-proposal.md)
and the planned Pro and Power plans on the website. Nothing here is on sale; the website
still says "free during early access."

## The premise

The whole tree goes public under Apache-2.0 (rename design,
`superpowers/specs/2026-10-21-kestrel-to-whimbrel-rename-design.md`). Every feature that
exists today stays free and open: capture, transcription, memory, search, chat, routines,
connectors, the MCP server, multi-device sync, and the relay that makes sync work. A
person can run all of it, including their own relay, without paying anyone.

Two constraints shape everything below:

1. **No revenue from passing on model costs.** Doc 14 and the website's Pro plan rest on
   "Whimbrel AI included": hosted models the company pays for, resold inside a
   subscription with a margin. That is out. Models are the user's business: local
   (Ollama) or a key the user or their organization owns, with any provider. The company
   never meters tokens and never marks them up. An organization's own key, handed to
   its seats through the product, counts as the user's own key.
2. **Nothing is held back from the open product to make the paid one look good.** The
   paid lines are things that genuinely need a server somebody runs, an organization
   around the user, or a person on the other end of a support contract.

With the source open, the code is not the moat, and neither are signed builds or an
update feed; every shipped desktop app has those. What can be sold is a server the user
does not want to run, with something running on it that cannot run on a closed laptop,
and the organization features a company needs around the product.

### What the plan is, in one sentence

A two-line business, a hosted agent runner for individuals and seats for teams, with a
support contract on the side. Everything else in this document is either a feature of
those two lines or a later option that has to earn its way in.

### The precedents, read honestly

- **Obsidian** is the right analog for a local-first app, and it points one way: free
  Obsidian syncs through your own iCloud, Dropbox or git (the analog of running your own
  relay), and the hosted relay, Obsidian Sync, is what they charge for. They give away
  everything local and sell the server.
- **anarlog** (ex-Hyprnote) is the closest competitor and already sells hosted AI plus
  encrypted sync plus sharing, with a team behind it, while its free app is local-first
  with the user's own models. "We leave the model bill with the user" is not a difference
  from anarlog; their free tier does that too. The difference has to be what runs on the
  server.
- **Screenpipe** is the graveyard in this exact category: open-source ambient screen and
  audio capture with a hosted tier, about 22k stars, which went source-available with a
  paywalled production app when the open model did not pay. Sentry relicensed for the
  same reason. This plan ends differently only if two things hold: the variable cost per
  free user stays near zero (no hosted models, which is constraint 1), and the company is
  run as a sustainable independent business whose free base is a cost line it can carry,
  not a venture business that needs the free base to convert at venture rates. That
  choice is decision 5 below, and it changes the gates.

## Revenue lines

| # | Line | Who pays and why | Needs | Confidence |
|---|---|---|---|---|
| 1 | **Cloud** (individual) | One person who wants routines that run while the laptop is shut, push to the phone, and sync across any number of devices on a relay with backups, run by someone else | Billing and entitlements, open sign-in with server-side limits, cloud routines, push, backup and restore | Medium, and it rests entirely on cloud routines. Without them Cloud is a backup add-on to a free sync service, which anarlog gives away |
| 2 | **Team** | A small team that wants shared meeting memory, routines that hand work between people's agents, an organization key for every seat, and one bill | Shared spaces with per-item consent, delegated routines, an org-key design, admin basics, seat billing | Medium. Seats are where Granola's money is; unproven for a local-first open product |
| 3 | **Support** | A self-hoster or a company that wants a person responsible: the self-hosted relay with a support contract, paid onboarding | A guide (exists) and time | High that a few will buy it, low that it is large |
| 4 | **Enterprise edition** | Later: a company that needs SSO and SCIM, device policy, audit export and a DPA | A private repository, and first a written answer to vendor viability (what happens to a one-person vendor) | Low until a buyer has paid for a pilot. Not a build line until then |

**Inside the lines, not lines of their own:** model routing intelligence (per-task model
and prompt selection, provider fallback, cost and quality tracking) ships inside Cloud
and Team once there is enough usage signal to beat the app's static defaults. The
client always calls the provider with the user's or organization's key; the service sees
task metadata and outcomes, never prompts or tokens. It is a reason to keep paying, not
a price.

**Excluded on purpose:** token resale in any form, including "fair use" bundles; affiliate
or referral revenue on providers; a later licence change to SSPL, BSL or FSL; any local
feature withheld from Free; a skills marketplace (the skill format, M2.3, does not exist);
an "at cost" managed key (zero revenue, real fraud and support exposure).

## The plans

| | **Free** | **Cloud** | **Team** |
|---|---|---|---|
| Price | $0 | **Not published until the package is decided** (see below) | **Not published until Cloud has paying teams** |
| Everything in the app | Unlimited: capture, meetings, history, search, chat, routines on your devices, connectors, MCP server | Same | Same |
| Models | Local or your own key | Same, plus routing intelligence when it exists | Same, plus an organization key for every seat and team-level routing policy |
| Sync | End-to-end encrypted. On **your own relay**: unlimited. On the **hosted relay**: one computer plus one phone, enforced server-side | Hosted relay, unlimited devices, daily backups and restore | Same, plus shared spaces |
| Cloud routines | No | Yes: scheduled agents that run server-side with keys you grant, per routine, revocable | Yes, including routines that hand off between teammates' agents |
| Push to phone | No | Yes | Yes |
| Team | | | Shared meeting memory with per-item consent, delegated routines, admin, one bill |
| Support | Community | Email | Email, priority |

**On the hosted-relay cap.** The hosted relay is the company's server and can enforce any
account limit; what cannot be enforced is a limit on a relay the user runs, which is the
free path and is meant to be. The earlier draft of this document gave the hosted relay
away unlimited and then cited Obsidian as the precedent; Obsidian does the opposite, and
so does this plan now.

**On the price.** Cloud's price depends on decision 3. If cloud routines are in, Cloud
competes with Littlebird ($17 a month) on "an agent that works while you are away," and
the number should sit at or above the website's current planned anchor of $12 annual /
$15 monthly; publishing anything lower is a visible price cut before anyone has paid.
If cloud routines are out, Cloud is hosted sync with backups and push, competes with
Obsidian Sync (check their current tiers before quoting), and is worth about $5; at that
package it does not justify building billing on its own. Team's price is set only after
the all-in comparison is honest: Granola Business at $14 includes transcription and the
model; a Team seat here is the seat price plus the organization's model bill, which doc
14 puts at roughly $5 a month for a typical user on a mid-tier hosted model. The seat
price has to be stated next to that second invoice, and the buyer has to be shown how
the organization key removes the provisioning work.

## What changes on the website before the repository goes public

A public repository and a pricing page that promises "Whimbrel AI included" cannot coexist;
the first reader to compare them will say so. Before step 4d of the rename:

- Pro and Power come off the pricing page and the comparison table. No new prices go up
  until billing exists and the Cloud package is decided; "planned" plans without numbers
  are fine.
- The FAQ entries "What is Whimbrel AI?" and "Do I need an AI API key on the Free plan?"
  are rewritten: there is no hosted model; local or your own key, always.
- The comparison page gains the "open source" row the scope document calls for.

## Roadmap, with the gate that opens each phase

Each phase has a gate; if the gate is not met, the phase does not start. The product
ships no telemetry and should not, so the instruments are the two servers the company
already runs: the update feed (every running install checks it, so weekly unique clients
is the active-install count) and the relay (the device list is the multi-device cohort).

### Phase C: launch (the four weeks after release N+1)

Public repository, "source on GitHub" on the website, the pricing changes above, Show HN
and the roundups. **No billing.** Free is the product.

Two things ship in Phase C because they are the instruments, not the product: hosted
relay sign-in opens beyond the email allowlist with the one-computer-plus-phone limit
enforced server-side, and the update feed records weekly unique clients. Without them the
next gate cannot be measured.

Gate to Phase D, all three:

- Weekly unique update-check clients in week 4 at or above 40% of week-1 downloads.
- 200 hosted-relay accounts whose second device is still syncing 30 days after enrollment.
- 20 named people who asked for routines-while-closed or phone push specifically.

### Phase D: Cloud (two to four months)

1. Billing and entitlements: Stripe checkout and portal, a plan on the account in the
   control plane (`server/`), an entitlement check in both apps.
2. Backup and restore of the per-account Turso database: nightly snapshot, restore from
   the app, 30 days of retention.
3. Cloud routines, version 1: a routine marked "run in the cloud" runs on a schedule in an
   isolated worker that holds a key the user granted for it, calls the user's own model
   provider with the user's key, and writes its result back as a synced row. The design
   must say what the server can read while such a routine runs (that routine's inputs,
   for that run) and the app must say it in the toggle. This is the one feature that
   softens "the relay sees only ciphertext," and it is opt-in per routine. A routine
   that uses the Gmail or Calendar connector needs those OAuth tokens on the server too,
   which makes Google's restricted-scope verification a prerequisite of this feature,
   not an option.
4. Push for routine results and approvals.
5. Routing intelligence, inside Cloud, when usage signal exists.

Gate to Phase E, all of these, measured in money and retention, not conversations:

- Cloud revenue covers the whole hosting and billing bill with margin.
- Month-3 retention of paying Cloud accounts above 85%.
- At least five work-email domains with three or more paying Cloud accounts each.

### Phase E: Team (three to six months)

Shared spaces: a meeting or a routine result shared into a space, with per-item consent
from the person who captured it, synced through a space key the relay issues. Delegated
routines: a routine one person runs whose result lands in a teammate's inbox, and whose
next step a teammate's agent can pick up with approval. The organization key: an admin
provisions one provider key for the team, seats use it through the product, spend is
visible to the admin. Admin basics: membership, seat billing, remove a member and rotate
the space key.

"Agentic collaboration" is deliberately narrow here: hand-offs between people's agents on
shared memory, not a shared autonomous agent. The latter is a research project; the
former is a product.

Gate to Phase F: one signed, prepaid pilot with a dollar figure, or one completed security
questionnaire from a buyer who then asked for a quote, and a written vendor-viability
answer (open source plus a documented self-host path plus a named successor or escrow
arrangement). Without that answer the first procurement review ends the conversation
regardless of SSO.

### Phase F: Enterprise edition (when the gate is met, not before)

A private repository from its first commit, so the boundary never needs a licence
change: SSO and SCIM, device policy (local-only, retention, exclusions, allowed
providers), audit export, a self-hosted control plane package with a support contract.
Sold by hand, annual, minimum seats. Until then the only enterprise SKU is the
self-hosted relay with a support contract, sold against a prepaid pilot.

### Always

- Nothing paid is ever written into the public tree and later removed.
- Every paid feature needs the company's server or the company's time. Anything that
  runs fine on the user's device goes into the open app.

## The arithmetic

The plan's own survivability ceiling is about a thousand Cloud subscribers before support
needs a second person. At $12 that is roughly $12,000 a month. At a 2% free-to-paid rate
that needs about 50,000 hosted-relay accounts, each holding a Turso database on the
company's bill. So the cost that matters is **per free syncing account**, not per payer:
Turso's current per-database price beyond the free plan's 100, times the free base, plus
the Cloudflare plan. Before Phase D: get that number from Turso's pricing page, decide
whether inactive free accounts are reclaimed (a database untouched for 90 days is
archived, restorable on sign-in), and note Turso's own vendor risk for a one-person
operator.

That is a good independent business and not a venture one. The gates above are written
for the former. If the goal is the latter, every number in this document is wrong and
the right document is a fundraising plan, which this is not.

**Cash.** There is no revenue until the end of Phase D, which is four to six months after
launch at the earliest, and no seat revenue until the end of Phase E, six to twelve
months out. The founder's time has to be funded through that. Write the runway number
down next to this plan; if the work is done beside a day job, double every duration
above.

## Risks, stated plainly

- **Free is complete and self-hostable, so conversion will be low.** Accept it; the
  free base costs cents per account if the Turso line is managed. If Cloud conversion is
  under 2% at 90 days, improve Cloud, do not worsen Free.
- **Cloud routines are the business and they weaken the privacy story.** One key per
  routine, revocable, isolated, with the toggle saying what the server can see, and
  Google verification done first. If that design cannot be made honest, cloud routines
  do not ship, and this plan reduces to hosted sync at $5 plus support.
- **anarlog can copy any of it** and has a team. The only answer is to ship cloud
  routines and delegated routines before they do, on top of screen memory and MCP
  routines they do not have.
- **Team buyers compare all-in cost.** The seat price plus the organization's model bill
  has to be shown next to Granola's $14, and the organization key has to make
  provisioning one click, or the comparison is lost at the first screen.
- **Support can eat the company.** Every plan's support line is written to be
  survivable by one person; enterprise support is sold only with a contract that funds it.

## What this drops from doc 14

The Pro and Power plans and their model allowances; the AI proxy with metering and a
spend cap; "frontier model access" as an upgrade reason. It keeps doc 14's one-computer
plus phone limit, now applied to the hosted relay only and enforced server-side.

## Decisions needed

1. Replace Pro and Power with Cloud and Team as above, and change the website before the
   repository goes public.
2. Open hosted-relay sign-in, with the limit enforced, as part of the launch.
3. **Whether cloud routines may hold user-granted keys server-side at all.** This is the
   decision the plan rests on. Yes means Cloud is an agent runner and prices at or above
   $12; no means Cloud is sync plus backups at about $5 and billing waits.
4. Whether to ever offer an at-cost managed key. The recommendation is no.
5. Whether this is an independent business or a venture one, because the gates differ.
