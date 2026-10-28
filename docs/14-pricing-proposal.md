# 14 — Pricing proposal

Status: **proposal, not decided.** The website still says "Free during early access" and shows no prices. Nothing below should go public until the billing work in [What has to be built first](#what-has-to-be-built-first) exists.

## The market (checked 2026-10-16)

| | Free | Entry paid | Upper tiers | Billing model |
|---|---|---|---|---|
| **Granola** ([pricing](https://www.granola.ai/pricing)) | Basic $0: 30-day meeting history, no briefs or follow-ups, few integrations | Business **$14/user/mo** | Enterprise $35/user/mo (SSO, SCIM, HIPAA, admin policies) | Per seat |
| **Littlebird** ([pricing](https://littlebird.ai/pricing)) | Basic $0: daily usage credits, limited meeting notes | Plus **$17/mo billed annually** | Power $42/mo (annual), Pro $100/mo, Team from $17/seat, Enterprise custom | Per user + usage credits |

The price anchor for this category is **$14–17 a month**. Both competitors pay hosted-AI and cloud-storage costs for every user, including free ones, so their free tiers are deliberately restricted (time-limited history, credits).

## Whimbrel's cost structure is different

Capture, transcription, storage and search all run on the user's machine. A user on local models (Ollama) or their own API key costs Whimbrel almost nothing. That user's sync rides on a per-account Turso database plus Cloudflare.

The only real variable cost is **hosted AI that Whimbrel pays for**, which doesn't exist yet. Here is an estimate for a heavy user: 60 meetings, 300 chat questions and 60 routine runs a month, which comes to about 5.9M input and 0.4M output tokens:

| Model running everything | Heavy user / month | Typical user (~⅓ of heavy) |
|---|---|---|
| Claude Haiku 5.5 ($0.10 / $0.50 per MTok) | ~$0.80 | ~$0.25 |
| Claude Sonnet 5.5 ($2 / $10) | ~$16 | ~$5 |
| Claude Opus 5.5 ($4 / $20) | ~$32 | ~$11 |

These numbers are estimates; measure real token usage from dogfooding before setting allowances. Prompt caching lowers them, and long chat context raises them.

The conclusion holds even if the estimates are off by 2×. **A cheap default model with a metered allowance of a stronger model keeps margins healthy at $12–15.** Unlimited Sonnet- or Opus-class usage at that price does not.

## Proposed tiers

| | **Free** | **Pro** | **Power** | **Team** *(later)* | **Enterprise** |
|---|---|---|---|---|---|
| Price | $0 | **$12/mo annual, $15 monthly** | **$24/mo annual, $29 monthly** | $16/seat/mo annual | Custom, ~$30/seat floor |
| Meeting capture, screen memory, search | Unlimited, no history limit | Unlimited | Unlimited | Unlimited | Unlimited |
| Devices | 1 computer + 1 phone, E2EE sync | Unlimited Mac + Windows + phones, E2EE sync | Unlimited | Unlimited | Unlimited |
| AI | Local (Ollama) or your own key, plus a small Whimbrel AI trial | Whimbrel AI included: fast model unlimited (fair use), monthly allowance of the stronger model | 3× Pro's stronger-model allowance, frontier model access | As Pro, pooled | Their own cloud (Bedrock / Azure / Vertex) or local-only |
| Routines | 2 active | All | All | All | All |
| Connectors, local MCP server | Yes | Yes | Yes | Yes | Yes |
| Team spaces, admin, SSO | | | | Yes, once built | SSO/SCIM, MDM, policy enforcement (local-only, retention), audit |

### Why these numbers

- **Free is the wedge.** Unlimited history with no meeting cap beats both competitors' free plans, and it costs almost nothing because the data is local. It also makes "your data never leaves your machine" a promise users can test for free.
- **Pro at $12 annual / $15 monthly sits just under Granola ($14) and Littlebird ($17).** It does more than Granola Business: screen memory, routines, local models. Paid features are things that really cost money (hosted AI, sync), not local features held back, so the upgrade reason is honest.
- **Power at $24 / $29 is for people who live in chat**, at about half of Littlebird Power ($42).
- **Enterprise is where local-first pays off.** "Work data never reaches our servers, and models run in your own cloud account" is a procurement answer neither competitor can give. It fits regulated buyers and companies whose employees take customer calls. Price it on value; don't race to the bottom.

## Risks

1. **Conversion from free is the weakest link.** A single-device user already running Ollama or their own key has almost no reason to pay. The upgrade triggers are a second computer (Mac + PC; Free already includes one phone), not wanting to manage keys, and routines past two. Expect a lower free-to-paid rate than Granola, whose free tier expires your history. If conversion is under ~2% after 90 days, tighten Free by limiting routines further, and surface multi-computer sync more prominently in the product.
2. **Don't undercut further.** $12 is already the low end. Going cheaper signals "less capable", and the cost advantage only applies to users who aren't on Whimbrel AI.
3. **Team is not sellable yet.** No shared spaces or admin exist. Listing a Team price before they ship invites churned trials.
4. **Windows is early access.** Charging Windows-only users full Pro before feature parity will produce refund requests. Consider keeping Windows free until the Phase 3 exit gate, or offering a discount until then.
5. **Turso has a per-account database cost.** Its free tier covers 100 databases, and every syncing account gets one. Decided 2026-10-17: Free includes the phone app, so Free accounts with a phone sync too ([15-mobile-app-design.md](15-mobile-app-design.md) §10). Price that in before sign-in opens.

## What has to be built first

None of these exist in the repo today:

- **Billing and entitlements:** Stripe checkout and customer portal, a plan field on the account in the control-plane database (`server/`), and an entitlement check in both apps.
- **Whimbrel AI proxy:** a server-side model gateway so no provider key ships in the app. It also needs per-account metering against the allowance and a hard spend cap; the `core/llm` ledger is a starting point for client-side accounting, but enforcement must be server-side.
- **Sync gating:** open sign-in beyond the current email allowlist (`ALLOWED_GOOGLE_EMAILS`), gated by plan (Free: one computer plus one phone).
- **Website:** a `/pricing/` page, and a Free/Pro choice in the registration flow. The comparison page's "Paid plans start at" row changes from "Not announced" to the real price.

## Recommendation

Launch **Free + Pro** first, at $12 annual / $15 monthly, with Whimbrel AI on Haiku 5.5 plus a Sonnet 5.5 allowance worth about $3–4 of tokens. Add Power once usage data shows how many users hit that allowance. Add Team once team spaces ship. Sell Enterprise by hand from day one: it is where local-first is worth the most and needs no extra code to start a conversation.
